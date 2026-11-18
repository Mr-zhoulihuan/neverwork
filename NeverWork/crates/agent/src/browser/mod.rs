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

/// 执行一个 `browser_*` 工具。
pub fn execute(call: &ToolCall, ctx: &ToolContext) -> ToolResult {
    match call.name.as_str() {
        "browser_open" => open(call),
        "browser_snapshot" => snapshot(call),
        "browser_click" => click(call),
        "browser_type" => type_text(call),
        "browser_press" => press(call),
        "browser_wait" => wait(call),
        "browser_screenshot" => screenshot(call, ctx),
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
    let headed = a.get("headed").and_then(Value::as_bool).unwrap_or(false);
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
        let abs = file.to_string_lossy().to_string();
        let md = abs.replace('\\', "/");
        Ok(format!(
            "截图已保存：{abs}\n![{name}]({md})\n（图片是给用户看的；页面内容请用 browser_snapshot 判断）"
        ))
    }) {
        Ok(msg) => ToolResult::ok(call, msg),
        Err(r) => r,
    }
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
        Some(_) => ToolResult::ok(call, "浏览器已关闭，临时配置目录已清理。"),
        None => ToolResult::ok(call, "当前没有打开浏览器。"),
    }
}

/// 需要已打开的浏览器；崩溃时重启一次再重试。
fn with_existing(
    call: &ToolCall,
    mut f: impl FnMut(&mut BrowserSession) -> Result<String, String>,
) -> Result<String, ToolResult> {
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
}
