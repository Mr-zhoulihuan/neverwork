# NeverWork 内置浏览器自动化 实施计划

> **For agentic workers:** 按任务顺序执行；每个任务先写测试再实现；每步跑命令验证。步骤用 `- [ ]` 跟踪。

**Goal:** 给 Agent 增加 9 个内置 `browser_*` 工具（系统 Edge/Chrome + CDP），支持免审批的页面操作、文本快照断言与截图产物，联动权限档位与验证门禁。

**Architecture:** 新模块 `crates/agent/src/browser/`（mod/ws/snapshot 三层）。全局单例持有浏览器进程与 CDP 连接；手写最小 WebSocket 客户端；模型看文本快照，截图落 artifacts 给人看。

**Tech Stack:** Rust 2021、serde_json、ureq（已有）、flume（已有）、base64（已有）、手写 WS 帧、Edge DevTools Protocol。

**设计依据:** `docs/superpowers/specs/2026-11-18-browser-autotest-design.md`

---

## 文件结构

| 文件 | 职责 |
| --- | --- |
| `crates/agent/src/browser/ws.rs`（新建） | RFC6455 客户端：握手、掩码帧编解码、分片重组、ping/pong |
| `crates/agent/src/browser/snapshot.rs`（新建） | `Accessibility.getFullAXTree` JSON → 带 `[n]` 编号的文本快照 |
| `crates/agent/src/browser/mod.rs`（新建） | 进程生命周期、CDP 会话、9 个工具实现、URL 策略、secret 打码 |
| `crates/agent/src/lib.rs`（修改） | `pub mod browser;` |
| `crates/agent/src/tools.rs`（修改） | 9 个 ToolSpec、`execute` 分发、`summary`、`is_browser_tool`、`redact_calls`、`specs_for` |
| `crates/agent/src/agent_loop.rs`（修改） | 权限兜底拦截、打码落历史、系统提示、浏览器验证门禁 |
| `crates/agent/src/tools.rs` 测试区（修改） | `specs_for` 档位、打码、URL 策略单测 |
| `crates/app/src/state.rs`（修改） | `browser_screenshot` 成功后登记产物 |
| `crates/agent/tests/browser_live.rs`（新建） | `NW_BROWSER_LIVE=1` 才跑的端到端测试 |
| `AGENTS.md`（修改） | 新增「浏览器工具」改前必读条目 |

---

### Task 1: WebSocket 客户端（ws.rs）

**Files:** Create `crates/agent/src/browser/ws.rs`

- [ ] 写失败测试：
  - `encode_client_frame` 对 "hello" 输出：FIN|text=0x81，mask 位 0x80，长度 5，4 字节掩码，负载被正确掩码。
  - `decode_server_frames` 接受未掩码文本帧；接受 126/127 长度编码；接受分片（text FIN=0 + continuation FIN=1）拼成一条消息；ping 帧返回 Pong 请求。
  - 非法 opcode / 超长（>16MB）返回错误。
- [ ] 运行 `cargo test -p neverwork-agent ws::` 确认编译失败（模块不存在）。
- [ ] 实现：
  ```rust
  pub struct WsClient { stream: TcpStream, buf: Vec<u8> }
  impl WsClient {
      pub fn connect(host: &str, port: u16, path: &str, timeout: Duration) -> Result<Self, String>;
      pub fn send_text(&mut self, text: &str) -> Result<(), String>;
      /// 读到一条完整消息；内部处理 ping→pong 与分片。
      pub fn read_message(&mut self) -> Result<String, String>;
      pub fn close(&mut self);
  }
  ```
  握手：`GET {path} HTTP/1.1\r\nHost: ...\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {base64(16 随机字节)}\r\nSec-WebSocket-Version: 13\r\n\r\n`，读头到 `\r\n\r\n`，要求状态 101（不校验 Accept，仅回环地址）。
  帧：仅文本/续帧/close/ping/pong；客户端发帧必须掩码；长度 7/16/64 位；分片按 opcode 0x0 累积。
- [ ] `cargo test -p neverwork-agent ws::` 全绿。
- [ ] `cargo build -p neverwork-agent` 通过。

### Task 2: 快照构建（snapshot.rs）

**Files:** Create `crates/agent/src/browser/snapshot.rs`

- [ ] 写失败测试（样例 AX 树 JSON）：
  - ignored 节点不出现；
  - button/link/textbox/checkbox/combobox 分配 `[1] [2] ...` 且携带 `backendDOMNodeId` 映射；
  - 纯文本 StaticText 保留为行；
  - 超 limit 时截断并标注「…（已截断）」；
  - 空名交互元素不分配编号。
- [ ] 实现：
  ```rust
  pub struct Snapshot { pub text: String, pub refs: std::collections::HashMap<u32, i64> }
  pub fn build(ax: &serde_json::Value, page_title: &str, url: &str, limit: usize) -> Snapshot;
  ```
  交互 role 集合：button/link/textbox/searchbox/checkbox/radio/combobox/listbox/menuitem/tab/switch/slider/spinbutton。
- [ ] `cargo test -p neverwork-agent snapshot::` 全绿。

### Task 3: URL 策略与工具规格（tools.rs 增量）

**Files:** Modify `crates/agent/src/tools.rs`

- [ ] 写失败测试：
  - `browser_url_ok("http://localhost:3000")` true；`https://example.com/a?b=1` true；
  - `file:///C:/x`、`javascript:alert(1)`、`data:text/html,x`、超 2048 字符 → false；
  - `specs_for(ReadOnly/Plan)` 不含任何 `browser_` 工具；`Standard/Full` 含 9 个且全部 `requires_approval=false`；
  - `redact_calls` 把 `secret=true` 的 `browser_type` text 替换成 `••••••`，`secret=false` 不变；非 browser 调用不变。
- [ ] 实现：
  - `pub const BROWSER_TOOLS: &[&str]`（9 个名字）；
  - `pub fn is_browser_tool(name: &str) -> bool`；
  - `pub fn browser_url_ok(url: &str) -> bool`；
  - `pub fn redact_calls(calls: &mut [ToolCall])` 与 `pub fn redacted_call(call: &ToolCall) -> ToolCall`；
  - `ToolCall::summary` 增加 browser 分支（browser_type 永远只显示目标与字数，不显示文本）；
  - `builtin_specs()` 追加 9 个 ToolSpec（`requires_approval: false`）；
  - `specs_for`：`hides_write_tools()` 时 `retain(|t| !t.requires_approval && !is_browser_tool(&t.name))`。
- [ ] `cargo test -p neverwork-agent tools::` 全绿。

### Task 4: 进程与 CDP 会话（browser/mod.rs 前半）

**Files:** Create `crates/agent/src/browser/mod.rs`；Modify `crates/agent/src/lib.rs`

- [ ] 实现（无单测，集成测试覆盖）：
  - `static BROWSER: OnceLock<Mutex<Option<Browser>>>`；
  - 探测浏览器：`NW_BROWSER_PATH` → Edge 常见路径 → Chrome 常见路径 → PATH 搜 `msedge.exe`/`chrome.exe`；
  - `launch(headed)`: 临时 profile `%TEMP%\neverwork-browser-<pid>-<ts>`，args 见 spec，Windows `CREATE_NO_WINDOW`；轮询 `DevToolsActivePort` ≤15s；
  - `Cdp`：连接 `/json/list` 里的 page target WS；reader 线程按 `id` 分发响应（flume 通道），事件进 `console` 缓冲（`Runtime.consoleAPICalled`、`Log.entryAdded`、`Network.loadingFailed`，上限 200 条）；
  - `call(method, params, timeout)` 60s；`Drop` kill + 删 profile；
  - `ensure()` 启动或崩溃后重启一次。
- [ ] `lib.rs` 加 `pub mod browser;`，`cargo build -p neverwork-agent` 通过。

### Task 5: 9 个工具实现（browser/mod.rs 后半）

**Files:** Modify `crates/agent/src/browser/mod.rs`；Modify `crates/agent/src/tools.rs`（execute 分发）

- [ ] 实现 `pub fn execute(call: &ToolCall, ctx: &ToolContext) -> ToolResult`：
  - `browser_open(url, headed?)`：启动/复用 → `Page.navigate` → 轮询 `document.readyState==="complete"` ≤20s → 返回标题/URL；
  - `browser_snapshot(limit?)`：`Accessibility.getFullAXTree` → `snapshot::build` → 存 refs → 返回文本（包「页面：标题（URL）」头）；
  - `browser_click(target)`：`[n]` → refs 映射 → `DOM.resolveNode` → `Runtime.callFunctionOn` click；CSS → `Runtime.evaluate` + `querySelector`；点击后等 ≤3s readyState；
  - `browser_type(target, text, secret?, submit?)`：native setter + input/change 事件（contenteditable 用 textContent）；返回「已输入 N 字符」，绝不回显；
  - `browser_press(key)`：`Input.dispatchKeyEvent`（Enter/Tab/Escape/ArrowUp/Down/Left/Right）；
  - `browser_wait(selector|ms)`：轮询 `document.querySelector` 200ms，上限 15s；
  - `browser_screenshot(name?)`：`Page.captureScreenshot` → 写 `<root>/artifacts/browser-<ts>.png`（无工作区 → `NW_CONFIG_DIR`/`~/.neverwork` 的 artifacts）→ 返回「截图已保存：<abs>\n![browser](<forward-slash path>)」；
  - `browser_console(clear?)`：输出 error/warn 与失败请求（脱敏后），clear 清缓冲；
  - `browser_close()`：`Browser.close` + kill + 删 profile。
  - 所有工具结果正文交给 `wrap_untrusted`（agent_loop 已有），错误走 `ToolResult::err_code`（`BROWSER_LAUNCH`/`NAV_FAILED`/`NOT_FOUND`/`TIMEOUT`/`BAD_ARGS`/`UNSUPPORTED`）。
- [ ] `tools.rs::execute` 分发 9 个名字到 `crate::browser::execute`。
- [ ] `cargo build -p neverwork-agent` 通过。

### Task 6: Agent 循环接入（权限、打码、提示词）

**Files:** Modify `crates/agent/src/agent_loop.rs`

- [ ] 权限兜底：`if permission.hides_write_tools() && (write_call || tools::is_browser_tool(&call.name))` → `PERMISSION_DENIED`（提示切档）。
- [ ] 打码：
  - `messages.push(ChatMessage::assistant_with_tools(text, calls.clone()))` 前克隆并 `tools::redact_calls(&mut stored)`；
  - `emit(AgentEvent::ToolStarted { call: tools::redacted_call(&call), .. })`；
  - 执行仍用原 `call`。
- [ ] 系统提示：
  - 工具清单追加 9 个 browser 工具的用法摘要；
  - 「工作方式」第 6 条追加「Web UI 改动要用浏览器实测（起 dev server → browser_open → snapshot → 操作 → 断言）」；
  - 新增 `## 浏览器验收与登录测试` 段：先 snapshot 再操作；验证码走 screenshot + ask_user；滑块/扫码用 `headed=true` 人工过；密码必须 `secret=true`；每次操作后用 snapshot/console 断言。
- [ ] 单测：`build_system_prompt` 含 `browser_open` 与 `secret`；`specs_for` 联动已在 Task 3 覆盖。

### Task 7: 浏览器验证门禁

**Files:** Modify `crates/agent/src/agent_loop.rs`

- [ ] 新增：
  ```rust
  pub(crate) fn is_frontend_path(path: &str) -> bool; // html/css/js/jsx/ts/tsx/vue/svelte/astro
  fn can_browser_verify_workspace(ctx: &ToolContext) -> bool; // 本地；根 index.html 或 package.json 含 dev/start/serve 或 vite/next 配置；向下探一层
  ```
- [ ] 状态：`wrote_frontend`（写工具成功且 `is_frontend_path`）、`browser_used`（`browser_open` 成功）；`VerifyGate` 收尾前：`wrote_frontend && can_browser_verify && !browser_used && verify_blocks < MAX_VERIFY_BLOCKS` → 注入 `BROWSER_GATE_REMINDER`（复用 `verify_blocks` 计数）；拦满追加「⚠️ 未经浏览器验收」附注。
- [ ] 单测：`is_frontend_path` 正反例；`can_browser_verify_workspace` 用临时目录构造 package.json（有/无 dev 脚本）；门禁决策纯函数测试。

### Task 8: 产物登记与端到端测试、文档

**Files:** Modify `crates/app/src/state.rs`；Create `crates/agent/tests/browser_live.rs`；Modify `AGENTS.md`；Modify spec 文档（`mutates` 字段改为 `is_browser_tool` 前缀判定）

- [ ] `state.rs::ToolFinished`：`browser_screenshot` 成功时从 `result.content` 首行「截图已保存：」后取路径，`register_artifact`。
- [ ] `tests/browser_live.rs`：`#[test] fn browser_login_flow()`，`NW_BROWSER_LIVE!=1` 直接 return；进程内 TcpListener 起静态页（用户名/密码框 + 按钮回显），经 `neverwork_agent::browser` 走 open→snapshot→type→click→snapshot→close，断言 `[n]` 与回显文本。
- [ ] 跑 `cargo test --workspace`（离线用例）；`NW_BROWSER_LIVE=1 cargo test -p neverwork-agent --test browser_live -- --nocapture` 真机验证。
- [ ] `AGENTS.md` 增补浏览器工具条目（免审批理由、URL 白名单、secret 打码、验证码流程、门禁规则）。
- [ ] 真机验收：`cargo run -p neverwork`，会话里让 Agent 起本地静态服务并浏览器登录验证，确认无审批弹窗、截图进产物面板。

---

## 风险与兜底

- Edge 版本差异导致某些 CDP 命令缺失：所有命令失败按 `CDP_ERROR` 返回并提示模型换 `browser_snapshot` 重试；不 panic。
- 快照过大：默认 10k 字符上限，优先交互元素；超限截断标注。
- 浏览器进程残留：Drop 兜底 kill；`browser_close` 主动清理临时 profile。
- 模型幻觉 secret：`browser_type` 无 `secret=true` 也永不回显 text；`summary` 只显示字数。
