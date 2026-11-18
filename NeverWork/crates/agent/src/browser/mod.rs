//! 内置浏览器工具：系统 Edge/Chrome + Chrome DevTools Protocol。
//!
//! 模块划分：
//! - `ws`：最小 WebSocket 客户端（只服务 127.0.0.1 的 CDP 连接）；
//! - `cdp`：浏览器进程生命周期、CDP 命令与事件缓冲；
//! - `snapshot`：可访问性树 → 带 `[n]` 编号的文本快照（模型据此“看见”页面）；
//! - 本模块：9 个 `browser_*` 工具的实现。
//!
//! 单例策略：进程内只保留一个活跃浏览器（同 `mcp.rs` 的全局锁思路）。
//! 切换会话不主动关闭——用户可能想接着看上一步的页面；`browser_close`
//! 或进程退出（Drop）时清理。

pub mod cdp;
pub mod report;
pub mod snapshot;
pub mod ws;

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};

use crate::tools::{self, ToolCall, ToolContext, ToolResult};
use cdp::BrowserSession;

/// `browser_wait` 的等待上限。
const WAIT_CAP_MS: u64 = 15_000;

static BROWSER: OnceLock<Mutex<Option<BrowserSession>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<BrowserSession>> {
    BROWSER.get_or_init(|| Mutex::new(None))
}

fn lock() -> MutexGuard<'static, Option<BrowserSession>> {
    slot().lock().unwrap_or_else(|p| p.into_inner())
}

/// 自动化浏览器默认显示可见窗口（用户要能看见测试在点什么）；
/// `NW_BROWSER_HEADLESS=1` 强制无头（CI / 自动化验收用）。
fn default_headed() -> bool {
    headed_from_env(std::env::var("NW_BROWSER_HEADLESS").ok().as_deref())
}

fn headed_from_env(headless: Option<&str>) -> bool {
    headless.map(|v| v != "1").unwrap_or(true)
}

/// 关闭全局浏览器会话（用户停止生成时调用）：Drop 里 Browser.close + kill，
/// 正在执行的 CDP 调用随之失败返回，不用等它自己跑完。
pub fn shutdown() {
    let _ = lock().take();
}

/// 执行一个 `browser_*` 工具。
pub fn execute(call: &ToolCall, ctx: &ToolContext) -> ToolResult {
    match call.name.as_str() {
        "browser_open" => open(call),
        "browser_takeover" => takeover(call),
        "browser_snapshot" => snapshot(call),
        "browser_click" => click(call),
        "browser_type" => type_text(call),
        "browser_press" => press(call),
        "browser_wait" => wait(call),
        "browser_screenshot" => screenshot(call, ctx),
        "browser_report" => report(call, ctx),
        "browser_console" => console(call),
        "browser_close" => close(call),
        other => ToolResult::err_code(
            call,
            "UNKNOWN_TOOL",
            format!("未知工具 `{other}`"),
            false,
        ),
    }
}

fn bad_args(call: &ToolCall, msg: impl Into<String>) -> ToolResult {
    ToolResult::err_code(call, "BAD_ARGS", msg, false)
}

/// 浏览器相关失败统一带 `BROWSER_*` 码：模型据此知道是环境问题还是操作问题。
fn fail(call: &ToolCall, code: &'static str, msg: impl Into<String>, retryable: bool) -> ToolResult {
    ToolResult::err_code(call, code, msg, retryable)
}

fn arg_str(a: &Value, key: &str) -> String {
    a.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn target_arg(call: &ToolCall) -> Option<String> {
    let t = arg_str(&call.args(), "target");
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

/// `[3]` / `3` → 3；不是编号就返回 None（按 CSS 选择器处理）。
fn parse_ref(target: &str) -> Option<u32> {
    let t = target.trim();
    let t = t
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(t);
    t.parse::<u32>().ok()
}

fn open(call: &ToolCall) -> ToolResult {
    let a = call.args();
    let url = arg_str(&a, "url");
    if !tools::browser_url_ok(&url) {
        return bad_args(
            call,
            "只支持 http/https 地址（本地 dev server 与内网放行；file://、data: 不允许）",
        );
    }
    let headed = a
        .get("headed")
        .and_then(Value::as_bool)
        .unwrap_or_else(default_headed);
    let mut guard = lock();
    let mut restart = guard.is_none();
    if let Some(b) = guard.as_mut() {
        if b.headed != headed || !b.alive() {
            restart = true;
        }
    }
    if restart {
        *guard = None;
        match BrowserSession::launch(headed) {
            Ok(b) => *guard = Some(b),
            Err(e) => return fail(call, "BROWSER_LAUNCH", e, true),
        }
    }
    let res = {
        let b = guard.as_mut().expect("刚确保过浏览器存在");
        b.navigate(&url)
    };
    match res {
        Ok((title, final_url)) => ToolResult::ok(
            call,
            format!("已打开：{title}\nURL：{final_url}\n下一步用 browser_snapshot 查看页面内容。"),
        ),
        Err(e) => {
            let dead = guard.as_mut().map(|b| !b.alive()).unwrap_or(true);
            if dead {
                *guard = None;
                if let Ok(mut b) = BrowserSession::launch(headed) {
                    let retry = b.navigate(&url);
                    *guard = Some(b);
                    if let Ok((title, final_url)) = retry {
                        return ToolResult::ok(
                            call,
                            format!("浏览器崩溃后已重启。已打开：{title}\nURL：{final_url}"),
                        );
                    }
                }
            }
            fail(call, "NAV_FAILED", e, true)
        }
    }
}

/// 人工接管：遇到验证码/账号密码等必须真人输入的画面时，把无头浏览器
/// 切换成可见窗口（关闭当前无头会话，以 headed 重开并导航回当前页面），
/// 让用户直接在窗口里输入。后续用 ask_user 请用户确认完成，再 snapshot 继续。
fn takeover(call: &ToolCall) -> ToolResult {
    let mut guard = lock();
    if guard.is_none() {
        return fail(
            call,
            "NO_SESSION",
            "还没有打开浏览器：先 browser_open 打开目标页面，再 browser_takeover",
            true,
        );
    }
    let (headed, current) = {
        let b = guard.as_ref().expect("刚检查过会话存在");
        (b.headed, b.title_url().ok())
    };
    if headed {
        let page = current
            .map(|(t, u)| format!("{t}（{u}）"))
            .unwrap_or_default();
        return ToolResult::ok(
            call,
            format!(
                "浏览器窗口已经可见：{page}\n请用户直接在窗口中完成验证码/账号密码等输入（若没看到窗口，从任务栏切到 Edge/Chrome）；完成后先 ask_user 请用户确认「已完成」，再 browser_snapshot 继续。"
            ),
        );
    }
    let Some((_title, url)) = current else {
        return fail(
            call,
            "NO_PAGE",
            "读不到当前页面地址；请先 browser_open 打开页面再接管",
            true,
        );
    };
    if !url.starts_with("http") {
        return fail(
            call,
            "NO_PAGE",
            "当前页面不是 http(s) 页面，无法人工接管；请先 browser_open 打开目标页面",
            true,
        );
    }
    // 关闭无头会话、以 headed 重开并回到同一页面；profile 保留，登录态不丢。
    *guard = None;
    match BrowserSession::launch(true) {
        Ok(mut b) => {
            let nav = b.navigate(&url);
            match nav {
                Ok((title, final_url)) => {
                    *guard = Some(b);
                    ToolResult::ok(
                        call,
                        format!(
                            "已切换为可见浏览器窗口：{title}\nURL：{final_url}\n页面已重新加载（登录状态会保留），请用户直接在窗口里输入验证码/账号密码并完成登录（若没看到窗口，从任务栏切到 Edge/Chrome）。\n下一步：先 ask_user 请用户确认「已完成」，再 browser_snapshot 探测页面。"
                        ),
                    )
                }
                Err(e) => {
                    *guard = Some(b);
                    fail(
                        call,
                        "NAV_FAILED",
                        format!(
                            "已打开可见浏览器窗口，但重新加载 {url} 失败：{e}\n用户可以手动在窗口里打开目标页面；完成后继续。"
                        ),
                        true,
                    )
                }
            }
        }
        Err(e) => fail(call, "BROWSER_LAUNCH", e, true),
    }
}

fn snapshot(call: &ToolCall) -> ToolResult {
    let limit = call
        .args()
        .get("limit")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(snapshot::DEFAULT_LIMIT);
    match with_existing(call, |b| {
        let (title, url) = b.title_url()?;
        let ax = b
            .cdp
            .call("Accessibility.getFullAXTree", json!({}), cdp::CALL_TIMEOUT)?;
        let snap = snapshot::build(&ax, &title, &url, limit);
        b.refs = snap.refs.clone();
        Ok(snap.text)
    }) {
        Ok(text) => ToolResult::ok(call, text),
        Err(r) => r,
    }
}

fn click(call: &ToolCall) -> ToolResult {
    let Some(target) = target_arg(call) else {
        return bad_args(call, "缺少 target：用快照编号 [n] 或 CSS 选择器");
    };
    match with_existing(call, |b| {
        if let Some(n) = parse_ref(&target) {
            let backend = b
                .refs
                .get(&n)
                .copied()
                .ok_or_else(|| format!("编号 [{n}] 不存在，请先 browser_snapshot 拿最新编号"))?;
            b.click_ref(backend)?;
        } else {
            b.click_selector(&target)?;
        }
        let (title, url) = b.title_url().unwrap_or_default();
        Ok(format!(
            "已点击 {target}\n当前页面：{title}（{url}）\n用 browser_snapshot 确认点击结果。"
        ))
    }) {
        Ok(msg) => ToolResult::ok(call, msg),
        Err(r) => r,
    }
}

fn type_text(call: &ToolCall) -> ToolResult {
    let a = call.args();
    let Some(target) = target_arg(call) else {
        return bad_args(call, "缺少 target：用快照编号 [n] 或 CSS 选择器");
    };
    let Some(text) = a.get("text").and_then(Value::as_str) else {
        return bad_args(call, "缺少 text");
    };
    let secret = a.get("secret").and_then(Value::as_bool).unwrap_or(false);
    let submit = a.get("submit").and_then(Value::as_bool).unwrap_or(false);
    let chars = text.chars().count();
    match with_existing(call, |b| {
        if let Some(n) = parse_ref(&target) {
            let backend = b
                .refs
                .get(&n)
                .copied()
                .ok_or_else(|| format!("编号 [{n}] 不存在，请先 browser_snapshot 拿最新编号"))?;
            b.type_ref(backend, text)?;
        } else {
            b.type_selector(&target, text)?;
        }
        if submit {
            b.press("Enter")?;
        }
        let kind = if secret { "密码（内容已隐藏）" } else { "文本" };
        let tail = if submit { "，并已按 Enter" } else { "" };
        Ok(format!("已在 {target} 输入{kind} {chars} 个字符{tail}。"))
    }) {
        Ok(msg) => ToolResult::ok(call, msg),
        Err(r) => r,
    }
}

fn press(call: &ToolCall) -> ToolResult {
    let key = arg_str(&call.args(), "key");
    if key.is_empty() {
        return bad_args(call, "缺少 key");
    }
    match with_existing(call, |b| {
        b.press(&key)?;
        Ok(format!("已按下 {key}。"))
    }) {
        Ok(msg) => ToolResult::ok(call, msg),
        Err(r) => r,
    }
}

fn wait(call: &ToolCall) -> ToolResult {
    let a = call.args();
    let selector = arg_str(&a, "selector");
    let ms = a.get("ms").and_then(Value::as_u64);
    let timeout = a
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(WAIT_CAP_MS)
        .clamp(200, WAIT_CAP_MS);
    if !selector.is_empty() {
        return match with_existing(call, |b| {
            let found = b.wait_selector(&selector, Duration::from_millis(timeout))?;
            Ok(if found { "found" } else { "timeout" }.to_string())
        }) {
            Ok(v) if v == "found" => ToolResult::ok(call, format!("元素已出现：{selector}")),
            Ok(_) => fail(
                call,
                "TIMEOUT",
                format!("等待 {timeout}ms 后仍没出现：{selector}"),
                true,
            ),
            Err(r) => r,
        };
    }
    if let Some(ms) = ms {
        let ms = ms.min(WAIT_CAP_MS);
        if !browser_open_now() {
            return fail(call, "NO_BROWSER", "还没有打开页面，先调用 browser_open", false);
        }
        std::thread::sleep(Duration::from_millis(ms));
        return ToolResult::ok(call, format!("已等待 {ms}ms。"));
    }
    bad_args(call, "需要 selector 或 ms 之一")
}

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
            at: neverwork_core::config::now_secs(),
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
    let include_console = a
        .get("include_console")
        .and_then(Value::as_bool)
        .unwrap_or(true);
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
                    format!(
                        "用例「{}」引用了不存在的截图 #{n}；可用编号：{avail}",
                        case.name
                    ),
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
            report::ShotInfo {
                title: s.title.clone(),
                data_uri,
                note,
            }
        })
        .collect();
    let console_items: Vec<report::ConsoleItem> = if include_console {
        console
            .iter()
            .filter(|l| {
                matches!(
                    l.level.as_str(),
                    "error" | "warning" | "warn" | "assert" | "http" | "net"
                )
            })
            .take(100)
            .map(|l| report::ConsoleItem {
                level: l.level.clone(),
                text: l.text.clone(),
            })
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
        root: if ctx.is_remote() {
            String::new()
        } else {
            // 去掉 canonicalize 带来的 `\\?\` 前缀，报告里只显示可读路径。
            let r = ctx.root.to_string_lossy();
            r.strip_prefix(r"\\?\").unwrap_or(r.as_ref()).to_string()
        },
        cases: cases.clone(),
        shots: infos,
        console: console_items,
    };
    let html = report::render_html(&input);
    let dir = artifacts_dir(ctx);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return fail(
            call,
            "BROWSER_ERROR",
            format!("创建 artifacts 目录失败（{}）：{e}", dir.display()),
            true,
        );
    }
    let stamp = neverwork_core::config::local_timestamp();
    let mut file = dir.join(format!("测试报告-{stamp}.html"));
    let mut seq = 2;
    while file.exists() {
        file = dir.join(format!("测试报告-{stamp}-{seq}.html"));
        seq += 1;
    }
    if let Err(e) = std::fs::write(&file, html) {
        return fail(
            call,
            "BROWSER_ERROR",
            format!("写入测试报告失败（{}）：{e}", file.display()),
            true,
        );
    }
    let abs = file.to_string_lossy().to_string();
    // 测试过程清理：截图已 base64 内嵌进报告，原始 png 不再保留（用户要求）。
    // 只删截图文件，不碰报告本身与其他产物。
    let mut removed = 0usize;
    for s in &shots {
        if std::fs::remove_file(&s.path).is_ok() {
            removed += 1;
        }
    }
    if let Some(b) = lock().as_mut() {
        b.shots.clear();
        b.report_written = true;
    }
    let passed = cases
        .iter()
        .filter(|c| c.status == report::CaseStatus::Pass)
        .count();
    let failed = cases
        .iter()
        .filter(|c| c.status == report::CaseStatus::Fail)
        .count();
    let skipped = cases.len() - passed - failed;
    let embedded = input.shots.iter().filter(|s| s.data_uri.is_some()).count();
    ToolResult::ok(
        call,
        format!(
            "测试报告已生成：{abs}\n用例 {} 个：通过 {passed} / 失败 {failed} / 跳过 {skipped}\n（已内嵌 {embedded} 张截图与 console 错误；测试过程截图已清理 {removed} 个；在产物里点击用默认浏览器打开）",
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
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
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
            steps: item
                .get("steps")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
            expected: item
                .get("expected")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
            actual: item
                .get("actual")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
            chapter: item
                .get("chapter")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
            screenshots,
        });
    }
    Ok(out)
}

fn console(call: &ToolCall) -> ToolResult {
    let clear = call
        .args()
        .get("clear")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match with_existing(call, |b| {
        let lines = b.console_lines();
        if clear {
            b.clear_console();
        }
        let interesting: Vec<&cdp::ConsoleLine> = lines
            .iter()
            .filter(|l| {
                matches!(
                    l.level.as_str(),
                    "error" | "warning" | "warn" | "assert" | "http" | "net"
                )
            })
            .collect();
        if interesting.is_empty() {
            return Ok("没有 console 错误或失败的网络请求。".to_string());
        }
        let mut out = format!("console / 网络问题（最近 {} 条）：", interesting.len());
        for l in interesting.iter().take(100) {
            out.push_str(&format!("\n[{}] {}", l.level, l.text));
        }
        Ok(out)
    }) {
        Ok(msg) => ToolResult::ok(call, msg),
        Err(r) => r,
    }
}

fn close(call: &ToolCall) -> ToolResult {
    let had = lock().take();
    match had {
        Some(b) => {
            // 显式关闭 = 结束整个浏览器任务：先出收尾文案，再关进程、删 profile
            // （profile 会被下次任务重新创建，登录态一并清掉）。
            let profile = b.profile.clone();
            let msg = if b.report_written {
                // 报告已内嵌图片：收尾时把报告之后新截的过程图也清掉。
                let mut removed = 0usize;
                for s in &b.shots {
                    if std::fs::remove_file(&s.path).is_ok() {
                        removed += 1;
                    }
                }
                if removed > 0 {
                    format!(
                        "浏览器已关闭，临时配置目录已清理。{removed} 张残留测试截图已删除（测试报告已内嵌图片，不受影响）。"
                    )
                } else {
                    "浏览器已关闭，临时配置目录已清理。测试过程截图均已清理，报告保留。".to_string()
                }
            } else if !b.shots.is_empty() {
                // 没出报告：截图是唯一证据，先不删；会话已关闭，无法再补内嵌报告。
                format!(
                    "浏览器已关闭，临时配置目录已清理。\n注意：本次有 {} 张过程截图还没归档（仍保留在原路径，未删除）；报告需要在关闭浏览器前生成，本次会话已结束。",
                    b.shots.len()
                )
            } else {
                "浏览器已关闭，临时配置目录已清理。".to_string()
            };
            // 先让 Drop 关浏览器（否则 Windows 下 profile 文件被占用删不掉），再删目录。
            drop(b);
            let _ = std::fs::remove_dir_all(&profile);
            ToolResult::ok(call, msg)
        }
        None => ToolResult::ok(call, "当前没有打开浏览器。"),
    }
}

/// 需要已打开的浏览器；崩溃时重启一次再重试。
fn with_existing<T>(
    call: &ToolCall,
    mut f: impl FnMut(&mut BrowserSession) -> Result<T, String>,
) -> Result<T, ToolResult> {
    let mut guard = lock();
    if guard.is_none() {
        return Err(fail(
            call,
            "NO_BROWSER",
            "还没有打开页面，先调用 browser_open 打开目标地址",
            false,
        ));
    }
    let res = {
        let b = guard.as_mut().expect("上面刚检查过");
        f(b)
    };
    match res {
        Ok(v) => Ok(v),
        Err(e) => {
            let dead = guard.as_mut().map(|b| !b.alive()).unwrap_or(true);
            if dead {
                let headed = guard.as_ref().map(|b| b.headed).unwrap_or(false);
                *guard = None;
                match BrowserSession::launch(headed) {
                    Ok(mut b2) => {
                        let retry = f(&mut b2);
                        *guard = Some(b2);
                        match retry {
                            Ok(v) => Ok(v),
                            Err(e2) => Err(fail(
                                call,
                                "BROWSER_CRASH",
                                format!("浏览器崩溃后已重启，但操作仍失败：{e2}"),
                                true,
                            )),
                        }
                    }
                    Err(e2) => Err(fail(call, "BROWSER_LAUNCH", e2, true)),
                }
            } else {
                Err(fail(call, "BROWSER_ERROR", e, true))
            }
        }
    }
}

fn browser_open_now() -> bool {
    lock().is_some()
}

/// 截图落盘目录：本地工作区用 `<root>/artifacts`，远程/无工作区用配置目录。
fn artifacts_dir(ctx: &ToolContext) -> PathBuf {
    if !ctx.is_remote() {
        return ctx.root.join("artifacts");
    }
    config_dir().join("artifacts")
}

fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("NW_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".neverwork")
}

/// 文件名白名单：字母数字/中文/下划线/短横线，其余换成 `-`，长度上限 40。
fn sanitize_name(name: &str) -> String {
    let mut s: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect();
    if s.is_empty() {
        s = format!("browser-{}", now_ms());
    }
    s
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_refs_only_for_numbers() {
        assert_eq!(parse_ref("[3]"), Some(3));
        assert_eq!(parse_ref("12"), Some(12));
        assert_eq!(parse_ref(" button "), None);
        assert_eq!(parse_ref("#login"), None);
    }

    #[test]
    fn sanitize_name_keeps_safe_chars() {
        assert_eq!(sanitize_name("登录页"), "登录页");
        assert_eq!(sanitize_name("a/b:c*d"), "a-b-c-d");
        assert!(sanitize_name("").starts_with("browser-"));
    }

    #[test]
    fn headed_defaults_to_visible_unless_headless_env() {
        assert!(headed_from_env(None), "默认显示窗口，用户能看到操作");
        assert!(headed_from_env(Some("0")));
        assert!(!headed_from_env(Some("1")), "NW_BROWSER_HEADLESS=1 才用无头");
    }
}
