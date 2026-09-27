//! Archon Embodied CLI — sim / MuJoCo assets / language instructions / TUI.

mod session;
mod tui_app;

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use archon_kinetic::Chronos;
use archon_perception::{ColorBlobDetector, PerceptionBridge, SyntheticColorCamera};
use archon_policy::{ColorBlobPolicy, RobotKind};
use archon_runtime::{Executive, ExecutiveConfig, RuntimeEvent};
use archon_sim::SimBackend;
use archon_sim_bridge::{
    default_catalog_path, default_worker_script, ensure_script_exists, list_builtins, load_catalog,
    resolve_model_spec, workspace_python_root, BridgeConfig, BridgedSimBackend,
};
use clap::Parser;
use tokio::sync::Mutex;

use session::{SessionConfig, TurnOutcome};

#[derive(Parser, Debug)]
#[command(
    name = "archon-embodied",
    about = "Archon Embodied Agent OS — MuJoCo-ready control loop"
)]
struct Args {
    /// Task id recorded in the episode
    #[arg(long, default_value = "demo_waypoints")]
    task_id: String,

    /// Backend: `sim` | `mujoco` | `maniskill`
    #[arg(long, default_value = "sim")]
    backend: String,

    /// Model: `builtin:desktop_arm` | local path/dir | https://...xml|.zip
    #[arg(long, default_value = "builtin:desktop_arm")]
    model: String,

    /// List builtin models and exit
    #[arg(long, default_value_t = false)]
    list_models: bool,

    /// Override worker script
    #[arg(long)]
    worker: Option<PathBuf>,

    /// Policy: `mock` | `color_blob` | `instruction` | `llm`
    #[arg(long, default_value = "mock")]
    policy: String,

    /// Natural-language instruction (for `instruction` / `llm`); with `--tui` becomes first turn
    #[arg(long)]
    instruction: Option<String>,

    /// LLM API key (or env DEEPSEEK_API_KEY / OPENAI_API_KEY)
    #[arg(long, env = "DEEPSEEK_API_KEY")]
    llm_api_key: Option<String>,

    /// LLM base URL (DeepSeek default if unset)
    #[arg(long, env = "LLM_BASE_URL")]
    llm_base_url: Option<String>,

    /// LLM model id
    #[arg(long, default_value = "deepseek-chat", env = "LLM_MODEL")]
    llm_model: String,

    /// Camera: `none` | `synthetic` | `synthetic_blank`
    #[arg(long, default_value = "none")]
    camera: String,

    /// Persist RGB frames under episode bundle media/
    #[arg(long, default_value_t = true)]
    save_frames: bool,

    /// Open MuJoCo interactive viewer window (needs local GUI / display)
    #[arg(long, default_value_t = false)]
    viewer: bool,

    /// Interactive ratatui chat (multi-turn; keeps sim session alive)
    #[arg(long, default_value_t = false)]
    tui: bool,

    /// Record offscreen MP4 for sharing (needs ffmpeg). Optional PATH; default `<episode>/demo.mp4`.
    #[arg(long, num_args = 0..=1, value_name = "PATH")]
    record_video: Option<Option<PathBuf>>,

    /// Control rate Hz for Chronos interpolation
    #[arg(long, default_value_t = 50.0)]
    rate_hz: f64,

    /// Wall-clock delay between commanded steps (ms)
    #[arg(long, default_value_t = 0)]
    step_ms: u64,

    /// Directory for episode bundles
    #[arg(long)]
    episode_dir: Option<PathBuf>,

    #[arg(long, default_value_t = false)]
    stdin_stop: bool,

    #[arg(long, default_value_t = 0)]
    auto_stop_ms: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    if args.list_models {
        let catalog_path = default_catalog_path();
        let catalog = load_catalog(&catalog_path)?;
        println!("Builtin MuJoCo assets (catalog: {}):", catalog_path.display());
        for (name, desc) in list_builtins(&catalog) {
            println!("  builtin:{name:24} {desc}");
        }
        println!("\nAlso accepted: local .xml / directory, or https://…xml|zip");
        return Ok(());
    }

    if args.tui && matches!(args.policy.as_str(), "color_blob") {
        anyhow::bail!("--tui does not support --policy color_blob yet; use instruction or llm");
    }

    let robot = RobotKind::parse_model_hint(&args.model);
    // One-shot defaults a phrase; TUI starts empty unless --instruction is given.
    let instruction = if args.tui {
        args.instruction.clone()
    } else {
        args.instruction.clone().or_else(|| {
            if args.policy == "instruction" || args.policy == "llm" {
                Some(match robot {
                    RobotKind::DiffCar => "向前走一点".into(),
                    RobotKind::Arm => "挥手".into(),
                })
            } else {
                None
            }
        })
    };

    let camera = if args.policy == "color_blob" && args.camera == "none" && args.backend == "sim" {
        "synthetic".to_string()
    } else {
        args.camera.clone()
    };

    let episode_root = args.episode_dir.clone().unwrap_or_else(|| {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        home.join(".archon").join("episodes")
    });

    let episode_id = format!("ep-{}", archon_embodied::now_us());
    let bundle_dir = episode_root.join(&episode_id);

    let mut joint_names: Vec<String> = (1..=6).map(|i| format!("joint_{i}")).collect();

    let backend: Arc<Mutex<dyn archon_embodied::RobotBackend>> = match args.backend.as_str() {
        "sim" => {
            let sim = SimBackend::desktop_arm();
            eprintln!(
                "[archon-embodied] sim backend topics: joint_states={}",
                sim.topic_contract().joint_states()
            );
            Arc::new(Mutex::new(sim))
        }
        "mujoco" | "maniskill" => {
            let script = args
                .worker
                .clone()
                .unwrap_or_else(|| default_worker_script(&args.backend));
            ensure_script_exists(&script)?;

            let catalog_path = default_catalog_path();
            let catalog = load_catalog(&catalog_path).ok();
            let resolved = if let Some(cat) = catalog.as_ref() {
                Some(resolve_model_spec(&args.model, cat, &catalog_path)?)
            } else if args.model.starts_with("builtin:") {
                anyhow::bail!("catalog.json not found; cannot resolve {}", args.model);
            } else {
                None
            };

            let mut cfg = if args.backend == "maniskill" {
                BridgeConfig::maniskill(&script)
            } else {
                BridgeConfig::mujoco(&script)
            };
            if args.save_frames {
                cfg = cfg.with_media_root(&bundle_dir);
            }
            if args.viewer {
                cfg = cfg.with_viewer(true);
                eprintln!(
                    "[archon-embodied] MuJoCo viewer enabled (close the window on quit to finish)"
                );
            }
            if let Some(path_opt) = &args.record_video {
                if args.tui {
                    eprintln!("[archon-embodied] warning: --record-video with --tui records only the session open; prefer one-shot for demos");
                }
                let out = path_opt
                    .clone()
                    .unwrap_or_else(|| bundle_dir.join("demo.mp4"));
                eprintln!(
                    "[archon-embodied] recording video → {} (ffmpeg required)",
                    out.display()
                );
                cfg = cfg.with_record_video(out);
            }
            if let Some(root) = workspace_python_root() {
                if let Some(repo) = root.parent() {
                    cfg = cfg.with_cwd(repo);
                }
            }
            if let Some(res) = &resolved {
                cfg = cfg.with_model(&res.model_path);
                if let Some(cam) = &res.camera {
                    cfg = cfg.with_camera(cam);
                }
                eprintln!(
                    "[archon-embodied] model={} ({})",
                    res.model_path.display(),
                    res.source
                );
            } else {
                let path = PathBuf::from(&args.model);
                cfg = cfg.with_model(std::fs::canonicalize(&path).unwrap_or(path));
            }

            eprintln!(
                "[archon-embodied] bridged backend={} worker={}",
                args.backend,
                script.display()
            );
            Arc::new(Mutex::new(BridgedSimBackend::new(cfg)))
        }
        other => anyhow::bail!("unknown backend '{other}'; use sim | mujoco | maniskill"),
    };

    // Connect early for mujoco so Chronos gets discovered joint names.
    if args.backend == "mujoco" || args.backend == "maniskill" {
        let mut b = backend.lock().await;
        b.connect().await.context("backend connect")?;
        let obs = b.read_observation().await.context("read observation")?;
        if !obs.joints().names.is_empty() {
            joint_names = obs.joints().names.clone();
        }
    }

    let chronos = Chronos::new(args.rate_hz, joint_names.clone());
    let mut executive = Executive::new(
        ExecutiveConfig {
            task_id: args.task_id.clone(),
            control_step_ms: args.step_ms,
            episode_id: Some(episode_id.clone()),
            keep_backend_alive: args.tui,
            ..Default::default()
        },
        chronos,
    );

    if args.stdin_stop && !args.tui {
        let events = executive.events.clone();
        std::thread::spawn(move || {
            let stdin = io::stdin();
            let mut lock = stdin.lock();
            let mut line = String::new();
            eprintln!("[archon-embodied] type 'stop' or 'estop' + Enter to preempt");
            loop {
                line.clear();
                if lock.read_line(&mut line).ok().filter(|n| *n > 0).is_none() {
                    break;
                }
                match line.trim().to_ascii_lowercase().as_str() {
                    "stop" => {
                        let _ = writeln!(io::stderr(), "[archon-embodied] user stop requested");
                        events.publish(RuntimeEvent::UserStop {
                            reason: "stdin".into(),
                        });
                        break;
                    }
                    "estop" => {
                        let _ = writeln!(io::stderr(), "[archon-embodied] ESTOP requested");
                        events.publish(RuntimeEvent::EStop {
                            reason: "stdin".into(),
                        });
                        break;
                    }
                    _ => {}
                }
            }
        });
    }

    if args.auto_stop_ms > 0 && !args.tui {
        let events = executive.events.clone();
        let ms = args.auto_stop_ms;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            events.publish(RuntimeEvent::UserStop {
                reason: format!("auto_stop_ms={ms}"),
            });
        });
    }

    let perception = match camera.as_str() {
        "none" => None,
        "synthetic" | "synthetic_blob" => {
            let mut bridge = PerceptionBridge::new(
                Box::new(SyntheticColorCamera::with_blob()),
                ColorBlobDetector::default(),
            );
            if args.save_frames {
                bridge = bridge.with_media_root(&bundle_dir);
            }
            Some(bridge)
        }
        "synthetic_blank" => {
            let mut bridge = PerceptionBridge::new(
                Box::new(SyntheticColorCamera::blank()),
                ColorBlobDetector::default(),
            );
            if args.save_frames {
                bridge = bridge.with_media_root(&bundle_dir);
            }
            Some(bridge)
        }
        other => anyhow::bail!("unknown camera '{other}'"),
    };

    let session_cfg = SessionConfig {
        task_id: args.task_id.clone(),
        backend_name: args.backend.clone(),
        policy_name: args.policy.clone(),
        camera: camera.clone(),
        model: args.model.clone(),
        robot,
        joint_names: joint_names.clone(),
        episode_root: episode_root.clone(),
        save_frames: args.save_frames,
        llm_api_key: args.llm_api_key.clone(),
        llm_base_url: args.llm_base_url.clone(),
        llm_model: args.llm_model.clone(),
    };

    if args.tui {
        eprintln!("[archon-embodied] entering TUI (MuJoCo viewer stays open across turns)");
        return tui_app::run_tui(executive, backend, perception, session_cfg, instruction).await;
    }

    println!(
        "Running embodied loop: task={} backend={} policy={} model={} robot={} instruction={:?}",
        args.task_id, args.backend, args.policy, args.model, robot.as_str(), instruction
    );

    let safety = session::default_safety(robot);
    let mut perception = perception;

    let outcome: TurnOutcome = match args.policy.as_str() {
        "color_blob" => {
            let policy = ColorBlobPolicy::new();
            let bridge = perception
                .as_mut()
                .context("color_blob requires --camera synthetic|synthetic_blank")?;
            let task_context = serde_json::json!({
                "task_id": args.task_id,
                "backend": args.backend,
                "policy": args.policy,
                "camera": camera,
                "model": args.model,
                "instruction": instruction,
                "robot": robot.as_str(),
            });
            let (result, episode) = executive
                .run_once_with_perception(&policy, &safety, backend, Some(bridge), task_context)
                .await
                .context("executive run_once")?;
            let path = if args.save_frames {
                episode.save_bundle(&bundle_dir)?;
                bundle_dir.join("episode.json")
            } else {
                let path = episode_root.join(format!("{}.json", episode.id));
                episode.save_to_file(&path)?;
                path
            };
            TurnOutcome {
                result,
                episode,
                episode_path: path,
            }
        }
        "mock" | "instruction" | "llm" => {
            let text = if args.policy == "mock" {
                instruction.unwrap_or_default()
            } else {
                instruction.context(format!("--policy {} needs --instruction", args.policy))?
            };
            session::run_turn(
                &mut executive,
                backend,
                &safety,
                &mut perception,
                &session_cfg,
                &text,
            )
            .await
            .context("executive run_once")?
        }
        other => {
            anyhow::bail!("unknown policy '{other}'; use mock | color_blob | instruction | llm")
        }
    };

    println!(
        "status={:?} commands_sent={} duration_ms={} episode={}",
        outcome.result.status,
        outcome.result.commands_sent,
        outcome.result.duration_ms,
        outcome.episode_path.display()
    );
    println!("message={}", outcome.result.message);

    Ok(())
}
