//! MCP 端到端：真的起一个 stdio server 走完整 JSON-RPC 往返。
//!
//! 用一个内联的 Node 脚本当 server（`node` 已是本机依赖），验证：
//! 握手 → tools/list → 注入工具清单 → 审批策略 → tools/call → 错误传播。
//!
//! 注册表是全局的，所以本文件所有用例用一把静态锁串行跑。

use std::path::Path;
use std::sync::Mutex;

use neverwork_agent::mcp;
use neverwork_agent::tools::{self, ToolCall, ToolContext};
use neverwork_core::McpServerConfig;

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// 一个最小的 MCP server：只认 initialize / tools/list / tools/call。
const SERVER_JS: &str = r#"
const readline = require('readline');
const rl = readline.createInterface({ input: process.stdin });
const send = (o) => process.stdout.write(JSON.stringify(o) + '\n');
rl.on('line', (line) => {
  let m;
  try { m = JSON.parse(line); } catch (e) { return; }
  if (m.method === 'initialize') {
    send({ jsonrpc: '2.0', id: m.id, result: {
      protocolVersion: '2024-11-05',
      capabilities: { tools: {} },
      serverInfo: { name: 'fake', version: '1' }
    }});
  } else if (m.method === 'notifications/initialized') {
    // 通知没有响应
  } else if (m.method === 'tools/list') {
    send({ jsonrpc: '2.0', id: m.id, result: { tools: [
      { name: 'echo', description: '原样返回',
        inputSchema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'] } },
      { name: 'boom', description: '总是失败',
        inputSchema: { type: 'object', properties: {} } }
    ]}});
  } else if (m.method === 'tools/call') {
    const p = m.params || {};
    if (p.name === 'boom') {
      send({ jsonrpc: '2.0', id: m.id, result: {
        isError: true, content: [{ type: 'text', text: '故意失败' }]
      }});
    } else {
      const text = (p.arguments || {}).text || '';
      send({ jsonrpc: '2.0', id: m.id, result: {
        content: [{ type: 'text', text: 'echo:' + text }]
      }});
    }
  } else {
    send({ jsonrpc: '2.0', id: m.id, error: { code: -32601, message: 'method not found' } });
  }
});
"#;

fn ws(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("nw-mcp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fake_server(dir: &Path) -> McpServerConfig {
    let js = dir.join("server.js");
    std::fs::write(&js, SERVER_JS).unwrap();
    McpServerConfig {
        name: "fake".into(),
        command: "node".into(),
        args: vec![js.to_string_lossy().to_string()],
        env: Default::default(),
        enabled: true,
    }
}

fn call(name: &str, args: &str) -> ToolCall {
    ToolCall {
        id: "m1".into(),
        name: name.into(),
        arguments: args.into(),
    }
}

#[test]
fn stdio_roundtrip_injects_tools_and_routes_calls() {
    let _g = serial();
    let dir = ws("e2e");
    mcp::configure(vec![fake_server(&dir)]);

    // ---- 工具清单：握手 + tools/list 后应该注入两个 ----
    let specs = mcp::specs();
    let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"mcp__fake__echo"),
        "应注入 echo：{names:?}"
    );
    assert!(names.contains(&"mcp__fake__boom"), "应注入 boom：{names:?}");
    assert!(
        specs.iter().all(|s| s.requires_approval),
        "MCP 工具一律要审批"
    );
    assert!(
        specs.iter().any(|s| s.description.contains("[MCP:fake]")),
        "描述要能看出工具来自哪台 server"
    );

    // 合进总清单，模型才看得到
    let all = tools::tool_specs();
    assert!(
        all.iter().any(|t| t.name == "mcp__fake__echo"),
        "tool_specs() 要合并 MCP"
    );
    assert!(
        tools::requires_approval("mcp__fake__echo"),
        "外部进程能做任何事，必须审批"
    );

    // ---- 调用成功路径 ----
    let ctx = ToolContext::new(&dir);
    let r = tools::execute(&call("mcp__fake__echo", r#"{"text":"你好"}"#), &ctx);
    assert!(r.ok, "{}", r.content);
    assert_eq!(r.content, "echo:你好");

    // ---- 调用失败路径：isError 要变成错误码 + 可重试 ----
    let r = tools::execute(&call("mcp__fake__boom", "{}"), &ctx);
    assert!(!r.ok);
    assert_eq!(r.error_code, Some("MCP_ERROR"));
    assert!(r.content.contains("故意失败"), "{}", r.content);

    // ---- 未知 MCP 工具 ----
    let r = tools::execute(&call("mcp__fake__nope", "{}"), &ctx);
    assert_eq!(r.error_code, Some("UNKNOWN_TOOL"), "{}", r.content);

    // ---- 状态摘要 ----
    let (servers, tools_n, errors) = mcp::status();
    assert_eq!(servers, 1, "{errors:?}");
    assert_eq!(tools_n, 2, "{errors:?}");
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn unconfigured_mcp_returns_clear_error() {
    let _g = serial();
    mcp::configure(Vec::new());
    assert!(mcp::specs().is_empty(), "没配置就不该有工具");
    let dir = ws("none");
    let ctx = ToolContext::new(&dir);
    let r = tools::execute(&call("mcp__x__y", "{}"), &ctx);
    assert_eq!(r.error_code, Some("MCP_NOT_CONFIGURED"), "{}", r.content);
    assert!(!r.ok);
}

#[test]
fn dead_server_is_reported_not_panicked() {
    let _g = serial();
    let dir = ws("dead");
    // 起来就退出：握手必然失败
    mcp::configure(vec![McpServerConfig {
        name: "dead".into(),
        command: "node".into(),
        args: vec!["-e".into(), "process.exit(0)".into()],
        env: Default::default(),
        enabled: true,
    }]);

    let specs = mcp::specs();
    assert!(specs.is_empty(), "起不来的 server 不该有工具：{specs:?}");
    let (servers, tool_n, errors) = mcp::status();
    assert_eq!(servers, 0);
    assert_eq!(tool_n, 0);
    assert!(!errors.is_empty(), "要把失败原因记下来");
    assert!(
        errors[0].contains("dead"),
        "错误里要点名是哪台 server：{errors:?}"
    );

    // 调它的工具：明确报未知/失败，而不是挂住
    let ctx = ToolContext::new(&dir);
    let r = tools::execute(&call("mcp__dead__anything", "{}"), &ctx);
    assert!(!r.ok);
    assert!(
        matches!(r.error_code, Some("UNKNOWN_TOOL") | Some("MCP_NOT_CONNECTED")),
        "{:?}",
        r.error_code
    );
}

#[test]
fn disabled_server_is_never_started() {
    let _g = serial();
    let dir = ws("disabled");
    let mut c = fake_server(&dir);
    c.enabled = false;
    mcp::configure(vec![c]);
    assert!(mcp::specs().is_empty());
    let (servers, _, errors) = mcp::status();
    assert_eq!(servers, 0);
    assert!(errors.is_empty(), "禁用的不该报错：{errors:?}");
}
