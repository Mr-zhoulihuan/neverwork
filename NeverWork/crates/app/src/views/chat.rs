use super::*;
use gpui::{ClickEvent, CursorStyle, KeyDownEvent};
use neverwork_core::MessageKind;
use crate::state::InputTarget;

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
                        // 团队模式才显示角色点；单助手模式不放任何说明文字。
                        // 「能问答也能读文件」这种事说了也没用：用户直接问就行，
                        // 挂在标题下面只会一直占着一行。
                        .when(self.team_mode, |d| d.child(role_dots_row(&self.config))),
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
                            // 单击直接切模式：这是最常用的动作。
                            // 编辑团队角色走侧栏「团队」或命令面板。
                            this.toggle_agent_mode(cx);
                        }))
                        .child(if self.team_mode {
                            SharedString::from(format!(
                                "◈ {} · 团队 ▾",
                                self.config.active_team().map(|t| t.name.clone()).unwrap_or_default()
                            ))
                        } else {
                            SharedString::from("◎ 单助手 ▾")
                        }),
                ),
        );

        // 顶部状态条：团队模式显示流水线，单助手模式默认整条不出现。
        // 之前单助手也会挂一条「需要时会读文件，改文件前会问你」，
        // 属于解释了也不影响操作的废话提示，还白占一行高度。
        // 只有正在生成时才给一条，让用户知道它在干活、能停止。
        if self.team_mode {
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
                            .child(if self.streaming { "生成中…" } else { "团队流水线" }),
                    ),
            );
        } else if self.streaming {
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
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::accent())
                            .child("生成中…"),
                    ),
            );
        }

        // 整个对话流用等宽字体：参考稿（opencode）是终端风格，
        // 等宽让工具行、代码块、正文的行首对齐在同一条竖线上。
        let mut thread = div()
            .flex()
            .flex_col()
            .gap_4()
            .px_5()
            .py_5()
            .font_family(theme::mono_family());
        let sidx = self.active_session;
        let total = session.messages.len();
        let mut i = 0usize;
        while i < total {
            // 连续的工具调用聚合成一行，展开才看细节（对齐参考稿的步骤条）
            if matches!(session.messages[i].kind, MessageKind::ToolCall { .. }) {
                let mut j = i + 1;
                while j < total
                    && matches!(session.messages[j].kind, MessageKind::ToolCall { .. })
                {
                    j += 1;
                }
                let pending_here = self
                    .pending
                    .as_ref()
                    .map(|p| p.session_idx == sidx && p.msg_idx >= i && p.msg_idx < j)
                    .unwrap_or(false);
                thread = thread.child(self.tool_steps_group(i, j, pending_here, cx));
                i = j;
                continue;
            }
            let m = session.messages[i].clone();
            thread = thread.child(self.render_msg(&m));
            i += 1;
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
        let has_open_file = self.open_path.is_some();

        let mut col = div()
            .px_6()
            .pt_4()
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
        // 输入框真实位置：IME 候选框要贴住这里，而不是窗口左上角。
        // 之前 GPUI 拿到的 bounds 来自那个 1x1 的输入桥，候选框永远飘在角落。
        let area_slot = self.input_area_bounds.clone();
        let mut input_el = div()
            .id("composer-input")
            .flex()
            .flex_row()
            // 单行内容在加高后的框里垂直居中；多行时从顶端自然排开
            .items_center()
            .flex_wrap()
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            // 正文用 text_base：原来 text_sm 偏小，长句在宽窗口里很难读
            .text_base()
            .line_height(px(24.0))
            .min_h(px(48.0))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.switch_input_target(InputTarget::Chat);
                let end = this.chat.text().len();
                this.chat.move_to(end, false);
                window.focus(&this.focus_handle);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| {
                this.on_chat_key(ev, cx);
            }))
            // 1x1 画布只为记录这块区域的真实坐标（渲染期拿 bounds 的唯一可靠时机）。
            .child(
                gpui::canvas(
                    |_b, _w, _cx| {},
                    move |bounds, _st, _w, _cx| {
                        area_slot.set(Some(bounds));
                    },
                )
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .size(px(1.0)),
            );

        if empty {
            // 空输入框也要画光标：否则默认状态只有一个占位提示，
            // 看不出「这里已经可以打字了」，用户得先点一下才敢输入。
            // 光标画在占位文字左侧，符合「插入点在文本起始处」的直觉。
            input_el = input_el
                .child(chat_caret())
                .child(
                    div()
                        .text_color(theme::dim())
                        .child("在这里提问，或下达要执行的任务…"),
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
                .px_4()
                .pt_3()
                .pb_3()
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
                        // 发送走 Enter，不再占一个按钮。
                        // 只有生成中才出现「停止」，否则没有任何中断入口。
                        .when(self.streaming, |d| {
                            d.child(
                                div()
                                    .id("stop-btn")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .h(px(28.0))
                                    .px_3()
                                    .rounded_md()
                                    .bg(theme::raised())
                                    .text_xs()
                                    .text_color(theme::yellow())
                                    .cursor_pointer()
                                    .hover(|d| d.opacity(0.85))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                        this.stop_streaming(cx);
                                    }))
                                    .child("■ 停止"),
                            )
                        })
                        // 生成中已经有「停止」按钮和顶部状态，这里不再重复提示
                        .when(!self.streaming, |d| {
                            d.child(
                                div()
                                    .text_xs()
                                    .text_color(theme::dim())
                                    .child("Enter 发送 · Shift+Enter 换行"),
                            )
                        }),
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
        .h(px(20.0))
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
    pub(crate) fn render_msg(&self, m: &neverwork_core::Message) -> gpui::AnyElement {
        match &m.kind {
            MessageKind::User => user_bubble(&m.body).into_any_element(),
            MessageKind::Agent { participant } => {
                let (model, provider) = self.model_labels();
                agent_bubble(
                    &participant.role_name,
                    &participant.role_id,
                    participant.initial.clone(),
                    participant.color_index,
                    &m.body,
                    m.code.as_ref(),
                    m.elapsed_ms,
                    &model,
                    &provider,
                )
                .into_any_element()
            }
            MessageKind::Handoff { to, action, .. } => handoff(to, action).into_any_element(),
            MessageKind::Tool { summary, .. } => tool_result(summary).into_any_element(),
            MessageKind::Parallel { items } => parallel_row(items).into_any_element(),
            // 工具调用统一由 `tool_steps_group` 聚合渲染。
            MessageKind::ToolCall { .. } => div().into_any_element(),
        }
    }

    /// 一组连续的工具调用：每步一行 `→ Read <path>`，点击展开输出 / diff。
    ///
    /// 参考稿（opencode）就是这种形态：工具调用不是一张张卡片，
    /// 而是列在回答上方的一行行灰字；点开才看细节。
    /// 待审批的步骤永远展开，否则用户看不到按钮。
    fn tool_steps_group(
        &self,
        start: usize,
        end: usize,
        pending_here: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        use neverwork_core::ToolRunStatus;

        let sidx = self.active_session;
        let session = match self.sessions.get(sidx) {
            Some(s) => s,
            None => return div().into_any_element(),
        };

        struct Step {
            idx: usize,
            name: String,
            path: String,
            status: ToolRunStatus,
            output: String,
            diff: Vec<neverwork_core::DiffLineView>,
            added: u32,
            removed: u32,
        }

        let mut steps: Vec<Step> = Vec::new();
        for i in start..end.min(session.messages.len()) {
            if let MessageKind::ToolCall {
                name,
                path,
                status,
                output,
                diff,
                added,
                removed,
                ..
            } = &session.messages[i].kind
            {
                steps.push(Step {
                    idx: i,
                    name: name.clone(),
                    path: path.clone(),
                    status: *status,
                    output: output.clone(),
                    diff: diff.clone(),
                    added: *added,
                    removed: *removed,
                });
            }
        }
        if steps.is_empty() {
            return div().into_any_element();
        }

        // 先把步骤搬进局部 Vec，避免借用生命周期问题
        let mut col = div().flex().flex_col().gap_0p5();
        for step in steps.iter() {
            // 复制出闭包要用的下标：`move` 闭包不能捕获 `step` 的借用
            let step_idx = step.idx;
            let awaiting = step.status == ToolRunStatus::Awaiting;
            let expanded =
                awaiting || self.expand_steps_all || self.expanded_steps.contains(&step.idx);

            // 参数：优先显示真实路径（绝对路径，同参考稿），为空时只留动词
            let arg = if step.path.is_empty() {
                String::new()
            } else {
                self.root.join(&step.path).display().to_string()
            };
            let verb = tool_verb(&step.name);

            let mark_color = match step.status {
                ToolRunStatus::Running => theme::accent(),
                ToolRunStatus::Awaiting => theme::yellow(),
                ToolRunStatus::Failed => theme::red(),
                _ => theme::dim(),
            };

            let mut head = div()
                .id(SharedString::from(format!("step-{}", step.idx)))
                .flex()
                .items_center()
                .gap_1()
                .py_0p5()
                .cursor_pointer()
                .rounded_sm()
                .hover(|d| d.bg(theme::with_alpha(0xffffff, 0.035)))
                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                    this.toggle_steps(step_idx, cx);
                }))
                .child(div().flex_shrink_0().text_color(mark_color).child("→"))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(if step.status == ToolRunStatus::Failed {
                            theme::red()
                        } else {
                            theme::muted()
                        })
                        .child(SharedString::from(verb)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_color(theme::dim())
                        .child(SharedString::from(arg)),
                );

            if step.added > 0 || step.removed > 0 {
                head = head
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(theme::green())
                            .child(SharedString::from(format!("+{}", step.added))),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(theme::red())
                            .child(SharedString::from(format!("−{}", step.removed))),
                    );
            }
            // 只有异常状态才加状态词，正常完成不加噪音
            match step.status {
                ToolRunStatus::Running => {
                    head = head.child(div().flex_shrink_0().text_color(theme::accent()).child("…"));
                }
                ToolRunStatus::Awaiting => {
                    head = head.child(div().flex_shrink_0().text_color(theme::yellow()).child("待确认"));
                }
                ToolRunStatus::Failed => {
                    head = head.child(div().flex_shrink_0().text_color(theme::red()).child("失败"));
                }
                ToolRunStatus::Denied | ToolRunStatus::Cancelled => {
                    head = head.child(div().flex_shrink_0().text_color(theme::dim()).child("中断"));
                }
                ToolRunStatus::Ok => {}
            }

            let mut item = div().flex().flex_col().gap_1().child(head);

            if expanded {
                if !step.diff.is_empty() {
                    item = item.child(self.diff_box(step_idx, &step.diff));
                } else if !step.output.is_empty() {
                    // 输出直接铺在正文下方（同参考稿的命令输出块），限高防刷屏
                    item = item.child(
                        div()
                            .max_h(px(220.0))
                            .overflow_hidden()
                            .pl(px(14.0))
                            .py_1()
                            .text_color(if step.status == ToolRunStatus::Failed {
                                theme::red()
                            } else {
                                theme::dim()
                            })
                            .child(SharedString::from(step.output.clone())),
                    );
                }

                let is_pending_step = self
                    .pending
                    .as_ref()
                    .map(|p| p.session_idx == sidx && p.msg_idx == step.idx)
                    .unwrap_or(false);
                if is_pending_step && awaiting {
                    item = item.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .pl(px(14.0))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::yellow())
                                    .child("会改文件，确认后写入："),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .id(SharedString::from(format!("deny-{}", step.idx)))
                                    .px_2()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(theme::raised())
                                    .text_color(theme::muted())
                                    .cursor_pointer()
                                    .hover(|d| d.bg(theme::border()))
                                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                                        this.answer_approval(false, cx);
                                    }))
                                    .child("拒绝"),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("allow-{}", step.idx)))
                                    .px_2()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(theme::green())
                                    .text_color(gpui::rgb(0x000000))
                                    .cursor_pointer()
                                    .hover(|d| d.opacity(0.88))
                                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                                        this.answer_approval(true, cx);
                                    }))
                                    .child("允许并写入"),
                            ),
                    );
                }
            }

            col = col.child(item);
        }

        // 待确认时给整组一圈提示色，避免混在灰字里看不见
        if pending_here {
            return div()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(theme::with_alpha(0xf59e0b, 0.45))
                .child(col)
                .into_any_element();
        }
        col.into_any_element()
    }

    /// diff 展示块（`+` 绿 / `-` 红 / 上下文灰）。
    fn diff_box(&self, key: usize, diff: &[neverwork_core::DiffLineView]) -> impl IntoElement {
        let mut boxel = div()
            .id(SharedString::from(format!("diff-{key}")))
            .flex()
            .flex_col()
            .max_h(px(200.0))
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
                            .text_color(if line.marker == " " {
                                theme::muted()
                            } else {
                                fg
                            })
                            .child(SharedString::from(line.text.clone())),
                    ),
            );
        }
        boxel
    }
}

/// 动作词：对齐参考稿的 `→ Read <path>` 写法。
///
/// 用动词原形而不是「读取了文件」这种叙述句：一行一条工具调用时，
/// 叙述句会把整列挤满，也看不出动作类型。
fn tool_verb(name: &str) -> &'static str {
    match name {
        "read_file" => "Read",
        "list_dir" => "List",
        "search_files" => "Search",
        "write_file" => "Write",
        "apply_patch" => "Patch",
        "multi_patch" => "Multi-edit",
        _ => "Run",
    }
}

/// 用户发言：整块浅底 + 左侧强调条，等宽字体，不画气泡和头像。
///
/// 对齐参考稿（opencode）：用户输入是一段被引用的文本块，
/// 左右留白对齐正文，一眼能分辨「谁说的」但不抢视觉。
pub(crate) fn user_bubble(text: &str) -> impl IntoElement {
    div()
        .w_full()
        .pl_3()
        .pr_3()
        .py_2()
        .border_l_2()
        .border_color(theme::accent())
        .bg(theme::with_alpha(0xffffff, 0.035))
        .child(
            div()
                .text_color(theme::fg())
                .line_height(px(22.0))
                .child(SharedString::from(text.to_string())),
        )
}

/// AI 回答：没有头像、没有气泡框，就是一段文档流 + 一行页脚。
///
/// 参考稿（opencode）里回答本身完全平铺，`▣ Build · 模型 · 厂商 · 4.2s`
/// 这条页脚承担「这是谁说的、花了多久」的信息，比头像更省地方。
pub(crate) fn agent_bubble(
    name: &str,
    role_id: &str,
    initial: String,
    color_idx: usize,
    text: &str,
    code: Option<&neverwork_core::CodeBlock>,
    elapsed_ms: u64,
    model_label: &str,
    provider_label: &str,
) -> impl IntoElement {
    let mut col = div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .text_color(theme::fg());

    if text.trim().is_empty() && code.is_none() {
        col = col.child(div().text_color(theme::dim()).child("…"));
    } else {
        for b in crate::markdown::parse(text) {
            col = col.child(markdown_block(b));
        }
    }
    if let Some(c) = code {
        col = col.child(code_block(c));
    }

    // 页脚：多角色时带上角色名，单助手时只写模型与厂商。
    let who = if role_id == "agent" {
        String::new()
    } else if initial.is_empty() {
        name.to_string()
    } else {
        format!("{name}")
    };
    let mut parts: Vec<String> = Vec::new();
    if !who.is_empty() {
        parts.push(who);
    }
    if !model_label.is_empty() {
        parts.push(model_label.to_string());
    }
    if !provider_label.is_empty() {
        parts.push(provider_label.to_string());
    }
    if elapsed_ms > 0 {
        parts.push(format_elapsed(elapsed_ms));
    }
    // 角色色仍用来区分多角色场景（单助手时是灰的）
    let foot_color = if role_id == "agent" {
        theme::dim()
    } else {
        theme::with_alpha(0xffffff, 0.45)
    };
    col = col.child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .pt_1()
            .child(
                div()
                    .text_xs()
                    .text_color(theme::role_color(color_idx))
                    .child("▣"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(foot_color)
                    .child(SharedString::from(parts.join(" · "))),
            ),
    );
    col
}

/// 毫秒 → `272ms` / `4.2s` / `1m 03s`。
pub(crate) fn format_elapsed(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let total = ms / 1000;
        format!("{}m {:02}s", total / 60, total % 60)
    }
}

/// 渲染一个 Markdown 块。
fn markdown_block(block: crate::markdown::Block) -> gpui::AnyElement {
    use crate::markdown::Block;
    match block {
        Block::Heading(level, text) => {
            // 标题按等级递减字号，并配一条细分隔线，让长回答有清晰层次
            let size = match level {
                1 => 18.0,
                2 => 16.0,
                _ => 14.5,
            };
            div()
                .flex()
                .flex_col()
                .gap_1()
                .pt_1()
                .border_1()
                .border_color(gpui::rgb(0xff0000))
                .child(
                    div()
                        .text_size(px(size))
                        .font_weight(FontWeight::BOLD)
                        .border_1()
                        .border_color(gpui::rgb(0x0000ff))
                        // 参考稿里小标题是琥珀色，比正文更亮一档
                        .text_color(theme::yellow())
                        .child(inline_row(&text)),
                )
                .when(level <= 2, |d| d.child(div().h(px(1.0)).bg(theme::border())))
                .into_any_element()
        }
        Block::Paragraph(text) => div()
            .text_color(theme::fg())
            .line_height(px(22.0))
            .border_1()
            .border_color(gpui::rgb(0x00ff00))
            .child(inline_row(&text))
            .into_any_element(),
        Block::Code { lang, lines } => code_fence_block(&lang, &lines).into_any_element(),
        Block::Bullet(items) => {
            let mut col = div().flex().flex_col().gap_1();
            for item in items {
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .items_start()
                        .child(div().text_color(theme::dim()).child("•"))
                        .child(div().flex_1().line_height(px(22.0)).child(inline_row(&item))),
                );
            }
            col.into_any_element()
        }
        Block::Numbered(start, items) => {
            let mut col = div().flex().flex_col().gap_1();
            for (i, item) in items.into_iter().enumerate() {
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .items_start()
                        .child(
                            div()
                                .text_color(theme::dim())
                                .child(SharedString::from(format!("{}.", start + i as u32))),
                        )
                        .child(div().flex_1().line_height(px(22.0)).child(inline_row(&item))),
                );
            }
            col.into_any_element()
        }
        Block::Quote(items) => {
            let mut col = div().flex().flex_col().gap_1();
            for line in items {
                col = col.child(div().line_height(px(22.0)).child(inline_row(&line)));
            }
            div()
                .pl_3()
                .border_l_2()
                .border_color(theme::accent_line())
                .text_color(theme::muted())
                .child(col)
                .into_any_element()
        }
        Block::Rule => div().h(px(1.0)).bg(theme::border()).into_any_element(),
    }
}

/// 行内样式（`**粗体**` / `` `代码` ``）渲染。
///
/// 折行自己做（`markdown::wrap_spans`）而不是交给 flex `flex_wrap`：
/// 气泡宽度按内容撑开，flex 拿不到确定宽度，长片段会顶出气泡边界；
/// 即使给上限，flex 也只在片段之间断行，长段落会挤成「首行只剩一个词」。
fn inline_row(text: &str) -> gpui::AnyElement {
    use crate::markdown::InlineKind;
    // 气泡可用宽度：正文已改为平铺，直接用上限本身
    let avail = theme::CHAT_MAX_W - 8.0;
    let spans = crate::markdown::inline_spans(text);
    // 字号按比实际略大取值：估宽偏保守，宁可少放几个字也不要溢出气泡。
    let lines = crate::markdown::wrap_spans(&spans, avail, 14.0);

    let mut col = div()
        .flex()
        .flex_col()
        .gap_0p5()
        .max_w(px(avail))
        .border_1()
        .border_color(gpui::rgb(0xffff00));
    for line in lines {
        let mut row = div().flex().flex_row().items_baseline().max_w(px(avail)).min_w(px(0.0));
        for span in line {
            let el = match span.kind {
                InlineKind::Plain => div().child(SharedString::from(span.text)),
                InlineKind::Bold => div()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme::fg())
                    .child(SharedString::from(span.text)),
                InlineKind::Code => div()
                    .text_color(theme::green())
                    .child(SharedString::from(span.text)),
                InlineKind::Italic => div()
                    .italic()
                    .text_color(theme::muted())
                    .child(SharedString::from(span.text)),
            };
            // 不设 `flex_shrink_0`：万一估宽偏差让整行超宽，
            // flex 还能把它压回可用宽度重新折行，而不是硬顶到气泡外面。
            row = row.child(el.min_w(px(0.0)));
        }
        col = col.child(row);
    }
    col.into_any_element()
}

/// 代码围栏：带语言标签的顶栏 + 语法高亮内容。
fn code_fence_block(lang: &str, lines: &[String]) -> impl IntoElement {
    let lang = crate::highlight::lang_from_tag(lang);
    let mut body = div().flex().flex_col();
    for line in lines {
        // 空行也要占位，否则代码块的空行会被压掉
        if line.trim().is_empty() {
            body = body.child(div().h(px(18.0)));
            continue;
        }
        let mut row = div().flex().flex_row().flex_wrap();
        for span in crate::highlight::highlight_line(line, lang) {
            row = row.child(
                div()
                    .text_color(crate::highlight::color_for(span.tok))
                    .child(SharedString::from(span.text)),
            );
        }
        body = body.child(row);
    }
    div()
        .rounded_md()
        .border_1()
        .border_color(theme::border())
        .overflow_hidden()
        .bg(theme::bg())
        .child(
            div()
                .px_3()
                .py_2()
                .text_xs()
                .font_family(theme::mono_family())
                .child(body),
        )
}

impl AppState {
    /// 当前模型 / 厂商的展示名（回答页脚用）。
    pub(crate) fn model_labels(&self) -> (String, String) {
        let provider = self
            .active_provider_config()
            .map(|(id, _)| {
                self.settings
                    .providers
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.name.clone())
                    .unwrap_or(id)
            })
            .unwrap_or_default();
        let model = self
            .active_provider_config()
            .map(|(id, _)| {
                self.settings
                    .providers
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.model.clone())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        (model, provider)
    }
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

