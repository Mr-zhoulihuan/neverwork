//! 真机浏览器验收：登录表单闭环。
//!
//! 默认跳过（普通 `cargo test` 不应该拉起浏览器）；真机验证：
//! `NW_BROWSER_LIVE=1 cargo test -p neverwork-agent --test browser_live -- --nocapture`
//!
//! 覆盖：open → snapshot 拿 `[n]` → type（含 secret 不回显）→ click → snapshot 断言回显
//! → screenshot 落盘 → close。断言点就是产品承诺：模型看见的是快照、密码不进结果。

use std::io::{Read, Write};
use std::net::TcpListener;

use neverwork_agent::browser;
use neverwork_agent::tools::{ToolCall, ToolContext};
use serde_json::json;

const PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>NW 登录验收</title></head>
<body>
  <h1>登录</h1>
  <form onsubmit="event.preventDefault();document.getElementById('out').textContent='欢迎 '+document.getElementById('u').value+' / '+document.getElementById('p').value.length;">
    <input id="u" type="text" aria-label="用户名">
    <input id="p" type="password" aria-label="密码">
    <button id="go" type="submit">登录</button>
  </form>
  <div id="out"></div>
</body></html>"#;

/// 极简静态服务：每个连接读掉请求头，回同一页 HTML 后关闭。
fn serve(listener: TcpListener) {
    for stream in listener.incoming() {
        let Ok(mut s) = stream else { continue };
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                PAGE.len(),
                PAGE
            );
            let _ = s.write_all(resp.as_bytes());
        });
    }
}

fn run(ctx: &ToolContext, name: &str, args: serde_json::Value) -> neverwork_agent::ToolResult {
    browser::execute(
        &ToolCall {
            id: format!("live-{name}"),
            name: name.into(),
            arguments: args.to_string(),
        },
        ctx,
    )
}

/// 从快照里找含 `name` 且带 `[n]` 编号的交互行，返回 `[n]`。
fn find_ref(snapshot: &str, name: &str) -> Option<String> {
    snapshot.lines().find_map(|l| {
        if !l.contains(name) {
            return None;
        }
        let start = l.rfind('[')?;
        let end = l.rfind(']')?;
        if end <= start {
            return None;
        }
        Some(l[start..=end].to_string())
    })
}

#[test]
fn browser_login_flow() {
    if std::env::var("NW_BROWSER_LIVE").ok().as_deref() != Some("1") {
        eprintln!("跳过真机浏览器测试：设 NW_BROWSER_LIVE=1 才跑");
        return;
    }

    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定本地端口失败");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || serve(listener));

    let dir = std::env::temp_dir().join(format!("nw-browser-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ctx = ToolContext::new(&dir);

    // 1. 打开页面
    let r = run(
        &ctx,
        "browser_open",
        json!({ "url": format!("http://127.0.0.1:{port}/") }),
    );
    assert!(r.ok, "browser_open 失败：{}", r.content);

    // 2. 快照里能看见文本与三个交互元素
    let snap = run(&ctx, "browser_snapshot", json!({}));
    assert!(snap.ok, "browser_snapshot 失败：{}", snap.content);
    assert!(snap.content.contains("页面：NW 登录验收"), "缺少页面标题：{}", snap.content);
    assert!(snap.content.contains("登录"), "缺少按钮文本：{}", snap.content);
    let user_ref = find_ref(&snap.content, "用户名").expect("快照里找不到用户名输入框");
    let pass_ref = find_ref(&snap.content, "密码").expect("快照里找不到密码输入框");
    let btn_ref = find_ref(&snap.content, "登录").expect("快照里找不到登录按钮");

    // 3. 填用户名 / 密码（密码 secret），点击登录
    let r = run(&ctx, "browser_type", json!({ "target": user_ref, "text": "alice" }));
    assert!(r.ok, "输入用户名失败：{}", r.content);
    assert!(!r.content.contains("alice"), "用户名也不该回显：{}", r.content);

    let r = run(
        &ctx,
        "browser_type",
        json!({ "target": pass_ref, "text": "s3cret", "secret": true }),
    );
    assert!(r.ok, "输入密码失败：{}", r.content);
    assert!(!r.content.contains("s3cret"), "密码绝不能出现在工具结果里：{}", r.content);

    let r = run(&ctx, "browser_click", json!({ "target": btn_ref }));
    assert!(r.ok, "点击登录失败：{}", r.content);

    // 4. 断言页面回显（用户名 + 密码长度）
    let snap = run(&ctx, "browser_snapshot", json!({}));
    assert!(snap.ok, "登录后快照失败：{}", snap.content);
    assert!(
        snap.content.contains("欢迎 alice / 6"),
        "登录回显不正确：{}",
        snap.content
    );

    // 5. 截图落到工作区 artifacts，并可作为图片产物
    let shot = run(&ctx, "browser_screenshot", json!({ "name": "login" }));
    assert!(shot.ok, "截图失败：{}", shot.content);
    assert!(
        shot.content.contains("截图已保存："),
        "截图结果缺少路径行：{}",
        shot.content
    );
    assert!(
        dir.join("artifacts/login.png").is_file(),
        "截图没有写到 <root>/artifacts/login.png"
    );

    // 6. 关闭并清理
    let r = run(&ctx, "browser_close", json!({}));
    assert!(r.ok, "关闭浏览器失败：{}", r.content);

    let _ = std::fs::remove_dir_all(&dir);
}
