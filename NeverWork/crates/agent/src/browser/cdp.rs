//! 浏览器进程与 CDP 会话。
//!
//! 进程策略：系统 Edge（找不到再 Chrome）以 `--headless=new` 启动，
//! `--remote-debugging-port=0` 让浏览器自己挑端口，再从 `DevToolsActivePort`
//! 读回来——不写死端口，也不用扫端口。`user-data-dir` 用进程级固定的临时目录：
//! 会话内重启/人工接管复用同一 profile，cookie 与登录态不丢；
//! `browser_close` 显式关闭时才删除，崩溃残留由下次启动按天清理。
//!
//! CDP 传输：读写两条线程共用一条 TCP 连接；按 `id` 把响应分发给等待中的
//! `call`，事件（console / 网络失败）进环形缓冲供 `browser_console` 读取。

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Value};

use super::ws;

/// 单条 CDP 命令超时。
pub(crate) const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// 页面导航后等 `readyState=complete` 的上限。
pub(crate) const NAV_TIMEOUT: Duration = Duration::from_secs(20);
/// 等 DevToolsActivePort 文件出现的上限。
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(15);
/// console/网络事件缓冲上限。
const CONSOLE_CAP: usize = 200;
/// 单条事件文本上限。
const LINE_CLAMP: usize = 500;

/// 一条 console / 网络记录。
#[derive(Debug, Clone)]
pub(crate) struct ConsoleLine {
    pub level: String,
    pub text: String,
}

/// 本次浏览器会话内的一张截图（测试报告素材，编号 = 清单下标 + 1）。
#[derive(Debug, Clone)]
pub struct Shot {
    pub path: PathBuf,
    /// 截图时的页面标题。
    pub title: String,
    /// 截图时间（Unix 秒）。
    pub at: u64,
}

/// 一个活跃的浏览器实例。
pub(crate) struct BrowserSession {
    child: Child,
    /// 进程级固定 profile（`session_profile`）：重启/接管复用，browser_close 时删。
    pub(crate) profile: PathBuf,
    pub cdp: Cdp,
    pub headed: bool,
    /// 最近一次快照的编号 → backendDOMNodeId。
    pub refs: HashMap<u32, i64>,
    console: Arc<Mutex<Vec<ConsoleLine>>>,
    /// 本会话截图清单：`browser_report` 按编号内嵌进报告。
    pub shots: Vec<Shot>,
    /// 会话开始时间（报告显示「浏览器会话耗时」）。
    pub started: Instant,
    /// 浏览器可执行文件名（如 msedge.exe），报告元信息用。
    pub browser: String,
    /// 本会话是否已生成测试报告：成功后截图已内嵌、过程 png 会被清理。
    pub report_written: bool,
}

impl BrowserSession {
    /// 启动浏览器并连上第一个 page target。
    pub fn launch(headed: bool) -> Result<Self, String> {
        let exe = find_browser().ok_or_else(|| {
            "未找到可用的 Edge/Chrome；可用环境变量 NW_BROWSER_PATH 指定浏览器可执行文件"
                .to_string()
        })?;
        // 进程级固定 profile：重启/接管复用，登录态不丢（见 session_profile）。
        let profile = session_profile().to_path_buf();
        std::fs::create_dir_all(&profile).map_err(|e| format!("创建浏览器 profile 失败：{e}"))?;
        // 复用目录时先清掉上一轮的端口文件（避免读到旧端口），
        // 并给上一个实例一点时间退出、释放目录锁。
        if profile.join("DevToolsActivePort").exists() {
            let _ = std::fs::remove_file(profile.join("DevToolsActivePort"));
            let _ = std::fs::remove_file(profile.join("SingletonLock"));
            std::thread::sleep(Duration::from_millis(300));
        }

        // 浏览器启动日志（stderr）落 profile：失败时把尾部带进错误信息。
        let log_path = profile.join("neverwork-launch.log");
        let log = std::fs::File::create(&log_path).ok();
        let spawn = || -> Result<Child, String> {
            let mut cmd = Command::new(&exe);
            if !headed {
                cmd.arg("--headless=new");
            }
            cmd.arg("--remote-debugging-port=0")
                .arg(format!("--user-data-dir={}", profile.display()))
                .arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--disable-gpu")
                .arg("--disable-extensions")
                .arg("--disable-sync")
                .arg("--disable-background-networking")
                .arg("--window-size=1280,800")
                .arg("about:blank")
                .stdin(Stdio::null());
            // 诊断日志：Edge 的关键报错和 "DevTools listening on ..." 都走 stderr，
            // 不再直接丢弃；两个管道都指向同一文件。
            let out = log.as_ref().and_then(|f| f.try_clone().ok());
            let err = log.as_ref().and_then(|f| f.try_clone().ok());
            cmd.stdout(out.map(Stdio::from).unwrap_or_else(Stdio::null))
                .stderr(err.map(Stdio::from).unwrap_or_else(Stdio::null));
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                // 不要终端里闪一个控制台窗口
                cmd.creation_flags(0x0800_0000);
            }
            cmd.spawn()
                .map_err(|e| format!("启动浏览器失败（{}）：{e}", exe.display()))
        };
        let mut child = match spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_file(profile.join("DevToolsActivePort"));
                return Err(e);
            }
        };

        // 浏览器启动后会写 DevToolsActivePort：首行端口、次行 browser ws 路径。
        // 注意：Edge 的启动器进程可能先把浏览器主进程转交给另一个 msedge 后
        // 自己先退出（本机实测会这样），但主进程仍继续往 profile 写端口。
        // 所以「拉起的进程退出」不能立刻判死：再给 3 秒等端口出现。
        let port_file = profile.join("DevToolsActivePort");
        let deadline = Instant::now() + LAUNCH_TIMEOUT;
        let mut child_exited_at: Option<Instant> = None;
        let mut port = None;
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(&port_file) {
                if let Some(p) = text
                    .lines()
                    .next()
                    .and_then(|l| l.trim().parse::<u16>().ok())
                {
                    port = Some(p);
                    break;
                }
            }
            if child_exited_at.is_none() && child.try_wait().ok().flatten().is_some() {
                child_exited_at = Some(Instant::now());
            }
            // 转交后的进程 3 秒内还没写端口，说明这次真没起来，别让用户干等 15s。
            if matches!(child_exited_at, Some(t) if t.elapsed() > Duration::from_secs(3)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let Some(port) = port else {
            let _ = child.kill();
            let detail = launch_log_tail(&log_path);
            let _ = std::fs::remove_file(&port_file);
            let what = if child_exited_at.is_some() {
                "浏览器进程启动后退出，且没有写出调试端口"
            } else {
                "等待浏览器调试端口超时"
            };
            let hint = if detail.is_empty() {
                String::new()
            } else {
                format!("；浏览器日志：{detail}")
            };
            return Err(format!(
                "{what}{hint}（可尝试升级/重装 Edge/Chrome，或用 NW_BROWSER_PATH 指定浏览器）"
            ));
        };

        let ws_url = match page_ws_url(port) {
            Ok(u) => u,
            Err(e) => {
                let _ = child.kill();
                let _ = std::fs::remove_file(&port_file);
                return Err(e);
            }
        };
        let console = Arc::new(Mutex::new(Vec::new()));
        let cdp = match Cdp::connect(&ws_url, console.clone()) {
            Ok(c) => c,
            Err(e) => {
                let _ = child.kill();
                let _ = std::fs::remove_file(&port_file);
                return Err(e);
            }
        };
        // 事件源：console、日志、网络失败。失败不致命（老版本可能缺域）。
        for domain in ["Page.enable", "Runtime.enable", "Log.enable", "Network.enable"] {
            let _ = cdp.call(domain, json!({}), Duration::from_secs(10));
        }
        Ok(Self {
            child,
            profile,
            cdp,
            headed,
            refs: HashMap::new(),
            console,
            shots: Vec::new(),
            started: Instant::now(),
            browser: exe
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Edge/Chrome")
                .to_string(),
            report_written: false,
        })
    }

    /// 浏览器还活着吗（崩溃检测）。
    /// 不能用 `child.try_wait()`：Edge 的启动器进程常在把主进程转交后先退出，
    /// 那样会把「正常运行」误判成「崩溃」→ 重启一个全新 profile，登录态全丢。
    /// 以 CDP 能应答为准（Browser.getVersion 走浏览器进程，不依赖页面）。
    pub fn alive(&mut self) -> bool {
        self.cdp
            .call("Browser.getVersion", json!({}), Duration::from_millis(800))
            .is_ok()
    }

    /// 打开一个 URL，等页面就绪后返回（标题，最终 URL）。
    pub fn navigate(&mut self, url: &str) -> Result<(String, String), String> {
        self.refs.clear();
        let r = self
            .cdp
            .call("Page.navigate", json!({ "url": url }), CALL_TIMEOUT)?;
        if let Some(err) = r.get("errorText").and_then(Value::as_str) {
            return Err(format!("打开页面失败：{err}"));
        }
        // 有些页面永远到不了 complete（长轮询），超时不算硬错误：
        // 只要标题/URL 能读到，后续 snapshot 仍可操作。
        let _ = self.wait_ready(NAV_TIMEOUT);
        self.title_url()
    }

    pub fn title_url(&self) -> Result<(String, String), String> {
        let v = self.eval_value("({title: document.title, url: location.href})")?;
        let title = v
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let url = v
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok((title, url))
    }

    pub fn wait_ready(&self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.eval_string("document.readyState").ok().as_deref() == Some("complete") {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("等待页面加载超时".into());
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    /// `Runtime.evaluate`，返回 RemoteObject 的 `value`。
    pub fn eval_value(&self, expr: &str) -> Result<Value, String> {
        let r = self.cdp.call(
            "Runtime.evaluate",
            json!({
                "expression": expr,
                "returnByValue": true,
                "awaitPromise": true,
            }),
            CALL_TIMEOUT,
        )?;
        if let Some(ex) = r.get("exceptionDetails") {
            let msg = ex
                .get("exception")
                .and_then(|e| e.get("description"))
                .and_then(Value::as_str)
                .or_else(|| ex.get("text").and_then(Value::as_str))
                .unwrap_or("页面脚本执行失败");
            return Err(format!("页面脚本异常：{msg}"));
        }
        Ok(r.get("result")
            .and_then(|o| o.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    pub fn eval_string(&self, expr: &str) -> Result<String, String> {
        Ok(match self.eval_value(expr)? {
            Value::String(s) => s,
            Value::Null => String::new(),
            other => other.to_string(),
        })
    }

    fn resolve_node(&self, backend: i64) -> Result<String, String> {
        let r = self.cdp.call(
            "DOM.resolveNode",
            json!({ "backendNodeId": backend }),
            CALL_TIMEOUT,
        )?;
        r.get("object")
            .and_then(|o| o.get("objectId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "元素已失效（页面可能刷新过），请重新 browser_snapshot".to_string())
    }

    pub fn click_ref(&self, backend: i64) -> Result<(), String> {
        let oid = self.resolve_node(backend)?;
        let r = self.cdp.call(
            "Runtime.callFunctionOn",
            json!({
                "objectId": oid,
                "functionDeclaration": "function(){ this.scrollIntoView({block:'center'}); this.click(); return true; }",
                "returnByValue": true,
            }),
            CALL_TIMEOUT,
        );
        let _ = self.cdp.call(
            "Runtime.releaseObject",
            json!({ "objectId": oid }),
            Duration::from_secs(5),
        );
        r?;
        self.settle();
        Ok(())
    }

    pub fn click_selector(&self, selector: &str) -> Result<(), String> {
        let sel = js_str(selector);
        let expr = format!(
            "(() => {{ const el = document.querySelector({sel}); if (!el) return false; \
             el.scrollIntoView({{block:'center'}}); el.click(); return true; }})()"
        );
        if self.eval_value(&expr)? != Value::Bool(true) {
            return Err(format!("找不到元素：{selector}"));
        }
        self.settle();
        Ok(())
    }

    /// 点击/导航后的短暂稳定期：只等 readyState，不追网络空闲（长连接页面会假死）。
    fn settle(&self) {
        std::thread::sleep(Duration::from_millis(300));
        let _ = self.wait_ready(Duration::from_secs(3));
    }

    pub fn type_ref(&self, backend: i64, text: &str) -> Result<(), String> {
        let oid = self.resolve_node(backend)?;
        let r = self.cdp.call(
            "Runtime.callFunctionOn",
            json!({
                "objectId": oid,
                "functionDeclaration": TYPE_FN,
                "arguments": [{ "value": text }],
                "returnByValue": true,
            }),
            CALL_TIMEOUT,
        );
        let _ = self.cdp.call(
            "Runtime.releaseObject",
            json!({ "objectId": oid }),
            Duration::from_secs(5),
        );
        let r = r?;
        if r.get("result").and_then(|o| o.get("value")) != Some(&Value::Bool(true)) {
            return Err("该元素无法输入文本（不是输入框）".into());
        }
        Ok(())
    }

    pub fn type_selector(&self, selector: &str, text: &str) -> Result<(), String> {
        let sel = js_str(selector);
        let txt = js_str(text);
        let expr = format!(
            "(() => {{ const el = document.querySelector({sel}); if (!el) return false; \
             return ({TYPE_JS_CALL})(el, {txt}); }})()"
        );
        if self.eval_value(&expr)? != Value::Bool(true) {
            return Err(format!("找不到元素或无法输入：{selector}"));
        }
        Ok(())
    }

    pub fn press(&self, key: &str) -> Result<(), String> {
        let (code, vk, text) = match key {
            "Enter" => ("Enter", 13, Some("\r")),
            "Tab" => ("Tab", 9, None),
            "Escape" => ("Escape", 27, None),
            "ArrowUp" => ("ArrowUp", 38, None),
            "ArrowDown" => ("ArrowDown", 40, None),
            "ArrowLeft" => ("ArrowLeft", 37, None),
            "ArrowRight" => ("ArrowRight", 39, None),
            "Backspace" => ("Backspace", 8, None),
            other => return Err(format!("不支持的按键：{other}")),
        };
        let mut down = json!({
            "type": "keyDown",
            "key": key,
            "code": code,
            "windowsVirtualKeyCode": vk,
            "nativeVirtualKeyCode": vk,
        });
        if let Some(t) = text {
            down["text"] = json!(t);
        }
        self.cdp.call("Input.dispatchKeyEvent", down, CALL_TIMEOUT)?;
        self.cdp.call(
            "Input.dispatchKeyEvent",
            json!({
                "type": "keyUp",
                "key": key,
                "code": code,
                "windowsVirtualKeyCode": vk,
                "nativeVirtualKeyCode": vk,
            }),
            CALL_TIMEOUT,
        )?;
        Ok(())
    }

    /// 等元素出现；超时返回 false。
    pub fn wait_selector(&self, selector: &str, timeout: Duration) -> Result<bool, String> {
        let sel = js_str(selector);
        let expr = format!("document.querySelector({sel}) !== null");
        let deadline = Instant::now() + timeout;
        loop {
            if self.eval_value(&expr)? == Value::Bool(true) {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    pub fn screenshot_png(&self) -> Result<Vec<u8>, String> {
        let r = self.cdp.call(
            "Page.captureScreenshot",
            json!({ "format": "png" }),
            CALL_TIMEOUT,
        )?;
        let data = r
            .get("data")
            .and_then(Value::as_str)
            .ok_or("截图返回为空")?;
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| format!("截图数据解码失败：{e}"))
    }

    pub fn console_lines(&self) -> Vec<ConsoleLine> {
        self.console
            .lock()
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    pub fn clear_console(&self) {
        if let Ok(mut v) = self.console.lock() {
            v.clear();
        }
    }
}

impl Drop for BrowserSession {
    fn drop(&mut self) {
        // 先记下浏览器主进程 id：Edge 会把主进程转交给另一个 msedge，我们拉起的
        // child 只是启动器——kill(child) 杀不到真浏览器，Browser.close 没送到时会
        // 留下一整棵 msedge（实测 18 个）。
        let browser_pid = self
            .cdp
            .call("SystemInfo.getProcessInfo", json!({}), Duration::from_secs(2))
            .ok()
            .and_then(|v| {
                v.get("processInfo")?.as_array()?.iter().find_map(|p| {
                    let t = p.get("type")?.as_str()?;
                    (t == "browser").then(|| p.get("id")?.as_u64()).flatten()
                })
            });
        // 先礼貌关浏览器，再兜底 kill 启动器与整棵浏览器进程树。
        let _ = self
            .cdp
            .call("Browser.close", json!({}), Duration::from_secs(3));
        let _ = self.child.kill();
        let _ = self.child.wait();
        #[cfg(windows)]
        if let Some(pid) = browser_pid {
            use std::os::windows::process::CommandExt;
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .creation_flags(0x0800_0000)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(not(windows))]
        let _ = browser_pid;
        // profile 目录保留：同一进程内重启/接管会复用它，cookie 与登录态不丢；
        // 只有 browser_close（显式关闭）时删。
    }
}

/// 输入框赋值函数：React/Vue 受控组件必须走原型 setter + input 事件，
/// 直接改 `el.value` 会被框架的受控逻辑吞掉。
const TYPE_FN: &str = "function(text){ \
    this.focus(); \
    if (this instanceof HTMLInputElement || this instanceof HTMLTextAreaElement) { \
        const proto = this instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype; \
        const setter = Object.getOwnPropertyDescriptor(proto, 'value').set; \
        if (setter) setter.call(this, text); else this.value = text; \
        this.dispatchEvent(new Event('input', { bubbles: true })); \
        this.dispatchEvent(new Event('change', { bubbles: true })); \
    } else if (this.isContentEditable) { \
        this.textContent = text; \
        this.dispatchEvent(new InputEvent('input', { bubbles: true, data: text })); \
    } else { \
        this.value = text; \
        this.dispatchEvent(new Event('input', { bubbles: true })); \
    } \
    return true; }";

/// 选择器版赋值（内联进 `Runtime.evaluate` 的表达式）。
const TYPE_JS_CALL: &str = "((el, text) => { \
    el.focus(); \
    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) { \
        const proto = el instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype; \
        const setter = Object.getOwnPropertyDescriptor(proto, 'value').set; \
        if (setter) setter.call(el, text); else el.value = text; \
        el.dispatchEvent(new Event('input', { bubbles: true })); \
        el.dispatchEvent(new Event('change', { bubbles: true })); \
    } else if (el.isContentEditable) { \
        el.textContent = text; \
        el.dispatchEvent(new InputEvent('input', { bubbles: true, data: text })); \
    } else { \
        el.value = text; \
        el.dispatchEvent(new Event('input', { bubbles: true })); \
    } \
    return true; })";

/// JavaScript 字符串字面量（含引号与转义）。
fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// CDP 客户端（可 clone，内部共享连接）。
#[derive(Clone)]
pub(crate) struct Cdp {
    inner: Arc<CdpInner>,
}

struct CdpInner {
    writer: Arc<Mutex<ws::WsWriter>>,
    pending: Mutex<HashMap<u64, flume::Sender<Value>>>,
    console: Arc<Mutex<Vec<ConsoleLine>>>,
    next_id: AtomicU64,
    closed: AtomicBool,
}

impl Cdp {
    fn connect(ws_url: &str, console: Arc<Mutex<Vec<ConsoleLine>>>) -> Result<Self, String> {
        let (host, port, path) = parse_ws_url(ws_url)?;
        let (mut reader, writer) = ws::connect(&host, port, &path, CALL_TIMEOUT)?;
        let inner = Arc::new(CdpInner {
            writer,
            pending: Mutex::new(HashMap::new()),
            console,
            next_id: AtomicU64::new(1),
            closed: AtomicBool::new(false),
        });
        let reader_inner = inner.clone();
        std::thread::Builder::new()
            .name("nw-cdp-reader".into())
            .spawn(move || {
                loop {
                    match reader.read_message() {
                        Ok(text) => {
                            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                                continue;
                            };
                            if let Some(id) = v.get("id").and_then(Value::as_u64) {
                                if let Some(tx) =
                                    reader_inner.pending.lock().ok().and_then(|mut m| m.remove(&id))
                                {
                                    let _ = tx.send(v);
                                }
                            } else if let Some(method) =
                                v.get("method").and_then(Value::as_str)
                            {
                                handle_event(method, &v, &reader_inner.console);
                            }
                        }
                        Err(_) => {
                            reader_inner.closed.store(true, Ordering::Relaxed);
                            if let Ok(mut m) = reader_inner.pending.lock() {
                                for (_, tx) in m.drain() {
                                    let _ = tx.send(json!({ "__closed": true }));
                                }
                            }
                            break;
                        }
                    }
                }
            })
            .map_err(|e| format!("启动 CDP 读线程失败：{e}"))?;
        Ok(Self { inner })
    }

    /// 发一条命令并等响应；`timeout` 到点返回错误（调用方决定是否重试）。
    pub fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        if self.inner.closed.load(Ordering::Relaxed) {
            return Err("浏览器连接已断开".into());
        }
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = flume::bounded(1);
        if let Ok(mut m) = self.inner.pending.lock() {
            m.insert(id, tx);
        }
        let msg = json!({ "id": id, "method": method, "params": params });
        {
            let mut w = self
                .inner
                .writer
                .lock()
                .map_err(|_| "浏览器写锁中毒".to_string())?;
            if let Err(e) = w.send_text(&msg.to_string()) {
                if let Ok(mut m) = self.inner.pending.lock() {
                    m.remove(&id);
                }
                return Err(e);
            }
        }
        let deadline = Instant::now() + timeout;
        loop {
            if self.inner.closed.load(Ordering::Relaxed) {
                if let Ok(mut m) = self.inner.pending.lock() {
                    m.remove(&id);
                }
                return Err("浏览器连接已断开".into());
            }
            let now = Instant::now();
            if now >= deadline {
                if let Ok(mut m) = self.inner.pending.lock() {
                    m.remove(&id);
                }
                return Err(format!("CDP {method} 超时（{}s）", timeout.as_secs()));
            }
            let wait = (deadline - now).min(Duration::from_millis(100));
            match rx.recv_timeout(wait) {
                Ok(v) => {
                    if v.get("__closed").is_some() {
                        return Err("浏览器连接已断开".into());
                    }
                    if let Some(err) = v.get("error") {
                        let msg = err
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("未知错误");
                        return Err(format!("CDP {method} 失败：{msg}"));
                    }
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                }
                Err(flume::RecvTimeoutError::Timeout) => continue,
                Err(flume::RecvTimeoutError::Disconnected) => {
                    return Err("浏览器连接已断开".into())
                }
            }
        }
    }
}

/// 解析 `ws://127.0.0.1:9222/devtools/page/xxx`。
fn parse_ws_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url
        .strip_prefix("ws://")
        .ok_or_else(|| format!("不支持的调试地址：{url}"))?;
    let (hostport, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().map_err(|_| "调试端口非法")?),
        None => (hostport.to_string(), 80),
    };
    Ok((host, port, path))
}

/// 从浏览器的 HTTP 端点取第一个 page target 的 ws 地址。
fn page_ws_url(port: u16) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{port}/json/list");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(resp) = ureq::get(&url).call() {
            let mut body = resp.into_body();
            if let Ok(text) = body.with_config().limit(1024 * 1024).read_to_string() {
                if let Ok(list) = serde_json::from_str::<Value>(&text) {
                    if let Some(ws) = list.as_array().and_then(|arr| {
                        arr.iter()
                            .find(|t| {
                                t.get("type").and_then(Value::as_str) == Some("page")
                            })
                            .and_then(|t| t.get("webSocketDebuggerUrl"))
                            .and_then(Value::as_str)
                    }) {
                        return Ok(ws.to_string());
                    }
                }
            }
        }
        if Instant::now() >= deadline {
            return Err("浏览器没有可用的页面调试目标".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// 取浏览器启动日志的尾部（错误诊断用；一行显示，避免多行把提示撑乱）。
fn launch_log_tail(path: &std::path::Path) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let tail: String = text
        .chars()
        .rev()
        .take(600)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    tail.trim().replace('\n', " ")
}

/// 探测系统浏览器：NW_BROWSER_PATH → 常见安装路径 → PATH。
fn find_browser() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("NW_BROWSER_PATH") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let mut cands: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        for env in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(env) {
                let base = PathBuf::from(base);
                cands.push(base.join("Microsoft/Edge/Application/msedge.exe"));
                cands.push(base.join("Google/Chrome/Application/chrome.exe"));
            }
        }
    }
    #[cfg(not(windows))]
    {
        for p in [
            "/usr/bin/google-chrome",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ] {
            cands.push(PathBuf::from(p));
        }
    }
    if let Some(p) = cands.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    // PATH 兜底：企业环境常用自定义安装目录。
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in ["msedge.exe", "chrome.exe", "google-chrome", "chromium"] {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn handle_event(method: &str, v: &Value, console: &Mutex<Vec<ConsoleLine>>) {
    let p = v.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "Runtime.consoleAPICalled" => {
            let level = p
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("log")
                .to_string();
            let text = p
                .get("args")
                .and_then(Value::as_array)
                .map(|args| {
                    args.iter()
                        .map(|a| {
                            a.get("value")
                                .map(|v| match v {
                                    Value::String(s) => s.clone(),
                                    other => other.to_string(),
                                })
                                .or_else(|| {
                                    a.get("description")
                                        .and_then(Value::as_str)
                                        .map(str::to_string)
                                })
                                .unwrap_or_default()
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            if !text.trim().is_empty() {
                push_line(console, level, clamp(&text));
            }
        }
        "Log.entryAdded" => {
            let e = &p["entry"];
            let level = e
                .get("level")
                .and_then(Value::as_str)
                .unwrap_or("info")
                .to_string();
            let mut text = e
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let Some(u) = e.get("url").and_then(Value::as_str) {
                if !u.is_empty() {
                    text = format!("{text} ({u})");
                }
            }
            if !text.trim().is_empty() {
                push_line(console, level, clamp(&text));
            }
        }
        "Network.responseReceived" => {
            let r = &p["response"];
            let status = r.get("status").and_then(Value::as_u64).unwrap_or(0);
            if status >= 400 {
                let url = r.get("url").and_then(Value::as_str).unwrap_or("");
                push_line(console, "http".into(), clamp(&format!("{status} {url}")));
            }
        }
        "Network.loadingFailed" => {
            let err = p
                .get("errorText")
                .and_then(Value::as_str)
                .unwrap_or("网络请求失败");
            push_line(console, "net".into(), clamp(err));
        }
        _ => {}
    }
}

fn push_line(console: &Mutex<Vec<ConsoleLine>>, level: String, text: String) {
    if let Ok(mut v) = console.lock() {
        if v.len() >= CONSOLE_CAP {
            v.remove(0);
        }
        v.push(ConsoleLine { level, text });
    }
}

fn clamp(s: &str) -> String {
    if s.chars().count() <= LINE_CLAMP {
        return s.to_string();
    }
    let mut t: String = s.chars().take(LINE_CLAMP).collect();
    t.push('…');
    t
}

/// 本进程的浏览器 profile（全进程固定一个）。
/// 重启/人工接管复用同一目录，cookie 与登录态不丢——「登录一次、后续步骤不再掉线」
/// 靠的就是它；`browser_close` 显式关闭时才删除。
pub(crate) fn session_profile() -> &'static std::path::Path {
    static PROFILE: OnceLock<PathBuf> = OnceLock::new();
    PROFILE
        .get_or_init(|| {
            sweep_stale_profiles();
            std::env::temp_dir().join(format!("neverwork-browser-{}", std::process::id()))
        })
        .as_path()
}

/// 清理一天前遗留的 profile（应用崩溃/强杀时 Drop 不跑，目录会留在 TEMP）。
fn sweep_stale_profiles() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(24 * 3600))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let own = format!("neverwork-browser-{}", std::process::id());
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("neverwork-browser-") || name == own {
            continue;
        }
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t < cutoff)
            .unwrap_or(false);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// 允许显式配置浏览器路径的测试钩子（集成测试使用）。
pub(crate) fn browser_available() -> bool {
    find_browser().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ws_url() {
        let (h, p, path) = parse_ws_url("ws://127.0.0.1:9222/devtools/page/ABC").unwrap();
        assert_eq!(h, "127.0.0.1");
        assert_eq!(p, 9222);
        assert_eq!(path, "/devtools/page/ABC");
        assert!(parse_ws_url("http://x").is_err());
    }

    #[test]
    fn js_string_is_escaped() {
        assert_eq!(js_str("a\"b"), "\"a\\\"b\"");
        assert_eq!(js_str("密码"), "\"密码\"");
    }

    #[test]
    fn session_profile_is_fixed_per_process() {
        let a = session_profile().to_path_buf();
        let b = session_profile().to_path_buf();
        assert_eq!(a, b, "同一进程必须复用同一个 profile（重启不丢登录态）");
        assert!(a.to_string_lossy().contains(&std::process::id().to_string()));
    }

    #[test]
    fn console_buffer_keeps_cap() {
        let c = std::sync::Mutex::new(Vec::new());
        for i in 0..(CONSOLE_CAP + 10) {
            push_line(&c, "log".into(), format!("{i}"));
        }
        let v = c.lock().unwrap();
        assert_eq!(v.len(), CONSOLE_CAP);
        assert_eq!(v.first().unwrap().text, "10");
    }
}
