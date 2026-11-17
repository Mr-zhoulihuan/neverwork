//! 命令面板（标题栏 ⌘K）与工作区切换浮层。

use gpui::prelude::*;
use gpui::{div, px, Context, FontWeight, SharedString};

use neverwork_ui::theme;

use crate::state::{AppState, Command, InputTarget};

impl AppState {
    pub(crate) fn palette_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut card = div()
            .flex()
            .flex_col()
            .w(px(460.0))
            .rounded_lg()
            .bg(theme::panel())
            .border_1()
            .border_color(theme::border())
            .overflow_hidden();

        card = card.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_4()
                .py_3()
                .border_b_1()
                .border_color(theme::border())
                .child(div().text_sm().font_weight(FontWeight::BOLD).child("命令"))
                .child(div().flex_1())
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::dim())
                        .child("Esc 关闭"),
                ),
        );

        for (cmd, label, hint) in Command::all() {
            card = card.child(
                div()
                    .id(SharedString::from(format!("cmd-{}", label)))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::raised()))
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                        this.run_command(cmd, cx);
                    }))
                    .child(div().text_sm().text_color(theme::fg()).child(SharedString::from(label)))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .child(SharedString::from(hint)),
                    ),
            );
        }

        // 工作区切换：内联输入，回车生效。
        if self.ws_prompt_open {
            let text = if self.ws_input.text().is_empty() {
                "输入工作区路径或 ssh://user@host:/path，回车切换".to_string()
            } else {
                self.ws_input.text().to_string()
            };
            let color = if self.ws_input.text().is_empty() {
                theme::dim()
            } else {
                theme::fg()
            };
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_4()
                    .py_3()
                    .border_t_1()
                    .border_color(theme::border())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_xs().text_color(theme::accent()).child("▤"))
                            .child(
                                div()
                                    .id("ws-input")
                                    .flex_1()
                                    .text_sm()
                                    .text_color(color)
                                    .track_focus(&self.ws_focus)
                                    .cursor(gpui::CursorStyle::IBeam)
                                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, window, cx| {
                                        this.switch_input_target(InputTarget::Workspace);
                                        window.focus(&this.ws_focus);
                                        cx.notify();
                                    }))
                                    .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
                                        this.on_workspace_key(ev, cx);
                                    }))
                                    .child(SharedString::from(text)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::dim())
                                    .child(SharedString::from(format!(
                                        "当前：{}",
                                        self.remote
                                            .as_ref()
                                            .map(|t| t.url())
                                            .or_else(|| {
                                                self.root
                                                    .as_ref()
                                                    .map(|r| r.display().to_string())
                                            })
                                            .unwrap_or_else(|| "未选择".into())
                                    ))),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .id("ws-apply")
                                    .px_3()
                                    .py_1()
                                    .rounded_md()
                                    .bg(theme::accent())
                                    .text_xs()
                                    .text_color(theme::fg())
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                                        this.apply_workspace_input(cx);
                                    }))
                                    .child("切换"),
                            ),
                    ),
            );
        }

        div()
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x00000088))
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_: &gpui::MouseDownEvent, _w, _cx| {})
            .child(card)
    }
}
