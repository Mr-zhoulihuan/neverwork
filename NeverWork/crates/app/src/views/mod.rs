//! 视图层：主界面各区域的渲染。
//!
//! `state.rs` 只保留状态与行为；所有 `div` 组装集中在这里。

pub(crate) mod chat;
pub(crate) mod editor;
pub(crate) mod file_menu;
pub(crate) mod files;
pub(crate) mod market_detail;
pub(crate) mod model_picker;
pub(crate) mod new_session;
pub(crate) mod palette;
pub(crate) mod providers;
pub(crate) mod session_menu;
pub(crate) mod settings;
pub(crate) mod sidebar;
pub(crate) mod skills;
pub(crate) mod text_fit;
pub(crate) mod team;
pub(crate) mod workspace_picker;

use gpui::prelude::*;
use gpui::{
    div, point, px, size, AnyElement, Bounds, Context, FontWeight, HitboxBehavior, IntoElement,
    SharedString, WindowControlArea,
};
use neverwork_core::ConnectionKind;
use neverwork_ui::theme;

use crate::state::{AppState, CenterView};

impl AppState {
    pub(crate) fn center_area(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.center {
            CenterView::Chat => self.chat_pane(cx).into_any_element(),
            CenterView::File => self.editor_pane(cx).into_any_element(),
            CenterView::Team => self.team_pane(cx).into_any_element(),
            CenterView::Settings => self.settings_pane(cx).into_any_element(),
            CenterView::Skills => self.skills_pane(cx).into_any_element(),
        }
    }
}

pub(crate) fn titlebar(
    state: &AppState,
    maximized: bool,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let (ws_name, kind) = match state.workspace.as_ref() {
        Some(w) if w.kind == ConnectionKind::Remote => (w.name.as_str(), " · 远程"),
        Some(w) => (w.name.as_str(), ""),
        None => ("未选择工作区", ""),
    };
    div()
        .relative()
        .flex()
        .items_center()
        .gap_3()
        .h(px(theme::TITLEBAR_H))
        .pl_3()
        .bg(theme::panel())
        .border_b_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .child(div().font_weight(FontWeight::BOLD).child("NeverWork"))
                .child(
                    div()
                        .text_color(theme::muted())
                        .child(SharedString::from(format!("— {ws_name}{kind}"))),
                ),
        )
        .child(div().flex_1())
        // 调试读数：布局实际使用的可见宽度。只在 NW_LAYOUT_DEBUG 时出现。
        .when(std::env::var("NW_LAYOUT_DEBUG").is_ok(), |d| {
            d.child(
                div()
                    .text_xs()
                    .text_color(theme::yellow())
                    .child(SharedString::from(format!("W={:.0}", state.layout_w.get()))),
            )
        })
        .child(
            div()
                .id("toggle-focus")
                .px_2()
                .py_1()
                .rounded_md()
                .text_xs()
                .text_color(if state.focus_mode { theme::accent() } else { theme::muted() })
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()))
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                    this.toggle_focus_mode(cx);
                }))
                .child(if state.focus_mode { "◧ 退出专注" } else { "◧ 专注" }),
        )
        .child(window_controls(maximized))
        .child(titlebar_hitboxes())
}

/// 标题栏右上角的窗口控制按钮（自绘）：最小化 / 最大化（还原） / 关闭。
///
/// 窗口是无边框的（`appears_transparent`），所以这三个按钮要自己画；
/// 点击不用 `on_click` —— 对应的系统热区在 [`titlebar_hitboxes`] 里注册，
/// 由 Windows 直接执行最小化 / 最大化 / 关闭。`maximized` 决定第二颗
/// 按钮画「最大化」还是「还原」图标（窗口尺寸变化会触发重绘）。
///
/// 图标用图形画而不是字符：`□`/`❐` 这类字形会被字体回退成大小相近
/// 的方块，最大化前后几乎看不出区别。
fn window_controls(maximized: bool) -> impl IntoElement {
    fn button(id: &'static str, icon: impl IntoElement) -> impl IntoElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(44.0))
            .h_full()
            .child(icon)
    }
    div()
        .flex()
        .items_center()
        .h_full()
        .child(button("win-min", icon_minimize()))
        .child(button(
            "win-max",
            if maximized {
                icon_restore().into_any_element()
            } else {
                icon_maximize().into_any_element()
            },
        ))
        .child(button("win-close", icon_close()))
}

/// 最小化：一条 10px 横线。
fn icon_minimize() -> impl IntoElement {
    div().w(px(10.0)).h(px(1.0)).bg(theme::fg())
}

/// 最大化：10px 空心方框。
fn icon_maximize() -> impl IntoElement {
    div()
        .w(px(10.0))
        .h(px(10.0))
        .border_1()
        .border_color(theme::fg())
}

/// 还原：两个错位小方框（后层左下、前层右上），前层用标题栏底色
/// 盖住后层被遮挡的那一角。
fn icon_restore() -> impl IntoElement {
    div()
        .relative()
        .w(px(10.0))
        .h(px(10.0))
        .child(
            div()
                .absolute()
                .left(px(0.0))
                .bottom(px(0.0))
                .w(px(8.0))
                .h(px(8.0))
                .border_1()
                .border_color(theme::fg()),
        )
        .child(
            div()
                .absolute()
                .right(px(0.0))
                .top(px(0.0))
                .w(px(8.0))
                .h(px(8.0))
                .border_1()
                .border_color(theme::fg())
                .bg(theme::panel()),
        )
}

/// 关闭：15px 的 ✕ 字符（字体对它的渲染没问题）。
fn icon_close() -> impl IntoElement {
    div()
        .text_size(px(15.0))
        .text_color(theme::fg())
        .child("✕")
}

/// Windows 无边框窗口的命中热区层（绝对定位铺满标题栏，只注册热区不绘制）。
///
/// - 左侧整片是拖拽区（`WindowControlArea::Drag`）：拖动、双击最大化、
///   右键弹系统菜单都由 Windows 按 HTCAPTION 处理，应用层不用写代码；
/// - 最右侧三个按钮注册为系统窗口控制区（最小化 / 最大化还原 / 关闭），
///   点击由系统直接执行，不需要 `on_click`。
///
/// 右侧「专注 / 窗口按钮」这一段的宽度要预留出来不铺拖拽热区，
/// 否则 HTCAPTION 会先截走鼠标，这几个按钮点不动。
fn titlebar_hitboxes() -> impl IntoElement {
    const RIGHT_RESERVED: f32 = 300.0;
    const BTN_W: f32 = 44.0;
    gpui::canvas(
        move |bounds, window, _cx| {
            let h = bounds.size.height;
            let drag_w = (bounds.size.width - px(RIGHT_RESERVED)).max(px(100.0));
            let drag = window.insert_hitbox(
                Bounds::new(bounds.origin, size(drag_w, h)),
                HitboxBehavior::Normal,
            );
            let buttons: Vec<(WindowControlArea, _)> =
                [WindowControlArea::Min, WindowControlArea::Max, WindowControlArea::Close]
                    .into_iter()
                    .enumerate()
                    .map(|(i, area)| {
                        let x = bounds.origin.x + bounds.size.width - px(BTN_W * (3.0 - i as f32));
                        let hitbox = window.insert_hitbox(
                            Bounds::new(point(x, bounds.origin.y), size(px(BTN_W), h)),
                            HitboxBehavior::Normal,
                        );
                        (area, hitbox)
                    })
                    .collect();
            (drag, buttons)
        },
        |_bounds, (drag, buttons), window, _cx| {
            window.insert_window_control_hitbox(WindowControlArea::Drag, drag);
            for (area, hitbox) in buttons {
                window.insert_window_control_hitbox(area, hitbox);
            }
        },
    )
    .absolute()
    .inset_0()
}

pub(crate) fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}
