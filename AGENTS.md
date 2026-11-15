# 项目约定

## 这是什么

纯 Rust + GPUI（Zed 内核）自绘的 AI 原生 IDE「NeverWork」。`E:\AI\ide` 只放文档、设计稿与截图；**真实代码在 `NeverWork/`**（4 crate：`core` 模型+配置、`ui` 令牌+组件、`agent` Provider+工具+编排、`app` 界面+状态）。`_nw_sandbox/` 是假沙盒，别在里面找实现。

## 常用命令（`NeverWork/` 下）

```bash
cargo run -p neverwork          # 主界面
cargo test --workspace          # 全部测试（247 个；3 条打真实 DeepSeek，需 NW_DEEPSEEK_KEY）
cargo build -p neverwork        # 只编应用（增量约 6s）
```

运行期变量：`NW_ROOT`、`NW_API_KEY`/`NW_PROVIDER`/`NW_BASE_URL`/`NW_MODEL`、`NW_VIEW`、`NW_SETTINGS_TAB`、`NW_OPEN`、`NW_WIN_W`/`NW_WIN_H`、`NW_MAXIMIZE`、`NW_CONFIG_DIR`；验证专用 `NW_SEND`/`NW_AUTO_APPROVE`/`NW_EXPAND_STEPS`/`NW_RIGHT_COLLAPSED`/`NW_FIND`/`NW_FOLD`/`NW_CURSORS`/`NW_PICK_MODEL`/`NW_LAYOUT_DEBUG`。

## 红线

- **仓库根是 `E:\AI\ide`（远程 `gitee.com/ZLH1995123/neverwork`）**，`NeverWork/` 是子目录而非独立仓库。`NeverWork/target`（约 9.7G）已在根 `.gitignore` 忽略，**不要 `git add -f`**，也不要 `git clean -xfd`。
- 分支：`main` 开发主线，`master` 是 Gitee 默认分支，两者同提交；`develop/feature/release` 同指主线。
- **`NeverWork/target` 约 9.4G**。增量几秒可用，但 `target/debug/deps` 被删过，首次全量重编约 700 crate；不要 `cargo clean`。应用进程没关时 `cargo build` 会报「failed to remove neverwork.exe」但**返回码仍是 0**，跑的是旧二进制——先 taskkill 再构建。
- 配置在 `~/.neverwork/`：`config.json`（团队）、`settings.json`（偏好，**含 Key 明文**，别提交别外传），均原子写。测试用 `NW_CONFIG_DIR`。
- **主题必须走 `theme::set_theme_mode()`**，不要在视图里写死颜色——深浅两套色板由全局原子量切换，写死 `rgb(0x…)` 在浅色主题下不可读。
- **API Key 一律脱敏**：导出/同步走 `config::export_all(..., ExportOptions::default())`（不含密钥）；界面只显示 `masked_key()`（末尾 4 位）；不要给 `ProviderEntry` 加回 `derive(Debug)`（手写版会打码）。新增任何序列化 Settings 到文件或日志的路径，先确认 Key 不跟着走。
- `design/`、`pic/` 是设计与截图。
- 计划文档演进：`rust-ide-plan.md` → `gpui-ide-plan.md` → `gpui-ide-plan-v2.md`（最新）。冲突以 v2 和 README 为准。

## 输入与交互的架构约束（改这里前必读）

- **文本插入只有一条路径**：`EntityInputHandler`（`input_handler.rs`）→ `TextBuffer`。**绝不要**在 `on_key_down` 里插 `key_char`，会和 IME 重复写入。键盘只管非文本键（`keymap.rs`）。
- 光标/选区/撤销统一在 `text_buffer.rs`，输入框与编辑器共用。
- 带 `cursor_pointer`/`hover` 的元素必须有 `on_click`/`on_mouse_down`，否则是死点击。
- 编辑器按字符渲染成可点击单元；行内容走 `line_cache.rs`，别在渲染里跑 `highlight_line`。
- **多光标是 `TextBuffer` 独立层**（`extra_cursors`），只有 `*_multi` 系列碰它。
- **折叠不重排文本**：只跳过隐藏行，真实行号映射回光标/选区。**`multi_patch` 必须先全量校验再写**，失败回滚。
- **文字截断别只靠 `.truncate()`**：gpui 首帧没确定宽度就不生效，`flex_1` 嵌套下会硬裁且无省略号。要省略号用 `views::text_fit::fit_text(text, 预算px, 字号)` + `.whitespace_nowrap()`。
- **布局优先级**：中栏 > 会话栏 > 右栏。窄窗口把右栏收成 16px 窄条（<1080），极窄（<620）才隐藏会话列表，图标栏永远保留。宽度判据用「可见尺寸」（见环境特性）。

## 左侧栏（`views/sidebar.rs`）

`RAIL_W=52` 图标栏 + `SESSION_W=264` 会话卡片（两行：头像+标题+状态圆圈 / 摘要+时间+状态胶囊）。
**卡片内文本要算准宽度**：`CARD_INNER = SESSION_W - LIST_PAD*2 - CARD_PAD*2`，漏算内边距会裁右侧胶囊。

## 环境特性（容易误判）

逻辑屏 1280x800、物理屏 2560x1600、`scale_factor()`=2。`bounds()` 与 `display.bounds()` 都是**逻辑值**，按逻辑值布局即可，不要再除 scale；根容器用 `size_full()`，`visible_w` 只判断点。

**验收截图须 DPI-aware**：PowerShell 默认 unaware，`CopyFromScreen` 只拿到左上 1/4，会伪造「内容被放大、右侧出界」的假象。先 `SetProcessDpiAwarenessContext(-4)` 再抓（见 `design/shot.ps1`）。

## Agent 工具调用（改这里前必读）

- 工具在 `agent/src/tools.rs`。**所有路径走 `ToolContext::resolve`**（挡 `..`、跨盘绝对路径、符号链接逃逸）。
- **写操作必须 `requires_approval = true`**，经 `NeedApproval` → 用户允许后才执行；不要在工具内部直接落盘。不新增 shell。
- 团队角色的 `model` 是**展示标签**（`Claude Opus`），不能当模型 id 或据此切厂商——只有全小写真实 id 才算。

## Git 写操作（改这里前必读）

- 写操作在 `app/src/git_ops.rs`。**只有「丢弃」是破坏性的**（`is_destructive()`），必须二次确认。
- **多仓库且未选中时禁止写操作**（`git_repo_chosen()`）：自动选第一个会让 `git add -A` 落到用户没点选的仓库，真实踩过。提交前查暂存区与信息非空，失败带回 git stderr 原文。

## 对话区与查找（改动前先读）

- 工具调用由 `views/chat.rs::tool_steps_group` 按「连续同类消息」聚合；改这块要同步 `expanded_steps` 的键（组首条下标）。待审批的组必须自动展开。
- 查找替换在 `app/src/find.rs`（纯逻辑）+ `views/editor.rs::find_bar`（UI），按字节区间匹配，中文安全。

## 代码现状与已知缺口

- 编辑器：光标、多光标、选区、撤销重做、剪贴板、按字符点击、行号、高亮、虚拟滚动、保存、查找替换、折叠、括号匹配。
- `Supervisor` 是真分派（Lead 逐轮决定下一角色）。Team 多角色编排仍是"轮流发言"、不调工具；带工具的是单 Agent 路径。
- Provider：OpenAI 兼容 / Anthropic / Gemini 原生；Gemini 工具调用未接。
- 无 LSP / tree-sitter / 终端 / 全局搜索。Git 有暂存/提交/丢弃/切分支，**无 push/pull、无冲突合并**。单文件 512KB、文件树 8000 节点上限。

<!-- proma:memory:start -->
## 记忆维护

- **个人层（常驻）**：[user_profile.md](<C:\Users\Administrator\.proma\memory\user_profile.md>)，跨项目共享，仅记录换个项目也成立的个人背景与稳定偏好。
- **项目护栏层（常驻）**：本文件（AGENTS.md，预算 ≤4000 非空白字符），仅保留不看就会在本项目犯错的子项目结构、常用命令与全局红线；具体模块细节、排障经过和某日状态快照一律下沉到项目 memory/。
- **专题手册层（按需）**：[MEMORY.md](<C:\Users\Administrator\.proma\agent-workspaces\ide\memory\MEMORY.md>) 仅维护一行一条的场景路由索引（预算 ≤3500 字符，不写长篇结论摘要），详细主题按索引按需读取；过期或已合并的存量文件（含历史 project-preferences.md）通过可恢复归档移出有效入口。读取不等于核验，不编造用户事实。
<!-- proma:memory:end -->
