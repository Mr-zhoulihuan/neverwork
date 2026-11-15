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
