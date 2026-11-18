# 浏览器自动化测试报告（HTML）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 浏览器自动化测试收尾时生成单文件 HTML 测试报告（用例汇总/步骤/预期/实际/状态 + base64 内嵌截图 + console 错误），作为唯一产物；测试过程截图不再登记产物；点产物用系统浏览器打开。

**Architecture:** 模型只传结构化用例数据给新工具 `browser_report`；Rust 从浏览器会话的截图清单（`cdp::Shot`）读图、base64 内嵌、按模板渲染 HTML 写入 `artifacts/`，app 侧按结果首行解析路径登记产物。系统提示要求测试收尾必须出报告，漏调时轻提醒一次。

**Tech Stack:** Rust / gpui / CDP / base64（已有）/ 手写 FFI（GetLocalTime）。

**Spec:** `docs/superpowers/specs/2026-11-18-browser-test-report-design.md`

---

## Task 1: core 本地时间戳

**Files:** Modify `NeverWork/crates/core/src/config.rs`

- [ ] 在 `now_secs()` 后新增（Windows `GetLocalTime` 手写 FFI；其它平台按 UTC 拆分，仅测试环境用）：

```rust
/// 本地时间 `[年, 月, 日, 时, 分, 秒]`。
///
/// Windows 走 `GetLocalTime`（手写 FFI，不引时间库）；其它平台退回 UTC
/// （NeverWork 只发布 Windows，这条分支仅测试环境使用）。
fn local_parts() -> [u16; 6] {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[allow(non_snake_case)]
        struct SystemTime {
            wYear: u16,
            wMonth: u16,
            wDayOfWeek: u16,
            wDay: u16,
            wHour: u16,
            wMinute: u16,
            wSecond: u16,
            wMilliseconds: u16,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetLocalTime(st: *mut SystemTime);
        }
        let mut st = SystemTime {
            wYear: 0, wMonth: 0, wDayOfWeek: 0, wDay: 0,
            wHour: 0, wMinute: 0, wSecond: 0, wMilliseconds: 0,
        };
        unsafe { GetLocalTime(&mut st) };
        [st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond]
    }
    #[cfg(not(windows))]
    {
        let secs = now_secs();
        let days = (secs / 86_400) as i64;
        let rem = secs % 86_400;
        let (y, m, d) = civil_from_days(days);
        [y as u16, m as u16, d as u16, (rem / 3600) as u16, ((rem % 3600) / 60) as u16, (rem % 60) as u16]
    }
}

/// 公历日数（1970-01-01 起）→ (年, 月, 日)，Howard Hinnant 算法。
#[cfg(not(windows))]
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    ((y + (m <= 2) as i64) as i32, m as u32, d as u32)
}

/// `20261118-153045`：报告文件名用。
pub fn local_timestamp() -> String {
    let [y, mo, d, h, mi, s] = local_parts();
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// `2026-11-18 15:30:45`：报告正文用。
pub fn local_datetime_now() -> String {
    let [y, mo, d, h, mi, s] = local_parts();
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}
```

- [ ] 单测（config.rs tests 模块）：

```rust
#[test]
fn local_timestamp_has_expected_shape() {
    let t = local_timestamp();
    assert_eq!(t.len(), 15, "YYYYMMDD-HHMMSS 共 15 字符: {t}");
    assert!(t.as_bytes()[8] == b'-');
    assert!(t.chars().filter(|c| c.is_ascii_digit()).count() == 14);
    let [y, mo, d, _, _, _] = local_parts();
    assert!(y >= 2024 && (1..=12).contains(&mo) && (1..=31).contains(&d));
}

#[test]
fn local_datetime_now_is_readable() {
    let t = local_datetime_now();
    assert_eq!(t.len(), 19, "YYYY-MM-DD HH:MM:SS 共 19 字符: {t}");
    assert_eq!(&t[4..5], "-");
    assert_eq!(&t[10..11], " ");
}
```

- [ ] 验证：`cargo test -p neverwork-core`（预期全部通过）

## Task 2: cdp.rs 截图清单与会话元信息

**Files:** Modify `NeverWork/crates/agent/src/browser/cdp.rs`

- [ ] `ConsoleLine` 后新增（`pub(crate)` 存量风格 → 用 `pub`，report 工具跨模块读）：

```rust
/// 本次浏览器会话内的一张截图（测试报告素材，编号 = 清单下标 + 1）。
#[derive(Debug, Clone)]
pub struct Shot {
    pub path: PathBuf,
    /// 截图时的页面标题。
    pub title: String,
    /// 截图时间（Unix 秒）。
    pub at: u64,
}
```

- [ ] `BrowserSession` 增加字段：

```rust
    /// 本会话截图清单：`browser_report` 按编号内嵌进报告。
    pub shots: Vec<Shot>,
    /// 会话开始时间（报告显示「浏览器会话耗时」）。
    pub started: Instant,
    /// 浏览器可执行文件名（如 msedge.exe），报告元信息用。
    pub browser: String,
```

- [ ] `launch()` 的 `Ok(Self { .. })` 补初始化：

```rust
            browser: exe
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Edge/Chrome")
                .to_string(),
            shots: Vec::new(),
            started: Instant::now(),
```

- [ ] 验证：`cargo check -p neverwork-agent`

## Task 3: HTML 渲染模块（纯函数）

**Files:** Create `NeverWork/crates/agent/src/browser/report.rs`

- [ ] 新建文件（渲染 + 转义 + 耗时格式 + 单测）：

```rust
//! HTML 测试报告：`browser_report` 的渲染层。
//!
//! 纯函数、无 IO：截图已是 `data:` URI，`render_html` 只拼字符串。
//! 浅色单文件样式，打印友好；不引 JS。

/// 用例状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseStatus {
    Pass,
    Fail,
    Skip,
}

impl CaseStatus {
    /// 解析模型给的状态（大小写不敏感，兼容中文）。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pass" | "通过" => Some(Self::Pass),
            "fail" | "失败" => Some(Self::Fail),
            "skip" | "跳过" => Some(Self::Skip),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Pass => "通过",
            Self::Fail => "失败",
            Self::Skip => "跳过",
        }
    }

    fn class(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Skip => "skip",
        }
    }
}

/// 一条测试用例。
#[derive(Debug, Clone)]
pub struct ReportCase {
    pub name: String,
    pub status: CaseStatus,
    pub steps: String,
    pub expected: String,
    pub actual: String,
    /// 1 起的截图编号（`browser_screenshot` 的返回值）。
    pub screenshots: Vec<usize>,
}

/// 截图素材：`data_uri` 为 None 表示文件缺失或过大，渲染占位。
pub struct ShotInfo {
    pub title: String,
    pub data_uri: Option<String>,
    pub note: String,
}

pub struct ConsoleItem {
    pub level: String,
    pub text: String,
}

pub struct ReportInput {
    pub title: String,
    pub summary: String,
    pub url: String,
    pub browser: String,
    pub generated_at: String,
    pub elapsed_secs: u64,
    pub root: String,
    pub cases: Vec<ReportCase>,
    pub shots: Vec<ShotInfo>,
    pub console: Vec<ConsoleItem>,
}

/// 秒数 → 「3 秒 / 1 分 23 秒 / 1 小时 2 分」。
pub fn format_duration(secs: u64) -> String {
    if secs < 60 {
        return format!("{secs} 秒");
    }
    let (m, s) = (secs / 60, secs % 60);
    if m < 60 {
        return if s == 0 { format!("{m} 分") } else { format!("{m} 分 {s} 秒") };
    }
    format!("{} 小时 {} 分", m / 60, m % 60)
}

/// HTML 文本转义：报告里所有模型/页面文本都必须经过它。
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

const STYLE: &str = "<style>
* { box-sizing: border-box; }
body { margin:0; background:#f3f4f6; color:#111827;
  font:15px/1.6 'Segoe UI','Microsoft YaHei',system-ui,sans-serif; }
main { max-width:960px; margin:0 auto; padding:32px 20px 60px; }
h1 { font-size:26px; margin:0 0 6px; }
.summary { color:#374151; margin:0 0 14px; white-space:pre-wrap; }
.meta { color:#6b7280; font-size:13px; display:flex; flex-wrap:wrap;
  gap:6px 18px; margin-bottom:22px; }
.stats { display:grid; grid-template-columns:repeat(5,1fr); gap:10px; margin-bottom:26px; }
.stat { background:#fff; border:1px solid #e5e7eb; border-radius:10px; padding:12px 14px; }
.stat b { display:block; font-size:24px; }
.stat span { color:#6b7280; font-size:12px; }
.stat.pass b { color:#16a34a; } .stat.fail b { color:#dc2626; } .stat.skip b { color:#6b7280; }
.case { background:#fff; border:1px solid #e5e7eb; border-left:4px solid #6b7280;
  border-radius:10px; padding:18px 20px; margin-bottom:16px; page-break-inside:avoid; }
.case.pass { border-left-color:#16a34a; } .case.fail { border-left-color:#dc2626; }
.case h2 { font-size:17px; margin:0 0 12px; display:flex; align-items:center; gap:10px; }
.badge { font-size:12px; font-weight:600; color:#fff; border-radius:999px; padding:2px 10px; }
.badge.pass { background:#16a34a; } .badge.fail { background:#dc2626; } .badge.skip { background:#6b7280; }
.field { margin:10px 0; }
.field > b { display:block; font-size:12px; color:#6b7280; text-transform:uppercase;
  letter-spacing:.04em; margin-bottom:4px; }
pre { margin:0; white-space:pre-wrap; word-break:break-word; background:#f9fafb;
  border:1px solid #e5e7eb; border-radius:8px; padding:10px 12px;
  font:13px/1.55 Consolas,Menlo,monospace; }
.pair { display:grid; grid-template-columns:1fr 1fr; gap:12px; }
.pair .actual-fail { background:#fef2f2; border-color:#fecaca; }
figure { margin:14px 0 0; }
figure img { width:100%; border:1px solid #e5e7eb; border-radius:8px; display:block; }
figcaption { color:#6b7280; font-size:12px; margin-top:6px; }
.shot-missing { color:#dc2626; font-size:13px; background:#fef2f2;
  border:1px dashed #fecaca; border-radius:8px; padding:10px 12px; }
.console { background:#fff; border:1px solid #e5e7eb; border-radius:10px;
  padding:18px 20px; margin-top:26px; }
.console h2 { font-size:16px; margin:0 0 10px; }
table { width:100%; border-collapse:collapse; font-size:13px; }
td { border-top:1px solid #e5e7eb; padding:6px 8px; vertical-align:top; word-break:break-word; }
td.lv { color:#dc2626; font-weight:600; white-space:nowrap; }
.ok { color:#6b7280; font-size:13px; margin:0; }
footer { color:#6b7280; font-size:12px; text-align:center; margin-top:34px; }
@media (max-width:720px) {
  .pair { grid-template-columns:1fr; }
  .stats { grid-template-columns:repeat(2,1fr); }
}
@media print { body { background:#fff; } main { padding:0; } .case,.console { break-inside:avoid; } }
</style>";

/// 渲染整份 HTML。调用方保证截图编号已校验过。
pub fn render_html(input: &ReportInput) -> String {
    let total = input.cases.len();
    let passed = input.cases.iter().filter(|c| c.status == CaseStatus::Pass).count();
    let failed = input.cases.iter().filter(|c| c.status == CaseStatus::Fail).count();
    let skipped = input.cases.iter().filter(|c| c.status == CaseStatus::Skip).count();

    let mut out = String::with_capacity(8_192);
    out.push_str("<!doctype html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str(&format!("<title>{}</title>\n", escape_html(&input.title)));
    out.push_str(STYLE);
    out.push_str("</head>\n<body>\n<main>\n");
    out.push_str(&format!("<h1>{}</h1>\n", escape_html(&input.title)));
    if !input.summary.trim().is_empty() {
        out.push_str(&format!("<p class=\"summary\">{}</p>\n", escape_html(&input.summary)));
    }
    let mut meta: Vec<String> = Vec::new();
    meta.push(format!("生成时间 {}", escape_html(&input.generated_at)));
    if !input.url.trim().is_empty() {
        meta.push(format!("目标页面 {}", escape_html(&input.url)));
    }
    if !input.browser.trim().is_empty() {
        meta.push(format!("浏览器 {}", escape_html(&input.browser)));
    }
    if !input.root.trim().is_empty() {
        meta.push(format!("工作区 {}", escape_html(&input.root)));
    }
    out.push_str(&format!("<div class=\"meta\">{}</div>\n", meta.join("<span>·</span>")));
    out.push_str(&format!(
        "<div class=\"stats\">\
         <div class=\"stat\"><b>{total}</b><span>用例总数</span></div>\
         <div class=\"stat pass\"><b>{passed}</b><span>通过</span></div>\
         <div class=\"stat fail\"><b>{failed}</b><span>失败</span></div>\
         <div class=\"stat skip\"><b>{skipped}</b><span>跳过</span></div>\
         <div class=\"stat\"><b>{}</b><span>浏览器会话耗时</span></div>\
         </div>\n",
        format_duration(input.elapsed_secs)
    ));
    for (i, case) in input.cases.iter().enumerate() {
        out.push_str(&render_case(i + 1, case, &input.shots));
    }
    out.push_str(&render_console(&input.console));
    out.push_str(&format!(
        "<footer>由 NeverWork 自动生成 · {}</footer>\n</main>\n</body>\n</html>\n",
        escape_html(&input.generated_at)
    ));
    out
}

fn render_case(idx: usize, case: &ReportCase, shots: &[ShotInfo]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "<article class=\"case {}\"><h2>#{} {} <span class=\"badge {}\">{}</span></h2>\n",
        case.status.class(),
        idx,
        escape_html(&case.name),
        case.status.class(),
        case.status.label()
    ));
    if !case.steps.trim().is_empty() {
        out.push_str(&format!(
            "<div class=\"field\"><b>步骤</b><pre>{}</pre></div>\n",
            escape_html(&case.steps)
        ));
    }
    let actual_class = if case.status == CaseStatus::Fail {
        "actual-fail"
    } else {
        ""
    };
    out.push_str(&format!(
        "<div class=\"pair\">\
         <div class=\"field\"><b>预期</b><pre>{}</pre></div>\
         <div class=\"field\"><b>实际</b><pre class=\"{actual_class}\">{}</pre></div>\
         </div>\n",
        escape_html(&case.expected),
        escape_html(&case.actual)
    ));
    for n in &case.screenshots {
        let shot = shots.get(n.saturating_sub(1));
        match shot.and_then(|s| s.data_uri.as_deref()) {
            Some(uri) => out.push_str(&format!(
                "<figure><img src=\"{}\" alt=\"截图 #{n}\"><figcaption>截图 #{n} · {}</figcaption></figure>\n",
                escape_html(uri),
                escape_html(shot.map(|s| s.title.as_str()).unwrap_or(""))
            )),
            None => out.push_str(&format!(
                "<div class=\"shot-missing\">截图 #{n}：{}</div>\n",
                escape_html(shot.map(|s| s.note.as_str()).unwrap_or("截图不存在"))
            )),
        }
    }
    out.push_str("</article>\n");
    out
}

fn render_console(console: &[ConsoleItem]) -> String {
    let mut out = String::from("<section class=\"console\"><h2>Console 错误与失败请求</h2>\n");
    if console.is_empty() {
        out.push_str("<p class=\"ok\">无 console 错误</p>\n");
    } else {
        out.push_str("<table>\n");
        for item in console {
            out.push_str(&format!(
                "<tr><td class=\"lv\">{}</td><td>{}</td></tr>\n",
                escape_html(&item.level),
                escape_html(&item.text)
            ));
        }
        out.push_str("</table>\n");
    }
    out.push_str("</section>\n");
    out
}
```

- [ ] 单测（同文件 `#[cfg(test)] mod tests`）：

```rust
#[test]
fn escapes_all_markup_chars() {
    assert_eq!(escape_html("<b a=\"x\">'1'&</b>"), "&lt;b a=&quot;x&quot;&gt;&#39;1&#39;&amp;&lt;/b&gt;");
}

#[test]
fn duration_formats() {
    assert_eq!(format_duration(9), "9 秒");
    assert_eq!(format_duration(83), "1 分 23 秒");
    assert_eq!(format_duration(120), "2 分");
    assert_eq!(format_duration(3_900), "1 小时 5 分");
}

#[test]
fn renders_cases_stats_and_shots() {
    let input = ReportInput {
        title: "登录验收".into(),
        summary: "1 失败".into(),
        url: "http://x/login".into(),
        browser: "msedge.exe".into(),
        generated_at: "2026-11-18 15:30:45".into(),
        elapsed_secs: 75,
        root: "D:/ws".into(),
        cases: vec![
            ReportCase {
                name: "管理员登录".into(),
                status: CaseStatus::Pass,
                steps: "1. 输入账号\n2. 点登录".into(),
                expected: "进入首页".into(),
                actual: "进入首页".into(),
                screenshots: vec![1],
            },
            ReportCase {
                name: "错误密码".into(),
                status: CaseStatus::Fail,
                steps: String::new(),
                expected: "提示错误".into(),
                actual: "页面白屏".into(),
                screenshots: vec![2],
            },
        ],
        shots: vec![
            ShotInfo { title: "首页".into(), data_uri: Some("data:image/png;base64,AA==".into()), note: String::new() },
            ShotInfo { title: "登录".into(), data_uri: None, note: "图片缺失".into() },
        ],
        console: vec![ConsoleItem { level: "error".into(), text: "TypeError: x".into() }],
    };
    let html = render_html(&input);
    assert!(html.contains("<b>2</b><span>用例总数</span>"), "总数");
    assert!(html.contains("<b>1</b><span>通过</span>"), "通过数");
    assert!(html.contains("<b>1</b><span>失败</span>"), "失败数");
    assert!(html.contains("1 分 15 秒"), "耗时");
    assert!(html.contains("管理员登录"), "用例名");
    assert!(html.contains("badge pass"), "状态徽章");
    assert!(html.contains("data:image/png;base64,AA=="), "内嵌截图");
    assert!(html.contains("图片缺失"), "缺失占位");
    assert!(html.contains("TypeError: x"), "console 段");
    assert!(html.contains("actual-fail"), "失败用例实际结果高亮");
}

#[test]
fn empty_console_shows_ok_line() {
    let input = ReportInput {
        title: "t".into(), summary: String::new(), url: String::new(),
        browser: String::new(), generated_at: "2026-11-18 00:00:00".into(),
        elapsed_secs: 0, root: String::new(), cases: vec![], shots: vec![], console: vec![],
    };
    let html = render_html(&input);
    assert!(html.contains("无 console 错误"));
    assert!(html.contains("<b>0</b><span>用例总数</span>"));
}

#[test]
fn status_parse_is_case_insensitive() {
    assert_eq!(CaseStatus::parse("PASS"), Some(CaseStatus::Pass));
    assert_eq!(CaseStatus::parse(" fail "), Some(CaseStatus::Fail));
    assert_eq!(CaseStatus::parse("skip"), Some(CaseStatus::Skip));
    assert_eq!(CaseStatus::parse("warn"), None);
}
```

- [ ] 验证：`cargo test -p neverwork-agent report`（预期通过）

## Task 4: browser/mod.rs 工具实现

**Files:** Modify `NeverWork/crates/agent/src/browser/mod.rs`

- [ ] 头部模块声明加 `pub mod report;`（在 `pub mod cdp;` 旁）。
- [ ] `execute` 增加 `"browser_report" => report(call, ctx),`。
- [ ] `with_existing` 泛型化（返回值从 `Result<String, ToolResult>` 改为 `Result<T, ToolResult>`）：

```rust
fn with_existing<T>(
    call: &ToolCall,
    mut f: impl FnMut(&mut BrowserSession) -> Result<T, String>,
) -> Result<T, ToolResult> {
```

- [ ] `screenshot()` 改为登记会话截图清单，不再返回 Markdown 图片：

```rust
fn screenshot(call: &ToolCall, ctx: &ToolContext) -> ToolResult {
    let name = sanitize_name(&arg_str(&call.args(), "name"));
    match with_existing(call, |b| {
        let png = b.screenshot_png()?;
        let dir = artifacts_dir(ctx);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("创建 artifacts 目录失败（{}）：{e}", dir.display()))?;
        let file = dir.join(format!("{name}.png"));
        std::fs::write(&file, png)
            .map_err(|e| format!("写入截图失败（{}）：{e}", file.display()))?;
        let title = b.title_url().map(|(t, _)| t).unwrap_or_default();
        b.shots.push(cdp::Shot {
            path: file,
            title: title.clone(),
            at: crate::tools::now_secs(),
        });
        let n = b.shots.len();
        Ok(format!(
            "截图已归档：#{n}（{title}）\n（截图只进 browser_report 生成的测试报告；不要在正文里引用截图）"
        ))
    }) {
        Ok(msg) => ToolResult::ok(call, msg),
        Err(r) => r,
    }
}
```

  注：`crate::tools::now_secs()` 若不存在，用 `neverwork_core::config::now_secs()`。

- [ ] 新增 `report()`（参数解析 + 素材收集 + 编号校验 + base64 + 渲染落盘）：

```rust
/// `browser_report` 报告工具：把本次测试整理成自包含 HTML 报告。
///
/// 截图 base64 内嵌（用户要求「全部放测试报告最终结果」），报告是唯一产物；
/// `browser_screenshot` 的原始 png 只留作排障，不登记产物。
fn report(call: &ToolCall, ctx: &ToolContext) -> ToolResult {
    let a = call.args();
    let title = arg_str(&a, "title");
    if title.is_empty() {
        return bad_args(call, "title 必填：给报告起一个标题");
    }
    let summary = arg_str(&a, "summary");
    let include_console = a.get("include_console").and_then(Value::as_bool).unwrap_or(true);
    let cases = match parse_cases(&a) {
        Ok(c) => c,
        Err(e) => return bad_args(call, e),
    };
    // 浏览器素材（截图清单 / console / 地址 / 浏览器名 / 耗时）一次性取出，
    // 渲染与写文件在锁外做。
    let (shots, console, url, browser, elapsed_secs) = match with_existing(call, |b| {
        let url = b.title_url().map(|(_, u)| u).unwrap_or_default();
        Ok((
            b.shots.clone(),
            b.console_lines(),
            url,
            b.browser.clone(),
            b.started.elapsed().as_secs(),
        ))
    }) {
        Ok(v) => v,
        Err(r) => return r,
    };
    // 编号校验放在渲染前：一次报全，模型好改。
    for case in &cases {
        for n in &case.screenshots {
            if *n == 0 || *n > shots.len() {
                let avail = if shots.is_empty() {
                    "（本次会话还没有截图）".to_string()
                } else {
                    format!("1..={}", shots.len())
                };
                return bad_args(
                    call,
                    format!("用例「{}」引用了不存在的截图 #{n}；可用编号：{avail}", case.name),
                );
            }
        }
    }
    // 截图 → data URI；缺失/过大给占位说明。
    const MAX_INLINE: u64 = 8 * 1024 * 1024;
    use base64::Engine as _;
    let infos: Vec<report::ShotInfo> = shots
        .iter()
        .map(|s| {
            let (data_uri, note) = match std::fs::metadata(&s.path) {
                Ok(m) if m.len() > MAX_INLINE => (None, "图片过大（>8MB）未内嵌".to_string()),
                _ => match std::fs::read(&s.path) {
                    Ok(bytes) => (
                        Some(format!(
                            "data:image/png;base64,{}",
                            base64::engine::general_purpose::STANDARD.encode(&bytes)
                        )),
                        String::new(),
                    ),
                    Err(_) => (None, "图片缺失".to_string()),
                },
            };
            report::ShotInfo { title: s.title.clone(), data_uri, note }
        })
        .collect();
    let console_items: Vec<report::ConsoleItem> = if include_console {
        console
            .iter()
            .filter(|l| matches!(l.level.as_str(), "error" | "warning" | "warn" | "assert" | "http" | "net"))
            .take(100)
            .map(|l| report::ConsoleItem { level: l.level.clone(), text: l.text.clone() })
            .collect()
    } else {
        Vec::new()
    };
    let generated_at = neverwork_core::config::local_datetime_now();
    let input = report::ReportInput {
        title: title.clone(),
        summary,
        url: url.clone(),
        browser,
        generated_at: generated_at.clone(),
        elapsed_secs,
        root: if ctx.is_remote() { String::new() } else { ctx.root.to_string_lossy().to_string() },
        cases: cases.clone(),
        shots: infos,
        console: console_items,
    };
    let html = report::render_html(&input);
    let dir = artifacts_dir(ctx);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return fail(call, "BROWSER_ERROR", format!("创建 artifacts 目录失败（{}）：{e}", dir.display()), true);
    }
    let stamp = neverwork_core::config::local_timestamp();
    let mut file = dir.join(format!("测试报告-{stamp}.html"));
    let mut seq = 2;
    while file.exists() {
        file = dir.join(format!("测试报告-{stamp}-{seq}.html"));
        seq += 1;
    }
    if let Err(e) = std::fs::write(&file, html) {
        return fail(call, "BROWSER_ERROR", format!("写入测试报告失败（{}）：{e}", file.display()), true);
    }
    let abs = file.to_string_lossy().to_string();
    let passed = cases.iter().filter(|c| c.status == report::CaseStatus::Pass).count();
    let failed = cases.iter().filter(|c| c.status == report::CaseStatus::Fail).count();
    let skipped = cases.len() - passed - failed;
    let embedded = input.shots.iter().filter(|s| s.data_uri.is_some()).count();
    ToolResult::ok(
        call,
        format!(
            "测试报告已生成：{abs}\n用例 {} 个：通过 {passed} / 失败 {failed} / 跳过 {skipped}\n（已内嵌 {embedded} 张截图与 console 错误；在产物里点击用默认浏览器打开）",
            cases.len()
        ),
    )
}

/// 解析 `cases` 数组：严格校验名称与状态，截图编号接受数字或 "[3]"/"3"。
fn parse_cases(a: &Value) -> Result<Vec<report::ReportCase>, String> {
    let arr = a
        .get("cases")
        .and_then(Value::as_array)
        .ok_or("cases 必填：测试用例数组")?;
    if arr.is_empty() {
        return Err("cases 至少要有一条用例".into());
    }
    let mut out = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let name = item.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if name.is_empty() {
            return Err(format!("cases[{i}] 缺少 name"));
        }
        let status = item
            .get("status")
            .and_then(Value::as_str)
            .and_then(report::CaseStatus::parse)
            .ok_or_else(|| format!("cases[{i}]（{name}）的 status 必须是 pass/fail/skip"))?;
        let mut screenshots = Vec::new();
        if let Some(nums) = item.get("screenshots").and_then(Value::as_array) {
            for v in nums {
                let n = v.as_u64().map(|x| x as usize).or_else(|| {
                    v.as_str()
                        .and_then(|s| s.trim().trim_matches(['[', ']']).parse().ok())
                });
                match n {
                    Some(n) => screenshots.push(n),
                    None => return Err(format!("cases[{i}].screenshots 里有不是编号的值：{v}")),
                }
            }
        }
        out.push(report::ReportCase {
            name,
            status,
            steps: item.get("steps").and_then(Value::as_str).unwrap_or("").trim().to_string(),
            expected: item.get("expected").and_then(Value::as_str).unwrap_or("").trim().to_string(),
            actual: item.get("actual").and_then(Value::as_str).unwrap_or("").trim().to_string(),
            screenshots,
        });
    }
    Ok(out)
}
```

  注：`trim_matches(['[', ']'])` 需 `char` 数组 pattern（Rust 1.6x+ 支持 `trim_matches` 带数组 pattern）；若编译报错改用闭包 `trim_matches(|c| c == '[' || c == ']')`。

- [ ] 验证：`cargo check -p neverwork-agent`

## Task 5: tools.rs 注册

**Files:** Modify `NeverWork/crates/agent/src/tools.rs`

- [ ] `tool_specs()` 在 `browser_screenshot` spec 之前插入：

```rust
        ToolSpec {
            name: "browser_report".into(),
            description: "把本次浏览器测试整理成自包含 HTML 测试报告：用例汇总/步骤/预期/实际/状态 + 内嵌截图 + console 错误，存工作区 artifacts/ 并登记为产物。浏览器测试收尾必须调用；先生成报告再 browser_close。截图编号来自 browser_screenshot 返回值。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "报告标题，如「FcStor 登录流程验收」" },
                    "summary": { "type": "string", "description": "可选，一句话总体结论" },
                    "include_console": { "type": "boolean", "description": "是否附 console 错误段，默认 true" },
                    "cases": {
                        "type": "array",
                        "description": "测试用例列表（至少 1 条）",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string", "description": "用例名" },
                                "status": { "type": "string", "enum": ["pass", "fail", "skip"] },
                                "steps": { "type": "string", "description": "操作步骤，多行文本，可选" },
                                "expected": { "type": "string", "description": "预期结果" },
                                "actual": { "type": "string", "description": "实际结果" },
                                "screenshots": {
                                    "type": "array",
                                    "items": { "type": "integer" },
                                    "description": "browser_screenshot 返回的截图编号"
                                }
                            },
                            "required": ["name", "status", "expected", "actual"]
                        }
                    }
                },
                "required": ["title", "cases"]
            }),
            requires_approval: false,
        },
```

- [ ] `BROWSER_TOOLS` 数组加 `"browser_report",`（放在 `"browser_screenshot",` 前）。
- [ ] `ToolCall::summary()` 的 match 加：

```rust
            "browser_report" => a
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("测试报告")
                .to_string(),
```

- [ ] 验证：`cargo test -p neverwork-agent browser_tools_present`（现有测试自动覆盖新工具免审批）

## Task 6: agent_loop 提示词与报告提醒

**Files:** Modify `NeverWork/crates/agent/src/agent_loop.rs`

- [ ] 常量（`BROWSER_UNVERIFIED_NOTICE` 后）：

```rust
/// 浏览器测试做了多步却没生成报告时注入的提醒（只提醒一次，不拦死）。
const BROWSER_REPORT_REMINDER: &str = "【测试报告提醒】这次浏览器测试已经执行了多个步骤，但还没有调用 browser_report 生成测试报告。\
    请整理已完成的验证：每条用例填名称/步骤/预期/实际/状态（pass/fail/skip），\
    截图编号引用 browser_screenshot 的返回值，调用 browser_report 生成 HTML 报告后再收尾。\
    如果用户明确不需要报告，就在最终回答里说明，直接收尾即可。";
```

- [ ] 纯函数 + 单测：

```rust
/// 是否需要提醒生成测试报告：浏览器操作 ≥3 步、还没出报告、且没提醒过。
fn report_should_remind(browser_steps: usize, report_written: bool, reminded: bool) -> bool {
    browser_steps >= 3 && !report_written && !reminded
}
```

```rust
#[test]
fn report_reminder_fires_once_after_three_steps() {
    assert!(!report_should_remind(2, false, false), "少于 3 步不提醒");
    assert!(report_should_remind(3, false, false));
    assert!(!report_should_remind(3, true, false), "已写报告不提醒");
    assert!(!report_should_remind(3, false, true), "只提醒一次");
}
```

- [ ] run_agent 状态变量（`verify_blocks` 旁）：

```rust
    // 测试报告：浏览器操作步数 / 是否已出报告 / 是否已提醒过。
    let mut browser_steps = 0usize;
    let mut report_written = false;
    let mut report_reminded = false;
```

- [ ] `result.ok` 统计 match 改造：

```rust
                    "browser_open" => {
                        browser_used = true;
                        browser_steps += 1;
                    }
                    "browser_report" => report_written = true,
                    "run_command" => {
                        verified |= verify_command_from_args(&call.arguments)
                            .map(|c| is_verify_command(&c))
                            .unwrap_or(false);
                    }
                    n if tools::is_browser_tool(n) && n != "browser_close" => {
                        browser_steps += 1;
                    }
                    _ => {}
```

- [ ] `calls.is_empty()` 收尾分支，browser_gate 之后：

```rust
            // 测试报告提醒：多步浏览器操作却没出报告（只提醒一次，不拦死）。
            if report_should_remind(browser_steps, report_written, report_reminded) {
                report_reminded = true;
                messages.push(ChatMessage::assistant(text));
                messages.push(ChatMessage::user(BROWSER_REPORT_REMINDER));
                trace.step(round, "report_reminder", serde_json::json!({ "steps": browser_steps }));
                continue;
            }
```

- [ ] 系统提示工具清单（`browser_console` 行后）加：

```
- browser_report(title, cases[], summary?, include_console?)：生成 HTML 测试报告
  （用例汇总+内嵌截图+console 错误），存 artifacts/ 并登记产物；浏览器测试收尾必须调用
```

- [ ] 「浏览器验收与登录测试」小节追加：

```
- 完成测试后必须调用 browser_report 出报告：每条用例填名称/步骤/预期/实际/状态
  （pass/fail/skip），截图编号引用 browser_screenshot 的返回值；先生成报告再 browser_close。
- 截图只进报告，不要在正文里贴图片（测试截图不作为独立产物）。
```

- [ ] 验证：`cargo test -p neverwork-agent report_reminder`（预期通过）

## Task 7: app 侧产物链路

**Files:** Modify `NeverWork/crates/app/src/state.rs`

- [ ] `now_secs()` 附近新增纯函数：

```rust
/// `browser_report` 结果首行的绝对路径（「测试报告已生成：<path>」）。
fn report_path_from(content: &str) -> Option<&str> {
    content
        .lines()
        .next()?
        .strip_prefix("测试报告已生成：")
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// 本地路径 → `file:///` URL：盘符保留、`\` 换 `/`、非安全字符百分号编码。
fn file_url(path: &std::path::Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file:///");
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
```

- [ ] `open_artifact` 图片分支后加 HTML 打开：

```rust
        // HTML（测试报告）用系统默认浏览器打开：编辑器只会显示源码。
        if p.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
            .unwrap_or(false)
        {
            cx.open_url(&file_url(&p));
            return;
        }
```

- [ ] 删除 `browser_screenshot` 登记分支（约 4700-4718 行整段），替换为 `browser_report` 登记：

```rust
                            // 测试报告不在 written_paths 里：从结果首行解析绝对路径登记为产物。
                            // 测试过程截图不再登记（截图只进报告，用户明确要求）。
                            if result.ok && result.name == "browser_report" {
                                if let Some(abs) = report_path_from(&result.content) {
                                    let thread = tmsg
                                        .and_then(|i| this.sessions[session_idx].messages.get(i))
                                        .and_then(|m| match &m.kind {
                                            MessageKind::ToolCall { thread, .. } => *thread,
                                            _ => None,
                                        })
                                        .unwrap_or(0);
                                    this.register_artifact(session_idx, abs, thread);
                                }
                            }
```

- [ ] 单测（state.rs tests 模块）：

```rust
#[test]
fn report_path_parses_first_line_only() {
    let content = "测试报告已生成：D:\\ws\\artifacts\\测试报告-20261118-153045.html\n用例 2 个：通过 2 / 失败 0 / 跳过 0";
    assert_eq!(
        report_path_from(content),
        Some("D:\\ws\\artifacts\\测试报告-20261118-153045.html")
    );
    assert_eq!(report_path_from("测试报告已生成：\n其它"), None, "空路径");
    assert_eq!(report_path_from("别的结果"), None);
}

#[test]
fn file_url_encodes_space_and_chinese() {
    let u = file_url(std::path::Path::new(r"D:\ws a\测试报告.html"));
    assert!(u.starts_with("file:///D:/ws%20a/"), "{u}");
    assert!(u.ends_with(".html"));
    assert!(!u.contains('\\'));
}
```

- [ ] 验证：`cargo test -p neverwork report_path_parses`；`cargo build -p neverwork`

## Task 8: 文档、回归与实机验收

**Files:** Modify `AGENTS.md`、spec 状态

- [ ] AGENTS.md「对话区与查找」或「Agent 工具」段补一条：

```
- **测试报告（`browser_report`）**：浏览器测试收尾由 Agent 自动生成 HTML 报告（`artifacts/测试报告-<时间>.html`，单文件、截图 base64 内嵌、console 段），报告是唯一产物——`browser_screenshot` 的截图不再登记产物（原文案「截图已保存：」解析已删）。app 侧按结果首行「测试报告已生成：」登记产物，产物点开 `.html` 走系统默认浏览器（`file_url` 百分号编码）。别把截图登记加回来，也别让报告走文本编辑器。`report_should_remind`：浏览器操作 ≥3 步且没出报告提醒一次。
```

- [ ] 回归：`cargo test --workspace`（219+ 全部通过）
- [ ] 构建：`cargo build -p neverwork`
- [ ] 实机：临时工作区 + 本地静态页（`python -m http.server`）→ `NW_SEND` 指令「打开页面→点击→截图→browser_report 出报告」；断言：
  - 会话 JSON `artifacts` 只有 1 条 `.html`（无 png）
  - 报告文件存在、`report` 内含 base64 图与用例表
  - 启动应用点产物 → 系统浏览器打开（人工确认或在报告页 headless 截图验证）
- [ ] 清理临时文件（pic/ 下 `.tmp_` 前缀）
