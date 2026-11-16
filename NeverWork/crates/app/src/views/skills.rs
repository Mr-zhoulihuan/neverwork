//! 技能市场：分类预览、搜索、筛选、排序与安装。
//!
//! 布局对齐 `pic/Snipaste_2026-11-15_15-41-41.png`：
//! 顶部「技能 / Agent 模板」+ 搜索框 → 分类卡片行 → 筛选条 + 排序 → 卡片网格。

use gpui::prelude::*;
use gpui::{div, px, Context, FontWeight, SharedString};

use neverwork_ui::theme;

use crate::marketplace::{MarketEntry, MarketSource};
use crate::state::{AppState, FormField, SettingsTab};
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

        // 导入 .md：真实安装的另一个入口（解析 front-matter 后落盘）
        if self.skill_import_open && self.skills_tab == 0 {
            col = col.child(self.skill_import_row(cx));
        }

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

        // 导入按钮：展开 .md 路径输入（技能要真实落盘，不只是市场里点安装）
        head = head.child(
            div()
                .id("skill-import-toggle")
                .px_3()
                .py_1()
                .rounded_full()
                .border_1()
                .border_color(if self.skill_import_open {
                    theme::accent_line()
                } else {
                    theme::border()
                })
                .text_xs()
                .text_color(theme::accent())
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()))
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, window, cx| {
                    if this.skill_import_open {
                        this.skill_import_open = false;
                        this.active_field = None;
                        this.field_buf.reset("");
                    } else {
                        this.skill_import_open = true;
                        this.import_path.clear();
                        this.begin_field(FormField::ImportPath, cx);
                        window.focus(&this.editor_focus);
                    }
                    cx.notify();
                }))
                .child("⇪ 导入 SKILL.md"),
        );
        head
    }

    /// 导入技能的路径输入行（只接受 `.md`；JSON 配置在设置页导入）。
    fn skill_import_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let editing = self.active_field == Some(FormField::ImportPath);
        let display = if editing {
            self.field_buf.text().to_string()
        } else {
            self.import_path.clone()
        };
        let empty = display.is_empty();

        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("skill-import-path")
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(px(34.0))
                    .px_3()
                    .rounded_lg()
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
                        this.begin_field(FormField::ImportPath, cx);
                        window.focus(&this.editor_focus);
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
                        if this.active_field != Some(FormField::ImportPath) {
                            return;
                        }
                        if ev.keystroke.key == "escape" {
                            this.skill_import_open = false;
                            this.active_field = None;
                            this.field_buf.reset("");
                            cx.notify();
                            return;
                        }
                        if ev.keystroke.key == "enter" {
                            this.commit_field(cx);
                            this.import_skill_from_path(cx);
                            return;
                        }
                        this.on_field_key(ev, cx);
                        this.import_path = this.field_buf.text().to_string();
                        cx.notify();
                    }))
                    .child(div().text_xs().text_color(theme::dim()).child("⇪"))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(if empty { theme::dim() } else { theme::fg() })
                            .child(SharedString::from(if empty {
                                "C:\\path\\to\\skill.md（front-matter 可带 id/name/summary/category）"
                                    .to_string()
                            } else {
                                display
                            })),
                    ),
            )
            .child(
                div()
                    .id("skill-import-run")
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(theme::accent())
                    .text_xs()
                    .text_color(gpui::rgb(0xffffff))
                    .cursor_pointer()
                    .hover(|d| d.opacity(0.88))
                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                        this.import_skill_from_path(cx);
                    }))
                    .child("导入"),
            )
            .child(
                div()
                    .id("skill-import-cancel")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_xs()
                    .text_color(theme::muted())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::raised()))
                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                        this.skill_import_open = false;
                        this.active_field = None;
                        this.field_buf.reset("");
                        cx.notify();
                    }))
                    .child("取消"),
            )
    }

    // ==================== 技能市场 ====================

    fn skills_market(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().gap_4();

        // ---- 工具条：来源 + 同步状态 + 刷新 + 镜像（一行） ----
        col = col.child(self.market_bar(cx));
        // ---- 真实标签筛选 + 排序（一行） ----
        col = col.child(self.market_filters(cx));

        // ---- 卡片网格 ----
        let list = self.filtered_entries();
        if list.is_empty() {
            let hint = if self.market_loading {
                "正在从远端拉取…"
            } else if self.market_error.is_some() {
                "拉取失败，且本地没有缓存"
            } else if self.skill_search.trim().is_empty() && self.market_tag.is_none() {
                "该来源暂无条目，点「⟳」刷新重试"
            } else {
                "换个关键词，或点「全部」看完整列表"
            };
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .py_8()
                    .text_sm()
                    .text_color(theme::dim())
                    .child("没有匹配的条目")
                    .child(div().text_xs().child(hint)),
            );
            return col;
        }

        // 列数按当前可用宽度算，窗口窄时自动减少。
        let per_row = self.skill_columns();
        let mut grid = div().flex().flex_col().gap_3();
        for chunk in list.chunks(per_row) {
            let mut r = div().flex().flex_row().gap_3();
            for (entry, installed) in chunk {
                r = r.child(self.skill_card(entry, *installed, cx));
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

    /// 市场工具条：来源切换 + 同步状态 + 刷新 + 镜像（一行）。
    fn market_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div().flex().items_center().gap_1().flex_wrap();

        for src in MarketSource::all() {
            let on = self.market_source == src;
            row = row.child(
                div()
                    .id(SharedString::from(format!("market-src-{}", src.key())))
                    .px_3()
                    .py_1()
                    .rounded_full()
                    .text_xs()
                    .cursor_pointer()
                    .when(on, |d| d.bg(theme::fg()).text_color(theme::bg()))
                    .when(!on, |d| {
                        d.text_color(theme::muted())
                            .hover(|d| d.bg(theme::raised()))
                    })
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                        this.set_market_source(src, cx);
                    }))
                    .child(SharedString::from(src.label())),
            );
        }

        // 真实来源 + 上次同步时间：数据不是编的，直接标出来
        let (status, color) = if self.market_loading {
            ("⟳ 正在拉取…".to_string(), theme::dim())
        } else if let Some(err) = &self.market_error {
            (
                fit_text(&format!("拉取失败：{err}（显示缓存）"), 320.0, 11.0),
                theme::accent(),
            )
        } else if let Some(at) = self.market_fetched_at.get(&self.market_source).copied() {
            (
                format!(
                    "{} · {}",
                    self.market_source.origin(),
                    crate::marketplace::ago_label(at)
                ),
                theme::dim(),
            )
        } else {
            ("尚未同步".to_string(), theme::dim())
        };
        row = row.child(
            div()
                .px_2()
                .text_xs()
                .text_color(color)
                .child(SharedString::from(status)),
        );

        row = row.child(div().flex_1());

        let loading = self.market_loading;
        row = row.child(
            div()
                .id("market-refresh")
                .flex()
                .items_center()
                .justify_center()
                .size(px(26.0))
                .rounded_md()
                .text_sm()
                .cursor_pointer()
                .when(loading, |d| d.text_color(theme::dim()))
                .when(!loading, |d| {
                    d.text_color(theme::muted())
                        .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                })
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                    this.refresh_market(cx);
                }))
                .child("⟳"),
        );

        // 镜像开关（MCP 安装依赖走国内源，与设置页同一个开关）
        let mirror = self.settings.mcp_mirror;
        row = row.child(
            div()
                .id("market-mirror")
                .px_2()
                .py_1()
                .rounded_md()
                .text_xs()
                .cursor_pointer()
                .when(mirror, |d| {
                    d.text_color(theme::accent()).bg(theme::accent_soft())
                })
                .when(!mirror, |d| {
                    d.text_color(theme::dim()).hover(|d| d.bg(theme::raised()))
                })
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                    this.toggle_mcp_mirror(cx);
                }))
                .child(if mirror { "镜像 开" } else { "镜像 关" }),
        );

        row
    }

    /// 当前来源真实存在的标签 + 数量（腾讯云 = 它自己的分类名，官方 = 包形态）。
    pub(crate) fn market_tag_counts(&self) -> Vec<(String, usize)> {
        let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for e in self.current_market_items() {
            for t in &e.tags {
                *counts.entry(t.clone()).or_default() += 1;
            }
        }
        let mut v: Vec<(String, usize)> = counts.into_iter().collect();
        // 数量降序；「仅远程」是负向标签，永远排最后
        v.sort_by(|a, b| {
            let rank = |t: &str| usize::from(t == "仅远程");
            rank(&a.0)
                .cmp(&rank(&b.0))
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.0.cmp(&b.0))
        });
        v
    }

    /// 真实标签筛选条 + 排序（不硬编码任何分类表）。
    fn market_filters(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div().flex().items_center().gap_1p5().flex_wrap();
        let total = self.current_market_items().len();

        let all_on = self.market_tag.is_none();
        row = row.child(
            div()
                .id("market-tag-all")
                .flex()
                .items_center()
                .gap_1()
                .px_2p5()
                .py_1()
                .rounded_full()
                .text_xs()
                .cursor_pointer()
                .when(all_on, |d| d.bg(theme::fg()).text_color(theme::bg()))
                .when(!all_on, |d| {
                    d.bg(theme::raised())
                        .text_color(theme::muted())
                        .hover(|d| d.bg(theme::border()))
                })
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                    this.set_market_tag(None, cx);
                }))
                .child("全部")
                .child(
                    div()
                        .opacity(0.55)
                        .child(SharedString::from(total.to_string())),
                ),
        );

        for (tag, count) in self.market_tag_counts() {
            let on = self.market_tag.as_deref() == Some(tag.as_str());
            let t = tag.clone();
            row = row.child(
                div()
                    .id(SharedString::from(format!("market-tag-{tag}")))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2p5()
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
                        // 再点选中的标签 = 取消筛选
                        let next = if this.market_tag.as_deref() == Some(t.as_str()) {
                            None
                        } else {
                            Some(t.clone())
                        };
                        this.set_market_tag(next, cx);
                    }))
                    .child(SharedString::from(fit_text(&tag, 130.0, 12.0)))
                    .child(
                        div()
                            .opacity(0.55)
                            .child(SharedString::from(count.to_string())),
                    ),
            );
        }

        row = row.child(div().flex_1());

        let sort_label = match self.skill_sort {
            1 => "名称",
            2 => "热度",
            _ => "推荐",
        };
        row = row.child(
            div()
                .id("skill-sort")
                .flex()
                .items_center()
                .gap_1()
                .px_2p5()
                .py_1()
                .rounded_full()
                .text_xs()
                .text_color(theme::dim())
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()))
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                    let next = (this.skill_sort + 1) % 3;
                    this.set_skill_sort(next, cx);
                }))
                .child("⇅ ")
                .child(SharedString::from(sort_label)),
        );
        row
    }

    /// 按搜索词 + 真实标签过滤 + 排序后的市场条目（含安装态）。
    pub(crate) fn filtered_entries(&self) -> Vec<(MarketEntry, bool)> {
        let q = self.skill_search.trim().to_lowercase();
        let mut list: Vec<(MarketEntry, bool)> = self
            .current_market_items()
            .into_iter()
            .map(|e| {
                let installed = self.is_skill_installed(&e.skill.id);
                (e, installed)
            })
            .collect();

        if let Some(tag) = &self.market_tag {
            list.retain(|(e, _)| e.tags.iter().any(|t| t == tag));
        }
        if !q.is_empty() {
            list.retain(|(e, _)| {
                let s = &e.skill;
                s.name.to_lowercase().contains(&q)
                    || s.id.to_lowercase().contains(&q)
                    || s.summary.to_lowercase().contains(&q)
                    || s.author.to_lowercase().contains(&q)
            });
        }
        match self.skill_sort {
            // 名称：A→Z
            1 => list.sort_by(|a, b| {
                a.0.skill
                    .name
                    .to_lowercase()
                    .cmp(&b.0.skill.name.to_lowercase())
            }),
            // 热度：腾讯云阅读量 / 安装量
            2 => list.sort_by(|a, b| b.0.skill.installs.cmp(&a.0.skill.installs)),
            // 推荐：可安装的排前面（官方 registry 大多只有远程端点），再按评分与名称
            _ => list.sort_by(|a, b| {
                a.0.remote_only
                    .cmp(&b.0.remote_only)
                    .then_with(|| {
                        b.0.skill
                            .score()
                            .partial_cmp(&a.0.skill.score())
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| a.0.skill.name.cmp(&b.0.skill.name))
            }),
        }
        list
    }

    /// 技能市场里的一张卡片（市场条目驱动，安装走 [`AppState::install_market_entry`]）。
    fn skill_card(
        &self,
        entry: &MarketEntry,
        installed: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let skill = &entry.skill;
        // 图标底色按来源（腾讯云蓝 / 官方紫 / GitHub 灰），不再用猜出来的分类色
        let hue = self.market_source.hue();
        let id = skill.id.clone();
        let id_for_btn = id.clone();
        let entry_for_btn = entry.clone();
        let mcp = skill.is_mcp();
        let remote_only = entry.remote_only;
        let busy_here = self.market_installing.as_deref() == Some(skill.id.as_str());
        let pending_here = self.pending_mcp_install.as_deref() == Some(skill.id.as_str());
        let mcp_cmd = skill
            .mcp
            .as_ref()
            .map(|m| format!("{} {}", m.command, m.args.join(" ")))
            .unwrap_or_default();
        // 摘要按两行预算截断（卡片宽约 260~300px，两行 ≈ 470px）
        let summary = fit_text(&skill.summary, 470.0, 12.0);
        let installs = (skill.installs > 0).then(|| skill.installs_label());

        // 标签行：形态 + 仅远程 + 前两个真实来源标签（固定高度，卡片等高）
        let mut tag_row = div()
            .flex()
            .items_center()
            .gap_1()
            .h(px(20.0))
            .overflow_hidden()
            .child(
                div()
                    .px_1()
                    .rounded_sm()
                    .bg(if mcp { theme::accent_soft() } else { theme::raised() })
                    .text_xs()
                    .text_color(if mcp { theme::accent() } else { theme::dim() })
                    .child(if mcp { "MCP" } else { "文本" }),
            );
        if remote_only {
            tag_row = tag_row.child(
                div()
                    .px_1()
                    .rounded_sm()
                    .bg(theme::raised())
                    .text_xs()
                    .text_color(theme::dim())
                    .child("仅远程"),
            );
        }
        // 「仅远程」已有独立徽标，作者行已显示 owner/repo，这里去重后再取前两个
        for t in entry
            .tags
            .iter()
            .filter(|t| t.as_str() != "仅远程" && t.as_str() != skill.author)
            .take(2)
        {
            tag_row = tag_row.child(
                div()
                    .px_1()
                    .rounded_sm()
                    .bg(theme::raised())
                    .text_xs()
                    .text_color(theme::muted())
                    .child(SharedString::from(fit_text(t, 96.0, 12.0))),
            );
        }

        // 头部：图标 + 名称 + 作者，阅读量靠右
        let mut head = div()
            .flex()
            .items_start()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.0))
                    .flex_shrink_0()
                    .rounded_md()
                    .bg(theme::with_alpha(hue, 0.16))
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
                            .child(SharedString::from(fit_text(&skill.name, 165.0, 14.0))),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(theme::dim())
                            .child(SharedString::from(fit_text(&skill.author, 175.0, 11.0))),
                    ),
            );
        if let Some(n) = installs {
            head = head.child(
                div()
                    .text_xs()
                    .text_color(theme::dim())
                    .child(SharedString::from(n)),
            );
        }

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
            .child(head)
            // 摘要：两行高度（行高固定 16px，否则文字会被压扁），带省略号
            .child(
                div()
                    .h(px(34.0))
                    .line_height(px(16.0))
                    .overflow_hidden()
                    .text_xs()
                    .text_color(theme::muted())
                    .child(SharedString::from(summary)),
            )
            .child(tag_row)
            .child(
                // 底行：安装按钮右对齐
                div()
                    .flex()
                    .items_center()
                    .child(div().flex_1())
                    .child(
                        div()
                            .id(SharedString::from(format!("install-{id_for_btn}")))
                            .flex()
                            .items_center()
                            .justify_center()
                            .min_w(px(66.0))
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .text_xs()
                            .when(installed, |d| {
                                d.bg(theme::with_alpha(0x22c55e, 0.12))
                                    .text_color(theme::green())
                                    .cursor_pointer()
                                    .hover(|d| d.bg(theme::with_alpha(0x22c55e, 0.2)))
                                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                                        this.uninstall_skill(&id, cx);
                                    }))
                            })
                            .when(!installed && !busy_here, |d| {
                                d.when(remote_only, |d| {
                                    d.border_1()
                                        .border_color(theme::border())
                                        .text_color(theme::dim())
                                        .hover(|d| d.bg(theme::raised()))
                                })
                                .when(!remote_only, |d| {
                                    d.bg(theme::accent())
                                        .text_color(gpui::rgb(0xffffff))
                                        .hover(|d| d.opacity(0.88))
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(
                                    move |this, _: &gpui::ClickEvent, _w, cx| {
                                        this.install_market_entry(entry_for_btn.clone(), cx);
                                    },
                                ))
                            })
                            .when(!installed && busy_here, |d| {
                                d.bg(theme::raised()).text_color(theme::muted())
                            })
                            .child(if installed {
                                "✓ 已安装"
                            } else if busy_here {
                                "获取配置…"
                            } else if remote_only {
                                "仅远程"
                            } else if mcp {
                                "安装 MCP"
                            } else {
                                "安装"
                            }),
                    ),
            )
            // MCP 插件安装确认：要注册外部进程，必须用户点头
            .when(pending_here, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_2()
                        .rounded_md()
                        .bg(theme::accent_soft())
                        .border_1()
                        .border_color(theme::accent_line())
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::fg())
                                .child(SharedString::from(format!(
                                    "将注册外部进程：{mcp_cmd}（工具调用逐次审批）"
                                ))),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .justify_end()
                                .child(
                                    div()
                                        .id(SharedString::from(format!("mcp-cancel-{id_for_btn}")))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_md()
                                        .text_xs()
                                        .text_color(theme::muted())
                                        .cursor_pointer()
                                        .hover(|d| d.bg(theme::raised()))
                                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                                            this.cancel_mcp_install(cx);
                                        }))
                                        .child("取消"),
                                )
                                .child(
                                    div()
                                        .id(SharedString::from(format!("mcp-confirm-{id_for_btn}")))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_md()
                                        .bg(theme::accent())
                                        .text_xs()
                                        .text_color(gpui::rgb(0xffffff))
                                        .cursor_pointer()
                                        .hover(|d| d.opacity(0.88))
                                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                                            this.confirm_mcp_install(cx);
                                        }))
                                        .child("确认安装"),
                                ),
                        ),
                )
            })
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

/// 两个颜色按比例混合（分类卡片的色点曾用它；保留给后续标签配色）。
#[cfg(test)]
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
