//! Archon Embodied CLI — run sim vertical slice and write Episode JSON.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use archon_kinetic::Chronos;
use archon_policy::{LimitSafetyGate, MockPolicy};
use archon_runtime::{Executive, ExecutiveConfig, RuntimeEvent};
use archon_sim::SimBackend;
use clap::Parser;
use tokio::sync::Mutex;

#[derive(Parser, Debug)]
#[command(
    name = "archon-embodied",
    about = "Archon Embodied Agent OS — sim-first control loop MVP"
)]
struct Args {
    /// Task id recorded in the episode
    #[arg(long, default_value = "demo_waypoints")]
    task_id: String,

    /// Backend: currently only `sim` (real comes later via same RobotBackend trait)
    #[arg(long, default_value = "sim")]
    backend: String,

    /// Control rate Hz for Chronos interpolation
    #[arg(long, default_value_t = 50.0)]
    rate_hz: f64,

    /// Wall-clock delay between commanded steps (ms). Use 0 for fast CI.
    #[arg(long, default_value_t = 0)]
    step_ms: u64,

    /// Directory for episode JSON (default: ~/.archon/episodes)
    #[arg(long)]
    episode_dir: Option<PathBuf>,

    /// Listen for stdin line "stop" / "estop" during the run
    #[arg(long, default_value_t = false)]
    stdin_stop: bool,

    /// Auto-send user stop after N milliseconds (0 = disabled)
    #[arg(long, default_value_t = 0)]
    auto_stop_ms: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    if args.backend != "sim" {
        anyhow::bail!(
            "backend '{}' not available yet; use --backend sim. \
             Real SO101/Microduck will share the same RobotBackend + ROS2 topic contract.",
            args.backend
        );
    }

    let chronos = Chronos::new(
        args.rate_hz,
        (1..=6).map(|i| format!("joint_{i}")).collect(),
    );
    let mut executive = Executive::new(
        ExecutiveConfig {
            task_id: args.task_id.clone(),
            control_step_ms: args.step_ms,
            ..Default::default()
        },
        chronos,
    );

    let sim = SimBackend::desktop_arm();
    eprintln!(
        "[archon-embodied] sim backend topics: joint_states={}",
        sim.topic_contract().joint_states()
    );
    let backend: Arc<Mutex<dyn archon_embodied::RobotBackend>> = Arc::new(Mutex::new(sim));

    if args.stdin_stop {
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

    if args.auto_stop_ms > 0 {
        let events = executive.events.clone();
        let ms = args.auto_stop_ms;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            events.publish(RuntimeEvent::UserStop {
                reason: format!("auto_stop_ms={ms}"),
            });
        });
    }

    let policy = MockPolicy::new();
    let safety = LimitSafetyGate::default();
    let task_context = serde_json::json!({
        "task_id": args.task_id,
        "backend": args.backend,
    });

    println!("Running embodied loop: task={} backend=sim", args.task_id);
    let (result, episode) = executive
        .run_once(&policy, &safety, backend, task_context)
        .await
        .context("executive run_once")?;

    let episode_dir = args.episode_dir.unwrap_or_else(|| {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        home.join(".archon").join("episodes")
    });
    let path = episode_dir.join(format!("{}.json", episode.id));
    episode
        .save_to_file(&path)
        .with_context(|| format!("save episode to {}", path.display()))?;

    println!(
        "status={:?} commands_sent={} duration_ms={} episode={}",
        result.status, result.commands_sent, result.duration_ms, path.display()
    );
    println!("message={}", result.message);

    Ok(())
}
