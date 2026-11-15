use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use neverwork_agent::{run_team, ProviderConfig, TeamEvent};
use neverwork_core::{FlowMode, Role, Team};

/// 启动一个 mock OpenAI 服务，对每个请求返回固定 SSE 文本，读取一个请求后退出。
fn spawn_mock_server(reply: &'static str, requests: usize) -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        for _ in 0..requests {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let body = format!(
                "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{reply}\"}}}}]}}\n\ndata: [DONE]\n\n"
            );
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
            // 写完半关闭并读走残余请求体，否则 Windows 发 RST，
            // 客户端偶发 "os error 10053"，第二个角色的 Delta 就丢了。
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
    (port, handle)
}

fn team_with_roles(flow: FlowMode, names: &[(&str, &str)]) -> Team {
    Team {
        id: "t".into(),
        name: "test".into(),
        flow,
        roles: names
            .iter()
            .enumerate()
            .map(|(i, (name, initial))| Role {
                id: format!("r{i}"),
                name: name.to_string(),
                kind_label: "".into(),
                // 用 gpt 系列避免切换 anthropic
                model: "gpt-4o".into(),
                prompt: String::new(),
                color_index: i,
                initial: initial.to_string(),
                skills: vec![],
            })
            .collect(),
    }
}

#[test]
fn pipeline_runs_roles_in_order() {
    // 两个角色 -> 两个请求。
    let (port, server) = spawn_mock_server("OK", 2);
    let team = team_with_roles(FlowMode::Pipeline, &[("P", "P"), ("C", "C")]);
    let cfg = ProviderConfig {
        api_key: "k".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        model: "gpt-4o".into(),
        temperature: 0.7,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let mut starts = Vec::new();
    let mut deltas: Vec<(usize, String)> = Vec::new();
    let mut dones = Vec::new();
    run_team(&team, "hi", cfg, &mut |ev| match ev {
        TeamEvent::RoleStart { role_index } => starts.push(role_index),
        TeamEvent::Delta { role_index, text } => deltas.push((role_index, text)),
        TeamEvent::RoleDone { role_index } => dones.push(role_index),
        _ => {}
    }, cancel);
    server.join().unwrap();

    assert_eq!(starts, vec![0, 1]);
    assert_eq!(dones, vec![0, 1]);
    assert_eq!(deltas.len(), 2);
    assert_eq!(deltas[0].0, 0);
    assert_eq!(deltas[1].0, 1);
}

#[test]
fn pipeline_cancel_stops_early() {
    let (port, server) = spawn_mock_server("OK", 1);
    // 预置取消=true，应立即停止，不发起请求；服务端等待 1 个请求会卡住，所以这里不 join。
    let team = team_with_roles(FlowMode::Pipeline, &[("P", "P"), ("C", "C")]);
    let cfg = ProviderConfig {
        api_key: "k".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        model: "gpt-4o".into(),
        temperature: 0.7,
    };
    let cancel = Arc::new(AtomicBool::new(true));
    let mut starts = 0;
    run_team(&team, "hi", cfg, &mut |ev| {
        if matches!(ev, TeamEvent::RoleStart { .. }) {
            starts += 1;
        }
    }, cancel);
    drop(server); // 不 join，服务端线程会被进程退出回收
    assert_eq!(starts, 0, "cancel=true 时不应启动任何角色");
}

/// 把一次 HTTP 请求读完整。
///
/// 本地回环下 header 与 body 可能分两段到达，只 `read` 一次常常只拿到 header，
/// 断言就会看起来像「历史没发出去」。
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

/// 收到请求后把请求体回传给测试，再回一条最小流式响应。
fn spawn_capturing_server() -> (
    u16,
    std::sync::mpsc::Receiver<String>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = tx.send(read_full_request(&mut stream));
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"OK\"}}]}\n\ndata: [DONE]\n\n";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.flush();
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
        let mut sink = [0u8; 1024];
        while let Ok(n) = stream.read(&mut sink) {
            if n == 0 {
                break;
            }
        }
    });
    (port, rx, handle)
}

#[test]
fn session_history_reaches_the_model() {
    let (port, rx, server) = spawn_capturing_server();
    let team = team_with_roles(FlowMode::Pipeline, &[("P", "P")]);
    let cfg = ProviderConfig {
        api_key: "k".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        model: "gpt-4o".into(),
        temperature: 0.7,
    };
    let history = vec![
        neverwork_agent::ChatMessage::user("上一轮的问题：读一下 a.rs"),
        neverwork_agent::ChatMessage::assistant("上一轮的回答：a.rs 导出 foo"),
    ];
    let cancel = Arc::new(AtomicBool::new(false));
    neverwork_agent::run_team_with_history(&team, history, "那 bar 呢？", cfg, &mut |_| {}, cancel);
    server.join().unwrap();

    let req = rx.recv().unwrap();
    assert!(
        req.contains("上一轮的问题：读一下 a.rs"),
        "请求里没带上会话历史：{req}"
    );
    assert!(
        req.contains("上一轮的回答：a.rs 导出 foo"),
        "请求里没带上上一轮回答：{req}"
    );
    assert!(req.contains("那 bar 呢？"), "请求里没带本次提问：{req}");
}

#[test]
fn plain_team_flattens_tool_history() {
    // 团队纯文本路径不传 tools：历史里的 tool_calls / tool 对必须压成正文。
    // 真实踩过：从单 Agent 切到团队（关工具）后，Anthropic 会因
    // 空正文的 assistant 整轮 400。
    let (port, rx, server) = spawn_capturing_server();
    let team = team_with_roles(FlowMode::Pipeline, &[("P", "P")]);
    let cfg = ProviderConfig {
        api_key: "k".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        model: "gpt-4o".into(),
        temperature: 0.7,
    };
    let history = vec![
        neverwork_agent::ChatMessage::user("读一下 a.rs"),
        neverwork_agent::ChatMessage::assistant_with_tools(
            "",
            vec![neverwork_agent::ToolCall {
                id: "call_1".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"a.rs"}"#.into(),
            }],
        ),
        neverwork_agent::ChatMessage::tool_result("call_1", "FILE_BODY_SURVIVES"),
    ];
    let cancel = Arc::new(AtomicBool::new(false));
    neverwork_agent::run_team_with_history(&team, history, "那 b.rs 呢", cfg, &mut |_| {}, cancel);
    server.join().unwrap();

    let req = rx.recv().unwrap();
    // 协议字段一个都不能留
    assert!(!req.contains("\"tool_calls\""), "还在发 tool_calls：{req}");
    assert!(!req.contains("\"tool_call_id\""), "还在发 tool_call_id：{req}");
    assert!(!req.contains("\"role\":\"tool\""), "还在发 tool 角色：{req}");
    assert!(!req.contains("\"content\":null"), "还有空正文：{req}");
    // 内容不能丢：模型仍要知道上一轮读过什么
    assert!(req.contains("FILE_BODY_SURVIVES"), "工具输出被丢了：{req}");
    assert!(req.contains("read_file"), "工具名被丢了：{req}");
}
