use super::*;
use gpui::{ClickEvent, CursorStyle, KeyDownEvent};
use neverwork_core::MessageKind;
use crate::state::InputTarget;
use crate::views::text_fit::fit_text;

impl AppState {
    pub(crate) fn chat_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session();
        let mut col = div().flex().flex_col().flex_1().h_full().overflow_hidden().bg(theme::surface());

        col = col.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .h(px(56.0))
                .px_5()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(SharedString::from(session.title.clone())))
                        .child(role_dots_row(&self.config)),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("open-team")
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_3()
                        .py_1p5()
                        .rounded_md()
                        .bg(theme::panel())
                        .border_1()
                        .border_color(theme::border())
                        .text_xs()
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.center = CenterView::Team;
                            cx.notify();
                        }))
                        .child(SharedString::from(format!(
                            "◈ {} · 编辑 ▾",
                            self.config.active_team().map(|t| t.name.clone()).unwrap_or_default()
                        ))),
                ),
        );

        col = col.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_5()
                .py_2()
                .bg(theme::accent_soft())
                .border_b_1()
                .border_color(theme::border())
                .child(flow_step("P", 0, "规划", true))
                .child(div().text_xs().text_color(theme::dim()).child("→"))
                .child(flow_step("C", 1, "编码", true))
                .child(div().text_xs().text_color(theme::dim()).child("→"))
                .child(flow_step("R", 2, "审查", false))
                .child(div().flex_1())
                .child(
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_md()
                        .bg(theme::accent_soft())
                        .text_xs()
                        .text_color(theme::accent())
                        .child(if self.streaming { "生成中…" } else { "步骤 2/3" }),
                ),
        );

        let mut thread = div().flex().flex_col().gap_4().px_6().py_5();
        let sidx = self.active_session;
        for i in 0..session.messages.len() {
            // 深拷贝这一条，避免同时借用 self 与 methods
            let m = session.messages[i].clone();
            let pending_here = self
                .pending
                .as_ref()
                .map(|p| p.session_idx == sidx && p.msg_idx == i)
                .unwrap_or(false);
            thread = thread.child(self.render_msg(&m, i, pending_here, cx));
        }
        if self.streaming {
            thread = thread.child(
                div()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("▍正在生成…"),
            );
        }

        col = col.child(
            div()
                .id("chat-scroll")
                .flex_1()
                .overflow_scroll()
                .track_scroll(&self.chat_scroll)
                .child(thread),
        );

        // 内容有更新时滚到底部。必须在这里（内容已装进容器之后）请求，
        // 否则滚动会作用在上一帧的布局上，表现为「停在中途」。
        if self.chat_scroll_dirty.take() {
            self.chat_scroll.scroll_to_bottom();
        }
        col = col.child(self.composer(cx));
        col
    }

    pub(crate) fn composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.chat.text().is_empty();
        let send_bg = if self.streaming { theme::dim() } else { theme::accent() };
        let send_label = if self.streaming { "停止 ■" } else { "发送 ↑" };
        let has_open_file = self.open_path.is_some();

        let mut col = div()
            .px_5()
            .pt_3()
            .pb_4()
            .border_t_1()
            .border_color(theme::border())
            .bg(theme::surface())
            .flex()
            .flex_col()
            .gap_2();

        // 有待确认的写操作时，输入框上方给出显式提示（卡片里也有按钮）。
        if let Some(p) = &self.pending {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(theme::with_alpha(0xf59e0b, 0.12))
                    .border_1()
                    .border_color(theme::with_alpha(0xf59e0b, 0.4))
                    .text_xs()
                    .text_color(theme::yellow())
                    .child("⚠")
                    .child(SharedString::from(format!(
                        "等待你确认：{} {}",
                        p.tool, p.summary
                    )))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .child("在上方卡片里点「允许并写入」或「拒绝」"),
                    ),
            );
        }

        if let Some(notice) = &self.notice {
            col = col.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(theme::green_soft())
                    .border_1()
                    .border_color(theme::green_line())
                    .text_xs()
                    .text_color(theme::yellow())
                    .child(SharedString::from(notice.clone())),
            );
        } else if !self.ai_ready {
            col = col.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(theme::accent_soft())
                    .text_xs()
                    .text_color(theme::muted())
                    .child("未检测到 AI 配置。设置环境变量 NW_API_KEY 后重启即可对话。"),
            );
        }

        // ===== 输入行：逐字符渲染，能看到光标，也能点任意位置改字 =====
        let mut input_el = div()
            .id("composer-input")
            .flex()
            .flex_row()
            .items_center()
            .flex_wrap()
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .text_sm()
            .min_h(px(22.0))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.switch_input_target(InputTarget::Chat);
                let end = this.chat.text().len();
                this.chat.move_to(end, false);
                window.focus(&this.focus_handle);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| {
                this.on_chat_key(ev, cx);
            }));

        if empty {
            input_el = input_el.child(
                div()
                    .text_color(theme::dim())
                    .child("继续指令，或让团队继续协作…（Enter 发送）"),
            );
        } else {
            let cursor = self.chat.cursor();
            let sel = self.chat.selection();
            let mut caret_drawn = false;
            let chars: Vec<(usize, char)> = self.chat.text().char_indices().collect();
            for (i, ch) in chars {
                if cursor == i {
                    input_el = input_el.child(chat_caret());
                    caret_drawn = true;
                }
                let selected = sel
                    .as_ref()
                    .map(|r| i >= r.start && i < r.end)
                    .unwrap_or(false);
                let display: SharedString = if ch == ' ' {
                    SharedString::from("\u{00A0}")
                } else {
                    SharedString::from(ch.to_string())
                };
                let cell = div()
                    .flex_shrink_0()
                    .text_color(theme::fg())
                    .when(selected, |d| d.bg(gpui::rgb(0x264f78)))
                    .child(display)
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, ev: &gpui::MouseDownEvent, window, cx| {
                            this.switch_input_target(InputTarget::Chat);
                            this.chat.move_to(i, ev.modifiers.shift);
                            window.focus(&this.focus_handle);
                            cx.notify();
                        }),
                    );
                input_el = input_el.child(cell);
            }
            if !caret_drawn {
                input_el = input_el.child(chat_caret());
            }
        }

        col = col.child(
            div()
                .rounded_lg()
                .border_1()
                .border_color(if self.streaming {
                    theme::accent_line()
                } else {
                    theme::border()
                })
                .bg(theme::panel())
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(input_el)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        // 这几个按钮之前只有样式、点了没反应，现在都是真功能。
                        .child(
                            div()
                                .id("composer-ref")
                                .px_2()
                                .py_0p5()
                                .rounded_sm()
                                .text_xs()
                                .cursor_pointer()
                                .when(has_open_file, |d| {
                                    d.text_color(theme::muted()).hover(|d| d.bg(theme::raised()))
                                })
                                .when(!has_open_file, |d| d.text_color(theme::dim()))
                                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                    this.insert_file_reference(cx);
                                }))
                                .child("@ 引用文件"),
                        )
                        .child(
                            div()
                                .id("toggle-tools")
                                .px_2()
                                .py_0p5()
                                .rounded_sm()
                                .text_xs()
                                .cursor_pointer()
                                .when(self.tools_enabled, |d| {
                                    d.bg(theme::accent_soft())
                                        .text_color(theme::accent())
                                        .hover(|d| d.opacity(0.85))
                                })
                                .when(!self.tools_enabled, |d| {
                                    d.text_color(theme::dim()).hover(|d| d.bg(theme::raised()))
                                })
                                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                    this.tools_enabled = !this.tools_enabled;
                                    this.notice = Some(if this.tools_enabled {
                                        "已开启工具：Agent 可以读文件、并在你批准后改文件".into()
                                    } else {
                                        "已关闭工具：只做纯对话".into()
                                    });
                                    cx.notify();
                                }))
                                .child(if self.tools_enabled {
                                    "⚒ 工具已开"
                                } else {
                                    "⚒ 工具已关"
                                }),
                        )
                        .child(
                            div()
                                .id("composer-clear")
                                .px_2()
                                .py_0p5()
                                .rounded_sm()
                                .text_xs()
                                .text_color(theme::muted())
                                .cursor_pointer()
                                .hover(|d| d.bg(theme::raised()))
                                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                    this.chat.reset("");
                                    this.notice = None;
                                    cx.notify();
                                }))
                                .child("清空"),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .id("send-btn")
                                .flex()
                                .items_center()
                                .gap_1()
                                .h(px(32.0))
                                .px_4()
                                .rounded_md()
                                .bg(send_bg)
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                    if this.streaming {
                                        this.stop_streaming(cx);
                                    } else {
                                        this.send_message(cx);
                                    }
                                }))
                                .child(SharedString::from(send_label)),
                        ),
                ),
        );

        col
    }

    /// 把当前打开的文件名以 @ 形式插入输入框。
    pub(crate) fn insert_file_reference(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.open_path.as_ref().and_then(|p| p.file_name()) else {
            self.notice = Some("还没有打开文件".into());
            cx.notify();
            return;
        };
        let name = name.to_string_lossy().to_string();
        self.switch_input_target(InputTarget::Chat);
        self.chat.insert(&format!("@{name} "));
        cx.notify();
    }
}

fn chat_caret() -> impl IntoElement {
    div()
        .w(px(2.0))
        .h(px(16.0))
        .flex_shrink_0()
        .bg(theme::accent())
}

pub(crate) fn flow_step(initial: &str, idx: usize, label: &str, active: bool) -> impl IntoElement {
    let color = theme::role_color(idx);
    let text = if active { theme::fg() } else { theme::dim() };
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(18.0))
                .rounded_md()
                .bg(color)
                .text_xs()
                .text_color(theme::bg())
                .child(SharedString::from(initial.to_string())),
        )
        .child(div().text_xs().text_color(text).child(SharedString::from(label.to_string())))
}

impl AppState {
    /// 渲染一条会话消息。`pending_here` 为真时给写操作卡片加上允许/拒绝按钮。
    pub(crate) fn render_msg(
        &self,
        m: &neverwork_core::Message,
        idx: usize,
        pending_here: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match &m.kind {
            MessageKind::User => user_bubble(&m.body).into_any_element(),
            MessageKind::Agent { participant } => agent_bubble(
                &participant.role_name,
                participant.initial.clone(),
                participant.color_index,
                &m.body,
                m.code.as_ref(),
            )
            .into_any_element(),
            MessageKind::Handoff { to, action, .. } => handoff(to, action).into_any_element(),
            MessageKind::Tool { summary, .. } => tool_result(summary).into_any_element(),
            MessageKind::Parallel { items } => parallel_row(items).into_any_element(),
            MessageKind::ToolCall {
                name,
                summary,
                status,
                output,
                diff,
                added,
                removed,
            } => self
                .tool_call_card(
                    name,
                    summary,
                    *status,
                    output,
                    diff,
                    *added,
                    *removed,
                    idx,
                    pending_here,
                    cx,
                )
                .into_any_element(),
        }
    }

    /// 工具调用卡片：名字 + 参数摘要 + 状态；写操作展示 diff 与审批按钮。
    #[allow(clippy::too_many_arguments)]
    fn tool_call_card(
        &self,
        name: &str,
        summary: &str,
        status: neverwork_core::ToolRunStatus,
        output: &str,
        diff: &[neverwork_core::DiffLineView],
        added: u32,
        removed: u32,
        idx: usize,
        pending_here: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use neverwork_core::ToolRunStatus;

        let (icon, color) = match name {
            "read_file" => ("◧", theme::cyan()),
            "list_dir" => ("▤", theme::cyan()),
            "search_files" => ("⌕", theme::cyan()),
            "write_file" => ("✎", theme::orange()),
            "apply_patch" => ("±", theme::orange()),
            _ => ("⚒", theme::muted()),
        };
        let (status_text, status_color) = match status {
            ToolRunStatus::Running => ("执行中…", theme::accent()),
            ToolRunStatus::Awaiting => ("等待你确认", theme::yellow()),
            ToolRunStatus::Ok => ("完成", theme::green()),
            ToolRunStatus::Denied => ("已拒绝", theme::dim()),
            ToolRunStatus::Failed => ("失败", theme::red()),
        };

        let mut card = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_3()
            .py_2()
            .rounded_lg()
            .border_1()
            .border_color(if pending_here {
                theme::with_alpha(0xf59e0b, 0.55)
            } else {
                theme::border()
            })
            .bg(theme::panel());

        // 头部：图标 + 工具名 + 摘要 + 状态
        let mut head = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(20.0))
                    .flex_shrink_0()
                    .rounded_sm()
                    .bg(theme::with_alpha(0xffffff, 0.06))
                    .text_xs()
                    .text_color(color)
                    .child(SharedString::from(icon)),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(theme::fg())
                    .child(SharedString::from(name.to_string())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme::muted())
                    .child(SharedString::from(fit_text(summary, 420.0, 12.0))),
            );

        if added > 0 || removed > 0 {
            head = head
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::green())
                        .child(SharedString::from(format!("+{added}"))),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::red())
                        .child(SharedString::from(format!("−{removed}"))),
                );
        }

        head = head.child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(status_color)
                .child(SharedString::from(status_text)),
        );

        card = card.child(head);

        // diff
        if !diff.is_empty() {
            let mut boxel = div()
                .id(SharedString::from(format!("diff-{idx}")))
                .flex()
                .flex_col()
                .max_h(px(220.0))
                .overflow_scroll()
                .rounded_md()
                .bg(theme::bg())
                .p_2();

            for line in diff.iter() {
                let (fg, bg) = match line.marker.as_str() {
                    "+" => (theme::green(), theme::with_alpha(0x22c55e, 0.10)),
                    "-" => (theme::red(), theme::with_alpha(0xef4444, 0.10)),
                    _ => (theme::dim(), theme::with_alpha(0x000000, 0.0)),
                };
                boxel = boxel.child(
                    div()
                        .flex()
                        .gap_1()
                        .px_1()
                        .rounded_sm()
                        .bg(bg)
                        .text_xs()
                        .child(
                            div()
                                .w(px(10.0))
                                .flex_shrink_0()
                                .text_color(fg)
                                .child(SharedString::from(line.marker.clone())),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_color(if line.marker == " " { theme::muted() } else { fg })
                                .child(SharedString::from(line.text.clone())),
                        ),
                );
            }
            card = card.child(boxel);
        }

        // 输出（失败信息或读文件结果摘要）
        if !output.is_empty() && diff.is_empty() {
            let first: String = output
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join("
");
            card = card.child(
                div()
                    .max_h(px(80.0))
                    .overflow_hidden()
                    .rounded_md()
                    .bg(theme::bg())
                    .p_2()
                    .text_xs()
                    .text_color(if status == ToolRunStatus::Failed {
                        theme::red()
                    } else {
                        theme::muted()
                    })
                    .child(SharedString::from(first)),
            );
        }

        // 审批按钮
        if pending_here && status == ToolRunStatus::Awaiting {
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::yellow())
                            .child("这个操作会改文件，确认后才会写入："),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id(SharedString::from(format!("deny-{idx}")))
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(theme::raised())
                            .text_xs()
                            .text_color(theme::muted())
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::border()))
                            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                                this.answer_approval(false, cx);
                            }))
                            .child("拒绝"),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("allow-{idx}")))
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(theme::green())
                            .text_xs()
                            .text_color(gpui::rgb(0xffffff))
                            .cursor_pointer()
                            .hover(|d| d.opacity(0.88))
                            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                                this.answer_approval(true, cx);
                            }))
                            .child("允许并写入"),
                    ),
            );
        }

        card
    }
}

pub(crate) fn user_bubble(text: &str) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .gap_3()
        .justify_end()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .items_end()
                .child(div().text_xs().text_color(theme::dim()).child("你"))
                .child(
                    div()
                        .max_w(px(theme::CHAT_MAX_W * 0.75))
                        .px_4()
                        .py_3()
                        .rounded_lg()
                        .bg(theme::accent_soft())
                        .border_1()
                        .border_color(theme::accent_line())
                        .text_sm()
                        .child(SharedString::from(text.to_string())),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(30.0))
                .rounded_md()
                .bg(theme::raised())
                .border_1()
                .border_color(theme::border())
                .text_xs()
                .text_color(theme::muted())
                .child("U"),
        )
}

pub(crate) fn agent_bubble(
    name: &str,
    initial: String,
    color_idx: usize,
    text: &str,
    code: Option<&neverwork_core::CodeBlock>,
) -> impl IntoElement {
    let body = if text.is_empty() { "…" } else { text };
    let mut bubble = div()
        .max_w(px(theme::CHAT_MAX_W * 0.78))
        .px_4()
        .py_3()
        .rounded_lg()
        .bg(theme::raised())
        .border_1()
        .border_color(theme::border())
        .text_sm()
        .child(SharedString::from(body.to_string()));
    if let Some(c) = code {
        bubble = bubble.child(code_block(c));
    }
    div()
        .flex()
        .flex_row()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(30.0))
                .rounded_md()
                .bg(theme::role_color(color_idx))
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(SharedString::from(initial)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).child(SharedString::from(name.to_string())))
                        .child(tag_el(role_kind(color_idx))),
                )
                .child(bubble),
        )
}

pub(crate) fn role_kind(idx: usize) -> &'static str {
    match idx {
        0 => "规划",
        1 => "AI",
        2 => "审查",
        _ => "角色",
    }
}

pub(crate) fn tag_el(text: &str) -> impl IntoElement {
    div()
        .px_1()
        .py_0p5()
        .rounded_sm()
        .bg(theme::raised())
        .border_1()
        .border_color(theme::border())
        .text_xs()
        .text_color(theme::dim())
        .child(SharedString::from(text.to_string()))
}

pub(crate) fn code_block(c: &neverwork_core::CodeBlock) -> impl IntoElement {
    div()
        .mt_2()
        .rounded_lg()
        .border_1()
        .border_color(theme::border())
        .overflow_hidden()
        .bg(theme::bg())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .bg(theme::raised())
                .border_b_1()
                .border_color(theme::border())
                .child(div().text_xs().text_color(theme::orange()).child("⬢"))
                .child(div().text_xs().text_color(theme::muted()).child(SharedString::from(c.filename.clone())))
                .child(div().flex_1())
                .child(div().text_xs().text_color(theme::green()).child(SharedString::from(format!("+{}", c.added))))
                .child(div().text_xs().text_color(theme::red()).child(SharedString::from(format!("−{}", c.removed)))),
        )
        .child(div().px_3().py_2().text_xs().text_color(theme::fg()).child(SharedString::from(c.code.clone())))
}

pub(crate) fn handoff(to: &str, action: &str) -> impl IntoElement {
    div()
        .flex()
        .justify_center()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .rounded_lg()
                .bg(theme::panel())
                .border_1()
                .border_color(theme::border())
                .text_xs()
                .text_color(theme::muted())
                .child(SharedString::from(format!("{to} {action}"))),
        )
}

pub(crate) fn tool_result(summary: &str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(theme::green_soft())
        .border_1()
        .border_color(theme::green_line())
        .text_xs()
        .text_color(theme::muted())
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(15.0))
                .rounded_sm()
                .bg(theme::green_soft())
                .text_color(theme::green())
                .child("✓"),
        )
        .child(SharedString::from(summary.to_string()))
}

pub(crate) fn parallel_row(items: &[neverwork_core::ParallelItem]) -> impl IntoElement {
    let mut row = div().flex().items_center().justify_center().gap_2();
    for it in items {
        let status = if it.done {
            div().text_color(theme::green()).child("✓")
        } else {
            div().text_color(theme::accent()).child("…")
        };
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .rounded_md()
                .bg(theme::panel())
                .border_1()
                .border_color(theme::border())
                .text_xs()
                .text_color(theme::muted())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(15.0))
                        .rounded_sm()
                        .bg(theme::role_color(it.color_index))
                        .text_color(theme::fg())
                        .child(SharedString::from(it.initial.clone())),
                )
                .child(SharedString::from(format!("{} {}", it.role_name, it.task)))
                .child(status),
        );
    }
    div().flex().justify_center().child(row)
}



pub(crate) fn role_dot(name: &str, idx: usize) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(div().size(px(6.0)).rounded_md().bg(theme::role_color(idx)))
        .child(div().text_xs().text_color(theme::muted()).child(SharedString::from(name.to_string())))
}

/// 顶栏角色圆点行（来自当前团队）。
pub(crate) fn role_dots_row(config: &neverwork_core::Config) -> impl IntoElement {
    let mut row = div().flex().items_center().gap_2();
    if let Some(team) = config.active_team() {
        for role in &team.roles {
            row = row.child(role_dot(&role.name, role.color_index));
        }
    }
    row
}

