//! 视图层：主界面各区域的渲染。
//!
//! `state.rs` 只保留状态与行为；所有 `div` 组装集中在这里。

pub(crate) mod chat;
pub(crate) mod editor;
pub(crate) mod files;
pub(crate) mod model_picker;
pub(crate) mod palette;
pub(crate) mod settings;
pub(crate) mod sidebar;
pub(crate) mod skills;
pub(crate) mod text_fit;
pub(crate) mod team;

use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, FontWeight, IntoElement, SharedString};
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

pub(crate) fn titlebar(state: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let kind = if state.workspace.kind == ConnectionKind::Remote {
        "· 远程"
    } else {
        ""
    };
    div()
        .flex()
        .items_center()
        .gap_3()
        .h(px(theme::TITLEBAR_H))
        .px_3()
        .bg(theme::panel())
        .border_b_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .gap_2()
                .child(div().size(px(12.0)).rounded_md().bg(theme::red()))
                .child(div().size(px(12.0)).rounded_md().bg(theme::yellow()))
                .child(div().size(px(12.0)).rounded_md().bg(theme::green())),
        )
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
                        .child(SharedString::from(format!(
                            "— {} {kind}",
                            state.workspace.name
                        ))),
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
        .child(
            div()
                .id("open-palette")
                .px_2()
                .py_1()
                .rounded_md()
                .text_xs()
                .text_color(theme::muted())
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()))
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                    this.toggle_palette(cx);
                }))
                .child("⌘K 命令"),
        )
}

pub(crate) fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}
