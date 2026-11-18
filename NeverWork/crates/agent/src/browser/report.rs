//! HTML 测试报告：`browser_report` 的渲染层。
//!
//! 纯函数、无 IO：截图已是 `data:` URI，`render_html` 只拼字符串。
//! 章节式单文件：左侧固定目录（可折叠、滚动高亮，内联 JS，离线可用），
//! 右侧按 `chapter` 分组渲染用例卡片；无 chapter 时退化为平铺列表。

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
    /// 所属章节名（空 = 「其他」章；全部为空时平铺显示）。
    pub chapter: String,
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

/// 分组后的章：章内用例带 1 起的序号。
struct Chapter<'a> {
    title: String,
    cases: Vec<(usize, &'a ReportCase)>,
}

/// 按 `chapter` 首次出现顺序分组（空 chapter 归「其他」章）。
fn group_cases(cases: &[ReportCase]) -> Vec<Chapter<'_>> {
    let mut out: Vec<Chapter> = Vec::new();
    for case in cases {
        let title = if case.chapter.trim().is_empty() {
            "其他".to_string()
        } else {
            case.chapter.trim().to_string()
        };
        let idx = match out.iter().position(|c| c.title == title) {
            Some(i) => i,
            None => {
                out.push(Chapter {
                    title,
                    cases: Vec::new(),
                });
                out.len() - 1
            }
        };
        let seq = out[idx].cases.len() + 1;
        out[idx].cases.push((seq, case));
    }
    out
}

/// 秒数 → 「3 秒 / 1 分 23 秒 / 1 小时 2 分」。
pub fn format_duration(secs: u64) -> String {
    if secs < 60 {
        return format!("{secs} 秒");
    }
    let (m, s) = (secs / 60, secs % 60);
    if m < 60 {
        return if s == 0 {
            format!("{m} 分")
        } else {
            format!("{m} 分 {s} 秒")
        };
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
.toc { position:fixed; left:0; top:0; bottom:0; width:264px; background:#fff;
  border-right:1px solid #e5e7eb; overflow-y:auto; padding:18px 12px 30px; }
.toc-head { font-weight:700; font-size:15px; margin:0 8px 12px; }
.toc nav { display:flex; flex-direction:column; gap:2px; font-size:13px; }
.toc-link { display:flex; align-items:center; gap:6px; color:#374151;
  text-decoration:none; padding:4px 8px; border-radius:6px; }
.toc-link:hover { background:#f3f4f6; }
.toc-link.active { background:#eef2ff; color:#1d4ed8; }
.toc-top { font-weight:600; }
.toc-chapter { margin:6px 0 2px; }
.toc-toggle { display:block; width:100%; text-align:left; background:none;
  border:none; font:inherit; font-weight:600; color:#111827; padding:4px 8px;
  border-radius:6px; cursor:pointer; }
.toc-toggle:hover { background:#f3f4f6; }
.toc-toggle::before { content:'▾ '; color:#9ca3af; }
.toc-chapter.collapsed .toc-toggle::before { content:'▸ '; }
.toc-chapter.collapsed .toc-cases { display:none; }
.toc-cases { display:flex; flex-direction:column; gap:1px; margin-left:10px; }
.dot { width:7px; height:7px; border-radius:50%; background:#6b7280; flex:none; }
.dot.pass { background:#16a34a; } .dot.fail { background:#dc2626; }
main { max-width:960px; margin:0 0 0 264px; padding:32px 32px 60px; }
h1 { font-size:26px; margin:0 0 6px; }
.summary { color:#374151; margin:0 0 14px; white-space:pre-wrap; }
.meta { color:#6b7280; font-size:13px; display:flex; flex-wrap:wrap;
  gap:6px 18px; margin-bottom:22px; }
.stats { display:grid; grid-template-columns:repeat(5,1fr); gap:10px; margin-bottom:26px; }
.stat { background:#fff; border:1px solid #e5e7eb; border-radius:10px; padding:12px 14px; }
.stat b { display:block; font-size:24px; }
.stat span { color:#6b7280; font-size:12px; }
.stat.pass b { color:#16a34a; } .stat.fail b { color:#dc2626; } .stat.skip b { color:#6b7280; }
.chapter h2 { font-size:20px; margin:36px 0 14px; padding-bottom:8px;
  border-bottom:2px solid #e5e7eb; }
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
@media (max-width:980px) {
  .toc { display:none; }
  main { margin:0 auto; }
  .pair { grid-template-columns:1fr; }
  .stats { grid-template-columns:repeat(2,1fr); }
}
@media print {
  body { background:#fff; }
  .toc { display:none; }
  main { margin:0; padding:0; }
  .case,.console { break-inside:avoid; }
}
</style>";

/// 目录交互：章节折叠 + 滚动高亮（无依赖，离线可用）。
const SCRIPT: &str = "<script>
(function () {
  document.querySelectorAll('.toc-chapter').forEach(function (ch) {
    var btn = ch.querySelector('.toc-toggle');
    if (btn) btn.addEventListener('click', function () { ch.classList.toggle('collapsed'); });
  });
  var links = Array.prototype.slice.call(document.querySelectorAll('.toc-link'));
  var targets = links.map(function (a) {
    return document.getElementById(a.getAttribute('href').slice(1));
  });
  if (!('IntersectionObserver' in window)) return;
  var io = new IntersectionObserver(function (entries) {
    entries.forEach(function (en) {
      if (!en.isIntersecting) return;
      var i = targets.indexOf(en.target);
      if (i < 0) return;
      links.forEach(function (l) { l.classList.remove('active'); });
      links[i].classList.add('active');
    });
  }, { rootMargin: '-60px 0px -65% 0px', threshold: 0 });
  targets.forEach(function (t) { if (t) io.observe(t); });
})();
</script>";

/// 渲染整份 HTML。调用方保证截图编号已校验过。
pub fn render_html(input: &ReportInput) -> String {
    let total = input.cases.len();
    let passed = input
        .cases
        .iter()
        .filter(|c| c.status == CaseStatus::Pass)
        .count();
    let failed = input
        .cases
        .iter()
        .filter(|c| c.status == CaseStatus::Fail)
        .count();
    let skipped = input
        .cases
        .iter()
        .filter(|c| c.status == CaseStatus::Skip)
        .count();
    let chapters = group_cases(&input.cases);
    // 只有一章且都是「其他」时平铺显示（不渲染章头），保持旧观感。
    let multi = chapters.len() > 1 || chapters.iter().any(|c| c.title != "其他");
    let has_console = !input.console.is_empty();

    let mut out = String::with_capacity(16_384);
    out.push_str("<!doctype html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str(&format!("<title>{}</title>\n", escape_html(&input.title)));
    out.push_str(STYLE);
    out.push_str("</head>\n<body>\n");
    out.push_str(&render_toc(&chapters, multi, has_console));
    out.push_str("<main>\n");
    out.push_str(&format!("<h1>{}</h1>\n", escape_html(&input.title)));
    if !input.summary.trim().is_empty() {
        out.push_str(&format!(
            "<p class=\"summary\">{}</p>\n",
            escape_html(&input.summary)
        ));
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
    out.push_str(&format!(
        "<div class=\"meta\">{}</div>\n",
        meta.join("<span>·</span>")
    ));
    out.push_str(&format!(
        "<div class=\"stats\" id=\"summary\">\
         <div class=\"stat\"><b>{total}</b><span>用例总数</span></div>\
         <div class=\"stat pass\"><b>{passed}</b><span>通过</span></div>\
         <div class=\"stat fail\"><b>{failed}</b><span>失败</span></div>\
         <div class=\"stat skip\"><b>{skipped}</b><span>跳过</span></div>\
         <div class=\"stat\"><b>{}</b><span>浏览器会话耗时</span></div>\
         </div>\n",
        format_duration(input.elapsed_secs)
    ));
    for (ci, chapter) in chapters.iter().enumerate() {
        let cn = ci + 1;
        if multi {
            out.push_str(&format!(
                "<section class=\"chapter\" id=\"ch-{cn}\"><h2>{cn} {}</h2>\n",
                escape_html(&chapter.title)
            ));
        }
        for (si, case) in &chapter.cases {
            let num = if multi {
                format!("{cn}.{si}")
            } else {
                si.to_string()
            };
            out.push_str(&render_case(&num, case, &input.shots, cn, *si));
        }
        if multi {
            out.push_str("</section>\n");
        }
    }
    out.push_str(&render_console(&input.console));
    out.push_str(&format!(
        "<footer>由 NeverWork 自动生成 · {}</footer>\n</main>\n",
        escape_html(&input.generated_at)
    ));
    out.push_str(SCRIPT);
    out.push_str("</body>\n</html>\n");
    out
}

fn render_toc(chapters: &[Chapter<'_>], multi: bool, has_console: bool) -> String {
    let mut out = String::from("<aside class=\"toc\"><div class=\"toc-head\">目录</div><nav>\n");
    out.push_str("<a class=\"toc-link toc-top\" href=\"#summary\">汇总</a>\n");
    for (ci, chapter) in chapters.iter().enumerate() {
        let cn = ci + 1;
        if multi {
            out.push_str(&format!(
                "<div class=\"toc-chapter\"><button class=\"toc-toggle\" type=\"button\">{} {}</button><div class=\"toc-cases\">\n",
                cn,
                escape_html(&chapter.title)
            ));
        }
        for (si, case) in &chapter.cases {
            let num = if multi {
                format!("{cn}.{si}")
            } else {
                si.to_string()
            };
            out.push_str(&format!(
                "<a class=\"toc-link\" href=\"#case-{cn}-{si}\"><span class=\"dot {}\"></span>{} {}</a>\n",
                case.status.class(),
                num,
                escape_html(&case.name)
            ));
        }
        if multi {
            out.push_str("</div></div>\n");
        }
    }
    if has_console {
        out.push_str("<a class=\"toc-link toc-top\" href=\"#console\">Console</a>\n");
    }
    out.push_str("</nav></aside>\n");
    out
}

fn render_case(
    num: &str,
    case: &ReportCase,
    shots: &[ShotInfo],
    chapter_no: usize,
    case_no: usize,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "<article class=\"case {}\" id=\"case-{chapter_no}-{case_no}\"><h2>{num} {} <span class=\"badge {}\">{}</span></h2>\n",
        case.status.class(),
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
    let mut out =
        String::from("<section class=\"console\" id=\"console\"><h2>Console 错误与失败请求</h2>\n");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn case(name: &str, status: CaseStatus, chapter: &str) -> ReportCase {
        ReportCase {
            name: name.into(),
            status,
            steps: String::new(),
            expected: "ok".into(),
            actual: "ok".into(),
            screenshots: vec![],
            chapter: chapter.into(),
        }
    }

    #[test]
    fn escapes_all_markup_chars() {
        assert_eq!(
            escape_html("<b a=\"x\">'1'&</b>"),
            "&lt;b a=&quot;x&quot;&gt;&#39;1&#39;&amp;&lt;/b&gt;"
        );
    }

    #[test]
    fn duration_formats() {
        assert_eq!(format_duration(9), "9 秒");
        assert_eq!(format_duration(83), "1 分 23 秒");
        assert_eq!(format_duration(120), "2 分");
        assert_eq!(format_duration(3_900), "1 小时 5 分");
    }

    #[test]
    fn groups_cases_by_chapter_in_first_seen_order() {
        let cases = vec![
            case("登录成功", CaseStatus::Pass, "登录流程"),
            case("错误密码", CaseStatus::Fail, "登录流程"),
            case("无章节", CaseStatus::Pass, ""),
            case("退出", CaseStatus::Pass, "会话"),
        ];
        let chapters = group_cases(&cases);
        assert_eq!(chapters.len(), 3);
        assert_eq!(chapters[0].title, "登录流程");
        assert_eq!(chapters[1].title, "其他");
        assert_eq!(chapters[2].title, "会话");
        assert_eq!(chapters[0].cases.len(), 2);
        assert_eq!(chapters[0].cases[0].0, 1);
        assert_eq!(chapters[0].cases[1].0, 2);
    }

    #[test]
    fn renders_toc_and_chapter_headers_for_multi_chapter() {
        let input = ReportInput {
            title: "验收".into(),
            summary: String::new(),
            url: String::new(),
            browser: String::new(),
            generated_at: "2026-11-18 15:30:45".into(),
            elapsed_secs: 30,
            root: String::new(),
            cases: vec![
                case("登录成功", CaseStatus::Pass, "登录流程"),
                case("错误密码", CaseStatus::Fail, "登录流程"),
                case("退出", CaseStatus::Pass, "会话"),
            ],
            shots: vec![],
            console: vec![],
        };
        let html = render_html(&input);
        assert!(html.contains("<aside class=\"toc\">"), "有目录");
        assert!(html.contains("<div class=\"toc-chapter\">"), "章可折叠");
        assert!(html.contains("toc-toggle"), "折叠按钮");
        assert!(html.contains("1 登录流程"), "章标题");
        assert!(html.contains("2 会话"), "第二章");
        assert!(html.contains("2.1 退出"), "用例编号 2.1");
        assert!(html.contains("id=\"case-1-2\""), "用例锚点");
        assert!(html.contains("href=\"#case-1-2\""), "目录链接");
        assert!(html.contains("id=\"ch-2\""), "章锚点");
        assert!(html.contains("IntersectionObserver"), "滚动高亮脚本");
        assert!(html.contains("dot fail"), "失败状态点");
    }

    #[test]
    fn single_default_chapter_falls_back_flat() {
        let input = ReportInput {
            title: "t".into(),
            summary: String::new(),
            url: String::new(),
            browser: String::new(),
            generated_at: "2026-11-18 00:00:00".into(),
            elapsed_secs: 0,
            root: String::new(),
            cases: vec![
                case("a", CaseStatus::Pass, ""),
                case("b", CaseStatus::Skip, ""),
            ],
            shots: vec![],
            console: vec![],
        };
        let html = render_html(&input);
        assert!(!html.contains("<section class=\"chapter\""), "单章不渲染章头");
        assert!(!html.contains("<div class=\"toc-chapter\">"), "单章目录平铺");
        assert!(html.contains("1 a"), "平铺编号从 1 起");
        assert!(html.contains("2 b"), "平铺编号 2");
        assert!(html.contains("id=\"case-1-2\""), "锚点仍按章内序号");
    }

    #[test]
    fn renders_cases_stats_and_shots() {
        let mut c1 = case("管理员登录", CaseStatus::Pass, "登录");
        c1.steps = "1. 输入账号\n2. 点登录".into();
        c1.expected = "进入首页".into();
        c1.actual = "进入首页".into();
        c1.screenshots = vec![1];
        let mut c2 = case("错误密码", CaseStatus::Fail, "登录");
        c2.expected = "提示错误".into();
        c2.actual = "页面白屏".into();
        c2.screenshots = vec![2];
        let input = ReportInput {
            title: "登录验收".into(),
            summary: "1 失败".into(),
            url: "http://x/login".into(),
            browser: "msedge.exe".into(),
            generated_at: "2026-11-18 15:30:45".into(),
            elapsed_secs: 75,
            root: "D:/ws".into(),
            cases: vec![c1, c2],
            shots: vec![
                ShotInfo {
                    title: "首页".into(),
                    data_uri: Some("data:image/png;base64,AA==".into()),
                    note: String::new(),
                },
                ShotInfo {
                    title: "登录".into(),
                    data_uri: None,
                    note: "图片缺失".into(),
                },
            ],
            console: vec![ConsoleItem {
                level: "error".into(),
                text: "TypeError: x".into(),
            }],
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
            title: "t".into(),
            summary: String::new(),
            url: String::new(),
            browser: String::new(),
            generated_at: "2026-11-18 00:00:00".into(),
            elapsed_secs: 0,
            root: String::new(),
            cases: vec![],
            shots: vec![],
            console: vec![],
        };
        let html = render_html(&input);
        assert!(html.contains("无 console 错误"));
        assert!(html.contains("<b>0</b><span>用例总数</span>"));
        assert!(!html.contains("href=\"#console\""), "无 console 段就不放目录项");
    }

    #[test]
    fn status_parse_is_case_insensitive() {
        assert_eq!(CaseStatus::parse("PASS"), Some(CaseStatus::Pass));
        assert_eq!(CaseStatus::parse(" fail "), Some(CaseStatus::Fail));
        assert_eq!(CaseStatus::parse("skip"), Some(CaseStatus::Skip));
        assert_eq!(CaseStatus::parse("warn"), None);
    }
}
