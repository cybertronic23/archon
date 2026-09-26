# Embodied Agent OS — Getting Started

面向使用者的快速入口。完整设计决策见开发者笔记：[`../notes/design/embodied-agent-os-design.md`](../notes/design/embodied-agent-os-design.md)。

## 它是什么

Archon 具身平面是一个 **模型无关** 的 Agent OS 运行时：把 VLA / WAM / World Model / 经典策略等产出的动作提案，经确定性安全层后下发到仿真或真机（ROS2 话题契约一致）。

核心闭环：

```
Observation → Policy → Safety → Chronos → RobotBackend → Episode
```

单机器人只有一个 **Executive（执行权威）**；急停与抢占由 Runtime 仲裁，不由多 LLM 协商控制。

## 运行仿真 MVP（M0）

```bash
# 完整路点演示（Episode 默认写入 ~/.archon/episodes）
cargo run -p archon-embodied-cli -- --task-id demo_waypoints --step-ms 0

# 中途抢占
cargo run -p archon-embodied-cli -- --step-ms 5 --auto-stop-ms 200

# 交互式 stop / estop（stdin）
cargo run -p archon-embodied-cli -- --stdin-stop --step-ms 10
```

## 视觉门控闭环（M1）

Observation 为多模态容器：`proprio` + `modalities` + `annotations`。M1 填充 `images.primary`（合成 RGB）并由 `ColorBlobPolicy` 依赖 detection 再运动。

```bash
# 合成红块 → 检测 → 抓放原语 → Episode 含 media/images.primary/
cargo run -p archon-embodied-cli -- \
  --task-id pick_red_blob_sim \
  --policy color_blob \
  --camera synthetic \
  --step-ms 0

# 无目标：Safety 拒绝空路点，系统不崩溃
cargo run -p archon-embodied-cli -- \
  --task-id pick_red_blob_sim \
  --policy color_blob \
  --camera synthetic_blank \
  --step-ms 0
```

常用参数：

| 参数 | 含义 |
|------|------|
| `--backend sim` | 当前仅支持仿真后端；真机接入后切换配置即可 |
| `--policy` | `mock`（M0）或 `color_blob`（M1） |
| `--camera` | `none` / `synthetic` / `synthetic_blank`；`color_blob` 默认 synthetic |
| `--save-frames` | 是否把 RGB 写入 episode bundle 的 `media/`（默认 true） |
| `--rate-hz` | Chronos 插值频率（默认 50） |
| `--step-ms` | 下发间隔；CI / 快速跑可用 `0` |
| `--episode-dir` | Episode / media bundle 输出目录 |

## ROS2 话题契约（sim 与 real 共用）

默认命名空间：`/archon/arm`

| 话题 | 用途 |
|------|------|
| `joint_states` | 关节状态观测 |
| `joint_command` | 关节位置指令 |
| `gripper_command` | 夹爪 |
| `estop` | 急停 |
| `camera/image_raw` | 相机（M1 合成相机；真机后续订阅） |
| `user_stop` | 用户停止 |

切换仿真 → 真机：实现同一 `RobotBackend` trait、遵守上述话题名，CLI / Executive / Safety 无需改逻辑。

## 相关 crates

`archon-embodied` · `archon-runtime` · `archon-kinetic` · `archon-policy` · `archon-perception` · `archon-ros2` · `archon-sim` · `archon-embodied-cli`
