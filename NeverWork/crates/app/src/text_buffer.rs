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
    /// 额外光标（多光标）。主光标仍是 `cursor`，这里只存"附加点"。
    /// 所有编辑操作在 `*_multi` 系列里统一处理，单光标路径完全不受影响。
    extra_cursors: Vec<usize>,
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

/// 取 `pos` 所在「词」的字节区间。
///
/// 中文等非 ASCII 字符也当作词字符，这样 Ctrl+D 对中文同样有效。
pub fn word_bounds(s: &str, pos: usize) -> (usize, usize) {
    let pos = pos.min(s.len());
    let pos = {
        let mut p = pos;
        while p > 0 && !s.is_char_boundary(p) {
            p -= 1;
        }
        p
    };
    let mut start = pos;
    while start > 0 {
        let prev = prev_boundary(s, start);
        let ch = s[prev..start].chars().next().unwrap_or(' ');
        if !is_word_char(ch) {
            break;
        }
        start = prev;
    }
    let mut end = pos;
    while end < s.len() {
        let next = next_boundary(s, end);
        let ch = s[end..next].chars().next().unwrap_or(' ');
        if !is_word_char(ch) {
            break;
        }
        end = next;
    }
    (start, end)
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
            extra_cursors: Vec::new(),
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
        self.extra_cursors.clear();
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

    // ===== 多光标 =====

    /// 全部光标（含主光标），已排序去重。
    pub fn cursors(&self) -> Vec<usize> {
        let mut v = self.extra_cursors.clone();
        v.push(self.cursor);
        v.sort_unstable();
        v.dedup();
        v
    }

    pub fn cursor_count(&self) -> usize {
        self.cursors().len()
    }

    pub fn has_multi_cursor(&self) -> bool {
        !self.extra_cursors.is_empty()
    }

    /// 在 `pos` 加一个光标（已存在则忽略）。
    pub fn add_cursor(&mut self, pos: usize) {
        let p = self.clamp(pos);
        if p != self.cursor && !self.extra_cursors.contains(&p) {
            self.extra_cursors.push(p);
        }
    }

    /// 清掉额外光标，只留主光标。
    pub fn clear_extra_cursors(&mut self) {
        self.extra_cursors.clear();
        self.anchor = None;
    }

    /// 每个光标独立左右移动一个字符。返回是否有变化。
    pub fn move_all(&mut self, forward: bool) {
        if self.extra_cursors.is_empty() {
            if forward {
                self.move_right(false);
            } else {
                self.move_left(false);
            }
            return;
        }
        let mut next: Vec<usize> = Vec::new();
        for p in self.cursors() {
            let np = if forward {
                next_boundary(&self.text, p)
            } else {
                prev_boundary(&self.text, p)
            };
            if !next.contains(&np) {
                next.push(np);
            }
        }
        // 第一个作为主光标，其余作为额外光标
        next.sort_unstable();
        self.cursor = next[0];
        self.extra_cursors = next[1..].to_vec();
        self.anchor = None;
    }

    /// 在所有光标处插入同一段文本。
    ///
    /// 从后往前插，前面的偏移就不会被后续插入影响；
    /// 整批只记一次撤销快照，撤销时一次全撤（符合直觉）。
    pub fn insert_multi(&mut self, s: &str) {
        if self.extra_cursors.is_empty() {
            self.insert(s);
            return;
        }
        let s: String = if self.multiline {
            s.to_string()
        } else {
            s.replace(['\n', '\r'], " ")
        };
        if s.is_empty() {
            return;
        }
        let mut cursors = self.cursors();
        cursors.sort_unstable();
        cursors.dedup();
        self.record();

        // 从后往前插，前面的偏移不受影响
        for &p in cursors.iter().rev() {
            self.text.insert_str(p, &s);
        }
        // 原第 i 个光标（升序下标）最终落在 p + i*len + len：
        // 它前面有 i 段插入，再加上自己这段。
        let new_cursors: Vec<usize> = cursors
            .iter()
            .enumerate()
            .map(|(i, &p)| p + i * s.len() + s.len())
            .collect();
        self.cursor = *new_cursors.last().unwrap_or(&self.cursor);
        self.extra_cursors = new_cursors[..new_cursors.len().saturating_sub(1)].to_vec();
        self.anchor = None;
    }

    /// 在所有光标处退格（删前一个字符）。
    pub fn backspace_multi(&mut self) {
        if self.extra_cursors.is_empty() {
            self.backspace();
            return;
        }
        let mut cursors = self.cursors();
        cursors.sort_unstable();
        self.record();

        // 先算出每个光标要删的区间（基于原始文本），再一次性从后往前删。
        // 直接边删边算会读到已被改动的位置，导致删错字符。
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for &p in &cursors {
            if p == 0 {
                continue;
            }
            let prev = prev_boundary(&self.text, p);
            ranges.push((prev, p));
        }
        ranges.sort_unstable();
        // 区间可能重叠（光标挨得很近），合并后用「删除点」记录新光标位置
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (s, e) in ranges {
            match merged.last_mut() {
                // 只有真正重叠才合并；相邻（s == last.1）是两段独立删除
                Some(last) if s < last.1 => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }

        let mut new_cursors: Vec<usize> = Vec::new();
        for &p in &cursors {
            if p == 0 {
                new_cursors.push(0);
                continue;
            }
            // 该光标删掉的是它前面那段；新位置 = 该段起点 - 前面已删的量
            let prev = prev_boundary(&self.text, p);
            let removed_before: usize = merged
                .iter()
                .filter(|(_rs, re)| *re <= prev)
                .map(|(rs, re)| re - rs)
                .sum();
            new_cursors.push(prev - removed_before);
        }

        for (s, e) in merged.iter().rev() {
            self.text.replace_range(*s..*e, "");
        }

        new_cursors.sort_unstable();
        new_cursors.dedup();
        if new_cursors.is_empty() {
            return;
        }
        self.cursor = *new_cursors.last().unwrap();
        self.extra_cursors = new_cursors[..new_cursors.len() - 1].to_vec();
        self.anchor = None;
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

#[cfg(test)]
mod multi_cursor_tests {
    use super::*;

    fn buf(s: &str) -> TextBuffer {
        TextBuffer::from_text(s, true)
    }

    #[test]
    fn single_cursor_by_default() {
        let b = buf("abc");
        assert_eq!(b.cursor_count(), 1);
        assert!(!b.has_multi_cursor());
    }

    #[test]
    fn add_cursor_dedups() {
        let mut b = buf("abcdef");
        b.move_to(1, false);
        b.add_cursor(3);
        b.add_cursor(3); // 重复
        assert_eq!(b.cursor_count(), 2);
        b.add_cursor(1); // 与主光标相同
        assert_eq!(b.cursor_count(), 2);
    }

    #[test]
    fn insert_at_all_cursors() {
        let mut b = buf("aaa");
        b.move_to(0, false);
        b.add_cursor(1);
        b.add_cursor(2);
        b.insert_multi("X");
        // 三个位置各插一个 X，从后往前
        assert_eq!(b.text(), "XaXaXa");
        assert_eq!(b.cursor_count(), 3);
    }

    #[test]
    fn insert_multi_undo_once() {
        let mut b = buf("aaa");
        b.move_to(0, false);
        b.add_cursor(1);
        b.add_cursor(2);
        b.insert_multi("X");
        assert_eq!(b.text(), "XaXaXa");
        assert!(b.undo(), "应能撤销");
        assert_eq!(b.text(), "aaa", "一次撤销应撤掉全部插入");
    }

    #[test]
    fn backspace_at_all_cursors() {
        // "abc"，光标分别停在 a、b、c 之后 → 三个字符都该被删掉
        let mut b = buf("abc");
        b.move_to(3, false);
        b.add_cursor(2);
        b.add_cursor(1);
        b.backspace_multi();
        assert_eq!(b.text(), "");
        // 都退到 0 附近
        for c in b.cursors() {
            assert_eq!(c, 0);
        }
    }

    #[test]
    fn backspace_multi_handles_position_zero() {
        let mut b = buf("ab");
        b.move_to(0, false);
        b.add_cursor(2);
        b.backspace_multi();
        // 位置 0 不动，位置 2 删掉 b
        assert_eq!(b.text(), "a");
        assert_eq!(b.cursor_count(), 2);
    }

    #[test]
    fn move_all_keeps_relative_order() {
        let mut b = buf("abcdef");
        b.move_to(1, false);
        b.add_cursor(4);
        b.move_all(true);
        let cs = b.cursors();
        assert_eq!(cs, vec![2, 5]);
    }

    #[test]
    fn move_all_backward_stops_at_zero() {
        let mut b = buf("abc");
        b.move_to(0, false);
        b.add_cursor(2);
        b.move_all(false);
        let cs = b.cursors();
        assert_eq!(cs[0], 0, "0 不会再往前");
        assert_eq!(cs[1], 1);
    }

    #[test]
    fn clear_extra_keeps_primary() {
        let mut b = buf("abcdef");
        b.move_to(4, false);
        b.add_cursor(1);
        b.clear_extra_cursors();
        assert_eq!(b.cursor_count(), 1);
        assert_eq!(b.cursor(), 4, "主光标保持不动");
    }

    #[test]
    fn single_cursor_falls_back_to_normal_insert() {
        let mut b = buf("ab");
        b.move_to(1, false);
        b.insert_multi("X");
        assert_eq!(b.text(), "aXb");
    }

    #[test]
    fn multi_cursor_cjk_safe() {
        let mut b = buf("中文测试");
        b.move_to(0, false);
        b.add_cursor(6); // "中" 之后是 "文"
        b.insert_multi("-");
        assert_eq!(b.text(), "-中文-测试");
        // 每个光标都应在 char boundary 上
        for c in b.cursors() {
            assert!(b.text().is_char_boundary(c), "光标 {c} 不在字符边界");
        }
    }

    #[test]
    fn reset_clears_extra_cursors() {
        let mut b = buf("abc");
        b.add_cursor(1);
        b.reset("new");
        assert_eq!(b.cursor_count(), 1);
    }
}
