# Archon

Archon（希腊语"统治者"）是一个用 Rust 从零构建的 Agent Harness 框架，灵感来自 Claude Code 的架构设计。项目目标是实现一个最小但完整的 AI Agent 运行时：接收用户输入、流式调用 LLM、解析并执行 tool_use、将结果回传 LLM、循环直到任务完成。

## 为什么做这个项目

Claude Code 展示了一种强大的交互范式——LLM 不再只是问答，而是一个能操作文件系统、执行命令、编辑代码的 Agent。但它是闭源的。我们想用 Rust 复刻这个核心循环，理解其中的工程取舍，并为后续扩展打下基础。

核心问题只有一个：**如何让 LLM 可靠地驱动一个 tool_use 循环？**

答案是一个极简的 while 循环：调用 LLM → 如果 stop_reason 是 tool_use 就执行工具并把结果喂回去 → 如果是 end_turn 就退出。所有复杂性都围绕这个循环展开。

## 技术栈

| 层级 | 选型 | 理由 |
|------|------|------|
| 语言 | **Rust** | 零成本抽象、async/await 原生支持、适合长期维护的 CLI 工具 |
| 异步运行时 | **Tokio** | Rust 生态事实标准，提供 process spawning、timer、channel |
| HTTP 客户端 | **reqwest** (streaming) | 支持 SSE 字节流，与 tokio 深度集成 |
| 序列化 | **serde / serde_json** | Rust JSON 处理的唯一选择 |
| CLI 框架 | **clap** (derive) | 声明式参数解析，支持环境变量回退 |
| 错误处理 | **anyhow + thiserror** | anyhow 用于应用层面传播，thiserror 用于库级别定义 |
| Trait 抽象 | **async-trait** | 在 trait 中使用 async fn（Rust 原生支持仍不稳定） |
| 流处理 | **futures** | `Stream` trait、`BoxStream`、`StreamExt` 组合子 |

没有前端。这是一个纯终端应用，Phase 1 直接用 stdin/stdout 做 REPL。

## 项目结构

```
archon/
├── Cargo.toml                     # Workspace，统一管理依赖版本
├── CLAUDE.md                      # Claude Code 项目指令
├── ARCHITECTURE.md                # 详细架构设计文档
│
└── crates/
    ├── archon-core/               # 内核：不依赖其他内部 crate
    │   ├── types.rs               #   Message, ContentBlock, StreamEvent 等所有共享类型
    │   ├── tool.rs                #   Tool trait + ToolRegistry（HashMap 派发）
    │   ├── session.rs             #   Session 会话历史管理（JSON 持久化）
    │   ├── agent_loop.rs          #   StreamProvider trait + run_agent_loop() 核心循环
    │   ├── permission.rs          #   RiskLevel 分级 + PermissionHandler trait
    │   └── context.rs             #   ContextConfig + 消息压缩/摘要逻辑
    │
    ├── archon-llm/                # LLM 通信层
    │   ├── provider.rs            #   Provider trait（多模型抽象）
    │   ├── streaming.rs           #   SSE 事件解析器：event type + JSON → StreamEvent
    │   ├── anthropic.rs           #   AnthropicProvider：HTTP POST + SSE 流处理
    │   ├── openai.rs              #   OpenAIProvider：OpenAI / Dashscope 兼容端点
    │   └── retry.rs               #   指数退避重试（429、5xx、Retry-After）
    │
    ├── archon-tools/              # 工具实现
    │   ├── read.rs                #   ReadTool —— 读文件，cat -n 格式输出
    │   ├── bash.rs                #   BashTool —— 执行 shell 命令，可配置超时
    │   ├── edit.rs                #   EditTool —— 精确字符串替换，唯一性校验
    │   ├── write.rs               #   WriteTool —— 创建/覆写文件
    │   ├── glob.rs                #   GlobTool —— 按 glob 模式查找文件
    │   ├── grep.rs                #   GrepTool —— 正则搜索文件内容
    │   └── sandbox.rs             #   DockerSandbox —— 三种沙箱模式（Off/Permissive/Strict）
    │
    └── archon-cli/                # 二进制入口
        ├── main.rs                #   clap 参数解析 → 组装组件 → REPL 循环
        └── permission.rs          #   交互式权限处理（终端提示 + "always allow"）
```

四个 crate 的依赖方向：`cli → {core, llm, tools}`，`llm → core`，`tools → core`。core 是零依赖的基座。

## 设计思路

### 1. 从 Agent Loop 反推架构

整个项目的起点不是"我要写哪些模块"，而是"Agent Loop 需要什么"。答案是三样东西：

- 一个能流式返回 `StreamEvent` 的 **LLM Provider**
- 一个能按名字查找并执行工具的 **Tool Registry**
- 一个能追加消息的 **Session**

所以就有了三个核心抽象。把它们拆成独立 crate 是因为它们的变化频率不同——工具会频繁增加，Provider 偶尔增加，Session 和 Loop 基本稳定。

### 2. 流式输出是第一优先级

用户体验的关键在于 LLM 的文本逐字流出，而不是等待整个响应完成。这意味着我们必须增量处理 SSE 事件：

```
SSE 字节流 → 按行缓冲 → 解析 event/data → StreamEvent → agent loop 消费
                                                              ↓
                                                     TextDelta → 立即 print!
                                                     InputJsonDelta → 静默累积
```

`TextDelta` 到达时立即 `print!` + `flush`，用户就能看到打字效果。`InputJsonDelta`（工具参数的 JSON 碎片）则静默累积，直到 `ContentBlockStop` 时一次性反序列化。

### 3. Tool 设计遵循 "输入 JSON，输出 String" 原则

所有工具的接口统一为：

```rust
async fn execute(&self, input: serde_json::Value) -> Result<String>
```

输入是 LLM 生成的 JSON（与 API 的 tool_use.input 对应），输出是纯文本字符串（塞进 ToolResult.content）。这个设计够简单，也够用——LLM 能理解纯文本结果，而工具不需要关心上下游的类型。

错误处理上，工具执行失败不会炸掉整个 agent loop。失败的结果会以 `is_error: true` 的 ToolResult 回传给 LLM，让它自己决定是重试还是换个方案。这模拟了 Claude Code 的行为——工具出错是常态，Agent 需要有应对能力。

### 4. Session 遵循 Anthropic API 的消息协议

一个容易踩坑的点：**工具结果必须放在 User 角色的消息里**，而不是 Assistant 消息。所以 Session 提供了三个类型化的 push 方法：

- `push_user(text)` —— 用户输入
- `push_assistant(blocks)` —— LLM 响应（可能包含 Text + ToolUse）
- `push_tool_results(results)` —— 工具执行结果（作为 User 消息发送）

消息序列严格遵循 User → Assistant → User → Assistant 的交替模式。

### 5. 为什么是 Workspace 而不是单 crate

单 crate 完全能实现同样的功能，但 workspace 有几个好处：

- **编译隔离**：改一个工具只重编译 `archon-tools`，不碰 LLM 层
- **依赖约束**：`archon-core` 不能依赖 `reqwest`，这在 crate 边界上被强制执行
- **可替换性**：未来可以只替换 `archon-llm` 来支持 OpenAI，或只替换 `archon-tools` 来定制工具集

## 快速开始

```bash
# 构建
cargo build

# 运行（需要 Anthropic API Key）
cargo run -p archon-cli -- --api-key $ANTHROPIC_API_KEY

# 或通过环境变量
export ANTHROPIC_API_KEY=sk-ant-...
cargo run -p archon-cli

# 指定模型和 max_tokens
cargo run -p archon-cli -- --model claude-sonnet-4-20250514 --max-tokens 4096
```

进入 REPL 后，可以尝试：

```
You> Read the file Cargo.toml
You> Run `ls -la`
You> Create a file hello.txt with content 'hello world', then read it back
```

最后一个例子会触发多轮 tool_use 循环——LLM 先调用 Bash 写文件，再调用 Read 读回来。

## 已完成功能

| 类别 | 功能 | 说明 |
|------|------|------|
| **Agent Loop** | 流式 tool_use 循环 | TextDelta 逐字输出，ToolUse JSON 静默累积，循环直到 end_turn |
| **LLM 提供者** | Anthropic Provider | SSE 流式调用，支持自定义 base URL |
| | OpenAI Provider | 兼容 OpenAI / Dashscope（阿里云）端点 |
| | 指数退避重试 | 自动处理 429、5xx 错误，尊重 Retry-After header |
| **工具（6个）** | ReadTool | 读文件，支持 offset/limit，cat -n 格式输出 |
| | BashTool | 执行 shell 命令，可配置超时，工作目录跨命令持久化 |
| | EditTool | 精确字符串替换，唯一性校验 |
| | WriteTool | 创建/覆写文件，自动创建父目录 |
| | GlobTool | 按 glob 模式查找文件（如 `**/*.rs`） |
| | GrepTool | 正则搜索文件内容，自动跳过二进制和隐藏目录 |
| **权限系统** | 三级风险分级 | Safe / Moderate / Dangerous，交互式终端提示 |
| | "Always Allow" | 用户可选择对某工具永久授权，减少重复确认 |
| **沙箱** | Docker 沙箱 | Off / Permissive（禁网络）/ Strict（只读、限资源） |
| **上下文管理** | 自动消息压缩 | 接近 context window 上限时自动压缩历史消息 |
| **Session** | 会话持久化 | JSON 格式自动保存/恢复，支持 `--resume` |
| **REPL** | 多行输入 | 行尾 `\` 触发续行 |
| | 命令历史 | 持久化到 `~/.archon/history` |
| **工具并行** | 并发执行 | 同一轮中多个独立 tool_use 通过 `join_all` 并发执行 |
| **测试** | 57 个测试 | 覆盖 core（20）、llm（11）、tools（26）及沙箱集成测试 |

## 后续规划

### P0 — 更多工具

- **LSP Tool** — 代码定义跳转、引用查找、hover 类型信息（类似 Claude Code 的 LSP 工具）
- **WebFetch Tool** — 抓取网页内容，HTML 转 Markdown
- **WebSearch Tool** — 网络搜索能力
- **NotebookEdit Tool** — Jupyter Notebook 单元格编辑支持

### P1 — 终端 UI 增强

- **Markdown 渲染** — 用 `termimad` 或 `comrak` 渲染 LLM 输出中的 Markdown
- **代码语法高亮** — 用 `syntect` 对代码块做高亮显示
- **Spinner / 进度条** — 等待 LLM 响应时显示加载动画
- **彩色输出** — 区分用户输入、LLM 输出、工具调用的颜色

### P2 — Agent 能力增强

- **Tool 输出截断** — 大文件 / 大输出自动截断，防止撑爆上下文窗口
- **Token 用量统计** — 实时显示每轮对话的 token 消耗和累计费用
- **项目指令加载** — 支持从 `CLAUDE.md` 类文件自动加载 system prompt
- **Tool 依赖解析** — 工具间存在依赖时按拓扑序执行，而非全并行

### P3 — Session 管理增强

- **命名 Session** — 支持 `--resume <name>` 恢复指定会话
- **Session 列表** — 查看和管理历史会话列表
- **对话分支 / 回滚** — 回退到上一轮重新提问

### P4 — 配置与扩展

- **配置文件** — `~/.archon/config.toml` 持久化默认参数（模型、Provider、沙箱模式等）
- **自定义工具加载** — 从外部配置 / 脚本动态注册工具
- **GrepTool skip 目录可配置** — 当前跳过目录是硬编码的

### P5 — 安全与可靠性

- **统一超时管理** — 所有工具统一的超时和取消机制
- **细粒度权限规则** — 基于路径 / 命令模式的白名单（如允许 `git status` 但拦截 `git push`）
- **Audit Log** — 记录所有工具执行历史，便于审计和回溯

### P6 — 多 Agent 协作（高级）

- **Sub-agent 支持** — 主 Agent 可 spawn 子 Agent 并行处理子任务
- **Team / Swarm 模式** — 多个 Agent 通过共享 Task List 协作完成复杂任务
