//! 模型选择器：可搜索的浮层，替代"点一下切一个"的循环切换。
//!
//! 为什么要它：豆包、通义这类厂商动辄几十个模型，循环点击要按十几次才能到目标；
//! 而纯手输又记不住准确名称。这里给一个**能搜索、能键盘操作、能顺手自定义**的选择器：
//! - 输入即过滤，支持分段模糊匹配（`4o` 能命中 `gpt-4o`、`4omini` 能命中 `gpt-4o-mini`）
//! - ↑/↓ 移动、Enter 选中、Esc 关闭
//! - 搜索词不在候选里时，Enter 直接把它当作自定义模型名
//! - 顶部显示当前模型，列表里标出当前项

use gpui::prelude::*;
use gpui::{div, px, Context, SharedString};

use neverwork_ui::theme;

use crate::state::{AppState, FormField};
use crate::views::text_fit::fit_text;

/// 模糊匹配：按顺序包含 query 的每个片段即算命中。
///
/// 例如 query = "4omini" 能命中 "gpt-4o-mini"（4o、mini 依次出现）；
/// query = "deep seek" 能命中 "deepseek-chat"（忽略空格）。
pub fn fuzzy_match(haystack: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let h = haystack.to_lowercase();
    // 先按空格拆成多段，每段都要按顺序出现。
    let mut rest = h.as_str();
    for part in q.split_whitespace() {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => {
                // 整串按顺序扫一遍（去掉空格后）也算命中
                return subsequence(&h, &q.replace(' ', ""));
            }
        }
    }
    true
}

/// 过滤候选模型。搜索词不在候选里时，把它作为第一条「新名称」插入，
/// 这样用户输入一个列表外的名字后按 Enter 就能直接用。
pub fn filter_candidates(models: &[String], query: &str) -> Vec<String> {
    let mut out: Vec<String> = models
        .iter()
        .filter(|m| fuzzy_match(m, query))
        .cloned()
        .collect();
    let q = query.trim();
    if !q.is_empty() && !out.iter().any(|m| m == q) {
        out.insert(0, q.to_string());
    }
    out
}

/// 子序列匹配：query 的字符按顺序出现在 haystack 中。
fn subsequence(haystack: &str, query: &str) -> bool {
    let mut it = haystack.chars();
    query.chars().all(|qc| it.any(|hc| hc == qc))
}

impl AppState {
    /// 打开某厂商的模型选择器。
    pub(crate) fn open_model_picker(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.settings.providers.len() {
            return;
        }
        self.model_picker = Some(idx);
        self.model_query.clear();
        self.model_highlight = 0;
        self.field_buf.reset("");
        self.active_field = Some(FormField::ModelSearch);
        cx.notify();
    }

    pub(crate) fn close_model_picker(&mut self, cx: &mut Context<Self>) {
        self.model_picker = None;
        self.model_query.clear();
        self.model_highlight = 0;
        if self.active_field == Some(FormField::ModelSearch) {
            self.active_field = None;
        }
        cx.notify();
    }

    /// 当前选择器过滤后的候选模型。
    pub(crate) fn picker_candidates(&self) -> Vec<String> {
        let Some(idx) = self.model_picker else {
            return Vec::new();
        };
        let Some(p) = self.settings.providers.get(idx) else {
            return Vec::new();
        };
        filter_candidates(&p.models, &self.model_query)
    }

    /// 选中模型：写回厂商配置。
    pub(crate) fn choose_model(&mut self, model: String, cx: &mut Context<Self>) {
        let Some(idx) = self.model_picker else {
            return;
        };
        if let Some(p) = self.settings.providers.get_mut(idx) {
            let m = model.trim().to_string();
            if m.is_empty() {
                return;
            }
            p.model = m.clone();
            // 手输的新模型进候选列表，下次直接可选。
            if !p.models.iter().any(|x| x == &m) {
                p.models.insert(0, m.clone());
            }
        }
        self.update_settings(|_| {}, cx);
        self.action_msg = Some(format!("已切换到模型 {model}"));
        self.close_model_picker(cx);
        cx.notify();
    }

    /// 选择器键盘处理。
    pub(crate) fn on_model_picker_key(
        &mut self,
        ev: &gpui::KeyDownEvent,
        cx: &mut Context<Self>,
    ) {
        let key = ev.keystroke.key.as_str();
        let candidates = self.picker_candidates();
        match key {
            "escape" => self.close_model_picker(cx),
            "enter" => {
                if let Some(m) = candidates.get(self.model_highlight).cloned() {
                    self.choose_model(m, cx);
                }
            }
            "up" => {
                if self.model_highlight > 0 {
                    self.model_highlight -= 1;
                }
                cx.notify();
            }
            "down" => {
                if self.model_highlight + 1 < candidates.len() {
                    self.model_highlight += 1;
                }
                cx.notify();
            }
            "backspace" => {
                if self.model_query.pop().is_none() {
                    // 搜索框已空时再退格就关掉，符合"Esc 之外第二条退路"的直觉。
                    self.close_model_picker(cx);
                    return;
                }
                self.model_highlight = 0;
                cx.notify();
            }
            _ => {
                // 可打印字符：IME 会把文本写进 field_buf，这里从 field_buf 同步。
                if let Some(ch) = &ev.keystroke.key_char {
                    if !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform {
                        self.model_query.push_str(ch);
                        self.model_highlight = 0;
                        cx.notify();
                    }
                }
            }
        }
    }


    /// 渲染模型选择面板（**内联**，嵌在厂商卡片里原地展开）。
    ///
    /// 不用浮层是有意的：浮层要处理遮罩、居中、超出屏幕、点空白关闭等一堆事，
    /// 而且在窄窗口/高 DPI 下很容易算歪；内联展开没有这些问题，
    /// 视觉上也和"点开这一行继续改"的直觉一致。
    pub(crate) fn model_picker_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let idx = self.model_picker.unwrap_or(0);
        let provider = self.settings.providers.get(idx);
        let current = provider.map(|p| p.model.clone()).unwrap_or_default();
        let candidates = self.picker_candidates();
        let total = provider.map(|p| p.models.len()).unwrap_or(0);
        let q = self.model_query.clone();

        let mut panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded_md()
            .bg(theme::bg())
            .border_1()
            .border_color(theme::accent_line());

        // 搜索框
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(px(28.0))
                .px_2()
                .rounded_md()
                .bg(theme::raised())
                .cursor(gpui::CursorStyle::IBeam)
                .child(div().text_xs().text_color(theme::dim()).child("⌕"))
                .child(
                    div()
                        .id("model-search")
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_1()
                        .min_w(px(0.0))
                        .track_focus(&self.editor_focus)
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, window, cx| {
                            this.active_field = Some(FormField::ModelSearch);
                            window.focus(&this.editor_focus);
                            cx.notify();
                        }))
                        .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
                            this.on_model_picker_key(ev, cx);
                        }))
                        .child(
                            div()
                                .text_xs()
                                .text_color(if q.is_empty() {
                                    theme::dim()
                                } else {
                                    theme::fg()
                                })
                                .child(SharedString::from(if q.is_empty() {
                                    format!("搜索 {total} 个模型，或直接输入新名称")
                                } else {
                                    q.clone()
                                })),
                        )
                        .when(!q.is_empty(), |d| {
                            d.child(div().text_color(theme::accent()).child("▏"))
                        }),
                )
                .child(
                    div()
                        .id("model-close")
                        .px_2()
                        .text_xs()
                        .text_color(theme::dim())
                        .cursor_pointer()
                        .hover(|d| d.text_color(theme::fg()))
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                            this.close_model_picker(cx);
                        }))
                        .child("收起"),
                ),
        );

        // 候选列表（限高可滚，模型再多也不会把页面撑爆）
        let mut list = div()
            .id("model-list")
            .flex()
            .flex_col()
            .gap_0p5()
            .max_h(px(240.0))
            .overflow_scroll();

        if candidates.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_3()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("没有匹配的模型"),
            );
        }

        let is_custom_entry =
            !q.trim().is_empty() && candidates.first().map(|c| c == q.trim()).unwrap_or(false);

        for (i, m) in candidates.iter().enumerate() {
            let highlighted = i == self.model_highlight;
            let active = m == &current;
            let is_new = is_custom_entry && i == 0;
            let m_for_click = m.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("model-opt-{i}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(28.0))
                    .px_2()
                    .rounded_sm()
                    .text_xs()
                    .cursor_pointer()
                    .when(highlighted, |d| d.bg(theme::accent_soft()))
                    .when(!highlighted, |d| d.hover(|d| d.bg(theme::raised())))
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                        this.choose_model(m_for_click.clone(), cx);
                    }))
                    .child(
                        div()
                            .w(px(14.0))
                            .flex_shrink_0()
                            .text_color(theme::accent())
                            .child(if active { "✓" } else { "" }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_color(if highlighted { theme::fg() } else { theme::muted() })
                            .child(SharedString::from(fit_text(m, 300.0, 12.0))),
                    )
                    .when(is_new, |d| {
                        d.child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded_sm()
                                .bg(theme::accent())
                                .text_color(gpui::rgb(0xffffff))
                                .child("新名称"),
                        )
                    }),
            );
        }
        panel = panel.child(list);

        panel.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .text_xs()
                .text_color(theme::dim())
                .child("↑↓ 选择")
                .child("Enter 确认")
                .child(div().flex_1())
                .child(SharedString::from(format!("{} 项", candidates.len()))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_all() {
        assert!(fuzzy_match("gpt-4o", ""));
        assert!(fuzzy_match("anything", "  "));
    }

    #[test]
    fn exact_and_substring() {
        assert!(fuzzy_match("gpt-4o", "4o"));
        assert!(fuzzy_match("claude-sonnet-4-5", "sonnet"));
        assert!(!fuzzy_match("gpt-4o", "claude"));
    }

    #[test]
    fn multi_segment_in_order() {
        assert!(fuzzy_match("gpt-4o-mini", "4o mini"));
        assert!(fuzzy_match("deepseek-chat", "deep seek"));
        // 顺序反了不该命中
        assert!(!fuzzy_match("gpt-4o-mini", "mini 4o"));
    }

    #[test]
    fn subsequence_fallback() {
        assert!(fuzzy_match("qwen2.5-coder", "qwc"));
        assert!(!fuzzy_match("qwen2.5-coder", "cwq"));
    }

    fn models() -> Vec<String> {
        ["gpt-4o", "gpt-4o-mini", "gpt-4.1", "o3", "o4-mini"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn filter_narrows_list() {
        let out = filter_candidates(&models(), "4o");
        // 含 4o 的两个 + 首位的自定义项（"4o" 本身不在列表里）
        assert!(out.contains(&"gpt-4o".to_string()));
        assert!(out.contains(&"gpt-4o-mini".to_string()));
        assert!(!out.contains(&"o3".to_string()));
        assert_eq!(out[0], "4o", "搜索词应作为第一条新名称");
    }

    #[test]
    fn filter_exact_match_is_not_duplicated() {
        let out = filter_candidates(&models(), "gpt-4o");
        let count = out.iter().filter(|m| *m == "gpt-4o").count();
        assert_eq!(count, 1, "已存在的模型不该被当成新名称重复插入");
        assert_eq!(out[0], "gpt-4o", "精确命中的应排第一");
    }

    #[test]
    fn filter_empty_query_returns_all() {
        let out = filter_candidates(&models(), "");
        assert_eq!(out.len(), models().len());
        // 空搜索词不插入自定义项
        assert!(!out.is_empty());
    }

    #[test]
    fn filter_no_match_still_offers_typed_name() {
        let out = filter_candidates(&models(), "my-private-model");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], "my-private-model");
    }

    #[test]
    fn case_insensitive() {
        assert!(fuzzy_match("GPT-4O", "gpt"));
        assert!(fuzzy_match("gpt-4o", "Gpt"));
    }
}
