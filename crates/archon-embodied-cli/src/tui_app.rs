//! Ratatui chat session for multi-turn embodied control (M2d).

use std::io::{self, Stdout};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size as term_size, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::cursor::MoveTo;
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;
use tokio::sync::{mpsc, Mutex};
use tui_textarea::{Input, Key, TextArea};

use archon_perception::PerceptionBridge;
use archon_runtime::{Executive, RuntimeEvent};
use archon_embodied::RobotBackend;

use crate::session::{self, SessionConfig, TurnOutcome};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Busy,
}

#[derive(Clone)]
enum ChatRole {
    You,
    Sys,
    Help,
}

struct ChatLine {
    role: ChatRole,
    text: String,
}

enum WorkerCmd {
    Turn(String),
    Estop,
    Quit,
}

enum WorkerEvt {
    Started,
    Finished(Result<TurnOutcome, String>),
    QuitDone,
}

pub async fn run_tui(
    mut executive: Executive,
    backend: Arc<Mutex<dyn RobotBackend>>,
    mut perception: Option<PerceptionBridge>,
    cfg: SessionConfig,
    first_instruction: Option<String>,
) -> Result<()> {
    executive.config.keep_backend_alive = true;

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<WorkerCmd>(8);
    let (evt_tx, mut evt_rx) = mpsc::channel::<WorkerEvt>(8);
    let events = executive.events.clone();

    let worker_cfg = cfg.clone();
    let worker_backend = backend.clone();
    let worker = tokio::spawn(async move {
        let safety = session::default_safety(worker_cfg.robot);
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                WorkerCmd::Turn(text) => {
                    let _ = evt_tx.send(WorkerEvt::Started).await;
                    let outcome = session::run_turn(
                        &mut executive,
                        worker_backend.clone(),
                        &safety,
                        &mut perception,
                        &worker_cfg,
                        &text,
                    )
                    .await
                    .map_err(|e| format!("{e:#}"));
                    let _ = evt_tx.send(WorkerEvt::Finished(outcome)).await;
                }
                WorkerCmd::Estop => {
                    events.publish(RuntimeEvent::EStop {
                        reason: "tui".into(),
                    });
                }
                WorkerCmd::Quit => {
                    {
                        let mut b = worker_backend.lock().await;
                        let _ = b.shutdown().await;
                    }
                    let _ = evt_tx.send(WorkerEvt::QuitDone).await;
                    break;
                }
            }
        }
    });

    let (mut terminal, prev_size) = setup_terminal().context("setup TUI terminal")?;
    let mut lines: Vec<ChatLine> = Vec::new();
    let mut phase = Phase::Idle;
    let mut scroll: u16 = 0;
    let mut textarea = TextArea::default();
    textarea.set_placeholder_text("在此输入指令，Enter 发送");
    textarea.set_cursor_line_style(Style::default());
    textarea.set_style(Style::default().fg(Color::White));
    set_input_block(&mut textarea, Phase::Idle);

    push_sys(
        &mut lines,
        format!(
            "就绪 · {} / {} · {} · 底部唯一输入框打字",
            cfg.backend_name, cfg.model, cfg.policy_name
        ),
    );
    push_help(
        &mut lines,
        "紧凑窗：旁侧看 MuJoCo。Enter 发送 · /estop · /quit · PgUp/PgDn",
    );

    if let Some(text) = first_instruction {
        let t = text.trim().to_string();
        if !t.is_empty() {
            push_you(&mut lines, t.clone());
            phase = Phase::Busy;
            let _ = cmd_tx.send(WorkerCmd::Turn(t)).await;
        }
    }

    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(80));
    let mut spinner = 0usize;
    let spin = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

    let mut should_quit = false;
    while !should_quit {
        let status = match phase {
            Phase::Idle => "Idle".into(),
            Phase::Busy => format!("{} Running", spin[spinner % spin.len()]),
        };
        set_input_block(&mut textarea, phase);
        draw(
            &mut terminal,
            &cfg,
            &lines,
            &textarea,
            &status,
            phase,
            scroll,
        )?;

        tokio::select! {
            _ = tick.tick() => {
                spinner = spinner.wrapping_add(1);
            }
            Some(evt) = evt_rx.recv() => {
                match evt {
                    WorkerEvt::Started => {
                        phase = Phase::Busy;
                    }
                    WorkerEvt::Finished(Ok(out)) => {
                        phase = Phase::Idle;
                        push_sys(
                            &mut lines,
                            format!(
                                "{:?} · {} cmds · {} ms · {}",
                                out.result.status,
                                out.result.commands_sent,
                                out.result.duration_ms,
                                out.episode_path.display()
                            ),
                        );
                        if !out.result.message.is_empty() {
                            push_sys(&mut lines, out.result.message.clone());
                        }
                    }
                    WorkerEvt::Finished(Err(e)) => {
                        phase = Phase::Idle;
                        push_sys(&mut lines, format!("error: {e}"));
                    }
                    WorkerEvt::QuitDone => {
                        should_quit = true;
                    }
                }
            }
            maybe = events.next() => {
                let Some(Ok(ev)) = maybe else { continue };
                if let Event::Key(key) = ev {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    if handle_key(
                        key,
                        &mut textarea,
                        &mut lines,
                        &mut phase,
                        &mut scroll,
                        &cmd_tx,
                        &mut should_quit,
                    ).await? {
                        // submitted or command handled
                    }
                }
            }
        }
    }

    restore_terminal(&mut terminal, prev_size)?;
    let _ = worker.await;
    Ok(())
}

fn set_input_block(textarea: &mut TextArea<'_>, phase: Phase) {
    let (title, border) = if phase == Phase::Busy {
        (
            " › busy — wait or /estop ",
            Style::default().fg(Color::DarkGray),
        )
    } else {
        (
            " › type here (唯一输入框) ",
            Style::default().fg(Color::Rgb(120, 200, 180)),
        )
    };
    textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title(title),
    );
}

async fn handle_key(
    key: KeyEvent,
    textarea: &mut TextArea<'_>,
    lines: &mut Vec<ChatLine>,
    phase: &mut Phase,
    scroll: &mut u16,
    cmd_tx: &mpsc::Sender<WorkerCmd>,
    should_quit: &mut bool,
) -> Result<bool> {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        let _ = cmd_tx.send(WorkerCmd::Quit).await;
        push_sys(lines, "正在退出并关闭仿真会话…".into());
        *phase = Phase::Busy;
        return Ok(true);
    }

    match key.code {
        KeyCode::PageUp => {
            *scroll = scroll.saturating_add(3);
            return Ok(true);
        }
        KeyCode::PageDown => {
            *scroll = scroll.saturating_sub(3);
            return Ok(true);
        }
        KeyCode::Enter => {
            if *phase == Phase::Busy {
                return Ok(true);
            }
            let text = textarea.lines().join("\n");
            let text = text.trim().to_string();
            if text.is_empty() {
                return Ok(true);
            }
            textarea.select_all();
            textarea.cut();
            *scroll = 0;

            if let Some(cmd) = text.strip_prefix('/') {
                return handle_slash(cmd.trim(), lines, phase, cmd_tx, should_quit).await;
            }

            push_you(lines, text.clone());
            *phase = Phase::Busy;
            cmd_tx
                .send(WorkerCmd::Turn(text))
                .await
                .context("send turn to worker")?;
            return Ok(true);
        }
        _ => {}
    }

    // Map crossterm key → tui-textarea Input
    textarea.input(crossterm_to_input(key));
    Ok(false)
}

async fn handle_slash(
    cmd: &str,
    lines: &mut Vec<ChatLine>,
    phase: &mut Phase,
    cmd_tx: &mpsc::Sender<WorkerCmd>,
    should_quit: &mut bool,
) -> Result<bool> {
    match cmd.to_ascii_lowercase().as_str() {
        "q" | "quit" | "exit" => {
            push_sys(lines, "正在退出…".into());
            *phase = Phase::Busy;
            let _ = cmd_tx.send(WorkerCmd::Quit).await;
            let _ = should_quit;
            Ok(true)
        }
        "estop" | "stop" => {
            push_sys(lines, "E-STOP 已请求".into());
            let _ = cmd_tx.send(WorkerCmd::Estop).await;
            Ok(true)
        }
        "help" | "h" | "?" => {
            push_help(
                lines,
                "命令: /help  /estop  /quit\n\
                 直接输入自然语言，例如「向前走一点再左转90度」\n\
                 Busy 时请等待当前动作结束；可用 /estop 抢占",
            );
            Ok(true)
        }
        "clear" => {
            lines.clear();
            push_sys(lines, "历史已清空".into());
            Ok(true)
        }
        other => {
            push_sys(lines, format!("未知命令 /{other} · 输入 /help"));
            Ok(true)
        }
    }
}

fn crossterm_to_input(key: KeyEvent) -> Input {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let key = match key.code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Enter => Key::Enter,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Tab => Key::Tab,
        KeyCode::Delete => Key::Delete,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Esc => Key::Esc,
        _ => Key::Null,
    };
    Input {
        key,
        ctrl,
        alt,
        shift,
    }
}

fn push_you(lines: &mut Vec<ChatLine>, text: String) {
    lines.push(ChatLine {
        role: ChatRole::You,
        text,
    });
}

fn push_sys(lines: &mut Vec<ChatLine>, text: String) {
    lines.push(ChatLine {
        role: ChatRole::Sys,
        text,
    });
}

fn push_help(lines: &mut Vec<ChatLine>, text: impl Into<String>) {
    lines.push(ChatLine {
        role: ChatRole::Help,
        text: text.into(),
    });
}

/// Compact panel height (rows). Keeps room for MuJoCo beside/above a small terminal.
const PANEL_ROWS: u16 = 16;
const PANEL_COLS: u16 = 100;

fn draw(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    cfg: &SessionConfig,
    lines: &[ChatLine],
    textarea: &TextArea<'_>,
    status: &str,
    phase: Phase,
    scroll: u16,
) -> Result<()> {
    terminal.draw(|f| {
        let full = f.area();
        // Draw only a bottom strip so the TUI feels like a dock, not a takeover.
        let h = PANEL_ROWS.min(full.height).max(10);
        let panel = Rect::new(0, full.height.saturating_sub(h), full.width, h);

        // Dim / clear everything above the panel (alt-screen leftover).
        if panel.y > 0 {
            let top = Rect::new(0, 0, full.width, panel.y);
            f.render_widget(Clear, top);
            f.render_widget(
                Paragraph::new("  (MuJoCo viewer 请放在旁边；此区域留空)")
                    .style(Style::default().fg(Color::DarkGray)),
                top,
            );
        }

        f.render_widget(Clear, panel);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(4),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(panel);

        let title = Paragraph::new(Line::from(vec![
            Span::styled(
                " Archon ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(120, 200, 180))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{} · {}", cfg.backend_name, cfg.model),
                Style::default().fg(Color::Rgb(180, 210, 230)),
            ),
            Span::raw(" "),
            Span::styled(
                status.to_string(),
                if phase == Phase::Busy {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Green)
                },
            ),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(60, 90, 100)))
                .title(" session (compact) "),
        );
        f.render_widget(title, chunks[0]);

        let max_rows = chunks[1].height.saturating_sub(2) as usize;
        let total = lines.len();
        let end = total.saturating_sub(scroll as usize);
        let start = end.saturating_sub(max_rows.max(1));
        let window = &lines[start..end];

        let items: Vec<ListItem> = window
            .iter()
            .map(|l| {
                let (prefix, style) = match l.role {
                    ChatRole::You => (
                        "you › ",
                        Style::default()
                            .fg(Color::Rgb(140, 220, 255))
                            .add_modifier(Modifier::BOLD),
                    ),
                    ChatRole::Sys => ("sys › ", Style::default().fg(Color::Rgb(160, 230, 180))),
                    ChatRole::Help => ("tip › ", Style::default().fg(Color::Rgb(200, 180, 120))),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(prefix, style),
                    Span::styled(l.text.clone(), Style::default().fg(Color::Gray)),
                ]))
            })
            .collect();

        let chat = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(60, 90, 100)))
                .title(" chat "),
        );
        f.render_widget(chat, chunks[1]);

        // Single input widget (border comes from TextArea::set_block).
        f.render_widget(textarea, chunks[2]);

        let footer = Paragraph::new("唯一输入框在上 ↑ · Enter 发送 · /help · /estop · /quit")
            .style(Style::default().fg(Color::DarkGray))
            .wrap(Wrap { trim: true });
        f.render_widget(footer, chunks[3]);
    })?;
    Ok(())
}

fn setup_terminal() -> Result<(Terminal<CrosstermBackend<Stdout>>, Option<(u16, u16)>)> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    let prev = term_size().ok();
    // Shrink terminal window when the host supports CSI 8 (iTerm / many macOS terminals).
    // Failure is fine — user can still drag the window smaller.
    let _ = execute!(
        stdout,
        crossterm::style::Print(format!("\x1b[8;{PANEL_ROWS};{PANEL_COLS}t")),
        MoveTo(0, 0),
    );
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Ok((Terminal::new(backend)?, prev))
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    prev: Option<(u16, u16)>,
) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    if let Some((cols, rows)) = prev {
        let _ = execute!(
            terminal.backend_mut(),
            crossterm::style::Print(format!("\x1b[8;{rows};{cols}t")),
        );
    }
    terminal.show_cursor()?;
    Ok(())
}
