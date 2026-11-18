//! 会话操作菜单：右键卡片（或点卡片上的 ⋯）在鼠标位置弹出。
//!
//! 用「全屏透明遮罩 + 绝对定位卡片」而不是 GPUI 的锚定弹层：
//! 项目里所有浮层都是这个形态（命令面板、工作区管理器），
//! 滚动列表里也不会因为元素移动而跑位。
//!
//! 行样式对齐设计稿：左侧语义图标 + 右侧 13px 文字，分组用分隔线，
//! 当前状态用「底色 + 高亮文字」标出。

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, App, ClickEvent, Context, MouseButton, MouseDownEvent, Rgba,
    SharedString, Window,
};

use neverwork_core::{SessionStatus, SessionTag};
use neverwork_ui::theme;

use crate::state::AppState;

/// 菜单卡片宽度（渲染侧 clamp 也用它，改这里要同步 `state.rs` 的 MENU_W）。
pub(crate) const MENU_W: f32 = 232.0;

impl AppState {
    /// `x` / `y` 是已经 clamp 进视口的左上角坐标（渲染侧算好）。
    pub(crate) fn session_menu_overlay(
        &self,
        idx: usize,
        x: f32,
        y: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(session) = self.sessions.get(idx) else {
            // 索引失效（被清理/删除）：只剩一层点击遮罩，点一下即关。
            return div()
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .size_full()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                    this.close_session_menu(cx);
                }))
                .into_any_element();
        };

        let status = session.status;
        let working = status == SessionStatus::Working;
        let pinned = session.pinned;
        let tag = session.tag;
        let labeled = matches!(status, SessionStatus::Todo | SessionStatus::Done | SessionStatus::Dropped);
        let confirming = self.session_delete_confirm == Some(idx);
        let title: String = session.title.chars().take(20).collect();

        let mut card = div()
            .absolute()
            .top(px(y))
            .left(px(x))
            .w(px(MENU_W))
            .flex()
            .flex_col()
            .p_1p5()
            .rounded(px(theme::radius_xl()))
            .bg(theme::panel())
            .border_1()
            .border_color(theme::border_strong())
            .occlude()
            // 菜单内点击不往外冒，避免误触底座
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _w, _cx| {})
            .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _w, _cx| {})
            // 顶部标题：确认操作的是哪一条
            .child(
                div()
                    .px_2()
                    .py_1()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_size(px(theme::text_caption()))
                    .text_color(theme::dim())
                    .child(SharedString::from(crate::views::text_fit::fit_text(
                        &format!("「{title}」"),
                        MENU_W - 28.0,
                        12.0,
                    ))),
            )
            .child(menu_divider())
            // ---- 列表操作 ----
            .child(menu_row(
                format!("sess-menu-{idx}-pin"),
                if pinned { "取消置顶" } else { "置顶" }.into(),
                theme::fg(),
                false,
                true,
                menu_glyph("⌖", theme::muted()),
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.toggle_pin(idx, cx);
                }),
            ))
            .child(menu_row(
                format!("sess-menu-{idx}-tag"),
                format!(
                    "分类：{}",
                    if tag == SessionTag::None {
                        "无"
                    } else {
                        tag.label()
                    }
                ),
                if tag == SessionTag::None {
                    theme::fg()
                } else {
                    gpui::rgb(tag.hue())
                },
                tag != SessionTag::None,
                true,
                menu_glyph(
                    "◇",
                    if tag == SessionTag::None {
                        theme::muted()
                    } else {
                        gpui::rgb(tag.hue())
                    },
                ),
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.cycle_session_tag(idx, cx);
                }),
            ))
            .child(menu_divider())
            // ---- 状态标记（运行态优先，生成中整组禁用） ----
            .child(menu_row(
                format!("sess-menu-{idx}-todo"),
                "标记为「待执行」".into(),
                if status == SessionStatus::Todo {
                    theme::yellow()
                } else {
                    theme::fg()
                },
                status == SessionStatus::Todo,
                !working,
                menu_glyph("◔", theme::yellow()),
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.set_session_status(idx, SessionStatus::Todo, cx);
                }),
            ))
            .child(menu_row(
                format!("sess-menu-{idx}-done"),
                "标记为「已完成」".into(),
                if status == SessionStatus::Done {
                    theme::accent()
                } else {
                    theme::fg()
                },
                status == SessionStatus::Done,
                !working,
                menu_check(theme::accent()),
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.set_session_status(idx, SessionStatus::Done, cx);
                }),
            ))
            .child(menu_row(
                format!("sess-menu-{idx}-dropped"),
                "标记为「已废弃」".into(),
                if status == SessionStatus::Dropped {
                    theme::muted()
                } else {
                    theme::fg()
                },
                status == SessionStatus::Dropped,
                !working,
                menu_glyph("⊘", theme::muted()),
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.set_session_status(idx, SessionStatus::Dropped, cx);
                }),
            ))
            .child(menu_row(
                format!("sess-menu-{idx}-clearmark"),
                "移除状态标记".into(),
                theme::fg(),
                false,
                !working && labeled,
                menu_glyph("↺", theme::dim()),
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.set_session_status(idx, SessionStatus::Idle, cx);
                }),
            ))
            .child(menu_divider())
            // ---- 管理 ----
            .child(menu_row(
                format!("sess-menu-{idx}-rename"),
                "重命名".into(),
                theme::fg(),
                false,
                true,
                menu_glyph("✎", theme::muted()),
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.begin_rename_session(idx, cx);
                }),
            ))
            .child(menu_row(
                format!("sess-menu-{idx}-export"),
                "导出 Markdown".into(),
                theme::fg(),
                false,
                true,
                menu_glyph("↧", theme::muted()),
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.export_session(idx, cx);
                }),
            ))
            .child(menu_divider())
            .child(menu_row(
                format!("sess-menu-{idx}-delete"),
                if confirming {
                    "再点确认删除".into()
                } else {
                    "删除会话".into()
                },
                theme::red(),
                false,
                true,
                menu_glyph("✕", theme::red()),
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.click_delete_session(idx, cx);
                }),
            ));

        if working {
            card = card.child(
                div()
                    .px_2()
                    .py_1()
                    .text_size(px(theme::text_caption()))
                    .text_color(theme::dim())
                    .child("生成中，状态以运行态为准"),
            );
        }

        div()
            .id("session-menu-backdrop")
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                    this.close_session_menu(cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                    this.close_session_menu(cx);
                }),
            )
            .child(card)
            .into_any_element()
    }
}

/// 一行菜单项：左侧语义图标 + 文案；`active` 用底色标出当前状态。
///
/// 文件树菜单 / 编辑器标签菜单也复用这三个绘制函数，保持所有菜单同形。
pub(crate) fn menu_row(
    id: String,
    label: String,
    label_fg: Rgba,
    active: bool,
    enabled: bool,
    icon: AnyElement,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let mut row = div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(px(theme::radius_lg()))
        .text_size(px(theme::text_ui()))
        .text_color(label_fg)
        .when(active, |d| d.bg(theme::raised()))
        .child(
            div()
                .w(px(16.0))
                .flex()
                .flex_shrink_0()
                .justify_center()
                .child(icon),
        )
        .child(SharedString::from(label));
    if enabled {
        row = row
            .cursor_pointer()
            .hover(|d| d.bg(theme::raised()))
            .on_click(on_click);
    } else {
        row = row.opacity(0.4);
    }
    row.into_any_element()
}

/// 单字符语义图标（置顶/分类/重命名/导出/删除/状态行用）。
pub(crate) fn menu_glyph(glyph: &'static str, color: Rgba) -> AnyElement {
    div()
        .text_size(px(15.0))
        .text_color(color)
        .child(glyph)
        .into_any_element()
}

/// 圆圈勾（已完成）：外圈 + 勾，比 emoji 在两套主题下都稳定。
fn menu_check(color: Rgba) -> AnyElement {
    div()
        .size(px(14.0))
        .rounded_full()
        .border_1()
        .border_color(color)
        .flex()
        .items_center()
        .justify_center()
        .child(div().text_size(px(8.0)).text_color(color).child("✓"))
        .into_any_element()
}

pub(crate) fn menu_divider() -> impl IntoElement {
    div().h(px(1.0)).mx_1().my_1().bg(theme::border())
}
