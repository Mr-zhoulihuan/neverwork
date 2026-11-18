# NeverWork Agent 内置浏览器自动化设计

日期：2026-11-18
状态：已确认，开发中

## 背景

Agent 目前只有 `web_search`（必应 RSS）与 `web_fetch`（HTML→纯文本），只能
「看静态正文」，无法操作页面。用户希望 Agent 能自己打开浏览器、输入账号密码、
走完含验证码的登录流程，对 Web 项目做端到端验收（开发→自测→修复闭环），
且全程免审批、不需要人工触发。

约束与决策（已与用户确认）：

- 浏览器内核用**系统 Edge/Chrome + CDP**，零下载、不引 Node/Chromium。
- 浏览器工具**免审批**（`standard`/`full` 档不弹审批），`read_only`/`plan` 档整体剔除。
- 验证码：**暂停 + `ask_user`** 由用户填写；滑块/扫码类引导切有头模式人工完成。
- 凭据：用户直接在对话里提供；`browser_type` 的 `secret=true` 在持久化时打码。
- 模型验收依据是**文本快照**（可访问性树 + 元素编号），截图只作为产物给人看。

## 架构

新模块 `crates/agent/src/browser/`，全局单例 `Mutex<Option<BrowserSession>>`
（同 `mcp.rs` 风格，进程内一个活跃浏览器），拆四块：`ws.rs`（最小 WebSocket）、
`cdp.rs`（进程 + CDP 会话 + 事件缓冲）、`snapshot.rs`（AX 树→文本）、
`mod.rs`（9 个工具实现）。

- **启动**：探测 `msedge.exe`（常见安装路径 / PATH）→ 回退 `chrome.exe`；
  参数 `--headless=new --remote-debugging-port=0 --user-data-dir=<temp>
  --no-first-run --disable-gpu --no-default-browser-check`；Windows 加
  `CREATE_NO_WINDOW`。`headed=true` 时去掉 `--headless=new`。
- **端口发现**：读 `<user-data-dir>/DevToolsActivePort` 首行；`GET
  http://127.0.0.1:<port>/json/list`（ureq）取 `webSocketDebuggerUrl`。
- **CDP 传输**：手写最小 WebSocket 客户端（HTTP/1.1 Upgrade + 客户端掩码
  文本帧 + 分片重组 + ping/pong），仅连 127.0.0.1，不引新依赖。
  读写两条线程基于 `TcpStream::try_clone` 分离；pong 与命令帧共用一把写锁，
  避免交错写坏帧；握手后取消读超时（空闲不算断开，命令超时由 `Cdp::call` 控制）。
  每条命令 60s 超时；浏览器崩溃自动重启一次。
- **生命周期**：首次 `browser_open` 启动；`browser_close` 或进程退出
  （Drop）时 kill 进程并删除临时 profile。

## 工具清单（9 个，全部 `requires_approval=false`）

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `browser_open` | `url`、`headed?` | 仅 http/https（**放行 localhost/内网**，测本地 dev server 是核心）；返回标题与最终 URL |
| `browser_snapshot` | `limit?` | 可访问性树精简文本 + 交互元素编号 `[n]`（编号映射只在内存），默认 10k 字符上限 |
| `browser_click` | `target`（`[n]` 或 CSS selector） | 点击后等加载/网络空闲（≤3s），返回新标题提示，让模型再 snapshot 断言 |
| `browser_type` | `target`、`text`、`secret?`、`submit?` | `secret=true` 时持久化前替换为 `••••••`；`submit=true` 自动 Enter |
| `browser_press` | `key` | Enter/Tab/Escape/方向键 |
| `browser_wait` | `selector` 或 `ms` | 等元素出现或定时，上限 15s |
| `browser_screenshot` | `name?` | `Page.captureScreenshot` → `<工作区或配置目录>/artifacts/`，返回路径 + Markdown 图片（人看，模型不读图） |
| `browser_console` | `clear?` | console error/warn + 失败请求（URL + 状态），用于断言 |
| `browser_close` | — | 关闭并清理临时 profile |

## 权限与安全边界

- 浏览器工具用 `tools::is_browser_tool(name)`（`BROWSER_TOOLS` 名单）判定，
  **不给 `ToolSpec` 加额外字段**：`specs_for` 在 `read_only`/`plan` 档
  `retain(|t| !t.requires_approval && !is_browser_tool(&t.name))`，剔除全部
  browser 工具（没 open 就没浏览器）；`standard`/`full` 全量可见且不弹审批。
  `agent_loop` 执行前再兜底拦一次（模型幻觉出的 browser 调用返回 `PERMISSION_DENIED`）。
- URL 白名单：仅 `http/https`，允许 `localhost/127.0.0.1/内网`；拒 `file://`、
  `data:` 顶层导航（防绕过路径校验读本地文件）。
- 临时 profile 独立、退出即删；CDP 端口只绑 127.0.0.1 随机端口；
  `RemoteAuth` 式的手写 `Debug` 不打印页面内容之外的信息。
- 密码打码：`secret=true` 的 `browser_type` 调用在写入 `Session.messages`
  （及轨迹页渲染）前把 `text` 替换为 `••••••`，明文只在本轮执行内存中；
  模型后续从历史看到的也是打码值。

## 验证码与凭据流程（系统提示引导）

- Agent 检测到验证码（快照含验证码图片/文案）：`browser_screenshot` →
  `ask_user`（「请输入验证码，或切有头模式手动完成」）→ `browser_type`
  填入后继续。
- 滑块/扫码类：引导 `browser_open(headed=true)` 人工完成后 `browser_wait` 接管。
- 登录凭据由用户直接在对话提供；Agent 必须用 `secret=true` 传密码。

## 验证门禁联动（闭环）

- 现有 `agent_loop` 验证门禁扩展：本轮成功改过**前端文件**
  （`.html/.css/.js/.jsx/.ts/.tsx/.vue/.svelte`，或工作区 `package.json`
  含 dev/serve 脚本）但**从未调用 `browser_open`** 时，注入「浏览器验收提醒」，
  计入现有 `MAX_VERIFY_BLOCKS=2` 上限；纯后端/文档改动不拦。
- 系统提示「工作方式」第 6 条补充：Web UI 改动优先浏览器实测，不只跑单测。

## 状态与验证

- 单测：WS 帧编解码/分片重组/握手响应头不得当帧、AX 树→快照文本、
  `secret` 打码（参数与摘要都不回显）、URL 校验、`specs_for` 档位剔除、
  `is_frontend_path`/`can_browser_verify_workspace`/`browser_gate` 门禁决策。
- 集成（默认 `NW_BROWSER_LIVE!=1` 直接返回）：起本地静态页 →
  open → snapshot → type（含 secret）→ click → 断言回显 → screenshot → close；
  已在本机 Edge 与 Chrome 上实际跑通（各约 1s）。
- 真机验证：`NW_VIEW=chat` 起会话，要求 Agent「起 dev server 并登录验证」，
  观察工具统计出现 browser_* 且无审批弹窗；截图落 artifacts 可点开。
