//! 编辑器行的缓存：行起始偏移 + 每行高亮片段。
//!
//! 之前每次按键都把整个文件重新 `split('\n')` 并对每一行重跑词法扫描，
//! 文件稍大就明显掉帧。现在按全文哈希缓存，未变化时直接复用。

use std::rc::Rc;

use crate::highlight::{self, Lang, Span};

pub struct EditorLines {
    /// 每行起始的字节偏移。
    pub starts: Vec<usize>,
    /// 每行原文（缓存成 Rc，渲染时只克隆指针）。
    pub texts: Vec<Rc<str>>,
    /// 每行的着色片段（已缓存，克隆只增计数）。
    pub spans: Vec<Rc<Vec<Span>>>,
}

impl EditorLines {
    pub fn line_count(&self) -> usize {
        self.spans.len()
    }

    pub fn line_start(&self, idx: usize) -> usize {
        self.starts.get(idx).copied().unwrap_or(0)
    }
}

pub struct LineCache {
    hash: u64,
    lang: Lang,
    lines: Rc<EditorLines>,
}

impl Default for LineCache {
    fn default() -> Self {
        Self::new()
    }
}

impl LineCache {
    pub fn new() -> Self {
        Self {
            hash: 0,
            lang: Lang::Other,
            lines: Rc::new(EditorLines {
                starts: vec![0],
                texts: vec![Rc::from("")],
                spans: vec![Rc::new(Vec::new())],
            }),
        }
    }

    /// 同步文本；内容与语言都没变时零成本返回缓存。
    pub fn sync(&mut self, text: &str, lang: Lang) -> Rc<EditorLines> {
        let h = fnv1a(text.as_bytes());
        if h == self.hash && lang == self.lang {
            return self.lines.clone();
        }
        let mut starts = Vec::new();
        let mut texts: Vec<Rc<str>> = Vec::new();
        let mut spans = Vec::new();
        let mut acc = 0usize;
        starts.push(0);
        // 跨行状态（块注释 / 三引号 / 反引号字符串）逐行传下去：
        // 否则注释或字符串的第二行起会失去颜色。
        let mut state = highlight::LineState::default();
        for line in text.split('\n') {
            texts.push(Rc::from(line));
            spans.push(Rc::new(highlight::highlight_line_stateful(
                line, lang, &mut state,
            )));
            acc += line.len() + 1;
            starts.push(acc);
        }
        // starts 比 spans 多一个（末尾哨兵），去掉以保持一一对应。
        starts.pop();

        self.hash = h;
        self.lang = lang;
        self.lines = Rc::new(EditorLines { starts, texts, spans });
        self.lines.clone()
    }

    /// 内容被外部替换后强制失效。
    pub fn invalidate(&mut self) {
        self.hash = 0;
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_until_text_or_lang_changes() {
        let mut cache = LineCache::new();
        let a = cache.sync("fn main() {}\nlet x = 1;", Lang::Rust);
        let b = cache.sync("fn main() {}\nlet x = 1;", Lang::Rust);
        assert!(Rc::ptr_eq(&a, &b), "内容未变应复用同一份缓存");

        let c = cache.sync("fn main() {}", Lang::Rust);
        assert!(!Rc::ptr_eq(&a, &c), "内容变化应重建");

        let d = cache.sync("fn main() {}", Lang::Ts);
        assert!(!Rc::ptr_eq(&c, &d), "语言变化应重建");
    }

    #[test]
    fn line_starts_are_byte_accurate() {
        let mut cache = LineCache::new();
        let text = "ab\n中文\ncd";
        let lines = cache.sync(text, Lang::Other);
        assert_eq!(lines.line_count(), 3);
        assert_eq!(lines.line_start(0), 0);
        assert_eq!(lines.line_start(1), 3);
        assert_eq!(lines.line_start(2), 10);
        // 每一行起点都必须是 char boundary
        for i in 0..lines.line_count() {
            assert!(text.is_char_boundary(lines.line_start(i)));
        }
    }

    #[test]
    fn empty_text_has_one_line() {
        let mut cache = LineCache::new();
        let lines = cache.sync("", Lang::Rust);
        assert_eq!(lines.line_count(), 1);
    }

    #[test]
    fn multiline_comment_keeps_state_across_lines() {
        let mut cache = LineCache::new();
        let lines = cache.sync("/* a\nb */ let x = 1;", Lang::Rust);
        let second = &lines.spans[1];
        assert_eq!(second[0].tok, crate::highlight::Tok::Comment);
        assert!(second
            .iter()
            .any(|s| s.tok == crate::highlight::Tok::Keyword && s.text == "let"));
    }
}
