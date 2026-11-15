//! 轻量语法高亮：按文件扩展名识别语言，把一行切分为带语义颜色的片段。
//!
//! 不引入重型 parser：用简单的词法扫描（关键字 / 字符串 / 注释 / 数字 / 类型/标识符）。
//! 目标是在编辑器里给出可读的着色，而非精确解析。

use neverwork_ui::theme;
use gpui::Rgba;

/// 语义类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    Plain,
    Keyword,
    Str,
    Comment,
    Number,
    Type,
    Func,
}

/// 一个着色片段。
#[derive(Debug, Clone)]
pub struct Span {
    pub text: String,
    pub tok: Tok,
}

/// 支持的语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Ts,
    Python,
    Json,
    Toml,
    Markdown,
    Other,
}

/// 根据文件名推断语言。
pub fn lang_for(filename: &str) -> Lang {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "rs" => Lang::Rust,
        "ts" | "tsx" | "js" | "jsx" => Lang::Ts,
        "py" => Lang::Python,
        "json" => Lang::Json,
        "toml" => Lang::Toml,
        "md" | "markdown" => Lang::Markdown,
        _ => Lang::Other,
    }
}

/// 语义类别 -> 颜色（对齐设计稿的语法色）。
pub fn color_for(tok: Tok) -> Rgba {
    match tok {
        Tok::Plain => theme::fg(),
        Tok::Keyword => gpui::rgb(0xc586c0), // 紫
        Tok::Str => gpui::rgb(0xce9178),     // 橙棕
        Tok::Comment => gpui::rgb(0x6a9955), // 绿
        Tok::Number => gpui::rgb(0xb5cea8),  // 浅绿
        Tok::Type => gpui::rgb(0x4ec9b0),    // 青
        Tok::Func => gpui::rgb(0xdcdcaa),    // 黄
    }
}

fn keywords(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Rust => &[
            "fn", "let", "mut", "pub", "use", "mod", "struct", "enum", "impl", "trait", "for", "while",
            "loop", "if", "else", "match", "return", "async", "await", "move", "self", "Self", "crate",
            "super", "const", "static", "ref", "as", "in", "where", "dyn", "type", "unsafe", "extern",
            "true", "false", "break", "continue",
        ],
        Lang::Ts => &[
            "function", "const", "let", "var", "return", "if", "else", "for", "while", "class", "interface",
            "type", "import", "export", "from", "async", "await", "new", "this", "true", "false", "null",
            "undefined", "extends", "implements", "public", "private", "static",
        ],
        Lang::Python => &[
            "def", "class", "return", "if", "elif", "else", "for", "while", "import", "from", "as", "with",
            "try", "except", "finally", "lambda", "None", "True", "False", "and", "or", "not", "in", "is",
            "async", "await", "yield",
        ],
        Lang::Json | Lang::Toml => &["true", "false", "null"],
        Lang::Markdown | Lang::Other => &[],
    }
}

/// 把一行文本切分为着色片段。
pub fn highlight_line(line: &str, lang: Lang) -> Vec<Span> {
    // Markdown / 其他：整行普通（标题可粗体由 UI 处理，这里简化）。
    if matches!(lang, Lang::Markdown | Lang::Other) {
        return vec![Span { text: line.to_string(), tok: Tok::Plain }];
    }

    let kws = keywords(lang);
    let chars: Vec<char> = line.chars().collect();
    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let mut i = 0;

    let flush_plain = |plain: &mut String, spans: &mut Vec<Span>| {
        if !plain.is_empty() {
            spans.push(Span { text: std::mem::take(plain), tok: Tok::Plain });
        }
    };

    while i < chars.len() {
        let c = chars[i];

        // 行注释：// 或 #
        if (c == '/' && i + 1 < chars.len() && chars[i + 1] == '/')
            || (c == '#' && matches!(lang, Lang::Python | Lang::Toml))
        {
            flush_plain(&mut plain, &mut spans);
            spans.push(Span { text: chars[i..].iter().collect(), tok: Tok::Comment });
            return spans;
        }

        // 字符串："..." 或 '...'
        if c == '"' || c == '\'' {
            flush_plain(&mut plain, &mut spans);
            let quote = c;
            let mut s = String::new();
            s.push(quote);
            i += 1;
            while i < chars.len() {
                let ch = chars[i];
                s.push(ch);
                i += 1;
                if ch == '\\' && i < chars.len() {
                    s.push(chars[i]);
                    i += 1;
                } else if ch == quote {
                    break;
                }
            }
            spans.push(Span { text: s, tok: Tok::Str });
            continue;
        }

        // 数字
        if c.is_ascii_digit() {
            flush_plain(&mut plain, &mut spans);
            let mut s = String::new();
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_') {
                s.push(chars[i]);
                i += 1;
            }
            spans.push(Span { text: s, tok: Tok::Number });
            continue;
        }

        // 标识符 / 关键字
        if c.is_alphabetic() || c == '_' {
            flush_plain(&mut plain, &mut spans);
            let mut word = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                word.push(chars[i]);
                i += 1;
            }
            let tok = if kws.contains(&word.as_str()) {
                Tok::Keyword
            } else if word.chars().next().map(|ch| ch.is_uppercase()).unwrap_or(false) {
                Tok::Type
            } else {
                // 后面紧跟 '(' 视为函数调用
                let mut j = i;
                while j < chars.len() && chars[j] == ' ' {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '(' {
                    Tok::Func
                } else {
                    Tok::Plain
                }
            };
            spans.push(Span { text: word, tok });
            continue;
        }

        plain.push(c);
        i += 1;
    }

    flush_plain(&mut plain, &mut spans);
    if spans.is_empty() {
        spans.push(Span { text: String::new(), tok: Tok::Plain });
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str, lang: Lang) -> Vec<Tok> {
        highlight_line(line, lang).into_iter().map(|s| s.tok).collect()
    }

    #[test]
    fn rust_keyword_and_string() {
        let spans = highlight_line("let x = \"hi\";", Lang::Rust);
        // 包含关键字 let、字符串 "hi"
        assert!(spans.iter().any(|s| s.tok == Tok::Keyword && s.text == "let"));
        assert!(spans.iter().any(|s| s.tok == Tok::Str && s.text == "\"hi\""));
    }

    #[test]
    fn rust_comment_to_end() {
        let spans = highlight_line("fn a() {} // trailing", Lang::Rust);
        let last = spans.last().unwrap();
        assert_eq!(last.tok, Tok::Comment);
        assert_eq!(last.text, "// trailing");
    }

    #[test]
    fn number_detected() {
        let spans = highlight_line("let n = 42;", Lang::Rust);
        assert!(spans.iter().any(|s| s.tok == Tok::Number && s.text == "42"));
    }

    #[test]
    fn uppercase_is_type() {
        let spans = highlight_line("let v: Vec = x;", Lang::Rust);
        assert!(spans.iter().any(|s| s.tok == Tok::Type && s.text == "Vec"));
    }

    #[test]
    fn func_call_detected() {
        let spans = highlight_line("foo();", Lang::Rust);
        assert!(spans.iter().any(|s| s.tok == Tok::Func && s.text == "foo"));
    }

    #[test]
    fn python_comment_hash() {
        let kinds = kinds("x = 1  # note", Lang::Python);
        assert_eq!(*kinds.last().unwrap(), Tok::Comment);
    }
}

