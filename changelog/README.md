# Archon Changelog

本文档记录 Archon 的所有版本变更历史。

## 版本格式

我们使用 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/) 格式，并遵循 [语义化版本](https://semver.org/lang/zh-CN/) (SemVer)。

### 变更类型

- `Added` - 新功能
- `Changed` - 现有功能的变更
- `Deprecated` - 即将移除的功能
- `Removed` - 已移除的功能
- `Fixed` - 错误修复
- `Security` - 安全相关的修复

## 版本索引

| 版本 | 发布日期 | 状态 | 关键特性 |
|------|----------|------|----------|
| [v0.1.0](./v0.1.0.md) | 2026-04-08 | ✅ 已发布 | 基础功能、核心工具、Web 工具 |
| v0.2.0 | 2026-05-08 | 🚧 开发中 | MCP 支持、RAG、增强 Git 集成 |
| v0.3.0 | 2026-06-08 | 📋 规划中 | 任务工作流、多 Agent 协作 |
| v0.4.0 | 2026-07-08 | 📋 规划中 | LSP 集成、高级开发工具 |
| v1.0.0 | 2026-09-08 | 🎯 目标 | 生产就绪、完整生态 |

## 最新版本

### v0.1.0 (2026-04-08)

**✨ 新增功能**
- 核心 Agent 架构和工具系统
- 8 个内置工具 (read, write, edit, bash, glob, grep, web_fetch, web_search)
- Anthropic、OpenAI、DashScope LLM 支持
- Docker 沙盒 (Off/Permissive/Strict)
- 会话管理和配置文件支持

**🐛 错误修复**
- 修复 Sandbox Permissive 模式网络禁用问题
- 修复权限分类缺少 write 工具问题

查看完整变更日志：[v0.1.0.md](./v0.1.0.md)

## 如何升级

### 使用 Cargo

```bash
# 安装/更新到最新版本
cargo install archon-cli

# 安装特定版本
cargo install archon-cli --version 0.1.0
```

### 从源码

```bash
git clone https://github.com/your-repo/archon.git
cd archon
git checkout v0.1.0
cargo build --release
```

## 迁移指南

### 从 pre-0.1.0 迁移

1. 备份配置和会话:
   ```bash
   cp -r ~/.archon ~/.archon.backup
   ```

2. 更新配置文件格式（如有变化）

3. 重新编译安装

## 废弃通知

暂无废弃功能。

## 安全公告

| 版本 | 漏洞 | 严重程度 | 修复版本 |
|------|------|----------|----------|
| 无 | - | - | - |

## 贡献者

感谢所有为 Archon 做出贡献的人！

- [贡献者列表](../../CONTRIBUTORS.md)

## 反馈与支持

- 🐛 [提交 Bug](https://github.com/your-repo/archon/issues/new?template=bug_report.md)
- 💡 [功能建议](https://github.com/your-repo/archon/issues/new?template=feature_request.md)
- 💬 [Discussions](https://github.com/your-repo/archon/discussions)

---

*最后更新: 2026-04-08*
*维护者: Archon Team*
*许可证: MIT*
