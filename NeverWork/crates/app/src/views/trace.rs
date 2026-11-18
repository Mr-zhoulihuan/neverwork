//! 对话轨迹页：把会话消息流渲染成可检查的请求日志。
//!
//! 对齐参考稿（DeepSeek session log）：左侧是带类型标签的消息列表
//! （USER / ASSISTANT / TOOL / HANDOFF），点条目在右侧展开详情
//! （Summary / Payload / Result / Timing）。数据完全来自
//! `Session.messages`：工具调用的 `arguments` 就是 Payload、`output`
//! 就是 Result，Agent 消息的 elapsed/tokens 就是 Timing。没有记录的
//! 字段（system prompt、时间戳）不展示，不造数据。

use gpui::prelude::*;
use gpui::{div, px, ClickEvent, Context, FontWeight, IntoElement, Rgba, SharedString};
use neverwork_core::{Message, MessageKind, ToolRunStatus};
use neverwork_ui::theme;

use crate::state::{AppState, InputTarget};
use crate::views::text_fit::fit_text;

/// 详情分栏宽度。中栏 ~940px 时列表还剩 ~510px，一行摘要放得下。
const DETAIL_W: f32 = 430.0;
/// 详情里长文本（参数 JSON / 工具输出 / 回答正文）的显示上限。
const TEXT_LIMIT: usize = 6000;

/// 类型标签：`(文字, 颜色)`。配色对齐参考稿：USER 蓝 / ASSISTANT 紫 /
/// TOOL 橙 / HANDOFF 黄，老格式的工具结果用灰。
fn kind_tag(kind: &MessageKind) -> (&'static str, Rgba) {
    match kind {
        MessageKind::User => ("USER", theme::accent()),
        MessageKind::Agent { .. } => ("ASSISTANT", theme::purple()),
        MessageKind::ToolCall { .. } => ("TOOL", theme::orange()),
        MessageKind::Tool { .. } => ("RESULT", theme::dim()),
        MessageKind::Handoff { .. } => ("HANDOFF", theme::yellow()),
        MessageKind::Parallel { .. } => ("PARALLEL", theme::cyan()),
    }
}

fn tag_chip(label: &str, color: Rgba) -> impl IntoElement {
    div()
        .w(px(74.0))
        .flex_shrink_0()
        .flex()
        .justify_center()
        .px_1()
        .py_0p5()
        .rounded(px(theme::radius_sm()))
        .bg(theme::with_alpha(rgba_hex(color), 0.13))
        .text_size(px(9.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(SharedString::from(label.to_string()))
}

/// 标签底色用同一个颜色的低透明度版本，但 `theme::with_alpha` 只收
/// 24 位 hex —— 主题色是 `Rgba`，从 `0xRRGGBBAA` 里取 RGB 三通道。
fn rgba_hex(c: Rgba) -> u32 {
    let raw: u32 = c.into();
    raw >> 8
}

/// 单行摘要：取第一行非空文本，压掉连续空白，超长截断。
fn one_line(text: &str, max: usize) -> String {
    let t = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string();
    let mut out: String = t.chars().take(max).collect();
    if t.chars().count() > max {
        out.push('…');
    }
    out
}

fn fmt_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f32 / 1000.0)
    } else {
        format!("{}m{:02}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

/// 搜索时在这段文字里做包含匹配（小写化后比较）。
fn haystack(m: &Message) -> String {
    match &m.kind {
        MessageKind::User | MessageKind::Agent { .. } => m.body.clone(),
        MessageKind::ToolCall {
            name,
            summary,
            path,
            arguments,
            output,
            ..
        } => format!("{name} {summary} {path} {arguments} {output}"),
        MessageKind::Tool { summary, files } => format!("{} {}", summary, files.join(" ")),
        MessageKind::Handoff {
            from,
            to,
            action,
            detail,
        } => format!("{from} {to} {action} {detail}"),
        MessageKind::Parallel { items } => items
            .iter()
            .map(|i| format!("{} {}", i.role_name, i.task))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// 行摘要（列表里显示的那一行）。
fn row_summary(m: &Message) -> String {
    match &m.kind {
        MessageKind::User => one_line(&m.body, 120),
        MessageKind::Agent { .. } => one_line(&m.body, 120),
        MessageKind::ToolCall {
            name,
            summary,
            arguments,
            output,
            ..
        } => {
            // 参考稿：`bash {"command": "ls -F /..."` → 结果首行。
            let args = one_line(arguments, 60);
            let mut s = if args.is_empty() || args == "{}" {
                name.clone()
            } else {
                format!("{name} {args}")
            };
            let tail = one_line(output, 60);
            let tail = if tail.is_empty() {
                one_line(summary, 60)
            } else {
                tail
            };
            if !tail.is_empty() {
                s.push_str("  →  ");
                s.push_str(&tail);
            }
            s
        }
        MessageKind::Tool { summary, .. } => one_line(summary, 120),
        MessageKind::Handoff { from, to, action, .. } => {
            format!("{from} → {to}：{action}")
        }
        MessageKind::Parallel { items } => format!("并行 {} 项", items.len()),
    }
}

/// 参数 JSON 美化输出；解析失败就原样返回（模型偶尔给半截 JSON）。
fn pretty_args(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| trimmed.to_string()),
        Err(_) => trimmed.to_string(),
    }
}

/// 详情里的一段键值（Summary 节）。
fn kv_row(key: &str, value: String, value_color: Rgba) -> impl IntoElement {
    div()
        .flex()
        .items_start()
        .gap_3()
        .py_0p5()
        .child(
            div()
                .w(px(84.0))
                .flex_shrink_0()
                .text_size(px(10.0))
                .text_color(theme::dim())
                .child(SharedString::from(key.to_string())),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(px(11.0))
                .text_color(value_color)
                .child(SharedString::from(value)),
        )
}

fn section_title(title: &str) -> impl IntoElement {
    div()
        .mb_1()
        .text_size(px(10.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::muted())
        .child(SharedString::from(title.to_string()))
}

/// 文本块：等宽、可折行、按行拆 div（gpui 的 text 不吃 `\n`）。
fn text_block(text: &str) -> impl IntoElement {
    let (shown, truncated) = if text.chars().count() > TEXT_LIMIT {
        (text.chars().take(TEXT_LIMIT).collect::<String>(), true)
    } else {
        (text.to_string(), false)
    };
    let mut col = div()
        .flex()
        .flex_col()
        .gap_0p5()
        .p_2()
        .rounded(px(theme::radius_md()))
        .border_1()
        .border_color(theme::border())
        .bg(theme::surface())
        .font_family(theme::mono_family())
        .text_size(px(11.0))
        .line_height(px(17.0))
        .text_color(theme::fg());
    for line in shown.lines() {
        if line.trim().is_empty() {
            col = col.child(div().h(px(6.0)));
        } else {
            col = col.child(
                div()
                    .min_w(px(0.0))
                    .child(SharedString::from(line.to_string())),
            );
        }
    }
    if truncated {
        col = col.child(
            div()
                .pt_1()
                .text_size(px(10.0))
                .text_color(theme::dim())
                .child(SharedString::from(format!(
                    "…已截断（共 {} 字符）",
                    text.chars().count()
                ))),
        );
    }
    col
}

impl AppState {
    /// 轨迹页签的整区视图：统计行 + 列表 + 详情分栏。
    ///
    /// 打开时替换整个对话内容区（滚动区与输入框都不渲染），
    /// 标题行和会话操作保持不动。
    pub(crate) fn trace_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session();
        let (provider, model) = self.session_model_labels(self.active_session);
        let model_label = if model.trim().is_empty() { provider } else { model };
        let query = self.trace_query.text().trim().to_lowercase();

        // 汇总：累计耗时 / 模型请求数 / 工具调用数 / token。
        let mut duration = 0u64;
        let mut turns = 0usize;
        let mut calls = 0usize;
        let mut tin = 0u64;
        let mut tout = 0u64;
        for m in &session.messages {
            match m.kind {
                MessageKind::Agent { .. } => {
                    turns += 1;
                    duration += m.elapsed_ms;
                    tin += m.tokens_in;
                    tout += m.tokens_out;
                }
                MessageKind::ToolCall { .. } => calls += 1,
                _ => {}
            }
        }

        // —— 统计行 ——
        let mut head = div()
            .flex()
            .items_center()
            .gap_4()
            .h(px(40.0))
            .px_4()
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme::border());
        let stat = |icon: &str, text: String| -> gpui::AnyElement {
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(10.0))
                .text_color(theme::muted())
                .child(div().text_color(theme::dim()).child(SharedString::from(icon.to_string())))
                .child(SharedString::from(text))
                .into_any_element()
        };
        head = head
            .child(stat("◷", fmt_duration(duration)))
            .child(stat("▤", format!("{turns} 轮请求")))
            .child(stat("▢", format!("{calls} 次调用")))
            .child(stat("⇅", format!("↑{tin} ↓{tout}")))
            .child(div().flex_1());
        if self.is_running() {
            head = head.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(10.0))
                    .text_color(theme::accent())
                    .child("●")
                    .child(SharedString::from(format!("请求进行中 · {model_label}"))),
            );
        }
        head = head.child(self.trace_search_box(cx));

        // —— 列表 ——
        // turn：用户每发一条消息进入新一轮；step：该轮里第几次工具调用。
        // 没有 USER 开头的会话（团队自动跑）也归 Turn 1，不显示 Turn 0。
        let mut list = div()
            .id("trace-list")
            .flex()
            .flex_col()
            .py_1()
            .flex_1()
            .min_w(px(0.0))
            .overflow_scroll();
        let detail_open = self
            .trace_selected
            .map(|i| i < session.messages.len())
            .unwrap_or(false);
        let list_w = if detail_open {
            (self.chat_w - DETAIL_W).max(280.0)
        } else {
            self.chat_w
        };
        let summary_w = (list_w - 74.0 - 12.0 - 32.0 - 120.0).max(160.0);
        let mut turn = 0usize;
        let mut shown = 0usize;
        for (idx, m) in session.messages.iter().enumerate() {
            // 老会话的「工具结果」消息并进 ToolCall 行，不单独占一条；
            // 并行状态是模块内部细节，同样不进轨迹。
            if matches!(m.kind, MessageKind::Tool { .. } | MessageKind::Parallel { .. }) {
                continue;
            }
            if matches!(m.kind, MessageKind::User) {
                turn += 1;
            }
            if turn == 0 {
                turn = 1;
            }
            if !query.is_empty() && !haystack(m).to_lowercase().contains(&query) {
                continue;
            }
            if matches!(m.kind, MessageKind::User) {
                list = list.child(
                    div()
                        .px_4()
                        .pt_3()
                        .pb_1()
                        .text_size(px(10.0))
                        .text_color(theme::cyan())
                        .child(SharedString::from(format!("Turn {turn}"))),
                );
            }
            list = list.child(self.trace_row(idx, m, summary_w, idx == self.trace_selected.unwrap_or(usize::MAX), cx));
            shown += 1;
        }
        if shown == 0 {
            list = list.child(
                div()
                    .px_4()
                    .py_6()
                    .text_size(px(11.0))
                    .line_height(px(18.0))
                    .text_color(theme::dim())
                    .child(SharedString::from(if query.is_empty() {
                        "发出第一条消息后，这里按时间顺序列出每次模型请求与工具调用；点条目看参数与结果。"
                    } else {
                        "没有匹配的记录。"
                    })),
            );
        }

        // —— 详情分栏 ——
        let detail = self
            .trace_selected
            .and_then(|idx| session.messages.get(idx).map(|m| (idx, m)))
            .map(|(idx, m)| self.trace_detail(idx, m, turn_of(&session.messages, idx), step_of(&session.messages, idx), cx));

        let mut body = div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h(px(0.0))
            .overflow_hidden();
        body = body.child(list);
        if let Some(d) = detail {
            body = body.child(d);
        }

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .h_full()
            .overflow_hidden()
            .bg(theme::surface())
            .child(head)
            .child(body)
    }

    /// 轨迹搜索框：点一下聚焦，输入走 `EntityInputHandler`（IME 安全）。
    fn trace_search_box(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.trace_query.text().to_string();
        let placeholder = query.is_empty();
        let focused = self.input_target == InputTarget::TraceSearch;
        let mut box_ = div()
            .id("trace-search")
            .flex()
            .items_center()
            .gap_1p5()
            .w(px(220.0))
            .px_2()
            .py_1()
            .rounded(px(theme::radius_md()))
            .bg(theme::raised())
            .border_1()
            .border_color(if focused {
                theme::accent_line()
            } else {
                theme::border()
            })
            .cursor(gpui::CursorStyle::IBeam)
            .track_focus(&self.focus_handle)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.switch_input_target(InputTarget::TraceSearch);
                let end = this.trace_query.text().len();
                this.trace_query.move_to(end, false);
                window.focus(&this.focus_handle);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
                this.on_trace_search_key(ev, cx);
            }))
            .child(div().flex_shrink_0().text_size(px(theme::text_caption())).text_color(theme::dim()).child("⌕"))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(11.0))
                    .text_color(if placeholder { theme::dim() } else { theme::fg() })
                    .child(SharedString::from(if placeholder {
                        "搜索轨迹…".to_string()
                    } else {
                        fit_text(&query, 170.0, 11.0)
                    })),
            );
        if !placeholder {
            box_ = box_.child(
                div()
                    .id("trace-search-clear")
                    .flex_shrink_0()
                    .px_1()
                    .rounded(px(theme::radius_sm()))
                    .text_size(px(theme::text_caption()))
                    .text_color(theme::dim())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::border()).text_color(theme::fg()))
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        cx.stop_propagation();
                        this.trace_query.reset("");
                        this.trace_selected = None;
                        cx.notify();
                    }))
                    .child("✕"),
            );
        }
        box_
    }

    /// 列表里的一行：`[类型标签] 摘要 … 右侧状态`。
    fn trace_row(
        &self,
        idx: usize,
        m: &Message,
        summary_w: f32,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (label, color) = kind_tag(&m.kind);
        let summary = fit_text(&row_summary(m), summary_w, 11.0);
        // 右侧 meta：工具显示状态（色），回答显示耗时与 token。
        let (meta, meta_color) = match &m.kind {
            MessageKind::ToolCall { status, .. } => {
                let c = match status {
                    ToolRunStatus::Failed => theme::red(),
                    ToolRunStatus::Denied | ToolRunStatus::Cancelled | ToolRunStatus::Awaiting => {
                        theme::yellow()
                    }
                    ToolRunStatus::Running => theme::accent(),
                    ToolRunStatus::Ok => theme::dim(),
                };
                (status.label().to_string(), c)
            }
            MessageKind::Agent { .. } => (
                format!("{:.1}s · ↑{} ↓{}", m.elapsed_ms as f32 / 1000.0, m.tokens_in, m.tokens_out),
                theme::dim(),
            ),
            _ => (String::new(), theme::dim()),
        };
        div()
            .id(SharedString::from(format!("trace-row-{idx}")))
            .flex()
            .items_center()
            .gap_3()
            .h(px(30.0))
            .px_4()
            .cursor_pointer()
            .when(selected, |d| d.bg(theme::accent_soft()))
            .hover(|d| d.bg(theme::raised()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                this.trace_selected = if this.trace_selected == Some(idx) {
                    None
                } else {
                    Some(idx)
                };
                cx.notify();
            }))
            .child(tag_chip(label, color))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(11.0))
                    .text_color(theme::fg())
                    .child(SharedString::from(summary)),
            )
            .when(!meta.is_empty(), |d| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(10.0))
                        .text_color(meta_color)
                        .child(SharedString::from(meta)),
                )
            })
    }

    /// 详情分栏：Summary / Payload / Result / Timing，只显示有记录的字段。
    fn trace_detail(
        &self,
        idx: usize,
        m: &Message,
        turn: usize,
        step: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (label, color) = kind_tag(&m.kind);
        let head = div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_3()
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme::border())
            .child(tag_chip(label, color))
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(theme::muted())
                    .child(SharedString::from(if matches!(m.kind, MessageKind::ToolCall { .. }) {
                        format!("Turn {turn} · Step {step}")
                    } else {
                        format!("Turn {turn}")
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("trace-detail-close")
                    .px_1()
                    .rounded(px(theme::radius_sm()))
                    .text_size(px(theme::text_caption()))
                    .text_color(theme::dim())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::raised()).text_color(theme::accent()))
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.trace_selected = None;
                        cx.notify();
                    }))
                    .child("✕"),
            );

        let mut body = div()
            .id("trace-detail-body")
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .flex_1()
            .min_h(px(0.0))
            .overflow_scroll();

        // —— Summary ——
        let mut summary = div().flex().flex_col();
        match &m.kind {
            MessageKind::User => {
                summary = summary
                    .child(kv_row("类型", "用户输入".into(), theme::fg()))
                    .child(kv_row(
                        "附件",
                        if m.attachments.is_empty() {
                            "无".into()
                        } else {
                            m.attachments
                                .iter()
                                .map(|a| a.name.clone())
                                .collect::<Vec<_>>()
                                .join("、")
                        },
                        theme::muted(),
                    ));
            }
            MessageKind::Agent { participant } => {
                let role = if participant.role_name.trim().is_empty() {
                    "助手".to_string()
                } else {
                    participant.role_name.clone()
                };
                summary = summary
                    .child(kv_row("类型", "助手回答（一次模型请求）".into(), theme::fg()))
                    .child(kv_row("角色", role, theme::muted()))
                    .child(kv_row("状态", "已完成".into(), theme::muted()));
            }
            MessageKind::ToolCall {
                name,
                status,
                thread,
                round,
                call_id,
                ..
            } => {
                let mut st_color = theme::muted();
                if matches!(status, ToolRunStatus::Failed) {
                    st_color = theme::red();
                }
                summary = summary
                    .child(kv_row("类型", "工具调用".into(), theme::fg()))
                    .child(kv_row("工具", name.clone(), theme::muted()))
                    .child(kv_row("状态", status.label().to_string(), st_color));
                let mut scope: Vec<String> = Vec::new();
                if let Some(t) = thread {
                    scope.push(format!("线程 {}", t + 1));
                }
                if let Some(r) = round {
                    scope.push(format!("第 {} 轮模型调用", r + 1));
                }
                if !scope.is_empty() {
                    summary = summary.child(kv_row("归属", scope.join(" · "), theme::muted()));
                }
                if !call_id.is_empty() {
                    summary = summary.child(kv_row("调用 ID", call_id.clone(), theme::dim()));
                }
            }
            MessageKind::Handoff { from, to, .. } => {
                summary = summary
                    .child(kv_row("类型", "角色转交".into(), theme::fg()))
                    .child(kv_row("发起", from.clone(), theme::muted()))
                    .child(kv_row("接手", to.clone(), theme::muted()));
            }
            MessageKind::Tool { summary: s, files } => {
                summary = summary.child(kv_row("类型", "工具结果".into(), theme::fg()));
                if !files.is_empty() {
                    summary = summary.child(kv_row("文件", files.join("、"), theme::muted()));
                }
                let _ = s;
            }
            MessageKind::Parallel { items } => {
                summary = summary.child(kv_row("类型", "并行执行".into(), theme::fg()));
                for it in items {
                    summary = summary.child(kv_row(
                        if it.done { "已完成" } else { "子任务" },
                        format!("{}：{}", it.role_name, it.task),
                        theme::muted(),
                    ));
                }
            }
        }
        body = body.child(div().flex().flex_col().child(section_title("Summary")).child(summary));

        // —— Payload：工具参数 / 用户原文 / 交接载荷 ——
        let payload = match &m.kind {
            MessageKind::ToolCall { arguments, .. } => pretty_args(arguments),
            MessageKind::User => m.body.clone(),
            MessageKind::Handoff { detail, action, .. } => {
                if detail.trim().is_empty() {
                    action.clone()
                } else {
                    detail.clone()
                }
            }
            _ => String::new(),
        };
        if !payload.trim().is_empty() {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .child(section_title("Payload"))
                    .child(text_block(&payload)),
            );
        }

        // —— Result：工具输出 / 回答正文 ——
        let result = match &m.kind {
            MessageKind::ToolCall {
                output,
                diff,
                added,
                removed,
                status,
                ..
            } => {
                let mut t = output.clone();
                if !diff.is_empty() || *added > 0 || *removed > 0 {
                    t.push_str(&format!("\n\n变更：+{added} −{removed} 行"));
                }
                if t.trim().is_empty() {
                    if matches!(status, ToolRunStatus::Running) {
                        "（仍在执行）".to_string()
                    } else {
                        String::new()
                    }
                } else {
                    t
                }
            }
            MessageKind::Agent { .. } => m.body.clone(),
            MessageKind::Tool { summary, .. } => summary.clone(),
            _ => String::new(),
        };
        if !result.trim().is_empty() {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .child(section_title("Result"))
                    .child(text_block(&result)),
            );
        }

        // —— Timing：Agent 这轮的真实耗时与用量（工具调用不单独计时）——
        if let MessageKind::Agent { .. } = m.kind {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .child(section_title("Timing"))
                    .child(kv_row("Duration", fmt_duration(m.elapsed_ms), theme::fg()))
                    .child(kv_row(
                        "Tokens",
                        format!("↑{} ↓{}", m.tokens_in, m.tokens_out),
                        theme::fg(),
                    )),
            );
        }

        div()
            .id(SharedString::from(format!("trace-detail-{idx}")))
            .w(px(DETAIL_W))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .border_l_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .child(head)
            .child(body)
    }
}

/// 某条消息属于第几轮（Turn）：数它之前（含自身）的 USER 消息条数，
/// 没有 USER 开头的会话归 Turn 1。
fn turn_of(messages: &[Message], idx: usize) -> usize {
    let n = messages
        .iter()
        .take(idx + 1)
        .filter(|m| matches!(m.kind, MessageKind::User))
        .count();
    n.max(1)
}

/// 某条消息在该轮里是第几步：数它之前（含自身）、上一个 USER 之后的
/// ToolCall 条数；非工具条目返回 0。
fn step_of(messages: &[Message], idx: usize) -> usize {
    let start = messages
        .iter()
        .take(idx + 1)
        .rposition(|m| matches!(m.kind, MessageKind::User))
        .map(|i| i + 1)
        .unwrap_or(0);
    messages[start..=idx]
        .iter()
        .filter(|m| matches!(m.kind, MessageKind::ToolCall { .. }))
        .count()
}
