//! 文件树右键菜单 + 编辑器标签右键菜单。
//!
//! 与会话菜单同形态：全屏透明遮罩 + 绝对定位卡片（`session_menu.rs` 的
//! `menu_row` / `menu_glyph` 复用），坐标 clamp 在 `state.rs::render` 里做。
//!
//! 文件树菜单：「＋ 添加到会话」（复制进附件夹）「⧉ 复制路径」。
//! 标签菜单：「✕ 关闭」「✕ 关闭全部」。

use gpui::prelude::*;
use gpui::{div, px, AnyElement, ClickEvent, Context, MouseButton, MouseDownEvent, SharedString};

use neverwork_ui::theme;

use crate::state::{AppState, FileMenuTarget};
use crate::views::session_menu::{menu_divider, menu_glyph, menu_row};
use crate::views::text_fit::fit_text;

/// 菜单卡片宽度（`state.rs` 的 clamp 用同一个常量）。
pub(crate) const FILE_MENU_W: f32 = 216.0;

impl AppState {
    /// 文件树右键菜单。`x` / `y` 是已经 clamp 进视口的左上角坐标。
    pub(crate) fn file_menu_overlay(
        &self,
        target: &FileMenuTarget,
        x: f32,
        y: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = target
            .abs
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .or_else(|| {
                target
                    .rel
                    .as_ref()
                    .map(|r| r.rsplit('/').next().unwrap_or(r).to_string())
            })
            .unwrap_or_else(|| "未命名".into());

        let add_t = target.clone();
        let copy_t = target.clone();
        // 远程文件没有本地副本，添加附件走不了；目录也不支持。
        let can_attach = !target.is_dir && target.abs.is_some();

        let card = div()
            .absolute()
            .top(px(y))
            .left(px(x))
            .w(px(FILE_MENU_W))
            .flex()
            .flex_col()
            .p_1p5()
            .rounded_xl()
            .bg(theme::panel())
            .border_1()
            .border_color(theme::border())
            .occlude()
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _w, _cx| {})
            .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _w, _cx| {})
            .child(
                div()
                    .px_2()
                    .py_1()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_xs()
                    .text_color(theme::dim())
                    .child(SharedString::from(fit_text(
                        &name,
                        FILE_MENU_W - 28.0,
                        12.0,
                    ))),
            )
            .child(menu_divider())
            .child(menu_row(
                "file-menu-add".into(),
                "＋ 添加到会话".into(),
                theme::fg(),
                false,
                can_attach,
                menu_glyph("✚", theme::green()),
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.add_to_chat_from_menu(add_t.clone(), cx);
                }),
            ))
            .child(menu_row(
                "file-menu-copy".into(),
                "⧉ 复制路径".into(),
                theme::fg(),
                false,
                true,
                menu_glyph("⧉", theme::accent()),
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.copy_path_from_menu(copy_t.clone(), cx);
                }),
            ));

        div()
            .id("file-menu-backdrop")
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                    this.close_file_menu(cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                    this.close_file_menu(cx);
                }),
            )
            .child(card)
            .into_any_element()
    }

    /// 编辑器标签右键菜单：关闭 / 关闭全部。
    pub(crate) fn tab_menu_overlay(
        &self,
        x: f32,
        y: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = self.open_file_name();
        let card = div()
            .absolute()
            .top(px(y))
            .left(px(x))
            .w(px(FILE_MENU_W))
            .flex()
            .flex_col()
            .p_1p5()
            .rounded_xl()
            .bg(theme::panel())
            .border_1()
            .border_color(theme::border())
            .occlude()
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _w, _cx| {})
            .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _w, _cx| {})
            .child(
                div()
                    .px_2()
                    .py_1()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_xs()
                    .text_color(theme::dim())
                    .child(SharedString::from(fit_text(
                        &name,
                        FILE_MENU_W - 28.0,
                        12.0,
                    ))),
            )
            .child(menu_divider())
            .child(menu_row(
                "tab-menu-close".into(),
                "✕ 关闭".into(),
                theme::fg(),
                false,
                true,
                menu_glyph("✕", theme::muted()),
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.close_current_file(cx);
                }),
            ))
            .child(menu_row(
                "tab-menu-close-all".into(),
                "✕ 关闭全部".into(),
                theme::fg(),
                false,
                true,
                menu_glyph("⊗", theme::muted()),
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.close_all_files(cx);
                }),
            ));

        div()
            .id("tab-menu-backdrop")
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                    this.close_tab_menu(cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                    this.close_tab_menu(cx);
                }),
            )
            .child(card)
            .into_any_element()
    }
}
