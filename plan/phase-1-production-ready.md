# Phase 1: 生产就绪

**目标版本**: v0.2.0
**目标日期**: 2026-05-08
**阶段时长**: 1-2 个月
**优先级**: 🔴 高

---

## 概述

Phase 1 的目标是让 Archon 达到生产环境可用的状态。这意味着用户可以安全、可靠地在真实项目中使用 Archon 进行日常开发工作。

## 关键交付物

### 1. MCP (Model Context Protocol) 客户端支持

**目标**: 让 Archon 能够连接外部数据源和工具

**具体任务**:
- [ ] 实现 MCP 客户端核心协议
- [ ] 支持 Stdio 传输层
- [ ] 支持 HTTP/SSE 传输层
- [ ] 实现工具发现和执行
- [ ] 实现资源订阅和通知
- [ ] 实现提示模板支持
- [ ] 添加 MCP 服务器配置管理 (`~/.archon/mcp.json`)

**验证标准**:
```bash
# 用户可以配置和使用 MCP 服务器
archon mcp add filesystem --command "npx -y @modelcontextprotocol/server-filesystem /path/to/allowed/dir"
archon mcp list
```

**参考**:
- [MCP 协议规范](https://modelcontextprotocol.io/)
- [MCP Python SDK](https://github.com/modelcontextprotocol/python-sdk)

---

### 2. 向量存储集成 (RAG)

**目标**: 让 Archon 具备长期记忆和文档检索能力

**具体任务**:
- [ ] 集成向量数据库客户端 (Chroma/Qdrant)
- [ ] 实现文档切分和嵌入生成
- [ ] 实现相似度搜索
- [ ] 添加文档管理命令
- [ ] 实现对话历史向量化
- [ ] 支持多模态嵌入 (文本+代码)

**验证标准**:
```rust
// Agent 可以检索相关文档
let context = rag.search("如何使用 bash 工具？", limit=5).await?;
session.add_context(context);
```

**技术选型**:
- **向量数据库**: Chroma (轻量) 或 Qdrant (高性能)
- **嵌入模型**: text-embedding-3-small (OpenAI) 或本地模型

---

### 3. 增强 Git 集成

**目标**: 让 Archon 能够更好地与 GitHub/GitLab 协作

**具体任务**:
- [ ] 实现 GitHub CLI 集成 (gh)
- [ ] 支持创建 Pull Request
- [ ] 支持查看和回复 PR 评论
- [ ] 支持创建和更新 Issues
- [ ] 支持代码审查建议
- [ ] 实现 Git 工作流自动化
- [ ] 支持 GitLab 集成

**验证标准**:
```bash
# Agent 可以自动创建 PR
archon pr create --title "修复登录bug" --body "修复了 #123"
```

---

### 4. 结构化日志系统

**目标**: 提高可观测性和调试能力

**具体任务**:
- [ ] 集成结构化日志框架 (tracing)
- [ ] 实现日志级别控制
- [ ] 添加性能指标收集
- [ ] 实现会话日志记录
- [ ] 支持日志导出 (JSON/CSV)
- [ ] 添加调试模式

**验证标准**:
```bash
# 结构化日志输出
RUST_LOG=debug archon --session-log session.json
```

---

## 发布标准 (Definition of Done)

Phase 1 完成需要满足以下条件：

- [ ] 所有 P0 任务完成
- [ ] 测试覆盖率达到 70%
- [ ] 文档完整（API 文档、用户指南）
- [ ] 无阻塞性 Bug
- [ ] 性能基准测试通过
- [ ] 社区反馈积极

## 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| MCP 协议变更 | 高 | 紧密跟踪 MCP 规范更新 |
| 向量存储性能 | 中 | 提供多种后端选择 |
| GitHub API 限制 | 中 | 实现请求节流和缓存 |

## 参考

- [主路线图](./README.md)
- [v0.1.0 里程碑](./milestone-v0.1.0.md) (已完成)
- [v0.2.0 里程碑](./milestone-v0.2.0.md) (下一阶段)

---

*文档版本: 1.0*
*最后更新: 2026-04-08*
*维护者: Archon Team*
