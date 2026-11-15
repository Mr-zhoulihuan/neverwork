# 项目约定

## 这是什么

纯 Rust + GPUI（Zed 内核）自绘的 AI 原生 IDE「NeverWork」。`E:\AI\ide` 只放计划文档、设计稿与截图；**真实代码在 `NeverWork/`**（4 crate：`core` 模型+配置、`ui` 令牌+组件、`agent` Provider+工具+编排、`app` 界面与状态）。`_nw_sandbox/` 是假沙盒，不要在里面找实现。

## 常用命令（在 `NeverWork/` 下执行）

```bash
cargo run -p neverwork          # 主界面
cargo test --workspace          # 全部测试（129 个，其中 3 条打真实 DeepSeek 需 NW_DEEPSEEK_KEY）
cargo build -p neverwork        # 只编应用（增量约 6s）
```

运行期变量：`NW_ROOT`、`NW_API_KEY`/`NW_PROVIDER`/`NW_BASE_URL`/`NW_MODEL`、`NW_VIEW=chat|team|skills|settings`、`NW_SETTINGS_TAB`、`NW_OPEN=<path>`、`NW_WIN_W`/`NW_WIN_H`、`NW_CONFIG_DIR`；验证专用 `NW_SEND`、`NW_AUTO_APPROVE`、`NW_EXPAND_STEPS`、`NW_RIGHT_COLLAPSED`、`NW_FIND`、`NW_PICK_MODEL`、`NW_LAYOUT_DEBUG`。

## 红线

- **仓库根是 `E:\AI\ide`（2026-11-15 起纳入 git，远程 `gitee.com/ZLH1995123/neverwork`）**，`NeverWork/` 是其中的子目录，不是独立仓库。`NeverWork/target`（约 9.7G）已在根 `.gitignore` 中忽略，**不要用 `git add -f` 强行加入**，也不要 `git clean -xfd`（会删构建产物）。
- 分支：`main` 为开发主线，`master` 是 Gitee 默认分支，两者保持同一提交；`develop/feature/release` 目前同样指向主线。
- **`NeverWork/target` 约 9.4G**。增量几秒可用，但 `target/debug/deps` 被删过，首次全量要重编约 700 crate；不要 `cargo clean`，也别把慢构建当故障。注意：应用进程没关时 `cargo build` 会报「failed to remove neverwork.exe」，此时跑的还是旧二进制——先 taskkill 再构建。
- 配置在 `~/.neverwork/`：团队 `config.json`、偏好 `settings.json`（**含 API Key 明文**，别提交别外传），均临时文件+rename 原子写。不要手改用户配置，测试用 `NW_CONFIG_DIR` 指向临时目录。
- **主题必须走 `theme::set_theme_mode()`**，不要在视图里直接写死颜色代码——深浅两套色板由全局原子量切换，直接写 `rgb(0x…)` 的元素在浅色主题下会不可读。
- **API Key 一律脱敏**：导出/同步必须走 `config::export_all(..., ExportOptions::default())`（不含密钥）；界面只显示 `masked_key()`（末尾 4 位）；不要给 `ProviderEntry` 加回 `derive(Debug)`，它手写的 Debug 会打码。新增任何把 Settings 序列化到文件或日志的路径，先确认 Key 不会跟着走。
- `design/`、`pic/` 是设计交付与截图，`message.md` 是历史汇报，不要当待改代码。
- 顶层三份计划文档是历史演进：`rust-ide-plan.md` → `gpui-ide-plan.md` → `gpui-ide-plan-v2.md`（最新口径）。冲突时以 v2 和 README 为准。

## 输入与交互的架构约束（改这里前必读）

- **文本插入只有一条路径**：`EntityInputHandler`（`input_handler.rs`）→ `TextBuffer`。**绝对不要**在 `on_key_down` 里对 `key_char` 做插入，会和 IME 重复写入。键盘只管非文本按键（`keymap.rs`）。
- 光标/选区/撤销统一在 `text_buffer.rs`，对话输入框与编辑器共用。
- 带 `cursor_pointer`/`hover` 的元素必须有 `on_click` 或 `on_mouse_down`，否则是死点击。
- 编辑器按字符渲染成可点击单元；行内容走 `line_cache.rs`，不要在渲染里跑 `highlight_line`。
- **文字截断别只靠 `.truncate()`**：gpui 首帧没确定宽度就不生效，`flex_1` 嵌套下会硬裁且无省略号。要省略号用 `views::text_fit::fit_text(text, 预算px, 字号)`，并加 `.whitespace_nowrap()`。
- **布局优先级**：中栏 > 会话栏 > 文件栏。窄窗口先隐藏右栏（<1080），极窄（<620）才隐藏会话列表，52px 图标栏永远保留。`window.bounds()` 已是逻辑像素，不要再除 `scale_factor`。

## 左侧栏（`views/sidebar.rs`）

`RAIL_W=52` 图标栏 + `SESSION_W=264` 会话卡片（两行：头像+徽标+标题+状态圆圈 / 摘要+时间+状态胶囊，无边框选中亮底）。
**卡片内文本必须算准宽度**：`CARD_INNER = SESSION_W - LIST_PAD*2 - CARD_PAD*2`，漏算任一层内边距会让右侧胶囊被裁。

## 环境特性（容易误判）

本机是 GameViewer 远程会话 + 200% DPI：GPUI 由 `GetDpiForWindow/96` 得 scale=2，但窗口按 1:1 物理像素创建，只显示逻辑画布的左上四分之一，`window.resize` 压不住。**常规显示器（scale=1）下三栏正常，不要为此改写布局常量。** 自动化点按在该环境坐标被虚拟化，只有左上象限点得到（右侧栏、聊天区底部含审批按钮都点不到）；验收靠集成测试 + `NW_*` 验证开关截图。

## Agent 工具调用（改这里前必读）

- 工具在 `agent/src/tools.rs`。**所有路径走 `ToolContext::resolve`**（挡 `..`、跨盘绝对路径、符号链接逃逸），新增工具不要自己拼路径。
- **写操作必须 `requires_approval = true`**，经 `NeedApproval` → 用户点允许后才执行；不要在工具内部直接落盘绕过审批。
- 团队角色的 `model` 是**展示标签**（`Claude Opus`），不能当模型 id 或据此切厂商——只有全小写真实 id 才算。
- 不新增 shell/exec 类工具。

## 对话区与查找（改动前先读）

- 工具调用在 `views/chat.rs` 里由 `tool_steps_group` 按「连续同类消息」聚合渲染，单条 `render_msg` 不再处理 `ToolCall`；改这块要同步 `expanded_steps` 的键（组首条消息下标）。
- 待审批的组必须自动展开，否则审批按钮不可见。
- 查找替换在 `app/src/find.rs`（纯逻辑）+ `views/editor.rs::find_bar`（UI）；匹配按字节区间，中文安全。
- 会话级改动汇总走 `core::aggregate_changes`，不要在视图里重复实现。

## 代码现状与已知缺口

- 编辑器已有：光标、选区、撤销/重做、剪贴板、按字符点击、行号、高亮、虚拟滚动、保存、查找替换。仍**没有**多光标、代码折叠、括号匹配。
- `Supervisor`（主管指派）**等同并行**，尚未真正分派。
- Team 编排（多角色）仍是"多模型轮流发言"，不调用工具；带工具的是单 Agent 路径（用团队第一个角色的人格）。
- Provider 只有 OpenAI 兼容与 Anthropic 两套协议；Gemini 原生尚未实现（选 Gemini 会走 OpenAI 兼容路径）。
- 无 LSP / tree-sitter / 终端 / Git 集成 / 全局搜索；单文件读取 512KB 上限，文件树 8000 节点上限。

<!-- proma:memory:start -->
## 记忆维护

- **个人层（常驻）**：[user_profile.md](<C:\Users\Administrator\.proma\memory\user_profile.md>)，跨项目共享，仅记录换个项目也成立的个人背景与稳定偏好。
- **项目护栏层（常驻）**：本文件（AGENTS.md，预算 ≤4000 非空白字符），仅保留不看就会在本项目犯错的子项目结构、常用命令与全局红线；具体模块细节、排障经过和某日状态快照一律下沉到项目 memory/。
- **专题手册层（按需）**：[MEMORY.md](<C:\Users\Administrator\.proma\agent-workspaces\ide\memory\MEMORY.md>) 仅维护一行一条的场景路由索引（预算 ≤3500 字符，不写长篇结论摘要），详细主题按索引按需读取；过期或已合并的存量文件（含历史 project-preferences.md）通过可恢复归档移出有效入口。读取不等于核验，不编造用户事实。
<!-- proma:memory:end -->
