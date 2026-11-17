//! 对话区模型选择器：按厂商分组，选中即写入**当前会话**。
//!
//! 为什么放在对话区：模型是会话级的（换会话就换模型）。设置页只负责
//! 「有哪些厂商、每个厂商开了哪些模型」，真正「这一轮用哪个」在聊天头部切。
//!
//! - 输入即过滤，支持分段模糊匹配（`4o` 能命中 `gpt-4o`、`4omini` 能命中 `gpt-4o-mini`）
//! - ↑/↓ 移动、Enter 选中、Esc 收起
//! - 第一行「跟随默认」= 清空会话里的厂商/模型，回到设置里的当前厂商
//! - 只列**启用且就绪**的厂商，厂商下只列逐条启用的模型

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

/// 子序列匹配：query 的字符按顺序出现在 haystack 中。
fn subsequence(haystack: &str, query: &str) -> bool {
    let mut it = haystack.chars();
    query.chars().all(|qc| it.any(|hc| hc == qc))
}

/// 选择器里的一行。`provider_id` 为空表示「跟随默认」。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModelChoice {
    pub provider_id: String,
    pub model: String,
}

/// 一组候选：厂商 id、展示名、分类标签、已启用模型。
pub(crate) struct ModelGroup {
    pub provider_id: String,
    pub name: String,
    pub kind: String,
    pub models: Vec<String>,
}

impl AppState {
    /// 打开对话区模型选择器（输入框同时进入编辑态）。
    pub(crate) fn open_chat_model_picker(&mut self, cx: &mut Context<Self>) {
        self.model_picker = true;
        self.model_query.clear();
        self.model_highlight = 0;
        // begin_field 会把 field_buf 填成 field_value(ModelSearch)（刚清空）。
        self.begin_field(FormField::ModelSearch, cx);
        // 高亮定到本会话当前用的模型：打开后 ↑/↓ 就是「在当前位置上下换」，
        // 而不是永远从顶上的「跟随默认」开始。
        let (cur_pid, cur_model) = {
            let s = self.session();
            (s.provider_id.clone(), s.model.clone())
        };
        self.model_highlight = highlight_for(&self.picker_choices(), &cur_pid, &cur_model);
    }

    pub(crate) fn close_chat_model_picker(&mut self, cx: &mut Context<Self>) {
        self.model_picker = false;
        self.model_query.clear();
        self.model_highlight = 0;
        if self.active_field == Some(FormField::ModelSearch) {
            self.active_field = None;
        }
        cx.notify();
    }

    /// 当前搜索词：编辑态看输入缓冲（边打边过滤），否则看已提交的值。
    fn picker_query(&self) -> String {
        if self.active_field == Some(FormField::ModelSearch) {
            self.field_buf.text().to_string()
        } else {
            self.model_query.clone()
        }
    }

    /// 过滤 + 分组后的候选。搜到厂商名时把它下面所有模型都带出来。
    pub(crate) fn picker_groups(&self, query: &str) -> Vec<ModelGroup> {
        let q = query.trim();
        let mut out = Vec::new();
        for p in &self.settings.providers {
            if !p.enabled || !p.ready() {
                continue;
            }
            let name_hit = !q.is_empty() && fuzzy_match(&p.name, q);
            let models: Vec<String> = p
                .enabled_model_ids()
                .into_iter()
                .filter(|m| q.is_empty() || name_hit || fuzzy_match(m, q))
                .collect();
            if models.is_empty() {
                continue;
            }
            out.push(ModelGroup {
                provider_id: p.id.clone(),
                name: p.name.clone(),
                kind: p.preset.kind().label().to_string(),
                models,
            });
        }
        out
    }

    /// 拉平后的可选行（第 0 行永远是「跟随默认」），键盘上下移动用它。
    pub(crate) fn picker_choices(&self) -> Vec<ModelChoice> {
        let q = self.picker_query();
        let mut out = vec![ModelChoice {
            provider_id: String::new(),
            model: String::new(),
        }];
        for g in self.picker_groups(&q) {
            for m in g.models {
                out.push(ModelChoice {
                    provider_id: g.provider_id.clone(),
                    model: m,
                });
            }
        }
        out
    }

    /// 选中一行：写进当前会话，换会话就跟着换。
    pub(crate) fn choose_session_model(
        &mut self,
        provider_id: String,
        model: String,
        cx: &mut Context<Self>,
    ) {
        let idx = self.active_session.min(self.sessions.len().saturating_sub(1));
        if let Some(s) = self.sessions.get_mut(idx) {
            s.provider_id = provider_id;
            s.model = model.clone();
        }
        self.save_sessions();
        self.action_msg = Some(if model.is_empty() {
            "本会话已改为跟随默认厂商".to_string()
        } else {
            format!("本会话模型：{model}")
        });
        self.close_chat_model_picker(cx);
    }

    /// 选择器键盘处理。文本输入走 `EntityInputHandler`，这里只管非文本键。
    pub(crate) fn on_model_picker_key(
        &mut self,
        ev: &gpui::KeyDownEvent,
        cx: &mut Context<Self>,
    ) {
        if self.active_field != Some(FormField::ModelSearch) {
            return;
        }
        let choices = self.picker_choices();
        let key = ev.keystroke.key.as_str();
        match key {
            "escape" => self.close_chat_model_picker(cx),
            "enter" => {
                if choices.is_empty() {
                    return;
                }
                let i = self.model_highlight.min(choices.len() - 1);
                if let Some(c) = choices.get(i).cloned() {
                    self.choose_session_model(c.provider_id, c.model, cx);
                }
            }
            "up" => {
                if self.model_highlight > 0 {
                    self.model_highlight -= 1;
                }
                cx.notify();
            }
            "down" => {
                if self.model_highlight + 1 < choices.len() {
                    self.model_highlight += 1;
                }
                cx.notify();
            }
            "backspace" => {
                // 搜索框已空时再退格就收起；非空交给输入处理器删字符。
                if self.picker_query().is_empty() {
                    self.close_chat_model_picker(cx);
                }
            }
            _ => {}
        }
    }

    /// 对话区头部的模型选择面板（内联在标题行下面展开）。
    ///
    /// 内联而不是浮层：浮层要处理遮罩、居中、出界、点空白关闭一堆事，
    /// 窄窗口/高 DPI 下很容易算歪；内联展开没有这些问题。
    pub(crate) fn chat_model_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session();
        let cur_pid = session.provider_id.clone();
        let cur_model = session.model.clone();
        let editing = self.active_field == Some(FormField::ModelSearch);

        let q = self.picker_query();
        let groups = self.picker_groups(&q);
        let highlight = self.model_highlight;

        let mut panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_5()
            .py_3()
            .border_b_1()
            .border_color(theme::border())
            .bg(theme::panel());

        // ---- 搜索行 ----
        panel = panel.child(
            div()
                .id("chat-model-search")
                .flex()
                .items_center()
                .gap_2()
                .h(px(30.0))
                .px_2()
                .rounded_md()
                .bg(theme::raised())
                .border_1()
                .border_color(if editing {
                    theme::accent_line()
                } else {
                    theme::border()
                })
                .text_xs()
                .cursor(gpui::CursorStyle::IBeam)
                .when(editing, |d| d.track_focus(&self.editor_focus))
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, window, cx| {
                    this.begin_field(FormField::ModelSearch, cx);
                    window.focus(&this.editor_focus);
                    cx.notify();
                }))
                .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
                    this.on_model_picker_key(ev, cx);
                }))
                .child(div().text_color(theme::dim()).child("⌕"))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_color(if q.is_empty() {
                            theme::dim()
                        } else {
                            theme::fg()
                        })
                        .child(SharedString::from(if q.is_empty() {
                            "搜索厂商或模型".to_string()
                        } else {
                            q.clone()
                        })),
                )
                .when(!q.is_empty(), |d| {
                    d.child(div().text_color(theme::accent()).child("▏"))
                })
                .child(
                    div()
                        .id("chat-model-close")
                        .px_2()
                        .text_color(theme::dim())
                        .cursor_pointer()
                        .hover(|d| d.text_color(theme::fg()))
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                            this.close_chat_model_picker(cx);
                        }))
                        .child("收起"),
                ),
        );

        // ---- 候选列表 ----
        let mut list = div()
            .id("chat-model-list")
            .flex()
            .flex_col()
            .gap_0p5()
            .max_h(px(320.0))
            .overflow_scroll();

        // 第 0 行：跟随默认（清空会话里的厂商/模型）；提示里带上实际的默认，
        // 不然用户不知道「默认」到底是哪个厂商哪个模型。
        let follow_default = cur_pid.is_empty() && cur_model.is_empty();
        let follow_hint = self
            .settings
            .active_provider()
            .map(|p| format!("{} · {}", p.name, p.effective_model()))
            .unwrap_or_else(|| "设置里的当前厂商".to_string());
        list = list.child(option_row(
            "chat-model-default".to_string(),
            highlight == 0,
            follow_default,
            "跟随默认".to_string(),
            follow_hint,
            cx,
            |this, cx| this.choose_session_model(String::new(), String::new(), cx),
        ));

        if groups.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_3()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("没有匹配的模型：先到「设置 → 模型与厂商」填 Key、启用模型"),
            );
        }

        // 行下标要和 picker_choices 对齐：0 是默认行，之后按分组顺序累加。
        let mut row_idx = 0usize;
        for group in groups {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .pt_2()
                    .pb_1()
                    .text_xs()
                    .text_color(theme::dim())
                    .child(SharedString::from(fit_text(&group.name, 220.0, 11.0)))
                    .child("·")
                    .child(SharedString::from(group.kind))
                    .child(div().flex_1())
                    .child(SharedString::from(format!("{} 个", group.models.len()))),
            );
            for model in group.models {
                row_idx += 1;
                let active = group.provider_id == cur_pid && model == cur_model;
                let pid = group.provider_id.clone();
                let m = model.clone();
                list = list.child(option_row(
                    format!("chat-model-{row_idx}"),
                    highlight == row_idx,
                    active,
                    model,
                    String::new(),
                    cx,
                    move |this, cx| this.choose_session_model(pid.clone(), m.clone(), cx),
                ));
            }
        }
        panel = panel.child(list);

        // ---- 页脚 ----
        panel.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .text_xs()
                .text_color(theme::dim())
                .child("↑↓ 选择")
                .child("Enter 确认")
                .child("Esc 收起")
                .child(div().flex_1())
                .child(SharedString::from(format!(
                    "{} 项",
                    self.picker_choices().len()
                ))),
        )
    }
}

/// 打开选择器时的高亮行：当前会话的模型行；找不到（厂商被删/停用）就回「跟随默认」。
fn highlight_for(choices: &[ModelChoice], pid: &str, model: &str) -> usize {
    choices
        .iter()
        .position(|c| c.provider_id == pid && c.model == model)
        .unwrap_or(0)
}

/// 一行候选项（只画、不挂事件）。
fn option_row(
    id: String,
    highlighted: bool,
    active: bool,
    title: String,
    hint: String,
    cx: &mut Context<AppState>,
    on_click: impl Fn(&mut AppState, &mut Context<AppState>) + 'static,
) -> impl IntoElement {
    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .gap_2()
        .h(px(30.0))
        .px_2()
        .rounded_sm()
        .text_xs()
        .cursor_pointer()
        .when(highlighted, |d| d.bg(theme::accent_soft()))
        .when(!highlighted, |d| d.hover(|d| d.bg(theme::raised())))
        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
            on_click(this, cx);
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
                .flex_shrink_0()
                .text_color(if highlighted {
                    theme::fg()
                } else {
                    theme::muted()
                })
                .child(SharedString::from(fit_text(&title, 280.0, 12.0))),
        )
        .when(!hint.is_empty(), |d| {
            d.child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_color(theme::dim())
                    .child(SharedString::from(fit_text(&hint, 320.0, 11.0))),
            )
        })
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

    #[test]
    fn case_insensitive() {
        assert!(fuzzy_match("GPT-4O", "gpt"));
        assert!(fuzzy_match("gpt-4o", "Gpt"));
    }

    #[test]
    fn highlight_lands_on_current_model() {
        let choices = vec![
            ModelChoice {
                provider_id: String::new(),
                model: String::new(),
            },
            ModelChoice {
                provider_id: "deepseek".into(),
                model: "deepseek-flash".into(),
            },
        ];
        assert_eq!(highlight_for(&choices, "deepseek", "deepseek-flash"), 1);
        // 跟随默认：第 0 行就是 ("","")，自然命中
        assert_eq!(highlight_for(&choices, "", ""), 0);
        // 厂商被删/停用后当前模型不在列表里：回「跟随默认」
        assert_eq!(highlight_for(&choices, "gone", "x"), 0);
    }
}
