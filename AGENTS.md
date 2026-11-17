# 项目约定

## 项目

纯 Rust + GPUI（Zed 内核）的 AI 原生 IDE「NeverWork」。`E:\AI\ide` 只放文档、设计稿与截图；**真实代码在 `NeverWork/`**（4 crate：`core` 模型+配置、`ui` 令牌+组件、`agent` Provider+工具+编排、`app` 界面+状态）。

## 常用命令

```bash
cargo run -p neverwork          # 主界面
cargo test --workspace          # 全部测试（3 条打真实 DeepSeek，需 NW_DEEPSEEK_KEY）
cargo build -p neverwork        # 只编应用（增量约 6s）
```

运行期变量：`NW_ROOT`、`NW_REMOTE`（`user@host:/path`）、`NW_API_KEY`/`NW_PROVIDER`/`NW_BASE_URL`/`NW_MODEL`、`NW_VIEW`、`NW_SETTINGS_TAB`、`NW_OPEN`、`NW_WIN_W`/`NW_WIN_H`、`NW_MAXIMIZE`、`NW_CONFIG_DIR`；验证专用 `NW_SEND`/`NW_AUTO_APPROVE`/`NW_TEAM`/`NW_RIGHT_COLLAPSED`/`NW_SESSION_COLLAPSED`/`NW_FIND`/`NW_FOLD`/`NW_CURSORS`/`NW_PICK_MODEL`/`NW_PROVIDER_VIEW`（`add`|下标）/`NW_COMPOSER`（`thinking`|`menu`）/`NW_LAYOUT_DEBUG`/`NW_MARKET_SOURCE`/`NW_MARKET_NO_REFRESH`/`NW_SKILLS_TAB`/`NW_SKILL_QUERY`/`NW_INSTALL_SKILL`/`NW_SESSION_QUERY`/`NW_SESSION_MENU`/`NW_ARCHIVE_OPEN`/`NW_WORKSPACE_POPUP`/`NW_DEMO_STATUS`/`NW_SWITCH_WS`；`NW_ASKPASS_MODE`/`NW_ASKPASS_SECRET` 由 ssh 回调应用自身时使用，不要手动设置；密码往返测试用 `NW_ASKPASS_EXE` 指向 `target/debug/neverwork.exe`。

工作区回落：`NW_ROOT` → `settings.last_workspace` → `~/.neverwork/workspace`。**不要**回落到当前目录/向上找 `Cargo.toml`——会把 IDE 自己的仓库当工作区。`last_workspace` 是 `ssh://` 时走远程工作区（本地 `root` 为 `None`）。

## 红线

- **仓库根是 `E:\AI\ide`（远程 `gitee.com/ZLH1995123/neverwork`）**，`NeverWork/` 是子目录而非独立仓库。`NeverWork/target` 已 gitignore（**不要 `git add -f` / `git clean -xfd` / `cargo clean`**，全量重编约 700 crate）。
- 分支：`main` 开发主线，`master` 是 Gitee 默认分支，两者同提交；`develop/feature/release` 同指主线。
- 应用进程没关时 `cargo build` 会报「failed to remove neverwork.exe」但**返回码仍是 0**，跑的是旧二进制——先 taskkill 再构建。
- 配置在 `~/.neverwork/`：`config.json`（团队）、`settings.json`（偏好，**含 Key 明文**，别提交别外传），均原子写。测试用 `NW_CONFIG_DIR`。
- **主题必须走 `theme::set_theme_mode()`**，不要在视图里写死颜色——`ui/theme.rs` 的 `PALETTES` 有 9 套完整色板（深色/浅色/跟随系统/Nord/Dracula/One Dark/Solarized 浅深/Gruvbox），写死 `rgb(0x…)` 只会适配其中一套；新增颜色要同步加到每套色板。
- **API Key 一律脱敏**：导出/同步走 `config::export_all(..., ExportOptions::default())`；界面只显示 `masked_key()`；不要给 `ProviderEntry` 加回 `derive(Debug)`（手写版会打码）。新增 Settings 序列化到文件/日志的路径先确认 Key 不跟着走。
- `design/`、`pic/` 是设计截图。
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
- **弹层遮罩必须 `.occlude()`**：gpui 的滚轮命中取「鼠标下所有 hitbox」（`should_handle_scroll`），遮罩不 occlude 时滚弹层内容会同时滚背后的滚动容器（技能详情抽屉踩过：滚正文整页跟着下移）。palette / session_menu / workspace_picker / new_session / market_detail 五个遮罩都走 occlude。
- **窗口是无边框自绘的**（`main.rs` 里 `titlebar: Some(TitlebarOptions { appears_transparent: true, .. })`）。**别退回 `WindowOptions::default()` 的 `appears_transparent: false`**——那样 Windows 会在最上面叠一条系统原生标题栏（浅色横条 + 左上角白色默认窗口图标 + 右键「还原/移动/关闭」菜单）。
- 标题栏拖动 / 最小化 / 最大化(还原) / 关闭由 `views::titlebar_hitboxes`（绝对定位 canvas）注册系统热区 `Drag`/`Min`/`Max`/`Close` 处理，点击不经过 `on_click`；右侧 300px 不铺 Drag 热区，否则 HTCAPTION 会吞掉「专注/窗口按钮」的点击。
- 窗口按钮图标必须用 `div` 画（横线/方框/错位双框），**不要用 `□`/`❐` 字符**：字体回退后两者几乎一样，最大化前后看不出区别。最大化状态在 `render` 里用 `window.is_maximized()` 传给 `titlebar`，窗口尺寸变化会触发重绘。

## 左侧栏

`RAIL_W=52` + `SESSION_W=264` 会话卡片（头部标题 + ＋新建 + ◀收起，头像+标题+状态 / 摘要+时间+胶囊）。卡片内宽 `CARD_INNER = SESSION_W - LIST_PAD*2 - CARD_PAD*2`，漏算内边距会裁右侧胶囊。会话操作是**右键上下文菜单**（`views/session_menu.rs`，全屏透明遮罩 + 232px 绝对定位卡片，行=语义图标+13px 文字；宽度用 `session_menu::MENU_W` 与 state 的 clamp 共用）；右键卡片与点卡片的 ⋯ 都走 `on_mouse_down` 取鼠标坐标（`ClickEvent` 不带坐标），渲染侧用 `visible_w/h` clamp 反向弹出防出界。筛选/归档判定用 core 的 `session_matches_list`/`is_archived_session`，不要在视图里另写一套。

## 环境特性

逻辑屏 1280x800、物理 2560x1600、`scale_factor()`=2；`bounds()`/`display.bounds()` 都是逻辑值，别除 scale。

**验收截图须 DPI-aware**：PowerShell 默认 unaware 只抓到左上 1/4，会伪造「内容放大、右侧出界」；先 `SetProcessDpiAwarenessContext(-4)`（见 `design/shot.ps1`）。

## Agent 工具（改前必读）

- 工具在 `agent/src/tools.rs`。**所有路径走 `ToolContext::resolve`**（挡 `..`、跨盘绝对路径、符号链接逃逸）。
- **联网工具在 `agent/src/web.rs`**：`web_search`（必应 RSS，不引 Key）/`web_fetch`（只收公网 http/https，拒 localhost 与内网，12s + 300KB 上限，HTML 转纯文本）。两者只读、`requires_approval = false`、**默认常开不设开关**；返回正文与文件内容一样走 `wrap_untrusted`，别让网页内容绕过这层。
- **写操作必须 `requires_approval = true`**，经 `NeedApproval` → 用户允许后才执行；不要在工具内部直接落盘。不新增 shell。
- 团队角色的 `model` 是**展示标签**（`Claude Opus`），不能当模型 id 或据此切厂商——只有全小写真实 id 才算。
- **运行上限三项（tokens/rounds/seconds）默认 0=不限**，从 settings 读；不要加回常量上限（读中型项目必被掐断）。防失控靠 `MAX_REPEAT=3`（同工具同参数）与手动停止。

## Git 写操作（改前必读）

- 写操作在 `app/src/git_ops.rs`。**只有「丢弃」是破坏性的**（`is_destructive()`），必须二次确认。
- **多仓库且未选中时禁止写操作**（`git_repo_chosen()`）：自动选第一个会让 `git add -A` 落到用户没点选的仓库，真实踩过。提交前查暂存区与信息非空，失败带回 git stderr 原文。

## 对话区与查找（改前先读）

- 工具调用由 `views/chat.rs::tool_steps_group` 把连续调用聚合成**一行统计**（线程 / 思考轮次 / 工具数 / 动作直方图），逐条路径与输出不再展示；不要再加展开明细的入口。写操作审批必须单独显示 diff 与「拒绝 / 允许并写入」。工具消息里的 `thread`（角色序号）/`round`（该角色第几轮模型调用）用于统计，旧会话缺省 0。
- 模型/厂商是**会话级**：`Session.provider_id`/`model`（空 = 跟随设置里的当前厂商），头部 chip 切换、按厂商分组；新会话继承上一个会话的选择（厂商被删/停用则回「跟随默认」）。
- 移除厂商是两步确认（`provider_remove_confirm`）：Key 会随配置一起删掉，不允许一点就没了。
- **思考等级是会话级**（`Session.thinking`，实现见 `views/chat.rs`）：`自动` 不向厂商发任何思考参数，只有低/中/高才映射 `reasoning_effort` / Anthropic `thinking.budget_tokens` / Gemini `thinkingConfig`。Anthropic 开思考必须 `temperature=1` 且 `max_tokens > budget`；Gemini 思考 token 计入输出，`maxOutputTokens` 要同比抬高。改这几处先看 `agent` 里对应的单测。
- 思考滑杆用**窗口级 `window.on_mouse_event`**（在 canvas paint 里注册）跟踪拖动，不要换成元素级 `on_mouse_move`：后者只在悬停时触发，光标滑出 256px 轨道就断，滑杆会粘在半路。拖动中只 `preview_thinking_level` 改内存，松开鼠标才 `save_sessions`。
- 输入框不再做自动联想（本地历史补全与模型补全都已移除）：不要在 `after_edit`/`on_chat_key` 里挂补全请求，聊天框的 Tab 不做任何事。
- **思考档位入口默认隐藏**：不常驻输入框底栏，从「＋」菜单（`⚙ 思考 · 档位`）进入滑杆面板；不要再加回右下 chip。面板互斥（开一个关另一个）与拖动实现见上一条。
- 查找替换在 `app/src/find.rs`（纯逻辑）+ `views/editor.rs::find_bar`（UI），按字节区间匹配，中文安全。

## 代码现状

- 编辑器：光标/多光标/选区/撤销重做/剪贴板/按字符点击/行号/高亮/虚拟滚动/保存/查找替换/折叠/括号匹配。
- `Supervisor` 是真分派（Lead 逐轮决定下一角色）。Team 开工具走 `spawn_agents_chain`（串行、每角色带工具、与单 Agent 同一套历史）；关工具走 `run_team_with_history`（纯文本、不传 tools，工具历史由 `flatten_tool_history` 压平，否则 Anthropic 空正文整轮 400）。
- Provider：OpenAI 兼容 / Anthropic / Gemini 原生；Gemini 工具调用未接。模型列表按协议拉（GET /models），失败回退手输。
- **Agent 联网**：`web_search`（必应 RSS）+ `web_fetch`（HTML→纯文本，去 script/style/head，中文按 Content-Type charset 解码）；两者默认常开、只读不审批。
- 技能市场**只接真实源**：MCP 官方 Registry / 腾讯云 MCP 广场（国内直达）/ skills.sh（Skill 页唯一来源：首页热榜 + 搜索 API ≥2 字符，400ms 防抖；安装时解析上游真实 SKILL.md，GitHub 源先直试 `skills/<id>/SKILL.md` 再回退 jsDelivr，站点源走 `.well-known/agent-skills`；搜索 API 无 description，详情抽屉打开时才懒加载上游正文回填（`detail_loaded` 标记，安装时复用不二次请求）；正文/回答的 Markdown 链接渲染为可点击 span 走 `App::open_url`），热榜缓存在 `~/.neverwork/marketplace.json`（搜索结果只进内存，启动时缓存 10 分钟内不自动重拉）；`skills_catalog.rs` 仅保留可离线验证管道的真实 npm 内建项，**不要再加虚构条目**。MCP 子进程默认注入 npmmirror / 清华 PyPI 镜像（settings.mcp_mirror 可关）。
- 无 LSP / tree-sitter / 终端 / 全局搜索。Git 有暂存/提交/丢弃/切分支，**无 push/pull、无冲突合并**。单文件 512KB、文件树 8000 节点上限。
- **远程 SSH 工作区**（`core/src/ssh.rs` + `ToolContext` 后端化）：连接串 `ssh://[user@]host[:port]/abs/path`（输入框也收 `user@host:/path`），认证优先系统 `ssh` 的密钥/agent；只在密码登录的机器可在连接行填密码。**密码只在内存**（`AppState.remote_pw`），经 `SSH_ASKPASS` 回调应用本体（`main.rs` 最先处理 `NW_ASKPASS_MODE`），不得写 settings/日志/命令行；`RemoteAuth` 手写 `Debug` 打码，别给它 derive。会话键是连接串。远程工具路径一律相对工作区根，写操作仍逐次审批；远程 `search_code` 返回 UNSUPPORTED，提示词引导用 `search_files`。**远程树/读文件/保存都是后台线程 + `remote_gen` 代次丢弃迟到结果**；`refresh_tree`/`collapse_all`/`open_remote_file`/`save_file`/`open_file_name` 都有 `self.remote` 分支。连接串与密码共用 `ws_focus`/`InputTarget::Workspace`，靠 `ws_pw_focus` 在 `buffer()`/`active_buffer()` 里分流，加新输入框要同步这三处。改这些不要**把 `~` 展开结果写回 `SshTarget`**（会话键会变，旧会话像丢了）；Git 面板、watcher、终端不做。

<!-- proma:memory:start -->
## 记忆维护

- **个人层（常驻）**：[user_profile.md](<C:\Users\Administrator\.proma\memory\user_profile.md>)，跨项目共享的个人偏好。
- **项目护栏层（常驻）**：本文件（≤4000 非空白字符），只留子项目结构、常用命令与全局红线；模块细节、排障与状态快照下沉到项目 memory/。
- **专题手册层（按需）**：[MEMORY.md](<C:\Users\Administrator\.proma\agent-workspaces\ide\memory\MEMORY.md>) 一行一条场景路由索引（≤3500 字符），按索引按需读取；过期/已合并文件可恢复归档。读取不等于核验，不编造用户事实。
<!-- proma:memory:end -->
