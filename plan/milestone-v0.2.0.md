# 里程碑: v0.2.0 - MCP 与 RAG 支持

**目标版本**: v0.2.0
**目标日期**: 2026-05-08
**阶段**: Phase 1
**优先级**: 🔴 高

---

## 概述

v0.2.0 的核心目标是让 Archon 能够连接外部世界，通过 MCP (Model Context Protocol) 协议与各种数据源和工具集成，同时引入向量存储支持，赋予 Archon 长期记忆能力。

---

## 关键交付物

### 1. MCP 客户端支持

**目标**: 实现 MCP 协议客户端，允许 Archon 连接和使用 MCP 服务器

**详细任务**:
- [ ] **MCP-001**: 研究 MCP 协议规范
  - 阅读 [MCP 协议文档](https://modelcontextprotocol.io/)
  - 理解生命周期、消息格式、传输层
  - 预估工作量: 2 天

- [ ] **MCP-002**: 实现 MCP 核心类型
  - 定义 JSON-RPC 消息结构
  - 实现工具、资源、提示的类型定义
  - 预估工作量: 3 天

- [ ] **MCP-003**: 实现 Stdio 传输层
  - 启动和管理 MCP 服务器进程
  - 处理 stdin/stdout 通信
  - 实现优雅关闭
  - 预估工作量: 4 天

- [ ] **MCP-004**: 实现 HTTP/SSE 传输层
  - 支持远程 MCP 服务器
  - 实现 SSE 事件流处理
  - 支持认证
  - 预估工作量: 4 天

- [ ] **MCP-005**: 集成到 Archon
  - 在 ToolRegistry 中支持 MCP 工具
  - 实现资源订阅和通知
  - 添加 MCP 配置管理
  - 预估工作量: 5 天

**验收标准**:
```bash
# 配置 MCP 服务器
archon mcp add filesystem --command "npx -y @modelcontextprotocol/server-filesystem /home/user/docs"

# 列出可用的 MCP 工具
archon mcp list-tools

# 使用 MCP 工具
archon chat "请读取 /home/user/docs/readme.md 的内容"
```

**依赖**: 无
**阻塞**: MCP-006 (文档)

---

### 2. 向量存储集成 (RAG)

**目标**: 添加向量存储支持，实现文档检索和长期记忆

**详细任务**:
- [ ] **RAG-001**: 调研向量数据库选项
  - 评估 Chroma (轻量)、Qdrant (高性能)、Milvus (分布式)
  - 确定默认选项
  - 预估工作量: 2 天

- [ ] **RAG-002**: 实现嵌入生成
  - 集成 OpenAI 嵌入 API
  - 支持本地嵌入模型 (如 sentence-transformers)
  - 实现批处理
  - 预估工作量: 4 天

- [ ] **RAG-003**: 实现向量存储客户端
  - 抽象向量存储接口
  - 实现 Chroma 后端
  - 实现 Qdrant 后端
  - 预估工作量: 5 天

- [ ] **RAG-004**: 实现文档管理
  - 文档切分策略 (按段落、按 Token)
  - 元数据提取和存储
  - 重复检测
  - 预估工作量: 4 天

- [ ] **RAG-005**: 实现检索和提示增强
  - 相似度搜索
  - 重排序 (reranking)
  - 上下文注入
  - 预估工作量: 4 天

- [ ] **RAG-006**: 集成到 Archon
  - 添加 `rag` 命令
  - 实现文档索引工作流
  - 在对话中自动使用 RAG
  - 预估工作量: 5 天

**验收标准**:
```bash
# 索引文档目录
archon rag index /path/to/docs --collection my-project

# 搜索相关文档
archon rag search "如何处理错误？" --collection my-project

# 在对话中使用
archon chat "根据文档，如何配置数据库？"
```

**依赖**: 无
**阻塞**: RAG-007 (文档)

---

### 3. 增强 Git 集成

**目标**: 改进与 GitHub/GitLab 的集成

**详细任务**:
- [ ] **GIT-001**: 集成 GitHub CLI
  - 检测和使用 `gh` 命令
  - 实现认证管理
  - 预估工作量: 3 天

- [ ] **GIT-002**: 实现 PR 管理
  - 创建 PR
  - 查看 PR 列表和详情
  - 更新 PR
  - 预估工作量: 4 天

- [ ] **GIT-003**: 实现 Issue 管理
  - 创建 Issue
  - 查看和搜索 Issue
  - 更新 Issue 状态
  - 预估工作量: 3 天

- [ ] **GIT-004**: 实现代码审查
  - 查看 PR diff
  - 添加评论
  - 批准/请求修改
  - 预估工作量: 4 天

**验收标准**:
```bash
# 创建 PR
archon pr create --title "修复登录 bug" --body "修复了 #123"

# 查看 PR
archon pr view 456

# 创建 Issue
archon issue create --title "功能请求" --label enhancement
```

**依赖**: GIT-001
**阻塞**: 无

---

### 4. 结构化日志系统

**目标**: 提高可观测性和调试能力

**详细任务**:
- [ ] **LOG-001**: 集成 tracing 框架
  - 添加 tracing 依赖
  - 配置日志级别
  - 实现格式化输出
  - 预估工作量: 3 天

- [ ] **LOG-002**: 实现性能指标
  - 跟踪 LLM 调用延迟
  - 跟踪工具执行时间
  - 内存使用监控
  - 预估工作量: 4 天

- [ ] **LOG-003**: 实现会话日志
  - 记录完整对话历史
  - 支持导出 JSON/CSV
  - 隐私保护（敏感信息脱敏）
  - 预估工作量: 3 天

- [ ] **LOG-004**: 添加调试模式
  - 详细请求/响应日志
  - 工具调用追踪
  - 状态机转换记录
  - 预估工作量: 2 天

**验收标准**:
```bash
# 启用调试日志
RUST_LOG=debug archon chat "Hello"

# 导出会话日志
archon session export --format json > session.json

# 查看性能指标
archon metrics
```

**依赖**: 无
**阻塞**: 无

---

## 发布检查清单

- [ ] 所有 P0 任务完成
- [ ] 代码审查通过
- [ ] 测试覆盖率 ≥ 70%
- [ ] 文档完整
- [ ] 性能基准测试通过
- [ ] 无阻塞性 Bug
- [ ] 发布说明完成
- [ ] Git Tag 创建
- [ ] crates.io 发布（可选）

---

## 时间线

```
Week 1-2:  MCP 核心实现
Week 3-4:  RAG 基础功能
Week 5-6:  Git 集成
Week 7-8:  日志系统 + 测试/文档
```

---

*最后更新: 2026-04-08*
*维护者: Archon Team*
