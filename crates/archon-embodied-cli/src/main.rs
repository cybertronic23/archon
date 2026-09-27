//! Archon Embodied CLI — sim / MuJoCo assets / language instructions.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use archon_kinetic::Chronos;
use archon_perception::{ColorBlobDetector, PerceptionBridge, SyntheticColorCamera};
use archon_policy::{
    ColorBlobPolicy, InstructionPolicy, LimitSafetyGate, LlmPolicy, LlmPolicyConfig, MockPolicy,
    RobotKind,
};
use archon_runtime::{Executive, ExecutiveConfig, RuntimeEvent};
use archon_sim::SimBackend;
use archon_sim_bridge::{
    default_catalog_path, default_worker_script, ensure_script_exists, list_builtins, load_catalog,
    resolve_model_spec, workspace_python_root, BridgeConfig, BridgedSimBackend,
};
use clap::Parser;
use tokio::sync::Mutex;

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

    /// Natural-language instruction (for `instruction` / `llm`)
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

    let robot = RobotKind::parse_model_hint(&args.model);
    let instruction = args.instruction.clone().or_else(|| {
        if args.policy == "instruction" || args.policy == "llm" {
            Some(match robot {
                RobotKind::DiffCar => "向前走一点".into(),
                RobotKind::Arm => "挥手".into(),
            })
        } else {
            None
        }
    });

    let camera = if args.policy == "color_blob" && args.camera == "none" && args.backend == "sim" {
        "synthetic".to_string()
    } else {
        args.camera.clone()
    };

    let episode_root = args.episode_dir.unwrap_or_else(|| {
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
                eprintln!("[archon-embodied] MuJoCo viewer enabled (close the window or wait for run to finish)");
            }
            if let Some(path_opt) = &args.record_video {
                let out = path_opt.clone().unwrap_or_else(|| {
                    bundle_dir.join("demo.mp4")
                });
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
        other => anyhow::bail!(
            "unknown backend '{other}'; use sim | mujoco | maniskill"
        ),
    };

    // Connect early for mujoco so Chronos gets discovered joint names.
    if args.backend == "mujoco" || args.backend == "maniskill" {
        {
            let mut b = backend.lock().await;
            b.connect().await.context("backend connect")?;
            let obs = b.read_observation().await.context("read observation")?;
            if !obs.joints().names.is_empty() {
                joint_names = obs.joints().names.clone();
            }
            // Leave connected; Executive.run_once will connect again (idempotent enough).
        }
    }

    let chronos = Chronos::new(args.rate_hz, joint_names.clone());
    let mut executive = Executive::new(
        ExecutiveConfig {
            task_id: args.task_id.clone(),
            control_step_ms: args.step_ms,
            episode_id: Some(episode_id.clone()),
            ..Default::default()
        },
        chronos,
    );

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

    let mut perception = match camera.as_str() {
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

    let safety = match robot {
        RobotKind::DiffCar => LimitSafetyGate::for_planar_base(),
        RobotKind::Arm => LimitSafetyGate::default(),
    };
    let task_context = serde_json::json!({
        "task_id": args.task_id,
        "backend": args.backend,
        "policy": args.policy,
        "camera": camera,
        "model": args.model,
        "instruction": instruction,
        "robot": robot.as_str(),
    });

    println!(
        "Running embodied loop: task={} backend={} policy={} model={} robot={} instruction={:?}",
        args.task_id, args.backend, args.policy, args.model, robot.as_str(), instruction
    );

    let (result, episode) = match args.policy.as_str() {
        "mock" => {
            let policy = MockPolicy::new();
            run_with_optional_perception(
                &mut executive,
                &policy,
                &safety,
                backend,
                perception.as_mut(),
                task_context,
            )
            .await
        }
        "color_blob" => {
            let policy = ColorBlobPolicy::new();
            let bridge = perception
                .as_mut()
                .context("color_blob requires --camera synthetic|synthetic_blank")?;
            executive
                .run_once_with_perception(&policy, &safety, backend, Some(bridge), task_context)
                .await
        }
        "instruction" => {
            let text = instruction.context("--policy instruction needs --instruction")?;
            let policy = InstructionPolicy::new(text)
                .with_joint_names(joint_names)
                .with_robot(robot);
            run_with_optional_perception(
                &mut executive,
                &policy,
                &safety,
                backend,
                perception.as_mut(),
                task_context,
            )
            .await
        }
        "llm" => {
            let text = instruction.context("--policy llm needs --instruction")?;
            let api_key = args
                .llm_api_key
                .clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .context(
                    "LLM policy needs DEEPSEEK_API_KEY or OPENAI_API_KEY (or --llm-api-key)",
                )?;
            let base_url = args
                .llm_base_url
                .clone()
                .unwrap_or_else(|| "https://api.deepseek.com".into());
            let cfg = LlmPolicyConfig {
                api_key,
                base_url,
                model: args.llm_model.clone(),
                robot,
                fallback_rules: true,
            };
            eprintln!(
                "[archon-embodied] LLM compiler model={} base={}",
                cfg.model, cfg.base_url
            );
            let policy = LlmPolicy::new(cfg, text);
            run_with_optional_perception(
                &mut executive,
                &policy,
                &safety,
                backend,
                perception.as_mut(),
                task_context,
            )
            .await
        }
        other => {
            anyhow::bail!("unknown policy '{other}'; use mock | color_blob | instruction | llm")
        }
    }
    .context("executive run_once")?;

    let bridged = matches!(args.backend.as_str(), "mujoco" | "maniskill");
    if (perception.is_some() || bridged) && args.save_frames {
        episode
            .save_bundle(&bundle_dir)
            .with_context(|| format!("save episode bundle to {}", bundle_dir.display()))?;
        println!(
            "status={:?} commands_sent={} duration_ms={} episode={}",
            result.status,
            result.commands_sent,
            result.duration_ms,
            bundle_dir.join("episode.json").display()
        );
    } else {
        let path = episode_root.join(format!("{}.json", episode.id));
        episode
            .save_to_file(&path)
            .with_context(|| format!("save episode to {}", path.display()))?;
        println!(
            "status={:?} commands_sent={} duration_ms={} episode={}",
            result.status, result.commands_sent, result.duration_ms, path.display()
        );
    }
    println!("message={}", result.message);

    Ok(())
}

async fn run_with_optional_perception(
    executive: &mut Executive,
    policy: &dyn archon_embodied::Policy,
    safety: &dyn archon_embodied::SafetyGate,
    backend: Arc<Mutex<dyn archon_embodied::RobotBackend>>,
    perception: Option<&mut PerceptionBridge>,
    task_context: serde_json::Value,
) -> Result<(archon_embodied::ExecutionResult, archon_embodied::Episode)> {
    if let Some(bridge) = perception {
        executive
            .run_once_with_perception(policy, safety, backend, Some(bridge), task_context)
            .await
    } else {
        executive
            .run_once(policy, safety, backend, task_context)
            .await
    }
}
