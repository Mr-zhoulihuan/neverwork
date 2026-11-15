//! NeverWork 共享 UI 组件。全部基于 GPUI 的 `div()` 组合。

use gpui::prelude::*;
use gpui::{div, px, Div, FontWeight, Hsla, IntoElement, SharedString};

use crate::theme;

/// 一个 Agent 身份圆形头像（用 div 圆角块 + 首字母模拟）。
pub fn avatar(initial: &str, color: Hsla, size: f32) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded_md()
        .bg(color)
        .text_color(theme::fg())
        .text_sm()
        .font_weight(FontWeight::BOLD)
        .child(SharedString::from(initial.to_string()))
}

/// 小组徽标（如"规划""编码"）。
pub fn tag(text: &str) -> Div {
    div()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .bg(theme::raised())
        .border_1()
        .border_color(theme::border())
        .text_color(theme::dim())
        .text_xs()
        .child(SharedString::from(text.to_string()))
}

/// 状态点（绿点 + 文字）。
pub fn status_dot(label: &str) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(div().size(px(7.0)).rounded_md().bg(theme::green()))
        .child(div().text_xs().text_color(theme::muted()).child(SharedString::from(label.to_string())))
}

/// 分段按钮（模式切换中的一项）。`on` 表示选中。
pub fn segment(label: &str, on: bool) -> Div {
    let base = div()
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .h(px(30.0))
        .flex_1()
        .rounded_md()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD);
    if on {
        base.bg(theme::accent_soft()).text_color(theme::accent())
    } else {
        base.text_color(theme::muted())
    }
    .child(SharedString::from(label.to_string()))
}

/// 会话列表项。
pub fn session_item(title: &str, time: &str, on: bool) -> Div {
    let base = div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(32.0))
        .px_2()
        .rounded_md()
        .text_sm();
    let row = if on {
        base.bg(theme::accent_soft()).text_color(theme::fg())
    } else {
        base.text_color(theme::muted())
    };
    row.child(div().text_xs().text_color(theme::dim()).child("◐"))
        .child(div().flex_shrink_0().child(SharedString::from(title.to_string())))
        .child(div().flex_1())
        .child(div().text_xs().text_color(theme::dim()).child(SharedString::from(time.to_string())))
}

/// 文件树行。
pub fn file_row(name: &str, level: u8, is_dir: bool, modified: bool, owner: Option<&str>, on: bool) -> Div {
    let indent = 8.0 + (level as f32) * 12.0;
    let base = div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(28.0))
        .pl(px(indent))
        .pr_2()
        .rounded_sm()
        .text_sm();
    let row = if on {
        base.bg(theme::raised()).text_color(theme::fg())
    } else {
        base.text_color(theme::muted())
    };
    let icon = if is_dir { "▣" } else { "⬢" };
    let mut row = row
        .child(div().text_xs().text_color(theme::dim()).child(SharedString::from(icon.to_string())))
        .child(div().flex_1().child(SharedString::from(name.to_string())));
    if let Some(o) = owner {
        row = row.child(
            div()
                .px_1()
                .rounded_sm()
                .bg(theme::accent())
                .text_color(theme::fg())
                .text_xs()
                .child(SharedString::from(o.to_string())),
        );
    } else if modified {
        row = row.child(div().text_xs().text_color(theme::yellow()).child("M"));
    }
    row
}

/// 卡片容器（边框 + 圆角 + 内边距）。
pub fn card() -> Div {
    div()
        .rounded_lg()
        .border_1()
        .border_color(theme::border())
        .bg(theme::raised())
        .p_3()
}

/// 主按钮（实心强调色）。
pub fn primary_button(label: &str) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .h(px(32.0))
        .px_4()
        .rounded_md()
        .bg(theme::accent())
        .text_color(theme::fg())
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(SharedString::from(label.to_string()))
}

/// 幽灵按钮（描边）。
pub fn ghost_button(label: &str) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(32.0))
        .px_3()
        .rounded_md()
        .border_1()
        .border_color(theme::border())
        .text_color(theme::fg())
        .text_sm()
        .child(SharedString::from(label.to_string()))
}

/// 分隔线。
pub fn divider() -> Div {
    div().h(px(1.0)).w_full().bg(theme::border())
}

/// 小工具：一段固定宽度空白。
pub fn spacer() -> Div {
    div().flex_1()
}

/// 让外部可直接使用的边界宽度常量透传。
pub fn px_w(w: f32) -> gpui::Pixels {
    px(w)
}

/// 文章/正文文本。
pub fn body_text(text: &str, color: Hsla) -> Div {
    div().text_sm().text_color(color).child(SharedString::from(text.to_string()))
}

/// 允许组件被 `.child()` 消费（Div 已实现 IntoElement，这里仅作占位以防类型推断问题）。
pub fn into_el(d: Div) -> impl IntoElement {
    d
}
