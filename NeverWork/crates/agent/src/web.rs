//! 联网工具：网页搜索与网页抓取。
//!
//! 设计取舍：
//! - 搜索走必应 RSS（`format=rss`）：返回结构化 XML，不像搜索结果页那样
//!   隔三差五换 DOM，也不需要任何 API Key；中文结果由必应自己本地化。
//! - 抓取只允许 http/https，拒绝本机/内网地址（防止被网页内容诱导探测内网），
//!   限制响应体积与超时，HTML 去标签后转纯文本。
//! - 两者都是只读操作：不弹审批。联网能力默认常开，不设开关——
//!   需要用的时候模型直接查，用户不用先去某个设置页打开它。
//!
//! 返回给模型的正文会被 `wrap_untrusted` 包成不可信数据（见 agent_loop），
//! 这里只负责取到内容。

use std::time::Duration;

use serde_json::Value;

use crate::tools::{ToolCall, ToolResult};

/// 一次搜索最多返回几条。
const MAX_RESULTS: usize = 10;
/// 搜索响应字节上限（RSS 一般几十 KB）。
const MAX_SEARCH_BYTES: u64 = 512 * 1024;
/// 抓取响应字节上限：够读一篇长文，又不至于被大文件拖死。
const MAX_FETCH_BYTES: u64 = 300 * 1024;
/// 抓取正文默认保留多少字符。
const DEFAULT_MAX_CHARS: usize = 8000;
/// 抓取正文允许的上限。
const MAX_CHARS_CAP: usize = 20_000;
/// 单条搜索摘要的字符上限（避免 8 条摘要吃掉半屏上下文）。
const MAX_SNIPPET_CHARS: usize = 300;
/// 联网统一超时：十几秒还没有结果，不如报错让模型换来源。
const TIMEOUT: Duration = Duration::from_secs(12);
/// 统一 UA：部分站点拒绝空 UA；用通用标识，不带任何个人信息。
const UA: &str = "Mozilla/5.0 (compatible; neverwork/0.1)";

/// 带系统代理的 ureq Agent（与 models.rs 的模型列表请求同一套做法）。
fn http() -> ureq::Agent {
    let mut b = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(UA);
    if let Some(p) = ureq::Proxy::try_from_env() {
        b = b.proxy(Some(p));
    }
    b.build().new_agent()
}

/// `web_search`：必应 RSS 搜索。
pub fn search(call: &ToolCall) -> ToolResult {
    let a = call.args();
    let query = a
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if query.is_empty() {
        return ToolResult::err_code(call, "BAD_ARGS", "缺少 query 参数", false);
    }
    let limit = a
        .get("count")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(8)
        .clamp(1, MAX_RESULTS);

    let url = format!(
        "https://www.bing.com/search?q={}&format=rss&count={limit}",
        urlencode(&query)
    );
    let resp = match http()
        .get(&url)
        .header("Accept", "application/rss+xml, application/xml;q=0.9, */*;q=0.8")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(code)) => {
            return ToolResult::err_code(
                call,
                "HTTP_ERROR",
                format!("必应搜索返回 HTTP {code}"),
                code == 429 || code >= 500,
            )
        }
        Err(e) => {
            return ToolResult::err_code(call, "NETWORK", format!("搜索请求失败：{e}"), true);
        }
    };
    let status = resp.status().as_u16();
    let mut body = resp.into_body();
    let xml = match body
        .with_config()
        .limit(MAX_SEARCH_BYTES)
        .read_to_string()
    {
        Ok(s) => s,
        Err(e) => {
            return ToolResult::err_code(call, "NETWORK", format!("读取搜索结果失败：{e}"), true);
        }
    };
    let hits = parse_rss(&xml, limit);
    if hits.is_empty() {
        return ToolResult::err_code(
            call,
            "NO_RESULTS",
            format!(
                "「{query}」没有搜到结果（HTTP {status}）；换个关键词，或确认链接后改用 web_fetch"
            ),
            true,
        );
    }

    let mut out = format!("搜索「{query}」（必应，{} 条）：", hits.len());
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!("\n{}. {}", i + 1, h.title));
        out.push_str(&format!("\n   {}", h.link));
        if !h.snippet.is_empty() {
            let snippet: String = h.snippet.chars().take(MAX_SNIPPET_CHARS).collect();
            out.push_str(&format!("\n   {snippet}"));
        }
    }
    ToolResult::ok(call, out)
}

/// `web_fetch`：抓取一个网页并转成纯文本。
pub fn fetch(call: &ToolCall) -> ToolResult {
    let a = call.args();
    let url = a
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if let Err(msg) = check_url(&url) {
        return ToolResult::err_code(call, "BAD_URL", msg, false);
    }
    let max_chars = a
        .get("max_chars")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_MAX_CHARS)
        .clamp(500, MAX_CHARS_CAP);

    let resp = match http()
        .get(&url)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,text/plain;q=0.9,application/json;q=0.8,*/*;q=0.5",
        )
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(code)) => {
            return ToolResult::err_code(
                call,
                "HTTP_ERROR",
                format!("{url} 返回 HTTP {code}"),
                code == 429 || code >= 500,
            )
        }
        Err(e) => {
            return ToolResult::err_code(call, "NETWORK", format!("抓取 {url} 失败：{e}"), true);
        }
    };
    let status = resp.status().as_u16();
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !is_textual(&ctype) {
        return ToolResult::err_code(
            call,
            "UNSUPPORTED_TYPE",
            format!("{url} 返回的是 {ctype}，不是网页或文本；只支持 html/text/json/xml"),
            false,
        );
    }

    let mut body = resp.into_body();
    let raw = match body
        .with_config()
        .limit(MAX_FETCH_BYTES)
        .read_to_string()
    {
        Ok(s) => s,
        Err(e) => {
            return ToolResult::err_code(call, "NETWORK", format!("读取 {url} 失败：{e}"), true);
        }
    };
    let oversized = raw.len() as u64 >= MAX_FETCH_BYTES;
    let is_html = ctype.contains("html") || raw.trim_start().starts_with('<');
    let title = if is_html { html_title(&raw) } else { String::new() };
    let text = if is_html { html_to_text(&raw) } else { raw };
    let text = text.trim();
    if text.is_empty() {
        return ToolResult::err_code(
            call,
            "EMPTY_PAGE",
            format!("{url} 没有可读的文本（可能是脚本渲染的页面）"),
            false,
        );
    }

    let total = text.chars().count();
    let clipped: String = text.chars().take(max_chars).collect();
    let ctype_disp = ctype.split(';').next().unwrap_or("").trim();
    let mut out = format!("网页 {url}（HTTP {status}{}）", fmt_ctype(ctype_disp));
    if !title.is_empty() {
        out.push_str(&format!("\n标题：{title}"));
    }
    out.push_str("\n\n");
    out.push_str(&clipped);
    if total > max_chars {
        out.push_str(&format!(
            "\n\n（正文共 {total} 字，已截断到 {max_chars} 字；需要更多可传更大的 max_chars）"
        ));
    } else if oversized {
        out.push_str("\n\n（响应过大，已按体积上限截断）");
    }
    ToolResult::ok(call, out)
}

fn fmt_ctype(ctype: &str) -> String {
    if ctype.is_empty() {
        String::new()
    } else {
        format!("，{ctype}")
    }
}

/// 只允许公网 http/https；本机与内网地址一律拒绝。
fn check_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err("缺少 url 参数".into());
    }
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(format!("只支持 http/https 链接，收到 `{url}`"));
    }
    let rest = lower.split_once("://").map(|(_, r)| r).unwrap_or("");
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // 去掉可能存在的 userinfo
    let host = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let host = host
        .split(':')
        .next()
        .unwrap_or(host)
        .trim_matches(|c| c == '[' || c == ']');
    if host.is_empty() {
        return Err(format!("链接缺少主机名：{url}"));
    }
    if is_local_host(host) {
        return Err(format!("拒绝访问本机/内网地址：{url}"));
    }
    Ok(())
}

fn is_local_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") || host == "::1" || host == "0.0.0.0" {
        return true;
    }
    let octets: Vec<u8> = host
        .split('.')
        .map(|p| p.parse::<u8>().ok())
        .collect::<Option<Vec<u8>>>()
        .unwrap_or_default();
    if octets.len() != 4 {
        return false;
    }
    matches!(octets[0], 10 | 127)
        || (octets[0] == 192 && octets[1] == 168)
        || (octets[0] == 172 && (16..=31).contains(&octets[1]))
        || (octets[0] == 169 && octets[1] == 254)
}

/// content-type 是否可读文本；空着就先放行，靠正文嗅探。
fn is_textual(ctype: &str) -> bool {
    if ctype.trim().is_empty() {
        return true;
    }
    let c = ctype.to_ascii_lowercase();
    c.starts_with("text/")
        || c.contains("json")
        || c.contains("xml")
        || c.contains("javascript")
        || c.contains("x-www-form-urlencoded")
}

/// 百分号编码：非保留字符以外一律转义（中文查询词必须编码）。
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 一条搜索结果。
#[derive(Debug, Default, PartialEq, Eq)]
struct Hit {
    title: String,
    link: String,
    snippet: String,
}

/// 解析必应 RSS 的 `<item>` 列表。
fn parse_rss(xml: &str, limit: usize) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut rest = xml;
    while hits.len() < limit {
        let Some(start) = rest.find("<item>") else {
            break;
        };
        let after = &rest[start + "<item>".len()..];
        let Some(end) = after.find("</item>") else {
            break;
        };
        let item = &after[..end];
        rest = &after[end..];

        let link = field_text(item, "link").unwrap_or_default();
        if link.is_empty() {
            continue;
        }
        let title = field_text(item, "title").unwrap_or_default();
        let snippet = field_text(item, "description")
            .map(|d| html_to_text(&d))
            .unwrap_or_default();
        hits.push(Hit {
            title: if title.is_empty() {
                link.clone()
            } else {
                title
            },
            link,
            snippet: snippet.replace('\n', " ").trim().to_string(),
        });
    }
    hits
}

/// 取 `<tag>…</tag>` 的文本内容（含 CDATA 与实体解码）。
fn field_text(item: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = item.find(&open)? + open.len();
    let end = item[start..].find(&close)? + start;
    let raw = &item[start..end];
    let raw = raw
        .strip_prefix("<![CDATA[")
        .and_then(|r| r.strip_suffix("]]>"))
        .unwrap_or(raw);
    Some(decode_entities(raw.trim()).trim().to_string())
}

/// 页面 `<title>`（在去标签之前取，且 head 会被 html_to_text 丢掉）。
fn html_title(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(start) = lower.find("<title") else {
        return String::new();
    };
    let Some(open_end) = lower[start..].find('>') else {
        return String::new();
    };
    let body_start = start + open_end + 1;
    let Some(len) = lower[body_start..].find("</title>") else {
        return String::new();
    };
    let raw = &html[body_start..body_start + len];
    let t = collapse_ws(&decode_entities(raw));
    t.chars().take(200).collect()
}

/// 去掉整块内容（script/style/head/...），大小写不敏感。
fn strip_blocks(html: &str, tags: &[&str]) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    'outer: while i < html.len() {
        if lower.as_bytes().get(i) == Some(&b'<') {
            for tag in tags {
                let open = format!("<{tag}");
                if !lower[i..].starts_with(&open) {
                    continue;
                }
                // `<scriptx>` 不是 script：后面必须是空白或 `>`
                let after = i + open.len();
                let ok = matches!(
                    lower.as_bytes().get(after).copied(),
                    None | Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'/')
                );
                if !ok {
                    continue;
                }
                let close = format!("</{tag}");
                if let Some(pos) = lower[i..].find(&close) {
                    let close_start = i + pos + close.len();
                    // 跳过闭合标签的 `>`
                    let next = lower[close_start..]
                        .find('>')
                        .map(|p| close_start + p + 1)
                        .unwrap_or(html.len());
                    i = next;
                    continue 'outer;
                }
                // 没闭合（页面被截断）：剩下的一律丢弃
                break 'outer;
            }
        }
        let step = html[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        out.push_str(&html[i..i + step]);
        i += step;
    }
    out
}

/// HTML → 纯文本：去标签、块级标签转换行、实体解码、压缩空白。
fn html_to_text(html: &str) -> String {
    let cleaned = strip_blocks(html, &["script", "style", "noscript", "head", "svg", "iframe"]);
    let mut out = String::with_capacity(cleaned.len() / 2);
    let mut tag = String::new();
    let mut in_tag = false;
    for ch in cleaned.chars() {
        if in_tag {
            match ch {
                '>' => {
                    let raw = tag.trim().trim_start_matches('/');
                    let name = raw.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
                    if matches!(
                        name.as_str(),
                        "p" | "br"
                            | "div"
                            | "li"
                            | "tr"
                            | "h1"
                            | "h2"
                            | "h3"
                            | "h4"
                            | "h5"
                            | "h6"
                            | "section"
                            | "article"
                            | "blockquote"
                            | "pre"
                            | "table"
                            | "ul"
                            | "ol"
                            | "hr"
                    ) {
                        out.push('\n');
                    } else if name == "td" || name == "th" {
                        out.push(' ');
                    }
                    tag.clear();
                    in_tag = false;
                }
                _ => tag.push(ch),
            }
            continue;
        }
        if ch == '<' {
            in_tag = true;
            tag.clear();
            continue;
        }
        out.push(ch);
    }
    normalize_ws(&decode_entities(&out))
}

/// 行内连续空白压成一个空格，去掉空行（段落之间最多留一行空行）。
fn normalize_ws(s: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut prev_blank = false;
    for line in s.lines() {
        let collapsed = collapse_ws(line);
        if collapsed.is_empty() {
            if !prev_blank && !lines.is_empty() {
                lines.push(String::new());
            }
            prev_blank = true;
        } else {
            lines.push(collapsed);
            prev_blank = false;
        }
    }
    while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    lines.join("\n")
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    out.trim().to_string()
}

/// 常见 HTML/XML 实体解码；数字实体（十进制/十六进制）一并处理。
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        // 实体名最长也就十几个字节；窗口按字符边界截，防止切碎中文
        let window_end = after
            .char_indices()
            .take(12)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(after.len())
            .min(after.len());
        let semi = after[..window_end].find(';');
        match semi.and_then(|semi| entity_char(&after[1..semi]).map(|c| (semi, c))) {
            Some((semi, ch)) => {
                out.push(ch);
                rest = &after[semi + 1..];
            }
            None => {
                if semi.is_some() {
                    // `&foo;` 不是已知实体：原样保留，避免把正文里的 & 吃掉
                    let semi = semi.unwrap_or(0);
                    out.push_str(&after[..=semi]);
                    rest = &after[semi + 1..];
                } else {
                    out.push('&');
                    rest = &after[1..];
                }
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity_char(ent: &str) -> Option<char> {
    match ent {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" | "ensp" | "emsp" | "thinsp" => Some(' '),
        "mdash" => Some('—'),
        "ndash" => Some('–'),
        "hellip" => Some('…'),
        "middot" => Some('·'),
        "copy" => Some('©'),
        "reg" => Some('®'),
        "ldquo" => Some('“'),
        "rdquo" => Some('”'),
        "lsquo" => Some('‘'),
        "rsquo" => Some('’'),
        _ => {
            let num = ent.strip_prefix('#')?;
            let hex = num.strip_prefix('x').or_else(|| num.strip_prefix('X'));
            let code = if let Some(hex) = hex {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                num.parse::<u32>().ok()?
            };
            char::from_u32(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: "c1".into(),
            name: name.into(),
            arguments: args.to_string(),
        }
    }

    #[test]
    fn rss_items_are_parsed_with_entities_and_cdata() {
        let xml = r#"<?xml version="1.0"?><rss><channel>
            <item><title>Rust &amp; GPUI</title><link>https://example.com/a</link>
            <description>&lt;strong&gt;快&lt;/strong&gt; 与 &lt;em&gt;稳&lt;/em&gt;</description></item>
            <item><title><![CDATA[第二 <条>]]></title><link>https://example.com/b</link>
            <description><![CDATA[纯文本]]></description></item>
            <item><title>没有链接</title><description>丢弃</description></item>
        </channel></rss>"#;
        let hits = parse_rss(xml, 8);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_eq!(hits[0].title, "Rust & GPUI");
        assert_eq!(hits[0].link, "https://example.com/a");
        assert_eq!(hits[0].snippet, "快 与 稳");
        assert_eq!(hits[1].title, "第二 <条>");
        assert_eq!(hits[1].snippet, "纯文本");
    }

    #[test]
    fn rss_limit_is_respected() {
        let mut xml = String::from("<rss><channel>");
        for i in 0..5 {
            xml.push_str(&format!(
                "<item><title>t{i}</title><link>https://e.com/{i}</link></item>"
            ));
        }
        xml.push_str("</channel></rss>");
        assert_eq!(parse_rss(&xml, 3).len(), 3);
    }

    #[test]
    fn entities_named_numeric_and_unknown() {
        assert_eq!(decode_entities("a&amp;b&nbsp;c"), "a&b c");
        assert_eq!(decode_entities("&#65;&#x4E2D;"), "A中");
        assert_eq!(decode_entities("&unknown; & x"), "&unknown; & x");
        // 未知实体原样保留，不会吞掉后半段正文
        assert_eq!(decode_entities("1 &lt; 2 & 3"), "1 < 2 & 3");
    }

    #[test]
    fn html_text_drops_scripts_and_keeps_structure() {
        let html = r#"<html><head><title>T</title><style>.x{}</style></head>
            <body><script>alert("x")</script><h1>标题</h1><p>第一段&nbsp;文字</p>
            <p>第二<br>行</p><noscript>no</noscript></body></html>"#;
        let text = html_to_text(html);
        assert_eq!(html_title(html), "T");
        assert!(!text.contains("alert"), "{text}");
        assert!(!text.contains(".x{}"), "{text}");
        assert!(!text.contains("no"), "{text}");
        assert!(text.contains("标题"), "{text}");
        assert!(text.contains("第一段 文字"), "{text}");
        assert!(text.contains("第二\n行"), "{text}");
    }

    #[test]
    fn strip_blocks_requires_tag_boundary() {
        let html = "<scriptx>keep</scriptx><script>drop</script>";
        let out = strip_blocks(html, &["script"]);
        assert!(out.contains("keep"), "{out}");
        assert!(!out.contains("drop"), "{out}");
    }

    #[test]
    fn url_rules() {
        assert!(check_url("https://example.com/doc?q=1").is_ok());
        assert!(check_url("ftp://example.com/a").is_err());
        assert!(check_url("example.com/a").is_err());
        assert!(check_url("http://localhost:8080/").is_err());
        assert!(check_url("http://127.0.0.1/").is_err());
        assert!(check_url("http://192.168.1.2/").is_err());
        assert!(check_url("http://172.20.0.1/").is_err());
        assert!(check_url("http://172.30.0.1/").is_err());
        assert!(check_url("http://172.32.0.1/").is_ok());
        assert!(check_url("http://10.0.0.5/").is_err());
        assert!(check_url("http://[::1]/").is_err());
    }

    #[test]
    fn urlencode_handles_chinese() {
        assert_eq!(urlencode("rust gpui"), "rust%20gpui");
        assert_eq!(urlencode("中 文"), "%E4%B8%AD%20%E6%96%87");
    }

    #[test]
    fn bad_scheme_fails_without_network() {
        let r = fetch(&call("web_fetch", serde_json::json!({"url": "file:///C:/x"})));
        assert!(!r.ok);
        assert_eq!(r.error_code, Some("BAD_URL"));
        let r = search(&call("web_search", serde_json::json!({})));
        assert!(!r.ok);
        assert_eq!(r.error_code, Some("BAD_ARGS"));
    }
}
