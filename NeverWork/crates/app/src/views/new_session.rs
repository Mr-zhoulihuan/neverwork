//! 新建会话浮层：先选运行模式（单助手 / 团队），创建后不可切换。
//!
//! 为什么不让切：单助手与团队的工具面、事件流、气泡样式完全不同，
//! 中途切换会让同一会话历史里两种发言混在一起，角色上下文也接不上。
//! 需要另一种模式就再新建一个会话。

use gpui::prelude::*;
use gpui::{div, px, Context, FontWeight, SharedString};

use neverwork_core::SessionMode;
use neverwork_ui::theme;

use crate::state::AppState;

impl AppState {
    pub(crate) fn new_session_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let team = self
            .config
            .active_team()
            .cloned()
            .unwrap_or_else(neverwork_core::Team::default_team);

        let mut card = div()
            .flex()
            .flex_col()
            .w(px(560.0))
            .rounded_lg()
            .bg(theme::panel())
            .border_1()
            .border_color(theme::border())
            .overflow_hidden();

        // ---- 标题 ----
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_5()
                .py_4()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .child("新建会话"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::dim())
                                .child("先选运行模式；创建后不能切换，需要另一种请再新建一个会话"),
                        ),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("new-session-cancel")
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .text_xs()
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::raised()))
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                            this.close_new_session_picker(cx);
                        }))
                        .child("Esc"),
                ),
        );

        // ---- 选项一：单助手 ----
        card = card.child(
            div()
                .id("pick-solo")
                .flex()
                .items_start()
                .gap_3()
                .mx_4()
                .mt_4()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(theme::border())
                .bg(theme::raised())
                .cursor_pointer()
                .hover(|d| d.bg(theme::border()))
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                    this.new_session(SessionMode::Solo, cx);
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(34.0))
                        .flex_shrink_0()
                        .rounded_lg()
                        .bg(theme::with_alpha(0x888888, 0.16))
                        .text_sm()
                        .text_color(theme::muted())
                        .child("◎"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::fg())
                                .child("单助手"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::muted())
                                .child("一个助手带工具：问答、读文件、改代码都由它负责。适合问答、单点修改与快速迭代。"),
                        ),
                ),
        );

        // ---- 选项二：团队（当前团队的角色一目了然）----
        let mut roles_row = div().flex().items_center().gap_2().flex_wrap();
        if team.roles.is_empty() {
            roles_row = roles_row.child(
                div()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("团队里还没有角色，创建后去「团队」页添加"),
            );
        } else {
            for role in team.roles.iter().take(6) {
                roles_row = roles_row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_1p5()
                        .py_0p5()
                        .rounded_full()
                        .bg(theme::with_alpha(0x888888, 0.12))
                        .child(
                            div()
                                .size(px(14.0))
                                .rounded_full()
                                .bg(theme::role_color(role.color_index)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::muted())
                                .child(SharedString::from(role.name.clone())),
                        ),
                );
            }
            if team.roles.len() > 6 {
                roles_row = roles_row.child(
                    div()
                        .text_xs()
                        .text_color(theme::dim())
                        .child(SharedString::from(format!("+{}", team.roles.len() - 6))),
                );
            }
        }

        card = card.child(
            div()
                .id("pick-team")
                .flex()
                .items_start()
                .gap_3()
                .mx_4()
                .mt_3()
                .mb_4()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(theme::border())
                .bg(theme::raised())
                .cursor_pointer()
                .hover(|d| d.bg(theme::border()))
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                    this.new_session(SessionMode::Team, cx);
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(34.0))
                        .flex_shrink_0()
                        .rounded_lg()
                        .bg(theme::accent_soft())
                        .text_sm()
                        .text_color(theme::accent())
                        .child("◈"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme::fg())
                                        .child(SharedString::from(format!(
                                            "团队 · {}",
                                            team.name
                                        ))),
                                )
                                .child(
                                    div()
                                        .px_1()
                                        .rounded_sm()
                                        .bg(theme::accent_soft())
                                        .text_xs()
                                        .text_color(theme::accent())
                                        .child("多角色交接"),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::muted())
                                .child("多角色按交接顺序协作；用 @角色名 指定入口角色，每个角色可绑定插件（技能）。适合分阶段的大任务。"),
                        )
                        .child(roles_row),
                ),
        );

        // ---- 遮罩：与命令面板一致的浮层行为 ----
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
