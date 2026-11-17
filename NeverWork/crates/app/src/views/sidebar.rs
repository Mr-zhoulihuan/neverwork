//! 左侧栏：窄图标导航栏 + 会话卡片列表。
//!
//! 参考 `pic/` 下的成品界面重做（见 `design/DESIGN-SPEC.md`）：
//! - 最左是一条 48px 的图标栏，只放全局入口，底部是 AI 状态与用户头像；
//! - 右边是会话列表，每条会话是一张双行卡片：标题 + 摘要 + 时间 + 状态胶囊。
//!
//! 与旧实现的差别：旧版本是一张大工作区卡片 + 几行纯文字会话行，
//! 信息密度低、点击区域小、状态不明显；现在把「工作区」压成一条可点击的细条，
//! 把空间让给会话卡片。

use gpui::prelude::*;
use gpui::{div, px, relative, ClickEvent, Context, Rgba, SharedString};

use neverwork_core::{SessionAction, SessionStatus};
use neverwork_ui::theme;

use crate::state::{AppState, CenterView, InputTarget};

/// 图标栏宽度。
pub(crate) const RAIL_W: f32 = 52.0;
/// 会话列表宽度。
pub(crate) const SESSION_W: f32 = 264.0;
/// 列表容器的左右内边距（list 上的 px_2）。
const LIST_PAD: f32 = 8.0;
/// 卡片自身的左右内边距（px(CARD_PAD)）。
const CARD_PAD: f32 = 8.0;
/// 卡片可用内容宽度 = 列表宽 - 两侧 list 内边距 - 两侧卡片内边距。
const CARD_INNER: f32 = SESSION_W - LIST_PAD * 2.0 - CARD_PAD * 2.0;

/// 图标栏上的入口。
#[derive(Clone, Copy, PartialEq, Eq)]
enum RailAction {
    Chat,
    Team,
    Skills,
    Settings,
}

impl AppState {
    /// `hide_list` 为真时只保留图标栏（极窄窗口 / 全页视图）。
    /// 会话列表可手动收起成 16px 窄条（展开按钮），此时图标栏照常显示。
    pub(crate) fn left_sidebar(&self, hide_list: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let row = div()
            .flex()
            .flex_row()
            .flex_shrink_0()
            .h_full()
            .child(self.icon_rail(cx));
        if hide_list {
            row
        } else if self.session_collapsed {
            row.child(self.session_panel_collapsed(cx))
        } else {
            row.child(self.session_panel(cx))
        }
    }

    // ==================== 图标栏 ====================

    fn icon_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_chat = self.center == CenterView::Chat;
        let is_team = self.center == CenterView::Team;
        let is_skills = self.center == CenterView::Skills;
        let is_settings = self.center == CenterView::Settings;

        let mut rail = div()
            .flex()
            .flex_col()
            .items_center()
            .w(px(RAIL_W))
            .flex_shrink_0()
            .h_full()
            .pt_3()
            .pb_2()
            .gap_3()
            .bg(theme::bg())
            .border_r_1()
            .border_color(theme::border());

        // 品牌标：参考稿是细栏顶部一枚绿色 logo
        rail = rail.child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(30.0))
                .rounded_xl()
                .bg(theme::green())
                .mb_3()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(gpui::rgb(0xffffff))
                .child("N"),
        );

        // 主图标组放在导航栏的垂直中段：上下各一段弹性空白，
        // 不贴着 logo，也不会被底部入口拉起。
        rail = rail
            .child(div().flex_1())
            .child(self.rail_icon_button(
                "rail-chat",
                "icons/rail-chat.svg",
                is_chat,
                RailAction::Chat,
                cx,
            ))
            .child(self.rail_icon_button(
                "rail-team",
                "icons/rail-team.svg",
                is_team,
                RailAction::Team,
                cx,
            ))
            .child(self.rail_icon_button(
                "rail-skills",
                "icons/rail-skills.svg",
                is_skills,
                RailAction::Skills,
                cx,
            ))
            .child(div().flex_1());

        // 底部固定入口：只剩设置（命令面板走会话面板「⋯」）
        rail = rail.child(self.rail_button(
            "rail-settings",
            "⚙",
            is_settings,
            RailAction::Settings,
            cx,
        ));

        rail
    }

    /// 图标栏矢量按钮：40px 方形，选中时亮底，图标悬停变亮。
    ///
    /// 图标走 gpui 资源系统（[`crate::assets::EmbeddedAssets`] 内嵌 SVG），
    /// 渲染时只取蒙版，颜色由 `text_color` 决定——深浅主题自动适配。
    fn rail_icon_button(
        &self,
        id: &'static str,
        icon: &'static str,
        active: bool,
        action: RailAction,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .size(px(40.0))
            .rounded_xl()
            .cursor_pointer()
            .when(active, |d| d.bg(theme::raised()))
            .when(!active, |d| d.hover(|d| d.bg(theme::panel())))
            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                this.run_rail(action, cx);
            }))
            .child(
                gpui::svg()
                    .path(icon)
                    .size(px(20.0))
                    .text_color(if active { theme::fg() } else { theme::muted() })
                    .when(!active, |s| s.hover(|s| s.text_color(theme::fg()))),
            )
    }

    /// 图标栏字符按钮：仅设置入口还在用（⚙）。
    fn rail_button(
        &self,
        id: &'static str,
        glyph: &'static str,
        active: bool,
        action: RailAction,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .size(px(40.0))
            // 符号字体（YaHei / Segoe UI Symbol）的墨迹重心比行盒中心低 ≈ 2.5px，
            // 不补偿时图标看起来偏下；pb 把行盒整体上提，让字形几何居中。
            .pb(px(5.0))
            .rounded_xl()
            .text_size(px(18.0))
            .cursor_pointer()
            .when(active, |d| d.bg(theme::raised()).text_color(theme::fg()))
            .when(!active, |d| {
                d.text_color(theme::muted()).hover(|d| {
                    d.bg(theme::panel()).text_color(theme::fg())
                })
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                this.run_rail(action, cx);
            }))
            .child(SharedString::from(glyph))
    }

    fn run_rail(&mut self, action: RailAction, cx: &mut Context<Self>) {
        match action {
            RailAction::Chat => self.show_chat(cx),
            RailAction::Team => self.set_center(CenterView::Team, cx),
            RailAction::Skills => self.open_skills(cx),
            RailAction::Settings => {
                self.open_settings(crate::state::SettingsTab::General, cx)
            }
        }
    }

    // ==================== 会话列表 ====================

    /// 收起态：只留一条 16px 窄边（对齐右栏的折叠条），点一下重新展开。
    /// 展开按钮垂直居中，与图标栏主图标组同一高度。
    fn session_panel_collapsed(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .w(px(16.0))
            .flex_shrink_0()
            .h_full()
            .bg(theme::bg())
            .border_r_1()
            .border_color(theme::border())
            .child(
                div()
                    .id("expand-session")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(16.0))
                    .h(px(64.0))
                    .rounded_lg()
                    .bg(theme::raised())
                    .border_1()
                    .border_color(theme::border())
                    .text_size(px(16.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(theme::fg())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::accent()).text_color(gpui::rgb(0xffffff)))
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.toggle_session_list(cx);
                    }))
                    .child("»"),
            )
    }

    fn session_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut panel = div()
            .relative()
            .flex()
            .flex_col()
            .w(px(SESSION_W))
            .flex_shrink_0()
            .h_full()
            .bg(theme::bg());

        // ---- 头部：标题 + 新建 ----
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .pt_3()
                .pb_2()
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(theme::fg())
                        .child("对话"),
                )
                .child(
                    div()
                        .id("new-session")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(28.0))
                        .rounded_lg()
                        .bg(theme::raised())
                        .border_1()
                        .border_color(theme::border())
                        .text_sm()
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::border()).text_color(theme::fg()))
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            // 先选模式（单助手 / 团队），选定后不可切换
                            this.open_new_session_picker(cx);
                        }))
                        .child("＋"),
                ),
        );

        // ---- 搜索（标题/摘要/副标题，不扫正文） ----
        panel = panel.child(self.session_search_row(cx));

        // ---- 工作区细条（点击打开工作区管理器） ----
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .mx_2()
                .mb_1()
                .px_2()
                .py_1()
                .rounded_md()
                .id("workspace-strip")
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()))
                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                    this.open_workspace_prompt(cx);
                }))
                .child(
                    div()
                        .text_xs()
                        .text_color(if self.remote.is_some() {
                            theme::yellow()
                        } else {
                            theme::green()
                        })
                        .child(if self.remote.is_some() { "⇅" } else { "▤" }),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(theme::muted())
                        .child(SharedString::from(crate::views::text_fit::fit_text(
                            self.workspace
                                .as_ref()
                                .map(|w| w.name.as_str())
                                .unwrap_or("未选择工作区"),
                            120.0,
                            12.0,
                        ))),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::dim())
                        .child(SharedString::from(format!("{} 项", self.files.len()))),
                )
                .child(div().text_xs().text_color(theme::accent()).child("管理")),
        );

        // ---- 会话卡片列表 ----
        let mut list = div()
            .id("session-list")
            .flex()
            .flex_col()
            .gap_1()
            .px(px(LIST_PAD))
            .pb_2()
            .flex_1()
            .min_h(px(0.0))
            .overflow_scroll();

        // 按「最近活跃优先」排序，最新在最前；筛选只影响显示，不改数据顺序
        let query = self.session_search.text().to_string();
        // 搜索激活时不收起归档：用户是在找东西，不能把命中结果藏起来
        let filter_active = !query.trim().is_empty();
        let mut archived: Vec<usize> = Vec::new();
        let mut pinned_done = false;
        let mut normal_started = false;
        for idx in self.ordered_sessions() {
            let s = &self.sessions[idx];
            if !neverwork_core::session_matches_list(s, &query, None, None) {
                continue;
            }
            // 当前打开的会话不进归档组：否则点一下就“消失”了
            if !filter_active && idx != self.active_session && neverwork_core::is_archived_session(s)
            {
                archived.push(idx);
                continue;
            }
            // 置顶区结束后插一条分隔，视觉上把两段分开
            if !s.pinned && !normal_started {
                normal_started = true;
                if pinned_done {
                    list = list.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .child(
                                div()
                                    .flex_1()
                                    .h(px(1.0))
                                    .bg(theme::border()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::dim())
                                    .child("最近"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .h(px(1.0))
                                    .bg(theme::border()),
                            ),
                    );
                }
            }
            if s.pinned {
                pinned_done = true;
            }
            list = list.child(self.session_card(idx, cx));
        }

        // 命中为空的提示：搜索/筛选后一片空白最容易让人以为会话丢了
        let empty_hit = !pinned_done && !normal_started && archived.is_empty();
        if empty_hit {
            list = list.child(
                div()
                    .px_3()
                    .py_4()
                    .text_xs()
                    .text_color(theme::dim())
                    .child(if filter_active {
                        "没有匹配的会话"
                    } else {
                        "还没有会话，点右上角 ＋ 开始"
                    }),
            );
        }

        // ---- 归档折叠组（完成/废弃且未置顶） ----
        if !archived.is_empty() {
            let count = archived.len();
            let confirm = self.archive_clean_confirm;
            let head = div()
                .id("archive-head")
                .flex()
                .items_center()
                .gap_1p5()
                .px_2()
                .py_1()
                .mt_1()
                .rounded_md()
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()))
                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                    this.toggle_archive(cx);
                }))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::dim())
                        .child(if self.archive_open { "▾" } else { "▸" }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::dim())
                        .child(SharedString::from(format!("已归档 {count}"))),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("archive-clean")
                        .px_1()
                        .rounded_sm()
                        .text_xs()
                        .text_color(if confirm { theme::red() } else { theme::dim() })
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::border()).text_color(theme::fg()))
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            cx.stop_propagation();
                            this.click_clean_archived(cx);
                        }))
                        .child(if confirm { "再点确认清理" } else { "清理" }),
                );
            list = list.child(head);
            if self.archive_open {
                for idx in archived {
                    list = list.child(self.session_card(idx, cx));
                }
            }
        }
        panel = panel.child(list);

        // ---- 底部：会话计数 + 更多菜单 ----
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_3()
                .py_1p5()
                .text_xs()
                .text_color(theme::dim())
                .child(SharedString::from(format!("{} 个会话", self.sessions.len())))
                .child(
                    div()
                        .id("session-more")
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px(22.0))
                        .px_2()
                        .rounded_md()
                        .text_sm()
                        .text_color(theme::dim())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.toggle_palette(cx);
                        }))
                        .child("⋯"),
                ),
        );

        // ---- 右侧边中部的收起把手 ----
        // 绝对定位贴在右边缘：会话区右侧 16px 恰好是列表与卡片的内边距，
        // 不会压到卡片内容；垂直居中（top 50% + 负外边距上提半个高度）。
        panel = panel.child(
            div()
                .absolute()
                .top(relative(0.5))
                .right(px(0.0))
                .mt(px(-32.0))
                .id("collapse-session")
                .flex()
                .items_center()
                .justify_center()
                .w(px(20.0))
                .h(px(64.0))
                .rounded_lg()
                .bg(theme::raised())
                .border_1()
                .border_color(theme::border())
                .text_size(px(16.0))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(theme::fg())
                .cursor_pointer()
                .hover(|d| d.bg(theme::accent()).text_color(gpui::rgb(0xffffff)))
                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                    this.toggle_session_list(cx);
                }))
                .child("«"),
        );

        panel
    }

    /// 搜索行：点一下聚焦，Esc 清空（中文输入走 IME，不碰 key_char）。
    fn session_search_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.session_search.text().to_string();
        let placeholder = query.is_empty();
        let focused = self.input_target == InputTarget::SessionSearch;
        let text = if placeholder {
            "搜索会话…".to_string()
        } else {
            crate::views::text_fit::fit_text(&query, 176.0, 12.0)
        };
        let mut row = div()
            .id("session-search")
            .flex()
            .items_center()
            .gap_1p5()
            .mx_2()
            .mb_1()
            .px_2()
            .py_1()
            .rounded_md()
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
                this.switch_input_target(InputTarget::SessionSearch);
                let end = this.session_search.text().len();
                this.session_search.move_to(end, false);
                window.focus(&this.focus_handle);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
                this.on_session_search_key(ev, cx);
            }))
            .child(div().flex_shrink_0().text_xs().text_color(theme::dim()).child("⌕"))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(if placeholder { theme::dim() } else { theme::fg() })
                    .child(SharedString::from(text)),
            );
        if !placeholder {
            row = row.child(
                div()
                    .id("session-search-clear")
                    .flex_shrink_0()
                    .px_1()
                    .rounded_sm()
                    .text_xs()
                    .text_color(theme::dim())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::border()).text_color(theme::fg()))
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        cx.stop_propagation();
                        this.clear_session_search(cx);
                    }))
                    .child("✕"),
            );
        }
        row
    }

    fn session_card(&self, idx: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let session = &self.sessions[idx];
        let selected = idx == self.active_session;
        let color = theme::role_color(session.accent);

        let icon_text = if session.icon.is_empty() {
            session.title.chars().next().unwrap_or('·').to_string()
        } else {
            session.icon.clone()
        };
        let has_badge = !session.badge.is_empty();
        let has_action = session.action != SessionAction::None;

        // 文本必须按像素预算截断：gpui 的 `.truncate()` 只在首帧测量时已有
        // 确定宽度才生效，嵌套 flex 里会硬裁且不出省略号。
        let action_w = if has_action { 18.0 } else { 0.0 };
        let badge_w = if has_badge { 16.0 } else { 0.0 };
        // 分类标签大约占 34px（两字 + 内边距）
        let tag_w = if session.tag != neverwork_core::SessionTag::None {
            34.0
        } else {
            0.0
        };
        let title_w = CARD_INNER - 20.0 - 8.0 - badge_w - action_w - tag_w - 6.0 - 18.0;
        let title_text = crate::views::text_fit::fit_text(&session.title, title_w, 14.0);

        let (pill_label, pill_fg, pill_bg) = match session.status {
            SessionStatus::Todo => (
                "待执行".to_string(),
                theme::yellow(),
                soft(theme::yellow(), 0.14),
            ),
            SessionStatus::Working => (
                if session.meta.is_empty() {
                    "工作中".to_string()
                } else {
                    session.meta.clone()
                },
                theme::green(),
                theme::green_soft(),
            ),
            SessionStatus::Done => (
                "已完成".to_string(),
                theme::accent(),
                theme::accent_soft(),
            ),
            SessionStatus::Dropped => (
                "已废弃".to_string(),
                theme::dim(),
                theme::with_alpha(0xffffff, 0.06),
            ),
            SessionStatus::Idle => (
                String::new(),
                theme::dim(),
                theme::with_alpha(0xffffff, 0.0),
            ),
        };
        let has_pill = !pill_label.is_empty();
        let pill_w = match session.status {
            SessionStatus::Todo => 44.0,
            SessionStatus::Working => 48.0,
            SessionStatus::Done => 44.0,
            SessionStatus::Dropped => 44.0,
            SessionStatus::Idle => 0.0,
        };
        let time_w = 46.0;
        // 摘要列吃掉除「时间 + 胶囊」以外的全部宽度。
        // 第二行右侧有置顶按钮（16px）、⋯ 按钮（16px）+ 间距
        let preview_w =
            CARD_INNER - time_w - if has_pill { pill_w + 6.0 } else { 0.0 } - 16.0 - 16.0 - 10.0;

        // ---- 第一行：头像 + 分支徽标 + 标题 +（右）状态圆圈 ----
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
                    .rounded_full()
                    .bg(soft(color, 0.22))
                    .text_xs()
                    .text_color(color)
                    .child(SharedString::from(icon_text)),
            );

        // 会话模式：团队会话一眼能认出来（新建时选定，不可切换）
        let mode = session.mode;
        head = head.child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(if mode.is_team() {
                    theme::accent()
                } else {
                    theme::dim()
                })
                .child(SharedString::from(mode.glyph())),
        );

        if has_badge {
            head = head.child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(theme::green())
                    .child(SharedString::from(session.badge.clone())),
            );
        }

        // 分类标签：点一下循环切换（功能 → 修复 → 重构 → 文档 → 调试 → 实验 → 无）
        if session.tag != neverwork_core::SessionTag::None {
            let hue = session.tag.hue();
            let label = session.tag.label();
            head = head.child(
                div()
                    .id(SharedString::from(format!("sess-tag-{idx}")))
                    .flex_shrink_0()
                    .px_1()
                    .rounded_sm()
                    .bg(theme::with_alpha(hue, 0.18))
                    .text_xs()
                    .text_color(gpui::rgb(hue))
                    .cursor_pointer()
                    .hover(|d| d.opacity(0.75))
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                        // 点分类只切分类，不要连带切会话
                        this.cycle_session_tag(idx, cx);
                    }))
                    .child(SharedString::from(label)),
            );
        }

        head = head.child(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme::fg())
                .child(SharedString::from(title_text)),
        );

        head = head.child(div().flex_1());

        if has_action {
            let (glyph, fg, bg) = match session.action {
                SessionAction::Resume => ("▶", theme::yellow(), soft(theme::yellow(), 0.16)),
                SessionAction::Replay => ("◉", theme::yellow(), soft(theme::yellow(), 0.16)),
                SessionAction::Done => ("✓", theme::accent(), soft(theme::accent(), 0.18)),
                SessionAction::Failed => ("✕", theme::muted(), soft(theme::fg(), 0.10)),
                SessionAction::None => ("", theme::dim(), theme::with_alpha(0xffffff, 0.0)),
            };
            let action = session.action;
            head = head.child(
                div()
                    .id(SharedString::from(format!("sess-action-{idx}")))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(16.0))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(bg)
                    .text_xs()
                    .text_color(fg)
                    .cursor_pointer()
                    .hover(|d| d.opacity(0.8))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.activate_session(idx, action, cx);
                    }))
                    .child(SharedString::from(glyph)),
            );
        }

        // ---- 第二行：摘要 + 时间 + 状态胶囊 ----
        let preview_raw = if session.preview.is_empty() {
            session.subtitle.clone()
        } else {
            session.preview.clone()
        };
        let preview_text = crate::views::text_fit::fit_text(&preview_raw, preview_w, 12.0);

        let mut foot = div()
            .flex()
            .items_center()
            .gap_1p5()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme::dim())
                    .child(SharedString::from(preview_text)),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme::dim())
                    .child(SharedString::from(session.updated_label.clone())),
            );

        if has_pill {
            foot = foot.child(
                div()
                    .flex_shrink_0()
                    .px_1()
                    .py_0p5()
                    .rounded_sm()
                    .bg(pill_bg)
                    .text_xs()
                    .text_color(pill_fg)
                    .child(SharedString::from(pill_label)),
            );
        }

        // 置顶按钮放第二行右侧：第一行留给标题，避免长标题被挤成省略号。
        // 已置顶显示实心图钉（黄色），未置顶是淡色空心。
        let pinned = session.pinned;
        let menu_open = self.session_menu == Some(idx);
        foot = foot.child(
            div()
                .id(SharedString::from(format!("sess-menu-{idx}")))
                .flex()
                .items_center()
                .justify_center()
                .size(px(16.0))
                .flex_shrink_0()
                .rounded_sm()
                .text_color(if menu_open { theme::fg() } else { theme::dim() })
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                // 用 down 而不是 click：只有 down 事件带鼠标坐标，菜单才能贴着按钮弹
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, ev: &gpui::MouseDownEvent, _w, cx| {
                        cx.stop_propagation();
                        let p = ev.position;
                        this.toggle_session_menu(idx, (p.x.into(), p.y.into()), cx);
                    }),
                )
                .child("⋯"),
        );
        foot = foot.child(
            div()
                .id(SharedString::from(format!("sess-pin-{idx}")))
                .flex()
                .items_center()
                .justify_center()
                .size(px(16.0))
                .flex_shrink_0()
                .rounded_sm()
                .text_color(if pinned { theme::yellow() } else { theme::dim() })
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                    // 只切置顶，不要连带切会话
                    this.toggle_pin(idx, cx);
                }))
                .child(if pinned { "📌" } else { "⚲" }),
        );

        // 参考稿里卡片无边框，靠底色区分；选中项用略亮的底色。
        let mut card = div()
            .id(SharedString::from(format!("sess-{idx}")))
            .flex()
            .flex_col()
            .gap_1()
            .px(px(CARD_PAD))
            .py(px(7.0))
            .rounded_lg()
            .cursor_pointer()
            .when(selected, |d| {
                d.bg(theme::raised())
                    .border_1()
                    .border_color(theme::accent_line())
            })
            .when(!selected, |d| {
                d.border_1()
                    .border_color(theme::with_alpha(0x000000, 0.0))
                    .hover(|d| d.bg(theme::with_alpha(0xffffff, 0.035)))
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                this.select_session(idx, cx);
            }))
            // 右键任意位置弹操作菜单（菜单里承担全部可做之事）
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(move |this, ev: &gpui::MouseDownEvent, _w, cx| {
                    cx.stop_propagation();
                    let p = ev.position;
                    this.toggle_session_menu(idx, (p.x.into(), p.y.into()), cx);
                }),
            )
            .child(head)
            .child(foot);

        // 重命名：行内输入（Enter/保存按钮提交，Esc 取消）
        if self.rename_session == Some(idx) {
            card = card.child(self.rename_session_row(cx));
        }
        card
    }

    /// 卡片内联重命名行。
    fn rename_session_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let value = self.rename_buf.text().to_string();
        div()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .py_1()
            .rounded_md()
            .bg(theme::bg())
            .border_1()
            .border_color(theme::accent_line())
            .child(
                div()
                    .id("session-rename-input")
                    .flex()
                    .items_center()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme::fg())
                    .cursor(gpui::CursorStyle::IBeam)
                    .track_focus(&self.focus_handle)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.switch_input_target(InputTarget::RenameSession);
                        window.focus(&this.focus_handle);
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
                        this.on_rename_session_key(ev, cx);
                    }))
                    .child(SharedString::from(crate::views::text_fit::fit_text(
                        &value, 120.0, 12.0,
                    ))),
            )
            .child(
                div()
                    .id("session-rename-save")
                    .px_1p5()
                    .rounded_sm()
                    .bg(theme::accent())
                    .text_xs()
                    .text_color(theme::bg())
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        cx.stop_propagation();
                        this.commit_rename_session(cx);
                    }))
                    .child("保存"),
            )
            .child(
                div()
                    .id("session-rename-cancel")
                    .px_1()
                    .rounded_sm()
                    .text_xs()
                    .text_color(theme::dim())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        cx.stop_propagation();
                        this.cancel_rename_session(cx);
                    }))
                    .child("✕"),
            )
    }

    /// 点卡片右上角圆圈：按圆圈语义执行（继续 / 重放 / 无操作）。
    fn activate_session(
        &mut self,
        idx: usize,
        action: SessionAction,
        cx: &mut Context<Self>,
    ) {
        if idx >= self.sessions.len() {
            return;
        }
        self.active_session = idx;
        self.center = CenterView::Chat;
        match action {
            SessionAction::Resume => {
                // 圆圈是「可续跑」：真的接着跑，而不是只给个提示。
                // 内部会重新校 stage（已跑完 / 还在生成就只提示不动手）。
                self.resume_session(idx, cx);
                return;
            }
            SessionAction::Replay => {
                self.notice = Some("这是一条历史任务，输入新指令即可重新执行".into());
            }
            SessionAction::Done => {
                self.notice = Some("该会话已完成".into());
            }
            SessionAction::Failed => {
                self.notice = Some("该会话已中止，可重新输入指令".into());
            }
            SessionAction::None => {}
        }
        cx.notify();
    }
}

/// 给颜色附加 alpha，用于柔和底色。
fn soft(color: Rgba, alpha: f32) -> Rgba {
    Rgba {
        a: alpha.clamp(0.0, 1.0),
        ..color
    }
}
