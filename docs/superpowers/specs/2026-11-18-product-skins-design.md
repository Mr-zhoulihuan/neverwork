# NeverWork 产品化皮肤设计（Graphite / Carbon / Ember）

日期：2026-11-18
状态：已确认，待实施

## 背景

现有 9 套主题（深色 / 浅色 / 跟随系统 / Nord / Dracula / One Dark / Solarized 浅深 /
Gruvbox）是通用编辑器配色，缺少自有产品气质。本次把第三方配色全部替换为三套自有
产品皮肤，并引入**形态 token**（圆角、字号）让皮肤不止换颜色。

约束（用户已确认）：

- **图标全部保留**：左侧手绘 SVG（`rail-*.svg`）、文件树/按钮 glyph、窗口按钮不动；
- **替换现有九套**：只保留深色 / 浅色 / 跟随系统 + Graphite / Carbon / Ember；
- **实现深度 = 配色 + 圆角字号**：新增形态 token 并应用到主界面；
- 布局、间距、结构、交互不变。

## 决策摘要

| 决策 | 结论 |
| --- | --- |
| 默认主题 | **Graphite**（新装）；老用户已选 Dark/Light 不迁移 |
| 落地范围 | **主界面优先**：标题栏、会话栏、对话流、右栏、弹层、设置/厂商页 |
| 未纳入本轮 | 团队 / 技能 / 市场三个独立页面只跟随新色板自动变色（共享组件仍会部分跟随） |
| Carbon mono | **启用**：token 统计、余额数字、文件大小用等宽字体 |

## 1. 主题模型与迁移

### 1.1 ThemeMode

`core/settings.rs` 的 `ThemeMode` 变为「6 个活跃 + 6 个 retired」：

```rust
pub enum ThemeMode {
    Dark,
    Light,
    System,
    #[default]
    Graphite,
    Carbon,
    Ember,
    // ---- retired：仅为旧 settings.json 反序列化保留，不出现在 all() ----
    Nord,
    Dracula,
    OneDark,
    SolarizedLight,
    SolarizedDark,
    Gruvbox,
}
```

- `#[default]` 从 `Dark` 移到 `Graphite`；`Settings::default()` 里显式的
  `theme: ThemeMode::Dark` 同步改为 `Graphite`；`Dark` 变体的「深色（默认）」注释
  同步去掉「（默认）」。
- `settings.rs` 与 `theme.rs` 模块头注释里的「9 套 / Nord / Dracula…」列表同步更新。
- `all()` 只返回 6 个活跃值，顺序与 `PALETTES` 下标一一对应：
  `Dark / Light / System / Graphite / Carbon / Ember`。
- `label()`：retired 变体保留原名（不会显示，但 match 要穷尽）。
- `index()`：先取 `self.migrated().index()`，retired 值回落到迁移目标的色板。
- `is_light_palette()`：`matches!(self.migrated(), ThemeMode::Light)`。
- 新增 `migrated()`：

```rust
pub fn migrated(self) -> ThemeMode {
    match self {
        ThemeMode::SolarizedLight => ThemeMode::Light,
        ThemeMode::Nord
        | ThemeMode::Dracula
        | ThemeMode::OneDark
        | ThemeMode::SolarizedDark
        | ThemeMode::Gruvbox => ThemeMode::Graphite,
        other => other,
    }
}
```

`resolved()` 不变（`System` → `Light` / `Dark`）。

### 1.2 为什么不能直接删旧变体

`core/config.rs::load_settings()` 是：

```rust
match std::fs::read_to_string(settings_path()) {
    Ok(text) => serde_json::from_str(strip_bom(&text)).unwrap_or_default(),
    Err(_) => Settings::default(),
}
```

整份 settings.json 一次反序列化。只要旧文件里出现 `"theme": "nord"` 而枚举里没有
`Nord`，**整个 Settings 解析失败并回默认值 —— API Key、厂商配置全部丢失**。
所以 retired 变体必须保留在枚举里，只从 `all()` 隐藏。

### 1.3 迁移落盘

迁移逻辑抽成 `Settings::migrate_theme(&mut self) -> bool`（纯逻辑、可单测），
`load_settings()` 在 `upgrade_builtin_models()` 之后调用并合并落盘：

```rust
let upgraded = settings.upgrade_builtin_models();
let theme_changed = settings.migrate_theme();
if upgraded || theme_changed {
    let _ = save_settings(&settings);
}
```

- 迁移规则：`SolarizedLight → Light`（保住浅色偏好），其余 5 个 → `Graphite`。
- 迁移后立即落盘，避免用户下次不改设置就白迁移；只改 theme 字段，
  providers / Key 原样保留（由单测守护）。
- **不迁移 Dark/Light/System**：这三个仍在新列表中，老用户视觉零变化。

## 2. 色板

### 2.1 Palette 扩展

`ui/theme.rs::Palette` 增加 5 个字段：

```rust
struct Palette {
    bg, surface, panel, raised, border: u32,
    border_strong: u32,   // 弹层/选中态边框，比 border 亮一档
    fg, muted, dim: u32,
    accent: u32,
    accent_fg: u32,       // accent 填充底上的文字色
    green, yellow, red: u32,
    light: bool,
}
```

`static PALETTES: [Palette; 6]`：顺序 Dark / Light / System / Graphite / Carbon / Ember；
System 位是 Dark 的完整副本（现状如此，只为对齐下标与色卡预览）。

### 2.2 六套色值

| token | Dark | Light | System | Graphite | Carbon | Ember |
| --- | --- | --- | --- | --- | --- | --- |
| bg | `181818` | `F4F4F5` | = Dark | `0A0C10` | `13151A` | `100F0D` |
| surface | `1E1E1E` | `FFFFFF` | = Dark | `0E1117` | `171A20` | `15130F` |
| panel | `202020` | `FAFAFA` | = Dark | `0B0D12` | `111318` | `12100D` |
| raised | `252526` | `ECECEF` | = Dark | `161A22` | `1E222A` | `201C16` |
| border | `2D2D30` | `E0E0E4` | = Dark | `1C2029` | `272C35` | `2C2820` |
| border_strong | `3E3E42` | `CFCFD6` | = Dark | `2A3040` | `343A45` | `3A342A` |
| fg | `D4D4D4` | `1F2023` | = Dark | `E9EDF4` | `D9DEE6` | `F0E9DD` |
| muted | `8A8A8A` | `6B6D76` | = Dark | `9AA3B2` | `8D96A3` | `A79C8B` |
| dim | `5F5F5F` | `9AA0A6` | = Dark | `5B6373` | `5D6673` | `71675A` |
| accent | `3B82F6` | `2563EB` | = Dark | `8B93FF` | `4D9FFF` | `E8A33D` |
| accent_fg | `FFFFFF` | `FFFFFF` | = Dark | `0B0D12` | `07121F` | `1B1305` |
| green | `22C55E` | `16A34A` | = Dark | `3FB950` | `3FB950` | `6FAF6A` |
| yellow | `EAB308` | `CA8A04` | = Dark | `D29922` | `D29922` | `D9B44A` |
| red | `EF4444` | `DC2626` | = Dark | `F85149` | `F85149` | `E06C5A` |

Dark / Light 的 10 个原有字段值**一个都不改**（border_strong 为新增，取 border 亮一档），
保证经典主题视觉零回归。

### 2.3 accent_fg（对比度硬约束）

现有 24 处 `bg(theme::accent())` 填充，文字用 `theme::fg()`：

- Dark/Light 的 fg 与蓝底对比度约 4:1，维持 `accent_fg = #FFFFFF` 不变；
- Graphite 的 `#8B93FF` 亮度高，白字对比度仅 ~2.5:1，**必须**用 `#0B0D12`（~7:1）；
- Ember 的 `#E8A33D` 同理，用 `#1B1305`（~8:1）。

所有 accent 填充上的文字（按钮、选中 chip、徽标）一律改 `text_color(theme::accent_fg())`。
逐个检查 24 处：只做背景、没有文字叠加的（如开关轨道、进度条）不改。

### 2.4 语义色进色板

- `green()/yellow()/red()` 改为读 `palette()` 对应字段；
- `green_soft()` → `with_alpha(palette().green, 0.10)`；
- `green_line()` → `with_alpha(palette().green, 0.28)`；
- `purple/orange/cyan/pink/indigo` 是角色身份色 / 文件类型徽标，**保持全局 `pick()`**。

### 2.5 新增 getter

```rust
pub fn accent_fg() -> Rgba       // palette().accent_fg
pub fn border_strong() -> Rgba   // palette().border_strong
```

## 3. 形态 token

### 3.1 Metrics 表

`ui/theme.rs` 新增与 `PALETTES` 平行的 `static METRICS: [Metrics; 6]`：

```rust
struct Metrics {
    radius_sm, radius_md, radius_lg, radius_xl: f32,
    text_caption, text_ui, text_body, text_title: f32,
    mono_nums: bool,
}
```

| token | Dark/Light/System | Graphite | Carbon | Ember |
| --- | --- | --- | --- | --- |
| radius_sm（小胶囊 / 小按钮） | 4 | 6 | 3 | 6 |
| radius_md（按钮 / 输入框 / 会话卡 / 代码块） | 6 | 8 | 4 | 10 |
| radius_lg（弹层 / 审批卡 / 大卡片） | 8 | 12 | 6 | 14 |
| radius_xl（头像方块 / 弹层容器） | 12 | 16 | 8 | 18 |
| text_caption（次要文字 / 标签 / 状态） | 12 | 12 | 11 | 12 |
| text_ui（按钮 / 列表行 / 菜单项） | 14 | 13 | 13 | 13 |
| text_body（消息正文 / 输入框） | 16 | 15 | 15 | 15.5 |
| text_title（会话标题 / 页头） | 14 | 14 | 14 | 15 |
| mono_nums（Carbon 等宽签名） | false | false | true | false |

取值依据：Dark/Light 完全等于现有 `text_xs=12` / `text_sm=14` / `text_base=16`
与 `rounded_sm=4` / `md=6` / `lg=8` / `xl=12`，不产生回归；三套新皮肤按 mockup
规范（紧凑 UI + 略小正文）靠拢。

公开函数：`radius_sm/md/lg/xl()`、`text_caption/ui/body/title()`、`mono_nums()`。

### 3.2 替换映射

| 现状 | 替换为 |
| --- | --- |
| `.rounded_sm()` | `.rounded(px(theme::radius_sm()))` |
| `.rounded_md()` | `.rounded(px(theme::radius_md()))` |
| `.rounded_lg()` | `.rounded(px(theme::radius_lg()))` |
| `.rounded_xl()` | `.rounded(px(theme::radius_xl()))` |
| `.text_xs()` | `.text_size(px(theme::text_caption()))` |
| `.text_sm()` | `.text_size(px(theme::text_ui()))` |
| 消息 / 输入正文 | `.text_size(px(theme::text_body()))` |
| 会话卡标题 / 页头标题 | `.text_size(px(theme::text_title()))` |
| `.rounded_full()` | **不动**（头像、状态点、开关圆点） |

`.text_lg()`（6 处，设置页/空状态大标题）与编辑器代码字号（走 `editor_font_size` 设置）
本轮不动。

替换规模：`.rounded_sm/md/lg/xl()` 约 264 处、`.text_xs/text_sm()` 约 437 处、
accent 填充 24 处。以机械替换为主，逐文件编译验证。

### 3.3 fit_text / BODY_PX 同步

字号改变后所有**文本宽度估算**必须用同一字号，否则截断算错：

- `views::text_fit::fit_text(text, budget_px, font_size)` 的 60 处调用点（如
  `sidebar.rs` 会话卡标题的 `fit_text(&session.title, title_w, 14.0)`）：
  标题类传 `theme::text_title()`，普通行传 `theme::text_ui()`；
  **渲染侧字号必须与传参一致**（会话卡标题同时把 `.text_sm()` 改为 `.text_title()`）；
- `chat.rs` 的 `const BODY_PX: f32 = 16.0` 改为函数 `fn body_px() -> f32 { theme::text_body() }`，
  引用点（`marker_w`、`inline_row`、表格列宽测量、composer 显式字号）全部改走它；
- markdown 正文行高 `line_height(px(22.0))` → `line_height(px(body_px() + 6.0))`
  （Dark 16+6=22 与现状一致）。

### 3.4 Carbon mono 签名

`mono_nums()` 为 true 时（仅 Carbon），以下处加 `.font_family(theme::mono_family())`：

- 对话页脚统计里的 token 数（`↑x ↓y`）与耗时；
- 右栏媒体预览 / 产物列表的文件大小；
- 用量弹框的余额与 token 数字（`stat_tile` 数值、按模型汇总表）。

会话卡的时间是「刚刚 / 3分钟前」这类中文相对时间，不纳入 mono。

**限定在不需要 `fit_text` 截断或截断预算单独按 mono 估算的位置**，避免等宽字符
撑开布局。其它主题不生效（`mono_nums()` 返回 false，`.when(...)` 跳过）。

## 4. 落地范围

### 4.1 改动文件

| 文件 | 改动 |
| --- | --- |
| `crates/ui/src/theme.rs` | Palette 扩字段、PALETTES 6 套、METRICS、新 getter、语义色改造、模块注释、单测改写 |
| `crates/ui/src/components.rs` | 共享组件的 rounded/text token 化、accent 按钮文字改 `accent_fg` |
| `crates/core/src/settings.rs` | ThemeMode 变体/默认值/`migrated()`/`all()`/`label()`/`index()`/`is_light_palette()`、单测 |
| `crates/core/src/config.rs` | `load_settings()` 主题迁移落盘、迁移单测 |
| `crates/app/src/views/mod.rs` | 标题栏 token 化 |
| `crates/app/src/views/sidebar.rs` | 会话卡/工作区头像 token 化、`fit_text` 字号、Carbon mono 时间 |
| `crates/app/src/views/chat.rs` | 全部 rounded/text、`BODY_PX` 函数化、accent 填充、Carbon mono 统计 |
| `crates/app/src/views/files.rs` | 文件树 token 化、Carbon mono 文件大小 |
| `crates/app/src/views/editor.rs` | 标签栏/查找条/状态栏 token 化（代码字号不动） |
| `crates/app/src/views/settings.rs` | token 化、文案「9 套」→「6 套」 |
| `crates/app/src/views/providers.rs` | token 化（设置子页） |
| `crates/app/src/views/session_menu.rs` / `file_menu.rs` / `palette.rs` / `new_session.rs` / `model_picker.rs` / `market_detail.rs` | 弹层 token 化 |
| `crates/app/src/views/usage.rs` | token 化、Carbon mono 数字 |
| `crates/app/src/views/trace.rs` | token 化（对话页签，不涉结构） |
| `crates/app/src/views/image_view.rs` | token 化（如有 rounded/text） |
| `crates/app/src/state.rs` | 新增 `NW_THEME` 验收变量 |

### 4.2 不动的部分

- 所有图标：左侧 SVG、文件树图标、按钮 glyph、窗口按钮（div 画的方框/横线）；
- 布局常量：`RAIL_W` / `SESSION_W` / `FILES_W` / `TITLEBAR_H` / `CHAT_MAX_W` 与全部间距；
- 团队 / 技能 / 市场页的 rounded/text **不逐个替换**（共享组件跟随新 token
  导致的少量变化是预期内的）；
- 编辑器代码字号、行高、`editor_font_size`；
- Dark/Light 全部原有色值。

## 5. 设置页与验收变量

- 主题选择器自动只剩 6 个 chip（`ThemeMode::all()`），说明文案
  「9 套完整配色；跟随系统读系统深浅」→「6 套完整配色；跟随系统读系统深浅」。
- 新增 `NW_THEME=dark|light|system|graphite|carbon|ember`：`AppState::new` 里
  加载 settings 后若环境变量存在则覆盖传给 `set_theme_mode` 的值（**不写盘**），
  仅用于截图验收，与 `NW_VIEW` 同款处理。

## 6. 测试

### 6.1 `ui/theme.rs`

- 保留：`mode_switch_changes_colors`、`foreground_inverts_with_theme`、`with_alpha_clamps`；
- `each_palette_has_its_own_colors`：改为遍历 6 个活跃主题（**仍跳过 System**，
  它与 Dark 共用色板），背景互不重复；
- 删除 `solarized_light_counts_as_light`，新增 `retired_themes_migrate`：
  - `SolarizedLight.migrated() == Light` 且 `is_light_palette()` 为 true；
  - 其余 5 个 retired 的 `migrated()` 均为 `Graphite`；
  - `set_theme_mode(SolarizedLight)` 后 `is_light()` 为 true；`set_theme_mode(Nord)`
    后的 `bg()` 等于 `set_theme_mode(Graphite)` 的值（index 走迁移，不 panic）；
- 新增 `accent_fg_contrasts_with_accent`：每套色板按 (luminance+0.05) 公式计算
  accent 与 accent_fg 对比度 ≥ 3.0。阈值取 3.0 而非 4.5 是因为 Dark 的
  `#3B82F6` 配白字只有 3.66（现状如此、本轮不改），新三套实测 6.9–8.6；
- 新增 `metrics_cover_all_modes`：`METRICS.len() == ThemeMode::all().len()`。

### 6.2 `core/settings.rs`

- `settings_roundtrip` 保留；
- 新增 `retired_theme_deserializes`：`{"theme":"nord"}` 能解析为 `ThemeMode::Nord`
  （证明旧 settings.json 不会整体失败）；
- 新增 `default_theme_is_graphite`：`Settings::default().theme == ThemeMode::Graphite`；
- 新增 `theme_migration_rules`：逐条断言 `migrated()` 映射。

### 6.3 `core/config.rs`

- 新增 `load_settings_migrates_retired_theme_preserving_providers`：
  - 用 `NW_CONFIG_DIR` 指向临时目录，写入旧格式 settings.json
    （`"theme": "nord"` + 一个带 Key 的 provider）；
  - `load_settings()` 返回 theme = Graphite，providers 与 Key 字段原样；
  - 重新读文件确认已落盘 `"graphite"`。
  - `NW_CONFIG_DIR` 是进程级环境变量，测试里用一把静态互斥锁串行化，避免与
    其它碰配置目录的测试并行互踩；临时目录清理（遵循项目约定放 `pic/.tmp_*`）。

### 6.4 构建与截图验收

- `cargo test -p neverwork-ui -p neverwork-core`；
- `cargo build -p neverwork`（构建前先关掉运行中的应用，避免旧二进制）；
- 截图（DPI-aware，放 `pic/.tmp_*`，验收后删除）：
  - Graphite / Carbon / Ember 各一张主界面（`NW_THEME=... NW_VIEW=chat`）；
  - Graphite 设置页（`NW_VIEW=settings`）确认 6 个 chip 与文案；
  - Dark / Light 回归对比（应只有 border_strong 无视觉影响、无字号变化）；
- `grep` 残留检查：目标文件里不应再有 `.text_sm()` / `.text_xs()` / `.rounded_sm()`
  / `.rounded_md()` / `.rounded_lg()` / `.rounded_xl()`。

## 7. 风险与缓解

| 风险 | 缓解 |
| --- | --- |
| 删主题变体导致旧 settings.json 整份解析失败、Key 丢失 | 保留 retired 变体 + 迁移落盘 + 「providers 原样」单测 |
| 亮强调色上白字不可读 | 新增 `accent_fg`，24 处 accent 填充逐一检查，单测校验对比度 |
| 字号变化导致 `fit_text` 截断错位/右侧失控 | 60 处 `fit_text` 与 `BODY_PX` 同步改用 token，编译后残留 grep + 截图 |
| Carbon mono 撑开布局 | 只用于不截断的数值/时间处，逐屏截图验收 |
| 437 处替换遗漏或误伤 | 分文件机械替换 + 每步 `cargo build -p neverwork` + 残留 grep |
| Dark/Light 视觉回归 | 色值一个不改；Dark/Light 的 Metrics 等于现有值 |

## 8. 实现顺序（供 writing-plans 拆解）

1. **core 主题模型**：ThemeMode 变体/默认/迁移方法 + `load_settings` 迁移 + 单测；
2. **ui 色板与 token**：Palette 扩字段、6 套色板、METRICS、新 getter、单测；
3. **components + 主界面视图 token 化**：按文件逐个替换，`accent_fg` 落地；
4. **Carbon mono 签名**；
5. **设置页文案 + `NW_THEME`**；
6. **构建、测试、三套 + Dark/Light 截图验收、残留 grep**。
