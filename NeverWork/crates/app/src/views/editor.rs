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
use crate::state::{AppState, InputTarget, MediaPreview};

/// 默认行高（会被设置里的字号覆盖）。
#[allow(dead_code)]
pub(crate) const LINE_H: f32 = 20.0;
const GUTTER_W: f32 = 46.0;
/// 选区底色。
const SEL_BG: u32 = 0x264f78;

impl AppState {
    pub(crate) fn editor_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // 图片 / PDF：二进制内容进不了文本编辑器，整块换成只读查看器。
        if let Some(media) = self.media_preview.clone() {
            return self.media_pane(media, cx).into_any_element();
        }
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
                        .id("editor-tab")
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
                        // 右键标签：关闭 / 关闭全部
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, ev: &MouseDownEvent, _w, cx| {
                                cx.stop_propagation();
                                let at = (ev.position.x.into(), ev.position.y.into());
                                this.open_tab_menu(at, cx);
                            }),
                        )
                        .child(div().text_xs().text_color(theme::orange()).child("⬢"))
                        .child(SharedString::from(format!("{filename}{dirty_mark}")))
                        // 关闭当前文件：有未保存改动会先落盘，失败则不关
                        .child(
                            div()
                                .id("close-file")
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(18.0))
                                .rounded_sm()
                                .text_xs()
                                .text_color(theme::dim())
                                .cursor_pointer()
                                .hover(|d| {
                                    d.bg(theme::with_alpha(0xef4444, 0.18))
                                        .text_color(theme::red())
                                })
                                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                                    this.close_current_file(cx);
                                }))
                                .child("✕"),
                        ),
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

        // ===== 查找 / 替换栏 =====
        if self.find.open {
            col = col.child(self.find_bar(cx));
        }

        // ===== 行列表 =====
        let lines = self.line_cache.borrow_mut().sync(self.editor.text(), lang);
        let cursor = self.editor.cursor();
        let all_cursors: std::rc::Rc<Vec<usize>> = std::rc::Rc::new(self.editor.cursors());
        let sel = self.editor.selection();
        let count = lines.line_count();
        let entity = cx.entity();
        let scroll = self.editor_scroll.clone();
        let for_list = lines.clone();

        let line_h = self.line_height();
        let font_px = self.editor_font_size();
        let show_numbers = self.settings.show_line_numbers;
        let gutter_w = if show_numbers { GUTTER_W } else { 8.0 };

        // 折叠：先算出「真实行号 → 是否可见」，再让列表只渲染可见行。
        // 用真实行号做映射（而不是重排文本），这样光标/选区/保存全都无需改动。
        let raw_text = self.editor.text();
        // 括号配对：光标停在括号上（或紧跟其后）时，两个括号都描边高亮。
        let bracket_hl: Option<(usize, usize)> =
            match crate::brackets::find_pair(raw_text, cursor, lang) {
                Some(b) if b.is_balanced() => Some((b.at, b.mate)),
                _ => None,
            };
        let bracket_rc = std::rc::Rc::new(bracket_hl);
        let raw_lines: Vec<&str> = raw_text.split('\n').collect();
        let foldable: Vec<usize> = crate::folding::foldable_starts(&raw_lines);
        let ranges = crate::folding::collapsed_ranges(&self.folds, &raw_lines);
        let mut visible: Vec<usize> = Vec::with_capacity(count);
        for i in 0..count {
            if !crate::folding::is_hidden(i, &ranges) {
                visible.push(i);
            }
        }
        let vis_count = visible.len();
        let visible_rc = std::rc::Rc::new(visible);
        let foldable_rc = std::rc::Rc::new(foldable);
        let folds_rc = std::rc::Rc::new(self.folds.clone());

        let list = uniform_list("editor-lines", vis_count, move |range, _window, _cx| {
            let mut out: Vec<AnyElement> = Vec::with_capacity(range.len());
            for vi in range {
                let Some(&i) = visible_rc.get(vi) else { continue };
                out.push(render_line(
                    entity.clone(),
                    focus.clone(),
                    i,
                    &for_list,
                    cursor,
                    all_cursors.clone(),
                    sel.clone(),
                    line_h,
                    font_px,
                    show_numbers,
                    gutter_w,
                    foldable_rc.binary_search(&i).is_ok(),
                    folds_rc.contains(&i),
                    bracket_rc.clone(),
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

        // 自动保存：有挂起的改动时启动一个 800ms 的计时，超时若无新改动就落盘。
        if self.auto_save_pending && self.settings.auto_save {
            let gen = self.auto_save_gen.clone();
            let target = gen.get();
            cx.spawn(async move |weak, async_cx| {
                async_cx
                    .background_executor()
                    .timer(std::time::Duration::from_millis(800))
                    .await;
                // 期间又改过，就让那一轮的计时器负责保存。
                if gen.get() != target {
                    return;
                }
                let _ = weak.update(async_cx, |this, cx| {
                    this.flush_auto_save(cx);
                });
            })
            .detach();
        }

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
                    "行 {} · 列 {}　{}　{} 行{}",
                    self.editor.line_index() + 1,
                    self.editor.column() + 1,
                    lang_name(lang),
                    vis_count,
                    if self.folds.is_empty() {
                        String::new()
                    } else {
                        format!("（折叠 {} 处）", self.folds.len())
                    }
                )))
                .child(SharedString::from(if self.dirty {
                    "未保存 · Ctrl+S 保存"
                } else {
                    "已保存"
                })),
        );

        col.into_any_element()
    }

    /// 媒体只读查看器（图片 / PDF / Markdown）：替换整块编辑器区域。
    fn media_pane(&self, media: MediaPreview, cx: &mut Context<Self>) -> impl IntoElement {
        let path = media.path().clone();
        let filename = self.open_file_name();
        let size = std::fs::metadata(&path)
            .map(|m| crate::state::human_size(m.len()))
            .unwrap_or_default();
        let (kind, color) = match &media {
            MediaPreview::Image(_) => ("图片", theme::pink()),
            MediaPreview::Pdf(_) => ("PDF", theme::red()),
            MediaPreview::Markdown(_) => ("Markdown", theme::indigo()),
        };
        let is_markdown = matches!(&media, MediaPreview::Markdown(_));

        // ===== 顶部标签栏：与文本编辑器同构，只留查看相关动作 =====
        let mut header = div()
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
                    .id("media-tab")
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
                    // 右键标签：关闭 / 关闭全部
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, ev: &MouseDownEvent, _w, cx| {
                            cx.stop_propagation();
                            let at = (ev.position.x.into(), ev.position.y.into());
                            this.open_tab_menu(at, cx);
                        }),
                    )
                    .child(div().text_xs().text_color(color).child("⬢"))
                    .child(SharedString::from(filename.clone()))
                    .child(
                        div()
                            .id("media-close")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(18.0))
                            .rounded_sm()
                            .text_xs()
                            .text_color(theme::dim())
                            .cursor_pointer()
                            .hover(|d| {
                                d.bg(theme::with_alpha(0xef4444, 0.18))
                                    .text_color(theme::red())
                            })
                            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                                this.close_current_file(cx);
                            }))
                            .child("✕"),
                    ),
            )
            .child(div().flex_1());

        if is_markdown {
            // Markdown 默认渲染预览，需要改内容时切回源码编辑器。
            header = header.child(
                div()
                    .id("media-edit-source")
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
                        this.edit_current_as_text(cx);
                    }))
                    .child("✎ 编辑源码"),
            );
        } else {
            header = header.child(
                div()
                    .id("media-open-external")
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_xs()
                    .text_color(theme::accent())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::raised()))
                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                        this.open_media_externally(cx);
                    }))
                    .child("↗ 用默认应用打开"),
            );
        }

        header = header.child(
                div()
                    .id("media-back-to-chat")
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
            );

        // ===== 主体 =====
        let mut body = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(theme::surface());

        match &media {
            MediaPreview::Image(p) => {
                let click = p.clone();
                body = body.child(
                    div()
                        .size_full()
                        .p_4()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .id("media-image")
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                                    this.open_image_view(click.clone(), cx);
                                }))
                                .child(
                                    gpui::img(p.clone())
                                        .max_w_full()
                                        .max_h_full()
                                        .object_fit(gpui::ObjectFit::Contain),
                                ),
                        ),
                );
            }
            MediaPreview::Pdf(_) => {
                body = body.child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(72.0))
                                .h(px(90.0))
                                .rounded_lg()
                                .bg(theme::tint(theme::red(), 0.14))
                                .border_1()
                                .border_color(theme::tint(theme::red(), 0.45))
                                .text_lg()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme::red())
                                .child("PDF"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme::fg())
                                .child(SharedString::from(filename.clone())),
                        )
                        .child(
                            div().text_xs().text_color(theme::dim()).child(
                                SharedString::from(format!("{size} · 内置查看暂不支持 PDF 排版渲染")),
                            ),
                        )
                        .child(
                            div()
                                .id("media-open-external-card")
                                .px_3()
                                .py(px(6.0))
                                .rounded_md()
                                .bg(theme::accent_soft())
                                .text_xs()
                                .text_color(theme::accent())
                                .cursor_pointer()
                                .hover(|d| d.bg(theme::border()))
                                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                                    this.open_media_externally(cx);
                                }))
                                .child("用默认应用打开"),
                        ),
                );
            }
            MediaPreview::Markdown(_) => {
                // 复用聊天区的 Markdown 块渲染，宽度按中栏实际可用宽度估算。
                let text = crate::filesystem::read_file(&path).unwrap_or_default();
                let avail = (self.chat_w - 40.0).max(240.0);
                let mut md = div()
                    .id("md-preview")
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_6()
                    .flex_1()
                    .overflow_scroll()
                    .text_size(px(14.0))
                    .text_color(theme::fg());
                if text.trim().is_empty() {
                    md = md.child(div().text_color(theme::dim()).child("（空文件）"));
                } else {
                    for b in crate::markdown::parse(&text) {
                        md = md.child(crate::views::chat::markdown_block(
                            b,
                            avail,
                            self.root.as_deref(),
                            cx,
                        ));
                    }
                }
                body = body.child(md);
            }
        }

        // ===== 底部状态栏 =====
        let footer = div()
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
                "{kind}查看 · {filename} · {size}"
            )))
            .child(SharedString::from(match &media {
                MediaPreview::Image(_) => "只读 · 点击图片放大",
                MediaPreview::Pdf(_) => "PDF 请用默认应用阅读",
                MediaPreview::Markdown(_) => "只读预览 · ✎ 可编辑源码",
            }));

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .bg(theme::surface())
            .child(header)
            .child(body)
            .child(footer)
    }

    /// 查找 / 替换栏：贴在编辑器标签栏下面。
    ///
    /// 键盘：Enter 下一个、Shift+Enter 上一个、Esc 关闭。
    /// 输入框复用 `field_buf` + `InputTarget::Field`，因此中文输入法可用。
    fn find_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::state::FormField;

        let find_editing = self.active_field == Some(FormField::FindQuery);
        let repl_editing = self.active_field == Some(FormField::FindReplace);
        let count = self.find.count();
        let cur = if count == 0 { 0 } else { self.find.current + 1 };

        let mut bar = div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .bg(theme::panel())
            .border_b_1()
            .border_color(theme::border());

        // ---- 查找行 ----
        let mut row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().text_xs().text_color(theme::dim()).child("⌕"))
            .child(
                div()
                    .id("find-query")
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_1()
                    .min_w(px(80.0))
                    .h(px(26.0))
                    .px_2()
                    .rounded_md()
                    .bg(theme::raised())
                    .border_1()
                    .border_color(if find_editing {
                        theme::accent_line()
                    } else {
                        theme::border()
                    })
                    .cursor(gpui::CursorStyle::IBeam)
                    .when(find_editing, |d| d.track_focus(&self.editor_focus))
                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, window, cx| {
                        this.begin_find_field(FormField::FindQuery, cx);
                        window.focus(&this.editor_focus);
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
                        this.on_find_key(ev, cx);
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(if self.find.query.is_empty() {
                                theme::dim()
                            } else {
                                theme::fg()
                            })
                            .child(SharedString::from(if self.find.query.is_empty() {
                                "查找".to_string()
                            } else {
                                self.find.query.clone()
                            })),
                    )
                    .when(find_editing, |d| {
                        d.child(div().text_color(theme::accent()).child("▏"))
                    }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .min_w(px(56.0))
                    .text_xs()
                    .text_color(if count == 0 { theme::dim() } else { theme::muted() })
                    .child(SharedString::from(if self.find.query.is_empty() {
                        "—".to_string()
                    } else if count == 0 {
                        "无结果".to_string()
                    } else {
                        format!("{cur}/{count}")
                    })),
            );

        let icon_btn = |id: &'static str,
                        glyph: &'static str,
                        cx: &mut Context<AppState>,
                        f: fn(&mut AppState, &mut Context<AppState>)| {
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_center()
                .size(px(22.0))
                .rounded_sm()
                .text_xs()
                .text_color(theme::muted())
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| f(this, cx)))
                .child(glyph)
        };

        row = row
            .child(icon_btn("find-prev", "↑", cx, |this, cx| this.find_prev(cx)))
            .child(icon_btn("find-next", "↓", cx, |this, cx| this.find_next(cx)));

        // 区分大小写开关
        row = row.child(
            div()
                .id("find-case")
                .px_1p5()
                .py_0p5()
                .rounded_sm()
                .text_xs()
                .cursor_pointer()
                .when(self.find.case_sensitive, |d| {
                    d.bg(theme::accent_soft()).text_color(theme::accent())
                })
                .when(!self.find.case_sensitive, |d| {
                    d.text_color(theme::dim()).hover(|d| d.bg(theme::raised()))
                })
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                    this.find.case_sensitive = !this.find.case_sensitive;
                    let q = this.find.query.clone();
                    this.set_find_query(q, cx);
                }))
                .child("Aa"),
        );

        row = row.child(div().flex_1());
        row = row.child(icon_btn("find-close", "✕", cx, |this, cx| this.close_find(cx)));
        bar = bar.child(row);

        // ---- 替换行 ----
        let mut row2 = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().text_xs().text_color(theme::dim()).child("⇄"))
            .child(
                div()
                    .id("find-replace")
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_1()
                    .min_w(px(80.0))
                    .h(px(26.0))
                    .px_2()
                    .rounded_md()
                    .bg(theme::raised())
                    .border_1()
                    .border_color(if repl_editing {
                        theme::accent_line()
                    } else {
                        theme::border()
                    })
                    .cursor(gpui::CursorStyle::IBeam)
                    .when(repl_editing, |d| d.track_focus(&self.editor_focus))
                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, window, cx| {
                        this.begin_find_field(FormField::FindReplace, cx);
                        window.focus(&this.editor_focus);
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
                        this.on_find_key(ev, cx);
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(if self.find.replace.is_empty() {
                                theme::dim()
                            } else {
                                theme::fg()
                            })
                            .child(SharedString::from(if self.find.replace.is_empty() {
                                "替换为".to_string()
                            } else {
                                self.find.replace.clone()
                            })),
                    )
                    .when(repl_editing, |d| {
                        d.child(div().text_color(theme::accent()).child("▏"))
                    }),
            )
            .child(
                div()
                    .id("replace-one")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(theme::raised())
                    .text_xs()
                    .text_color(theme::muted())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::border()).text_color(theme::fg()))
                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                        this.replace_current_match(cx);
                    }))
                    .child("替换"),
            )
            .child(
                div()
                    .id("replace-all")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(theme::accent())
                    .text_xs()
                    .text_color(gpui::rgb(0xffffff))
                    .cursor_pointer()
                    .hover(|d| d.opacity(0.88))
                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                        this.replace_all_matches(cx);
                    }))
                    .child("全部替换"),
            );
        row2 = row2.child(div().flex_1());
        bar = bar.child(row2);

        bar
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
#[allow(clippy::too_many_arguments)]
fn render_line(
    entity: Entity<AppState>,
    focus: FocusHandle,
    idx: usize,
    lines: &EditorLines,
    cursor: usize,
    all_cursors: std::rc::Rc<Vec<usize>>,
    sel: Option<Range<usize>>,
    line_h: f32,
    font_px: f32,
    show_numbers: bool,
    gutter_w: f32,
    foldable: bool,
    folded: bool,
    bracket_hl: std::rc::Rc<Option<(usize, usize)>>,
) -> AnyElement {
    let start = lines.line_start(idx);
    let text: &str = &lines.texts[idx];
    let end = start + text.len();
    // 本行内所有光标（多光标时可能不止一个）
    let cursors_in_line: Vec<usize> = all_cursors
        .iter()
        .copied()
        .filter(|c| *c >= start && *c <= end)
        .map(|c| c - start)
        .collect();
    let cursor_here = if cursor >= start && cursor <= end {
        Some(cursor - start)
    } else {
        None
    };
    let active_line = !cursors_in_line.is_empty() || cursor_here.is_some();

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
            if let Some(pos) = cursors_in_line.iter().position(|p| *p == inner) {
                // 主光标用强调色，额外光标用次强调色，一眼能区分
                let primary = cursor_here == Some(inner);
                text_el = text_el.child(caret_colored(
                    entity.clone(),
                    focus.clone(),
                    active_line,
                    primary,
                    pos,
                ));
                caret_drawn = true;
            }
            let abs = start + inner;
            let selected = sel
                .as_ref()
                .map(|r| abs >= r.start && abs < r.end)
                .unwrap_or(false);
            let is_bracket = matches!(
                bracket_hl.as_ref(),
                Some((a, b)) if abs == *a || abs == *b
            );
            text_el = text_el.child(char_cell(
                entity.clone(),
                focus.clone(),
                ch,
                color,
                selected,
                is_bracket,
                abs,
                font_px,
            ));
        }
        offset += span.text.len();
    }
    if !caret_drawn {
        if let Some(pos) = cursors_in_line.iter().position(|p| *p == text.len()) {
            let primary = cursor_here == Some(text.len());
            text_el = text_el.child(caret_colored(
                entity.clone(),
                focus.clone(),
                active_line,
                primary,
                pos,
            ));
        }
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
        .h(px(line_h))
        .w_full()
        .when(active_line, |d| d.bg(theme::with_alpha(0xffffff, 0.035)))
        .when(show_numbers, |d| {
            // 行号栏：可折叠的行在行号左边显示箭头，点箭头折叠 / 展开。
            let mut gutter = div()
                .flex()
                .items_center()
                .justify_end()
                .w(px(gutter_w))
                .flex_shrink_0()
                .pr_2();

            if foldable {
                let e2 = entity.clone();
                let f2 = focus.clone();
                gutter = gutter.child(
                    div()
                        .id(SharedString::from(format!("fold-{idx}")))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(12.0))
                        .mr_1()
                        .rounded_sm()
                        .text_xs()
                        .text_color(if folded { theme::accent() } else { theme::dim() })
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                        .on_mouse_down(
                            MouseButton::Left,
                            move |_ev: &MouseDownEvent, window, cx| {
                                e2.update(cx, |this, cx| {
                                    this.toggle_fold(idx, cx);
                                });
                                window.focus(&f2);
                            },
                        )
                        .child(if folded { "▸" } else { "▾" }),
                );
            } else {
                gutter = gutter.child(div().w(px(16.0)).child(""));
            }

            d.child(
                gutter.child(
                    div()
                        .text_xs()
                        .text_color(theme::dim())
                        .child(SharedString::from(format!("{}", idx + 1))),
                ),
            )
        })
        .child(text_el)
        .into_any_element()
}

/// 带「主/次」区分的 caret：主光标用强调色，额外光标用半透明强调色，
/// 并且额外光标略窄，重叠时也能看出有几个。
fn caret_colored(
    entity: Entity<AppState>,
    focus: FocusHandle,
    active: bool,
    primary: bool,
    _ordinal: usize,
) -> AnyElement {
    let color = if primary {
        if active {
            theme::accent()
        } else {
            theme::muted()
        }
    } else {
        theme::with_alpha(0x3b82f6, 0.65)
    };
    let w = if primary { 2.0 } else { 1.5 };
    div()
        .w(px(w))
        .h(px(15.0))
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


#[allow(clippy::too_many_arguments)]
fn char_cell(
    entity: Entity<AppState>,
    focus: FocusHandle,
    ch: char,
    color: Rgba,
    selected: bool,
    is_bracket: bool,
    abs: usize,
    font_px: f32,
) -> AnyElement {
    let display: SharedString = match ch {
        ' ' => SharedString::from("\u{00A0}"),
        '\t' => SharedString::from("    "),
        _ => SharedString::from(ch.to_string()),
    };
    let cell = div()
        .text_size(px(font_px))
        .flex_shrink_0()
        .text_color(color)
        .when(selected, |d| d.bg(gpui::rgb(SEL_BG)))
        // 配对括号：加底色 + 描边，比单纯变色更容易在密集代码里看到
        .when(is_bracket, |d| {
            d.bg(theme::with_alpha(0x3b82f6, 0.22))
                .border_b_1()
                .border_color(theme::accent())
        })
        .child(display)
        .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
            let extend = ev.modifiers.shift;
            // Alt/Option + 点击 = 增加一个光标（多光标编辑）
            let add = ev.modifiers.alt || ev.modifiers.platform;
            entity.update(cx, |this, cx| {
                this.switch_input_target(InputTarget::Editor);
                if add {
                    this.add_cursor_at(abs, cx);
                } else {
                    this.clear_multi_cursor(cx);
                    this.editor.move_to(abs, extend);
                }
                cx.notify();
            });
            window.focus(&focus);
        });
    cell.into_any_element()
}

pub(crate) fn lang_name(lang: Lang) -> &'static str {
    match lang {
        Lang::Rust => "Rust",
        Lang::Go => "Go",
        Lang::Ts => "TypeScript",
        Lang::Python => "Python",
        Lang::Json => "JSON",
        Lang::Toml => "TOML",
        Lang::Yaml => "YAML",
        Lang::Markdown => "Markdown",
        Lang::Html => "HTML",
        Lang::Css => "CSS",
        Lang::Sql => "SQL",
        Lang::Shell => "Shell",
        Lang::CLike => "C/C++/Java",
        Lang::Ruby => "Ruby",
        Lang::Lua => "Lua",
        Lang::Other => "Plain",
    }
}

/// 预留：Rc 包装的滚动句柄类型别名（便于将来换实现）。
pub(crate) type EditorScroll = Rc<()>;
const _: Option<EditorScroll> = None;

/// 只是为了让 `FontWeight` 等导入在裁剪时保持稳定。
#[allow(dead_code)]
fn _unused(_w: FontWeight) {}
