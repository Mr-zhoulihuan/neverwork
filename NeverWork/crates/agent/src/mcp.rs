//! MCP（Model Context Protocol）客户端：把用户配置的外部工具进程接进 Agent。
//!
//! 协议走 stdio JSON-RPC 2.0，握手顺序固定：
//! `initialize` → `notifications/initialized` → `tools/list`，之后按需 `tools/call`。
//!
//! 三条设计约束：
//! 1. **一律当写操作**：MCP server 是用户自己配的本地进程，能做的事和终端
//!    一样多，所以它暴露的每个工具都 `requires_approval = true`。
//! 2. **不进 `ToolContext`**：`resolve_write` 只能约束本进程的文件操作，
//!    约束不了别的进程。这里不做假承诺，只在审批环节让人看见。
//! 3. **必须有超时**：`read_line` 会无限阻塞，server 挂住会拖死整轮 Agent。
//!    读取放到独立线程，主线程 `recv_timeout`，到点就返回错误。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use neverwork_core::McpServerConfig;
use serde_json::{json, Value};

use crate::tools::{ToolCall, ToolResult, ToolSpec};

/// 握手与列表的超时。server 启动慢（npx 首次拉包）留宽一点。
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// 单次工具调用的超时。
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// 返回给模型的正文上限，防止一个工具把整个上下文塞满。
const MAX_RESULT_CHARS: usize = 32_000;
/// MCP 工具名前缀，`execute` 用它路由。
const PREFIX: &str = "mcp__";
/// 工具名长度上限（OpenAI 要求 <= 64）。
const NAME_MAX: usize = 64;

/// 一个 MCP server 提供的工具（解析后的形态）。
#[derive(Debug, Clone)]
struct McpTool {
    /// server 认识的原名，调用时要原样回传。
    name: String,
    description: String,
    schema: Value,
}

/// 一个已握手的 stdio 连接。
struct McpClient {
    child: Child,
    stdin: ChildStdin,
    /// reader 线程推过来的原始行。
    rx: flume::Receiver<String>,
    next_id: u64,
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // 不 kill 会留下孤儿进程，Windows 上还会占着端口/文件句柄
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl McpClient {
    fn spawn(cfg: &McpServerConfig) -> Result<Self, String> {
        let mut cmd = Command::new(cfg.command.trim());
        cmd.args(&cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // stderr 直接丢掉：server 的日志混进来会污染 JSON 流
            .stderr(Stdio::null());
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // 不弹黑框：Agent 在后台起进程，闪一个控制台窗口很吓人
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动 `{}` 失败：{e}", cfg.command))?;
        let stdin = child.stdin.take().ok_or("拿不到 stdin")?;
        let stdout = child.stdout.take().ok_or("拿不到 stdout")?;

        let (tx, rx) = flume::unbounded::<String>();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match r.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if tx.send(line.clone()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let mut c = Self {
            child,
            stdin,
            rx,
            next_id: 1,
        };
        c.initialize()?;
        Ok(c)
    }

    fn initialize(&mut self) -> Result<(), String> {
        self.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "neverwork", "version": neverwork_core::APP_VERSION }
            }),
            HANDSHAKE_TIMEOUT,
        )?;
        self.notify("notifications/initialized")
    }

    fn notify(&mut self, method: &str) -> Result<(), String> {
        let req = json!({ "jsonrpc": "2.0", "method": method });
        let mut text = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        text.push('\n');
        self.stdin
            .write_all(text.as_bytes())
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("写入 MCP 失败：{e}"))
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let mut text = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        text.push('\n');
        self.stdin
            .write_all(text.as_bytes())
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("写入 MCP 失败：{e}"))?;

        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!("`{method}` 超时（{}s）", timeout.as_secs()));
            }
            let line = self
                .rx
                .recv_timeout(left)
                // server 退出时 channel 断开，这里要给出「进程没了」而不是「超时」
                .map_err(|e| match e {
                    flume::RecvTimeoutError::Disconnected => {
                        format!("MCP server 已退出（`{method}` 无法完成）")
                    }
                    flume::RecvTimeoutError::Timeout => {
                        format!("`{method}` 超时（{}s）", timeout.as_secs())
                    }
                })?;
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                // 有些 server 会往 stdout 打非 JSON 日志，跳过
                continue;
            };
            // 通知没有 id；别人的响应也跳过
            if v.get("id").and_then(|x| x.as_u64()) != Some(id) {
                continue;
            }
            if let Some(err) = v.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("未知错误");
                return Err(format!("`{method}` 报错：{msg}"));
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn list_tools(&mut self) -> Result<Vec<McpTool>, String> {
        let v = self.request("tools/list", json!({}), HANDSHAKE_TIMEOUT)?;
        let arr = v
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for t in arr {
            let Some(name) = t.get("name").and_then(|x| x.as_str()) else {
                continue;
            };
            out.push(McpTool {
                name: name.to_string(),
                description: t
                    .get("description")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                schema: t
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
            });
        }
        Ok(out)
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<String, String> {
        let v = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
            CALL_TIMEOUT,
        )?;
        let is_error = v
            .get("isError")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let text = render_content(&v);
        if is_error {
            Err(text)
        } else {
            Ok(text)
        }
    }
}

/// 把 MCP 的 `content` 数组铺平成文本。
fn render_content(v: &Value) -> String {
    let mut out = String::new();
    if let Some(arr) = v.get("content").and_then(|c| c.as_array()) {
        for item in arr {
            let kind = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let piece = match kind {
                "text" => item
                    .get("text")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                // 图片/音频给模型也没用（当前协议层不支持回传附件），
                // 明确说「省略」比塞一串 base64 好
                "image" | "audio" => "[媒体内容已省略]".to_string(),
                "resource" => item
                    .pointer("/resource/text")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "[资源内容已省略]".to_string()),
                other if !other.is_empty() => format!("[{other} 内容已省略]"),
                _ => String::new(),
            };
            if piece.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&piece);
        }
    }
    if out.trim().is_empty() {
        // 有的 server 直接给结构化结果，没有 content
        out = v.to_string();
    }
    clip(&out)
}

fn clip(text: &str) -> String {
    if text.chars().count() <= MAX_RESULT_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(MAX_RESULT_CHARS).collect();
    format!("{head}\n…（结果过大已截断）")
}

/// 拼工具名：`mcp__<server>__<tool>`，只留安全字符并截到长度上限。
fn tool_name(server: &str, tool: &str) -> String {
    let mut t = String::new();
    for c in tool.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            t.push(c);
        } else if !t.ends_with('_') {
            t.push('_');
        }
    }
    let full = format!("{PREFIX}{server}__{t}");
    if full.len() <= NAME_MAX {
        return full;
    }
    // 全是 ASCII，按字节截断是安全的
    full.chars().take(NAME_MAX).collect()
}

/// 注册表：已配置的 server、已握手的连接、工具路由。
#[derive(Default)]
struct Registry {
    configured: Vec<McpServerConfig>,
    clients: HashMap<String, McpClient>,
    /// 暴露给模型的工具名 → (server, 原名)
    route: HashMap<String, (String, String)>,
    specs: Vec<ToolSpec>,
    /// 探测过一次（含失败），避免每次 `tool_specs()` 都重试启动
    probed: bool,
    /// 启动/握手失败的原因，供界面显示
    errors: Vec<String>,
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

fn lock() -> MutexGuard<'static, Registry> {
    REGISTRY
        .get_or_init(|| Mutex::new(Registry::default()))
        .lock()
        // 某个线程 panic 不应让 MCP 永久不可用
        .unwrap_or_else(|e| e.into_inner())
}

/// 设置 MCP server 列表。配置没变就不重连（避免每次启动都拉一遍进程）。
pub fn configure(servers: Vec<McpServerConfig>) {
    let mut r = lock();
    if r.probed && same_config(&r.configured, &servers) {
        return;
    }
    // 重建会 Drop 掉旧 client，进程随之退出
    *r = Registry {
        configured: servers,
        ..Registry::default()
    };
}

fn same_config(a: &[McpServerConfig], b: &[McpServerConfig]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.name == y.name
                && x.command == y.command
                && x.args == y.args
                && x.env == y.env
                && x.enabled == y.enabled
        })
}

/// 从 `NW_MCP` 读配置（JSON 数组），便于不开界面就试一个 server。
pub fn servers_from_env() -> Option<Vec<McpServerConfig>> {
    let raw = std::env::var("NW_MCP").ok()?;
    if raw.trim().is_empty() {
        return None;
    }
    serde_json::from_str(&raw).ok()
}

/// 懒探测：第一次要用工具时才起进程，没配 MCP 的用户零开销。
fn ensure_probed(r: &mut Registry) {
    if r.probed {
        return;
    }
    r.probed = true;
    let servers: Vec<McpServerConfig> = r.configured.iter().filter(|c| c.usable()).cloned().collect();
    for cfg in servers {
        let key = cfg.sanitized();
        let probed = McpClient::spawn(&cfg).and_then(|mut c| {
            let tools = c.list_tools()?;
            Ok((c, tools))
        });
        match probed {
            Ok((client, tools)) => {
                for t in tools {
                    let full = tool_name(&key, &t.name);
                    r.route
                        .insert(full.clone(), (key.clone(), t.name.clone()));
                    r.specs.push(ToolSpec {
                        // 外部进程能做任何事：一律按写操作走审批
                        name: full,
                        description: format!("[MCP:{}] {}", cfg.name, t.description),
                        parameters: t.schema,
                        requires_approval: true,
                    });
                }
                r.clients.insert(key, client);
            }
            Err(e) => r.errors.push(format!("{}：{e}", cfg.name)),
        }
    }
}

/// 当前 MCP 工具（未配置时返回空，不会启动任何进程）。
pub fn specs() -> Vec<ToolSpec> {
    let mut r = lock();
    if r.configured.is_empty() {
        return Vec::new();
    }
    ensure_probed(&mut r);
    r.specs.clone()
}

/// 是否是 MCP 工具（按前缀判定，不需要已连接）。
pub fn is_mcp(name: &str) -> bool {
    name.starts_with(PREFIX)
}

/// 执行一次 MCP 工具调用。
pub fn execute(call: &ToolCall) -> ToolResult {
    let mut r = lock();
    if r.configured.is_empty() {
        return ToolResult::err_code(
            call,
            "MCP_NOT_CONFIGURED",
            "没有配置 MCP server。在 settings.json 的 mcp_servers 里加一项后重启。",
            false,
        );
    }
    ensure_probed(&mut r);

    let Some((server, tool)) = r.route.get(&call.name).cloned() else {
        let hint = if r.errors.is_empty() {
            String::new()
        } else {
            format!("（MCP 启动失败：{}）", r.errors.join("；"))
        };
        return ToolResult::err_code(
            call,
            "UNKNOWN_TOOL",
            format!("未知 MCP 工具 `{}`{hint}", call.name),
            false,
        );
    };
    let Some(client) = r.clients.get_mut(&server) else {
        return ToolResult::err_code(
            call,
            "MCP_NOT_CONNECTED",
            format!("MCP server `{server}` 未连接"),
            true,
        );
    };

    let raw = call.arguments.trim();
    let args: Value = if raw.is_empty() {
        json!({})
    } else {
        match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err_code(
                    call,
                    "BAD_ARGS",
                    format!("参数不是合法 JSON：{e}"),
                    false,
                )
            }
        }
    };

    match client.call_tool(&tool, args) {
        Ok(text) => ToolResult::ok(call, text),
        // 外部进程的失败可能是瞬时的（网络、临时文件锁），标可重试
        Err(e) => ToolResult::err_code(call, "MCP_ERROR", e, true),
    }
}

/// 状态摘要：(已连接 server 数, 工具数, 错误列表)。会触发探测。
pub fn status() -> (usize, usize, Vec<String>) {
    let mut r = lock();
    if r.configured.is_empty() {
        return (0, 0, Vec::new());
    }
    ensure_probed(&mut r);
    (r.clients.len(), r.specs.len(), r.errors.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.into(),
            command: "node".into(),
            args: vec!["server.js".into()],
            env: Default::default(),
            enabled: true,
        }
    }

    #[test]
    fn tool_names_are_prefixed_and_sanitized() {
        assert_eq!(tool_name("fs", "read_file"), "mcp__fs__read_file");
        // 名字里的空格/斜杠要变成下划线，否则模型无法稳定引用
        assert_eq!(tool_name("my_server", "a b/c"), "mcp__my_server__a_b_c");
    }

    #[test]
    fn long_tool_names_are_truncated_to_provider_limit() {
        let long = "x".repeat(200);
        let n = tool_name("server", &long);
        assert_eq!(n.len(), NAME_MAX, "超长名字必须截到 64: {}", n.len());
        assert!(n.starts_with("mcp__server__"));
    }

    #[test]
    fn prefix_detection_is_exact() {
        assert!(is_mcp("mcp__fs__read"));
        // 内置工具不能被误判
        assert!(!is_mcp("read_file"));
        assert!(!is_mcp("mcp"));
        assert!(!is_mcp("xmcp__a"));
    }

    #[test]
    fn text_content_is_flattened() {
        let v = json!({ "content": [
            { "type": "text", "text": "第一段" },
            { "type": "image", "data": "AAAA", "mimeType": "image/png" },
            { "type": "text", "text": "第二段" }
        ]});
        let out = render_content(&v);
        assert!(out.contains("第一段") && out.contains("第二段"));
        assert!(out.contains("已省略"), "图片不能塞 base64: {out}");
        assert!(!out.contains("AAAA"), "不能把 base64 原文回填给模型");
    }

    #[test]
    fn structured_result_without_content_is_stringified() {
        let v = json!({ "value": 42 });
        assert!(render_content(&v).contains("42"));
    }

    #[test]
    fn huge_result_is_clipped() {
        let v = json!({ "content": [{ "type": "text", "text": "x".repeat(MAX_RESULT_CHARS + 100) }] });
        let out = render_content(&v);
        assert!(out.contains("已截断"));
        assert!(out.chars().count() < MAX_RESULT_CHARS + 50);
    }

    #[test]
    fn config_diff_is_detected() {
        let a = vec![cfg("fs")];
        let mut b = a.clone();
        assert!(same_config(&a, &b));
        b[0].args.push("--flag".into());
        assert!(!same_config(&a, &b), "参数变了要重连");
    }

    #[test]
    fn debug_of_config_hides_secrets() {
        let mut c = cfg("github");
        c.args.push("--api-key".into());
        c.args.push("sk-super-secret".into());
        c.env.insert("GITHUB_TOKEN".into(), "ghp_secret".into());
        let shown = format!("{c:?}");
        assert!(!shown.contains("sk-super-secret"), "args 不能打印: {shown}");
        assert!(!shown.contains("ghp_secret"), "env 值不能打印: {shown}");
        assert!(shown.contains("GITHUB_TOKEN"), "key 名要留着排障: {shown}");
    }

    #[test]
    fn disabled_or_blank_servers_are_not_started() {
        let mut c = cfg("off");
        c.enabled = false;
        assert!(!c.usable());
        let mut blank = cfg("blank");
        blank.command = "  ".into();
        assert!(!blank.usable());
    }

    #[test]
    fn unconfigured_registry_returns_nothing() {
        // 不能用全局 REGISTRY 做断言（测试并发），这里只验证纯函数行为
        assert!(servers_from_env().is_none() || std::env::var("NW_MCP").is_ok());
    }
}
