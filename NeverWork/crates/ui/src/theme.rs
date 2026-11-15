//! NeverWork 设计令牌 —— 对齐 `design/DESIGN-SPEC.md`。
//!
//! 深浅两套色板通过全局模式切换：`set_theme_mode(ThemeMode::Light)` 之后，
//! 所有 `theme::xxx()` 调用立刻返回浅色值，视图无需改一行代码。
//! 模式存在原子变量里，只在主线程写、渲染线程读，避免给每个视图传上下文。

use std::sync::atomic::{AtomicU8, Ordering};

use gpui::Rgba;

use neverwork_core::ThemeMode;

/// 0 = 深色，1 = 浅色。`System` 在解析时由调用方根据系统外观落到这两者之一。
static MODE: AtomicU8 = AtomicU8::new(0);

/// 当前是否为浅色。
pub fn is_light() -> bool {
    MODE.load(Ordering::Relaxed) == 1
}

/// 设置主题模式。`System` 需要调用方先用 `resolve_system_light()` 解析。
pub fn set_theme_mode(mode: ThemeMode) {
    let light = match mode {
        ThemeMode::Dark => false,
        ThemeMode::Light => true,
        ThemeMode::System => system_is_light(),
    };
    MODE.store(if light { 1 } else { 0 }, Ordering::Relaxed);
}

/// 系统是否浅色。GPUI 暴露窗口外观，但这里没有窗口句柄，
/// 用 Windows 的注册表项判断（读不到时退回深色）。
fn system_is_light() -> bool {
    #[cfg(windows)]
    {
        // HKEY_CURRENT_USER\...\Themes\Personalize\AppsUseLightTheme
        let out = std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                "/v",
                "AppsUseLightTheme",
            ])
            .output();
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
    pick(0x181818, 0xf4f4f5)
}
/// 编辑区 / 主内容。
pub fn surface() -> Rgba {
    pick(0x1e1e1e, 0xffffff)
}
/// 左右侧栏 / 标题栏。
pub fn panel() -> Rgba {
    pick(0x202020, 0xfafafa)
}
/// hover 背景 / 气泡 / 代码块头。
pub fn raised() -> Rgba {
    pick(0x252526, 0xececef)
}

// ============ 边框 ============
pub fn border() -> Rgba {
    pick(0x2d2d30, 0xe0e0e4)
}

// ============ 文字 ============
pub fn fg() -> Rgba {
    pick(0xd4d4d4, 0x1f2023)
}
pub fn muted() -> Rgba {
    pick(0x8a8a8a, 0x6b6d76)
}
pub fn dim() -> Rgba {
    pick(0x5f5f5f, 0x9aa0a6)
}

// ============ 强调 / 语义 ============
pub fn accent() -> Rgba {
    pick(0x3b82f6, 0x2563eb)
}
pub fn accent_soft() -> Rgba {
    with_alpha(if is_light() { 0x2563eb } else { 0x3b82f6 }, 0.14)
}
pub fn accent_line() -> Rgba {
    with_alpha(if is_light() { 0x2563eb } else { 0x3b82f6 }, 0.5)
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
    fn with_alpha_clamps() {
        assert_eq!(with_alpha(0xffffff, 2.0).a, 1.0);
        assert_eq!(with_alpha(0xffffff, -1.0).a, 0.0);
    }
}
