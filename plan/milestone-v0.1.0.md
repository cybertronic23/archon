# 里程碑: v0.1.0 - 基础功能发布

**发布日期**: 2026-04-08
**版本状态**: ✅ 已发布
**Git Tag**: `v0.1.0`

---

## 概述

v0.1.0 是 Archon 的初始版本，提供了基础的 AI Agent 功能，包括核心工具集、LLM 集成、会话管理和权限控制。

---

## 主要特性

### 🤖 核心架构
- ✅ 工具注册和执行系统 (ToolRegistry)
- ✅ 消息类型和流事件定义
- ✅ Agent 循环 (run_agent_loop)
- ✅ 权限控制框架
- ✅ 会话管理 (保存/加载)
- ✅ 上下文压缩 (maybe_compress)

### 🛠️ 内置工具
| 工具 | 功能 | 状态 |
|------|------|------|
| `read` | 读取文件内容 | ✅ |
| `bash` | 执行 shell 命令 | ✅ |
| `edit` | 精确字符串替换 | ✅ |
| `write` | 创建/覆盖文件 | ✅ |
| `glob` | 文件模式匹配 | ✅ |
| `grep` | 正则搜索 | ✅ |
| `web_fetch` | 网页内容获取 | ✅ v0.1.0 新增 |
| `web_search` | 网络搜索 | ✅ v0.1.0 新增 |

### 🧠 LLM 支持
- ✅ Anthropic API (Claude 系列)
- ✅ OpenAI 兼容 API (包括 DashScope)
- ✅ 流式响应 (SSE)
- ✅ 自动重试机制

### 🔐 安全与权限
- ✅ 风险分级 (Safe/Moderate/Dangerous)
- ✅ 交互式权限提示
- ✅ Docker 沙盒 (Off/Permissive/Strict 模式)
- ✅ `always_allow` 缓存

### 💾 会话管理
- ✅ 自动保存/恢复会话
- ✅ 历史记录 (rustyline)
- ✅ 配置文件支持 (`~/.archon/config.toml`)

---

## Bug 修复 (v0.1.0)

| Issue | 描述 | 修复文件 |
|-------|------|----------|
| #1 | Sandbox Permissive 模式错误禁用网络 | `sandbox.rs` |
| #2 | 权限分类缺少 `write` 工具 | `permission.rs` |

---

## 技术栈

- **语言**: Rust 2021 Edition
- **异步运行时**: Tokio
- **HTTP 客户端**: reqwest
- **LLM 协议**: SSE (Server-Sent Events)
- **容器化**: Docker (bollard)
- **配置**: TOML
- **CLI**: clap + rustyline

---

## 已知限制

1. **无 MCP 支持** - 无法连接外部数据源
2. **无向量存储** - 缺乏长期记忆能力
3. **无多 Agent 协作** - 只能单 Agent 运行
4. **无 LSP 集成** - 缺少代码补全和诊断
5. **网络依赖** - 编译和运行需要网络连接

---

## 升级指南

### 从 pre-0.1.0 升级

1. 备份现有会话:
   ```bash
   cp -r ~/.archon/sessions ~/.archon/sessions.backup
   ```

2. 更新代码:
   ```bash
   git pull origin main
   ```

3. 重新编译:
   ```bash
   cargo build --release
   ```

4. 更新配置文件（如需要）:
   ```bash
   # 新增 web 工具相关配置
   ```

---

## 下一步 (v0.2.0 预览)

- [ ] MCP 客户端支持
- [ ] 向量存储集成 (Chroma/Qdrant)
- [ ] 增强 GitHub 集成 (PR、Issues)
- [ ] 结构化日志系统
- [ ] 性能优化

---

## 参考

- [完整路线图](./README.md)
- [Phase 1: 生产就绪](./phase-1-production-ready.md)
- [开发指南](../../docs/development.md)
- [API 文档](../../docs/api.md)

---

*发布日期: 2026-04-08*
*维护者: Archon Team*
*许可证: MIT*
