# 项目约定

## 项目

纯 Rust + GPUI（Zed 内核）的 AI 原生 IDE「NeverWork」。`E:\AI\ide` 只放文档、设计稿与截图；**真实代码在 `NeverWork/`**（4 crate：`core` 模型+配置、`ui` 令牌+组件、`agent` Provider+工具+编排、`app` 界面+状态）。

## 常用命令

```bash
cargo run -p neverwork          # 主界面
cargo test --workspace          # 全部测试（3 条打真实 DeepSeek，需 NW_DEEPSEEK_KEY）
cargo build -p neverwork        # 只编应用（增量约 6s）
powershell -ExecutionPolicy Bypass -File installer\build.ps1   # NSIS 安装包 → dist\NeverWork-<版本>-setup.exe（版本取自 workspace Cargo.toml，发版同步改 crates\app\assets\neverwork.rc；dist/ 已 gitignore，需 NSIS；exe 图标由 build.rs 内嵌，图标源 crates\app\assets\neverwork.ico）
```

运行期变量：`NW_ROOT`、`NW_REMOTE`（`user@host:/path`）、`NW_API_KEY`/`NW_PROVIDER`/`NW_BASE_URL`/`NW_MODEL`、`NW_VIEW`、`NW_SETTINGS_TAB`、`NW_OPEN`、`NW_WIN_W`/`NW_WIN_H`、`NW_MAXIMIZE`、`NW_CONFIG_DIR`；验证专用 `NW_SEND`/`NW_AUTO_APPROVE`/`NW_TEAM`/`NW_RIGHT_COLLAPSED`/`NW_RIGHT_TAB`（`files`|`git`|`artifacts`|`preview`）/`NW_SESSION_COLLAPSED`/`NW_FIND`/`NW_FOLD`/`NW_CURSORS`/`NW_PICK_MODEL`/`NW_PROVIDER_VIEW`（`add`|下标）/`NW_COMPOSER`（`thinking`|`menu`|`permission`）/`NW_PERMISSION`（`read_only`|`plan`|`standard`|`full`）/`NW_LAYOUT_DEBUG`/`NW_MARKET_SOURCE`/`NW_MARKET_NO_REFRESH`/`NW_SKILLS_TAB`/`NW_SKILL_QUERY`/`NW_INSTALL_SKILL`/`NW_SESSION_QUERY`/`NW_SESSION_MENU`/`NW_ARCHIVE_OPEN`/`NW_WORKSPACE_POPUP`/`NW_DEMO_STATUS`/`NW_SWITCH_WS`/`NW_TRACE`（=1 启动即开轨迹页；=0 关）/`NW_TRACE_SELECT`（选中下标）/`NW_TRACE_QUERY`（搜索词）/`NW_USAGE`（=1 启动即开「用量」弹框）/`NW_ALLOW_MULTI`（=1 绕过单实例互斥量并行多开）；`NW_ASKPASS_MODE`/`NW_ASKPASS_SECRET` 由 ssh 回调应用自身时使用，不要手动设置；密码往返测试用 `NW_ASKPASS_EXE` 指向 `target/debug/neverwork.exe`。

工作区回落：`NW_ROOT` → `settings.last_workspace` → `~/.neverwork/workspace`。**不要**回落到当前目录/向上找 `Cargo.toml`——会把 IDE 自己的仓库当工作区。`last_workspace` 是 `ssh://` 时走远程工作区（本地 `root` 为 `None`）。

## 红线

- **仓库根是 `E:\AI\ide`（远程 `gitee.com/ZLH1995123/neverwork`）**，`NeverWork/` 是子目录而非独立仓库。`NeverWork/target` 已 gitignore（**不要 `git add -f` / `git clean -xfd` / `cargo clean`**，全量重编约 700 crate）。
- 分支：`main` 开发主线，`master` 是 Gitee 默认分支，两者同提交；`develop/feature/release` 同指主线。
- 应用进程没关时 `cargo build` 会报「failed to remove neverwork.exe」但**返回码仍是 0**，跑的是旧二进制——先 taskkill 再构建。
- 配置在 `~/.neverwork/`：`config.json`（团队）、`settings.json`（偏好，**含 Key 明文**，别提交别外传），均原子写。测试用 `NW_CONFIG_DIR`。
- **主题必须走 `theme::set_theme_mode()`**，不要在视图里写死颜色——`ui/theme.rs` 的 `PALETTES` 有 9 套完整色板（深色/浅色/跟随系统/Nord/Dracula/One Dark/Solarized 浅深/Gruvbox），写死 `rgb(0x…)` 只会适配其中一套；新增颜色要同步加到每套色板。
- **API Key 一律脱敏**：导出/同步走 `config::export_all(..., ExportOptions::default())`；界面只显示 `masked_key()`；不要给 `ProviderEntry` 加回 `derive(Debug)`（手写版会打码）。新增 Settings 序列化到文件/日志的路径先确认 Key 不跟着走。
- `design/`、`pic/` 是设计截图（**原有设计图不要动**）。**开发过程产生的测试数据一律放 `E:\AI\ide\pic\`（文件名加 `.tmp_` 前缀，包括验证脚本、验收截图、测试配置目录、临时工作区），开发完成立刻删掉**；代码目录 `NeverWork/` 下不放任何临时文件。
- 计划冲突以 `gpui-ide-plan-v2.md`（最新）与 README 为准。

## 输入与交互（改前必读）

- **文本插入只有一条路径**：`EntityInputHandler`（`input_handler.rs`）→ `TextBuffer`。**绝不要**在 `on_key_down` 里插 `key_char`，会和 IME 重复写入。键盘只管非文本键（`keymap.rs`）。
- 光标/选区/撤销统一在 `text_buffer.rs`，输入框与编辑器共用。
- 带 `cursor_pointer`/`hover` 的元素必须有 `on_click`/`on_mouse_down`，否则是死点击。
- 编辑器按字符渲染成可点击单元；行内容走 `line_cache.rs`，别在渲染里跑 `highlight_line`。
- **多光标是 `TextBuffer` 独立层**（`extra_cursors`），只有 `*_multi` 系列碰它。
- **折叠不重排文本**：只跳过隐藏行，真实行号映射回光标/选区。**`multi_patch` 必须先全量校验再写**，失败回滚。
- **文字截断别只靠 `.truncate()`**：gpui 首帧没确定宽度就不生效，`flex_1` 嵌套下会硬裁且无省略号。要省略号用 `views::text_fit::fit_text(text, 预算px, 字号)` + `.whitespace_nowrap()`。
- **布局优先级**：中栏 > 会话栏 > 右栏。窄窗口把右栏收成 16px 窄条（<1080），极窄（<620）才隐藏会话列表，图标栏永远保留。**设置/技能/编辑器页收起会话列表**（编辑器保留右栏文件树）；`CenterView::Team` 照常显示。会话列表本身也可手动收起：会话栏右边缘中部的「«」把手（id `collapse-session`，绝对定位不占列表高度）收起成 16px 窄条，再点窄条中部的「»」（`expand-session`）展开，`session_collapsed` 是内存态、默认展开，与右栏 `right_collapsed` 同构；`left_w` 计算必须把收起态算成 `RAIL_W + 16`。宽度用「可见尺寸」（见环境特性）。
- 图标栏只有对话/团队/技能（主组垂直居中）+ 底部一个设置入口；编辑器页靠右栏文件树打开文件进入，不占图标；命令面板只走会话面板「⋯」入口（标题栏不再放 ⌘K 按钮）。三个主图标是手绘 SVG（`crates/app/assets/icons/rail-*.svg`，样式参考 `pic/minimal-dark-mode-sidebar-icon-1.jpg`），经 `src/assets.rs` 的 `EmbeddedAssets`（`Application::new().with_assets(...)`）内嵌，用 `gpui::svg().path("icons/…").text_color(...)` 按 alpha 蒙版上色、随主题与悬停变色——不要写死颜色，也不要退回字符图标（设置 ⚙ 仍是字符版 `rail_button`）；外观验收图 `pic/rail-icons.png`。
- **弹层遮罩必须 `.occlude()`**：gpui 的滚轮命中取「鼠标下所有 hitbox」（`should_handle_scroll`），遮罩不 occlude 时滚弹层内容会同时滚背后的滚动容器（技能详情抽屉踩过：滚正文整页跟着下移）。palette / session_menu / workspace_picker / new_session / market_detail / file_menu / tab_menu 七个遮罩都走 occlude。
- **窗口是无边框自绘的**（`main.rs` 里 `titlebar: Some(TitlebarOptions { appears_transparent: true, .. })`）。**别退回 `WindowOptions::default()` 的 `appears_transparent: false`**——那样 Windows 会在最上面叠一条系统原生标题栏（浅色横条 + 左上角白色默认窗口图标 + 右键「还原/移动/关闭」菜单）。
- **Release 不得弹控制台**：`main.rs` 顶部必须有 `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`——否则双击 exe / 快捷方式启动时 Windows 会额外挂一个终端窗口，tracing 日志全打在里面；debug 保留控制台方便 `cargo run` 看日志。
- 标题栏拖动 / 最小化 / 最大化(还原) / 关闭由 `views::titlebar_hitboxes`（绝对定位 canvas）注册系统热区 `Drag`/`Min`/`Max`/`Close` 处理，点击不经过 `on_click`；右侧 300px 不铺 Drag 热区，否则 HTCAPTION 会吞掉「专注/窗口按钮」的点击。
- 窗口按钮图标必须用 `div` 画（横线/方框/错位双框），**不要用 `□`/`❐` 字符**：字体回退后两者几乎一样，最大化前后看不出区别。最大化状态在 `render` 里用 `window.is_maximized()` 传给 `titlebar`，窗口尺寸变化会触发重绘。
- **文件树右键菜单与标签关闭**（`views/file_menu.rs`，复用 session_menu 的 `menu_row`/`menu_glyph`/`menu_divider`）：文件行右键（本地+远程两分支）→ `open_file_menu`（状态 `file_menu: Option<FileMenuTarget>`），菜单含「添加到会话」（走 `attach_paths`，目录与远程置灰）与「复制路径」（远程拼 `url()/rel`）；编辑器/媒体标签右键 → `tab_menu_at`（`open_tab_menu`），菜单含「关闭」「关闭全部」；标签尾部的 ✕（id `close-file`/`media-close`）与菜单项都走 `close_current_file`——**未保存先 `save_file`，同步保存失败就不关**（防丢内容），关完回对话页。菜单坐标与 session_menu 同款：state 存鼠标坐标，render 侧按 `visible_w/h` clamp 反向弹出。
- **单实例**：`main.rs::single_instance` 用 Windows 命名互斥量 `Local\NeverWork-SingleInstance-v1`（`CreateMutexW` + `ERROR_ALREADY_EXISTS`）；已在跑时 `EnumWindows` 按 exe 名找窗口并 `ShowWindow(SW_RESTORE)`+`SetForegroundWindow` 激活后退出。检查必须在 askpass 分支**之后**（ssh 密码回调会反复启动本进程）；`NW_ALLOW_MULTI=1` 仅供自动化并行多开，不要删。全部手写 FFI，不引新依赖。
- **右栏「改动」面板是只读的**（`RightTab::Git`，tab 头「文件 / 改动 / 预览」）：只调用 `git status` 列出改动文件（增/改/删/重命名/冲突标记 + 已暂存绿点），点击行用编辑器打开文件（Deleted 行提示无法打开）。**不要往面板加任何 git 写操作**（暂存/提交/丢弃/分支切换均已被移除，`git_ops.rs` 已删，别加回）。刷新时机：切 tab（`set_right_tab` 已内置）、展开右栏（`toggle_right_panel`）、切工作区（`enter_workspace` 尾部按 tab 判断）、文件 watcher 回调（仅面板展开时，300ms 去抖后跑 `git status`）。远程工作区显示占位；`NW_RIGHT_TAB=git` 验证直达。**别把 `reset_git_state` 改回无条件 `refresh_git`**：切工作区会同步起 6+ 条 git 子进程，大仓库下界面卡几秒。

## 左侧栏

`RAIL_W=52` + `SESSION_W=264` 会话卡片（头部标题 + ＋新建 + ◀收起，头像+标题+状态 / 摘要+时间+胶囊）。卡片内宽 `CARD_INNER = SESSION_W - LIST_PAD*2 - CARD_PAD*2`，漏算内边距会裁右侧胶囊。会话操作是**右键上下文菜单**（`views/session_menu.rs`，全屏透明遮罩 + 232px 绝对定位卡片，行=语义图标+13px 文字；宽度用 `session_menu::MENU_W` 与 state 的 clamp 共用）；右键卡片与点卡片的 ⋯ 都走 `on_mouse_down` 取鼠标坐标（`ClickEvent` 不带坐标），渲染侧用 `visible_w/h` clamp 反向弹出防出界。筛选/归档判定用 core 的 `session_matches_list`/`is_archived_session`，不要在视图里另写一套。

## 环境特性

逻辑屏 1280x800、物理 2560x1600、`scale_factor()`=2；`bounds()`/`display.bounds()` 都是逻辑值，别除 scale。

**验收截图须 DPI-aware**：PowerShell 默认 unaware 只抓到左上 1/4，会伪造「内容放大、右侧出界」；先 `SetProcessDpiAwarenessContext(-4)`（见 `design/shot.ps1`）。

**崩溃兜底**：`main.rs::install_crash_log` 把 panic + backtrace 追加到 `<配置目录>/crash.log`（release 无控制台，闪退靠它）；栈溢出/abort（异常码 `0xc0000409`，窗口过程里 panic 穿过 `extern "system"` 也会 abort）不经过 panic 钩子，已在 `HKCU\Software\Microsoft\Windows\Windows Error Reporting\LocalDumps\neverwork.exe` 注册全量 dump（落 `%LOCALAPPDATA%\CrashDumps`）。排障先看这两处。

## Agent 工具（改前必读）

- 工具在 `agent/src/tools.rs`。**所有路径走 `ToolContext::resolve`**（挡 `..`、跨盘绝对路径、符号链接逃逸）。
- **联网工具在 `agent/src/web.rs`**：`web_search`（必应 RSS，不引 Key）/`web_fetch`（只收公网 http/https，拒 localhost 与内网，12s + 300KB 上限，HTML 转纯文本）。两者只读、`requires_approval = false`、**默认常开不设开关**；返回正文与文件内容一样走 `wrap_untrusted`，别让网页内容绕过这层。
- **浏览器自动化在 `agent/src/browser/`**（`ws.rs` 最小 WebSocket + `cdp.rs` 进程与 CDP + `snapshot.rs` AX 树→文本 + `mod.rs` 9 个工具）：系统 Edge/Chrome `--remote-debugging-port=0` + **进程级固定 profile**（`cdp::session_profile`：会话内重启/人工接管复用同一目录，cookie 与登录态不丢），全局单例一个浏览器，`browser_close` 才删 profile（Drop 只关进程保留目录）；**默认有头（可见窗口，用户要能看见测试在点什么）**，`NW_BROWSER_HEADLESS=1` 强制无头（CI/自动化验收必须带上），`NW_BROWSER_PATH` 可指定浏览器，真机测试 `NW_BROWSER_HEADLESS=1 NW_BROWSER_LIVE=1 cargo test -p neverwork-agent --test browser_live -- --nocapture`。9 个工具 `requires_approval=false`（**免审批**），但 `read_only`/`plan` 档由 `specs_for` 与 agent 循环双重剔除（`is_browser_tool`）；`browser_open` 只收 http/https（localhost/内网放行，拒 file:/data:）。模型靠 `browser_snapshot` 的 `[n]` 编号操作；密码/验证码/密钥必须 `secret=true`——落历史与 `ToolStarted` 前 `redact_calls`/`redacted_call` 打码，工具结果永不回显 text，明文只在本轮执行内存。遇到验证码/账号密码/扫码/滑块：调 `browser_takeover` 打开可见窗口让用户自己输入（见下条 bullet）。WS 读写分线程共用一把写锁回 pong；**握手响应头不得当帧数据**（曾把 `HT` 解成 close 帧导致「连接已断开」，有回归单测）。
- **`cdp::launch` 只认 `DevToolsActivePort`，不能把 `child.try_wait()` 命中当失败**：Edge 的启动器常先把浏览器主进程转交给另一个 msedge 后自己先退出（实测 4/4），主进程随后才写端口。旧逻辑据此报「浏览器启动后立即退出」并留下孤儿 msedge（命令行含 `neverwork-browser-*`，要按它清理）。现在给 3 秒宽限继续等端口（拉起的进程一直活着则等满 15s）；浏览器 stderr 落 profile 下 `neverwork-launch.log`，失败时尾部带进错误信息。**`alive()` 同样不能用 `child.try_wait()`**：启动器退出会被误判成「崩溃」→ `browser_open` 静默重启新 profile、登录态全丢（用户登录后又要登录，实测踩过）；已改为 CDP `Browser.getVersion` 探测（800ms）。**关浏览器要按主进程 pid 杀整棵树**：`kill(child)` 只杀启动器，`Browser.close` 没送达时整个 msedge 进程树会残留（实测一场测试漏 18 个）；Drop 里先取 `SystemInfo.getProcessInfo` 的 `type=browser` id，close 后 Windows `taskkill /PID <id> /T /F` 兜底。
- **浏览器验收门禁**：本轮写了前端文件（`is_frontend_path`）且工作区能起 Web 服务（`can_browser_verify_workspace`：index.html / vite/next 配置 / package.json 的 dev|start|serve 脚本，且本机有浏览器）却从未成功 `browser_open` 时，注入 `BROWSER_GATE_REMINDER`，与代码验证门禁共用 `MAX_VERIFY_BLOCKS`=2 计数；拦满或其它原因收尾时追加「⚠️ 未经浏览器验收」附注。别把纯后端/文档改动算进 `wrote_frontend`。
- **写操作与 `run_command` 必须 `requires_approval = true`**，经 `NeedApproval` → 用户允许后才执行；不要在工具内部直接落盘/起进程。`run_command` 是唯一 shell 入口：`cwd` 走 `resolve`，Windows 用 `cmd /S /C` + `CREATE_NO_WINDOW`（别弹控制台）与 `chcp 65001`，Unix 走 `/bin/sh -lc`；超时默认 120s（上限 600s），输出按头 3KB + 尾 12KB 截断；远程工作区返回 `UNSUPPORTED`。严禁绕过审批静默执行。审批「没批准」分三种：用户点拒绝 / 5 分钟没人点（超时）/ 生成被中断，回灌给模型的文案必须分开写——全写成「用户拒绝」会让模型对用户说「被你拒了」，而用户其实没拒绝（实战踩过）。
- **会话权限档位 `PermissionMode`**（`Session.permission`，随会话保存，默认 `standard`）：`read_only`/`plan` 由 `tools::specs_for` 剔除所有 `requires_approval` 工具（含 MCP），agent 循环执行前再拦一次（`PERMISSION_DENIED`，不弹审批）；`plan` 另外靠系统提示要求先出实施计划。`full` 免审批直接执行，但**只免审批**——不得绕过 `ToolContext::resolve`、`.git` 禁写、命令超时/截断。UI 入口是对话框底栏 `permission-chip`，切 `full` 必须二次确认。
- 团队角色的 `model` 是**展示标签**（`Claude Opus`），不能当模型 id 或据此切厂商——只有全小写真实 id 才算。
- **运行上限三项（tokens/rounds/seconds）默认 0=不限**，从 settings 读；不要加回常量上限（读中型项目必被掐断）。防失控靠 `MAX_REPEAT=3`（同工具同参数）与手动停止。
- **验证门禁（`agent_loop` 收尾前）**：本次运行成功改过**代码或构建清单**（`write_file`/`apply_patch`/`multi_patch` 的路径命中 `is_code_path`：.rs/.ts/.py/.go 等源码后缀与 Cargo.toml/package.json/go.mod 等清单）却没有任何成功的构建/测试命令（`run_command` 返回 ok 且命令命中 `is_verify_command` 关键词表：cargo/npm/go/pytest/maven…）时，注入「验证提醒」让模型继续验证，最多拦 `MAX_VERIFY_BLOCKS`=2 次；拦满强制收尾并追加「⚠️ 本次改动未验证」附注（时间/轮数/成本上限收尾同样附注）。**纯文档/数据改动（README.md、settings.json、图片等）不拦**；工作区无构建入口（`can_verify_workspace` 向下探两层找清单，远程工作区恒 false）也不拦——「要不要验证」由机制判定，不交给模型（它会顺手跳过）；别把 README 类改动重新算进 `wrote_code`。用户明确要求不跑命令/拒绝命令属例外，要求模型如实写「未验证」而不是编造。系统提示「工作方式」第 6 条同样区分代码与文档；改验证相关行为先看 `code_path_distinguishes_code_from_docs`/`verify_availability_follows_manifests`/`verify_gate_blocks_only_unverified_writes` 单测。
- **浏览器自动化与人工接管**：`browser_screenshot` 返回「截图已归档：#N（标题）」，截图**不登记产物**、只进 `browser_report` 报告（会话内 `cdp::Shot` 清单，编号 = 下标+1）。遇到验证码/账号密码/扫码/滑块等需要真人输入的画面，模型应调 `browser_takeover`（`browser/mod.rs::takeover`）：关闭无头会话、以 headed 重开并导航回当前页，让用户自己输入，再用 `ask_user` 等确认、`browser_snapshot` 继续；**不要**用截图+ask_user 让用户口述验证码，也不要猜测/编造凭据。headed 重开复用同一 profile，页面重新加载但**登录状态保留**；该工具免审批、无参数。**用户点停止 = 本轮自动化结束**：`agent_loop` 的 cancel 检查点与 `state.rs::stop_streaming` 都会调 `browser::shutdown()` 关掉会话（Drop 里 Browser.close + kill，正在跑的 CDP 调用随即失败返回）；别把「停止」改回只置标志（会“停了还在测”）。
- **`ask_user` 交互选择**（关键决策交给用户）：工具定义在 `tools.rs`（question 必填、options 1-4 个、allow_custom 默认 true，`requires_approval=false`，只读/计划档保留）；`agent_loop` 在 repeat 检测后拦截：`validate_ask` → `AgentEvent::AskUser` → flume 阻塞等待（200ms 轮询 cancel、300s 超时按「跳过」）→ 回答以 `ToolResult::ok("用户回答：…")` 回灌后继续。app 侧 `PendingAsk` + `InputTarget::AskCustom`（枚举/`buffer()`/`active_buffer()`/焦点两处共五处同步），卡片在 `chat.rs::tool_steps_group` 与审批卡同位置渲染（第一个选项绿底=推荐，自定义输入行 + 跳过），按钮走 `answer_ask`/`skip_ask`/`commit_ask_custom`。自动化验收用 `NW_ASK_ANSWER=1`（选第一项）或其它文本（自定义回答）；别把 ask_user 改成 `requires_approval=true`，它只是交互不是写操作。
- **ask_user 卡片只渲染一次**：`pending_ask` 按 `msg_idx ∈ [start,end)` 归属到唯一一组工具调用（`chat.rs::ask_card_in_group`，索引未知时只放最后一组）。**别退回只按 session 过滤**——历史里每组工具后面都会重复画一张卡，两份卡片元素 id 相同，gpui 的 element_state 按 GlobalElementId 存储，鼠标/焦点状态互相踩踏，按钮会全部点不动（实测踩过）。卡片元素 id 带组起点（`ask-opt-{start}-{i}`/`ask-custom-{start}`）防重复。

## 对话区与查找（改前先读）

- 工具调用由 `views/chat.rs::tool_steps_group` 把连续调用聚合成**一行统计**（线程 / 思考轮次 / 工具数 / 动作直方图），逐条路径与输出不再展示；不要再加展开明细的入口。写操作审批必须单独显示 diff 与「拒绝 / 允许并写入」。工具消息里的 `thread`（角色序号）/`round`（该角色第几轮模型调用）用于统计，旧会话缺省 0。
- 模型/厂商是**会话级**：`Session.provider_id`/`model`（空 = 跟随设置里的当前厂商），头部 chip 切换、按厂商分组；新会话继承上一个会话的选择（厂商被删/停用则回「跟随默认」）。
- 移除厂商是两步确认（`provider_remove_confirm`）：Key 会随配置一起删掉，不允许一点就没了。
- **思考等级是会话级**（`Session.thinking`，实现见 `views/chat.rs`）：`自动` 不向厂商发任何思考参数，只有低/中/高才映射 `reasoning_effort` / Anthropic `thinking.budget_tokens` / Gemini `thinkingConfig`。Anthropic 开思考必须 `temperature=1` 且 `max_tokens > budget`；Gemini 思考 token 计入输出，`maxOutputTokens` 要同比抬高。改这几处先看 `agent` 里对应的单测。
- 思考滑杆用**窗口级 `window.on_mouse_event`**（在 canvas paint 里注册）跟踪拖动，不要换成元素级 `on_mouse_move`：后者只在悬停时触发，光标滑出 256px 轨道就断，滑杆会粘在半路。拖动中只 `preview_thinking_level` 改内存，松开鼠标才 `save_sessions`。
- 输入框不再做自动联想（本地历史补全与模型补全都已移除）：不要在 `after_edit`/`on_chat_key` 里挂补全请求，聊天框的 Tab 不做任何事。
- **思考档位入口默认隐藏**：不常驻输入框底栏，从「＋」菜单（`⚙ 思考 · 档位`）进入滑杆面板；不要再加回右下 chip。面板互斥（开一个关另一个）与拖动实现见上一条。
- 查找替换在 `app/src/find.rs`（纯逻辑）+ `views/editor.rs::find_bar`（UI），按字节区间匹配，中文安全。
- **回答正文走 `markdown.rs` 子集**：标题/列表/代码围栏/引用/分隔线/**表格** + 行内粗体/斜体/代码/链接（`views/chat.rs::markdown_block`）。表格列宽按 `char_width_mono×1.12` 估算、每列保底 88px——**别去掉余量与保底**，否则 `Rust`/`Python` 会被拆成两行（实测踩过）；单元格文本走 `inline_row_layout`（可指定最小折行宽度与列对齐）。
- **数据块自动渲染**（`markdown.rs::data_block`）：`csv`/`tsv` 围栏 → 表格（≥2 行、列数一致）；`json` 围栏 → serde 美化（≤64KB）；**未标注**围栏只在严格特征下才猜（JSON 以 `{`/`[` 开头；CSV/TSV ≥3 行且格子 ≤40 字符、无代码符号与中文句号），其它语言不猜——防止把普通代码误渲染成表格。**图片行**：整行 `![alt](src)` → `Block::Image`，`artifacts::resolve_image` 支持绝对路径 / 工作区相对路径 / `data:image/...;base64`（解码落 `<config_dir>/artifacts/`，内容哈希幂等）；http(s) 不抓取，给占位。改这两处先跑 `markdown.rs` 里的 9 个相关单测。
- **产物（Artifact）**：`core` 的 `Artifact{name,path,kind,at,thread}` 挂在 `Session.artifacts`（`#[serde(default)]`，旧会话兼容）。收集双通道——写工具成功（`state.rs::written_paths` 解 `write_file`/`apply_patch` 的 path 与 `multi_patch` 的 `edits[].path`）与每轮收尾扫描 Agent 正文图片（`markdown::extract_images` + `resolve_image`）。UI：对话流末尾「产物 · N」条（最新 8 个，点 `open_artifact`，远程/丢失文件给 notice）+ 右栏第四 tab「产物」（完整列表）。收尾登记后必须 `chat_scroll_dirty.set(true)`，否则新 footer 落在视口外（别删）。右栏默认收起（`right_default_collapsed()`，切/建会话同样回到收起），`NW_RIGHT_COLLAPSED=0` 验证时强制展开、`NW_RIGHT_TAB=artifacts` 直达产物面板。协议层原生图片输出（Gemini inlineData 等）未接，模型出图靠 Markdown 图片 / data URI 承载。
- **产物另存为**：右栏产物行、对话底部 chip、图片查看器都带 ⤓（`state.rs::export_artifact` + `chat.rs::artifact_save_button`）：点它弹**系统保存对话框**（`cx.prompt_for_new_path`，gpui Windows 已实现 IFileSaveDialog），默认目录 `<工作区>/artifacts/`（无工作区 → 配置目录 `artifacts/exports/`）、默认原文件名，可改到任意位置；重名覆盖由对话框确认，取消则什么都不做。保存按钮必须与卡片/行**并排**而非嵌套（嵌套时点击区重叠，点保存会连带打开产物）。图片查看器整层从 `theme::TITLEBAR_H` 下方开始，否则顶栏按钮会命中系统最小化热区。
- **测试报告（`browser_report`）**：浏览器测试收尾由 Agent 生成 HTML 报告（`artifacts/测试报告-<本地时间>.html`，单文件、截图 base64 内嵌、console 段），报告是唯一产物——`browser_screenshot` 的「截图已保存：」登记分支已删，截图只进报告（会话内 `cdp::Shot` 清单按 `#1..#N` 编号供报告引用）。**报告是章节式单文件**：用例可带 `chapter` 字段，同章用例聚合（章号 1/2/3、用例号 1.1/1.2）；左侧固定 TOC（`browser/report.rs::render_toc`，可折叠 + IntersectionObserver 滚动高亮，少量内联 JS 允许）；全无 chapter 时退化为平铺列表（不渲染章头）。渲染层是 `browser/report.rs` 纯函数（无 IO），新样式/段落改那里并跑 `cargo test -p neverwork-agent --lib browser`。app 侧按结果首行「测试报告已生成：」登记产物，产物点开 `.html` 走系统默认浏览器（`state.rs::file_url` 百分号编码 + 剥 `\\?\` 扩展前缀 + `cx.open_url`），别让报告进文本编辑器。`report_should_remind`：浏览器操作 ≥3 步且没出报告时提醒一次（只一次）。**测试过程清理**：报告写成功后才删本会话截图 PNG（已 base64 内嵌）并 `b.shots.clear()` + `report_written=true`；`browser_close` 在 `report_written` 时删残留截图，未出报告则保留并提示（截图是唯一证据，会话已关无法补内嵌报告）。别把报告本身删掉；提示词已禁止 run_command 清理 artifacts/ 里的报告。
- **对话轨迹页**（`views/trace.rs`）：标题行「对话 | 轨迹」两段页签（不是浮层），打开后整区替换对话内容（状态条/消息流/输入框都不渲染）；列表是 USER/ASSISTANT/TOOL/HANDOFF 彩色标签 + Turn 标记，点条目出右侧 430px 详情分栏（Summary/Payload/Result/Timing；Payload 取 `ToolCall.arguments` 并 serde 美化、Result 取 `output`、Timing 取 Agent 的 elapsed/tokens）。数据全部来自 `Session.messages`——没有 system prompt / CONTEXT / 时间戳，**不要画假甘特图或补不存在的记录**。Esc 或切「对话」关闭并清选中与搜索词。搜索框走 `InputTarget::TraceSearch`：新增输入框要同步 `buffer()`/`buffer_mut()`/`active_buffer()`/`active_buffer_mut()` 与焦点 `want`/`handle` 四处。页签右侧「⇅ 用量」是**弹框**（`views/usage.rs` + `usage_ops.rs`，Esc/✕ 关闭，不是页签）：上半是会话自己厂商的余额（仅 DeepSeek/Moonshot/SiliconFlow/OpenRouter 有官方接口，其余显示不支持 + 控制台入口），下半是会话/工作区 token 统计（取 `Session.messages` 的 `tokens_in/out`）。

## 代码现状

- 编辑器：光标/多光标/选区/撤销重做/剪贴板/按字符点击/行号/高亮/虚拟滚动/保存/查找替换/折叠/括号匹配。
- 团队协作：开工具/关工具都从**入口角色**起步——`@角色名` 点名优先，没点名由 `roles[0]` 接手（不再把一句话派发给全员）。开工具走 `spawn_agents_chain` 的 handoff 链（入口角色带 handoff 工具、按需接力）；关工具只跑入口角色（纯文本、不传 tools，工具历史由 `flatten_tool_history` 压平，否则 Anthropic 空正文整轮 400）。编排模式（流水线/并行/主管指派）已从团队页移除，`FlowMode` 字段保留兼容、agent 侧 `run_team_with_history` 能力保留但 app 不再调用。
- Provider：OpenAI 兼容 / Anthropic / Gemini 原生；Gemini 工具调用未接。模型列表按协议拉（GET /models），失败回退手输。**内置预设换代**（厂商下线旧模型）：在 `ProviderPreset::legacy_model_sets` 登记旧内置列表，`config::load_settings` 会对仍与旧预设完全一致的配置自动升级并落盘；用户手动增删过模型的列表不动。智谱当前预设为 `glm-5.3` 一代（4.7 及更早已被官方自动切换）。
- **Agent 联网**：`web_search`（必应 RSS）+ `web_fetch`（HTML→纯文本，去 script/style/head，中文按 Content-Type charset 解码）；两者默认常开、只读不审批。
- 技能市场**只接真实源**：MCP 官方 Registry / 腾讯云 MCP 广场（国内直达）/ skills.sh（Skill 页唯一来源：首页热榜 + 搜索 API ≥2 字符，400ms 防抖；安装时解析上游真实 SKILL.md，GitHub 源先直试 `skills/<id>/SKILL.md` 再回退 jsDelivr，站点源走 `.well-known/agent-skills`；搜索 API 无 description，详情抽屉打开时才懒加载上游正文回填（`detail_loaded` 标记，安装时复用不二次请求）；正文/回答的 Markdown 链接渲染为可点击 span 走 `App::open_url`），热榜缓存在 `~/.neverwork/marketplace.json`（搜索结果只进内存，启动时缓存 10 分钟内不自动重拉）；`skills_catalog.rs` 仅保留可离线验证管道的真实 npm 内建项，**不要再加虚构条目**。MCP 子进程默认注入 npmmirror / 清华 PyPI 镜像（settings.mcp_mirror 可关）。
- 无 LSP / tree-sitter / 终端 / 全局搜索。Git 面板只读（`git status` 文件列表），**无暂存/提交/丢弃/切分支、无 push/pull、无冲突合并**。单文件 512KB、文件树 8000 节点上限。
- **远程 SSH 工作区**（`core/src/ssh.rs` + `ToolContext` 后端化）：连接串 `ssh://[user@]host[:port]/abs/path`（输入框也收 `user@host:/path`），认证优先系统 `ssh` 的密钥/agent。工作区管理器里的「连接远程 SSH」按钮与远程输入行（含密码框）已移除（用户要求），远程入口只剩「最近工作区」里的远程条目（需密钥/agent 或复用内存密码）与手输连接串（无密码）。**密码只在内存**（`AppState.remote_pw`），经 `SSH_ASKPASS` 回调应用本体（`main.rs` 最先处理 `NW_ASKPASS_MODE`），不得写 settings/日志/命令行；`RemoteAuth` 手写 `Debug` 打码，别给它 derive。会话键是连接串。远程工具路径一律相对工作区根，写操作仍逐次审批；远程 `search_code` 返回 UNSUPPORTED，提示词引导用 `search_files`。**远程树/读文件/保存都是后台线程 + `remote_gen` 代次丢弃迟到结果**；`refresh_tree`/`collapse_all`/`open_remote_file`/`save_file`/`open_file_name` 都有 `self.remote` 分支。连接串手输与重命名共用 `ws_focus`/`InputTarget::Workspace`，加新输入框要同步 `buffer()`/`buffer_mut()`/`active_buffer()`/`active_buffer_mut()`。改这些不要**把 `~` 展开结果写回 `SshTarget`**（会话键会变，旧会话像丢了）；Git 面板、watcher、终端不做。

<!-- proma:memory:start -->
## 记忆维护

- **个人层（常驻）**：[user_profile.md](<C:\Users\Administrator\.proma\memory\user_profile.md>)，跨项目共享的个人偏好。
- **项目护栏层（常驻）**：本文件（≤4000 非空白字符），只留子项目结构、常用命令与全局红线；模块细节、排障与状态快照下沉到项目 memory/。
- **专题手册层（按需）**：[MEMORY.md](<C:\Users\Administrator\.proma\agent-workspaces\ide\memory\MEMORY.md>) 一行一条场景路由索引（≤3500 字符），按索引按需读取；过期/已合并文件可恢复归档。读取不等于核验，不编造用户事实。
<!-- proma:memory:end -->
