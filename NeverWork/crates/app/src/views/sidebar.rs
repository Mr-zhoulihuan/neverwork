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
use gpui::{div, px, ClickEvent, Context, Rgba, SharedString};

use neverwork_core::{SessionAction, SessionStatus};
use neverwork_ui::theme;

use crate::state::{AppState, CenterView};

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
    Editor,
    Palette,
}

impl AppState {
    /// `hide_list` 为真时只保留图标栏（极窄窗口）。
    pub(crate) fn left_sidebar(&self, hide_list: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let row = div()
            .flex()
            .flex_row()
            .flex_shrink_0()
            .h_full()
            .child(self.icon_rail(cx));
        if hide_list {
            row
        } else {
            row.child(self.session_panel(cx))
        }
    }

    // ==================== 图标栏 ====================

    fn icon_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_chat = self.center == CenterView::Chat;
        let is_team = self.center == CenterView::Team;
        let is_editor = self.center == CenterView::File;

        let mut rail = div()
            .flex()
            .flex_col()
            .items_center()
            .w(px(RAIL_W))
            .flex_shrink_0()
            .h_full()
            .pt_3()
            .pb_2()
            .gap_1()
            .bg(theme::bg())
            .border_r_1()
            .border_color(theme::border());

        // 品牌标
        rail = rail.child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.0))
                .rounded_lg()
                .bg(theme::accent())
                .mb_2()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(gpui::rgb(0xffffff))
                .child("N"),
        );

        rail = rail
            .child(self.rail_button("rail-chat", "◐", "对话", is_chat, RailAction::Chat, cx))
            .child(self.rail_button("rail-team", "◈", "团队", is_team, RailAction::Team, cx))
            .child(self.rail_button("rail-editor", "▤", "编辑器", is_editor, RailAction::Editor, cx))
            .child(self.rail_button("rail-cmd", "⌘", "命令", self.palette_open, RailAction::Palette, cx));

        // 底部：AI 状态 + 用户头像
        rail = rail
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.0))
                    .rounded_full()
                    .bg(if self.ai_ready {
                        theme::green_soft()
                    } else {
                        theme::raised()
                    })
                    .text_xs()
                    .text_color(if self.ai_ready {
                        theme::green()
                    } else {
                        theme::dim()
                    })
                    .child(if self.ai_ready { "●" } else { "○" }),
            )
            .child(
                div()
                    .id("rail-user")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(28.0))
                    .rounded_full()
                    .bg(theme::raised())
                    .border_1()
                    .border_color(theme::border())
                    .text_xs()
                    .text_color(theme::fg())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::border()))
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.toggle_palette(cx);
                    }))
                    .child("U"),
            );

        rail
    }

    fn rail_button(
        &self,
        id: &'static str,
        glyph: &'static str,
        label: &'static str,
        active: bool,
        action: RailAction,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_0p5()
            .w(px(38.0))
            .py_1()
            .rounded_lg()
            .cursor_pointer()
            .when(active, |d| d.bg(theme::raised()))
            .when(!active, |d| d.hover(|d| d.bg(theme::panel())))
            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                this.run_rail(action, cx);
            }))
            .child(
                div()
                    .text_sm()
                    .text_color(if active { theme::fg() } else { theme::muted() })
                    .child(SharedString::from(glyph)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(if active { theme::fg() } else { theme::dim() })
                    .child(SharedString::from(label)),
            )
    }

    fn run_rail(&mut self, action: RailAction, cx: &mut Context<Self>) {
        match action {
            RailAction::Chat => self.show_chat(cx),
            RailAction::Team => self.set_center(CenterView::Team, cx),
            RailAction::Editor => {
                if self.open_path.is_some() {
                    self.focus_mode = false;
                    self.set_center(CenterView::File, cx);
                } else {
                    self.notice = Some("先在右侧文件树里打开一个文件".into());
                    cx.notify();
                }
            }
            RailAction::Palette => self.toggle_palette(cx),
        }
    }

    // ==================== 会话列表 ====================

    fn session_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut panel = div()
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
                            this.new_session(cx);
                        }))
                        .child("＋"),
                ),
        );

        // ---- 工作区细条（点击切换工作区） ----
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
                .child(div().text_xs().text_color(theme::green()).child("▤"))
                .child(
                    div()
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(theme::muted())
                        .child(SharedString::from(crate::views::text_fit::fit_text(
                            &self.workspace.name,
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
                .child(div().text_xs().text_color(theme::accent()).child("切换")),
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

        for idx in 0..self.sessions.len() {
            list = list.child(self.session_card(idx, cx));
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

        panel
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
        let title_w = CARD_INNER - 20.0 - 8.0 - badge_w - action_w - 6.0;
        let title_text = crate::views::text_fit::fit_text(&session.title, title_w, 14.0);

        let (pill_label, pill_fg, pill_bg) = match session.status {
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
            SessionStatus::Working => 48.0,
            SessionStatus::Done => 44.0,
            SessionStatus::Dropped => 44.0,
            SessionStatus::Idle => 0.0,
        };
        let time_w = 46.0;
        // 摘要列吃掉除「时间 + 胶囊」以外的全部宽度。
        let preview_w = CARD_INNER - time_w - if has_pill { pill_w + 6.0 } else { 0.0 } - 6.0;

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

        if has_badge {
            head = head.child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(theme::green())
                    .child(SharedString::from(session.badge.clone())),
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

        // 参考稿里卡片无边框，靠底色区分；选中项用略亮的底色。
        div()
            .id(SharedString::from(format!("sess-{idx}")))
            .flex()
            .flex_col()
            .gap_1()
            .px(px(CARD_PAD))
            .py(px(7.0))
            .rounded_lg()
            .cursor_pointer()
            .when(selected, |d| d.bg(theme::raised()))
            .when(!selected, |d| d.hover(|d| d.bg(theme::with_alpha(0xffffff, 0.035))))
            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                this.select_session(idx, cx);
            }))
            .child(head)
            .child(foot)
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
                self.notice = Some("已切到该会话，可继续输入指令".into());
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
