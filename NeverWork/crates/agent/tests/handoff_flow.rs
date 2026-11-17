//! 团队角色交接的真实 HTTP 闭环：mock OpenAI 兼容服务端回 `handoff` 工具调用，
//! 验证请求体里带 handoff 工具（`to` 的 enum 是团队成员、系统提示含交接规则），
//! 以及事件流把合法交接折算成 `HandoffRequest`、把非法交接折回工具错误后继续本轮。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use neverwork_agent::{run_agent_in_team, AgentEvent, ProviderConfig, RunLimits};
use neverwork_core::{FlowMode, Role, Team};
use serde_json::{json, Value};

fn team() -> Team {
    Team {
        id: "t".into(),
        name: "交付组".into(),
        flow: FlowMode::Pipeline,
        roles: vec![
            Role {
                id: "r0".into(),
                name: "需求验证".into(),
                kind_label: "评审".into(),
                model: "gpt-4o".into(),
                prompt: "验证需求是否可实现".into(),
                color_index: 0,
                initial: "验".into(),
                skills: vec![],
            },
            Role {
                id: "r1".into(),
                name: "接口成员".into(),
                kind_label: "开发".into(),
                model: "gpt-4o".into(),
                prompt: "按需求修改接口".into(),
                color_index: 1,
                initial: "接".into(),
                skills: vec![],
            },
        ],
    }
}

fn cfg(port: u16) -> ProviderConfig {
    ProviderConfig {
        api_key: "k".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        model: "gpt-4o".into(),
        temperature: 0.2,
        thinking: neverwork_core::ThinkingLevel::Auto,
    }
}

/// 三项全 0 = 不限，与默认设置一致。
fn limits() -> RunLimits {
    RunLimits {
        tokens: 0,
        rounds: 0,
        seconds: 0,
    }
}

/// 把若干 JSON 帧拼成一段完整 SSE 正文。
fn sse(frames: &[Value]) -> String {
    let mut s = String::new();
    for f in frames {
        s.push_str("data: ");
        s.push_str(&f.to_string());
        s.push_str("\n\n");
    }
    s.push_str("data: [DONE]\n\n");
    s
}

fn text_reply(text: &str) -> String {
    sse(&[
        json!({"choices":[{"delta":{"content":text}}]}),
        json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
    ])
}

fn handoff_reply(call_id: &str, args: &Value) -> String {
    sse(&[
        json!({"choices":[{"delta":{"tool_calls":[{
            "index": 0,
            "id": call_id,
            "function": {"name": "handoff", "arguments": args.to_string()}
        }]}}]}),
        json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
    ])
}

/// 把一次 HTTP 请求读完整。
///
/// 本地回环下 header 与 body 可能分两段到达，只 `read` 一次常常只拿到 header，
/// 断言就会看起来像「工具/提示没发出去」。
fn read_full_request(stream: &mut std::net::TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
    let mut raw: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        raw.extend_from_slice(&buf[..n]);
        if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&raw[..pos]).to_lowercase();
            let want = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if raw.len() >= pos + 4 + want {
                break;
            }
        }
    }
    String::from_utf8_lossy(&raw).to_string()
}

/// 按顺序回放 `replies`（每个元素是一段完整 SSE 正文），
/// 并把每次的原始请求回传给测试。
fn spawn_server(
    replies: Vec<String>,
) -> (
    u16,
    std::sync::mpsc::Receiver<String>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        for body in replies {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = tx.send(read_full_request(&mut stream));
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
            // 写完半关闭并读走残余请求体，否则 Windows 发 RST，
            // 客户端偶发 "os error 10053"，第二轮直接失败。
            let _ = stream.shutdown(std::net::Shutdown::Write);
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
            let mut sink = [0u8; 1024];
            while let Ok(n) = stream.read(&mut sink) {
                if n == 0 {
                    break;
                }
            }
        }
    });
    (port, rx, handle)
}

/// 从原始请求里切出 body 并解析成 JSON。
fn body_of(raw: &str) -> Value {
    let pos = raw
        .find("\r\n\r\n")
        .expect("请求里没有 header/body 分隔");
    serde_json::from_str(&raw[pos + 4..]).expect("请求体不是合法 JSON")
}

fn run_turn(
    port: u16,
    role_idx: usize,
    user_text: &str,
    emit: &mut dyn FnMut(AgentEvent),
) {
    run_agent_in_team(
        "openai",
        cfg(port),
        vec![],
        user_text,
        neverwork_agent::tools::ToolContext::new(std::env::temp_dir()),
        true,
        limits(),
        &team(),
        role_idx,
        emit,
        Arc::new(AtomicBool::new(false)),
    );
}

#[test]
fn handoff_round_offers_the_tool_and_emits_the_request() {
    let payload = json!({
        "to": "接口成员",
        "verdict": "needs_change",
        "summary": "登录接口没有判空",
        "changes": [{
            "file": "src/api.rs",
            "location": "login",
            "issue": "未判空",
            "required": "补 if null 返回 400"
        }],
        "acceptance": "null 入参返回 400"
    });
    let (port, rx, server) = spawn_server(vec![handoff_reply("call_h1", &payload)]);

    let mut handoff: Option<(String, neverwork_agent::HandoffPayload)> = None;
    let mut handoff_ok = false;
    let mut approvals = 0usize;
    let mut saw_done = false;
    run_turn(port, 0, "验证一下登录需求", &mut |ev| match ev {
        AgentEvent::HandoffRequest { from, payload } => handoff = Some((from, payload)),
        AgentEvent::ToolFinished { result } if result.name == "handoff" => {
            handoff_ok = result.ok;
        }
        AgentEvent::NeedApproval { call, .. } => {
            approvals += 1;
            eprintln!("handoff 不该要审批：{}", call.name);
        }
        AgentEvent::Done => saw_done = true,
        AgentEvent::Error(e) => panic!("不该有错误：{e}"),
        _ => {}
    });
    server.join().unwrap();

    let req = body_of(&rx.recv().unwrap());
    let tools = req["tools"].as_array().expect("请求没带 tools");
    let spec = tools
        .iter()
        .find(|t| t["function"]["name"] == "handoff")
        .expect("tools 里没有 handoff");
    let names: Vec<&str> = spec["function"]["parameters"]["properties"]["to"]["enum"]
        .as_array()
        .expect("to 没有 enum")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["需求验证", "接口成员"],
        "to 的 enum 必须列出全部成员，模型才知道能交给谁"
    );
    let system = req["messages"][0]["content"].as_str().unwrap_or_default();
    assert!(
        system.contains("团队协作") && system.contains("需求验证") && system.contains("交接规则"),
        "系统提示缺团队名单或交接规则：{system}"
    );

    assert_eq!(approvals, 0, "handoff 是只读动作，不该弹审批");
    let (from, p) = handoff.expect("合法交接必须发 HandoffRequest");
    assert_eq!(from, "需求验证");
    assert_eq!(p.to, "接口成员");
    assert_eq!(p.verdict, "needs_change");
    assert_eq!(p.changes.len(), 1);
    assert!(handoff_ok, "handoff 工具结果应记为成功");
    assert!(saw_done, "交接后本轮要以 Done 收尾");
}

#[test]
fn self_handoff_is_returned_as_error_and_the_round_continues() {
    // 第一轮：把任务交给自己 -> 必须被拒；第二轮：模型收下错误、改口自己处理。
    let bad = json!({"to": "需求验证", "verdict": "pass", "summary": "交给自己"});
    let (port, rx, server) = spawn_server(vec![
        handoff_reply("call_h1", &bad),
        text_reply("明白了，我自己处理。"),
    ]);

    let mut handoff_seen = false;
    let mut err_text = String::new();
    let mut text = String::new();
    let mut saw_done = false;
    run_turn(port, 0, "验证一下登录需求", &mut |ev| match ev {
        AgentEvent::HandoffRequest { .. } => handoff_seen = true,
        AgentEvent::ToolFinished { result } if !result.ok => err_text = result.content.clone(),
        AgentEvent::TextDelta(d) => text.push_str(&d),
        AgentEvent::Done => saw_done = true,
        AgentEvent::Error(e) => panic!("不该有传输错误：{e}"),
        _ => {}
    });
    server.join().unwrap();

    let _first = rx.recv().unwrap();
    let second = body_of(&rx.recv().unwrap());

    assert!(!handoff_seen, "自交接绝不能触发交接，否则会把任务原地打转");
    assert!(
        err_text.contains("不能把任务交接给自己"),
        "模型要收到结构化错误才能改口：{err_text}"
    );
    assert!(text.contains("我自己处理"), "被拒后应继续本轮：{text}");
    assert!(saw_done);

    // 第二轮请求必须带上配对的工具结果：缺了它 OpenAI 协议会整轮 400，
    // 模型也无从知道自己错在哪。
    let msgs = second["messages"].as_array().expect("第二轮没有 messages");
    let tool_msg = msgs
        .iter()
        .find(|m| m["role"] == "tool")
        .expect("第二轮没带工具结果");
    assert_eq!(tool_msg["tool_call_id"], "call_h1");
    assert!(
        tool_msg["content"]
            .as_str()
            .unwrap_or_default()
            .contains("交接给自己"),
        "工具结果里没有校验错误：{tool_msg}"
    );
}
