//! 「用量」弹框：居中卡片 + 遮罩，展示账户余额与 token 用量。
//!
//! 从对话头部「⇅ 用量」打开；✕ / Esc 关闭。卡片不关闭时也可以刷新余额，
//! 结果与「当前会话实际使用的厂商」绑定（切厂商后旧结果不再展示）。

use gpui::prelude::*;
use gpui::{div, px, Context, FontWeight, SharedString};

use neverwork_ui::theme;

use crate::state::AppState;
use crate::usage_ops::{currency_symbol, BalanceOutcome, ModelUsage};

impl AppState {
    /// 弹框整层：半透明遮罩 + 居中卡片（同 new_session 浮层的做法）。
    pub(crate) fn usage_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (provider, model) = self.session_model_labels(self.active_session);
        let model_label = if model.trim().is_empty() {
            provider.clone()
        } else {
            format!("{provider} · {model}")
        };
        let totals = self.usage_totals(self.active_session);
        let rows = self.usage_by_model();

        let mut card = div()
            .flex()
            .flex_col()
            .w(px(640.0))
            .max_h(px(560.0))
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::surface())
            .overflow_hidden();

        // ---- 头部 ----
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .py_3()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::fg())
                                .child("用量"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::dim())
                                .child(SharedString::from(format!("当前模型 {model_label}"))),
                        ),
                )
                .child(
                    div()
                        .id("usage-close")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(24.0))
                        .rounded_md()
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                            this.close_usage(cx);
                        }))
                        .child("✕"),
                ),
        );

        // ---- 内容（可滚动）----
        let mut body = div()
            .id("usage-body")
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .flex_1()
            .min_h(px(0.0))
            .overflow_scroll();
        body = body.child(self.usage_balance_card(cx));
        body = body.child(session_card(&totals));
        body = body.child(models_card(&rows));
        card = card.child(body);

        // ---- 遮罩 ----
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

    /// 余额卡片：查询中 / 余额 / 不支持 / 失败四种形态。
    fn usage_balance_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let provider = self.provider_for_session(self.active_session);
        let name = provider.map(|p| p.name.clone()).unwrap_or_default();
        let preset = provider.map(|p| p.preset);
        let busy = self.usage_balance_busy;

        let mut head = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::fg())
                    .child("账户余额"),
            )
            .child(div().flex_1());
        if !name.is_empty() {
            head = head.child(
                div()
                    .text_xs()
                    .text_color(theme::dim())
                    .child(SharedString::from(name.clone())),
            );
        }
        head = head.child(
            div()
                .id("usage-balance-refresh")
                .px_2()
                .py_1()
                .rounded_md()
                .text_xs()
                .text_color(if busy { theme::dim() } else { theme::accent() })
                .cursor_pointer()
                .hover(|d| d.bg(theme::border()))
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                    this.refresh_usage_balance(cx);
                }))
                .child(if busy { "查询中…" } else { "⟳ 刷新" }),
        );

        let mut body = div().flex().flex_col().gap_2().p_3();
        match self.usage_balance.as_ref() {
            None if busy => {
                body = body.child(
                    div()
                        .text_xs()
                        .text_color(theme::muted())
                        .child("正在向厂商查询余额…"),
                );
            }
            None => {
                body = body.child(
                    div()
                        .text_xs()
                        .text_color(theme::muted())
                        .child("点「刷新」查询余额"),
                );
            }
            Some(BalanceOutcome::Unsupported) => {
                body = body.child(
                    div()
                        .text_xs()
                        .line_height(px(18.0))
                        .text_color(theme::muted())
                        .child("该厂商没有公开的余额查询接口，请到厂商控制台查看账户余额。"),
                );
                if let Some(url) = preset
                    .map(|p| p.console_url())
                    .filter(|u| !u.is_empty())
                    .map(|u| u.to_string())
                {
                    body = body.child(
                        div()
                            .flex()
                            .child(
                                div()
                                    .id("usage-console")
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .text_xs()
                                    .text_color(theme::accent())
                                    .cursor_pointer()
                                    .hover(|d| d.bg(theme::raised()))
                                    .on_click(cx.listener(move |_this, _: &gpui::ClickEvent, _w, cx| {
                                        cx.open_url(&url);
                                    }))
                                    .child("打开厂商控制台 ↗"),
                            ),
                    );
                }
            }
            Some(BalanceOutcome::Ok(info)) => {
                let symbol = currency_symbol(&info.currency);
                let mut amount = div()
                    .flex()
                    .items_baseline()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(22.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::fg())
                            .child(SharedString::from(format!("{symbol}{}", info.total))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .child(SharedString::from(info.currency.clone())),
                    );
                if !info.note.is_empty() {
                    amount = amount.child(
                        div()
                            .text_xs()
                            .text_color(theme::yellow())
                            .child(SharedString::from(format!("· {}", info.note))),
                    );
                }
                body = body.child(amount);
                for (k, v) in &info.rows {
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_xs()
                            .child(div().text_color(theme::dim()).child(SharedString::from(k.clone())))
                            .child(
                                div()
                                    .text_color(theme::muted())
                                    .child(SharedString::from(format!("{symbol}{v}"))),
                            ),
                    );
                }
            }
            Some(BalanceOutcome::Failed(e)) => {
                body = body.child(
                    div()
                        .text_xs()
                        .line_height(px(18.0))
                        .text_color(theme::red())
                        .child(SharedString::from(format!("查询失败：{e}"))),
                );
            }
        }

        div()
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .child(div().flex().items_center().gap_2().px_3().py_2().child(head))
            .child(body)
    }
}

/// 本会话用量：请求轮数 / 输入输出 token / 工具调用 / 累计耗时。
fn session_card(t: &crate::usage_ops::UsageTotals) -> impl IntoElement {
    let tiles = div()
        .flex()
        .flex_row()
        .gap_2()
        .child(stat_tile("请求轮数", t.requests.to_string()))
        .child(stat_tile("↑ 输入 token", fmt_int(t.tokens_in)))
        .child(stat_tile("↓ 输出 token", fmt_int(t.tokens_out)))
        .child(stat_tile("工具调用", t.tools.to_string()))
        .child(stat_tile("累计耗时", fmt_duration(t.elapsed_ms)));
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(theme::border())
        .bg(theme::panel())
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::fg())
                .child("本会话用量"),
        )
        .child(tiles)
}

/// 按模型汇总：整张表在本工作区范围内统计。
fn models_card(rows: &[ModelUsage]) -> impl IntoElement {
    let mut list = div().flex().flex_col();
    if rows.is_empty() {
        list = list.child(
            div()
                .py_2()
                .text_xs()
                .text_color(theme::dim())
                .child("还没有可统计的用量：发出第一条消息后就有了。"),
        );
    } else {
        // 表头
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .py_1()
                .text_size(px(10.0))
                .text_color(theme::dim())
                .child(div().flex_1().min_w(px(0.0)).child("厂商 · 模型"))
                .child(div().w(px(52.0)).text_right().child("请求"))
                .child(div().w(px(84.0)).text_right().child("↑ 输入"))
                .child(div().w(px(84.0)).text_right().child("↓ 输出"))
                .child(div().w(px(46.0)).text_right().child("工具")),
        );
        for (i, r) in rows.iter().enumerate() {
            let label = if r.model.trim().is_empty() {
                r.provider.clone()
            } else {
                format!("{} · {}", r.provider, r.model)
            };
            let mut row = div()
                .flex()
                .items_center()
                .gap_2()
                .py_1p5()
                .text_xs()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_color(theme::fg())
                        .child(SharedString::from(label)),
                )
                .child(
                    div()
                        .w(px(52.0))
                        .text_right()
                        .text_color(theme::muted())
                        .child(r.requests.to_string()),
                )
                .child(
                    div()
                        .w(px(84.0))
                        .text_right()
                        .text_color(theme::green())
                        .child(fmt_int(r.tokens_in)),
                )
                .child(
                    div()
                        .w(px(84.0))
                        .text_right()
                        .text_color(theme::cyan())
                        .child(fmt_int(r.tokens_out)),
                )
                .child(
                    div()
                        .w(px(46.0))
                        .text_right()
                        .text_color(theme::dim())
                        .child(r.tools.to_string()),
                );
            if i > 0 {
                row = row.border_t_1().border_color(theme::border());
            }
            list = list.child(row);
        }
    }
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(theme::border())
        .bg(theme::panel())
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
                        .child("按模型汇总"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(theme::dim())
                        .child("本工作区 · 累计"),
                ),
        )
        .child(list)
        .child(
            div()
                .text_size(px(10.0))
                .text_color(theme::dim())
                .child("token 取自厂商每轮回传的用量；没回传或旧会话未记录的轮次不计入。"),
        )
}

fn stat_tile(label: &str, value: String) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_w(px(0.0))
        .px_2()
        .py_2()
        .rounded_md()
        .bg(theme::raised())
        .child(
            div()
                .text_size(px(10.0))
                .text_color(theme::dim())
                .child(SharedString::from(label.to_string())),
        )
        .child(
            div()
                .text_sm()
                .text_color(theme::fg())
                .overflow_hidden()
                .whitespace_nowrap()
                .child(SharedString::from(value)),
        )
}

/// 千分位：1234567 → 1,234,567。
fn fmt_int(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn fmt_duration(ms: u64) -> String {
    if ms == 0 {
        return "—".to_string();
    }
    let secs = ms / 1000;
    if secs < 60 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousand_separator() {
        assert_eq!(fmt_int(0), "0");
        assert_eq!(fmt_int(999), "999");
        assert_eq!(fmt_int(1000), "1,000");
        assert_eq!(fmt_int(1234567), "1,234,567");
    }

    #[test]
    fn duration_formats() {
        assert_eq!(fmt_duration(0), "—");
        assert_eq!(fmt_duration(800), "0.8s");
        assert_eq!(fmt_duration(65_000), "1m5s");
    }
}
