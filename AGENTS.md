# 项目约定

## 这是什么

纯 Rust + GPUI（Zed 内核）自绘的 AI 原生 IDE「NeverWork」。`E:\AI\ide` 是工作区根，只放计划文档、设计稿与截图；**真实代码在子目录 `NeverWork/`**（独立 Cargo workspace，4 个 crate：`core` 领域模型+配置、`ui` 设计令牌+组件、`agent` LLM Provider+团队编排、`app` GPUI 界面与状态）。
`_nw_sandbox/` 是伪造的假沙盒，仅供 UI 测试，不是真实源码，不要在其中找实现。

## 常用命令（必须在 `NeverWork/` 目录下执行）

```bash
cargo run -p neverwork          # 主界面
cargo test --workspace          # 全部测试（40 个）
cargo build -p neverwork        # 只编应用（增量约 6s）
```

运行期环境变量：`NW_ROOT`（工作区根）、`NW_API_KEY` / `NW_PROVIDER=openai|anthropic` / `NW_BASE_URL` / `NW_MODEL`（AI 配置）、`NW_VIEW=team`、`NW_OPEN=<path>`（启动即开文件）、`NW_WIN_W` / `NW_WIN_H`（覆盖窗口逻辑尺寸）、`NW_CONFIG_DIR`（覆盖 `~/.neverwork`）。

## 红线

- **仓库根是 `E:\AI\ide`（2026-11-15 起纳入 git，远程 `gitee.com/ZLH1995123/neverwork`）**，`NeverWork/` 是其中的子目录，不是独立仓库。`NeverWork/target`（约 9.7G）已在根 `.gitignore` 中忽略，**不要用 `git add -f` 强行加入**，也不要 `git clean -xfd`（会删构建产物）。
- 分支：`main` 为开发主线，`master` 是 Gitee 默认分支，两者保持同一提交；`develop/feature/release` 目前同样指向主线。
- **`NeverWork/target` 约 9.4G**。增量构建只需几秒，但 `target/debug/deps` 被删过，首次全量要重编约 700 个 crate；不要执行 `cargo clean`，也不要把慢构建当故障。
- 团队配置持久化在 `~/.neverwork/config.json`（临时文件+rename 原子写）。不要删除或手改用户配置，测试请用 `NW_CONFIG_DIR` 指向临时目录。
- `design/`、`pic/` 是设计交付与运行截图，`message.md` 是历史交付汇报，都不要当作待改代码。
- 顶层三份计划文档是历史演进：`rust-ide-plan.md` → `gpui-ide-plan.md` → `gpui-ide-plan-v2.md`（最新口径）。冲突时以 v2 和 README 为准。

## 输入与交互的架构约束（改这里前必读）

- **文本插入只有一条路径**：`EntityInputHandler`（`input_handler.rs`）→ `TextBuffer`。**绝对不要**在 `on_key_down` 里对 `key_char` 做插入，那会和 IME 重复写入，之前的 bug 就是这么来的。键盘只管非文本按键（`keymap.rs`）。
- 光标/选区/撤销统一在 `text_buffer.rs`，对话输入框与编辑器共用；单行/多行只由 `multiline` 区分。
- 带 `cursor_pointer` / `hover` 的元素必须有 `on_click` 或 `on_mouse_down`，否则就是骗人的死点击。
- 编辑器按字符渲染成可点击单元，因此点击能精确落位；行内容走 `line_cache.rs` 缓存，不要在渲染里直接跑 `highlight_line`。
- **文字截断不要只依赖 `.truncate()`**：gpui 只在首帧测量时已有确定宽度才生效，`flex_1` 嵌套下会硬裁且不出省略号。需要省略号时用 `views::text_fit::fit_text(text, 预算像素, 字号)`，并给元素 `.whitespace_nowrap()`。
- **布局优先级**：中栏 > 左侧会话栏 > 右侧文件栏。窄窗口先隐藏右栏（<1080 逻辑宽），极窄（<620）才隐藏会话列表，48px 图标栏永远保留。`window.bounds()` 已是逻辑像素，不要再除以 `scale_factor`。

## 左侧栏结构（`views/sidebar.rs`）

`RAIL_W=52` 图标栏（对话/团队/编辑器/命令 + AI 状态 + 头像）+ `SESSION_W=264` 会话卡片列表。
卡片两行：`头像 + 徽标 + 标题 +（右）状态圆圈` / `摘要 + 时间 + 状态胶囊`；无边框，选中项亮底。
会话展示字段在 `core::Session`（`preview/status/meta/accent/icon/badge/action`，均带 serde 默认值）+ `SessionStatus`/`SessionAction` 枚举。
**卡片内任何文本都要算准宽度**：`CARD_INNER = SESSION_W - LIST_PAD*2 - CARD_PAD*2`，漏算任一层内边距都会让右侧胶囊被裁。

## 环境特性（容易误判）

本机是 GameViewer 远程会话 + 200% DPI：GPUI 用 `GetDpiForWindow/96` 得到 scale=2，但窗口按 1:1 物理像素创建，内容被放大一倍并裁掉右侧，`window.resize` 也压不住。因此布局用 `visible_w = bounds.width / scale` 做响应式折叠（<1100 收右栏、<820 只留中栏），标题栏可用「◧ 专注」手动切换。**常规显示器（scale=1）下三栏正常，不要为此改写布局常量。** 自动化点按/键盘注入在该环境坐标被 DPI 虚拟化，且前台焦点常被其他进程抢占，无法作为可靠验收手段。

## 代码现状与已知缺口

- 编辑器已具备：光标、选区、撤销/重做、剪贴板、按字符点击、行号、语法高亮、虚拟滚动、保存（原子写）。仍**没有**查找替换、多光标、代码折叠、自动缩进重排、括号匹配。
- 编排模式 `Supervisor`（主管指派）在 `agent/src/team.rs` 里**等同并行**，尚未真正分派。
- Team 编排只是"多模型轮流发言"：角色没有工具调用、读不到工作区文件、产出不落盘成 diff。
- Provider 只实现 OpenAI 兼容与 Anthropic；按模型名 `claude` 前缀硬编码切 Provider，新增厂商需改 `role_provider`。
- 无 LSP / tree-sitter（`highlight.rs` 是自写词法扫描）/ 终端 / Git 集成 / 全局搜索；单文件读取上限 512KB，文件树节点上限 8000。

<!-- proma:memory:start -->
## 记忆维护

- **个人层（常驻）**：[user_profile.md](<C:\Users\Administrator\.proma\memory\user_profile.md>)，跨项目共享，仅记录换个项目也成立的个人背景与稳定偏好。
- **项目护栏层（常驻）**：本文件（AGENTS.md，预算 ≤4000 非空白字符），仅保留不看就会在本项目犯错的子项目结构、常用命令与全局红线；具体模块细节、排障经过和某日状态快照一律下沉到项目 memory/。
- **专题手册层（按需）**：[MEMORY.md](<C:\Users\Administrator\.proma\agent-workspaces\ide\memory\MEMORY.md>) 仅维护一行一条的场景路由索引（预算 ≤3500 字符，不写长篇结论摘要），详细主题按索引按需读取；过期或已合并的存量文件（含历史 project-preferences.md）通过可恢复归档移出有效入口。读取不等于核验，不编造用户事实。
<!-- proma:memory:end -->
