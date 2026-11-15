//! 技能市场：分类预览、搜索、筛选、排序与安装。
//!
//! 布局对齐 `pic/Snipaste_2026-11-15_15-41-41.png`：
//! 顶部「技能 / Agent 模板」+ 搜索框 → 分类卡片行 → 筛选条 + 排序 → 卡片网格。

use gpui::prelude::*;
use gpui::{div, px, Context, FontWeight, SharedString};

use neverwork_core::{Skill, SkillCategory};
use neverwork_ui::theme;

use crate::state::{AppState, FormField, SettingsTab};
use crate::skills_catalog;
use crate::views::text_fit::fit_text;

/// 卡片最小宽度（决定每行能放几张）。
const MIN_CARD_W: f32 = 230.0;

impl AppState {
    pub(crate) fn skills_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div()
            .id("skills-scroll")
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .px_6()
            .py_5()
            .overflow_y_scroll()
            .bg(theme::surface());

        // ===== 顶部：Tab + 搜索 =====
        col = col.child(self.skills_header(cx));

        col = col.child(match self.skills_tab {
            0 => self.skills_market(cx).into_any_element(),
            _ => self.agent_templates(cx).into_any_element(),
        });

        col
    }

    fn skills_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let editing = self.active_field == Some(FormField::SkillSearch);
        let display = if editing {
            self.field_buf.text().to_string()
        } else {
            self.skill_search.clone()
        };
        let empty = display.is_empty();

        let narrow = self.layout_w.get() < 900.0;
        let mut head = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(
                div()
                    .id("skills-tab-0")
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1p5()
                    .rounded_lg()
                    .cursor_pointer()
                    .when(self.skills_tab == 0, |d| d.bg(theme::fg()))
                    .when(self.skills_tab != 0, |d| d.hover(|d| d.bg(theme::raised())))
                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                        this.set_skills_tab(0, cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(if self.skills_tab == 0 {
                                theme::bg()
                            } else {
                                theme::muted()
                            })
                            .child("✦ 技能"),
                    ),
            )
            .child(
                div()
                    .id("skills-tab-1")
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1p5()
                    .rounded_lg()
                    .cursor_pointer()
                    .when(self.skills_tab == 1, |d| d.bg(theme::fg()))
                    .when(self.skills_tab != 1, |d| d.hover(|d| d.bg(theme::raised())))
                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                        this.set_skills_tab(1, cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(if self.skills_tab == 1 {
                                theme::bg()
                            } else {
                                theme::muted()
                            })
                            .child("◈ Agent 模板"),
                    ),
            )
            .when(!narrow, |d| d.child(div().flex_1()));

        // 搜索框（窄屏时整行独占，宽度自适应）
        let search = div()
            .id("skill-search")
            .flex()
            .items_center()
            .gap_2()
            .when(narrow, |d| d.w_full())
            .when(!narrow, |d| d.w(px(280.0)))
            .h(px(32.0))
            .px_3()
            .rounded_full()
            .bg(theme::raised())
            .border_1()
            .border_color(if editing {
                theme::accent_line()
            } else {
                theme::border()
            })
            .cursor(gpui::CursorStyle::IBeam)
            .when(editing, |d| d.track_focus(&self.editor_focus))
            .on_click(cx.listener(|this, _: &gpui::ClickEvent, window, cx| {
                this.begin_field(FormField::SkillSearch, cx);
                window.focus(&this.editor_focus);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
                if this.active_field != Some(FormField::SkillSearch) {
                    return;
                }
                if ev.keystroke.key == "enter" || ev.keystroke.key == "escape" {
                    this.commit_field(cx);
                    return;
                }
                if ev.keystroke.key == "backspace" && this.field_buf.text().is_empty() {
                    this.skill_search.clear();
                    cx.notify();
                    return;
                }
                this.on_field_key(ev, cx);
                this.skill_search = this.field_buf.text().to_string();
                cx.notify();
            }))
            .child(div().text_xs().text_color(theme::dim()).child("⌕"))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(if empty { theme::dim() } else { theme::fg() })
                    .child(SharedString::from(if empty {
                        "搜索技能或技能包".to_string()
                    } else {
                        fit_text(&display, 230.0, 12.0)
                    })),
            )
            .when(!empty, |d| {
                d.child(
                    div()
                        .id("skill-search-clear")
                        .text_xs()
                        .text_color(theme::dim())
                        .cursor_pointer()
                        .hover(|d| d.text_color(theme::fg()))
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                            this.skill_search.clear();
                            this.field_buf.reset("");
                            this.active_field = None;
                            cx.notify();
                        }))
                        .child("✕"),
                )
            });

        head = head.child(search);
        head
    }

    // ==================== 技能市场 ====================

    fn skills_market(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().gap_4();

        // ---- 分类预览卡片 ----
        // 分类预览：宽屏放 5 个，窄屏按可用宽度减少，避免换行把首屏挤满。
        let all_cats = [
            SkillCategory::Office,
            SkillCategory::Knowledge,
            SkillCategory::Dev,
            SkillCategory::Content,
            SkillCategory::Design,
        ];
        let avail = self.layout_w.get() - 52.0 - 48.0;
        let show = ((avail / 240.0).floor() as usize).clamp(1, all_cats.len());
        let mut row = div().flex().flex_row().gap_3().flex_wrap();
        for cat in all_cats.into_iter().take(show) {
            row = row.child(self.category_card(cat, cx));
        }
        col = col.child(row);

        // ---- 筛选条 ----
        let mut bar = div()
            .flex()
            .items_center()
            .gap_2()
            .flex_wrap()
            .child(
                div()
                    .id("filter-all")
                    .px_3()
                    .py_1()
                    .rounded_full()
                    .text_xs()
                    .cursor_pointer()
                    .when(self.skill_filter.is_none(), |d| {
                        d.bg(theme::fg()).text_color(theme::bg())
                    })
                    .when(self.skill_filter.is_some(), |d| {
                        d.bg(theme::raised())
                            .text_color(theme::muted())
                            .hover(|d| d.bg(theme::border()))
                    })
                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                        this.set_skill_filter(None, cx);
                    }))
                    .child("全部"),
            );

        for cat in SkillCategory::all() {
            let c = *cat;
            let on = self.skill_filter == Some(c);
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("filter-{}", c.label())))
                    .px_3()
                    .py_1()
                    .rounded_full()
                    .text_xs()
                    .cursor_pointer()
                    .when(on, |d| d.bg(theme::fg()).text_color(theme::bg()))
                    .when(!on, |d| {
                        d.bg(theme::raised())
                            .text_color(theme::muted())
                            .hover(|d| d.bg(theme::border()))
                    })
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                        this.set_skill_filter(Some(c), cx);
                    }))
                    .child(SharedString::from(c.label())),
            );
        }

        // 排序
        let sort_label = match self.skill_sort {
            1 => "安装量",
            2 => "星标",
            _ => "综合评分",
        };
        bar = bar.child(
            div()
                .id("skill-sort")
                .flex()
                .items_center()
                .gap_1()
                .px_3()
                .py_1()
                .rounded_full()
                .bg(theme::raised())
                .text_xs()
                .text_color(theme::muted())
                .cursor_pointer()
                .hover(|d| d.bg(theme::border()))
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                    let next = (this.skill_sort + 1) % 3;
                    this.set_skill_sort(next, cx);
                }))
                .child("⇅ ")
                .child(SharedString::from(sort_label)),
        );
        col = col.child(bar);

        // ---- 卡片网格 ----
        let list = self.filtered_skills();
        col = col.child(
            div()
                .text_xs()
                .text_color(theme::dim())
                .child(SharedString::from(format!("共 {} 个技能", list.len()))),
        );

        if list.is_empty() {
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .py_8()
                    .text_sm()
                    .text_color(theme::dim())
                    .child("没有匹配的技能")
                    .child(
                        div()
                            .text_xs()
                            .child("换个关键词，或点「全部」看完整列表"),
                    ),
            );
            return col;
        }

        // 列数按当前可用宽度算，窗口窄时自动减少。
        let per_row = self.skill_columns();
        let mut grid = div().flex().flex_col().gap_3();
        for chunk in list.chunks(per_row) {
            let mut r = div().flex().flex_row().gap_3();
            for (skill, installed) in chunk {
                r = r.child(self.skill_card(skill, *installed, cx));
            }
            // 补齐空位，保持卡片等宽
            for _ in chunk.len()..per_row {
                r = r.child(div().flex_1().min_w(px(MIN_CARD_W)));
            }
            grid = grid.child(r);
        }
        col = col.child(grid);

        col
    }

    /// 当前宽度下每行能放几张卡片。
    pub(crate) fn skill_columns(&self) -> usize {
        let avail = self.layout_w.get() - 52.0 - 48.0; // 减图标栏与页面左右内边距
        let n = (avail / (MIN_CARD_W + 12.0)).floor() as usize;
        n.clamp(1, 4)
    }

    /// 按搜索词 + 分类过滤 + 排序后的技能列表。
    pub(crate) fn filtered_skills(&self) -> Vec<(Skill, bool)> {
        let q = self.skill_search.trim().to_lowercase();
        let installed: Vec<String> = self
            .settings
            .skills
            .iter()
            .filter(|s| s.installed)
            .map(|s| s.id.clone())
            .collect();
        let mut list = skills_catalog::catalog_with_installed(&installed);

        if let Some(cat) = self.skill_filter {
            list.retain(|(s, _)| s.category == cat);
        }
        if !q.is_empty() {
            list.retain(|(s, _)| {
                s.name.to_lowercase().contains(&q)
                    || s.id.to_lowercase().contains(&q)
                    || s.summary.to_lowercase().contains(&q)
                    || s.author.to_lowercase().contains(&q)
            });
        }
        match self.skill_sort {
            1 => list.sort_by(|a, b| b.0.installs.cmp(&a.0.installs)),
            2 => list.sort_by(|a, b| b.0.stars.cmp(&a.0.stars)),
            _ => list.sort_by(|a, b| {
                b.0.score()
                    .partial_cmp(&a.0.score())
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
        }
        list
    }

    /// 分类预览卡：分类名 + 前 4 个技能名。
    fn category_card(&self, cat: SkillCategory, cx: &mut Context<Self>) -> impl IntoElement {
        let hue = cat.hue();
        let tops = skills_catalog::top_by_category(cat, 4);

        let mut card = div()
            .id(SharedString::from(format!("cat-{}", cat.label())))
            .flex()
            .flex_col()
            .gap_2()
            .w(px(228.0))
            .flex_shrink_0()
            .p_3()
            .rounded_lg()
            .bg(theme::raised())
            .cursor_pointer()
            .hover(|d| d.bg(theme::border()))
            .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                this.set_skill_filter(Some(cat), cx);
            }))
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
                            .child(SharedString::from(cat.label())),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_0p5()
                            .children((0..3).map(|i| {
                                div()
                                    .size(px(14.0))
                                    .rounded_full()
                                    .bg(gpui::rgb(if i == 0 { hue } else { mix(hue, 0x888888, i as f32 * 0.35) }))
                            })),
                    ),
            );

        for s in tops {
            card = card.child(
                div()
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
                            .rounded_md()
                            .bg(theme::with_alpha(hue, 0.22))
                            .text_xs()
                            .text_color(gpui::rgb(hue))
                            .child(SharedString::from(s.glyph.clone())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(theme::muted())
                            .child(SharedString::from(fit_text(&s.name, 168.0, 12.0))),
                    ),
            );
        }

        card.child(
            div()
                .text_xs()
                .text_color(theme::dim())
                .child(SharedString::from(format!(
                    "{} 个技能 →",
                    skills_catalog::count_in_category(cat)
                ))),
        )
    }

    /// 技能市场里的一张卡片。
    fn skill_card(&self, skill: &Skill, installed: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let hue = skill.category.hue();
        let id = skill.id.clone();
        let id_for_btn = id.clone();
        let skill_clone = skill.clone();

        div()
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .min_w(px(MIN_CARD_W))
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .child(
                // 头：图标 + 名称 + id
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.0))
                            .flex_shrink_0()
                            .rounded_md()
                            .bg(theme::with_alpha(hue, 0.18))
                            .text_xs()
                            .text_color(gpui::rgb(hue))
                            .child(SharedString::from(skill.glyph.clone())),
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
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::fg())
                                    .child(SharedString::from(fit_text(&skill.name, 170.0, 14.0))),
                            )
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_xs()
                                    .text_color(theme::dim())
                                    .child(SharedString::from(fit_text(
                                        &format!("skillhub.{}", skill.id),
                                        170.0,
                                        11.0,
                                    ))),
                            ),
                    ),
            )
            // 摘要（固定两行高度，卡片等高）
            .child(
                div()
                    .h(px(34.0))
                    .overflow_hidden()
                    .text_xs()
                    .text_color(theme::muted())
                    .child(SharedString::from(fit_text(&skill.summary, 250.0, 12.0))),
            )
            .child(
                // 底：分类 + 安装量 + 星标 + 安装按钮
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .px_1p5()
                            .py_0p5()
                            .rounded_sm()
                            .bg(theme::with_alpha(hue, 0.14))
                            .text_xs()
                            .text_color(gpui::rgb(hue))
                            .child(SharedString::from(skill.category.label())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .child(SharedString::from(format!("↓ {}", skill.installs_label()))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .child(SharedString::from(format!("☆ {}", skill.stars_label()))),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id(SharedString::from(format!("install-{id_for_btn}")))
                            .px_2()
                            .py_0p5()
                            .rounded_md()
                            .text_xs()
                            .cursor_pointer()
                            .when(installed, |d| {
                                d.bg(theme::with_alpha(0x22c55e, 0.14))
                                    .text_color(theme::green())
                                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                                        this.uninstall_skill(&id, cx);
                                    }))
                            })
                            .when(!installed, |d| {
                                d.bg(theme::accent())
                                    .text_color(gpui::rgb(0xffffff))
                                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                                        this.install_skill(skill_clone.clone(), cx);
                                    }))
                            })
                            .child(if installed { "已安装" } else { "安装" }),
                    ),
            )
    }

    // ==================== Agent 模板 ====================

    fn agent_templates(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().gap_3();

        col = col.child(
            div()
                .text_xs()
                .text_color(theme::dim())
                .child("模板是预置的团队编排，套用后会成为「当前团队」。之后的增删角色在团队编辑器里做。"),
        );

        let templates: [(&str, &str, &str, &str); 3] = [
            (
                "流水线三人组",
                "⊞",
                "Planner 拆解 → Coder 实现 → Reviewer 审查。适合有明确交付物的改造任务。",
                "pipeline",
            ),
            (
                "并行探索团",
                "⋮⋮",
                "三个角色同时从不同角度给方案，最后由你挑选。适合技术选型与方案评审。",
                "parallel",
            ),
            (
                "单人对练",
                "◐",
                "一个角色走完全程，回答更集中、token 更省。适合问答与代码片段生成。",
                "solo",
            ),
        ];

        for (name, glyph, desc, kind) in templates {
            let kind = kind.to_string();
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .flex_shrink_0()
                            .rounded_lg()
                            .bg(theme::accent_soft())
                            .text_sm()
                            .text_color(theme::accent())
                            .child(SharedString::from(glyph)),
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
                                    .child(SharedString::from(name)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::muted())
                                    .child(SharedString::from(fit_text(desc, 520.0, 12.0))),
                            ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("apply-{name}")))
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(theme::accent())
                            .text_xs()
                            .text_color(gpui::rgb(0xffffff))
                            .cursor_pointer()
                            .hover(|d| d.opacity(0.88))
                            .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                                this.apply_template(&kind, cx);
                            }))
                            .child("套用"),
                    ),
            );
        }

        col = col.child(
            div()
                .flex()
                .gap_2()
                .child(
                    div()
                        .id("goto-team")
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(theme::raised())
                        .text_xs()
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::border()))
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                            this.open_settings(SettingsTab::Skills, cx);
                            this.set_settings_tab(SettingsTab::Skills, cx);
                        }))
                        .child("查看已安装技能 →"),
                ),
        );

        col
    }

    /// 套用模板：替换当前团队的编排与角色。
    pub(crate) fn apply_template(&mut self, kind: &str, cx: &mut Context<Self>) {
        use neverwork_core::{FlowMode, Team};

        let team = match kind {
            "pipeline" => Team {
                id: "tpl-pipeline".into(),
                name: "流水线三人组".into(),
                flow: FlowMode::Pipeline,
                roles: vec![
                    role("planner", "Planner", "规划者", "P", 0, "负责拆解任务、制定步骤与验收标准。"),
                    role("coder", "Coder", "编码", "C", 1, "负责实现代码改动，产出可运行的补丁。"),
                    role("reviewer", "Reviewer", "审查者", "R", 2, "负责审查正确性、边界与性能，指出风险。"),
                ],
            },
            "parallel" => Team {
                id: "tpl-parallel".into(),
                name: "并行探索团".into(),
                flow: FlowMode::Parallel,
                roles: vec![
                    role("a", "方案 A", "探索", "A", 0, "从最保守的角度给出可行方案。"),
                    role("b", "方案 B", "探索", "B", 1, "从最激进/最优性能的角度给出方案。"),
                    role("c", "评判", "决策", "J", 2, "对比其余方案的取舍，给出推荐与理由。"),
                ],
            },
            _ => Team {
                id: "tpl-solo".into(),
                name: "单人对练".into(),
                flow: FlowMode::Pipeline,
                roles: vec![role("solo", "Assistant", "助手", "A", 0, "直接、准确地回答问题，必要时给出代码。")],
            },
        };

        self.config.teams.push(team.clone());
        self.config.active_team_id = team.id.clone();
        let name = team.name.clone();
        match neverwork_core::config::save(&self.config) {
            Ok(()) => self.action_msg = Some(format!("已套用模板「{name}」")),
            Err(e) => self.action_msg = Some(format!("保存失败：{e}")),
        }
        self.center = crate::state::CenterView::Chat;
        cx.notify();
    }
}

fn role(id: &str, name: &str, kind: &str, initial: &str, color: usize, prompt: &str) -> neverwork_core::Role {
    let mut r = neverwork_core::Role::new(id, name, initial);
    r.kind_label = kind.into();
    r.color_index = color;
    r.model = "gpt-4o".into();
    r.prompt = prompt.into();
    r
}

/// 两个颜色按比例混合（用于分类卡片的三个色点）。
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let f = |shift: u32| -> u32 {
        let ca = (a >> shift) & 0xff;
        let cb = (b >> shift) & 0xff;
        ((ca as f32 * (1.0 - t) + cb as f32 * t) as u32) & 0xff
    };
    (f(16) << 16) | (f(8) << 8) | f(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_mix_ends() {
        assert_eq!(mix(0xff0000, 0x00ff00, 0.0), 0xff0000);
        assert_eq!(mix(0xff0000, 0x00ff00, 1.0), 0x00ff00);
        let mid = mix(0x000000, 0xffffff, 0.5);
        assert!(mid > 0x7a7a7a && mid < 0x868686, "中间值应在灰附近: {mid:x}");
    }
}
