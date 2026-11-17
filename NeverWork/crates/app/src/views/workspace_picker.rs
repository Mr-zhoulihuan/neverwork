//! 工作区管理器浮层：最近工作区列表（收藏 / 重命名 / 移除）+ 打开新文件夹。
//!
//! 与命令面板同款「全屏遮罩 + 居中卡片」：项目里没有锚定 popup 的先例，
//! 居中卡片在不同窗口尺寸下都不会跑位。

use gpui::prelude::*;
use gpui::{div, px, ClickEvent, Context, FontWeight, SharedString};

use neverwork_ui::theme;

use crate::state::{AppState, InputTarget};

impl AppState {
    pub(crate) fn workspace_popup_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut card = div()
            .flex()
            .flex_col()
            .w(px(520.0))
            .max_h(px(520.0))
            .rounded_lg()
            .bg(theme::panel())
            .border_1()
            .border_color(theme::border())
            .overflow_hidden();

        // ---- 头部 ----
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_4()
                .py_3()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .child("切换工作区"),
                )
                .child(div().flex_1())
                .child(div().text_xs().text_color(theme::dim()).child("Esc 关闭")),
        );

        // ---- 最近列表 ----
        let root_str = self
            .root
            .as_ref()
            .map(|r| r.to_string_lossy().to_string())
            .unwrap_or_default();
        // 远程时当前项按连接串比对（本地 root 为空）
        let current_key = self
            .remote
            .as_ref()
            .map(|t| t.url())
            .unwrap_or_else(|| root_str.clone());
        let mut list = div()
            .id("ws-list")
            .flex()
            .flex_col()
            .gap_1()
            .px_2()
            .py_2()
            .flex_1()
            .min_h(px(0.0))
            .overflow_scroll();
        let order = self.settings.workspace_order();
        if order.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .py_4()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("还没有最近工作区，点下方「打开文件夹」或「连接远程 SSH」开始"),
            );
        }
        for i in order {
            let w = &self.settings.workspaces[i];
            let current = w.path.eq_ignore_ascii_case(&current_key);
            // 远程条目存的是连接串，不能拿到本地磁盘上判存在
            let is_remote = neverwork_core::is_remote_url(&w.path);
            let exists = is_remote || std::path::Path::new(&w.path).is_dir();
            let pinned = w.pinned;
            let display = w.display_name();
            let renaming = self.ws_rename == Some(i);

            let mut row = div()
                .id(SharedString::from(format!("ws-row-{i}")))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1p5()
                .rounded_md()
                .border_1()
                .when(current, |d| {
                    d.bg(theme::raised())
                        .border_color(theme::accent_line())
                })
                .when(!current, |d| {
                    d.border_color(theme::with_alpha(0x000000, 0.0))
                        .hover(|d| d.bg(theme::raised()))
                });
            // 重命名时不响应整行点击，避免一边打字一边被切走
            if !renaming {
                row = row.cursor_pointer().on_click(cx.listener(
                    move |this, _: &ClickEvent, _w, cx| {
                        this.switch_to_recent_workspace(i, cx);
                    },
                ));
            }

            if renaming {
                let value = self.ws_input.text().to_string();
                row = row
                    .child(
                        div()
                            .id("ws-rename-input")
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_sm()
                            .text_color(theme::fg())
                            .cursor(gpui::CursorStyle::IBeam)
                            .track_focus(&self.ws_focus)
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.switch_input_target(InputTarget::Workspace);
                                    window.focus(&this.ws_focus);
                                    cx.notify();
                                },
                            ))
                            .on_key_down(cx.listener(
                                |this, ev: &gpui::KeyDownEvent, _w, cx| {
                                    this.on_workspace_key(ev, cx);
                                },
                            ))
                            .child(SharedString::from(crate::views::text_fit::fit_text(
                                &value, 300.0, 14.0,
                            ))),
                    )
                    .child(
                        div()
                            .id("ws-rename-save")
                            .px_2()
                            .py_0p5()
                            .rounded_sm()
                            .bg(theme::accent())
                            .text_xs()
                            .text_color(theme::bg())
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                cx.stop_propagation();
                                this.commit_rename_workspace(cx);
                            }))
                            .child("保存"),
                    )
                    .child(
                        div()
                            .id("ws-rename-cancel")
                            .px_1()
                            .rounded_sm()
                            .text_xs()
                            .text_color(theme::dim())
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                cx.stop_propagation();
                                this.cancel_rename_workspace(cx);
                            }))
                            .child("✕"),
                    );
            } else {
                row = row
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(if current {
                                theme::accent()
                            } else if is_remote {
                                theme::yellow()
                            } else {
                                theme::green()
                            })
                            .child(if current {
                                "●"
                            } else if is_remote {
                                "⇅"
                            } else {
                                "▤"
                            }),
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
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .whitespace_nowrap()
                                            .text_sm()
                                            .text_color(theme::fg())
                                            .child(SharedString::from(
                                                crate::views::text_fit::fit_text(
                                                    &display, 200.0, 14.0,
                                                ),
                                            )),
                                    )
                                    .when(current, |d| {
                                        d.child(
                                            div()
                                                .flex_shrink_0()
                                                .px_1()
                                                .rounded_sm()
                                                .bg(theme::with_alpha(0x3b82f6, 0.18))
                                                .text_xs()
                                                .text_color(theme::accent())
                                                .child("当前"),
                                        )
                                    })
                                    .when(!exists, |d| {
                                        d.child(
                                            div()
                                                .flex_shrink_0()
                                                .text_xs()
                                                .text_color(theme::red())
                                                .child("不存在"),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .whitespace_nowrap()
                                    .text_xs()
                                    .text_color(theme::dim())
                                    .child(SharedString::from(
                                        crate::views::text_fit::fit_text(&w.path, 340.0, 12.0),
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("ws-rename-{i}")))
                            .flex_shrink_0()
                            .px_1()
                            .rounded_sm()
                            .text_xs()
                            .text_color(theme::dim())
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::border()).text_color(theme::fg()))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                                cx.stop_propagation();
                                this.begin_rename_workspace(i, cx);
                            }))
                            .child("✎"),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("ws-pin-{i}")))
                            .flex_shrink_0()
                            .px_1()
                            .rounded_sm()
                            .text_xs()
                            .text_color(if pinned { theme::yellow() } else { theme::dim() })
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::border()))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                                cx.stop_propagation();
                                this.toggle_workspace_pin(i, cx);
                            }))
                            .child(if pinned { "★" } else { "☆" }),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("ws-remove-{i}")))
                            .flex_shrink_0()
                            .px_1()
                            .rounded_sm()
                            .text_xs()
                            .text_color(theme::dim())
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::border()).text_color(theme::red()))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                                cx.stop_propagation();
                                this.remove_recent_workspace(i, cx);
                            }))
                            .child("✕"),
                    );
            }
            list = list.child(row);
        }
        card = card.child(list);

        // ---- 连接远程 SSH：内联输入（回车 / 「连接」都生效）----
        if self.ws_remote_open {
            let value = self.ws_input.text().to_string();
            let pw = self.ws_pw_input.text().to_string();
            let pw_focus = self.ws_pw_focus;
            let err = self.ws_remote_err.clone();
            let placeholder = if value.is_empty() {
                "user@host:/abs/path（密钥优先）".to_string()
            } else {
                crate::views::text_fit::fit_text(&value, 320.0, 14.0)
            };
            // 密码框：失焦时只显示圆点，不把明文挂在屏幕上；空串显示占位提示。
            let pw_display = if pw.is_empty() {
                "密码（可选，留空用 ssh 密钥）".to_string()
            } else if pw_focus {
                crate::views::text_fit::fit_text(&pw, 320.0, 14.0)
            } else {
                "•".repeat(pw.chars().count().min(32))
            };
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(theme::border())
                    // 键盘只挂在这个容器上：连接串与密码共用 ws_focus，
                    // 两个 on_key_down 会同时收到同一次回车。
                    .track_focus(&self.ws_focus)
                    .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
                        this.on_workspace_key(ev, cx);
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(theme::yellow())
                                    .child("⇅"),
                            )
                            .child(
                                div()
                                    .id("ws-remote-input")
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_sm()
                                    .text_color(if value.is_empty() {
                                        theme::dim()
                                    } else {
                                        theme::fg()
                                    })
                                    .cursor(gpui::CursorStyle::IBeam)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.ws_pw_focus = false;
                                        this.switch_input_target(InputTarget::Workspace);
                                        window.focus(&this.ws_focus);
                                        cx.notify();
                                    }))
                                    .child(SharedString::from(placeholder)),
                            )
                            .child(
                                div()
                                    .id("ws-remote-connect")
                                    .px_2()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(theme::accent())
                                    .text_xs()
                                    .text_color(theme::bg())
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                        cx.stop_propagation();
                                        this.connect_remote_from_input(cx);
                                    }))
                                    .child("连接"),
                            )
                            .child(
                                div()
                                    .id("ws-remote-cancel")
                                    .px_1()
                                    .rounded_sm()
                                    .text_xs()
                                    .text_color(theme::dim())
                                    .cursor_pointer()
                                    .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                        cx.stop_propagation();
                                        this.close_remote_workspace_input(cx);
                                    }))
                                    .child("✕"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(14.0))
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(theme::dim())
                                    .child("∗"),
                            )
                            .child(
                                div()
                                    .id("ws-remote-pw")
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_sm()
                                    .text_color(if pw.is_empty() {
                                        theme::dim()
                                    } else {
                                        theme::fg()
                                    })
                                    .cursor(gpui::CursorStyle::IBeam)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.ws_pw_focus = true;
                                        this.switch_input_target(InputTarget::Workspace);
                                        window.focus(&this.ws_focus);
                                        cx.notify();
                                    }))
                                    .child(SharedString::from(pw_display)),
                            ),
                    )
                    .when_some(err, |d, e| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(theme::red())
                                .child(SharedString::from(crate::views::text_fit::fit_text(
                                    &e, 470.0, 12.0,
                                ))),
                        )
                    })
                    .child(
                        div().text_xs().text_color(theme::dim()).child(
                            "密码只保存在内存（不写配置、不进命令行）；回车即连接",
                        ),
                    ),
            );
        }

        // ---- 底部：打开新文件夹 / 手动路径 / 连接远程 + 提示 ----
        // 三个按钮 + 提示一行放不下（卡宽 520），按钮与提示分两行。
        card = card.child(
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap_1()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(theme::border())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .id("ws-open-folder")
                                .flex()
                                .items_center()
                                .gap_1()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .bg(theme::raised())
                                .border_1()
                                .border_color(theme::border())
                                .text_xs()
                                .text_color(theme::muted())
                                .cursor_pointer()
                                .hover(|d| d.bg(theme::border()).text_color(theme::fg()))
                                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                    this.pick_workspace_folder(cx);
                                }))
                                .child("＋ 打开文件夹…"),
                        )
                        .child(
                            div()
                                .id("ws-type-path")
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .text_xs()
                                .text_color(theme::dim())
                                .cursor_pointer()
                                .hover(|d| d.bg(theme::raised()).text_color(theme::muted()))
                                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                    this.open_workspace_folder_input(cx);
                                }))
                                .child("⌨ 手动输入路径…"),
                        )
                        .child(
                            div()
                                .id("ws-remote")
                                .flex()
                                .items_center()
                                .gap_1()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .border_1()
                                .border_color(theme::with_alpha(0xf59e0b, 0.35))
                                .text_xs()
                                .text_color(theme::yellow())
                                .cursor_pointer()
                                .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                    cx.stop_propagation();
                                    this.open_remote_workspace_input(cx);
                                }))
                                .child("⇅ 连接远程 SSH…"),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::dim())
                        .child("✕ 只从列表移除，不删磁盘目录；远程只用密钥 / ssh-agent"),
                ),
        );

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
