//! NeverWork 设计令牌 —— 对齐 `docs/superpowers/specs/2026-11-18-product-skins-design.md`。
//!
//! 6 套**完整主题色板**（深色 / 浅色 / 跟随系统 / Graphite / Carbon / Ember）
//! 通过全局模式切换：`set_theme_mode(...)` 之后，所有 `theme::xxx()` 调用立刻
//! 返回新色板的值，视图无需改一行代码。模式存在原子变量里，只在主线程写、
//! 渲染线程读，避免给每个视图传上下文。形态 token（圆角 / 字号 / mono 数字）
//! 与色板平行，同一套下标。

use std::sync::atomic::{AtomicU8, Ordering};

use gpui::Rgba;

use neverwork_core::ThemeMode;

/// 当前主题在 `ThemeMode::all()` 里的下标（`System` 会先解析成具体的深浅）。
static MODE: AtomicU8 = AtomicU8::new(0);

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
    // Graphite：近黑中性灰 + 鞑蓝
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

fn palette_of(mode: ThemeMode) -> &'static Palette {
    &PALETTES[mode.index().min(PALETTES.len() - 1)]
}

fn palette() -> &'static Palette {
    palette_of(current_mode())
}

/// 当前生效的主题（`System` 已解析）。
pub fn current_mode() -> ThemeMode {
    let all = ThemeMode::all();
    let idx = (MODE.load(Ordering::Relaxed) as usize).min(all.len() - 1);
    all[idx]
}

/// 当前是否为浅色底。
pub fn is_light() -> bool {
    palette().light
}

/// 设置主题。`System` 在这里按 Windows 深浅解析成 `Dark` / `Light`。
pub fn set_theme_mode(mode: ThemeMode) {
    let resolved = mode.resolved(system_is_light());
    MODE.store(resolved.index() as u8, Ordering::Relaxed);
}

/// 色卡预览：(背景, 文字, 强调)；用于设置页的主题选择器。
/// `System` 预览成深色，避免每帧去读注册表。
pub fn palette_preview(mode: ThemeMode) -> (Rgba, Rgba, Rgba) {
    let p = palette_of(mode.resolved(false));
    (gpui::rgb(p.bg), gpui::rgb(p.fg), gpui::rgb(p.accent))
}

/// 系统是否浅色。GPUI 暴露窗口外观，但这里没有窗口句柄，
/// 用 Windows 的注册表项判断（读不到时退回深色）。
fn system_is_light() -> bool {
    #[cfg(windows)]
    {
        // HKEY_CURRENT_USER\...\Themes\Personalize\AppsUseLightTheme
        let mut cmd = std::process::Command::new("reg");
        cmd.args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            "AppsUseLightTheme",
        ]);
        {
            use std::os::windows::process::CommandExt;
            // 无窗口应用里子进程默认会弹控制台；CREATE_NO_WINDOW 压住它。
            cmd.creation_flags(0x0800_0000);
        }
        let out = cmd.output();
        if let Ok(o) = out {
            let text = String::from_utf8_lossy(&o.stdout);
            // 输出形如 `    AppsUseLightTheme    REG_DWORD    0x1`
            if let Some(pos) = text.rfind("0x") {
                return text[pos..].trim().starts_with("0x1");
            }
        }
    }
    false
}

/// 按模式取色：深色值 / 浅色值。
fn pick(dark: u32, light: u32) -> Rgba {
    gpui::rgb(if is_light() { light } else { dark })
}

// ============ 背景层级 ============
/// 窗口最底。
pub fn bg() -> Rgba {
    gpui::rgb(palette().bg)
}
/// 编辑区 / 主内容。
pub fn surface() -> Rgba {
    gpui::rgb(palette().surface)
}
/// 左右侧栏 / 标题栏。
pub fn panel() -> Rgba {
    gpui::rgb(palette().panel)
}
/// hover 背景 / 气泡 / 代码块头。
pub fn raised() -> Rgba {
    gpui::rgb(palette().raised)
}

// ============ 边框 ============
pub fn border() -> Rgba {
    gpui::rgb(palette().border)
}
/// 弹层 / 选中态的强边框。
pub fn border_strong() -> Rgba {
    gpui::rgb(palette().border_strong)
}

// ============ 文字 ============
pub fn fg() -> Rgba {
    gpui::rgb(palette().fg)
}
pub fn muted() -> Rgba {
    gpui::rgb(palette().muted)
}
pub fn dim() -> Rgba {
    gpui::rgb(palette().dim)
}

// ============ 强调 / 语义 ============
pub fn accent() -> Rgba {
    gpui::rgb(palette().accent)
}
pub fn accent_soft() -> Rgba {
    with_alpha(palette().accent, 0.14)
}
pub fn accent_line() -> Rgba {
    with_alpha(palette().accent, 0.5)
}
/// accent 填充底上的文字色。
pub fn accent_fg() -> Rgba {
    gpui::rgb(palette().accent_fg)
}
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
pub fn purple() -> Rgba {
    pick(0xa855f7, 0x9333ea)
}
pub fn orange() -> Rgba {
    pick(0xf97316, 0xea580c)
}
pub fn cyan() -> Rgba {
    pick(0x06b6d4, 0x0891b2)
}
pub fn pink() -> Rgba {
    pick(0xec4899, 0xdb2777)
}
pub fn indigo() -> Rgba {
    pick(0x6366f1, 0x4f46e5)
}

/// 给 24 位色值附加 alpha（0.0–1.0）。
pub fn with_alpha(hex: u32, alpha: f32) -> Rgba {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
    gpui::rgba((hex << 8) | (a & 0xff))
}

/// 给主题色叠透明度（文件类型徽标、胶囊底色等用）。
pub fn tint(color: Rgba, alpha: f32) -> Rgba {
    let mut c = color;
    c.a = alpha.clamp(0.0, 1.0);
    c
}

// ============ 字体 ============
/// 等宽字体族名（代码块、执行命令用）。
///
/// 必须按平台给不同候选：Windows 上 "Menlo"/"DejaVu Sans Mono" 都不存在，
/// 一旦 `font_family` 指向不存在的族名，gpui 会回退到比例字体，
/// 代码块的对齐就全乱了（实测 Consolas 写死时在 Linux 上退化成无衬线）。
pub fn mono_family() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "Cascadia Mono"
    }
    #[cfg(target_os = "macos")]
    {
        "SF Mono"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "DejaVu Sans Mono"
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    {
        "monospace"
    }
}

/// 等宽字体候选列表：首选不存在时按顺序回退。
pub fn mono_fallbacks() -> &'static [&'static str] {
    const CANDIDATES: &[&str] = &[
        "Cascadia Mono",
        "Cascadia Code",
        "Consolas",
        "SF Mono",
        "Menlo",
        "DejaVu Sans Mono",
        "Liberation Mono",
        "Ubuntu Mono",
        "Noto Sans Mono",
        "monospace",
    ];
    CANDIDATES
}

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

// ============ 尺寸 ============
pub const SIDEBAR_W: f32 = 236.0;
pub const FILES_W: f32 = 300.0;
pub const TITLEBAR_H: f32 = 36.0;
pub const CHAT_MAX_W: f32 = 780.0;

// ============ 角色身份色（可按角色索引取色） ============
/// 6 种可选身份色，用于自定义 Agent 角色。
pub fn role_color(index: usize) -> Rgba {
    const COLORS: [u32; 6] = [0xa855f7, 0x3b82f6, 0xf97316, 0x22c55e, 0xec4899, 0x06b6d4];
    gpui::rgb(COLORS[index % COLORS.len()])
}

/// 线性渐变的两端（用于角色头像，简化：返回主色 + 稍暗色）。
pub fn role_color_alt(index: usize) -> Rgba {
    const ALTS: [u32; 6] = [0x6366f1, 0x06b6d4, 0xeab308, 0x10b981, 0xa855f7, 0x3b82f6];
    gpui::rgb(ALTS[index % ALTS.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 主题是全局状态，多个测试并行切换会互相踩（`is_light` 断言随机失败），
    /// 用一把锁把所有碰全局主题的测试串行化。
    static THEME_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn theme_test_guard() -> std::sync::MutexGuard<'static, ()> {
        THEME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn mode_switch_changes_colors() {
        let _guard = theme_test_guard();
        set_theme_mode(ThemeMode::Dark);
        let dark_bg = bg();
        assert!(!is_light());
        set_theme_mode(ThemeMode::Light);
        let light_bg = bg();
        assert!(is_light());
        assert_ne!(dark_bg, light_bg, "深浅两套背景必须不同");
        // 浅色背景应更亮
        assert!(light_bg.r > dark_bg.r);
        set_theme_mode(ThemeMode::Graphite);
    }

    #[test]
    fn foreground_inverts_with_theme() {
        let _guard = theme_test_guard();
        set_theme_mode(ThemeMode::Dark);
        let d = fg();
        set_theme_mode(ThemeMode::Light);
        let l = fg();
        assert!(d.r > l.r, "深色主题的文字应更亮");
        set_theme_mode(ThemeMode::Graphite);
    }

    /// 相对亮度（WCAG 公式），用于 accent / accent_fg 对比度断言。
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
            // 文字与背景必须有对比度
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
        assert_eq!(
            (radius_md(), text_ui(), text_body()),
            (6.0, 14.0, 16.0),
            "Dark 必须维持现状"
        );
        set_theme_mode(ThemeMode::Carbon);
        assert!(mono_nums());
        assert_eq!(radius_md(), 4.0);
        set_theme_mode(ThemeMode::Graphite);
        assert!(!mono_nums());
    }

    #[test]
    fn with_alpha_clamps() {
        assert_eq!(with_alpha(0xffffff, 2.0).a, 1.0);
        assert_eq!(with_alpha(0xffffff, -1.0).a, 0.0);
    }
}
