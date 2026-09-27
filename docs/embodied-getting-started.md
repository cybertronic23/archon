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

## 外接仿真（M2a MuJoCo 最小完备集）

拿到仓库后，可在 MuJoCo 里：**选资产 → 语言指令 → 驱动本体 → 写 Episode**。

```bash
# 1) Python 依赖（建议 venv）
python3 -m venv .venv-mujoco
.venv-mujoco/bin/pip install -r python/requirements-mujoco.txt

# 2) 查看内置资产
cargo run -p archon-embodied-cli -- --list-models

# 3) 内置臂 + 语言指令「挥手」（加 --viewer 可弹 MuJoCo 窗口）
cargo run -p archon-embodied-cli -- \
  --backend mujoco \
  --model builtin:desktop_arm \
  --policy instruction \
  --instruction "挥手" \
  --viewer \
  --step-ms 20
```

| 参数 | 说明 |
|------|------|
| `--backend mujoco` | NDJSON 桥接 MuJoCo worker |
| `--model builtin:desktop_arm` | 内置资产；也可本地 `.xml`/目录，或 `https://…xml\|zip`（缓存到 `~/.archon/assets/cache`） |
| `--policy instruction` | 短语→运动原语（挥手/回零/伸出/点头/开合夹爪…） |
| `--instruction` | 中英文指令文本 |
| `--viewer` | 弹出 MuJoCo 交互窗口（macOS 会自动用 `mjpython`；建议 `--step-ms 20`） |
| `--worker PATH` | 覆盖默认 worker 脚本 |
| `ARCHON_PYTHON` | 指定 Python 解释器（可选） |

语言策略为**确定性原语路由**（不是完整 VLA）；自由文本需命中已知短语。远程资产需为 **MJCF（.xml）或含 MJCF 的 zip**（公开 Menagerie 等请下载后 `--model` 指向 `scene.xml`）。

设计：`notes/design/m2a-mujoco-complete.md`。

## LLM 自然语言（M2c，DeepSeek 等）

规则策略之外，可用云端 LLM 把自然语言编译成运动原语（仍经 Safety，不直连电机）：

```bash
export DEEPSEEK_API_KEY=sk-...

# A：桌面臂
cargo run -p archon-embodied-cli -- \
  --backend mujoco --model builtin:desktop_arm \
  --policy llm --instruction "向右边挥一下手" --viewer --step-ms 0

# B：平面小车
cargo run -p archon-embodied-cli -- \
  --backend mujoco --model builtin:diff_car \
  --policy llm --instruction "向前走一点再左转" --viewer --step-ms 0

# 录一段演示 MP4（需 ffmpeg；可不开 viewer）
brew install ffmpeg   # 若尚未安装
cargo run -p archon-embodied-cli -- \
  --backend mujoco --model builtin:diff_car \
  --policy instruction --instruction "向前走一点再左转180度" \
  --record-video ./tmp-episodes/car-demo.mp4 --step-ms 0
# 结束后看 stderr 里的 video saved 路径；也可用 --record-video 不带路径（写入 episode 目录 demo.mp4）
```

带 **Viewer 截图 / 前后帧对比 / 参考成片** 的逐步说明见示例文档：[`examples/diff-car-mujoco/`](../examples/diff-car-mujoco/)。

无 Key 时可用规则策略：`--policy instruction`（短语表，不调用云端）。录屏也可用 macOS「Cmd+Shift+5」对准 viewer 窗口。

| 变量 / 参数 | 说明 |
|-------------|------|
| `DEEPSEEK_API_KEY` / `--llm-api-key` | API Key |
| `LLM_BASE_URL` / `--llm-base-url` | 默认 `https://api.deepseek.com` |
| `LLM_MODEL` / `--llm-model` | 默认 `deepseek-chat` |

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

`archon-embodied` · `archon-runtime` · `archon-kinetic` · `archon-policy` · `archon-perception` · `archon-sim` · `archon-sim-bridge` · `archon-ros2` · `archon-embodied-cli`
