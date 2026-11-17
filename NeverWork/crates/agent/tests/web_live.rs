//! 联网工具的真实网络冒烟：默认 `#[ignore]`，`cargo test --workspace` 不会跑。
//!
//! 手动执行：
//! `cargo test -p neverwork-agent --test web_live -- --ignored --nocapture`
//!
//! 断网环境下失败是预期内的，不属于回归。

use neverwork_agent::tools::{self, ToolCall, ToolContext};

fn call(name: &str, args: &str) -> ToolCall {
    ToolCall {
        id: "live".into(),
        name: name.into(),
        arguments: args.into(),
    }
}

#[test]
#[ignore]
fn live_search_then_fetch() {
    let ctx = ToolContext::unrestricted();

    let r = tools::execute(
        &call("web_search", r#"{"query":"Rust 语言 最新版本","count":3}"#),
        &ctx,
    );
    println!("--- web_search ---\n{}", r.content);
    assert!(r.ok, "{}", r.content);
    assert!(r.content.contains("http"), "{}", r.content);

    // 从搜索结果里取第一条链接，验证抓取链路
    let first_url = r
        .content
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("http"))
        .expect("搜索结果里没有链接")
        .to_string();
    let args = format!(r#"{{"url":"{first_url}","max_chars":600}}"#);
    let r = tools::execute(&call("web_fetch", &args), &ctx);
    println!("--- web_fetch {first_url} ---\n{}", r.content);
    assert!(r.ok, "{}", r.content);
    assert!(r.content.contains("网页"), "{}", r.content);
    // 中文不能是乱码：编码检测（charset）错位时这里是唯一能抓住的地方
    assert!(
        r.content.contains("语言") || r.content.contains("程序") || r.content.contains("文档"),
        "网页正文疑似乱码：{}",
        r.content
    );
}
