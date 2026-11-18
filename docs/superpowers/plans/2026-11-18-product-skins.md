# 三套产品化皮肤（Graphite / Carbon / Ember）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 9 套通用编辑器主题替换为 6 套（深色 / 浅色 / 跟随系统 + Graphite / Carbon / Ember），并让圆角与字号随主题变化。图标、布局、交互与全部功能行为保持不变。

**Architecture:** 色板与形态 token 集中在 `NeverWork/crates/ui/src/theme.rs`（`PALETTES` / `METRICS` 两个平行数组，下标对齐 `ThemeMode::all()`）；`core` 保留 retired 主题变体用于旧 settings.json 反序列化，加载时迁移并落盘；app 各视图只做调用点替换，不重排结构。

**Tech Stack:** Rust / GPUI 0.2.1（`.rounded(px(..))`、`.text_size(px(..))`，已确认 `Pixels: Into<AbsoluteLength>`）、serde。

---

## 红线（违反任何一条即算失败）

- **只改样式**：不改 Agent、工具、会话、文件读写、SSH、审批、MCP、浏览器等任何功能代码；
- **不改图标**：`rail-*.svg`、文件树/按钮 glyph、窗口按钮（div 画的方框/横线）；
- **不改布局**：`RAIL_W` / `SESSION_W` / `FILES_W` / `TITLEBAR_H` / `CHAT_MAX_W` 与全部间距、结构、交互；
- **不改编辑器代码字号**（走 `editor_font_size` 设置）；
- **唯一涉及行为的地方**：主题枚举列表 + 旧主题值迁移加载。这是「替换主题」的必要兼容——`load_settings` 用 `unwrap_or_default()`，删掉旧枚举值会让旧 settings.json 整份解析失败，**连 API Key 一起重置**；
- 所有 cargo 命令在 `NeverWork/` 下执行；构建前若应用在跑先 `taskkill /IM neverwork.exe /F`（该命令可能报"找不到进程"，忽略即可）；
- 提交步骤默认跳过，除非用户明确要求 commit。

---

## 通用替换规则（Task 4 起反复使用）

| 现状 | 替换为 |
| --- | --- |
| `.rounded_sm()` | `.rounded(px(theme::radius_sm()))` |
| `.rounded_md()` | `.rounded(px(theme::radius_md()))` |
| `.rounded_lg()` | `.rounded(px(theme::radius_lg()))` |
| `.rounded_xl()` | `.rounded(px(theme::radius_xl()))` |
| `.text_xs()` | `.text_size(px(theme::text_caption()))` |
| `.text_sm()` | `.text_size(px(theme::text_ui()))`（正文/标题按下面各任务的显式说明） |
| `.bg(theme::accent())` 上的 `.text_color(theme::fg())` | `.text_color(theme::accent_fg())` |
| `fit_text(..., N.0)` 的数字字号 | 与渲染侧一致的 token（caption / ui / title / body） |

**不要替换**：`.rounded_full()`（头像、状态点、开关圆点）、`text_lg` / `text_base`（设置页大标题与 composer 正文单独处理）、`theme.rs` 与 `text_fit.rs` 自身。

**单个文件替换后的验证**（示例，把 `$f` 换成实际文件）：

```powershell
$f = "crates\app\src\views\sidebar.rs"
Select-String -Path $f -Pattern '\.(text_xs|text_sm)\(\)|\.rounded_(sm|md|lg|xl)\(\)' | Measure-Object
```

期望：`Count` 为 `0`（本文件内所有目标调用点都已替换）。

---

### Task 1: ThemeMode 新集与迁移（core）

**Files:**
- Modify: `NeverWork/crates/core/src/settings.rs:14-90`（枚举与方法）
- Modify: `NeverWork/crates/core/src/settings.rs`（`Settings::default()` 附近的 `theme: ThemeMode::Dark`）
- Test: `NeverWork/crates/core/src/settings.rs` 的 `mod tests`（文件末尾）

- [ ] **Step 1: 写失败测试**

在 `settings.rs` 的 `mod tests` 里追加：

```rust
#[test]
fn default_theme_is_graphite() {
    assert_eq!(Settings::default().theme, ThemeMode::Graphite);
    assert_eq!(ThemeMode::default(), ThemeMode::Graphite);
}

#[test]
fn retired_theme_deserializes() {
    // 旧 settings.json 里的主题名不能让整份配置解析失败（否则 Key 会被重置）。
    let back: Settings = serde_json::from_str(r#"{"theme":"nord"}"#).unwrap();
    assert_eq!(back.theme, ThemeMode::Nord);
    assert_eq!(back.theme.migrated(), ThemeMode::Graphite);
}

#[test]
fn theme_migration_rules() {
    assert_eq!(ThemeMode::SolarizedLight.migrated(), ThemeMode::Light);
    for old in [
        ThemeMode::Nord,
        ThemeMode::Dracula,
        ThemeMode::OneDark,
        ThemeMode::SolarizedDark,
        ThemeMode::Gruvbox,
    ] {
        assert_eq!(old.migrated(), ThemeMode::Graphite, "{} 应迁到 Graphite", old.label());
    }
    for keep in [
        ThemeMode::Dark,
        ThemeMode::Light,
        ThemeMode::System,
        ThemeMode::Graphite,
        ThemeMode::Carbon,
        ThemeMode::Ember,
    ] {
        assert_eq!(keep.migrated(), keep, "{} 不该被迁移", keep.label());
    }
}

#[test]
fn active_theme_list_has_six() {
    let all = ThemeMode::all();
    assert_eq!(all.len(), 6);
    for m in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::System, ThemeMode::Graphite, ThemeMode::Carbon, ThemeMode::Ember] {
        assert!(all.contains(&m), "{} 缺失", m.label());
    }
}

#[test]
fn retired_theme_index_falls_back_to_target() {
    // index() 用于取色板：retired 值不能越界，也不能取错色板。
    assert_eq!(ThemeMode::Nord.index(), ThemeMode::Graphite.index());
    assert_eq!(ThemeMode::SolarizedLight.index(), ThemeMode::Light.index());
    assert!(ThemeMode::SolarizedLight.is_light_palette());
    assert!(!ThemeMode::Nord.is_light_palette());
}

#[test]
fn migrate_theme_flags_changes() {
    let mut s = Settings::default();
    assert!(!s.migrate_theme(), "默认 Graphite 不需要迁移");
    s.theme = ThemeMode::Dracula;
    assert!(s.migrate_theme());
    assert_eq!(s.theme, ThemeMode::Graphite);
    s.theme = ThemeMode::Light;
    assert!(!s.migrate_theme());
}
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
cargo test -p neverwork-core default_theme_is_graphite
```

Expected: 编译失败，报 `no variant named Graphite` / `no method named migrated`。

- [ ] **Step 3: 改枚举与方法**

把 `ThemeMode` 定义（现 `settings.rs:14-36`）替换为：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    /// 深色。
    Dark,
    /// 浅色。
    Light,
    /// 跟随系统（深浅二选一，不选具体风格）。
    System,
    /// Graphite：近黑中性灰 + 靛蓝（默认）。
    #[default]
    Graphite,
    /// Carbon：碳灰 + 蓝，紧凑圆角与等宽数字。
    Carbon,
    /// Ember：暖墨灰 + 琥珀。
    Ember,
    // ---- retired：仅为旧 settings.json 反序列化保留，不出现在 all() ----
    /// Nord（已下线，加载时迁到 Graphite）。
    Nord,
    /// Dracula（已下线，加载时迁到 Graphite）。
    Dracula,
    /// One Dark（已下线，加载时迁到 Graphite）。
    OneDark,
    /// Solarized 浅（已下线，加载时迁到 Light）。
    SolarizedLight,
    /// Solarized 深（已下线，加载时迁到 Graphite）。
    SolarizedDark,
    /// Gruvbox（已下线，加载时迁到 Graphite）。
    Gruvbox,
}
```

把 `impl ThemeMode`（现 `:38-90`）替换为：

```rust
impl ThemeMode {
    pub fn label(&self) -> &'static str {
        match self {
            ThemeMode::Dark => "深色",
            ThemeMode::Light => "浅色",
            ThemeMode::System => "跟随系统",
            ThemeMode::Graphite => "Graphite",
            ThemeMode::Carbon => "Carbon",
            ThemeMode::Ember => "Ember",
            // retired：不会显示在 UI，只为 match 穷尽。
            ThemeMode::Nord => "Nord",
            ThemeMode::Dracula => "Dracula",
            ThemeMode::OneDark => "One Dark",
            ThemeMode::SolarizedLight => "Solarized 浅",
            ThemeMode::SolarizedDark => "Solarized 深",
            ThemeMode::Gruvbox => "Gruvbox",
        }
    }

    /// 旧主题的迁移目标：浅色偏好保留，其余统一到 Graphite。
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

    /// 在 `all()` 里的下标（也是 UI 色板表的下标）。retired 值走迁移目标。
    pub fn index(&self) -> usize {
        Self::all().iter().position(|m| *m == self.migrated()).unwrap_or(0)
    }

    /// 该主题是不是浅色底（决定语义色取亮/暗档）。
    pub fn is_light_palette(&self) -> bool {
        matches!(self.migrated(), ThemeMode::Light)
    }

    /// `System` 按系统深浅解析成具体主题；其它主题原样返回。
    pub fn resolved(self, system_light: bool) -> ThemeMode {
        match self {
            ThemeMode::System => {
                if system_light {
                    ThemeMode::Light
                } else {
                    ThemeMode::Dark
                }
            }
            other => other,
        }
    }

    pub fn all() -> &'static [ThemeMode] {
        &[
            ThemeMode::Dark,
            ThemeMode::Light,
            ThemeMode::System,
            ThemeMode::Graphite,
            ThemeMode::Carbon,
            ThemeMode::Ember,
        ]
    }
}
```

在 `impl Settings` 里（`upgrade_builtin_models` 旁边）加：

```rust
/// 旧主题迁移：返回是否有改动。加载后调用并落盘。
pub fn migrate_theme(&mut self) -> bool {
    let target = self.theme.migrated();
    if target != self.theme {
        self.theme = target;
        true
    } else {
        false
    }
}
```

把 `Settings::default()` 里的 `theme: ThemeMode::Dark,` 改为 `theme: ThemeMode::Graphite,`。

- [ ] **Step 4: 跑测试确认通过**

```powershell
cargo test -p neverwork-core theme
```

Expected: `default_theme_is_graphite`、`retired_theme_deserializes`、`theme_migration_rules`、`active_theme_list_has_six`、`retired_theme_index_falls_back_to_target`、`migrate_theme_flags_changes` 全部 PASS。

- [ ] **Step 5: 全量 core 测试**

```powershell
cargo test -p neverwork-core
```

Expected: 全绿（`settings_roundtrip` 等旧测试不受影响）。

- [ ] **Step 6: Commit（用户未明确要求提交时跳过）**

```powershell
git add NeverWork/crates/core/src/settings.rs
git commit -m "feat(core): 主题集换成 6 套并保留旧值迁移"
```

---

### Task 2: load_settings 迁移落盘（core）

**Files:**
- Modify: `NeverWork/crates/core/src/config.rs:85-98`
- Test: `NeverWork/crates/core/tests/config_test.rs`（文件末尾）

- [ ] **Step 1: 写失败测试**

在 `config_test.rs` 末尾追加：

```rust
#[test]
fn load_settings_migrates_retired_theme_preserving_keys() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = std::env::temp_dir().join(format!("neverwork-test-theme-{}", std::process::id()));
    std::env::set_var("NW_CONFIG_DIR", &tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    // 旧主题 + 带 Key 的厂商：迁移只能动 theme，Key 必须原样。
    let json = r#"{"theme":"nord","providers":[{"id":"glm","preset":"glm","name":"智谱 GLM","base_url":"https://open.bigmodel.cn/api/paas/v4","api_key":"sk-keep-me","models":["glm-4-plus","glm-4","glm-4-flash","glm-4v"],"model":"glm-4","enabled":true}]}"#;
    std::fs::write(tmp.join("settings.json"), json).unwrap();

    let loaded = config::load_settings();
    assert_eq!(
        loaded.theme,
        neverwork_core::ThemeMode::Graphite,
        "旧主题要迁到 Graphite"
    );
    let glm = loaded.providers.iter().find(|p| p.id == "glm").unwrap();
    assert_eq!(glm.api_key, "sk-keep-me", "迁移不能丢 Key");

    // 迁移必须落盘，且不能再写回旧主题名。
    let text = std::fs::read_to_string(tmp.join("settings.json")).unwrap();
    assert!(text.contains("\"graphite\""), "迁移结果要落盘：{text}");
    assert!(!text.contains("\"nord\""), "落盘后不该再有旧主题名");

    let _ = std::fs::remove_dir_all(&tmp);
}
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
cargo test -p neverwork-core --test config_test load_settings_migrates_retired_theme_preserving_keys
```

Expected: FAIL，`loaded.theme` 仍是 `Nord`（`migrate_theme` 还没接进加载流程）。

- [ ] **Step 3: 接入 load_settings**

把 `config.rs` 的 `load_settings` 替换为：

```rust
/// 加载设置；文件不存在或损坏时返回默认值。
///
/// 内置模型列表换代后（旧模型下线），仍沿用旧内置预设的厂商会自动升级；
/// 已下线的旧主题（Nord/Dracula/…）迁到新主题集。两件事都立即落盘，
/// 避免用户本次不改设置就白迁移。
pub fn load_settings() -> Settings {
    let mut settings = match std::fs::read_to_string(settings_path()) {
        Ok(text) => serde_json::from_str(strip_bom(&text)).unwrap_or_default(),
        Err(_) => Settings::default(),
    };
    let upgraded = settings.upgrade_builtin_models();
    let theme_changed = settings.migrate_theme();
    if upgraded || theme_changed {
        let _ = save_settings(&settings);
    }
    settings
}
```

- [ ] **Step 4: 跑测试确认通过**

```powershell
cargo test -p neverwork-core --test config_test
```

Expected: 全部 PASS（含 `config_roundtrip`、`load_settings_upgrades_legacy_builtin_models`）。

- [ ] **Step 5: Commit（默认跳过）**

```powershell
git add NeverWork/crates/core/src/config.rs NeverWork/crates/core/tests/config_test.rs
git commit -m "feat(core): 加载设置时迁移旧主题并落盘"
```

---

### Task 3: 色板与形态 token（ui/theme.rs）

**Files:**
- Modify: `NeverWork/crates/ui/src/theme.rs`（整份文件：模块头、Palette、PALETTES、METRICS、getter、语义色、单测）

- [ ] **Step 1: 改写模块头与 Palette**

模块头注释替换为：

```rust
//! NeverWork 设计令牌 —— 对齐 `docs/superpowers/specs/2026-11-18-product-skins-design.md`。
//!
//! 6 套**完整主题色板**（深色 / 浅色 / 跟随系统 / Graphite / Carbon / Ember）
//! 通过全局模式切换：`set_theme_mode(...)` 之后，所有 `theme::xxx()` 调用立刻
//! 返回新色板的值，视图无需改一行代码。形态 token（圆角 / 字号 / mono 数字）
//! 与色板平行，同一套下标。
```

`Palette` 替换为：

```rust
/// 一套完整色板：背景层级 + 文字 + 边框 + 强调色。
struct Palette {
    bg: u32,
    surface: u32,
    panel: u32,
    raised: u32,
    border: u32,
    /// 弹层 / 选中态的强边框，比 border 亮一档。
    border_strong: u32,
    fg: u32,
    muted: u32,
    dim: u32,
    accent: u32,
    /// accent 填充底上的文字色（亮强调色用深字，保证对比度）。
    accent_fg: u32,
    green: u32,
    yellow: u32,
    red: u32,
    /// 浅色底：语义色（绿/黄/红…）取暗档。
    light: bool,
}
```

- [ ] **Step 2: 替换 PALETTES**

```rust
/// 下标与 `ThemeMode::all()` 一一对应。
static PALETTES: [Palette; 6] = [
    // Dark（经典深色，色值维持原样）
    Palette {
        bg: 0x181818,
        surface: 0x1e1e1e,
        panel: 0x202020,
        raised: 0x252526,
        border: 0x2d2d30,
        border_strong: 0x3e3e42,
        fg: 0xd4d4d4,
        muted: 0x8a8a8a,
        dim: 0x5f5f5f,
        accent: 0x3b82f6,
        accent_fg: 0xffffff,
        green: 0x22c55e,
        yellow: 0xeab308,
        red: 0xef4444,
        light: false,
    },
    // Light（色值维持原样）
    Palette {
        bg: 0xf4f4f5,
        surface: 0xffffff,
        panel: 0xfafafa,
        raised: 0xececef,
        border: 0xe0e0e4,
        border_strong: 0xcfcfd6,
        fg: 0x1f2023,
        muted: 0x6b6d76,
        dim: 0x9aa0a6,
        accent: 0x2563eb,
        accent_fg: 0xffffff,
        green: 0x16a34a,
        yellow: 0xca8a04,
        red: 0xdc2626,
        light: true,
    },
    // System（解析后不会落在这里，只为对齐下标 / 色卡预览）
    Palette {
        bg: 0x181818,
        surface: 0x1e1e1e,
        panel: 0x202020,
        raised: 0x252526,
        border: 0x2d2d30,
        border_strong: 0x3e3e42,
        fg: 0xd4d4d4,
        muted: 0x8a8a8a,
        dim: 0x5f5f5f,
        accent: 0x3b82f6,
        accent_fg: 0xffffff,
        green: 0x22c55e,
        yellow: 0xeab308,
        red: 0xef4444,
        light: false,
    },
    // Graphite：近黑中性灰 + 靛蓝
    Palette {
        bg: 0x0a0c10,
        surface: 0x0e1117,
        panel: 0x0b0d12,
        raised: 0x161a22,
        border: 0x1c2029,
        border_strong: 0x2a3040,
        fg: 0xe9edf4,
        muted: 0x9aa3b2,
        dim: 0x5b6373,
        accent: 0x8b93ff,
        accent_fg: 0x0b0d12,
        green: 0x3fb950,
        yellow: 0xd29922,
        red: 0xf85149,
        light: false,
    },
    // Carbon：碳灰 + 蓝
    Palette {
        bg: 0x13151a,
        surface: 0x171a20,
        panel: 0x111318,
        raised: 0x1e222a,
        border: 0x272c35,
        border_strong: 0x343a45,
        fg: 0xd9dee6,
        muted: 0x8d96a3,
        dim: 0x5d6673,
        accent: 0x4d9fff,
        accent_fg: 0x07121f,
        green: 0x3fb950,
        yellow: 0xd29922,
        red: 0xf85149,
        light: false,
    },
    // Ember：暖墨灰 + 琥珀
    Palette {
        bg: 0x100f0d,
        surface: 0x15130f,
        panel: 0x12100d,
        raised: 0x201c16,
        border: 0x2c2820,
        border_strong: 0x3a342a,
        fg: 0xf0e9dd,
        muted: 0xa79c8b,
        dim: 0x71675a,
        accent: 0xe8a33d,
        accent_fg: 0x1b1305,
        green: 0x6faf6a,
        yellow: 0xd9b44a,
        red: 0xe06c5a,
        light: false,
    },
];
```

- [ ] **Step 3: 加 METRICS 与 getter**

在 `PALETTES` 之后加：

```rust
/// 形态 token：圆角与字号随主题变化（下标与 PALETTES 一致）。
struct Metrics {
    radius_sm: f32,
    radius_md: f32,
    radius_lg: f32,
    radius_xl: f32,
    text_caption: f32,
    text_ui: f32,
    text_body: f32,
    text_title: f32,
    /// 数值/统计用等宽字体（Carbon 签名）。
    mono_nums: bool,
}

static METRICS: [Metrics; 6] = [
    // Dark：等于历史值（rounded_sm/md/lg/xl = 4/6/8/12，text_xs/sm/base = 12/14/16）
    Metrics { radius_sm: 4.0, radius_md: 6.0, radius_lg: 8.0, radius_xl: 12.0, text_caption: 12.0, text_ui: 14.0, text_body: 16.0, text_title: 14.0, mono_nums: false },
    // Light
    Metrics { radius_sm: 4.0, radius_md: 6.0, radius_lg: 8.0, radius_xl: 12.0, text_caption: 12.0, text_ui: 14.0, text_body: 16.0, text_title: 14.0, mono_nums: false },
    // System（与 Dark 一致）
    Metrics { radius_sm: 4.0, radius_md: 6.0, radius_lg: 8.0, radius_xl: 12.0, text_caption: 12.0, text_ui: 14.0, text_body: 16.0, text_title: 14.0, mono_nums: false },
    // Graphite
    Metrics { radius_sm: 6.0, radius_md: 8.0, radius_lg: 12.0, radius_xl: 16.0, text_caption: 12.0, text_ui: 13.0, text_body: 15.0, text_title: 14.0, mono_nums: false },
    // Carbon
    Metrics { radius_sm: 3.0, radius_md: 4.0, radius_lg: 6.0, radius_xl: 8.0, text_caption: 11.0, text_ui: 13.0, text_body: 15.0, text_title: 14.0, mono_nums: true },
    // Ember
    Metrics { radius_sm: 6.0, radius_md: 10.0, radius_lg: 14.0, radius_xl: 18.0, text_caption: 12.0, text_ui: 13.0, text_body: 15.5, text_title: 15.0, mono_nums: false },
];

fn metrics() -> &'static Metrics {
    &METRICS[current_mode().index().min(METRICS.len() - 1)]
}
```

在 `// ============ 尺寸 ============` 之前加：

```rust
// ============ 形态 token ============
pub fn radius_sm() -> f32 {
    metrics().radius_sm
}
pub fn radius_md() -> f32 {
    metrics().radius_md
}
pub fn radius_lg() -> f32 {
    metrics().radius_lg
}
pub fn radius_xl() -> f32 {
    metrics().radius_xl
}
pub fn text_caption() -> f32 {
    metrics().text_caption
}
pub fn text_ui() -> f32 {
    metrics().text_ui
}
pub fn text_body() -> f32 {
    metrics().text_body
}
pub fn text_title() -> f32 {
    metrics().text_title
}
/// 数值/统计是否用等宽字体（Carbon 为 true）。
pub fn mono_nums() -> bool {
    metrics().mono_nums
}
```

- [ ] **Step 4: 加 accent_fg / border_strong，语义色改读色板**

`// ============ 边框 ============` 段追加：

```rust
/// 弹层 / 选中态的强边框。
pub fn border_strong() -> Rgba {
    gpui::rgb(palette().border_strong)
}
```

`accent_line()` 之后追加：

```rust
/// accent 填充底上的文字色。
pub fn accent_fg() -> Rgba {
    gpui::rgb(palette().accent_fg)
}
```

把 `green/green_soft/green_line/yellow/red` 替换为：

```rust
pub fn green() -> Rgba {
    gpui::rgb(palette().green)
}
pub fn green_soft() -> Rgba {
    with_alpha(palette().green, 0.10)
}
pub fn green_line() -> Rgba {
    with_alpha(palette().green, 0.28)
}
pub fn yellow() -> Rgba {
    gpui::rgb(palette().yellow)
}
pub fn red() -> Rgba {
    gpui::rgb(palette().red)
}
```

（`purple/orange/cyan/pink/indigo` 保持全局 `pick()` 不动。）

- [ ] **Step 5: 改写单测**

删除 `solarized_light_counts_as_light`，把 `each_palette_has_its_own_colors` 替换为下面三组，并追加对比度函数：

```rust
fn luminance(hex: u32) -> f32 {
    let ch = |v: u32| {
        let c = v as f32 / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch((hex >> 16) & 0xff) + 0.7152 * ch((hex >> 8) & 0xff) + 0.0722 * ch(hex & 0xff)
}

#[test]
fn each_palette_has_its_own_colors() {
    let _guard = theme_test_guard();
    let mut seen = Vec::new();
    for mode in ThemeMode::all().iter().copied() {
        set_theme_mode(mode);
        let bg = bg();
        if mode != ThemeMode::System {
            assert!(
                !seen.iter().any(|(r, g, b)| *r == bg.r && *g == bg.g && *b == bg.b),
                "{} 的背景色与其它主题重复",
                mode.label()
            );
            seen.push((bg.r, bg.g, bg.b));
        }
        assert_ne!(fg(), bg, "{} 的文字与背景同色", mode.label());
    }
    set_theme_mode(ThemeMode::Graphite);
}

#[test]
fn retired_themes_migrate() {
    let _guard = theme_test_guard();
    set_theme_mode(ThemeMode::SolarizedLight);
    assert!(is_light(), "Solarized 浅要迁到 Light");
    set_theme_mode(ThemeMode::Nord);
    let nord_bg = bg();
    set_theme_mode(ThemeMode::Graphite);
    assert_eq!(nord_bg, bg(), "其余旧主题统一迁到 Graphite");
    set_theme_mode(ThemeMode::Graphite);
}

#[test]
fn accent_fg_contrasts_with_accent() {
    // Dark 的 #3B82F6 配白字只有 3.66（现状如此、本轮不改），阈值取 3.0；
    // 新三套实测 6.9–8.6。
    for (i, p) in PALETTES.iter().enumerate() {
        if i == 2 {
            continue; // System 与 Dark 同色板
        }
        let (l1, l2) = (luminance(p.accent), luminance(p.accent_fg));
        let ratio = (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05);
        assert!(ratio >= 3.0, "第 {i} 套 accent/accent_fg 对比度只有 {ratio:.2}");
    }
}

#[test]
fn metrics_cover_all_modes() {
    assert_eq!(METRICS.len(), ThemeMode::all().len());
    let _guard = theme_test_guard();
    set_theme_mode(ThemeMode::Dark);
    assert_eq!((radius_md(), text_ui(), text_body()), (6.0, 14.0, 16.0), "Dark 必须维持现状");
    set_theme_mode(ThemeMode::Carbon);
    assert!(mono_nums());
    assert_eq!(radius_md(), 4.0);
    set_theme_mode(ThemeMode::Graphite);
    assert!(!mono_nums());
}
```

同时把其余旧测试里 `set_theme_mode(ThemeMode::Dark)` 的收尾改为 `ThemeMode::Graphite`（可选，保持默认主题一致）。

- [ ] **Step 6: 跑 ui 测试**

```powershell
cargo test -p neverwork-ui
```

Expected: 全绿（`mode_switch_changes_colors`、`foreground_inverts_with_theme`、`with_alpha_clamps`、新增 4 个）。

- [ ] **Step 7: Commit（默认跳过）**

```powershell
git add NeverWork/crates/ui/src/theme.rs
git commit -m "feat(ui): 6 套主题色板 + 圆角字号形态 token"
```

---

### Task 4: 共享组件 token 化（ui/components.rs）

**Files:**
- Modify: `NeverWork/crates/ui/src/components.rs`（全文件）

- [ ] **Step 1: 按通用规则替换**

本文件内：

- `.rounded_sm()` → `.rounded(px(theme::radius_sm()))`（tag、file_row 行与 owner 徽标）
- `.rounded_md()` → `.rounded(px(theme::radius_md()))`（avatar、status_dot 的 7px 点改 `rounded_full` 保持圆点：`div().size(px(7.0)).rounded_full()`；segment、session_item、primary_button、ghost_button）
- `.rounded_lg()` → `.rounded(px(theme::radius_lg()))`（card）
- `.text_xs()` → `.text_size(px(theme::text_caption()))`
- `.text_sm()` → `.text_size(px(theme::text_ui()))`（`body_text` 也用它）
- `file_row` 的 owner 徽标：`.bg(theme::accent()).text_color(theme::fg())` → `.text_color(theme::accent_fg())`
- `primary_button`：同上改 `accent_fg`

改完 `status_dot` 应为：

```rust
pub fn status_dot(label: &str) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(div().size(px(7.0)).rounded_full().bg(theme::green()))
        .child(
            div()
                .text_size(px(theme::text_caption()))
                .text_color(theme::muted())
                .child(SharedString::from(label.to_string())),
        )
}
```

- [ ] **Step 2: 编译 + 残留检查**

```powershell
cargo test -p neverwork-ui
Select-String -Path crates\ui\src\components.rs -Pattern '\.(text_xs|text_sm)\(\)|\.rounded_(sm|md|lg|xl)\(\)' | Measure-Object
```

Expected: 测试 PASS；`Count` = 0。

- [ ] **Step 3: Commit（默认跳过）**

```powershell
git add NeverWork/crates/ui/src/components.rs
git commit -m "style(ui): 共享组件改用形态 token"
```

---

### Task 5: 标题栏 + 会话栏 + 右栏（mod.rs / sidebar.rs / files.rs）

**Files:**
- Modify: `NeverWork/crates/app/src/views/mod.rs`（标题栏，只涉 `:72`、`:85-96` 一带）
- Modify: `NeverWork/crates/app/src/views/sidebar.rs`（全文件）
- Modify: `NeverWork/crates/app/src/views/files.rs`（全文件）

- [ ] **Step 1: mod.rs 按通用规则替换**（`.text_sm()`/`.text_xs()`/`.rounded_md()` 等）

- [ ] **Step 2: sidebar.rs 特殊点 + 通用替换**

会话卡标题（现 `:657` 与 `:789-797`）：

```rust
let title_text = crate::views::text_fit::fit_text(&session.title, title_w, theme::text_title());
```

```rust
head = head.child(
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .text_size(px(theme::text_title()))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme::fg())
        .child(SharedString::from(title_text)),
);
```

摘要行（现 `:837`）：`fit_text(&preview_raw, preview_w, theme::text_caption())`，渲染侧 `.text_xs()` 按通用规则改 caption。

其余按通用规则替换；`.rounded_full()`（20px 头像、状态点、4px 圆点）不动；`text_size(px(18.0))` 等手写图标字号不动。

- [ ] **Step 3: files.rs 通用替换**

`text_size(px(9.0))` / `text_size(px(10.0))` 这类手写小字号**保持原值**（文件类型徽标、时间戳等），只替换 `.text_xs()`/`.text_sm()`/`.rounded_*()`。`fit_text` 若有调用，其字号参数改为对应 token。

- [ ] **Step 4: 编译 + 残留检查**

```powershell
cargo build -p neverwork
$fs = @("crates\app\src\views\mod.rs","crates\app\src\views\sidebar.rs","crates\app\src\views\files.rs")
Select-String -Path $fs -Pattern '\.(text_xs|text_sm)\(\)|\.rounded_(sm|md|lg|xl)\(\)' | Measure-Object
```

Expected: 编译通过；`Count` = 0。

- [ ] **Step 5: Commit（默认跳过）**

```powershell
git add NeverWork/crates/app/src/views/mod.rs NeverWork/crates/app/src/views/sidebar.rs NeverWork/crates/app/src/views/files.rs
git commit -m "style(app): 标题栏/会话栏/文件树改用形态 token"
```

---

### Task 6: 对话流（chat.rs）

**Files:**
- Modify: `NeverWork/crates/app/src/views/chat.rs`（全文件，约 3394 行）

- [ ] **Step 1: BODY_PX 常量改为函数**

把文件头（现 `:7-9`）：

```rust
/// 对话正文基准字号。折行估宽与渲染必须用同一个值：
/// 估窄了行会顶出气泡右侧被父容器裁掉半截。
const BODY_PX: f32 = 16.0;
```

改为：

```rust
/// 对话正文基准字号（随主题）。折行估宽与渲染必须用同一个值：
/// 估窄了行会顶出气泡右侧被父容器裁掉半截。
fn body_px() -> f32 {
    theme::text_body()
}
```

把全文所有 `BODY_PX` 引用改为 `body_px()`（`marker_w`、`code_fence_block` 的显式字号、`markdown_block` 各 `inline_row(..., BODY_PX)`、表格列宽测量、`char_width_mono(ch, BODY_PX)`）。

- [ ] **Step 2: 正文行高与输入框**

- `markdown_block` 里 4 处 `.line_height(px(22.0))` → `.line_height(px(body_px() + 6.0))`（Dark 16+6=22 与现状一致）；
- composer 输入行（现 `:711-712`）：`.text_base()` → `.text_size(px(body_px()))`；`.line_height(px(24.0))` → `.line_height(px(body_px() + 8.0))`；
- `user_bubble` 正文（现 `:2451-2456`）：加 `.text_size(px(body_px()))`，`.line_height(px(22.0))` → `.line_height(px(body_px() + 6.0))`；
- `agent_bubble` 外层 col（现 `:2495-2500`）：加 `.text_size(px(body_px()))`；
- 附件名 `fit_text(&a.name, 180.0, 11.0)`（现 `:2439`）→ `fit_text(&a.name, 180.0, theme::text_caption())`。

- [ ] **Step 3: 全部 rounded / text 按通用规则替换**

`.text_sm()` 出现处若是标题/正文（如 `agent_bubble` 的角色名、ask 问题），用 `text_ui()`；diff/审批卡正文用 `text_body()`。`text_size(px(...))` 手写字号（如 markdown 标题 18/16/14.5、`w(px(20.0))` 的图标字号）不动。

- [ ] **Step 4: accent 填充文字改 accent_fg**

全文搜 `.bg(theme::accent())`，对每处确认同元素（或紧跟的 `.text_color`）是否叠加文字；有文字的改 `.text_color(theme::accent_fg())`。典型位置：审批「允许并写入」按钮、选中的模式/思考 chip、发送按钮。只做背景没有文字的（进度条等）不改。

- [ ] **Step 5: fit_text 字号参数对齐**

```powershell
Select-String -Path crates\app\src\views\chat.rs -Pattern 'fit_text\(' -Context 0,1
```

逐个把数字字号换成与渲染侧一致 token（标题→`text_title()`、正文→`text_body()`、次要→`text_caption()`、普通行→`text_ui()`）。

- [ ] **Step 6: 编译 + 残留检查**

```powershell
cargo build -p neverwork
Select-String -Path crates\app\src\views\chat.rs -Pattern '\.(text_xs|text_sm)\(\)|\.rounded_(sm|md|lg|xl)\(\)|BODY_PX' | Measure-Object
```

Expected: 编译通过；`Count` = 0。

- [ ] **Step 7: Commit（默认跳过）**

```powershell
git add NeverWork/crates/app/src/views/chat.rs
git commit -m "style(chat): 对话流改用形态 token，正文字号随主题"
```

---

### Task 7: 编辑器外壳 / 设置 / 厂商（editor.rs / settings.rs / providers.rs）

**Files:**
- Modify: `NeverWork/crates/app/src/views/editor.rs`
- Modify: `NeverWork/crates/app/src/views/settings.rs`
- Modify: `NeverWork/crates/app/src/views/providers.rs`

- [ ] **Step 1: editor.rs 通用替换**

只替换标签栏、查找条、状态栏等外壳的 `.text_xs()`/`.text_sm()`/`.rounded_*()`；**代码行渲染与行号字号保持 `settings.editor_font_size` 逻辑不动**（`text_size(px(self.editor_font_size))` 之类不要碰）。

- [ ] **Step 2: settings.rs 通用替换 + 文案**

主题说明（现 `:174`）：

```rust
"9 套完整配色；跟随系统读系统深浅",
```

改为：

```rust
"6 套完整配色；跟随系统读系统深浅",
```

其余按通用规则替换（chip 的 `.rounded_sm()` 色卡块按规则替换、`.text_xs()` → caption）。

- [ ] **Step 3: providers.rs 通用替换**

按通用规则整体替换；Key 输入框、掩码逻辑、任何交互代码不动。

- [ ] **Step 4: 编译 + 残留检查**

```powershell
cargo build -p neverwork
$fs = @("crates\app\src\views\editor.rs","crates\app\src\views\settings.rs","crates\app\src\views\providers.rs")
Select-String -Path $fs -Pattern '\.(text_xs|text_sm)\(\)|\.rounded_(sm|md|lg|xl)\(\)' | Measure-Object
```

Expected: 编译通过；`Count` = 0（editor.rs 里若代码行用 `.text_sm()` 属遗漏，必须换掉，但字号值仍走 `editor_font_size` 的显式 `text_size` 不动）。

- [ ] **Step 5: Commit（默认跳过）**

```powershell
git add NeverWork/crates/app/src/views/editor.rs NeverWork/crates/app/src/views/settings.rs NeverWork/crates/app/src/views/providers.rs
git commit -m "style(app): 编辑器外壳/设置/厂商页改用形态 token"
```

---

### Task 8: 弹层（session_menu / file_menu / palette / new_session / model_picker / market_detail / image_view）

**Files:**
- Modify: `NeverWork/crates/app/src/views/session_menu.rs`
- Modify: `NeverWork/crates/app/src/views/file_menu.rs`
- Modify: `NeverWork/crates/app/src/views/palette.rs`
- Modify: `NeverWork/crates/app/src/views/new_session.rs`
- Modify: `NeverWork/crates/app/src/views/model_picker.rs`
- Modify: `NeverWork/crates/app/src/views/market_detail.rs`
- Modify: `NeverWork/crates/app/src/views/image_view.rs`（有 rounded/text 才改）

- [ ] **Step 1: 按通用规则逐个文件替换**

注意：

- 弹层容器的 `.rounded_xl()` → `.rounded(px(theme::radius_xl()))`（session_menu / file_menu 卡片）；
- 弹层边缘若用 `.border_color(theme::border())`，容器级改 `theme::border_strong()`（raised 底上 border 太弱）；行分隔线保持 `border()`；
- `.occlude()`、坐标计算、点击回调、`MENU_W` 等一律不动。

- [ ] **Step 2: 编译 + 残留检查**

```powershell
cargo build -p neverwork
$fs = @("crates\app\src\views\session_menu.rs","crates\app\src\views\file_menu.rs","crates\app\src\views\palette.rs","crates\app\src\views\new_session.rs","crates\app\src\views\model_picker.rs","crates\app\src\views\market_detail.rs","crates\app\src\views\image_view.rs")
Select-String -Path $fs -Pattern '\.(text_xs|text_sm)\(\)|\.rounded_(sm|md|lg|xl)\(\)' | Measure-Object
```

Expected: 编译通过；`Count` = 0。

- [ ] **Step 3: Commit（默认跳过）**

```powershell
git add NeverWork/crates/app/src/views/session_menu.rs NeverWork/crates/app/src/views/file_menu.rs NeverWork/crates/app/src/views/palette.rs NeverWork/crates/app/src/views/new_session.rs NeverWork/crates/app/src/views/model_picker.rs NeverWork/crates/app/src/views/market_detail.rs NeverWork/crates/app/src/views/image_view.rs
git commit -m "style(app): 弹层改用形态 token"
```

---

### Task 9: 轨迹与用量（trace.rs / usage.rs）

**Files:**
- Modify: `NeverWork/crates/app/src/views/trace.rs`
- Modify: `NeverWork/crates/app/src/views/usage.rs`

- [ ] **Step 1: 按通用规则替换**

轨迹条目标签、详情分栏、表格数值等 `.text_xs()`/`.text_sm()`/`.rounded_*()` 全部换 token。`text_size(px(10.0))` 等手写小字号保持原值。

- [ ] **Step 2: 编译 + 残留检查**

```powershell
cargo build -p neverwork
$fs = @("crates\app\src\views\trace.rs","crates\app\src\views\usage.rs")
Select-String -Path $fs -Pattern '\.(text_xs|text_sm)\(\)|\.rounded_(sm|md|lg|xl)\(\)' | Measure-Object
```

Expected: 编译通过；`Count` = 0。

- [ ] **Step 3: Commit（默认跳过）**

```powershell
git add NeverWork/crates/app/src/views/trace.rs NeverWork/crates/app/src/views/usage.rs
git commit -m "style(app): 轨迹与用量页改用形态 token"
```

---

### Task 10: Carbon 等宽数字签名

**Files:**
- Modify: `NeverWork/crates/app/src/views/chat.rs`（页脚统计）
- Modify: `NeverWork/crates/app/src/views/usage.rs`（`stat_tile` 数值、余额金额）
- Modify: `NeverWork/crates/app/src/views/files.rs`（媒体预览 / 产物列表的文件大小）

- [ ] **Step 1: chat.rs 页脚统计**

`agent_footer` 的统计文本（现 `:2576-2581`）：

```rust
.child(
    div()
        .text_xs()
        .text_color(theme::muted())
        .child(SharedString::from(parts.join(" · "))),
)
```

改为：

```rust
.child(
    div()
        .text_size(px(theme::text_caption()))
        .text_color(theme::muted())
        .when(theme::mono_nums(), |d| d.font_family(theme::mono_family()))
        .child(SharedString::from(parts.join(" · "))),
)
```

团队页脚同理（`team_bubble` 里相同的统计行）。

- [ ] **Step 2: usage.rs 数字**

`stat_tile` 的数值（现 `:445` 起的 div）加：

```rust
.when(theme::mono_nums(), |d| d.font_family(theme::mono_family()))
```

余额金额行（`usage_balance_card` 里显示余额的 div）同样处理。

- [ ] **Step 3: files.rs 文件大小**

媒体预览的 `"{name} · {size} · 图片"` 行、PDF 的 `"{size} · 内置查看暂不支持 PDF 排版渲染"` 行、产物列表的 `sub`（现 `:317-321`）都加：

```rust
.when(theme::mono_nums(), |d| d.font_family(theme::mono_family()))
```

- [ ] **Step 4: 编译 + 测试**

```powershell
cargo build -p neverwork
cargo test -p neverwork-ui -p neverwork-core
```

Expected: 编译通过、测试全绿。

- [ ] **Step 5: Commit（默认跳过）**

```powershell
git add NeverWork/crates/app/src/views/chat.rs NeverWork/crates/app/src/views/usage.rs NeverWork/crates/app/src/views/files.rs
git commit -m "style(app): Carbon 统计数字使用等宽字体"
```

---

### Task 11: 全量验收

**Files:** 无代码改动（只验证 + 截图 + 清理）

- [ ] **Step 1: 关掉运行中的应用并全量测试**

```powershell
taskkill /IM neverwork.exe /F 2>$null
cargo test -p neverwork-ui -p neverwork-core
cargo test -p neverwork-core --test config_test
```

Expected: 全绿。

- [ ] **Step 2: 构建**

```powershell
cargo build -p neverwork
```

Expected: 编译成功（exit 0）。

- [ ] **Step 3: 残留 grep（目标文件清单）**

```powershell
$fs = @(
  "crates\ui\src\components.rs",
  "crates\app\src\views\mod.rs","crates\app\src\views\sidebar.rs","crates\app\src\views\chat.rs",
  "crates\app\src\views\files.rs","crates\app\src\views\editor.rs","crates\app\src\views\settings.rs",
  "crates\app\src\views\providers.rs","crates\app\src\views\session_menu.rs","crates\app\src\views\file_menu.rs",
  "crates\app\src\views\palette.rs","crates\app\src\views\new_session.rs","crates\app\src\views\model_picker.rs",
  "crates\app\src\views\market_detail.rs","crates\app\src\views\image_view.rs","crates\app\src\views\trace.rs",
  "crates\app\src\views\usage.rs"
)
Select-String -Path $fs -Pattern '\.(text_xs|text_sm)\(\)|\.rounded_(sm|md|lg|xl)\(\)' | Measure-Object
```

Expected: `Count` = 0。

- [ ] **Step 4: 三套主题截图（不改代码，用临时配置目录指定主题）**

按项目约定，开发验证数据放 `d:\opensource\neverwork\pic\.tmp_*`，验证完删除：

```powershell
$dir = "d:\opensource\neverwork\pic\.tmp_skin_graphite"
New-Item -ItemType Directory -Force $dir | Out-Null
'{"theme":"graphite"}' | Set-Content -Encoding UTF8 "$dir\settings.json"
$env:NW_CONFIG_DIR = $dir
cargo run -p neverwork
```

启动后按 DPI-aware 方式截图（`SetProcessDpiAwarenessContext(-4)` 后再抓，否则只抓到左上 1/4），保存到 `pic\.tmp_skin_graphite.png`。对 `carbon` / `ember` / `dark` / `light` 重复（只改 settings.json 的 theme 值）。

Expected：
- Graphite/Carbon/Ember 三套配色、圆角、字号明显不同；
- 图标（左侧 SVG、文件树、窗口按钮）与布局和改动前完全一致；
- Dark/Light 与改动前视觉一致（只可能多出 border_strong 的使用，不影响主界面）。

- [ ] **Step 5: 清理临时数据**

```powershell
Remove-Item -Recurse -Force "d:\opensource\neverwork\pic\.tmp_skin_*"
Remove-Item Env:NW_CONFIG_DIR
```

- [ ] **Step 6: Commit（默认跳过）**

如有收尾修正，一并提交；否则跳过。

---

## 自审记录

- **Spec 覆盖**：主题集合与迁移（Task 1-2）、色板 + accent_fg + 语义色（Task 3）、形态 token（Task 3）、主界面替换（Task 4-9）、Carbon mono（Task 10）、验收与残留检查（Task 11）。`NW_THEME` 已按「只改样式、不加功能」要求移除，改用 `NW_CONFIG_DIR` + 临时 settings.json 截图。
- **无占位符**：所有代码步骤给出可粘贴代码；机械替换给出精确映射表与逐文件 grep 校验。
- **类型一致**：`migrated()` / `migrate_theme()` / `accent_fg()` / `border_strong()` / `radius_*()` / `text_*()` / `mono_nums()` / `body_px()` 在定义处与使用处同名同签名。
- **红线**：不改图标、布局、交互、编辑器代码字号与任何功能模块；唯一行为面变更是主题列表与旧值迁移（保证旧配置不丢 Key）。
