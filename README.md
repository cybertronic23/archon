# Archon

Archon（希腊语「统治者」）是用 Rust 构建的 **Agent 运行时仓库**，包含两个平面：

| 平面 | 定位 | 状态（本分支） |
|------|------|----------------|
| **Embodied** | 具身原生 Agent OS：观测 → 策略提案 → 安全校验 → 执行 → Episode | **主线 / workspace 默认可构建** |
| **Digital** | 对标 Claude Code 的 LLM tool_use Harness（读改文件、bash、REPL） | 源码保留；workspace 成员暂注释，需自行重新启用 |

本分支（`dev-physical-ai`）优先验证：**模型无关的具身闭环**（可接 VLA / WAM / World Model / 经典策略），仿真先行、ROS2 话题契约对齐真机；**不是**把 LLM function calling 直接接到电机上。

用户文档：[`docs/`](docs/)（如 [具身快速开始](docs/embodied-getting-started.md)）。  
开发沉淀：本地 `notes/`（暂未纳入版本库）。

---

## 两个核心循环

**具身（本分支默认）**

```
Observation → Policy.propose → SafetyGate → Chronos → RobotBackend → Episode
```

单机器人只有一个 Executive（执行权威）；急停 / 抢占由 Runtime 仲裁。

**数字（源码仍在 `crates/archon-{core,llm,tools,cli}`）**

```
User → LLM stream → tool_use → ToolRegistry → 结果回传 → 循环至 end_turn
```

---

## 快速开始（具身）

```bash
# M0：进程内 sim
cargo run -p archon-embodied-cli -- --task-id demo_waypoints --step-ms 0

# M1：合成相机 + ColorBlobPolicy
cargo run -p archon-embodied-cli -- \
  --task-id pick_red_blob_sim --policy color_blob --camera synthetic --step-ms 0

# 带窗口看动作（结束后窗口会停住，关掉窗口才退出）
cargo run -p archon-embodied-cli -- \
  --backend mujoco --model builtin:desktop_arm \
  --policy instruction --instruction "挥手" \
  --viewer --step-ms 0

# 平面小车 + 导出 MP4（需 ffmpeg）
cargo run -p archon-embodied-cli -- \
  --backend mujoco --model builtin:diff_car \
  --policy instruction --instruction "向前走一点再左转180度" \
  --record-video ./tmp-episodes/car-demo.mp4 --step-ms 0

# 多轮 TUI（会话常驻，边聊边指挥）
cargo run -p archon-embodied-cli -- \
  --backend mujoco --model builtin:diff_car \
  --policy instruction --viewer --tui --step-ms 0

# 中途抢占
cargo run -p archon-embodied-cli -- --step-ms 5 --auto-stop-ms 200
```

更多参数与 ROS2 话题约定见 [docs/embodied-getting-started.md](docs/embodied-getting-started.md)。  
带贴图的小车逐步示例：[examples/diff-car-mujoco/](examples/diff-car-mujoco/)。

### 数字平面 CLI（当前默认 workspace 未包含）

数字 crates 仍在树中。若要重新编进 workspace，在根 [`Cargo.toml`](Cargo.toml) 取消注释 `archon-core` / `archon-llm` / `archon-tools` / `archon-cli` 后：

```bash
cargo run -p archon-cli -- --api-key $ANTHROPIC_API_KEY
# 或 OPENAI_API_KEY / DASHSCOPE_API_KEY，见 archon-cli 参数
```

---

## 项目结构

```
archon/
├── Cargo.toml                 # Workspace（默认仅具身 members）
├── docs/                      # 面向用户的稳定文档
├── examples/                  # 带贴图的可运行示例
├── notes/                     # 开发者设计/评估/讨论（本地，gitignore）
├── ARCHITECTURE.md            # 数字 Harness 架构说明（偏 Digital）
│
└── crates/
    # —— Embodied（本分支主线）——
    ├── archon-embodied/       # 类型与 Policy / SafetyGate / RobotBackend
    ├── archon-runtime/        # Executive、事件总线、资源锁、Arbiter
    ├── archon-kinetic/        # Chronos 插值、关节限位
    ├── archon-policy/         # MockPolicy、ColorBlobPolicy、LimitSafetyGate、VLA/WAM stub
    ├── archon-perception/     # CameraSource、合成相机、色块检测、Observation enrich
    ├── archon-ros2/           # 话题/消息契约（sim↔real）
    ├── archon-sim/            # 进程内仿真 Backend（CI 默认）
    ├── archon-sim-bridge/     # NDJSON 桥：MuJoCo / ManiSkill / …
    ├── archon-embodied-cli/   # 二进制 archon-embodied
    │
    # —— Digital（源码保留，默认未加入 workspace）——
    ├── archon-core/           # Session、Tool、agent_loop、权限、上下文压缩
    ├── archon-llm/            # Anthropic / OpenAI 流式客户端
    ├── archon-tools/          # read/bash/edit/write/glob/grep/web_*、沙箱
    └── archon-cli/            # 数字 REPL 入口
```

---

## 技术栈（共用）

| 层级 | 选型 |
|------|------|
| 语言 | Rust |
| 异步 | Tokio |
| 序列化 | serde / serde_json |
| CLI | clap |
| 错误 | anyhow / thiserror |
| 具身 HAL | ROS2 话题契约（Rust 侧先 trait + sim；真机后续接 r2r 等） |

---

## 已完成能力（摘要）

### Embodied（M0 + M1）

- 统一类型：`Observation`（`proprio` + `modalities` + `annotations`）/ `ActionProposal` / `Episode`
- `MockPolicy` + `ColorBlobPolicy`（vision-gated）+ 关节限位 `SafetyGate` + Chronos（≥50Hz）
- `archon-perception`：合成 RGB、色块检测、`images.primary` MediaRef
- `SimBackend`（与 `/archon/arm/*` 话题契约对齐）
- Executive：感知 enrich、资源锁、事件抢占、Episode + media bundle
- CLI：`archon-embodied`（`--policy` / `--camera` / `--backend sim|mujoco`）
- M2a：`archon-sim-bridge` + Python MuJoCo worker（平台无关 NDJSON）

### Digital（源码能力；启用 workspace 后可用）

- 流式 tool_use 循环、Anthropic / OpenAI 兼容 Provider、重试
- 工具：read / bash / edit / write / glob / grep / web_fetch / web_search
- 权限分级、Docker 沙箱、会话持久化、上下文压缩、工具并行

数字平面细节见 [ARCHITECTURE.md](ARCHITECTURE.md)。

---

## 后续规划（具身优先）

| 里程碑 | 内容 |
|--------|------|
| **M0** | ✅ MockPolicy + SimBackend + Executive + Episode |
| **M1** | ✅ 感知桥接；`images.primary` + ColorBlobPolicy 视觉门控 |
| **M2a** | ✅ 平台无关仿真桥 + MuJoCo（资产 / viewer / 语言原语） |
| **M2b** | ⏳ ManiSkill worker（可置于 M2d 之后） |
| **M2c** | ✅ DeepSeek 等 LLM 编译自然语言 → 原语；平面小车 `diff_car` |
| **M2d** | 🚧 会话常驻 + ratatui TUI 多轮语言指挥（`--tui`；网页 Chat → 以后 M2e） |
| **M3** | Isaac Sim（次优先） |
| **M4** | Gazebo 等其它平台 |
| **M5** | 真机 SO101 / Microduck 等 `RobotBackend` |
| **M6** | `VlaAdapter` + 低频 action chunk |

数字平面原有路线图（MCP、RAG、富 UI 等）仍参考 `plan/`；与具身主线并行，不互相替代。

---

## 设计原则（具身）

1. **模型不是系统中心** — Harness 统一接纳不同粒度的行为提案，再校验与执行。  
2. **环境反馈驱动闭环** — 不是文本回合驱动的 tool_use 套壳。  
3. **安全与仲裁确定性** — LLM/策略可提议，不能绕过 Safety / Arbiter / 资源锁。  
4. **仿真与真机同契约** — 换 Backend，不换 Executive。
