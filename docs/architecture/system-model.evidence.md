# 系统模型证据索引

对应模型：[neverwork-architecture.md](neverwork-architecture.md) · [system-context.dot](system-context.dot) · [system-containers.dot](system-containers.dot)

所有 `sourceRefs` 均相对 `NeverWork/`。置信度：high = 代码/配置直接确认；medium = 多处部分信号一致；low = 由命名或结构推断。

## 节点

| id | label | type | state | sourceRefs | confidence |
| --- | --- | --- | --- | --- | --- |
| app.bin | NeverWork.exe（bin neverwork） | container | current | `crates/app/Cargo.toml`；`crates/app/src/main.rs` | high |
| app.state | AppState 状态机 | module | current | `crates/app/src/state.rs:347-622` | high |
| app.views | views/（19 个视图） | module | current | `crates/app/src/views/mod.rs:5-22` | high |
| app.editor | 编辑内核（TextBuffer/输入/键位/查找/折叠） | module | current | `crates/app/src/{text_buffer,input_handler,keymap,find,folding,highlight,line_cache}.rs` | high |
| app.integration | 本地集成（fs/git/artifacts/markdown/marketplace） | module | current | `crates/app/src/{filesystem,git,artifacts,markdown,marketplace,skills_ops,settings_ops}.rs` | high |
| app.persist | 会话落盘（SessionStore 调用点） | module | current | `crates/app/src/state.rs`（`save_sessions`/`autosave_sessions`）；`crates/core/src/sessions.rs` | high |
| agent.crate | neverwork-agent | container | current | `crates/agent/Cargo.toml`；`crates/agent/src/lib.rs:6-33` | high |
| agent.runner | spawn_agents_chain | module | current | `crates/app/src/state.rs:4315`、`:4063`、`:4084`、`:4116` | high |
| agent.loop | agent_loop | module | current | `crates/agent/src/agent_loop.rs:620-695`、`:542-600` | high |
| agent.providers | Provider 适配（openai/anthropic/gemini/sse） | module | current | `crates/agent/src/lib.rs:11-15`、`:195-255` | high |
| agent.tools | tools + ToolContext | module | current | `crates/agent/src/tools.rs:259-290`、`:1-40` | high |
| agent.rag | rag（BM25 检索） | module | current | `crates/agent/src/rag.rs:1-40`；`crates/agent/src/tools.rs:1488-1489` | high |
| agent.web | web（必应 RSS / 抓取） | module | current | `crates/agent/src/web.rs:1-46` | high |
| agent.mcp | MCP stdio 客户端 | module | current | `crates/agent/src/mcp.rs:1-42` | high |
| agent.team | team + supervisor | module | current | `crates/agent/src/team.rs`；`crates/agent/src/supervisor.rs`；app 侧仅 `run_agent_in_team`（`state.rs:4404`） | high（存在）/ medium（运行时未用） |
| core.crate | neverwork-core | container | current | `crates/core/Cargo.toml`；`crates/core/src/lib.rs:20-29` | high |
| core.model | 领域模型 | module | current | `crates/core/src/model.rs:1-60` | high |
| core.settings | Settings / ProviderEntry / McpServerConfig | module | current | `crates/core/src/settings.rs` | high |
| core.sessions | SessionStore | module | current | `crates/core/src/sessions.rs:1-40` | high |
| core.ssh | SshTarget / RemoteFs | module | current | `crates/core/src/ssh.rs` | high |
| core.config | Config 与 config_dir | module | current | `crates/core/src/config.rs:1-42` | high |
| ui.crate | neverwork-ui | container | current | `crates/ui/Cargo.toml`；`crates/ui/src/lib.rs` | high |
| ext.llm | LLM 厂商 API | external-system | current | `crates/agent/src/{openai,anthropic,gemini}.rs` | high |
| ext.fs | 本地工作区文件系统 + git + watcher | external-system | current | `crates/app/src/{filesystem,git}.rs`；`notify` 依赖（`crates/app/Cargo.toml:26`） | high |
| ext.sshhost | 远程 SSH 主机 | external-system | current | `crates/core/src/ssh.rs`；`crates/agent/src/tools.rs`（`run_command` 的 UNSUPPORTED 分支之外） | high |
| ext.mcp | MCP server 子进程 | external-system | current | `crates/agent/src/mcp.rs:14-19` | high |
| ext.market | 市场源（Registry/腾讯云/skills.sh） | external-system | current | `crates/app/src/marketplace.rs:24-38` | high |
| ext.web | 必应 RSS / 公网站点 | external-system | current | `crates/agent/src/web.rs:20-35` | high |
| store.configdir | ~/.neverwork | database（文件存储） | current | `crates/core/src/config.rs:29-42`；`crates/core/src/sessions.rs:27-29` | high |

## 边

| id | from → to | type | sync | protocol | sourceRefs | confidence |
| --- | --- | --- | --- | --- | --- | --- |
| e.ui.spawn | app.state → agent.runner | calls | sync | in-process | `state.rs:4357-4361` | high |
| e.ui.events | agent.loop → app.state | publishes | async | flume | `state.rs:4469-4484`；`agent_loop.rs:542-600` | high |
| e.ui.approval | agent.loop ↔ app.state | calls | sync（阻塞） | flume bounded | `agent_loop.rs:556-570`；`state.rs:4573-4603`（PendingApproval）、`PendingAsk` | high |
| e.loop.provider | agent.loop → agent.providers | calls | sync | in-process | `agent_loop.rs:620-635`；`lib.rs:195-220` | high |
| e.provider.llm | agent.providers → ext.llm | calls | sync streaming | HTTPS/SSE | `crates/agent/src/{openai,anthropic,gemini}.rs` | high |
| e.loop.tools | agent.loop → agent.tools | calls | sync | in-process | `agent_loop.rs:14`、工具执行分支 | high |
| e.tools.fs | agent.tools → ext.fs | reads/writes | sync | file I/O | `tools.rs:259-290` | high |
| e.tools.ssh | agent.tools → ext.sshhost | reads/writes | sync | ssh | `tools.rs:265-266`（`remote: Option<RemoteFs>`） | high |
| e.tools.mcp | agent.tools → agent.mcp → ext.mcp | calls | sync | JSON-RPC/stdio | `mcp.rs:1-12`、`PREFIX` 路由 | high |
| e.tools.rag | agent.tools → agent.rag | calls | sync | in-process | `tools.rs:1488-1489` | high |
| e.tools.web | agent.tools → agent.web → ext.web | calls | sync | HTTPS | `web.rs:1-12` | high |
| e.state.store | app.persist → store.configdir | writes | sync | file（tmp+rename） | `sessions.rs:1-7`；`state.rs::save_sessions` | high |
| e.state.market | app.integration → ext.market | calls | sync | HTTPS | `marketplace.rs:24-38` | high |
| e.state.git | app.integration → ext.fs | reads | sync | 子进程 `git status` | `crates/app/src/git.rs` | high |
| e.state.theme | app.views → ui.crate | depends-on | sync | in-process | `crates/app/Cargo.toml`；`views/mod.rs:30` | high |

## 静态依赖（crate 层）

| from | to | 证据 |
| --- | --- | --- |
| neverwork(app) | neverwork-core / neverwork-ui / neverwork-agent | `crates/app/Cargo.toml:17-20` |
| neverwork-agent | neverwork-core | `crates/agent/Cargo.toml:8` |
| neverwork-ui | neverwork-core | `crates/ui/Cargo.toml:9` |
| neverwork-core | （无内部依赖） | `crates/core/Cargo.toml` |

## 假设与未知

- **assumed**：`views/` 的 19 个文件对应 19 个独立界面区域——由 `views/mod.rs` 的模块声明与
  `CenterView` 分派确认模块存在，但「每个文件只对应一个区域」是按命名归纳，未逐一核实。
- **unknown**：`run_team` / `run_team_with_history`（并行、主管指派）没有 app 侧调用点，
  运行时行为无证据；未见对应端到端测试（`agent/tests/team_flow.rs` 覆盖的是 handoff 链）。
- **unknown**：远程 SSH 下 `git` 面板、watcher 明确不支持（占位），未做运行验证。
- **unknown**：Windows 之外平台只有 `single_instance` 的空实现，其余行为未在本仓库验证。
- **未观测**：线程模型描述来自代码结构（`std::thread` + `flume` + `cx.spawn`），
  没有做运行时线程 dump 或性能采样。