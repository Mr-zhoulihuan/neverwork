//! 编辑器内查找 / 替换。
//!
//! 设计取舍：
//! - 纯子串匹配（不做正则）。正则要引 regex 依赖，且用户在这个场景里 95% 是找一个字面量。
//! - 匹配在每次 query 变化时重算并缓存，`next/prev` 只是移动索引，不会每次全文扫描。
//! - 替换支持「替换当前」与「全部替换」，全部替换走一次整体重建，
//!   避免边替换边移动光标导致的错位。

use std::ops::Range;

use crate::text_buffer::TextBuffer;

/// 查找栏状态。
#[derive(Debug, Clone, Default)]
pub struct FindState {
    pub open: bool,
    pub query: String,
    pub replace: String,
    /// 命中的字节区间（升序）。
    pub matches: Vec<Range<usize>>,
    /// 当前命中下标。
    pub current: usize,
    /// 是否区分大小写。
    pub case_sensitive: bool,
    /// 焦点在替换框还是查找框。
    pub replace_focused: bool,
}

impl FindState {
    /// 重新计算命中。`query` 为空时清空。
    pub fn refresh(&mut self, text: &str) {
        self.matches.clear();
        self.current = 0;
        if self.query.is_empty() {
            return;
        }
        let hay = if self.case_sensitive {
            text.to_string()
        } else {
            text.to_lowercase()
        };
        let needle = if self.case_sensitive {
            self.query.clone()
        } else {
            self.query.to_lowercase()
        };
        // 小写化可能改变字节长度（某些语言），这里只用于 ASCII/常见场景；
        // 为保证区间正确，大小写不敏感时改用逐字符比较。
        if needle.len() == self.query.len() && hay.len() == text.len() {
            let mut from = 0usize;
            while let Some(pos) = hay[from..].find(&needle) {
                let s = from + pos;
                self.matches.push(s..s + needle.len());
                from = s + needle.len().max(1);
                if self.matches.len() > 5000 {
                    break;
                }
            }
        } else {
            self.matches = find_ci(text, &self.query);
        }
    }

    pub fn count(&self) -> usize {
        self.matches.len()
    }

    pub fn current_range(&self) -> Option<Range<usize>> {
        self.matches.get(self.current).cloned()
    }

    /// 下一个命中（循环）。
    pub fn next(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.current = (self.current + 1) % self.matches.len();
    }

    /// 上一个命中（循环）。
    pub fn prev(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.current = if self.current == 0 {
            self.matches.len() - 1
        } else {
            self.current - 1
        };
    }
}

/// 大小写不敏感的子串查找，返回字节区间。
///
/// 用逐字符推进而不是 `to_lowercase()` 整串替换，避免小写化改变字节长度导致区间错位。
fn find_ci(text: &str, needle: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let n: Vec<char> = needle.chars().flat_map(|c| c.to_lowercase()).collect();
    if n.is_empty() {
        return out;
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let low: Vec<char> = chars
        .iter()
        .flat_map(|(_, c)| c.to_lowercase())
        .collect();
    // low 与 chars 的长度可能不同（极少数字符小写后变多个），此时退化到逐位置比较。
    if low.len() != chars.len() {
        let lower = text.to_lowercase();
        let mut from = 0;
        let nl = needle.to_lowercase();
        while let Some(pos) = lower[from..].find(&nl) {
            let s = from + pos;
            out.push(s..s + nl.len());
            from = s + nl.len().max(1);
            if out.len() > 5000 {
                break;
            }
        }
        return out;
    }

    let mut i = 0usize;
    while i + n.len() <= low.len() {
        if low[i..i + n.len()] == n[..] {
            let start = chars[i].0;
            let end = if i + n.len() < chars.len() {
                chars[i + n.len()].0
            } else {
                text.len()
            };
            out.push(start..end);
            if out.len() > 5000 {
                break;
            }
            i += n.len().max(1);
        } else {
            i += 1;
        }
    }
    out
}

/// 把光标/选区挪到某个命中。
pub fn select_match(buf: &mut TextBuffer, range: &Range<usize>) {
    buf.move_to(range.start, false);
    buf.move_to(range.end, true);
}

/// 替换当前命中，返回是否成功。
pub fn replace_current(buf: &mut TextBuffer, find: &mut FindState) -> bool {
    let Some(range) = find.current_range() else {
        return false;
    };
    // 把选区对齐到这个命中再插入，等价于「替换这段」。
    select_match(buf, &range);
    buf.insert(&find.replace.clone());
    // 替换后内容变了，重新扫描并尽量停在原下标。
    let old_current = find.current;
    find.refresh(buf.text());
    if !find.matches.is_empty() {
        find.current = old_current.min(find.matches.len() - 1);
    }
    true
}

/// 全部替换，返回替换次数。
pub fn replace_all(buf: &mut TextBuffer, find: &mut FindState) -> usize {
    find.refresh(buf.text());
    if find.matches.is_empty() {
        return 0;
    }
    let n = find.matches.len();
    let mut text = buf.text().to_string();
    // 从后往前替换，前面的区间就不会失效。
    for r in find.matches.iter().rev() {
        text.replace_range(r.clone(), &find.replace);
    }
    let pos = buf.cursor().min(text.len());
    buf.reset(text);
    buf.move_to(pos, false);
    find.refresh(buf.text());
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(s: &str) -> TextBuffer {
        TextBuffer::from_text(s, true)
    }

    #[test]
    fn finds_all_occurrences() {
        let mut f = FindState {
            query: "ab".into(),
            ..Default::default()
        };
        f.refresh("ab x ab y ab");
        assert_eq!(f.count(), 3);
        assert_eq!(f.matches[0], 0..2);
        assert_eq!(f.matches[1], 5..7);
    }

    #[test]
    fn empty_query_has_no_matches() {
        let mut f = FindState::default();
        f.refresh("anything");
        assert_eq!(f.count(), 0);
        assert!(f.current_range().is_none());
    }

    #[test]
    fn next_wraps_around() {
        let mut f = FindState {
            query: "a".into(),
            ..Default::default()
        };
        f.refresh("a a");
        assert_eq!(f.current, 0);
        f.next();
        assert_eq!(f.current, 1);
        f.next();
        assert_eq!(f.current, 0, "应循环回第一个");
        f.prev();
        assert_eq!(f.current, 1, "prev 从 0 回到末尾");
    }

    #[test]
    fn case_insensitive_by_default() {
        let mut f = FindState {
            query: "hello".into(),
            ..Default::default()
        };
        f.refresh("Hello HELLO hello");
        assert_eq!(f.count(), 3);
    }

    #[test]
    fn case_sensitive_opt_in() {
        let mut f = FindState {
            query: "Hello".into(),
            case_sensitive: true,
            ..Default::default()
        };
        f.refresh("Hello HELLO hello");
        assert_eq!(f.count(), 1);
    }

    #[test]
    fn cjk_search_works_with_byte_offsets() {
        let mut f = FindState {
            query: "第二".into(),
            ..Default::default()
        };
        f.refresh("第一行\n第二行\n第三行");
        assert_eq!(f.count(), 1);
        let r = &f.matches[0];
        assert_eq!(&"第一行\n第二行\n第三行"[r.clone()], "第二");
    }

    #[test]
    fn replace_current_only_replaces_one() {
        let mut b = buf("a a a");
        let mut f = FindState {
            query: "a".into(),
            replace: "X".into(),
            ..Default::default()
        };
        f.refresh(b.text());
        assert!(replace_current(&mut b, &mut f));
        assert_eq!(b.text(), "X a a");
    }

    #[test]
    fn replace_all_replaces_every_occurrence() {
        let mut b = buf("a a a");
        let mut f = FindState {
            query: "a".into(),
            replace: "X".into(),
            ..Default::default()
        };
        assert_eq!(replace_all(&mut b, &mut f), 3);
        assert_eq!(b.text(), "X X X");
    }

    #[test]
    fn replace_all_with_different_length() {
        let mut b = buf("foo bar foo");
        let mut f = FindState {
            query: "foo".into(),
            replace: "longer".into(),
            ..Default::default()
        };
        assert_eq!(replace_all(&mut b, &mut f), 2);
        assert_eq!(b.text(), "longer bar longer");
    }

    #[test]
    fn replace_all_cjk() {
        let mut b = buf("第一行\n第二行");
        let mut f = FindState {
            query: "行".into(),
            replace: "条".into(),
            ..Default::default()
        };
        assert_eq!(replace_all(&mut b, &mut f), 2);
        assert_eq!(b.text(), "第一条\n第二条");
    }

    #[test]
    fn select_match_sets_selection() {
        let mut b = buf("hello world");
        let mut f = FindState {
            query: "world".into(),
            ..Default::default()
        };
        f.refresh(b.text());
        let r = f.current_range().unwrap();
        select_match(&mut b, &r);
        assert_eq!(b.selected_text(), Some("world"));
    }

    #[test]
    fn no_match_replace_returns_zero() {
        let mut b = buf("abc");
        let mut f = FindState {
            query: "zzz".into(),
            replace: "x".into(),
            ..Default::default()
        };
        assert_eq!(replace_all(&mut b, &mut f), 0);
        assert_eq!(b.text(), "abc");
    }
}
