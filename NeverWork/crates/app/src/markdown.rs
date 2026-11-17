//! 轻量 Markdown 解析：把模型输出的正文切成可渲染的块。
//!
//! 之前气泡里直接 `SharedString::from(body)`，于是 ` ```rust `、
//! `## 标题`、`**加粗**` 全部以原始符号显示——回答越长越像一坨乱码。
//!
//! 这里只做「聊天回答里真正会出现的子集」，不追求完整 Markdown：
//! 代码围栏、标题、无序/有序列表、引用、分隔线、空行。
//! 单行内的 `**粗体**` 与 `` `代码` `` 由 [`inline_spans`] 处理。

/// 一个可渲染的内容块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// 标题（等级 1..=6, 文本）。
    Heading(u8, String),
    /// 普通段落（已合并同一段内的软换行）。
    Paragraph(String),
    /// 代码块（语言标签, 原始行）。语言标签为空表示未标注。
    Code { lang: String, lines: Vec<String> },
    /// 无序列表。
    Bullet(Vec<String>),
    /// 有序列表（起始序号, 各项文本）。
    Numbered(u32, Vec<String>),
    /// 引用块。
    Quote(Vec<String>),
    /// 分隔线。
    Rule,
}

/// 行内的样式片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineSpan {
    pub text: String,
    pub kind: InlineKind,
    /// `Link` 的目标地址；其他样式为 None。
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineKind {
    Plain,
    /// `**粗体**` / `__粗体__`
    Bold,
    /// `` `行内代码` ``
    Code,
    /// `*斜体*` / `_斜体_`
    Italic,
    /// `[文字](url)`
    Link,
}

/// 解析整段 Markdown 文本。
pub fn parse(text: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let normalized = text.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut i = 0usize;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // 代码围栏：``` 或 ~~~ 开头，直到闭合围栏或文末
        if let Some(lang) = fence_lang(trimmed) {
            i += 1;
            let mut code: Vec<String> = Vec::new();
            while i < lines.len() {
                let l = lines[i];
                if is_fence_close(l.trim()) {
                    i += 1;
                    break;
                }
                code.push(l.to_string());
                i += 1;
            }
            // 去掉代码块首尾的空行，避免气泡里多出空白
            while code.first().is_some_and(|l| l.trim().is_empty()) {
                code.remove(0);
            }
            while code.last().is_some_and(|l| l.trim().is_empty()) {
                code.pop();
            }
            blocks.push(Block::Code { lang, lines: code });
            continue;
        }

        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // 分隔线
        if is_rule(trimmed) {
            blocks.push(Block::Rule);
            i += 1;
            continue;
        }

        // 标题
        if let Some((level, rest)) = heading(trimmed) {
            blocks.push(Block::Heading(level, rest.to_string()));
            i += 1;
            continue;
        }

        // 引用：连续的 > 行合并成一块
        if trimmed.starts_with('>') {
            let mut items = Vec::new();
            while i < lines.len() {
                let t = lines[i].trim();
                if let Some(rest) = t.strip_prefix('>') {
                    items.push(rest.trim_start_matches(' ').to_string());
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(Block::Quote(items));
            continue;
        }

        // 无序列表
        if bullet_item(trimmed).is_some() {
            let mut items = Vec::new();
            while i < lines.len() {
                let t = lines[i].trim();
                if let Some(rest) = bullet_item(t) {
                    items.push(rest.to_string());
                    i += 1;
                } else if t.is_empty() {
                    break;
                } else {
                    // 续行并入上一项
                    if let Some(last) = items.last_mut() {
                        last.push(' ');
                        last.push_str(t);
                    }
                    i += 1;
                }
            }
            blocks.push(Block::Bullet(items));
            continue;
        }

        // 有序列表
        if let Some((start, _)) = numbered_item(trimmed) {
            let mut items = Vec::new();
            while i < lines.len() {
                let t = lines[i].trim();
                if let Some((_, rest)) = numbered_item(t) {
                    items.push(rest.to_string());
                    i += 1;
                } else if t.is_empty() {
                    break;
                } else {
                    if let Some(last) = items.last_mut() {
                        last.push(' ');
                        last.push_str(t);
                    }
                    i += 1;
                }
            }
            blocks.push(Block::Numbered(start, items));
            continue;
        }

        // 普通段落：连续非空、非块级语法的行合并
        let mut para: Vec<&str> = Vec::new();
        while i < lines.len() {
            let t = lines[i].trim();
            if t.is_empty()
                || fence_lang(t).is_some()
                || is_rule(t)
                || heading(t).is_some()
                || t.starts_with('>')
                || bullet_item(t).is_some()
                || numbered_item(t).is_some()
            {
                break;
            }
            para.push(t);
            i += 1;
        }
        blocks.push(Block::Paragraph(para.join(" ")));
    }

    blocks
}

/// 拆分行内样式。未闭合的标记按普通文本处理，不吞掉字符。
pub fn inline_spans(text: &str) -> Vec<InlineSpan> {
    let mut spans: Vec<InlineSpan> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut plain = String::new();
    let mut i = 0usize;

    while i < chars.len() {
        // 链接：[文字](url)。未闭合、空地址或带空白的都按普通文本处理，
        // 不吞掉后面的字符。
        if chars[i] == '[' {
            if let Some(close) = find_char(&chars, i + 1, ']') {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = find_char(&chars, close + 2, ')') {
                        let label: String = chars[i + 1..close].iter().collect();
                        let url: String = chars[close + 2..end].iter().collect();
                        let url = url.trim().to_string();
                        if !url.is_empty() && !url.chars().any(char::is_whitespace) {
                            if !plain.is_empty() {
                                spans.push(InlineSpan {
                                    text: std::mem::take(&mut plain),
                                    kind: InlineKind::Plain,
                                    url: None,
                                });
                            }
                            let text = if label.trim().is_empty() { url.clone() } else { label };
                            spans.push(InlineSpan {
                                text,
                                kind: InlineKind::Link,
                                url: Some(url),
                            });
                            i = end + 1;
                            continue;
                        }
                    }
                }
            }
        }
        // 行内代码：`...`
        if chars[i] == '`' {
            if let Some(end) = find_char(&chars, i + 1, '`') {
                let code: String = chars[i + 1..end].iter().collect();
                if !plain.is_empty() {
                    spans.push(InlineSpan {
                        text: std::mem::take(&mut plain),
                        kind: InlineKind::Plain,
                        url: None,
                    });
                }
                spans.push(InlineSpan { text: code, kind: InlineKind::Code, url: None });
                i = end + 1;
                continue;
            }
        }
        // 斜体：*...*（单个星号，且紧邻的不是星号，避免和粗体抢）
        if chars[i] == '*'
            && chars.get(i + 1).is_some_and(|c| *c != '*')
            && chars.get(i.wrapping_sub(1)).is_none_or(|c| *c != '*')
        {
            if let Some(end) = find_char(&chars, i + 1, '*') {
                let italic: String = chars[i + 1..end].iter().collect();
                if !italic.trim().is_empty() && !italic.contains('\n') {
                    if !plain.is_empty() {
                        spans.push(InlineSpan {
                            text: std::mem::take(&mut plain),
                            kind: InlineKind::Plain,
                            url: None,
                        });
                    }
                    spans.push(InlineSpan { text: italic, kind: InlineKind::Italic, url: None });
                    i = end + 1;
                    continue;
                }
            }
        }
        // 粗体：**...**
        if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            if let Some(end) = find_double(&chars, i + 2) {
                let bold: String = chars[i + 2..end].iter().collect();
                if !bold.trim().is_empty() {
                    if !plain.is_empty() {
                        spans.push(InlineSpan {
                            text: std::mem::take(&mut plain),
                            kind: InlineKind::Plain,
                            url: None,
                        });
                    }
                    spans.push(InlineSpan { text: bold, kind: InlineKind::Bold, url: None });
                    i = end + 2;
                    continue;
                }
            }
        }
        plain.push(chars[i]);
        i += 1;
    }

    if !plain.is_empty() {
        spans.push(InlineSpan { text: plain, kind: InlineKind::Plain, url: None });
    }
    if spans.is_empty() {
        spans.push(InlineSpan { text: String::new(), kind: InlineKind::Plain, url: None });
    }
    spans
}

fn find_char(chars: &[char], from: usize, target: char) -> Option<usize> {
    (from..chars.len()).find(|&i| chars[i] == target)
}

/// 找闭合的 `**`（跳过 `***` 这类干扰）。
fn find_double(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < chars.len() {
        if chars[i] == '*' && chars[i + 1] == '*' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// 判断是否是代码围栏起始行，返回语言标签。
fn fence_lang(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("```").or_else(|| trimmed.strip_prefix("~~~"))?;
    // 围栏行之后不能再有非空内容以外的东西（语言标签可以有）
    Some(rest.trim().split_whitespace().next().unwrap_or("").to_string())
}

fn is_fence_close(trimmed: &str) -> bool {
    trimmed == "```" || trimmed == "~~~" || trimmed.starts_with("```") && trimmed[3..].trim().is_empty()
}

fn is_rule(trimmed: &str) -> bool {
    let t = trimmed.trim();
    if t.len() < 3 {
        return false;
    }
    let first = t.chars().next().unwrap();
    matches!(first, '-' | '*' | '_') && t.chars().all(|c| c == first || c == ' ')
}

fn heading(trimmed: &str) -> Option<(u8, &str)> {
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    // 必须有空格分隔，否则 `#tag` 不是标题
    if !rest.starts_with(' ') {
        return None;
    }
    Some((hashes as u8, rest.trim()))
}

fn bullet_item(trimmed: &str) -> Option<&str> {
    for p in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(p) {
            // 排除分隔线（--- / *** 已在上游处理）
            if !rest.trim().is_empty() {
                return Some(rest.trim());
            }
        }
    }
    None
}

fn numbered_item(trimmed: &str) -> Option<(u32, &str)> {
    let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let after = &trimmed[digits.len()..];
    let rest = after.strip_prefix(". ").or_else(|| after.strip_prefix(") "))?;
    let n: u32 = digits.parse().ok()?;
    Some((n, rest.trim()))
}


/// 把行内片段按像素宽度折成多行。
///
/// 为什么不交给 flex 的 `flex_wrap` 断行：气泡宽度是按内容撑开的，
/// flex 拿不到确定宽度，长片段会直接顶出气泡边界；即使给了上限，
/// 各片段也只在**片段之间**断行，长段落仍会挤成「首行只剩一个词」的样子。
///
/// 断行规则：优先在空格处断（英文单词不拆开），中文等无空格文本按字符断。
///
/// 估宽必须用**等宽**公式：对话流整体是 `theme::mono_family()`（Cascadia
/// Mono，ASCII 前进宽度 0.6em）。之前用比例字体的 0.5em 估算，每行会
/// 少算 20% 宽度，折出来的行在渲染时超宽、右侧被父容器裁掉半截。
pub fn wrap_spans(spans: &[InlineSpan], max_px: f32, font_px: f32) -> Vec<Vec<InlineSpan>> {
    use crate::views::text_fit::char_width_mono as char_width;

    // 先摊平成「字符 + 样式」，跨片段折行才不会丢样式
    let mut chars: Vec<(char, InlineKind, Option<String>)> = Vec::new();
    for span in spans {
        for ch in span.text.chars() {
            chars.push((ch, span.kind, span.url.clone()));
        }
    }
    if chars.is_empty() {
        return vec![vec![InlineSpan { text: String::new(), kind: InlineKind::Plain, url: None }]];
    }

    let mut lines: Vec<Vec<(char, InlineKind, Option<String>)>> = Vec::new();
    let mut line: Vec<(char, InlineKind, Option<String>)> = Vec::new();
    let mut width = 0.0f32;
    // 当前行内最近一个空格的位置，用于优先在词间断行
    let mut last_space: Option<usize> = None;

    for (ch, kind, url) in chars {
        let w = char_width(ch, font_px);
        if width + w > max_px && !line.is_empty() {
            // 优先退到最近空格处，避免把 `Fn` 断成 `F` + `n`
            let cut = last_space.unwrap_or(line.len());
            let rest: Vec<(char, InlineKind, Option<String>)> = line.split_off(cut);
            // 断点处的空格留在上一行末尾会被吃掉，直接去掉行首空格更自然
            lines.push(std::mem::take(&mut line));
            line = rest;
            if line.first().is_some_and(|(c, _, _)| *c == ' ') {
                line.remove(0);
            }
            width = line.iter().map(|(c, _, _)| char_width(*c, font_px)).sum();
            last_space = line.iter().rposition(|(c, _, _)| *c == ' ');
            if ch == ' ' && line.is_empty() {
                continue;
            }
        }
        if ch == ' ' {
            last_space = Some(line.len());
        }
        line.push((ch, kind, url));
        width += w;
    }
    if !line.is_empty() {
        lines.push(line);
    }

    // 再把每行合并回样式片段（同样式且同链接才合并）
    lines
        .into_iter()
        .map(|line| {
            let mut out: Vec<InlineSpan> = Vec::new();
            for (ch, kind, url) in line {
                match out.last_mut() {
                    Some(last) if last.kind == kind && last.url == url => last.text.push(ch),
                    _ => out.push(InlineSpan { text: ch.to_string(), kind, url }),
                }
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod wrap_tests {
    use super::*;

    fn total(spans: &[InlineSpan]) -> String {
        spans.iter().map(|s| s.text.clone()).collect()
    }

    #[test]
    fn short_line_stays_one_line() {
        let spans = inline_spans("短句");
        let lines = wrap_spans(&spans, 600.0, 14.0);
        assert_eq!(lines.len(), 1);
        assert_eq!(total(&lines[0]), "短句");
    }

    #[test]
    fn long_text_splits_into_multiple_lines() {
        let text = "这是一段很长的中文说明用来验证折行是否真的生效".repeat(4);
        let spans = inline_spans(&text);
        let lines = wrap_spans(&spans, 300.0, 14.0);
        assert!(lines.len() > 1, "长文本必须折行");
        for line in &lines {
            let w: f32 = total(line).chars().map(|c| crate::views::text_fit::char_width_mono(c, 14.0)).sum();
            assert!(w <= 310.0, "行宽 {w} 超出预算");
        }
        // 不丢字
        let joined: String = lines.iter().map(|l| total(l)).collect::<Vec<_>>().join("");
        assert_eq!(joined.replace(' ', ""), text.replace(' ', ""));
    }

    #[test]
    fn break_prefers_space() {
        let spans = inline_spans("aaaa bbbb cccc");
        let lines = wrap_spans(&spans, 14.0 * 5.0, 14.0);
        assert!(lines.len() >= 2);
        // 行首不应是空格
        assert!(!total(&lines[1]).starts_with(' '), "换行后行首不应留空格");
    }

    #[test]
    fn ascii_words_are_not_split() {
        // 实测踩到过：`Fn / FnMut` 被断成 `F` + `n / FnMut`
        let spans = inline_spans("闭包实现了 Fn / FnMut / FnOnce 三种之一");
        let lines = wrap_spans(&spans, 14.0 * 8.0, 14.0);
        assert!(lines.len() > 1);
        for line in &lines {
            let t = total(line);
            let trimmed = t.trim();
            // 行首行尾不能是被切断的半个英文单词
            assert!(!trimmed.is_empty());
            if let Some(first) = trimmed.chars().next() {
                if first.is_ascii_alphanumeric() {
                    assert!(
                        t.starts_with("Fn") || !trimmed.starts_with("n"),
                        "行首出现了被切断的单词: {t:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn never_breaks_before_first_char() {
        // 单个超长单词也必须能输出，不能死循环或产出空行
        let spans = inline_spans("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let lines = wrap_spans(&spans, 20.0, 14.0);
        assert!(!lines.is_empty());
        let joined: String = lines.iter().map(|l| total(l)).collect();
        assert_eq!(joined, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    }

    #[test]
    fn styles_survive_wrapping() {
        let spans = inline_spans("**粗体开头**然后是一段足够长的普通文本需要折行");
        let lines = wrap_spans(&spans, 120.0, 14.0);
        assert!(lines.len() > 1);
        // 第一行应保留粗体样式
        assert!(lines[0].iter().any(|s| s.kind == InlineKind::Bold));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_code_fence_with_lang() {
        let blocks = parse("说明：\n\n```rust\nlet a = 1;\nlet b = 2;\n```\n");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], Block::Paragraph("说明：".into()));
        match &blocks[1] {
            Block::Code { lang, lines } => {
                assert_eq!(lang, "rust");
                assert_eq!(lines, &vec!["let a = 1;".to_string(), "let b = 2;".to_string()]);
            }
            other => panic!("应为代码块，实际 {other:?}"),
        }
    }

    #[test]
    fn unclosed_fence_takes_rest_of_text() {
        // 模型输出被截断时也要能渲染，不能丢掉后面的内容
        let blocks = parse("```go\nfunc main() {}\n");
        match &blocks[0] {
            Block::Code { lang, lines } => {
                assert_eq!(lang, "go");
                assert_eq!(lines.len(), 1);
            }
            other => panic!("应为代码块，实际 {other:?}"),
        }
    }

    #[test]
    fn fence_without_lang() {
        let blocks = parse("```\nplain\n```");
        assert_eq!(blocks[0], Block::Code { lang: String::new(), lines: vec!["plain".into()] });
    }

    #[test]
    fn parses_headings() {
        let blocks = parse("# 一级\n## 二级\n文字");
        assert_eq!(blocks[0], Block::Heading(1, "一级".into()));
        assert_eq!(blocks[1], Block::Heading(2, "二级".into()));
        assert_eq!(blocks[2], Block::Paragraph("文字".into()));
    }

    #[test]
    fn hash_without_space_is_not_heading() {
        // `#tag` 是话题标记，不是标题
        let blocks = parse("#tag 这不是标题");
        assert_eq!(blocks[0], Block::Paragraph("#tag 这不是标题".into()));
    }

    #[test]
    fn parses_bullets_and_numbered() {
        let blocks = parse("- 甲\n- 乙\n\n1. 一\n2. 二");
        assert_eq!(blocks[0], Block::Bullet(vec!["甲".into(), "乙".into()]));
        assert_eq!(blocks[1], Block::Numbered(1, vec!["一".into(), "二".into()]));
    }

    #[test]
    fn numbered_keeps_start_number() {
        let blocks = parse("3. 三\n4. 四");
        assert_eq!(blocks[0], Block::Numbered(3, vec!["三".into(), "四".into()]));
    }

    #[test]
    fn rule_is_not_bullet() {
        // `---` 曾被当成列表项，渲染出一个只含 `--` 的怪异条目
        let blocks = parse("上\n---\n下");
        assert_eq!(blocks[0], Block::Paragraph("上".into()));
        assert_eq!(blocks[1], Block::Rule);
        assert_eq!(blocks[2], Block::Paragraph("下".into()));
    }

    #[test]
    fn parses_quote() {
        let blocks = parse("> 引用一\n> 引用二");
        assert_eq!(blocks[0], Block::Quote(vec!["引用一".into(), "引用二".into()]));
    }

    #[test]
    fn paragraph_soft_wraps_into_one_line() {
        let blocks = parse("第一行\n第二行");
        assert_eq!(blocks[0], Block::Paragraph("第一行 第二行".into()));
    }

    #[test]
    fn inline_bold_and_code() {
        let spans = inline_spans("这是 **重点** 和 `code` 结尾");
        assert_eq!(
            spans,
            vec![
                InlineSpan { text: "这是 ".into(), kind: InlineKind::Plain, url: None },
                InlineSpan { text: "重点".into(), kind: InlineKind::Bold, url: None },
                InlineSpan { text: " 和 ".into(), kind: InlineKind::Plain, url: None },
                InlineSpan { text: "code".into(), kind: InlineKind::Code, url: None },
                InlineSpan { text: " 结尾".into(), kind: InlineKind::Plain, url: None },
            ]
        );
    }

    #[test]
    fn inline_italic() {
        // 实测残留：`*how* things are captured` 里单星号被原样显示
        let spans = inline_spans("它只决定 *how* 捕获");
        assert!(spans.iter().any(|s| s.kind == InlineKind::Italic && s.text == "how"));
        // 星号本身不能出现在任何片段里
        assert!(!spans.iter().any(|s| s.text.contains('*')));
    }

    #[test]
    fn bold_wins_over_italic() {
        // `**粗**` 必须整体是粗体，不能被单星号规则切成两个斜体
        let spans = inline_spans("**粗体**");
        assert_eq!(spans, vec![InlineSpan { text: "粗体".into(), kind: InlineKind::Bold, url: None }]);
    }

    #[test]
    fn unclosed_inline_markers_stay_plain() {
        // 未闭合时不能吞掉字符
        let spans = inline_spans("a **b");
        let joined: String = spans.iter().map(|s| s.text.clone()).collect();
        assert_eq!(joined, "a **b");
    }

    #[test]
    fn link_spans_parse() {
        // 技能说明里的 `[Conventional Commits 1.0.0](https://...)` 曾原样显示
        let spans = inline_spans("见 [规范](https://example.com/a) 说明");
        assert_eq!(
            spans,
            vec![
                InlineSpan { text: "见 ".into(), kind: InlineKind::Plain, url: None },
                InlineSpan {
                    text: "规范".into(),
                    kind: InlineKind::Link,
                    url: Some("https://example.com/a".into()),
                },
                InlineSpan { text: " 说明".into(), kind: InlineKind::Plain, url: None },
            ]
        );
        // 空地址按普通文本处理，不吞掉字符
        let spans = inline_spans("[坏]( )尾");
        let joined: String = spans.iter().map(|s| s.text.clone()).collect();
        assert_eq!(joined, "[坏]( )尾");
        assert!(spans.iter().all(|s| s.kind != InlineKind::Link));
    }

    #[test]
    fn empty_input_yields_no_blocks() {
        assert!(parse("").is_empty());
        assert!(parse("\n\n\n").is_empty());
    }

    #[test]
    fn real_world_answer_renders_without_markup() {
        // 用上一轮实际返回的内容做回归：这些原始符号都不该出现在渲染文本里
        let answer = "A closure is anonymous.\n\n```rust\nlet add = |a, b| a + b;\n```\n\n## How capture works\n文字";
        let blocks = parse(answer);
        assert_eq!(blocks.len(), 4);
        assert!(matches!(blocks[1], Block::Code { .. }));
        assert_eq!(blocks[2], Block::Heading(2, "How capture works".into()));
    }
}
