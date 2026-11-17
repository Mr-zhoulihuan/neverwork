//! 「模型与厂商」页：厂商列表 → 厂商详情 / 添加面板。
//!
//! 三段式（参考 magpie）：列表只放已添加的厂商（名称 / 基址 / 模型数 /
//! Key 状态 / 启停开关 / 箭头），详情页管 Key、基址、模型（拉取、筛选、
//! 逐条启停、设为默认、手动添加），添加面板按分类列出全部预设并给
//! 已添加的打勾。所有改动即时落盘（与设置页其它项一致）。

use gpui::prelude::*;
use gpui::{div, px, Context, FontWeight, SharedString};

use neverwork_core::{ProviderEntry, ProviderKind, ProviderPreset};
use neverwork_ui::theme;

use crate::state::{AppState, FormField, ProviderView};
use crate::views::model_picker::fuzzy_match;
use crate::views::settings::{btn, form_row, input_box, section_card, setting_row, stepper, tag};
use crate::views::text_fit::fit_text;

impl AppState {
    /// 按当前视图分发：列表 / 添加面板 / 厂商详情。
    pub(crate) fn settings_models(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        match self.provider_view {
            ProviderView::List => self.provider_list(cx).into_any_element(),
            ProviderView::Add => self.provider_add_panel(cx).into_any_element(),
            ProviderView::Detail(idx) => self.provider_detail(idx, cx).into_any_element(),
        }
    }

    // ==================== 厂商列表 ====================

    fn provider_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().gap_3().min_w(px(0.0));

        // 汇总条
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded_md()
                .bg(theme::accent_soft())
                .text_xs()
                .text_color(theme::muted())
                .child(SharedString::from(format!(
                    "{} 个厂商 · {} 个已填 Key · 默认使用 {}",
                    self.settings.providers.len(),
                    self.settings.keyed_provider_count(),
                    self.settings
                        .active_provider()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "未选择".into())
                )))
                .child(div().flex_1())
                .child(btn("add-provider", "＋ 添加厂商", cx, |this, cx| {
                    this.open_provider_add(cx);
                })),
        );

        if self.settings.providers.is_empty() {
            return col.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .py_8()
                    .text_sm()
                    .text_color(theme::dim())
                    .child("还没有添加任何厂商")
                    .child(div().text_xs().child("点右上「＋ 添加厂商」，从预设里选一个或自定义")),
            );
        }

        // 当前会话指定的厂商 / 模型（列表里标「本会话」）
        let session_pid = self
            .sessions
            .get(self.active_session)
            .map(|s| s.provider_id.clone())
            .unwrap_or_default();

        for idx in 0..self.settings.providers.len() {
            let p = &self.settings.providers[idx];
            let is_default = self.settings.active_provider_id == p.id;
            let in_use = !session_pid.is_empty() && session_pid == p.id;
            let ready = p.ready();
            let enabled_models = p.enabled_model_ids().len();
            let masked = p.masked_key();
            let is_local = p.preset.is_local();

            let row = div()
                .id(SharedString::from(format!("provider-row-{idx}")))
                .flex()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded_lg()
                .border_1()
                .border_color(if is_default {
                    theme::accent_line()
                } else {
                    theme::border()
                })
                .bg(theme::panel())
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()))
                // 停用的厂商整行降透明度：一眼看出它不参与选模型。
                .when(!p.enabled, |d| d.opacity(0.6))
                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                    this.open_provider_detail(idx, cx);
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(30.0))
                        .flex_shrink_0()
                        .rounded_md()
                        .bg(theme::accent_soft())
                        .text_sm()
                        .text_color(theme::accent())
                        .child(SharedString::from(
                            p.name.chars().next().unwrap_or('?').to_string(),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
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
                                        .child(SharedString::from(fit_text(&p.name, 240.0, 14.0))),
                                )
                                .child(tag(p.preset.kind().label(), theme::dim()))
                                .child(tag(p.protocol().label(), theme::dim()))
                                .when(is_default, |d| d.child(tag("默认", theme::accent())))
                                .when(in_use, |d| d.child(tag("本会话", theme::green())))
                                .when(!p.enabled, |d| d.child(tag("已停用", theme::dim()))),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::dim())
                                .child(SharedString::from(fit_text(&p.base_url, 320.0, 12.0))),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .flex_shrink_0()
                        .child(tag(&format!("{} 个模型", p.models.len()), theme::muted()))
                        .when(enabled_models < p.models.len(), |d| {
                            d.child(tag(&format!("{enabled_models} 启用"), theme::dim()))
                        })
                        .when(!is_local && !masked.is_empty(), |d| {
                            d.child(tag(&masked, theme::dim()))
                        })
                        .child(
                            div()
                                .size(px(7.0))
                                .rounded_full()
                                .bg(if ready { theme::green() } else { theme::border() }),
                        ),
                )
                // 启停开关：单独 on_click，不触发「进详情」
                .child(provider_switch(idx, p.enabled, cx))
                .child(div().text_color(theme::dim()).child("›"));
            col = col.child(row);
        }
        col
    }

    // ==================== 添加厂商 ====================

    fn provider_add_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().gap_3().min_w(px(0.0));

        col = col.child(back_row(cx));
        col = col.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::fg())
                        .child("添加厂商"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::dim())
                        .child("选一个预设直接填 Key，或自定义地址；已添加的点开即进入详情"),
                ),
        );

        // 搜索：名称 / 标识 / 地址
        let editing = self.active_field == Some(FormField::ProviderSearch);
        let q = if editing {
            self.field_buf.text().to_string()
        } else {
            self.provider_query.clone()
        };
        col = col.child(input_box(
            SharedString::from("provider-search"),
            q,
            "搜索名称、标识或地址",
            editing,
            self.editor_focus.clone(),
            self.field_area_bounds.clone(),
            cx,
            FormField::ProviderSearch,
        ));

        let query = self.provider_query.clone();
        for kind in ProviderKind::all() {
            let presets: Vec<ProviderPreset> = ProviderPreset::all()
                .iter()
                .copied()
                .filter(|p| p.kind() == *kind)
                .filter(|p| {
                    if query.trim().is_empty() {
                        return true;
                    }
                    fuzzy_match(
                        &format!("{} {} {}", p.name(), p.id(), p.base_url()),
                        &query,
                    )
                })
                .collect();
            if presets.is_empty() {
                continue;
            }
            col = col.child(
                div()
                    .text_xs()
                    .text_color(theme::dim())
                    .pt_2()
                    .child(SharedString::from(kind.label())),
            );
            let mut grid = div().flex().flex_row().flex_wrap().gap_2();
            for preset in presets {
                grid = grid.child(self.preset_tile(preset, cx));
            }
            col = col.child(grid);
        }
        col
    }

    /// 添加面板里的一个预设卡片；已添加的显示对勾。
    fn preset_tile(&self, preset: ProviderPreset, cx: &mut Context<Self>) -> impl IntoElement {
        // 自定义永远可再添加一个，不参与「已添加」判定。
        let added = if preset == ProviderPreset::Custom {
            None
        } else {
            self.settings.providers.iter().find(|p| p.preset == preset)
        };
        let ready = added.map(|p| p.ready()).unwrap_or(false);
        div()
            .id(SharedString::from(format!("preset-{}", preset.id())))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .w(px(220.0))
            .h(px(56.0))
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .cursor_pointer()
            .hover(|d| d.bg(theme::raised()))
            .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                this.add_provider_from_preset(preset, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.0))
                    .flex_shrink_0()
                    .rounded_md()
                    .bg(theme::accent_soft())
                    .text_xs()
                    .text_color(theme::accent())
                    .child(SharedString::from(
                        preset.name().chars().next().unwrap_or('?').to_string(),
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::fg())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(SharedString::from(fit_text(preset.name(), 150.0, 12.0))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(SharedString::from(fit_text(
                                &preset_host(preset),
                                150.0,
                                11.0,
                            ))),
                    ),
            )
            .when(added.is_some(), |d| {
                d.child(div().text_xs().text_color(if ready {
                    theme::green()
                } else {
                    theme::dim()
                })
                .child(if ready { "✓ 已就绪" } else { "✓ 已添加" }))
            })
    }

    // ==================== 厂商详情 ====================

    fn provider_detail(&self, idx: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().gap_3().min_w(px(0.0));
        let Some(p) = self.settings.providers.get(idx) else {
            return col
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::dim())
                        .child("这个厂商已被移除"),
                )
                .child(back_row(cx));
        };
        let i8 = idx as u8;
        let is_default = self.settings.active_provider_id == p.id;
        let ready = p.ready();
        let local = p.preset.is_local();
        let masked = p.masked_key();
        let is_custom = p.preset == ProviderPreset::Custom;

        col = col.child(back_row(cx));

        // ---- 头部：图标 / 名称 / 状态 + 操作 ----
        let mut head = div()
            .flex()
            .items_center()
            .gap_3()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(if is_default {
                theme::accent_line()
            } else {
                theme::border()
            })
            .bg(theme::panel())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(34.0))
                    .flex_shrink_0()
                    .rounded_md()
                    .bg(theme::accent_soft())
                    .text_sm()
                    .text_color(theme::accent())
                    .child(SharedString::from(
                        p.name.chars().next().unwrap_or('?').to_string(),
                    )),
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
                                    .child(SharedString::from(fit_text(&p.name, 260.0, 14.0))),
                            )
                            .child(tag(p.preset.kind().label(), theme::dim()))
                            .child(tag(p.protocol().label(), theme::dim()))
                            .when(is_custom, |d| d.child(tag("自定义", theme::purple())))
                            .when(ready, |d| d.child(tag("已就绪", theme::green())))
                            .when(!ready, |d| d.child(tag("未配置", theme::dim()))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .child(SharedString::from(fit_text(&p.base_url, 360.0, 12.0))),
                    ),
            )
            .child(div().flex_1());

        head = head.child(ghost(
            "use-provider",
            if is_default { "默认中" } else { "设为默认" },
            if is_default { theme::accent() } else { theme::muted() },
            cx,
            move |this, cx| {
                let id = this
                    .settings
                    .providers
                    .get(idx)
                    .map(|p| p.id.clone())
                    .unwrap_or_default();
                this.set_active_provider(id, cx);
            },
        ));
        head = head.child(ghost(
            "test-provider",
            if self.provider_test_busy {
                "测试中…"
            } else {
                "测试"
            },
            if self.provider_test_busy {
                theme::dim()
            } else {
                theme::accent()
            },
            cx,
            move |this, cx| this.test_provider(idx, cx),
        ));
        let confirming_remove = self.provider_remove_confirm == Some(idx);
        head = head.child(ghost(
            "remove-provider",
            if confirming_remove {
                "确认移除"
            } else {
                "移除"
            },
            theme::red(),
            cx,
            move |this, cx| this.click_remove_provider(idx, cx),
        ));
        col = col.child(head);

        // 未就绪时把缺什么直接写在页面上（不靠按钮点一下才弹提示）。
        if let Some(reason) = not_ready_reason(p) {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(theme::accent_soft())
                    .text_xs()
                    .text_color(theme::muted())
                    .child(
                        div()
                            .text_color(theme::accent())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("!"),
                    )
                    .child(SharedString::from(reason)),
            );
        }

        // ---- 字段：显示名 / Key / 基址 ----
        let mut fields = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel());

        let editing_name = self.active_field == Some(FormField::ProviderName(i8));
        let name_display = if editing_name {
            self.field_buf.text().to_string()
        } else {
            p.name.clone()
        };
        fields = fields.child(form_row(
            "显示名",
            "列表与回答页脚展示",
            input_box(
                SharedString::from(format!("pname-{idx}")),
                name_display,
                "厂商显示名",
                editing_name,
                self.editor_focus.clone(),
                self.field_area_bounds.clone(),
                cx,
                FormField::ProviderName(i8),
            ),
        ));

        if !local {
            let editing_key = self.active_field == Some(FormField::ProviderKey(i8));
            let key_display = if editing_key {
                self.field_buf.text().to_string()
            } else {
                masked.clone()
            };
            let hint = if p.preset.console_url().is_empty() {
                "只存本机，导出默认脱敏".to_string()
            } else {
                format!("只存本机 · 在 {} 申请", short_host(p.preset.console_url()))
            };
            fields = fields.child(form_row(
                "API Key",
                &hint,
                input_box(
                    SharedString::from(format!("pkey-{idx}")),
                    key_display,
                    "粘贴 API Key（回车或点击别处即保存）",
                    editing_key,
                    self.editor_focus.clone(),
                    self.field_area_bounds.clone(),
                    cx,
                    FormField::ProviderKey(i8),
                ),
            ));
        }

        let editing_url = self.active_field == Some(FormField::ProviderUrl(i8));
        let url_display = if editing_url {
            self.field_buf.text().to_string()
        } else {
            p.base_url.clone()
        };
        fields = fields.child(form_row(
            "API 基址",
            "对话与拉取模型都以它为根地址",
            input_box(
                SharedString::from(format!("purl-{idx}")),
                url_display,
                "https://…/v1",
                editing_url,
                self.editor_focus.clone(),
                self.field_area_bounds.clone(),
                cx,
                FormField::ProviderUrl(i8),
            ),
        ));
        col = col.child(fields);

        // ---- 模型 ----
        let enabled_cnt = p.models.iter().filter(|m| m.enabled).count();
        let all_on = !p.models.is_empty() && enabled_cnt == p.models.len();
        let filtering = !self.provider_model_filter.trim().is_empty();
        let visible: Vec<(usize, String, bool)> = p
            .models
            .iter()
            .enumerate()
            .filter(|(_, m)| fuzzy_match(&m.id, &self.provider_model_filter))
            .map(|(i, m)| (i, m.id.clone(), m.enabled))
            .collect();
        let current = p.effective_model();
        let busy = self.provider_models_busy;
        // 默认模型置顶：几十个模型的厂商里不用滚半天才找到当前用的那个。
        let mut visible = visible;
        visible.sort_by_key(|(_, id, _)| if *id == current { 0 } else { 1 });

        let mut tools = div().flex().items_center().gap_2().flex_wrap();
        tools = tools.child(ghost(
            "fetch-models",
            if busy { "拉取中…" } else { "⟳ 拉取模型" },
            if busy { theme::dim() } else { theme::accent() },
            cx,
            move |this, cx| this.fetch_provider_models(idx, cx),
        ));
        tools = tools.child(ghost(
            "toggle-all-models",
            if all_on { "全部停用" } else { "全部启用" },
            theme::muted(),
            cx,
            move |this, cx| {
                this.update_settings(
                    |s| {
                        if let Some(p) = s.providers.get_mut(idx) {
                            for m in &mut p.models {
                                m.enabled = !all_on;
                            }
                        }
                    },
                    cx,
                );
            },
        ));
        tools = tools.child(div().flex_1());
        tools = tools.child(
            div()
                .text_xs()
                .text_color(theme::dim())
                .child(SharedString::from(format!(
                    "{} 个 · {} 启用",
                    p.models.len(),
                    enabled_cnt
                ))),
        );

        let editing_filter = self.active_field == Some(FormField::ProviderModelFilter);
        let editing_new = self.active_field == Some(FormField::ProviderNewModel(i8));
        let new_display = if editing_new {
            self.field_buf.text().to_string()
        } else {
            String::new()
        };
        let inputs = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div().w(px(200.0)).flex_shrink_0().child(input_box(
                    SharedString::from("model-filter"),
                    self.provider_model_filter.clone(),
                    "筛选模型",
                    editing_filter,
                    self.editor_focus.clone(),
                    self.field_area_bounds.clone(),
                    cx,
                    FormField::ProviderModelFilter,
                )),
            )
            .child(input_box(
                SharedString::from(format!("new-model-{idx}")),
                new_display,
                "输入模型 id，回车添加",
                editing_new,
                self.editor_focus.clone(),
                self.field_area_bounds.clone(),
                cx,
                FormField::ProviderNewModel(i8),
            ));

        let mut list = div()
            .id("provider-model-list")
            .flex()
            .flex_col()
            .gap_0p5()
            .max_h(px(340.0))
            .overflow_scroll();

        if visible.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_3()
                    .text_xs()
                    .text_color(theme::dim())
                    .child(if filtering {
                        "没有匹配的模型"
                    } else {
                        "还没有模型：点「拉取模型」或手动添加"
                    }),
            );
        }

        for (j, (_, id, on)) in visible.iter().enumerate() {
            let is_current = *id == current;
            let id_for_toggle = id.clone();
            let id_for_default = id.clone();
            let id_for_remove = id.clone();
            let row = div()
                .id(SharedString::from(format!("pm-{idx}-{j}")))
                .flex()
                .items_center()
                .gap_2()
                .h(px(28.0))
                .px_2()
                .rounded_sm()
                .text_xs()
                .when(!*on, |d| d.opacity(0.55))
                .hover(|d| d.bg(theme::raised()))
                .child(
                    div()
                        .id(SharedString::from(format!("pm-toggle-{idx}-{j}")))
                        .w(px(18.0))
                        .flex_shrink_0()
                        .cursor_pointer()
                        .text_color(if *on { theme::green() } else { theme::dim() })
                        .child(if *on { "✓" } else { "○" })
                        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                            this.toggle_provider_model(idx, &id_for_toggle, cx);
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_color(if is_current { theme::fg() } else { theme::muted() })
                        .child(SharedString::from(fit_text(id, 420.0, 12.0))),
                )
                .when(is_current, |d| d.child(tag("默认", theme::accent())))
                .child(div().flex_1())
                .when(!is_current, |d| {
                    d.child(
                        div()
                            .id(SharedString::from(format!("pm-default-{idx}-{j}")))
                            .px_2()
                            .py_0p5()
                            .rounded_sm()
                            .text_color(theme::accent())
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::border()))
                            .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                                this.set_provider_default_model(idx, &id_for_default, cx);
                            }))
                            .child("设为默认"),
                    )
                })
                .child(
                    div()
                        .id(SharedString::from(format!("pm-del-{idx}-{j}")))
                        .px_2()
                        .py_0p5()
                        .rounded_sm()
                        .text_color(theme::dim())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::border()).text_color(theme::red()))
                        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                            this.remove_provider_model(idx, &id_for_remove, cx);
                        }))
                        .child("×"),
                );
            list = list.child(row);
        }

        col = col.child(section_card(
            "模型",
            format!(
                "{} 个模型 · {} 个启用 · 默认 {}",
                p.models.len(),
                enabled_cnt,
                if current.is_empty() {
                    "未选择"
                } else {
                    current.as_str()
                }
            ),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(tools)
                .child(inputs)
                .child(list),
        ));

        // ---- 温度 ----
        col = col.child(section_card(
            "采样温度",
            "越低越稳定，越高越发散",
            setting_row(
                "温度",
                format!("{:.1}", p.temperature),
                stepper(
                    &format!("ptemp-{idx}"),
                    cx,
                    (p.temperature * 10.0).round() as i32,
                    0,
                    20,
                    1,
                    "",
                    move |this, v, cx| this.set_provider_temperature(idx, v as f32 / 10.0, cx),
                ),
            ),
        ));

        col
    }
}

// ==================== 小组件 ====================

/// 返回厂商列表的一行。
fn back_row(cx: &mut Context<AppState>) -> impl IntoElement {
    div()
        .id("back-providers")
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .text_color(theme::muted())
        .cursor_pointer()
        .hover(|d| d.text_color(theme::fg()))
        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
            this.close_provider_detail(cx);
        }))
        .child("‹ 厂商列表")
}

/// 列表行右侧的启停开关（独立点击，不触发进详情）。
fn provider_switch(idx: usize, on: bool, cx: &mut Context<AppState>) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("provider-switch-{idx}")))
        .flex()
        .items_center()
        .w(px(38.0))
        .h(px(20.0))
        .px_0p5()
        .flex_shrink_0()
        .rounded_full()
        .cursor_pointer()
        .when(on, |d| d.bg(theme::accent()).justify_end())
        .when(!on, |d| d.bg(theme::border()).justify_start())
        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
            cx.stop_propagation();
            this.toggle_provider(idx, cx);
        }))
        .child(div().size(px(16.0)).rounded_full().bg(gpui::rgb(0xffffff)))
}

/// 轻量文字按钮（详情页头部与模型工具行）。
fn ghost(
    id: &str,
    label: &str,
    color: gpui::Rgba,
    cx: &mut Context<AppState>,
    f: impl Fn(&mut AppState, &mut Context<AppState>) + 'static,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("{id}")))
        .px_2()
        .py_1()
        .rounded_md()
        .text_xs()
        .text_color(color)
        .cursor_pointer()
        .hover(|d| d.bg(theme::border()))
        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| f(this, cx)))
        .child(SharedString::from(label.to_string()))
}

/// `https://open.bigmodel.cn/usercenter/apikeys` → `open.bigmodel.cn`。
fn short_host(url: &str) -> String {
    let s = url
        .trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"))
        .unwrap_or(url.trim());
    s.split('/').next().unwrap_or("").to_string()
}

/// 预设卡片第二行的地址：自定义显示「自填地址」。
fn preset_host(p: ProviderPreset) -> String {
    if p == ProviderPreset::Custom {
        "自填地址与协议".to_string()
    } else if p.base_url().is_empty() {
        p.id().to_string()
    } else {
        short_host(p.base_url())
    }
}

/// 未就绪的具体原因（详情页顶部提示条）：一次把缺的都说清，
/// 不要只给一个状态点让用户自己猜。
fn not_ready_reason(p: &ProviderEntry) -> Option<String> {
    let mut missing: Vec<&str> = Vec::new();
    if p.effective_model().trim().is_empty() {
        missing.push("没有可用模型（点「拉取模型」或手动添加）");
    }
    if p.preset.is_local() {
        if p.base_url.trim().is_empty() {
            missing.push("缺少 API 基址");
        }
    } else if p.api_key.trim().is_empty() {
        missing.push("还没填 API Key");
    }
    if missing.is_empty() {
        None
    } else {
        Some(missing.join("；"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_provider_has_no_reason() {
        let mut p = ProviderEntry::from_preset(ProviderPreset::DeepSeek);
        p.api_key = "sk-x".into();
        assert!(not_ready_reason(&p).is_none());
    }

    #[test]
    fn remote_missing_key_is_reported() {
        let p = ProviderEntry::from_preset(ProviderPreset::DeepSeek);
        let reason = not_ready_reason(&p).unwrap_or_default();
        assert!(reason.contains("API Key"), "应提示缺 Key: {reason}");
    }

    #[test]
    fn local_missing_base_url_is_reported() {
        let mut p = ProviderEntry::from_preset(ProviderPreset::Ollama);
        p.base_url.clear();
        let reason = not_ready_reason(&p).unwrap_or_default();
        assert!(reason.contains("基址"), "本机服务应提示缺地址: {reason}");
    }

    #[test]
    fn missing_model_is_reported() {
        let mut p = ProviderEntry::from_preset(ProviderPreset::Ollama);
        p.models.clear();
        p.model.clear();
        let reason = not_ready_reason(&p).unwrap_or_default();
        assert!(reason.contains("模型"), "应提示没有模型: {reason}");
    }
}
