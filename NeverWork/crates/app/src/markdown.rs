//! 轻量 Markdown 解析：把模型输出的正文切成可渲染的块。
//!
//! 之前气泡里直接 `SharedString::from(body)`，于是 ` ```rust `、
//! `## 标题`、`**加粗**` 全部以原始符号显示——回答越长越像一坨乱码。
//!
//! 这里只做「聊天回答里真正会出现的子集」，不追求完整 Markdown：
//! 代码围栏、标题、无序/有序列表、引用、分隔线、空行、表格。
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
    /// 表格：表头、列对齐、数据行。数据行可能比表头短（渲染时补空）。
    Table { headers: Vec<String>, aligns: Vec<Align>, rows: Vec<Vec<String>> },
    /// 图片（整行 `![alt](src)`，src 可为本地路径或 data URI）。
    Image { alt: String, src: String },
}

/// 表格列对齐方式（`---` / `:---` / `:---:` / `---:`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
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
            // 自动识别数据：CSV/TSV 转表格、JSON 美化。识别不了照常当代码块。
            if let Some(block) = data_block(&lang, &code) {
                blocks.push(block);
            } else {
                blocks.push(Block::Code { lang, lines: code });
            }
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

        // 表格：表头行 + 分隔行（`| --- | :---: |`）必须相邻，缺分隔行就按普通段落。
        if trimmed.contains('|') && i + 1 < lines.len() && is_table_sep(lines[i + 1]) {
            let headers = split_row(trimmed);
            let aligns: Vec<Align> =
                split_row(lines[i + 1].trim()).iter().map(|c| align_of(c)).collect();
            i += 2;
            let mut rows: Vec<Vec<String>> = Vec::new();
            while i < lines.len() {
                let t = lines[i].trim();
                if t.is_empty() || !t.contains('|') {
                    break;
                }
                rows.push(split_row(t));
                i += 1;
            }
            blocks.push(Block::Table { headers, aligns, rows });
            continue;
        }

        // 图片：整行 `![alt](src)`（段落中间的图片仍按普通文本，产物提取不受影响）
        if let Some((alt, src)) = image_line(trimmed) {
            blocks.push(Block::Image { alt, src });
            i += 1;
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
                || (t.contains('|') && i + 1 < lines.len() && is_table_sep(lines[i + 1]))
                || image_line(t).is_some()
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

/// 拆一行表格为单元格：按 `|` 分割、去首尾空段、逐格 trim。
/// 支持 `\|` 转义（单元格文字里的管道符）。
fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let mut cells: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut pipes = 0usize;
    let mut chars = t.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek() == Some(&'|') {
            cur.push('|');
            chars.next();
            continue;
        }
        if ch == '|' {
            pipes += 1;
            cells.push(std::mem::take(&mut cur));
            continue;
        }
        cur.push(ch);
    }
    if pipes > 0 {
        cells.push(cur);
        // 首尾空段来自表格两侧的外管道，去掉
        if cells.first().is_some_and(|c| c.trim().is_empty()) {
            cells.remove(0);
        }
        if cells.last().is_some_and(|c| c.trim().is_empty()) {
            cells.pop();
        }
    } else {
        cells.push(cur);
    }
    cells.into_iter().map(|c| c.trim().to_string()).collect()
}

/// 是否是表格分隔行：`| --- | :---: |`，每格至少一个 `-`、可带对齐冒号。
fn is_table_sep(line: &str) -> bool {
    let t = line.trim();
    if !t.contains('-') || !t.contains('|') {
        return false;
    }
    let cells = split_row(t);
    !cells.is_empty()
        && cells.iter().all(|c| {
            let c = c.trim();
            let c = c.strip_prefix(':').unwrap_or(c);
            let c = c.strip_suffix(':').unwrap_or(c);
            !c.is_empty() && c.chars().all(|ch| ch == '-')
        })
}

fn align_of(cell: &str) -> Align {
    let c = cell.trim();
    match (c.starts_with(':'), c.ends_with(':')) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    }
}

/// 整行图片：`![alt](src)`。src 不能带空白（与行内链接同口径）。
fn image_line(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    let rest = t.strip_prefix("![")?;
    let close = rest.find("](")?;
    let alt = rest[..close].to_string();
    let src_part = &rest[close + 2..];
    let end = src_part.find(')')?;
    // 右括号必须是行尾（容忍尾随空白），否则是段落中间的行内图片
    if !src_part[end + 1..].trim().is_empty() {
        return None;
    }
    let src = src_part[..end].trim().trim_matches('"').to_string();
    if src.is_empty() || src.chars().any(char::is_whitespace) {
        return None;
    }
    Some((alt, src))
}

/// 扫描全文的图片地址（包括段落中间的图片），供产物登记使用。
pub fn extract_images(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("![") {
        let after = &rest[start + 2..];
        let Some(close) = after.find("](") else { break };
        let src_part = &after[close + 2..];
        let Some(end) = src_part.find(')') else { break };
        let src = src_part[..end].trim().trim_matches('"').to_string();
        if !src.is_empty() && !src.chars().any(char::is_whitespace) && !out.contains(&src) {
            out.push(src);
        }
        rest = &src_part[end + 1..];
    }
    out
}

/// 代码块内容的自动数据识别：
///
/// - 标注 `csv` / `tsv` 的代码块 → 表格；
/// - 标注 `json` 的代码块 → 美化缩进；
/// - 未标注的代码块：内容像 JSON / 分隔符对齐的 CSV-TSV 才转换，
///   普通代码与正文一律不动（宁可少转，不能把 `let a = 1, b = 2` 变成表格）。
fn data_block(lang: &str, lines: &[String]) -> Option<Block> {
    let l = lang.to_ascii_lowercase();
    match l.as_str() {
        "csv" => csv_table(lines, ',', 2),
        "tsv" => csv_table(lines, '\t', 2),
        "json" | "jsonc" => json_block(lines),
        "" => json_block(lines)
            .or_else(|| csv_table(lines, '\t', 3))
            .or_else(|| csv_table(lines, ',', 3)),
        // 其它语言：信任标注，不做猜测
        _ => None,
    }
}

/// JSON 代码块美化。识别失败返回 None（照原样显示）。
fn json_block(lines: &[String]) -> Option<Block> {
    let joined = lines.join("\n");
    let t = joined.trim();
    // 只认对象/数组字面量：裸数字、裸字符串不当 JSON 处理
    if t.len() > 64 * 1024 || !(t.starts_with('{') || t.starts_with('[')) {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(t).ok()?;
    let pretty = serde_json::to_string_pretty(&value).ok()?;
    Some(Block::Code { lang: "json".into(), lines: pretty.lines().map(str::to_string).collect() })
}

/// 把分隔符对齐的几行转成表格；第一行是表头。
///
/// `min_rows`：最少几行才转换。标注了 `csv` 的两行就够（表头+一条数据），
/// 未标注的内容要求至少三行，避免把两行普通短句（`a, b`）误判成表格。
fn csv_table(lines: &[String], sep: char, min_rows: usize) -> Option<Block> {
    if lines.len() < min_rows || lines.len() > 300 {
        return None;
    }
    let rows: Vec<Vec<String>> = lines
        .iter()
        .map(|l| l.split(sep).map(|c| c.trim().trim_matches('"').to_string()).collect())
        .collect();
    let cols = rows[0].len();
    if cols < 2 || rows.iter().any(|r| r.len() != cols) {
        return None;
    }
    // 未标注内容额外把关：格子要短、不能带代码/句读特征
    let strict = min_rows > 2;
    if strict {
        if lines.join("\n").len() > 20_000 {
            return None;
        }
        for row in &rows {
            for cell in row {
                if cell.chars().count() > 40
                    || cell.contains(['{', '}', '[', ']', '(', ')', ';', '=', '<', '>', '`', '。', '；'])
                {
                    return None;
                }
            }
        }
    }
    Some(Block::Table {
        headers: rows[0].clone(),
        aligns: vec![Align::Left; cols],
        rows: rows[1..].to_vec(),
    })
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

    #[test]
    fn parses_table() {
        let blocks = parse(
            "说明：\n\n| 语言 | 并发 | 用途 |\n|---|---|---|\n| Rust | 无 GC | 系统 |\n| Go | goroutine | 服务 |",
        );
        assert_eq!(blocks[0], Block::Paragraph("说明：".into()));
        match &blocks[1] {
            Block::Table { headers, aligns, rows } => {
                assert_eq!(headers, &vec!["语言".to_string(), "并发".to_string(), "用途".to_string()]);
                assert_eq!(aligns, &vec![Align::Left, Align::Left, Align::Left]);
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0], vec!["Rust".to_string(), "无 GC".to_string(), "系统".to_string()]);
            }
            other => panic!("应为表格，实际 {other:?}"),
        }
    }

    #[test]
    fn table_aligns_parse() {
        let blocks = parse("| a | b | c |\n|:--|:-:|--:|\n| 1 | 2 | 3 |");
        match &blocks[0] {
            Block::Table { aligns, .. } => {
                assert_eq!(aligns, &vec![Align::Left, Align::Center, Align::Right])
            }
            other => panic!("应为表格，实际 {other:?}"),
        }
    }

    #[test]
    fn table_row_shorter_than_header_is_kept() {
        // 列数不齐（模型偶发少一格）不能 panic，渲染时补空
        let blocks = parse("| a | b |\n|---|---|\n| 1 |");
        match &blocks[0] {
            Block::Table { rows, .. } => assert_eq!(rows[0], vec!["1".to_string()]),
            other => panic!("应为表格，实际 {other:?}"),
        }
    }

    #[test]
    fn pipe_without_separator_is_paragraph() {
        // 普通含管道的文本不能被误判成表格
        let blocks = parse("a | b\nc | d");
        assert!(matches!(blocks[0], Block::Paragraph(_)));
    }

    #[test]
    fn escaped_pipe_in_cell() {
        let blocks = parse("| 命令 | 说明 |\n|---|---|\n| a \\| b | 管道 |");
        match &blocks[0] {
            Block::Table { rows, .. } => assert_eq!(rows[0][0], "a | b"),
            other => panic!("应为表格，实际 {other:?}"),
        }
    }

    #[test]
    fn csv_fence_becomes_table() {
        let blocks = parse("```csv\n语言,并发,用途\nRust,无 GC,系统\nGo,goroutine,服务\n```");
        match &blocks[0] {
            Block::Table { headers, aligns, rows } => {
                assert_eq!(headers, &vec!["语言".to_string(), "并发".to_string(), "用途".to_string()]);
                assert_eq!(aligns.len(), 3);
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[1][0], "Go");
            }
            other => panic!("csv 代码块应转表格，实际 {other:?}"),
        }
    }

    #[test]
    fn unlabeled_aligned_csv_becomes_table() {
        // 未标注但三行对齐的数据：自动识别成表格
        let blocks = parse("```\nname,age,city\nalice,30,beijing\nbob,25,shanghai\n```");
        assert!(matches!(&blocks[0], Block::Table { rows, .. } if rows.len() == 2));
    }

    #[test]
    fn normal_code_is_not_mistaken_for_table() {
        // 带代码特征的块（括号/分号/等号）即使行行有逗号也不能变表格
        let blocks = parse("```rust\nlet a = 1, b = 2;\nlet c = 3, d = 4;\nlet e = 5, f = 6;\n```");
        assert!(matches!(&blocks[0], Block::Code { .. }));
    }

    #[test]
    fn two_line_plain_text_is_not_table() {
        // 未标注内容要求至少三行：普通两行短句不能被拆成表格
        let blocks = parse("```\nhello, world\ngoodbye, moon\n```");
        assert!(matches!(&blocks[0], Block::Code { .. }));
    }

    #[test]
    fn json_fence_is_pretty_printed() {
        let blocks = parse("```json\n{\"a\":1,\"b\":[2,3]}\n```");
        match &blocks[0] {
            Block::Code { lang, lines } => {
                assert_eq!(lang, "json");
                // 美化后是逐行缩进的对象
                assert!(lines.len() > 1, "JSON 应被展开成多行: {lines:?}");
                assert!(lines[1].contains("\"a\""));
            }
            other => panic!("应为代码块，实际 {other:?}"),
        }
    }

    #[test]
    fn unlabeled_json_is_pretty_printed() {
        let blocks = parse("```\n{\"k\": 1}\n```");
        assert!(matches!(&blocks[0], Block::Code { lang, .. } if lang == "json"));
    }

    #[test]
    fn image_line_parses() {
        let blocks = parse("说明\n![截图](E:/pics/a.png)\n结尾");
        assert_eq!(
            blocks[1],
            Block::Image { alt: "截图".into(), src: "E:/pics/a.png".into() }
        );
    }

    #[test]
    fn inline_image_is_not_a_block() {
        // 段落中间的图片不拆块（仍按文本渲染，但产物提取能扫到）
        let blocks = parse("前面 ![图](a.png) 后面");
        assert!(matches!(&blocks[0], Block::Paragraph(_)));
        assert_eq!(extract_images("前面 ![图](a.png) 后面"), vec!["a.png".to_string()]);
    }
}
