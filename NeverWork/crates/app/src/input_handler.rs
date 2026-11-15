//! IME / 文本输入实现：**这是文本插入的唯一入口**。
//!
//! 之前 `on_key_down` 里手写 `key_char → push_str` 与这里的 `replace_text_in_range`
//! 并存，同一个字符会被插入两次或被吞掉。现在所有可打印字符与中文输入法组合
//! 都只经过 `ElementInputHandler → 本文件`，键盘只处理非文本按键（见 keymap.rs）。

use std::ops::Range;

use gpui::{Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window};

use crate::state::{AppState, InputTarget};
use crate::text_buffer::TextBuffer;

/// UTF-16 索引范围 → UTF-8 字节范围（对齐到 char boundary）。
pub fn utf16_to_byte(s: &str, range: &Range<usize>) -> Range<usize> {
    let mut utf16_count = 0usize;
    let mut start_byte = None;
    let mut end_byte = None;
    for (byte_idx, ch) in s.char_indices() {
        if start_byte.is_none() && utf16_count >= range.start {
            start_byte = Some(byte_idx);
        }
        if end_byte.is_none() && utf16_count >= range.end {
            end_byte = Some(byte_idx);
        }
        if start_byte.is_some() && end_byte.is_some() {
            break;
        }
        utf16_count += ch.len_utf16();
    }
    let start = start_byte.unwrap_or(s.len()).min(s.len());
    let end = end_byte.unwrap_or(s.len()).min(s.len());
    let start = floor_boundary(s, start);
    let end = floor_boundary(s, end.max(start));
    start..end
}

/// UTF-8 字节范围 → UTF-16 索引范围。
pub fn byte_to_utf16(s: &str, range: &Range<usize>) -> Range<usize> {
    let start = floor_boundary(s, range.start.min(s.len()));
    let end = floor_boundary(s, range.end.min(s.len()).max(start));
    let start_u16 = s[..start].encode_utf16().count();
    let end_u16 = start_u16 + s[start..end].encode_utf16().count();
    start_u16..end_u16
}

fn floor_boundary(s: &str, mut idx: usize) -> usize {
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

impl AppState {
    /// 当前输入目标的可变缓冲区。
    pub(crate) fn active_buffer_mut(&mut self) -> &mut TextBuffer {
        match self.input_target {
            InputTarget::Chat => &mut self.chat,
            InputTarget::Editor => &mut self.editor,
            InputTarget::Workspace => &mut self.ws_input,
            InputTarget::Field(_) => &mut self.field_buf,
        }
    }

    /// 当前输入目标的只读缓冲区。
    pub(crate) fn active_buffer(&self) -> &TextBuffer {
        match self.input_target {
            InputTarget::Chat => &self.chat,
            InputTarget::Editor => &self.editor,
            InputTarget::Workspace => &self.ws_input,
            InputTarget::Field(_) => &self.field_buf,
        }
    }

    /// 文本被修改后统一收尾：标记脏、通知重绘。
    pub(crate) fn after_edit(&mut self, cx: &mut Context<Self>) {
        if self.input_target == InputTarget::Editor {
            self.dirty = true;
        }
        cx.notify();
    }

    /// 切换输入目标时调用：清掉上一个目标的 IME 组合态，避免 marked 范围错位。
    pub(crate) fn switch_input_target(&mut self, target: InputTarget) {
        if self.input_target != target {
            self.marked_range = None;
            self.input_target = target;
        }
    }
}

impl EntityInputHandler for AppState {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.active_buffer().text().to_string();
        let bytes = utf16_to_byte(&text, &range_utf16);
        actual_range.replace(byte_to_utf16(&text, &bytes));
        Some(text[bytes].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let buf = self.active_buffer();
        let text = buf.text();
        if let Some(sel) = buf.selection() {
            let cursor = buf.cursor();
            return Some(UTF16Selection {
                range: byte_to_utf16(text, &sel),
                // 光标在选区起点说明是反向选择（向左/上扩展）。
                reversed: cursor == sel.start,
            });
        }
        let c = buf.cursor();
        Some(UTF16Selection {
            range: byte_to_utf16(text, &(c..c)),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range.clone()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.marked_range.take().is_some() {
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.active_buffer().text().to_string();
        // 优先级：显式 range > 上次 IME 组合范围 > 当前光标/选区。
        let byte_range = range_utf16
            .as_ref()
            .map(|r| utf16_to_byte(&text, r))
            .or_else(|| self.marked_range.as_ref().map(|r| utf16_to_byte(&text, r)));

        let buf = self.active_buffer_mut();
        match byte_range {
            Some(r) => {
                // 精确替换指定区间（IME 提交、候选词替换）。
                buf.move_to(r.start, false);
                if r.end > r.start {
                    buf.move_to(r.end, true);
                }
                buf.insert(new_text);
            }
            // None：光标处插入；若存在选区，insert 会自动替换选区。
            None => buf.insert(new_text),
        }
        self.marked_range = None;
        self.after_edit(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.active_buffer().text().to_string();
        let byte_range = range_utf16
            .as_ref()
            .map(|r| utf16_to_byte(&text, r))
            .or_else(|| self.marked_range.as_ref().map(|r| utf16_to_byte(&text, r)));

        let (start, len) = {
            let buf = self.active_buffer_mut();
            let start = match byte_range {
                Some(r) => {
                    buf.move_to(r.start, false);
                    if r.end > r.start {
                        buf.move_to(r.end, true);
                    }
                    r.start
                }
                None => buf.cursor(),
            };
            buf.insert(new_text);
            (start, new_text.len())
        };

        let after = self.active_buffer().text().to_string();
        if new_text.is_empty() {
            self.marked_range = None;
        } else {
            // 保住 IME 给出的候选词内选中范围（未给出时整段标记）。
            let inner = new_selected_range_utf16
                .unwrap_or_else(|| 0..new_text.encode_utf16().count());
            let inner_start = start + utf16_to_byte(&after[start..start + len], &inner).start;
            let inner_end = start + utf16_to_byte(&after[start..start + len], &inner).end;
            self.marked_range = Some(byte_to_utf16(&after, &(inner_start..inner_end)));
        }
        self.after_edit(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // 用「行号 + 字符列」估算屏幕位置，让输入法候选框贴住光标。
        let buf = self.active_buffer();
        let text = buf.text();
        let bytes = utf16_to_byte(text, &range_utf16);
        let pos = bytes.start;
        let line = text[..pos].bytes().filter(|b| *b == b'\n').count();
        let line_start = text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col = text[line_start..pos].chars().count();

        let line_h = 18.0_f32;
        let char_w = 7.2_f32;
        let x = element_bounds.origin.x + gpui::px(col as f32 * char_w);
        let y = element_bounds.origin.y + gpui::px(line as f32 * line_h);
        Some(Bounds {
            origin: Point { x, y },
            size: gpui::size(gpui::px(char_w), gpui::px(line_h)),
        })
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let area = self.input_area_bounds.get()?;
        let char_w = 7.2_f32;
        let line_h = 18.0_f32;
        let rel_x: f32 = (point.x - area.origin.x).into();
        let rel_y: f32 = (point.y - area.origin.y).into();
        if rel_x < 0.0 || rel_y < 0.0 {
            return None;
        }
        let line = (rel_y / line_h) as usize;
        let col = (rel_x / char_w) as usize;

        let buf = self.active_buffer();
        let text = buf.text();
        let mut cur_line = 0usize;
        let mut line_start = 0usize;
        for (i, ch) in text.char_indices() {
            if cur_line == line {
                break;
            }
            if ch == '\n' {
                cur_line += 1;
                line_start = i + 1;
            }
        }
        if cur_line != line {
            return None;
        }
        let line_end = text[line_start..]
            .find('\n')
            .map(|i| line_start + i)
            .unwrap_or(text.len());
        let mut pos = line_end;
        for (n, (i, _)) in text[line_start..line_end].char_indices().enumerate() {
            if n == col {
                pos = line_start + i;
                break;
            }
        }
        Some(byte_to_utf16(text, &(pos..pos)).start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_roundtrip_ascii() {
        let s = "hello";
        let r = 1..3;
        assert_eq!(utf16_to_byte(s, &r), 1..3);
        assert_eq!(byte_to_utf16(s, &(1..3)), 1..3);
    }

    #[test]
    fn utf16_roundtrip_cjk() {
        let s = "a中文b";
        // UTF-16: a=0, 中=1, 文=2, b=3
        assert_eq!(utf16_to_byte(s, &(1..3)), 1..7);
        assert_eq!(byte_to_utf16(s, &(1..7)), 1..3);
    }

    #[test]
    fn utf16_clamps_out_of_range() {
        let s = "ab";
        assert_eq!(utf16_to_byte(s, &(5..9)), 2..2);
    }

    #[test]
    fn utf16_handles_emoji_surrogate_pairs() {
        let s = "😀x";
        // 😀 在 UTF-16 里占两个单元。
        assert_eq!(utf16_to_byte(s, &(0..2)), 0..4);
        assert_eq!(byte_to_utf16(s, &(0..4)), 0..2);
    }
}
