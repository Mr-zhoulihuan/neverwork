//! IME / 文本输入实现：**这是文本插入的唯一入口**。
//!
//! 之前 `on_key_down` 里手写 `key_char → push_str` 与这里的 `replace_text_in_range`
//! 并存，同一个字符会被插入两次或被吞掉。现在所有可打印字符与中文输入法组合
//! 都只经过 `ElementInputHandler → 本文件`，键盘只处理非文本按键（见 keymap.rs）。

use std::ops::Range;

use gpui::{Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window};

use crate::state::{AppState, FormField, InputTarget};
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


/// 计算本次 IME 替换应作用的范围（UTF-16 下标）。
///
/// GPUI 在 Windows 上一律传 `range = None`，自己不做范围记忆，
/// 所以优先级是：显式 range > 上次组合范围（marked）> `None`（表示在光标/选区处插入）。
pub fn ime_target_range(
    text: &str,
    explicit: Option<&Range<usize>>,
    marked: Option<&Range<usize>>,
) -> Option<Range<usize>> {
    explicit
        .map(|r| utf16_to_byte(text, r))
        .or_else(|| marked.map(|r| utf16_to_byte(text, r)))
}

/// 组合串更新后应标记为 IME 组合态的范围（UTF-16）。
///
/// 传入的「选中范围」在 Windows 上常是折叠的（IME 光标位置），
/// 不能当成选区，否则标记范围为空、下一次更新会重复插入。
pub fn composition_marked_range(text_after: &str, start_byte: usize, len_byte: usize) -> Range<usize> {
    let end = (start_byte + len_byte).min(text_after.len());
    byte_to_utf16(text_after, &(start_byte.min(end)..end))
}

impl AppState {
    /// 当前输入目标的可变缓冲区。
    pub(crate) fn active_buffer_mut(&mut self) -> &mut TextBuffer {
        match self.input_target {
            InputTarget::Chat => &mut self.chat,
            InputTarget::Editor => &mut self.editor,
            InputTarget::Workspace => &mut self.ws_input,
            InputTarget::SessionSearch => &mut self.session_search,
            InputTarget::TraceSearch => &mut self.trace_query,
            InputTarget::AskCustom => &mut self.ask_custom,
            InputTarget::RenameSession => &mut self.rename_buf,
            InputTarget::RolePrompt => &mut self.role_prompt_buf,
            InputTarget::Field(_) => &mut self.field_buf,
        }
    }

    /// 当前输入目标的只读缓冲区。
    pub(crate) fn active_buffer(&self) -> &TextBuffer {
        match self.input_target {
            InputTarget::Chat => &self.chat,
            InputTarget::Editor => &self.editor,
            InputTarget::Workspace => &self.ws_input,
            InputTarget::SessionSearch => &self.session_search,
            InputTarget::TraceSearch => &self.trace_query,
            InputTarget::AskCustom => &self.ask_custom,
            InputTarget::RenameSession => &self.rename_buf,
            InputTarget::RolePrompt => &self.role_prompt_buf,
            InputTarget::Field(_) => &self.field_buf,
        }
    }

    /// 文本被修改后统一收尾：标记脏、通知重绘。
    pub(crate) fn after_edit(&mut self, cx: &mut Context<Self>) {
        if self.input_target == InputTarget::Editor {
            self.dirty = true;
        }
        // 技能搜索框：IME 上屏的文本也要同步到搜索词并触发防抖搜索。
        // 组合串（marked）期间不发请求，避免拿拼音去搜。
        if self.input_target == InputTarget::Field(FormField::SkillSearch)
            && self.marked_range.is_none()
        {
            let value = self.field_buf.text().to_string();
            if self.skill_search != value {
                self.skill_search = value;
                self.on_skill_search_changed(cx);
            }
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
        _new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.active_buffer().text().to_string();
        let byte_range = ime_target_range(&text, range_utf16.as_ref(), self.marked_range.as_ref());

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

        // 关键：整段组合串都要算作 marked。
        //
        // GPUI 的 `new_selected_range_utf16` 传的其实是 IME **光标位置**
        // （Windows 上是 `pos..pos`，折叠范围），不是选区。以前拿它算标记范围，
        // 得到的是一个空区间，于是下一次组合更新（`range=None`）找不到要替换的位置，
        // 直接在光标处插入 —— 表现为「每打一个字就把前面已输入的整段重复一遍」。
        if new_text.is_empty() {
            self.marked_range = None;
        } else {
            let after = self.active_buffer().text().to_string();
            self.marked_range = Some(composition_marked_range(&after, start, len));
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
        // 基准点用输入框自身的位置（`input_area_bounds`），而不是 GPUI 传进来的
        // element_bounds —— 后者是那个 1x1 输入桥，拿它当基准会让候选框
        // 永远飘在窗口左上角。
        let element_bounds = self.input_area_bounds.get().unwrap_or(element_bounds);
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


    #[test]
    fn composition_marks_whole_string_not_ime_caret() {
        // Windows 传的「选中范围」是 IME 光标位置（折叠区间）。
        // 标记范围必须是整段组合串，否则下一次组合更新找不到替换位置，
        // 会在光标处再插一遍 —— 这就是「打一个字重复一整段」的 bug。
        let text = "a'b";
        let r = composition_marked_range(text, 0, text.len());
        assert_eq!(r, 0..3, "整段都算组合态");
    }

    #[test]
    fn composition_marks_only_inserted_part() {
        let text = "开头组合串";
        let start = "开头".len();
        let r = composition_marked_range(text, start, "组合串".len());
        assert_eq!(r, 2..5, "只标记本次插入的部分（UTF-16）");
    }

    #[test]
    fn composition_marked_range_clamps_overflow() {
        let text = "abc";
        // len 超出（比如 IME 给的字节数比实际长）时不能越界
        let r = composition_marked_range(text, 1, 99);
        assert_eq!(r, 1..3);
    }

    #[test]
    fn empty_composition_has_empty_range() {
        let r = composition_marked_range("", 0, 0);
        assert_eq!(r, 0..0);
    }

    #[test]
    fn ime_target_prefers_explicit_range() {
        let text = "abcdef";
        let explicit = 2..4;
        let marked = 0..1;
        let r = ime_target_range(text, Some(&explicit), Some(&marked));
        assert_eq!(r, Some(2..4), "显式 range 优先于上次组合范围");
    }

    #[test]
    fn ime_target_falls_back_to_marked() {
        let text = "abcdef";
        let marked = 1..3;
        let r = ime_target_range(text, None, Some(&marked));
        assert_eq!(r, Some(1..3), "没有显式 range 时替换上次组合串");
    }

    #[test]
    fn ime_target_none_means_insert_at_cursor() {
        let r = ime_target_range("abc", None, None);
        assert_eq!(r, None, "无范围表示在光标处插入");
    }

    #[test]
    fn ime_target_handles_utf16_offsets() {
        // 「中」是 1 个 UTF-16 单元但 3 个 UTF-8 字节，转换必须按字符边界走
        let text = "中文ab";
        let r = ime_target_range(text, Some(&(1..2)), None);
        assert_eq!(r, Some(3..6), "第二个字符 文 的字节范围（3 字节）");
    }
