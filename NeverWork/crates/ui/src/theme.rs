//! NeverWork 设计令牌 —— 对齐 `design/DESIGN-SPEC.md`。
//!
//! 多套**完整主题色板**（深色 / 浅色 / Nord / Dracula / One Dark /
//! Solarized 浅深 / Gruvbox）通过全局模式切换：`set_theme_mode(...)` 之后，
//! 所有 `theme::xxx()` 调用立刻返回新色板的值，视图无需改一行代码。
//! 模式存在原子变量里，只在主线程写、渲染线程读，避免给每个视图传上下文。

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
    fg: u32,
    muted: u32,
    dim: u32,
    accent: u32,
    /// 浅色底：语义色（绿/黄/红…）取暗档。
    light: bool,
}

/// 下标与 `ThemeMode::all()` 一一对应。
static PALETTES: [Palette; 9] = [
    // Dark（默认）
    Palette {
        bg: 0x181818,
        surface: 0x1e1e1e,
        panel: 0x202020,
        raised: 0x252526,
        border: 0x2d2d30,
        fg: 0xd4d4d4,
        muted: 0x8a8a8a,
        dim: 0x5f5f5f,
        accent: 0x3b82f6,
        light: false,
    },
    // Light
    Palette {
        bg: 0xf4f4f5,
        surface: 0xffffff,
        panel: 0xfafafa,
        raised: 0xececef,
        border: 0xe0e0e4,
        fg: 0x1f2023,
        muted: 0x6b6d76,
        dim: 0x9aa0a6,
        accent: 0x2563eb,
        light: true,
    },
    // System（解析后不会落在这里，只为对齐下标 / 色卡预览）
    Palette {
        bg: 0x181818,
        surface: 0x1e1e1e,
        panel: 0x202020,
        raised: 0x252526,
        border: 0x2d2d30,
        fg: 0xd4d4d4,
        muted: 0x8a8a8a,
        dim: 0x5f5f5f,
        accent: 0x3b82f6,
        light: false,
    },
    // Nord：冷调蓝灰
    Palette {
        bg: 0x2e3440,
        surface: 0x2b303b,
        panel: 0x333a46,
        raised: 0x3b4252,
        border: 0x434c5e,
        fg: 0xd8dee9,
        muted: 0xa0a8b8,
        dim: 0x6d7a8c,
        accent: 0x88c0d0,
        light: false,
    },
    // Dracula：紫黑底 + 紫强调
    Palette {
        bg: 0x282a36,
        surface: 0x2b2d3a,
        panel: 0x242631,
        raised: 0x343746,
        border: 0x44475a,
        fg: 0xf8f8f2,
        muted: 0xa8adc4,
        dim: 0x6272a4,
        accent: 0xbd93f9,
        light: false,
    },
    // One Dark
    Palette {
        bg: 0x282c34,
        surface: 0x2c313a,
        panel: 0x21252b,
        raised: 0x333842,
        border: 0x3e4451,
        fg: 0xabb2bf,
        muted: 0x8b93a3,
        dim: 0x5c6370,
        accent: 0x61afef,
        light: false,
    },
    // Solarized 浅：暖米色纸感
    Palette {
        bg: 0xfdf6e3,
        surface: 0xfffbf0,
        panel: 0xf5efdc,
        raised: 0xeee8d5,
        border: 0xe4dcc6,
        fg: 0x586e75,
        muted: 0x839496,
        dim: 0x93a1a1,
        accent: 0x268bd2,
        light: true,
    },
    // Solarized 深：青黑底
    Palette {
        bg: 0x002b36,
        surface: 0x073642,
        panel: 0x00252e,
        raised: 0x0a3d4a,
        border: 0x11505f,
        fg: 0x93a1a1,
        muted: 0x8a9a9a,
        dim: 0x586e75,
        accent: 0x268bd2,
        light: false,
    },
    // Gruvbox：暖褐 + 琥珀
    Palette {
        bg: 0x282828,
        surface: 0x32302f,
        panel: 0x2c2a29,
        raised: 0x3c3836,
        border: 0x504945,
        fg: 0xebdbb2,
        muted: 0xa89984,
        dim: 0x7c6f64,
        accent: 0xd79921,
        light: false,
    },
];

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
pub fn green() -> Rgba {
    pick(0x22c55e, 0x16a34a)
}
pub fn green_soft() -> Rgba {
    with_alpha(if is_light() { 0x16a34a } else { 0x22c55e }, 0.10)
}
pub fn green_line() -> Rgba {
    with_alpha(if is_light() { 0x16a34a } else { 0x22c55e }, 0.28)
}
pub fn yellow() -> Rgba {
    pick(0xeab308, 0xca8a04)
}
pub fn red() -> Rgba {
    pick(0xef4444, 0xdc2626)
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

    #[test]
    fn mode_switch_changes_colors() {
        set_theme_mode(ThemeMode::Dark);
        let dark_bg = bg();
        assert!(!is_light());
        set_theme_mode(ThemeMode::Light);
        let light_bg = bg();
        assert!(is_light());
        assert_ne!(dark_bg, light_bg, "深浅两套背景必须不同");
        // 浅色背景应更亮
        assert!(light_bg.r > dark_bg.r);
        set_theme_mode(ThemeMode::Dark);
    }

    #[test]
    fn foreground_inverts_with_theme() {
        set_theme_mode(ThemeMode::Dark);
        let d = fg();
        set_theme_mode(ThemeMode::Light);
        let l = fg();
        assert!(d.r > l.r, "深色主题的文字应更亮");
        set_theme_mode(ThemeMode::Dark);
    }

    #[test]
    fn each_palette_has_its_own_colors() {
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
        set_theme_mode(ThemeMode::Dark);
    }

    #[test]
    fn with_alpha_clamps() {
        assert_eq!(with_alpha(0xffffff, 2.0).a, 1.0);
        assert_eq!(with_alpha(0xffffff, -1.0).a, 0.0);
    }

    #[test]
    fn solarized_light_counts_as_light() {
        set_theme_mode(ThemeMode::SolarizedLight);
        assert!(is_light());
        set_theme_mode(ThemeMode::SolarizedDark);
        assert!(!is_light());
        set_theme_mode(ThemeMode::Dark);
    }
}
