//! 端到端工具调用测试：打真实 DeepSeek API，验证「读文件 → 改文件」闭环。
//!
//! 需要显式提供密钥才会跑，避免默认测试打外网、也避免把密钥写进代码：
//!
//! ```bash
//! set NW_DEEPSEEK_KEY=sk-xxx
//! cargo test -p neverwork-agent --test deepseek_tools -- --nocapture --test-threads=1
//! ```
//!
//! 没有密钥时自动跳过（测试通过但打印跳过原因）。

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use neverwork_agent::{run_agent, AgentEvent, ProviderConfig};

fn key() -> Option<String> {
    std::env::var("NW_DEEPSEEK_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
}

fn temp_workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nw-ds-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 记录一轮 Agent 跑出来的全部事件摘要。
#[derive(Default)]
struct Trace {
    text: String,
    tools: Vec<String>,
    ok: Vec<String>,
    errors: Vec<String>,
    denied: usize,
    rounds_limited: bool,
    /// Provider 报的真实用量（全轮累加），拿不到时为 (0, 0)。
    input_tokens: u64,
    output_tokens: u64,
}

fn run(
    dir: &PathBuf,
    prompt: &str,
    system: &str,
    approve: bool,
) -> Trace {
    let key = key().expect("需要 NW_DEEPSEEK_KEY");
    let config = ProviderConfig {
        api_key: key,
        base_url: "https://api.deepseek.com/v1".into(),
        model: std::env::var("NW_DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-flash".into()),
        temperature: 0.2,
        thinking: neverwork_core::ThinkingLevel::Auto,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let mut trace = Trace::default();

    let mut emit = |ev: AgentEvent| match ev {
        AgentEvent::TextDelta(d) => trace.text.push_str(&d),
        AgentEvent::ToolStarted { call, .. } => trace.tools.push(call.name.clone()),
        AgentEvent::ToolFinished { result } => {
            if result.ok {
                trace.ok.push(result.name.clone());
                eprintln!(
                    "[工具成功] {} -> {}",
                    result.name,
                    result.content.lines().next().unwrap_or("")
                );
            } else {
                trace.errors.push(result.content.clone());
                eprintln!("[工具失败] {} -> {}", result.name, result.content);
            }
        }
        AgentEvent::NeedApproval {
            call, respond, preview, ..
        } => {
            eprintln!(
                "[请求批准] {} {} （diff {} 行）-> {}",
                call.name,
                call.summary(),
                preview.len(),
                if approve { "自动允许" } else { "拒绝" }
            );
            let _ = respond.send(approve);
        }
        AgentEvent::Denied { call } => {
            trace.denied += 1;
            eprintln!("[已拒绝] {}", call.name);
        }
        AgentEvent::RoundLimit => trace.rounds_limited = true,
        AgentEvent::TimeLimit => trace.rounds_limited = true,
        AgentEvent::CostLimit { .. } => trace.rounds_limited = true,
        // 单 Agent 跑的路径没有团队，交接事件不会出现；出现即视为异常。
        AgentEvent::HandoffRequest { from, .. } => {
            trace.errors.push(format!("意外交接：{from}"));
        }
        AgentEvent::HandoffLimit => trace.errors.push("意外触发交接上限".into()),
        AgentEvent::Compacting => eprintln!("[上下文] 正在压缩更早的对话…"),
        AgentEvent::Usage {
            input_tokens,
            output_tokens,
            ..
        } => {
            trace.input_tokens = input_tokens;
            trace.output_tokens = output_tokens;
            eprintln!("[用量] 输入 {input_tokens} / 输出 {output_tokens} tokens");
        }
        AgentEvent::Error(e) => trace.errors.push(e.clone()),
        AgentEvent::Done => {}
    };

    run_agent(
        "openai",
        config,
        system,
        Vec::new(),
        prompt,
        neverwork_agent::tools::ToolContext::new(dir.clone()),
        true,
        neverwork_core::PermissionMode::Standard,
        // 测试里不设运行上限；上限判定由 RunLimits 的调用方负责。
        neverwork_agent::RunLimits::default(),
        &mut emit,
        cancel,
    );
    trace
}

#[test]
fn deepseek_reads_file() {
    if key().is_none() {
        eprintln!("跳过：未设置 NW_DEEPSEEK_KEY");
        return;
    }
    let dir = temp_workspace("read");
    std::fs::write(dir.join("note.txt"), "苹果\n香蕉\n樱桃\n").unwrap();

    let trace = run(
        &dir,
        "读取 note.txt，然后只回答里面有哪几种水果，用顿号分隔。",
        "你是一个简洁的助手。",
        true,
    );

    eprintln!("--- 模型回答 ---\n{}", trace.text);
    assert!(
        trace.tools.iter().any(|t| t == "read_file"),
        "模型应该调用 read_file，实际：{:?}",
        trace.tools
    );
    assert!(trace.ok.iter().any(|t| t == "read_file"), "read_file 应执行成功");
    assert!(
        trace.text.contains("苹果") || trace.text.contains("樱桃"),
        "回答里应包含文件内容：{}",
        trace.text
    );
    assert!(trace.errors.is_empty(), "不应有错误：{:?}", trace.errors);
}

#[test]
fn deepseek_creates_file_after_approval() {
    if key().is_none() {
        eprintln!("跳过：未设置 NW_DEEPSEEK_KEY");
        return;
    }
    let dir = temp_workspace("create");

    let trace = run(
        &dir,
        "在工作区根目录创建一个文件 hello.txt，内容只有一行：HELLO。",
        "你是一个在 IDE 里工作的编程助手。",
        true,
    );

    eprintln!("--- 模型回答 ---\n{}", trace.text);
    let written = dir.join("hello.txt");
    assert!(
        written.exists(),
        "批准后应创建 hello.txt；调用过的工具：{:?}；错误：{:?}",
        trace.tools,
        trace.errors
    );
    let content = std::fs::read_to_string(&written).unwrap();
    assert!(
        content.contains("HELLO"),
        "文件内容应包含 HELLO，实际：{content:?}"
    );
}

#[test]
fn deepseek_denied_write_leaves_file_untouched() {
    if key().is_none() {
        eprintln!("跳过：未设置 NW_DEEPSEEK_KEY");
        return;
    }
    let dir = temp_workspace("deny");
    std::fs::write(dir.join("keep.txt"), "原始内容\n").unwrap();

    let trace = run(
        &dir,
        "把 keep.txt 的内容改成「被改掉了」。",
        "你是一个在 IDE 里工作的编程助手。",
        false, // 全部拒绝
    );

    eprintln!("--- 模型回答 ---\n{}", trace.text);
    let still = std::fs::read_to_string(dir.join("keep.txt")).unwrap();
    assert_eq!(still, "原始内容\n", "拒绝写入后文件必须保持原样");
    assert!(trace.denied > 0, "应当记录到至少一次拒绝");
}
