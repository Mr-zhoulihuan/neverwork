//! 括号匹配：找出光标处括号的配对位置。
//!
//! 只做「同类型括号按嵌套配对」，不解析语法。这已经覆盖了实际需求：
//! 输入 `}` 时高亮配对的 `{`、光标停在括号上时提示它在哪配对的。
//!
//! 跳过字符串与注释里的括号——否则 `"}{"` 这种内容会让配对彻底错乱。
//! 字符串/注释的识别直接复用高亮器的词法扫描结果，避免再写一套。

use crate::highlight::{highlight_line, Lang, Tok};

/// 一对括号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BracketPair {
    /// 光标所在的括号位置（字节偏移）。
    pub at: usize,
    /// 配对的另一个括号位置。
    pub mate: usize,
    pub ch: char,
    pub mate_ch: char,
}

impl BracketPair {
    /// 配对是否成功成对（比如 `{` 配 `}`）。
    pub fn is_balanced(&self) -> bool {
        matches!(
            (self.ch, self.mate_ch),
            ('(', ')') | ('[', ']') | ('{', '}') | (')', '(') | (']', '[') | ('}', '{')
        )
    }
}

fn is_open(c: char) -> bool {
    matches!(c, '(' | '[' | '{')
}

fn is_close(c: char) -> bool {
    matches!(c, ')' | ']' | '}')
}

fn mate_of(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        other => other,
    }
}

/// 判断某个字节偏移处的字符是否在字符串/注释里。
///
/// 用高亮器的 span 结果判断：`Str` / `Comment` 里的括号不参与配对。
fn masked_positions(line: &str, lang: Lang) -> Vec<(usize, usize)> {
    let spans = highlight_line(line, lang);
    let mut out = Vec::new();
    let mut off = 0usize;
    for sp in &spans {
        let len = sp.text.len();
        if matches!(sp.tok, Tok::Str | Tok::Comment) {
            out.push((off, off + len));
        }
        off += len;
    }
    out
}

fn is_masked(pos: usize, masked: &[(usize, usize)]) -> bool {
    masked.iter().any(|(s, e)| pos >= *s && pos < *e)
}

/// 在 `text` 里找 `pos` 处括号的配对。
///
/// `pos` 可以是括号本身，也可以是紧跟在括号之后的位置（光标常见落点）。
/// 返回 `None` 表示：不在括号上，或找不到配对。
pub fn find_pair(text: &str, pos: usize, lang: Lang) -> Option<BracketPair> {
    if pos > text.len() {
        return None;
    }

    // 1. 抽出全文里「有效」的括号（跳过字符串/注释），带字节偏移
    let brackets = collect_brackets(text, lang);
    if brackets.is_empty() {
        return None;
    }

    // 2. 定位光标站在哪个括号上：先看 pos，再看前一个字符
    let target = brackets
        .iter()
        .position(|(p, _)| *p == pos)
        .or_else(|| {
            if pos == 0 {
                return None;
            }
            let prev = crate::text_buffer::prev_boundary(text, pos);
            brackets.iter().position(|(p, _)| *p == prev)
        })?;

    let (at, ch) = brackets[target];

    // 3. 沿一个方向做深度计数
    let mate = if is_open(ch) {
        // 往后找：同类型 +1，配对类型 -1，归零即命中
        let want = mate_of(ch);
        let mut depth = 0i32;
        let mut found = None;
        for &(p, c) in &brackets[target..] {
            if c == ch {
                depth += 1;
            } else if c == want {
                depth -= 1;
                if depth == 0 {
                    found = Some(p);
                    break;
                }
            }
        }
        found
    } else {
        // 往前找：同样深度计数，方向反过来
        let want = mate_of(ch);
        let mut depth = 0i32;
        let mut found = None;
        for &(p, c) in brackets[..=target].iter().rev() {
            if c == ch {
                depth += 1;
            } else if c == want {
                depth -= 1;
                if depth == 0 {
                    found = Some(p);
                    break;
                }
            }
        }
        found
    }?;

    Some(BracketPair {
        at,
        mate,
        ch,
        mate_ch: mate_of(ch),
    })
}

/// 收集全文有效括号（按出现顺序），跳过字符串与注释内的。
fn collect_brackets(text: &str, lang: Lang) -> Vec<(usize, char)> {
    let mut out = Vec::new();
    let mut abs = 0usize;
    for line in text.split('\n') {
        let masked = masked_positions(line, lang);
        for (off, c) in line.char_indices() {
            if is_masked(off, &masked) {
                continue;
            }
            if is_open(c) || is_close(c) {
                out.push((abs + off, c));
            }
        }
        abs += line.len() + 1;
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    fn pair(text: &str, pos: usize) -> Option<BracketPair> {
        find_pair(text, pos, Lang::Rust)
    }

    #[test]
    fn matches_simple_braces() {
        let t = "fn f() { abc }";
        let p = pair(t, 7).expect("应找到配对");
        assert_eq!(p.ch, '{');
        assert_eq!(t[p.mate..].chars().next(), Some('}'));
        assert!(p.is_balanced());
    }

    #[test]
    fn matches_from_closing_side() {
        let t = "fn f() { abc }";
        let close = t.rfind('}').unwrap();
        let p = pair(t, close).expect("从右括号也应找到配对");
        assert_eq!(p.ch, '}');
        assert_eq!(p.mate, 7);
    }

    #[test]
    fn handles_nesting() {
        let t = "{ a { b } c }";
        let outer = pair(t, 0).unwrap();
        assert_eq!(outer.mate, t.len() - 1, "外层应配到最后一个");
        // "{ a { b } c }" 内层 { 在 4，配对的 } 在 8
        let inner = pair(t, 4).unwrap();
        assert_eq!(inner.mate, 8, "内层应配到自己的收尾");
    }

    #[test]
    fn ignores_braces_in_string() {
        // 字符串里的 { 不应参与配对
        let t = r#"fn f() { let s = "}"; }"#;
        let p = pair(t, 7).unwrap();
        let mate_ch = t[p.mate..].chars().next().unwrap();
        assert_eq!(mate_ch, '}', "应配到真正的收尾括号");
        assert!(p.mate > t.rfind("\"}").unwrap());
    }

    #[test]
    fn ignores_braces_in_comment() {
        let t = "fn f() { // }\n}";
        let p = pair(t, 7).unwrap();
        let mate_ch = t[p.mate..].chars().next().unwrap();
        assert_eq!(mate_ch, '}');
        // 不应配到注释里的那个
        assert_ne!(p.mate, 13);
    }

    #[test]
    fn returns_none_when_not_on_bracket() {
        let t = "fn main() {}";
        assert!(pair(t, 1).is_none(), "在 'n' 上不该有配对");
    }

    #[test]
    fn returns_none_when_unbalanced() {
        let t = "fn f() { abc";
        assert!(pair(t, 7).is_none(), "没有收尾括号时返回 None");
    }

    #[test]
    fn works_on_brackets_and_parens() {
        let t = "let v = a[0];";
        let i = t.find('[').unwrap();
        let p = pair(t, i).unwrap();
        assert_eq!(p.ch, '[');
        let m = p.mate;
        assert_eq!(t[m..].chars().next(), Some(']'));
    }

    #[test]
    fn cursor_after_bracket_still_matches() {
        // 光标常停在右括号后面一格
        let t = "fn f() { x }";
        let after = t.len();
        let p = pair(t, after).expect("光标在右括号之后也应能匹配");
        assert!(p.is_balanced());
    }

    #[test]
    fn chinese_content_does_not_break_offsets() {
        let t = "fn f() { 中文 }";
        let p = pair(t, 7).unwrap();
        let m = p.mate;
        assert!(t.is_char_boundary(m));
        assert_eq!(t[m..].chars().next(), Some('}'));
    }

    #[test]
    fn unbalanced_mix_is_reported() {
        let p = BracketPair {
            at: 0,
            mate: 1,
            ch: '{',
            mate_ch: ']',
        };
        assert!(!p.is_balanced());
    }

    #[test]
    fn mate_of_is_symmetric() {
        assert_eq!(mate_of('{'), '}');
        assert_eq!(mate_of('}'), '{');
        assert_eq!(mate_of('['), ']');
        assert_eq!(mate_of(')'), '(');
    }

    #[test]
    fn empty_text_is_safe() {
        assert!(pair("", 0).is_none());
    }
}
