//! 文本缓冲区：光标、选区、撤销/重做、按行移动的统一实现。
//!
//! 设计要点：
//! - 光标与选区一律用 **UTF-8 字节偏移**表示，但所有修改都保证落在 char boundary 上。
//! - 单行缓冲（对话输入框）与多行缓冲（编辑器）共用同一套操作，只由 `multiline` 决定换行行为。
//! - 撤销采用整串快照：实现简单、绝不会出现增量栈不同步的幽灵 bug；栈深上限 400。

use std::ops::Range;

#[derive(Clone, Debug)]
struct Snapshot {
    text: String,
    cursor: usize,
    anchor: Option<usize>,
}

const MAX_UNDO: usize = 400;

#[derive(Clone, Debug, Default)]
pub struct TextBuffer {
    text: String,
    /// 光标字节偏移，始终在 char boundary 上。
    cursor: usize,
    /// 选区锚点；存在且不等于 cursor 时构成选区。
    anchor: Option<usize>,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    multiline: bool,
}

/// 上一个字符边界。
pub fn prev_boundary(s: &str, idx: usize) -> usize {
    if idx == 0 {
        return 0;
    }
    let mut i = idx.min(s.len()) - 1;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// 下一个字符边界。
pub fn next_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx + 1;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(s.len())
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[allow(dead_code)] // 部分行级操作是给编辑器预留的公开 API
impl TextBuffer {
    pub fn single_line() -> Self {
        Self::default()
    }

    pub fn multi_line() -> Self {
        Self {
            multiline: true,
            ..Default::default()
        }
    }

    pub fn from_text(text: impl Into<String>, multiline: bool) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self {
            text,
            cursor,
            anchor: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            multiline,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn multiline(&self) -> bool {
        self.multiline
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 选区（有序）。无选区返回 None。
    pub fn selection(&self) -> Option<Range<usize>> {
        let a = self.anchor?;
        if a == self.cursor {
            return None;
        }
        Some(if a < self.cursor { a..self.cursor } else { self.cursor..a })
    }

    pub fn has_selection(&self) -> bool {
        self.selection().is_some()
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|r| &self.text[r])
    }

    /// 整体替换内容（打开文件 / 新建会话），清空历史。
    pub fn reset(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
        self.anchor = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// 标记「脏」：内容被外部逻辑直接改过（例如程序化插入）。
    pub fn mark_modified(&mut self) {
        self.anchor = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    fn clamp(&self, pos: usize) -> usize {
        let mut p = pos.min(self.text.len());
        while p > 0 && !self.text.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    fn record(&mut self) {
        if self.undo_stack.len() >= MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        });
        self.redo_stack.clear();
    }

    // ===== 光标移动 =====

    pub fn move_to(&mut self, pos: usize, extend: bool) {
        let p = self.clamp(pos);
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        self.cursor = p;
    }

    pub fn move_left(&mut self, extend: bool) {
        if !extend {
            if let Some(r) = self.selection() {
                self.cursor = r.start;
                self.anchor = None;
                return;
            }
        }
        let p = prev_boundary(&self.text, self.cursor);
        self.move_to(p, extend);
    }

    pub fn move_right(&mut self, extend: bool) {
        if !extend {
            if let Some(r) = self.selection() {
                self.cursor = r.end;
                self.anchor = None;
                return;
            }
        }
        let p = next_boundary(&self.text, self.cursor);
        self.move_to(p, extend);
    }

    /// Ctrl+Left：跳到上一个词首。
    pub fn move_word_left(&mut self, extend: bool) {
        let mut p = self.cursor;
        while p > 0 {
            let prev = prev_boundary(&self.text, p);
            let ch = self.text[prev..p].chars().next().unwrap_or(' ');
            if is_word_char(ch) {
                break;
            }
            p = prev;
        }
        while p > 0 {
            let prev = prev_boundary(&self.text, p);
            let ch = self.text[prev..p].chars().next().unwrap_or(' ');
            if !is_word_char(ch) {
                break;
            }
            p = prev;
        }
        self.move_to(p, extend);
    }

    /// Ctrl+Right：跳到下一个词尾。
    pub fn move_word_right(&mut self, extend: bool) {
        let mut p = self.cursor;
        while p < self.text.len() {
            let next = next_boundary(&self.text, p);
            let ch = self.text[p..next].chars().next().unwrap_or(' ');
            if is_word_char(ch) {
                break;
            }
            p = next;
        }
        while p < self.text.len() {
            let next = next_boundary(&self.text, p);
            let ch = self.text[p..next].chars().next().unwrap_or(' ');
            if !is_word_char(ch) {
                break;
            }
            p = next;
        }
        self.move_to(p, extend);
    }

    pub fn line_start_of(&self, pos: usize) -> usize {
        let p = pos.min(self.text.len());
        self.text[..p].rfind('\n').map(|i| i + 1).unwrap_or(0)
    }

    pub fn line_end_of(&self, pos: usize) -> usize {
        let p = pos.min(self.text.len());
        self.text[p..].find('\n').map(|i| p + i).unwrap_or(self.text.len())
    }

    /// 光标所在行号（0 基）。
    pub fn line_index(&self) -> usize {
        self.text[..self.cursor.min(self.text.len())]
            .bytes()
            .filter(|b| *b == b'\n')
            .count()
    }

    /// 光标在行内的字符列（0 基）。
    pub fn column(&self) -> usize {
        let ls = self.line_start_of(self.cursor);
        self.text[ls..self.cursor].chars().count()
    }

    fn pos_at_col(&self, line_start: usize, line_end: usize, col: usize) -> usize {
        let seg = &self.text[line_start..line_end];
        for (n, (i, _)) in seg.char_indices().enumerate() {
            if n == col {
                return line_start + i;
            }
        }
        line_end
    }

    pub fn move_home(&mut self, extend: bool) {
        let p = self.line_start_of(self.cursor);
        self.move_to(p, extend);
    }

    pub fn move_end(&mut self, extend: bool) {
        let p = self.line_end_of(self.cursor);
        self.move_to(p, extend);
    }

    pub fn move_doc_start(&mut self, extend: bool) {
        self.move_to(0, extend);
    }

    pub fn move_doc_end(&mut self, extend: bool) {
        self.move_to(self.text.len(), extend);
    }

    pub fn move_up(&mut self, extend: bool) {
        let ls = self.line_start_of(self.cursor);
        if ls == 0 {
            self.move_to(0, extend);
            return;
        }
        let col = self.column();
        let prev_ls = self.line_start_of(ls - 1);
        let prev_le = ls - 1;
        let target = self.pos_at_col(prev_ls, prev_le, col);
        self.move_to(target, extend);
    }

    pub fn move_down(&mut self, extend: bool) {
        let le = self.line_end_of(self.cursor);
        if le >= self.text.len() {
            self.move_to(self.text.len(), extend);
            return;
        }
        let col = self.column();
        let next_ls = le + 1;
        let next_le = self.line_end_of(next_ls);
        let target = self.pos_at_col(next_ls, next_le, col);
        self.move_to(target, extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.text.len();
    }

    // ===== 编辑 =====

    pub fn insert(&mut self, s: &str) {
        let s: String = if self.multiline {
            s.to_string()
        } else {
            s.replace(['\n', '\r'], " ")
        };
        if s.is_empty() {
            return;
        }
        self.record();
        let range = self.selection().unwrap_or(self.cursor..self.cursor);
        self.text.replace_range(range.clone(), &s);
        self.cursor = range.start + s.len();
        self.anchor = None;
    }

    pub fn newline(&mut self) {
        if self.multiline {
            self.insert("\n");
        }
    }

    /// 删除选区，或光标前一个字符。
    pub fn backspace(&mut self) {
        if let Some(r) = self.selection() {
            self.record();
            self.text.replace_range(r.clone(), "");
            self.cursor = r.start;
            self.anchor = None;
            return;
        }
        if self.cursor == 0 {
            return;
        }
        self.record();
        let prev = prev_boundary(&self.text, self.cursor);
        self.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
    }

    /// 删除选区，或光标后一个字符。
    pub fn delete_forward(&mut self) {
        if let Some(r) = self.selection() {
            self.record();
            self.text.replace_range(r.clone(), "");
            self.cursor = r.start;
            self.anchor = None;
            return;
        }
        if self.cursor >= self.text.len() {
            return;
        }
        self.record();
        let next = next_boundary(&self.text, self.cursor);
        self.text.replace_range(self.cursor..next, "");
    }

    /// 取走选区（剪切）。无选区时返回 None。
    pub fn take_selection(&mut self) -> Option<String> {
        let r = self.selection()?;
        let taken = self.text[r.clone()].to_string();
        self.record();
        self.text.replace_range(r.clone(), "");
        self.cursor = r.start;
        self.anchor = None;
        Some(taken)
    }

    pub fn undo(&mut self) -> bool {
        let Some(snap) = self.undo_stack.pop() else {
            return false;
        };
        self.redo_stack.push(Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        });
        self.text = snap.text;
        self.cursor = snap.cursor.min(self.text.len());
        self.anchor = snap.anchor.map(|a| a.min(self.text.len()));
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snap) = self.redo_stack.pop() else {
            return false;
        };
        self.undo_stack.push(Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        });
        self.text = snap.text;
        self.cursor = snap.cursor.min(self.text.len());
        self.anchor = snap.anchor.map(|a| a.min(self.text.len()));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_cursor() {
        let mut b = TextBuffer::single_line();
        b.insert("hello");
        assert_eq!(b.text(), "hello");
        assert_eq!(b.cursor(), 5);
        b.move_left(false);
        b.insert("X");
        assert_eq!(b.text(), "hellXo");
        assert_eq!(b.cursor(), 5);
    }

    #[test]
    fn insert_in_middle_with_cursor() {
        let mut b = TextBuffer::from_text("abcd", true);
        b.move_to(2, false);
        b.insert("ZZ");
        assert_eq!(b.text(), "abZZcd");
        assert_eq!(b.cursor(), 4);
    }

    #[test]
    fn backspace_handles_utf8() {
        let mut b = TextBuffer::from_text("中文", true);
        b.backspace();
        assert_eq!(b.text(), "中");
        b.backspace();
        assert_eq!(b.text(), "");
        b.backspace();
        assert_eq!(b.text(), "");
    }

    #[test]
    fn selection_replace() {
        let mut b = TextBuffer::from_text("hello world", true);
        b.move_to(0, false);
        b.move_to(5, true);
        assert_eq!(b.selected_text(), Some("hello"));
        b.insert("bye");
        assert_eq!(b.text(), "bye world");
        assert!(!b.has_selection());
    }

    #[test]
    fn undo_redo_roundtrip() {
        let mut b = TextBuffer::multi_line();
        b.insert("a");
        b.insert("b");
        assert_eq!(b.text(), "ab");
        assert!(b.undo());
        assert_eq!(b.text(), "a");
        assert!(b.undo());
        assert_eq!(b.text(), "");
        assert!(b.redo());
        assert_eq!(b.text(), "a");
        assert!(b.redo());
        assert_eq!(b.text(), "ab");
        assert!(!b.redo());
    }

    #[test]
    fn vertical_movement_keeps_column() {
        let mut b = TextBuffer::from_text("abcdef\nxy\nabcdef", true);
        b.move_to(3, false); // abc|def
        assert_eq!(b.column(), 3);
        b.move_down(false);
        assert_eq!(b.text()[..b.cursor()].lines().last().unwrap_or(""), "xy");
        // xy 行只有 2 列，光标落到该行行尾（byte 9）
        assert_eq!(b.cursor(), 9);
        b.move_down(false);
        assert_eq!(b.column(), 2);
    }

    #[test]
    fn home_end() {
        let mut b = TextBuffer::from_text("line one\nline two", true);
        b.move_home(false);
        assert_eq!(b.cursor(), 9);
        b.move_end(false);
        assert_eq!(b.cursor(), 17);
        b.move_home(false);
        b.move_doc_start(false);
        assert_eq!(b.cursor(), 0);
    }

    #[test]
    fn single_line_blocks_newlines() {
        let mut b = TextBuffer::single_line();
        b.insert("a\nb");
        assert_eq!(b.text(), "a b");
    }

    #[test]
    fn cut_selection() {
        let mut b = TextBuffer::from_text("hello world", true);
        b.select_all();
        let taken = b.take_selection();
        assert_eq!(taken.as_deref(), Some("hello world"));
        assert_eq!(b.text(), "");
        assert_eq!(b.cursor(), 0);
    }

    #[test]
    fn word_movement() {
        let mut b = TextBuffer::from_text("foo bar_baz qux", true);
        b.move_to(0, false);
        b.move_word_right(false);
        assert_eq!(b.cursor(), 3);
        b.move_word_right(false);
        assert_eq!(b.cursor(), 11);
        b.move_word_left(false);
        assert_eq!(b.cursor(), 4);
    }

    #[test]
    fn cursor_stays_on_char_boundary() {
        let mut b = TextBuffer::from_text("a中b", true);
        b.move_to(2, false); // 中间字节，应回退到 1
        assert!(b.text().is_char_boundary(b.cursor()));
        b.insert("X");
        assert_eq!(b.text(), "aX中b");
    }
}
