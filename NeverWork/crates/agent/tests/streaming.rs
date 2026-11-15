use std::io::{Read, Write};
use std::net::TcpListener;

use neverwork_agent::{build_provider, ChatMessage, ProviderConfig, StreamEvent};

#[test]
fn openai_streaming_end_to_end() {
    // 启动一个极简 HTTP 服务，返回 OpenAI 流式 SSE。
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        // 读取请求头（忽略内容）。
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"！\"}}]}\n\ndata: [DONE]\n\n";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.flush();
    });

    let config = ProviderConfig {
        api_key: "test-key".into(),
        base_url: format!("http://{addr}"),
        model: "test-model".into(),
    };
    let provider = build_provider("openai", config);
    let messages = vec![ChatMessage::user("hi")];
    let mut text = String::new();
    let mut saw_done = false;
    provider
        .stream_chat(&messages, &mut |ev| match ev {
            StreamEvent::Delta(d) => text.push_str(&d),
            StreamEvent::Done => saw_done = true,
            StreamEvent::Error(e) => panic!("unexpected error: {e}"),
        })
        .unwrap();

    handle.join().unwrap();
    assert_eq!(text, "你好！");
    assert!(saw_done, "should receive Done");
}
