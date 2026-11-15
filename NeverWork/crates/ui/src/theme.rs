//! NeverWork 设计令牌 —— 对齐 `design/DESIGN-SPEC.md`。
//!
//! 颜色统一返回 `gpui::Rgba`，通过 `gpui::rgb(u32)` 构造。

use gpui::Rgba;

// ============ 背景层级 ============
/// 窗口最底。
pub fn bg() -> Rgba {
    gpui::rgb(0x181818)
}
/// 编辑区 / 主内容。
pub fn surface() -> Rgba {
    gpui::rgb(0x1e1e1e)
}
/// 左右侧栏 / 标题栏。
pub fn panel() -> Rgba {
    gpui::rgb(0x202020)
}
/// hover 背景 / 气泡 / 代码块头。
pub fn raised() -> Rgba {
    gpui::rgb(0x252526)
}

// ============ 边框 ============
pub fn border() -> Rgba {
    gpui::rgb(0x2d2d30)
}

// ============ 文字 ============
pub fn fg() -> Rgba {
    gpui::rgb(0xd4d4d4)
}
pub fn muted() -> Rgba {
    gpui::rgb(0x8a8a8a)
}
pub fn dim() -> Rgba {
    gpui::rgb(0x5f5f5f)
}

// ============ 强调 / 语义 ============
pub fn accent() -> Rgba {
    gpui::rgb(0x3b82f6)
}
pub fn accent_soft() -> Rgba {
    with_alpha(0x3b82f6, 0.14)
}
pub fn accent_line() -> Rgba {
    with_alpha(0x3b82f6, 0.5)
}
pub fn green() -> Rgba {
    gpui::rgb(0x22c55e)
}
pub fn green_soft() -> Rgba {
    with_alpha(0x22c55e, 0.07)
}
pub fn green_line() -> Rgba {
    with_alpha(0x22c55e, 0.25)
}
pub fn yellow() -> Rgba {
    gpui::rgb(0xeab308)
}
pub fn red() -> Rgba {
    gpui::rgb(0xef4444)
}
pub fn purple() -> Rgba {
    gpui::rgb(0xa855f7)
}
pub fn orange() -> Rgba {
    gpui::rgb(0xf97316)
}
pub fn cyan() -> Rgba {
    gpui::rgb(0x06b6d4)
}
pub fn pink() -> Rgba {
    gpui::rgb(0xec4899)
}
pub fn indigo() -> Rgba {
    gpui::rgb(0x6366f1)
}

/// 给 24 位色值附加 alpha（0.0–1.0）。
pub fn with_alpha(hex: u32, alpha: f32) -> Rgba {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
    gpui::rgba((hex << 8) | (a & 0xff))
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
