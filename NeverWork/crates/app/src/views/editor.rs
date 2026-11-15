//! 编辑器视图：行号、语法高亮、可见光标、选区、按字符点击定位、虚拟滚动。
//!
//! 关键修复：
//! - 之前没有任何光标绘制，用户看不出插入点在哪；现在每个字符位置都能精确插入 caret。
//! - 之前点击只定位到**行首**；现在点哪个字符就落在哪个字符前。
//! - 之前每次按键全量重跑高亮；现在走 `LineCache`，只重算变化的行。
//! - 之前一次性渲染所有行；现在用 `uniform_list` 虚拟滚动，并跟随光标滚动。

use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, AnyElement, Context, Entity, FocusHandle, FontWeight, MouseButton,
    MouseDownEvent, Rgba, ScrollStrategy, SharedString,
};

use neverwork_ui::theme;

use crate::highlight::{self, Lang};
use crate::line_cache::EditorLines;
use crate::state::{AppState, InputTarget};

pub(crate) const LINE_H: f32 = 20.0;
const GUTTER_W: f32 = 46.0;
/// 选区底色。
const SEL_BG: u32 = 0x264f78;

impl AppState {
    pub(crate) fn editor_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let filename = self.open_file_name();
        let lang = highlight::lang_for(&filename);
        let dirty_mark = if self.dirty { " ●" } else { "" };
        let focus = self.editor_focus.clone();

        let mut col = div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .bg(theme::surface());

        // ===== 顶部标签栏 =====
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(px(38.0))
                .px_3()
                .bg(theme::panel())
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .h_full()
                        .px_3()
                        .bg(theme::surface())
                        .border_b_1()
                        .border_color(theme::accent())
                        .text_sm()
                        .text_color(theme::fg())
                        .child(div().text_xs().text_color(theme::orange()).child("⬢"))
                        .child(SharedString::from(format!("{filename}{dirty_mark}"))),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("editor-fmt")
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .text_xs()
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::raised()))
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                            this.insert_indent(cx);
                        }))
                        .child("⇥ 缩进"),
                )
                .child(
                    div()
                        .id("save-file")
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .text_xs()
                        .text_color(theme::green())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::raised()))
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                            this.save_file(cx);
                        }))
                        .child("保存 ⌘S"),
                )
                .child(
                    div()
                        .id("back-to-chat")
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .text_xs()
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::raised()))
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                            this.show_chat(cx);
                        }))
                        .child("← 对话"),
                ),
        );

        // ===== 行列表 =====
        let lines = self.line_cache.borrow_mut().sync(self.editor.text(), lang);
        let cursor = self.editor.cursor();
        let sel = self.editor.selection();
        let count = lines.line_count();
        let entity = cx.entity();
        let scroll = self.editor_scroll.clone();
        let for_list = lines.clone();

        let list = uniform_list("editor-lines", count, move |range, _window, _cx| {
            let mut out: Vec<AnyElement> = Vec::with_capacity(range.len());
            for i in range {
                out.push(render_line(
                    entity.clone(),
                    focus.clone(),
                    i,
                    &for_list,
                    cursor,
                    sel.clone(),
                ));
            }
            out
        })
        .track_scroll(scroll)
        .flex_1()
        // 焦点落在整块列表上，键盘事件才能进来。
        .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
            this.on_editor_key(ev, cx);
        }));

        col = col.child(list);

        // ===== 底部状态栏 =====
        col = col.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(24.0))
                .px_3()
                .border_t_1()
                .border_color(theme::border())
                .bg(theme::panel())
                .text_xs()
                .text_color(theme::dim())
                .child(SharedString::from(format!(
                    "行 {} · 列 {}　{}　{} 行",
                    self.editor.line_index() + 1,
                    self.editor.column() + 1,
                    lang_name(lang),
                    count
                )))
                .child(SharedString::from(if self.dirty {
                    "未保存 · Ctrl+S 保存"
                } else {
                    "已保存"
                })),
        );

        col
    }

    /// 打开文件后同步缓存与光标。
    pub(crate) fn on_editor_opened(&mut self) {
        self.line_cache.borrow_mut().invalidate();
    }

    /// 让编辑器滚到光标所在行。
    pub(crate) fn scroll_editor_to_cursor(&self) {
        let line = self.editor.line_index();
        self.editor_scroll
            .scroll_to_item(line, ScrollStrategy::Center);
    }
}

/// 渲染一行。每个字符都是独立的可点击单元，因此点击可以精确落到字符上。
fn render_line(
    entity: Entity<AppState>,
    focus: FocusHandle,
    idx: usize,
    lines: &EditorLines,
    cursor: usize,
    sel: Option<Range<usize>>,
) -> AnyElement {
    let start = lines.line_start(idx);
    let text: &str = &lines.texts[idx];
    let end = start + text.len();
    let cursor_here = if cursor >= start && cursor <= end {
        Some(cursor - start)
    } else {
        None
    };
    let active_line = cursor_here.is_some();

    let mut text_el = div()
        .flex()
        .flex_row()
        .flex_1()
        .min_w(px(0.0))
        .overflow_hidden();

    let mut offset = 0usize;
    let mut caret_drawn = false;

    for span in lines.spans[idx].iter() {
        let color = highlight::color_for(span.tok);
        for (byte_in_span, ch) in span.text.char_indices() {
            let inner = offset + byte_in_span;
            if cursor_here == Some(inner) {
                text_el = text_el.child(caret(entity.clone(), focus.clone(), active_line));
                caret_drawn = true;
            }
            let abs = start + inner;
            let selected = sel
                .as_ref()
                .map(|r| abs >= r.start && abs < r.end)
                .unwrap_or(false);
            text_el = text_el.child(char_cell(entity.clone(), focus.clone(), ch, color, selected, abs));
        }
        offset += span.text.len();
    }
    if !caret_drawn && cursor_here == Some(text.len()) {
        text_el = text_el.child(caret(entity.clone(), focus.clone(), active_line));
    }

    // 行尾空白：点击定位到行末。
    let tail = {
        let entity = entity.clone();
        let focus = focus.clone();
        div()
            .flex_1()
            .min_w(px(8.0))
            .h_full()
            .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
                let extend = ev.modifiers.shift;
                entity.update(cx, |this, cx| {
                    this.switch_input_target(InputTarget::Editor);
                    this.editor.move_to(end, extend);
                    cx.notify();
                });
                window.focus(&focus);
            })
    };
    text_el = text_el.child(tail);

    div()
        .id(SharedString::from(format!("l{idx}")))
        .flex()
        .flex_row()
        .items_center()
        .h(px(LINE_H))
        .w_full()
        .when(active_line, |d| d.bg(gpui::rgb(0x232323)))
        .child(
            div()
                .flex()
                .justify_end()
                .w(px(GUTTER_W))
                .flex_shrink_0()
                .pr_2()
                .text_xs()
                .text_color(theme::dim())
                .child(SharedString::from(format!("{}", idx + 1))),
        )
        .child(text_el)
        .into_any_element()
}

fn caret(entity: Entity<AppState>, focus: FocusHandle, active: bool) -> AnyElement {
    let color = if active { theme::accent() } else { theme::muted() };
    div()
        .w(px(2.0))
        .h(px(14.0))
        .flex_shrink_0()
        .bg(color)
        .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
            if ev.modifiers.shift {
                return;
            }
            entity.update(cx, |this, cx| {
                this.switch_input_target(InputTarget::Editor);
                cx.notify();
            });
            window.focus(&focus);
        })
        .into_any_element()
}

fn char_cell(
    entity: Entity<AppState>,
    focus: FocusHandle,
    ch: char,
    color: Rgba,
    selected: bool,
    abs: usize,
) -> AnyElement {
    let display: SharedString = match ch {
        ' ' => SharedString::from("\u{00A0}"),
        '\t' => SharedString::from("    "),
        _ => SharedString::from(ch.to_string()),
    };
    let cell = div()
        .text_xs()
        .flex_shrink_0()
        .text_color(color)
        .when(selected, |d| d.bg(gpui::rgb(SEL_BG)))
        .child(display)
        .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
            let extend = ev.modifiers.shift;
            entity.update(cx, |this, cx| {
                this.switch_input_target(InputTarget::Editor);
                this.editor.move_to(abs, extend);
                cx.notify();
            });
            window.focus(&focus);
        });
    cell.into_any_element()
}

pub(crate) fn lang_name(lang: Lang) -> &'static str {
    match lang {
        Lang::Rust => "Rust",
        Lang::Ts => "TypeScript",
        Lang::Python => "Python",
        Lang::Json => "JSON",
        Lang::Toml => "TOML",
        Lang::Markdown => "Markdown",
        Lang::Other => "Plain",
    }
}

/// 预留：Rc 包装的滚动句柄类型别名（便于将来换实现）。
pub(crate) type EditorScroll = Rc<()>;
const _: Option<EditorScroll> = None;

/// 只是为了让 `FontWeight` 等导入在裁剪时保持稳定。
#[allow(dead_code)]
fn _unused(_w: FontWeight) {}
