//! 轻量语法高亮：按文件扩展名识别语言，把一行切分为带语义颜色的片段。
//!
//! 不引入重型 parser：用简单的词法扫描（关键字 / 字符串 / 注释 / 数字 / 类型 /
//! 函数名 / 属性名）。目标是在编辑器里给出可读的着色，而非精确解析。
//!
//! 跨行状态（块注释、Go 反引号字符串、Python 三引号）记在 [`LineState`] 里，
//! 由 `LineCache` 逐行传下去；[`highlight_line`] 是单行无状态版本，供括号匹配
//! 等只需要「本行字符串/注释范围」的调用方使用。

use gpui::Rgba;
use neverwork_ui::theme;

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
    /// 键 / 属性名：YAML 的 key、CSS 属性、HTML 属性。
    Attr,
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
    Go,
    Ts,
    Python,
    Json,
    Toml,
    Yaml,
    Markdown,
    Html,
    Css,
    Sql,
    Shell,
    /// C / C++ / C# / Java / Kotlin / Swift / PHP…（花括号家族）
    CLike,
    Ruby,
    Lua,
    Other,
}

/// 根据文件名推断语言。
pub fn lang_for(filename: &str) -> Lang {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" => Lang::Rust,
        "go" => Lang::Go,
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Lang::Ts,
        "py" | "pyi" => Lang::Python,
        "json" | "jsonc" | "json5" | "jsonl" => Lang::Json,
        "toml" => Lang::Toml,
        "yaml" | "yml" => Lang::Yaml,
        "md" | "markdown" | "mdx" => Lang::Markdown,
        "html" | "htm" | "xhtml" | "xml" | "svg" | "vue" | "svelte" | "astro" => Lang::Html,
        "css" | "scss" | "sass" | "less" => Lang::Css,
        "sql" => Lang::Sql,
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "psm1" => Lang::Shell,
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hxx" | "hh" | "m" | "mm" | "cs" | "java"
        | "kt" | "kts" | "swift" | "scala" | "dart" | "php" | "groovy" => Lang::CLike,
        "rb" => Lang::Ruby,
        "lua" => Lang::Lua,
        _ => Lang::Other,
    }
}

/// 由 Markdown 代码围栏的语言标签推断语言。
///
/// 与 `lang_for` 的区别：那里的输入是**文件名**（按扩展名），
/// 这里的输入是围栏标签（`rust` / `go` / `ts` / `python`…），两者写法不同。
pub fn lang_from_tag(tag: &str) -> Lang {
    match tag.trim().to_lowercase().as_str() {
        "rust" | "rs" => Lang::Rust,
        "go" | "golang" => Lang::Go,
        "ts" | "tsx" | "typescript" | "js" | "jsx" | "javascript" | "node" => Lang::Ts,
        "py" | "python" => Lang::Python,
        "json" => Lang::Json,
        "toml" => Lang::Toml,
        "yaml" | "yml" => Lang::Yaml,
        "md" | "markdown" => Lang::Markdown,
        "html" | "xml" | "svg" | "vue" => Lang::Html,
        "css" | "scss" | "less" => Lang::Css,
        "sql" => Lang::Sql,
        "sh" | "bash" | "zsh" | "shell" | "console" => Lang::Shell,
        "c" | "cpp" | "c++" | "csharp" | "cs" | "java" | "kotlin" | "kt" | "swift" | "php"
        | "dart" => Lang::CLike,
        "ruby" | "rb" => Lang::Ruby,
        "lua" => Lang::Lua,
        _ => Lang::Other,
    }
}

/// 深/浅两套语法色：浅色底用 VS Code Light+ 的暗档，避免浅绿/浅黄在白底上看不见。
fn syntax(dark: u32, light: u32) -> Rgba {
    gpui::rgb(if theme::is_light() { light } else { dark })
}

/// 语义类别 -> 颜色。
pub fn color_for(tok: Tok) -> Rgba {
    match tok {
        Tok::Plain => theme::fg(),
        Tok::Keyword => syntax(0xc586c0, 0x0000ff),
        Tok::Str => syntax(0xce9178, 0xa31515),
        Tok::Comment => syntax(0x6a9955, 0x008000),
        Tok::Number => syntax(0xb5cea8, 0x098658),
        Tok::Type => syntax(0x4ec9b0, 0x267f99),
        Tok::Func => syntax(0xdcdcaa, 0x795e26),
        Tok::Attr => syntax(0x9cdcfe, 0x0451a5),
    }
}

fn keywords(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Rust => &[
            "fn", "let", "mut", "pub", "use", "mod", "struct", "enum", "impl", "trait", "for",
            "while", "loop", "if", "else", "match", "return", "async", "await", "move", "self",
            "Self", "crate", "super", "const", "static", "ref", "as", "in", "where", "dyn", "type",
            "unsafe", "extern", "true", "false", "break", "continue", "macro_rules",
        ],
        Lang::Go => &[
            "break", "case", "chan", "const", "continue", "default", "defer", "else",
            "fallthrough", "for", "func", "go", "goto", "if", "import", "interface", "map",
            "package", "range", "return", "select", "struct", "switch", "type", "var", "true",
            "false", "nil", "iota",
        ],
        Lang::Ts => &[
            "function", "const", "let", "var", "return", "if", "else", "for", "while", "class",
            "interface", "type", "import", "export", "from", "async", "await", "new", "this",
            "true", "false", "null", "undefined", "extends", "implements", "public", "private",
            "protected", "static", "readonly", "switch", "case", "default", "break", "continue",
            "try", "catch", "finally", "throw", "typeof", "instanceof", "delete", "void", "yield",
            "super", "enum", "namespace", "declare", "abstract", "of", "in",
        ],
        Lang::Python => &[
            "def", "class", "return", "if", "elif", "else", "for", "while", "import", "from", "as",
            "with", "try", "except", "finally", "lambda", "None", "True", "False", "and", "or",
            "not", "in", "is", "async", "await", "yield", "assert", "del", "global", "nonlocal",
            "pass", "raise", "match", "case",
        ],
        Lang::Json | Lang::Toml => &["true", "false", "null"],
        Lang::Yaml => &[
            "true", "false", "null", "yes", "no", "on", "off",
        ],
        Lang::Sql => &[
            "select", "from", "where", "insert", "into", "values", "update", "set", "delete",
            "create", "table", "drop", "alter", "add", "index", "view", "database", "join", "left",
            "right", "inner", "outer", "full", "cross", "on", "group", "by", "order", "having",
            "limit", "offset", "as", "and", "or", "not", "null", "is", "in", "like", "between",
            "distinct", "union", "all", "case", "when", "then", "else", "end", "primary", "key",
            "foreign", "references", "unique", "default", "constraint", "primary_key", "begin",
            "commit", "rollback", "transaction", "exists", "count", "sum", "avg", "min", "max",
        ],
        Lang::Shell => &[
            "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case",
            "esac", "in", "function", "return", "local", "export", "unset", "readonly", "shift",
            "exit", "break", "continue", "source", "alias",
        ],
        Lang::CLike => &[
            "abstract", "assert", "auto", "bool", "boolean", "break", "byte", "case", "catch",
            "char", "class", "const", "continue", "default", "delete", "do", "double", "else",
            "enum", "extends", "extern", "final", "finally", "float", "for", "foreach", "friend",
            "goto", "if", "implements", "import", "inline", "instanceof", "int", "interface",
            "internal", "long", "namespace", "native", "new", "null", "nullptr", "open",
            "operator", "override", "package", "private", "protected", "public", "readonly",
            "return", "short", "signed", "sizeof", "static", "struct", "super", "switch",
            "synchronized", "template", "this", "throw", "throws", "transient", "try", "typedef",
            "typename", "union", "unsigned", "using", "var", "virtual", "void", "volatile",
            "while", "true", "false", "nil", "yield", "async", "await", "fun", "val", "object",
            "data",
        ],
        Lang::Ruby => &[
            "class", "module", "def", "end", "if", "unless", "elsif", "else", "while", "until",
            "for", "in", "do", "then", "begin", "rescue", "ensure", "raise", "return", "yield",
            "break", "next", "self", "nil", "true", "false", "and", "or", "not", "require",
            "attr_accessor", "attr_reader", "attr_writer", "puts",
        ],
        Lang::Lua => &[
            "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto",
            "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until",
            "while", "require",
        ],
        Lang::Markdown | Lang::Html | Lang::Css | Lang::Other => &[],
    }
}

/// 行注释前缀。
fn line_comment(lang: Lang) -> Option<&'static str> {
    match lang {
        Lang::Rust | Lang::Go | Lang::Ts | Lang::CLike => Some("//"),
        Lang::Python | Lang::Shell | Lang::Ruby | Lang::Toml => Some("#"),
        Lang::Lua | Lang::Sql => Some("--"),
        // JSONC 允许 //；纯 JSON 里这不会出现在字符串之外，按注释处理无害。
        Lang::Json => Some("//"),
        _ => None,
    }
}

/// 是否支持 `/* … */` 块注释。
fn has_block_comment(lang: Lang) -> bool {
    matches!(
        lang,
        Lang::Rust | Lang::Go | Lang::Ts | Lang::CLike | Lang::Css | Lang::Sql | Lang::Json
    )
}

/// 跨行词法状态：块注释 / 三引号 / 反引号字符串。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Multi {
    #[default]
    None,
    Backtick,
    TripleDouble,
    TripleSingle,
}

/// 逐行传给 `LineCache` 的跨行状态。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineState {
    block_comment: bool,
    multi: Multi,
}

/// 把一行文本切分为着色片段（单行、无跨行状态）。
pub fn highlight_line(line: &str, lang: Lang) -> Vec<Span> {
    let mut state = LineState::default();
    highlight_line_stateful(line, lang, &mut state)
}

/// 带跨行状态的版本：`state` 由上一行传入、在本行结束时更新。
pub fn highlight_line_stateful(line: &str, lang: Lang, state: &mut LineState) -> Vec<Span> {
    match lang {
        Lang::Markdown => return highlight_markdown(line),
        Lang::Html => return highlight_html(line, state),
        Lang::Yaml => return highlight_yaml(line),
        Lang::Css => return highlight_css(line, state),
        Lang::Other => {
            return vec![Span {
                text: line.to_string(),
                tok: Tok::Plain,
            }]
        }
        _ => {}
    }
    highlight_code(line, lang, state)
}

fn starts_with(chars: &[char], i: usize, pat: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    i + p.len() <= chars.len() && chars[i..i + p.len()] == p[..]
}

fn find_pat(chars: &[char], from: usize, pat: &str) -> Option<usize> {
    let p: Vec<char> = pat.chars().collect();
    if p.is_empty() || chars.len() < p.len() || from > chars.len() - p.len() {
        return None;
    }
    (from..=chars.len() - p.len()).find(|&j| chars[j..j + p.len()] == p[..])
}

fn flush_plain(plain: &mut String, spans: &mut Vec<Span>) {
    if !plain.is_empty() {
        spans.push(Span {
            text: std::mem::take(plain),
            tok: Tok::Plain,
        });
    }
}

fn push_span(spans: &mut Vec<Span>, plain: &mut String, text: String, tok: Tok) {
    flush_plain(plain, spans);
    if !text.is_empty() {
        spans.push(Span { text, tok });
    }
}

/// 花括号家族的通用词法扫描。
fn highlight_code(line: &str, lang: Lang, state: &mut LineState) -> Vec<Span> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let mut i = 0usize;
    let kws = keywords(lang);
    let block_pat = "*/";
    let allow_block = has_block_comment(lang);
    let line_pat = line_comment(lang);
    let backtick = matches!(lang, Lang::Go | Lang::Ts | Lang::Shell);
    let triple = matches!(lang, Lang::Python);
    let sql_case_insensitive = matches!(lang, Lang::Sql);

    while i < chars.len() {
        // ---- 跨行块注释续行 ----
        if state.block_comment {
            if let Some(end) = find_pat(&chars, 0, block_pat) {
                let cut = end + block_pat.chars().count();
                push_span(
                    &mut spans,
                    &mut plain,
                    chars[..cut].iter().collect(),
                    Tok::Comment,
                );
                state.block_comment = false;
                i = cut;
                continue;
            }
            push_span(&mut spans, &mut plain, line.to_string(), Tok::Comment);
            return spans;
        }

        // ---- 跨行字符串续行 ----
        if state.multi != Multi::None {
            let (start_pat, delimiters): (&str, usize) = match state.multi {
                Multi::Backtick => ("`", 1),
                Multi::TripleDouble => ("\"\"\"", 3),
                Multi::TripleSingle => ("'''", 3),
                Multi::None => unreachable!(),
            };
            if let Some(end) = find_pat(&chars, 0, start_pat) {
                let cut = end + delimiters;
                push_span(
                    &mut spans,
                    &mut plain,
                    chars[..cut].iter().collect(),
                    Tok::Str,
                );
                state.multi = Multi::None;
                i = cut;
                continue;
            }
            push_span(&mut spans, &mut plain, line.to_string(), Tok::Str);
            return spans;
        }

        let c = chars[i];

        // ---- 块注释开始 ----
        if allow_block && starts_with(&chars, i, "/*") {
            if let Some(end) = find_pat(&chars, i + 2, block_pat) {
                let cut = end + block_pat.chars().count();
                push_span(
                    &mut spans,
                    &mut plain,
                    chars[i..cut].iter().collect(),
                    Tok::Comment,
                );
                i = cut;
            } else {
                push_span(
                    &mut spans,
                    &mut plain,
                    chars[i..].iter().collect(),
                    Tok::Comment,
                );
                state.block_comment = true;
                return spans;
            }
            continue;
        }

        // ---- 行注释 ----
        if let Some(pat) = line_pat {
            if starts_with(&chars, i, pat) {
                push_span(
                    &mut spans,
                    &mut plain,
                    chars[i..].iter().collect(),
                    Tok::Comment,
                );
                return spans;
            }
        }

        // ---- Python 三引号 ----
        if triple && (starts_with(&chars, i, "\"\"\"") || starts_with(&chars, i, "'''")) {
            let (pat, kind) = if c == '"' {
                ("\"\"\"", Multi::TripleDouble)
            } else {
                ("'''", Multi::TripleSingle)
            };
            if let Some(end) = find_pat(&chars, i + 3, pat) {
                let cut = end + 3;
                push_span(&mut spans, &mut plain, chars[i..cut].iter().collect(), Tok::Str);
                i = cut;
            } else {
                push_span(&mut spans, &mut plain, chars[i..].iter().collect(), Tok::Str);
                state.multi = kind;
                return spans;
            }
            continue;
        }

        // ---- 反引号字符串（Go 原始字符串 / TS 模板 / Shell） ----
        if backtick && c == '`' {
            if let Some(end) = find_pat(&chars, i + 1, "`") {
                let cut = end + 1;
                push_span(&mut spans, &mut plain, chars[i..cut].iter().collect(), Tok::Str);
                i = cut;
            } else {
                push_span(&mut spans, &mut plain, chars[i..].iter().collect(), Tok::Str);
                state.multi = Multi::Backtick;
                return spans;
            }
            continue;
        }

        // ---- 单/双引号字符串 ----
        if c == '"' || c == '\'' {
            // Rust 生命周期（&'a）与 Go rune：只有真正闭合的字符字面量才当字符串。
            if c == '\'' && matches!(lang, Lang::Rust | Lang::Go | Lang::CLike) {
                if let Some(len) = char_literal_len(&chars, i) {
                    push_span(
                        &mut spans,
                        &mut plain,
                        chars[i..i + len].iter().collect(),
                        Tok::Str,
                    );
                    i += len;
                    continue;
                }
                plain.push(c);
                i += 1;
                continue;
            }
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
            spans.push(Span {
                text: s,
                tok: Tok::Str,
            });
            continue;
        }

        // ---- 数字 ----
        if c.is_ascii_digit() {
            flush_plain(&mut plain, &mut spans);
            let mut s = String::new();
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_')
            {
                s.push(chars[i]);
                i += 1;
            }
            spans.push(Span {
                text: s,
                tok: Tok::Number,
            });
            continue;
        }

        // ---- Shell 变量 ----
        if matches!(lang, Lang::Shell) && c == '$' {
            flush_plain(&mut plain, &mut spans);
            let mut s = String::from("$");
            i += 1;
            while i < chars.len()
                && (chars[i].is_alphanumeric()
                    || chars[i] == '_'
                    || chars[i] == '{'
                    || chars[i] == '}')
            {
                s.push(chars[i]);
                i += 1;
            }
            spans.push(Span {
                text: s,
                tok: Tok::Attr,
            });
            continue;
        }

        // ---- 注解 / 装饰器 ----
        if c == '@' && matches!(lang, Lang::Python | Lang::CLike) {
            flush_plain(&mut plain, &mut spans);
            let mut s = String::from("@");
            i += 1;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                s.push(chars[i]);
                i += 1;
            }
            spans.push(Span {
                text: s,
                tok: Tok::Func,
            });
            continue;
        }

        // ---- 标识符 / 关键字 ----
        if c.is_alphabetic() || c == '_' {
            flush_plain(&mut plain, &mut spans);
            let mut word = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                word.push(chars[i]);
                i += 1;
            }
            let lower = word.to_lowercase();
            let is_kw = kws.contains(&word.as_str())
                || (sql_case_insensitive && kws.contains(&lower.as_str()));
            let tok = if is_kw {
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
        spans.push(Span {
            text: String::new(),
            tok: Tok::Plain,
        });
    }
    spans
}

/// 识别字符字面量（`'x'` / `'\n'` / `'\u{1F600}'`）的长度（含两端的引号）。
///
/// Rust 的生命周期（`&'a`）也以单引号开头，但它后面没有闭合引号，
/// 不能当成字符串，否则整行后半段都会染成字符串色。
fn char_literal_len(chars: &[char], i: usize) -> Option<usize> {
    let n = chars.len();
    if i + 2 >= n {
        return None;
    }
    let mut j = i + 1;
    if chars[j] == '\\' {
        j += 1;
        if j >= n {
            return None;
        }
        match chars[j] {
            'u' => {
                if j + 1 < n && chars[j + 1] == '{' {
                    j += 2;
                    while j < n && chars[j] != '}' {
                        j += 1;
                    }
                    if j >= n {
                        return None;
                    }
                    j += 1;
                } else {
                    j += 1;
                    for _ in 0..4 {
                        if j < n && chars[j].is_ascii_hexdigit() {
                            j += 1;
                        } else {
                            return None;
                        }
                    }
                }
            }
            'x' => {
                j += 1;
                for _ in 0..2 {
                    if j < n && chars[j].is_ascii_hexdigit() {
                        j += 1;
                    } else {
                        return None;
                    }
                }
            }
            _ => j += 1,
        }
    } else {
        j += 1;
    }
    if j < n && chars[j] == '\'' {
        Some(j + 1 - i)
    } else {
        None
    }
}

/// YAML：`#` 注释、引号字符串、`key:` 的键名、布尔/null 关键字。
fn highlight_yaml(line: &str) -> Vec<Span> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let mut i = 0usize;
    let mut at_key_pos = true;
    let mut seen_colon = false;

    while i < chars.len() {
        let c = chars[i];

        // 注释：只在行首或空白之后
        if c == '#' && (i == 0 || chars[i - 1].is_whitespace()) {
            push_span(
                &mut spans,
                &mut plain,
                chars[i..].iter().collect(),
                Tok::Comment,
            );
            return spans;
        }

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
            spans.push(Span {
                text: s,
                tok: Tok::Str,
            });
            at_key_pos = false;
            continue;
        }

        if c.is_ascii_digit() {
            flush_plain(&mut plain, &mut spans);
            let mut s = String::new();
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_')
            {
                s.push(chars[i]);
                i += 1;
            }
            spans.push(Span {
                text: s,
                tok: Tok::Number,
            });
            at_key_pos = false;
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            flush_plain(&mut plain, &mut spans);
            let mut word = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '-')
            {
                word.push(chars[i]);
                i += 1;
            }
            let mut j = i;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            let colon_next = j < chars.len() && chars[j] == ':';
            let lower = word.to_lowercase();
            let tok = if at_key_pos && !seen_colon && colon_next {
                Tok::Attr
            } else if matches!(
                lower.as_str(),
                "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~"
            ) {
                Tok::Keyword
            } else {
                Tok::Plain
            };
            spans.push(Span { text: word, tok });
            at_key_pos = false;
            continue;
        }

        // `- ` 列表项之后仍可开始一个新 key
        if c == '-' && at_key_pos {
            let next_ws = i + 1 >= chars.len() || chars[i + 1].is_whitespace();
            if next_ws {
                plain.push(c);
                i += 1;
                continue;
            }
        }
        if c == ':' {
            seen_colon = true;
        }
        if !c.is_whitespace() {
            at_key_pos = false;
        }
        plain.push(c);
        i += 1;
    }

    flush_plain(&mut plain, &mut spans);
    if spans.is_empty() {
        spans.push(Span {
            text: String::new(),
            tok: Tok::Plain,
        });
    }
    spans
}

/// HTML / XML / Vue：注释、标签名、属性名、属性值。
fn highlight_html(line: &str, state: &mut LineState) -> Vec<Span> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let mut i = 0usize;

    while i < chars.len() {
        if state.block_comment {
            if let Some(end) = find_pat(&chars, 0, "-->") {
                let cut = end + 3;
                push_span(
                    &mut spans,
                    &mut plain,
                    chars[..cut].iter().collect(),
                    Tok::Comment,
                );
                state.block_comment = false;
                i = cut;
                continue;
            }
            push_span(&mut spans, &mut plain, line.to_string(), Tok::Comment);
            return spans;
        }

        if starts_with(&chars, i, "<!--") {
            if let Some(end) = find_pat(&chars, i + 4, "-->") {
                let cut = end + 3;
                push_span(
                    &mut spans,
                    &mut plain,
                    chars[i..cut].iter().collect(),
                    Tok::Comment,
                );
                i = cut;
            } else {
                push_span(
                    &mut spans,
                    &mut plain,
                    chars[i..].iter().collect(),
                    Tok::Comment,
                );
                state.block_comment = true;
                return spans;
            }
            continue;
        }

        let c = chars[i];
        let tag_start = c == '<'
            && i + 1 < chars.len()
            && (chars[i + 1].is_alphabetic() || chars[i + 1] == '/' || chars[i + 1] == '!');

        if !tag_start {
            plain.push(c);
            i += 1;
            continue;
        }

        // 标签开头：`<` + 可选 `/`
        push_span(&mut spans, &mut plain, "<".into(), Tok::Plain);
        i += 1;
        if i < chars.len() && chars[i] == '/' {
            push_span(&mut spans, &mut plain, "/".into(), Tok::Plain);
            i += 1;
        } else if i < chars.len() && chars[i] == '!' {
            // `<!DOCTYPE html>` 之类
            push_span(&mut spans, &mut plain, "!".into(), Tok::Plain);
            i += 1;
        }

        // 标签名
        let mut name = String::new();
        while i < chars.len()
            && (chars[i].is_alphanumeric()
                || chars[i] == '_'
                || chars[i] == '-'
                || chars[i] == ':')
        {
            name.push(chars[i]);
            i += 1;
        }
        if !name.is_empty() {
            push_span(&mut spans, &mut plain, name, Tok::Keyword);
        }

        // 属性区：直到 `>`
        loop {
            while i < chars.len() && chars[i].is_whitespace() {
                plain.push(chars[i]);
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
            let ac = chars[i];
            if ac == '>' {
                push_span(&mut spans, &mut plain, ">".into(), Tok::Plain);
                i += 1;
                break;
            }
            if ac == '/' || ac == '?' || ac == '=' || ac == '!' {
                plain.push(ac);
                i += 1;
                continue;
            }
            if ac == '"' || ac == '\'' {
                flush_plain(&mut plain, &mut spans);
                let quote = ac;
                let mut s = String::new();
                s.push(quote);
                i += 1;
                while i < chars.len() {
                    let ch = chars[i];
                    s.push(ch);
                    i += 1;
                    if ch == quote {
                        break;
                    }
                }
                spans.push(Span {
                    text: s,
                    tok: Tok::Str,
                });
                continue;
            }
            if ac.is_alphabetic() || ac == '_' || ac == ':' || ac == '@' {
                flush_plain(&mut plain, &mut spans);
                let mut attr = String::new();
                while i < chars.len()
                    && (chars[i].is_alphanumeric()
                        || chars[i] == '_'
                        || chars[i] == '-'
                        || chars[i] == ':'
                        || chars[i] == '@'
                        || chars[i] == '.')
                {
                    attr.push(chars[i]);
                    i += 1;
                }
                spans.push(Span {
                    text: attr,
                    tok: Tok::Attr,
                });
                continue;
            }
            if ac.is_ascii_digit() {
                flush_plain(&mut plain, &mut spans);
                let mut num = String::new();
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
                    num.push(chars[i]);
                    i += 1;
                }
                spans.push(Span {
                    text: num,
                    tok: Tok::Number,
                });
                continue;
            }
            plain.push(ac);
            i += 1;
        }
    }

    flush_plain(&mut plain, &mut spans);
    if spans.is_empty() {
        spans.push(Span {
            text: String::new(),
            tok: Tok::Plain,
        });
    }
    spans
}

/// CSS / SCSS / LESS：注释、字符串、at-rule、选择器、属性名、数字。
fn highlight_css(line: &str, state: &mut LineState) -> Vec<Span> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let mut i = 0usize;

    while i < chars.len() {
        if state.block_comment {
            if let Some(end) = find_pat(&chars, 0, "*/") {
                let cut = end + 2;
                push_span(
                    &mut spans,
                    &mut plain,
                    chars[..cut].iter().collect(),
                    Tok::Comment,
                );
                state.block_comment = false;
                i = cut;
                continue;
            }
            push_span(&mut spans, &mut plain, line.to_string(), Tok::Comment);
            return spans;
        }

        if starts_with(&chars, i, "/*") {
            if let Some(end) = find_pat(&chars, i + 2, "*/") {
                let cut = end + 2;
                push_span(
                    &mut spans,
                    &mut plain,
                    chars[i..cut].iter().collect(),
                    Tok::Comment,
                );
                i = cut;
            } else {
                push_span(
                    &mut spans,
                    &mut plain,
                    chars[i..].iter().collect(),
                    Tok::Comment,
                );
                state.block_comment = true;
                return spans;
            }
            continue;
        }

        let c = chars[i];

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
            spans.push(Span {
                text: s,
                tok: Tok::Str,
            });
            continue;
        }

        if c == '@' {
            flush_plain(&mut plain, &mut spans);
            let mut s = String::from("@");
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '-') {
                s.push(chars[i]);
                i += 1;
            }
            spans.push(Span {
                text: s,
                tok: Tok::Keyword,
            });
            continue;
        }

        if c == '#' {
            flush_plain(&mut plain, &mut spans);
            let mut s = String::from("#");
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric()) {
                s.push(chars[i]);
                i += 1;
            }
            let hex = s.len() > 1 && s[1..].chars().all(|ch| ch.is_ascii_hexdigit());
            spans.push(Span {
                text: s,
                tok: if hex { Tok::Number } else { Tok::Type },
            });
            continue;
        }

        if c == '.' && i + 1 < chars.len() && (chars[i + 1].is_alphabetic() || chars[i + 1] == '_') {
            plain.push(c);
            i += 1;
            flush_plain(&mut plain, &mut spans);
            let mut s = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '-')
            {
                s.push(chars[i]);
                i += 1;
            }
            spans.push(Span {
                text: s,
                tok: Tok::Type,
            });
            continue;
        }

        if c.is_ascii_digit() {
            flush_plain(&mut plain, &mut spans);
            let mut s = String::new();
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '%')
            {
                s.push(chars[i]);
                i += 1;
            }
            spans.push(Span {
                text: s,
                tok: Tok::Number,
            });
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            flush_plain(&mut plain, &mut spans);
            let mut word = String::new();
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '-')
            {
                word.push(chars[i]);
                i += 1;
            }
            let mut j = i;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            let prop = j < chars.len() && chars[j] == ':';
            spans.push(Span {
                text: word,
                tok: if prop { Tok::Attr } else { Tok::Plain },
            });
            continue;
        }

        plain.push(c);
        i += 1;
    }

    flush_plain(&mut plain, &mut spans);
    if spans.is_empty() {
        spans.push(Span {
            text: String::new(),
            tok: Tok::Plain,
        });
    }
    spans
}

/// Markdown 源码：标题、围栏、行内代码、链接、列表/引用标记。
fn highlight_markdown(line: &str) -> Vec<Span> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return vec![Span {
            text: line.to_string(),
            tok: Tok::Keyword,
        }];
    }
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        return vec![Span {
            text: line.to_string(),
            tok: Tok::Comment,
        }];
    }

    let chars: Vec<char> = line.chars().collect();
    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let mut i = 0usize;
    let mut at_line_start = true;

    while i < chars.len() {
        let c = chars[i];

        // 行首标记：`- ` / `* ` / `+ ` / `> ` / `1. `
        if at_line_start {
            if c.is_whitespace() {
                plain.push(c);
                i += 1;
                continue;
            }
            let marker_len = if (c == '-' || c == '*' || c == '+' || c == '>')
                && i + 1 < chars.len()
                && chars[i + 1] == ' '
            {
                1
            } else if c.is_ascii_digit() {
                let mut j = i;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j + 1 < chars.len() && chars[j] == '.' && chars[j + 1] == ' ' {
                    j - i + 1
                } else {
                    0
                }
            } else {
                0
            };
            if marker_len > 0 {
                push_span(
                    &mut spans,
                    &mut plain,
                    chars[i..i + marker_len].iter().collect(),
                    Tok::Keyword,
                );
                i += marker_len;
                at_line_start = false;
                continue;
            }
            at_line_start = false;
        }

        // 行内代码
        if c == '`' {
            flush_plain(&mut plain, &mut spans);
            let mut s = String::new();
            s.push('`');
            i += 1;
            while i < chars.len() {
                let ch = chars[i];
                s.push(ch);
                i += 1;
                if ch == '`' {
                    break;
                }
            }
            spans.push(Span {
                text: s,
                tok: Tok::Str,
            });
            continue;
        }

        // 链接地址：[text](url) / ![alt](url)
        if c == '(' {
            let prev = plain.chars().last();
            if prev == Some(']') {
                flush_plain(&mut plain, &mut spans);
                let mut s = String::new();
                s.push('(');
                i += 1;
                let mut depth = 1usize;
                while i < chars.len() {
                    let ch = chars[i];
                    s.push(ch);
                    i += 1;
                    if ch == '(' {
                        depth += 1;
                    } else if ch == ')' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                spans.push(Span {
                    text: s,
                    tok: Tok::Str,
                });
                continue;
            }
        }

        plain.push(c);
        i += 1;
    }

    flush_plain(&mut plain, &mut spans);
    if spans.is_empty() {
        spans.push(Span {
            text: String::new(),
            tok: Tok::Plain,
        });
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str, lang: Lang) -> Vec<Tok> {
        highlight_line(line, lang)
            .into_iter()
            .map(|s| s.tok)
            .collect()
    }

    fn joined(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn rust_keyword_and_string() {
        let spans = highlight_line("let x = \"hi\";", Lang::Rust);
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Keyword && s.text == "let"));
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Str && s.text == "\"hi\""));
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
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Number && s.text == "42"));
    }

    #[test]
    fn uppercase_is_type() {
        let spans = highlight_line("let v: Vec = x;", Lang::Rust);
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Type && s.text == "Vec"));
    }

    #[test]
    fn func_call_detected() {
        let spans = highlight_line("foo();", Lang::Rust);
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Func && s.text == "foo"));
    }

    #[test]
    fn python_comment_hash() {
        let kinds = kinds("x = 1  # note", Lang::Python);
        assert_eq!(*kinds.last().unwrap(), Tok::Comment);
    }

    #[test]
    fn rust_lifetime_not_a_string() {
        let spans = highlight_line("fn f<'a>(x: &'a str) {}", Lang::Rust);
        let text = joined(&spans);
        assert_eq!(text, "fn f<'a>(x: &'a str) {}");
        // 不应有被整段染色的字符串
        assert!(spans.iter().all(|s| s.tok != Tok::Str));
    }

    #[test]
    fn go_keywords_and_runes() {
        let spans = highlight_line("func main() { s := 'x' }", Lang::Go);
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Keyword && s.text == "func"));
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Str && s.text == "'x'"));
    }

    #[test]
    fn go_backtick_string_spans_lines() {
        let mut st = LineState::default();
        let a = highlight_line_stateful("x := `line1", Lang::Go, &mut st);
        assert_eq!(a.last().unwrap().tok, Tok::Str);
        let b = highlight_line_stateful("line2`", Lang::Go, &mut st);
        assert_eq!(b[0].tok, Tok::Str);
        assert_eq!(st.multi, Multi::None);
    }

    #[test]
    fn python_triple_quote_spans_lines() {
        let mut st = LineState::default();
        let a = highlight_line_stateful("s = \"\"\"abc", Lang::Python, &mut st);
        assert!(a.iter().any(|s| s.tok == Tok::Str));
        assert_eq!(st.multi, Multi::TripleDouble);
        let b = highlight_line_stateful("def\"\"\"", Lang::Python, &mut st);
        assert!(b.iter().any(|s| s.tok == Tok::Str));
        assert_eq!(st.multi, Multi::None);
    }

    #[test]
    fn block_comment_spans_lines() {
        let mut st = LineState::default();
        let a = highlight_line_stateful("fn main() { /* 注释", Lang::Rust, &mut st);
        assert_eq!(a.last().unwrap().tok, Tok::Comment);
        let b = highlight_line_stateful("还在注释 */ let x = 1;", Lang::Rust, &mut st);
        assert_eq!(b[0].tok, Tok::Comment);
        assert!(b.iter().any(|s| s.tok == Tok::Keyword && s.text == "let"));
    }

    #[test]
    fn yaml_key_comment_and_bool() {
        let spans = highlight_line("name: true  # 注释", Lang::Yaml);
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Attr && s.text == "name"));
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Keyword && s.text == "true"));
        assert_eq!(spans.last().unwrap().tok, Tok::Comment);
    }

    #[test]
    fn html_tag_and_attr() {
        let spans = highlight_line("<div class=\"box\">x</div>", Lang::Html);
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Keyword && s.text == "div"));
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Attr && s.text == "class"));
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Str && s.text == "\"box\""));
    }

    #[test]
    fn html_comment_spans_lines() {
        let mut st = LineState::default();
        let a = highlight_line_stateful("<!-- 注释", Lang::Html, &mut st);
        assert_eq!(a.last().unwrap().tok, Tok::Comment);
        let b = highlight_line_stateful("结束 --> <p>ok</p>", Lang::Html, &mut st);
        assert_eq!(b[0].tok, Tok::Comment);
        assert!(b.iter().any(|s| s.tok == Tok::Keyword && s.text == "p"));
    }

    #[test]
    fn css_property_and_selector() {
        let spans = highlight_line(".box { color: #fff; }", Lang::Css);
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Type && s.text == "box"));
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Attr && s.text == "color"));
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Number && s.text == "#fff"));
    }

    #[test]
    fn sql_keywords_case_insensitive() {
        let spans = highlight_line("SELECT id FROM users", Lang::Sql);
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Keyword && s.text == "SELECT"));
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Keyword && s.text == "FROM"));
    }

    #[test]
    fn shell_hash_comment_and_var() {
        let spans = highlight_line("echo $HOME  # 注释", Lang::Shell);
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Attr && s.text == "$HOME"));
        assert_eq!(spans.last().unwrap().tok, Tok::Comment);
    }

    #[test]
    fn markdown_heading_and_code() {
        let spans = highlight_line("## 标题", Lang::Markdown);
        assert_eq!(spans[0].tok, Tok::Keyword);
        let spans = highlight_line("前 `code` 后", Lang::Markdown);
        assert!(spans
            .iter()
            .any(|s| s.tok == Tok::Str && s.text == "`code`"));
    }

    #[test]
    fn plain_other_stays_plain() {
        let spans = highlight_line("随便 \"text\" 123", Lang::Other);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].tok, Tok::Plain);
    }

    #[test]
    fn lang_for_common_extensions() {
        assert_eq!(lang_for("a.go"), Lang::Go);
        assert_eq!(lang_for("a.yaml"), Lang::Yaml);
        assert_eq!(lang_for("a.yml"), Lang::Yaml);
        assert_eq!(lang_for("a.html"), Lang::Html);
        assert_eq!(lang_for("a.vue"), Lang::Html);
        assert_eq!(lang_for("a.css"), Lang::Css);
        assert_eq!(lang_for("a.sql"), Lang::Sql);
        assert_eq!(lang_for("a.sh"), Lang::Shell);
        assert_eq!(lang_for("a.cpp"), Lang::CLike);
        assert_eq!(lang_for("a.java"), Lang::CLike);
        assert_eq!(lang_for("a.rb"), Lang::Ruby);
        assert_eq!(lang_for("a.lua"), Lang::Lua);
        assert_eq!(lang_for("a.txt"), Lang::Other);
    }
}
