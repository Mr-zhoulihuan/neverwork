import io

p = 'crates/app/src/views/chat.rs'
s = io.open(p, encoding='utf-8', newline='').read().replace('\r\n', '\n')

start = s.index('    /// 一组连续的工具调用：折叠成一行，展开看每一步的细节。')
end = s.index('    /// diff 展示块（`+` 绿 / `-` 红 / 上下文灰）。')

new_fn = r'''    /// 一组连续的工具调用：每步一行 `→ Read <path>`，点击展开输出 / diff。
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

        let mut col = div().flex().flex_col().gap_0p5();
        for step in &steps {
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
                    this.toggle_steps(step.idx, cx);
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
                    item = item.child(self.diff_box(step.idx, &step.diff));
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
                                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
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
                                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
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

'''

s = s[:start] + new_fn + s[end:]
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('tool lines ok')
