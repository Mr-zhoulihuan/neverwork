//! 多 Agent 团队编排：把用户消息按团队/编排模式分派给各角色，流式产出。
//!
//! 与 UI 解耦：通过 `TeamEvent` 回调把「某角色开始/增量/结束」交给调用方。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use neverwork_core::{FlowMode, Role, Team};

use crate::{ChatMessage, ProviderConfig, StreamEvent};

/// 团队编排产生的事件。
#[derive(Debug, Clone)]
pub enum TeamEvent {
    /// 某个角色开始发言。
    RoleStart { role_index: usize },
    /// 某角色的文本增量（role_index 对应 team.roles 顺序）。
    Delta { role_index: usize, text: String },
    /// 某个角色结束。
    RoleDone { role_index: usize },
    /// 某角色出错。
    RoleError { role_index: usize, error: String },
    /// 整个编排结束。
    AllDone,
}

/// 运行团队协作。`cancel` 用于中途停止。
///
/// 说明：
/// - 流水线：按角色顺序依次执行，后一个角色能看到前一个角色的输出。
/// - 并行 / 主管：当前实现为并行地向所有角色发起请求（主管模式暂等同并行）。
pub fn run_team(
    team: &Team,
    user_text: &str,
    base_config: ProviderConfig,
    emit: &mut dyn FnMut(TeamEvent),
    cancel: Arc<AtomicBool>,
) {
    run_team_with_history(team, Vec::new(), user_text, base_config, emit, cancel);
}

/// 带会话历史的团队编排。
///
/// `history` 是同一会话里之前聊过的正文：不传就退化成只看得见本次提问，
/// 用户在同一个会话里追问上一轮的事必然答不出。
///
/// 这条路径**不给模型 tools**（走 `stream_chat`），所以历史里的工具对会先被
/// `flatten_tool_history` 压成普通正文。
#[allow(clippy::too_many_arguments)]
pub fn run_team_with_history(
    team: &Team,
    history: Vec<ChatMessage>,
    user_text: &str,
    base_config: ProviderConfig,
    emit: &mut dyn FnMut(TeamEvent),
    cancel: Arc<AtomicBool>,
) {
    let provider_id = std::env::var("NW_PROVIDER").unwrap_or_else(|_| "openai".into());
    // 纯文本编排：历史里的 tool_calls / tool 对必须先压平，否则
    // Anthropic 会因为空正文的 assistant 整轮 400、OpenAI 也会让模型
    // 以为自己还能调工具。
    let history = flatten_tool_history(&history);
    match team.flow {
        FlowMode::Pipeline => run_pipeline(team, &history, user_text, base_config, emit, cancel),
        FlowMode::Parallel => run_parallel(team, &history, user_text, base_config, emit, cancel),
        // 主管指派：由 Lead 逐轮决定下一个角色，而不是当成并行
        FlowMode::Supervisor => {
            crate::supervisor::run_supervisor(
                team,
                &history,
                user_text,
                base_config,
                &provider_id,
                emit,
                cancel,
            )
        }
    }
    emit(TeamEvent::AllDone);
}

/// 纯文本编排用的历史：把工具调用压成普通正文。
///
/// 这条路径不给模型 tools（走 `stream_chat` 而非 `stream_chat_with_tools`），
/// 但同一会话里可能存在上一轮单 Agent 留下的 `assistant.tool_calls` + `tool` 对。
/// 原样发出去有两处会坏：
/// - Anthropic 的正文是纯字符串，空正文的 assistant 直接整轮 400；
/// - 它还会把 `Role::Tool` 当成 `user`，工具输出变成「用户说的话」。
///
/// 压成「调用过什么 + 读到了什么」的 assistant 正文：信息不丢，协议也安全。
/// 已经压过的历史原样返回，重复调用无副作用。
pub(crate) fn flatten_tool_history(history: &[ChatMessage]) -> Vec<ChatMessage> {
    use std::collections::HashMap;

    /// 把挂起的工具结果合成一条正文
    fn flush(pending: &mut Vec<String>, out: &mut Vec<ChatMessage>) {
        if pending.is_empty() {
            return;
        }
        out.push(ChatMessage::assistant(format!(
            "（上一轮工具输出）\n{}",
            pending.join("\n\n")
        )));
        pending.clear();
    }

    // call_id → 工具名：工具结果消息自己不带名字
    let mut names: HashMap<&str, &str> = HashMap::new();
    let mut out: Vec<ChatMessage> = Vec::with_capacity(history.len());
    let mut pending: Vec<String> = Vec::new();
    for m in history {
        match m.role {
            crate::Role::Assistant if !m.tool_calls.is_empty() => {
                flush(&mut pending, &mut out);
                for c in &m.tool_calls {
                    names.insert(c.id.as_str(), c.name.as_str());
                }
                let called: Vec<&str> = m.tool_calls.iter().map(|c| c.name.as_str()).collect();
                if !m.content.trim().is_empty() {
                    out.push(ChatMessage::assistant(m.content.clone()));
                }
                out.push(ChatMessage::assistant(format!(
                    "（调用工具：{}）",
                    called.join("、")
                )));
            }
            crate::Role::Tool => {
                let name = m
                    .tool_call_id
                    .as_deref()
                    .and_then(|id| names.get(id).copied())
                    .unwrap_or("工具");
                pending.push(format!("[{name}]\n{}", m.content));
            }
            _ => {
                flush(&mut pending, &mut out);
                out.push(m.clone());
            }
        }
    }
    flush(&mut pending, &mut out);
    out
}

/// 解析角色模型名，若看起来是跨 provider 的模型（如 claude*）则切换到 anthropic。
/// 返回 (provider 名称, 应用于该角色的配置)。
pub(crate) fn role_provider(base_provider: &str, base: &ProviderConfig, role: &Role) -> (String, ProviderConfig) {
    // 只有看起来像**真实模型 id** 的才覆盖模型名。
    // 团队里 `Claude Opus`、`GPT-5` 这类是展示标签（带空格/大写），
    // 拿它们当模型 id 发给服务端只会得到 400；拿 `claude` 前缀切厂商更糟——
    // 会把 DeepSeek 的 Key 发到 Anthropic。
    let raw = role.model.trim();
    // 真实模型 id 约定全小写（gpt-4o-mini / claude-sonnet-4-5 / deepseek-flash）；
    // 展示标签总有大写（Claude Opus / GPT-5），据此区分。
    let looks_like_id = !raw.is_empty()
        && !raw.contains(' ')
        && !raw.chars().any(|c| c.is_ascii_uppercase())
        && raw.chars().any(|c| c.is_ascii_digit() || c == '-' || c == '.');

    let mut cfg = base.clone();
    let mut provider = base_provider.to_string();
    if looks_like_id {
        cfg.model = raw.to_string();
        if raw.to_lowercase().starts_with("claude-") {
            provider = "anthropic".to_string();
        }
    }
    (provider, cfg)
}

/// 流水线：顺序执行，累积上下文。
fn run_pipeline(
    team: &Team,
    history: &[ChatMessage],
    user_text: &str,
    base_config: ProviderConfig,
    emit: &mut dyn FnMut(TeamEvent),
    cancel: Arc<AtomicBool>,
) {
    let provider_id = std::env::var("NW_PROVIDER").unwrap_or_else(|_| "openai".into());
    // 累积的对话上下文：起手是同一会话的历史，再加上本次提问。
    let mut conversation: Vec<ChatMessage> = history.to_vec();
    conversation.push(ChatMessage::user(user_text.to_string()));

    for (idx, role) in team.roles.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        emit(TeamEvent::RoleStart { role_index: idx });

        // 每个角色用自己的 system prompt + 共享对话历史。
        let mut msgs = vec![ChatMessage::system(role_system_prompt(role))];
        msgs.extend(conversation.iter().cloned());

        let (prov, cfg) = role_provider(&provider_id, &base_config, role);
        let provider = crate::build_provider(&prov, cfg);

        let mut collected = String::new();
        let mut on = |ev: StreamEvent| match ev {
            StreamEvent::Delta(d) => {
                if !cancel.load(Ordering::Relaxed) {
                    collected.push_str(&d);
                    emit(TeamEvent::Delta { role_index: idx, text: d });
                }
            }
            StreamEvent::Error(e) => emit(TeamEvent::RoleError { role_index: idx, error: e }),
            // 团队编排暂不使用工具调用（角色是「轮流发言」模型），忽略即可。
            StreamEvent::ToolCalls(_) => {}
            // 用量也不汇总到团队层（成员各自计费，走 trace 看）
            StreamEvent::Usage(_) => {}
            StreamEvent::Done => {}
        };
        let _ = provider.stream_chat(&msgs, &mut on);
        emit(TeamEvent::RoleDone { role_index: idx });

        // 把该角色产出加入上下文，供后续角色参考。
        if !collected.trim().is_empty() {
            conversation.push(ChatMessage::assistant(format!("[{}] {}", role.name, collected)));
        }
    }
}

/// 并行：同时向所有角色发起请求。
fn run_parallel(
    team: &Team,
    history: &[ChatMessage],
    user_text: &str,
    base_config: ProviderConfig,
    emit: &mut dyn FnMut(TeamEvent),
    cancel: Arc<AtomicBool>,
) {
    use std::sync::mpsc;

    let provider_id = std::env::var("NW_PROVIDER").unwrap_or_else(|_| "openai".into());
    let (tx, rx) = mpsc::channel::<(usize, StreamEvent)>();

    let mut handles = Vec::new();
    for (idx, role) in team.roles.iter().enumerate() {
        let role = role.clone();
        let base = base_config.clone();
        let pid = provider_id.clone();
        let tx = tx.clone();
        let cancel = cancel.clone();
        let user_text = user_text.to_string();
        // 每个角色都要一份历史（线程内部拥有）
        let history: Vec<ChatMessage> = history.to_vec();
        handles.push(std::thread::spawn(move || {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let (prov, cfg) = role_provider(&pid, &base, &role);
            let provider = crate::build_provider(&prov, cfg);
            let mut msgs = vec![ChatMessage::system(role_system_prompt(&role))];
            msgs.extend(history);
            msgs.push(ChatMessage::user(user_text));
            let mut on = |ev: StreamEvent| {
                let _ = tx.send((idx, ev));
            };
            let _ = provider.stream_chat(&msgs, &mut on);
        }));
    }
    drop(tx);

    let mut done_count = 0usize;
    let total = team.roles.len();
    let mut started = vec![false; total];
    while done_count < total {
        match rx.recv() {
            Ok((idx, ev)) => match ev {
                StreamEvent::Delta(d) => {
                    if !started[idx] {
                        started[idx] = true;
                        emit(TeamEvent::RoleStart { role_index: idx });
                    }
                    emit(TeamEvent::Delta { role_index: idx, text: d });
                }
                StreamEvent::Error(e) => emit(TeamEvent::RoleError { role_index: idx, error: e }),
                StreamEvent::ToolCalls(_) => {}
                StreamEvent::Usage(_) => {}
                StreamEvent::Done => {
                    emit(TeamEvent::RoleDone { role_index: idx });
                    done_count += 1;
                }
            },
            Err(_) => break,
        }
    }
    for h in handles {
        let _ = h.join();
    }
}

pub(crate) fn role_system_prompt(role: &Role) -> String {
    if role.prompt.trim().is_empty() {
        format!(
            "你是团队中的 {}（{}）。请专注你的职责，简洁、专业地完成分配给你的部分。",
            role.name, role.kind_label
        )
    } else {
        role.prompt.clone()
    }
}

#[cfg(test)]
mod flatten_tests {
    use super::flatten_tool_history;
    use crate::{ChatMessage, ToolCall};

    #[test]
    fn tool_pairs_become_plain_text() {
        let history = vec![
            ChatMessage::user("读一下 a.rs"),
            ChatMessage::assistant_with_tools(
                "",
                vec![ToolCall {
                    id: "call_1".into(),
                    name: "read_file".into(),
                    arguments: "{}".into(),
                }],
            ),
            ChatMessage::tool_result("call_1", "FILE_BODY"),
            ChatMessage::user("那 b.rs 呢"),
        ];
        let out = flatten_tool_history(&history);
        // 协议字段一个不留
        assert!(out.iter().all(|m| m.tool_calls.is_empty()));
        assert!(out.iter().all(|m| m.tool_call_id.is_none()));
        // 也不能留下空正文（Anthropic 会直接 400）
        assert!(out.iter().all(|m| !m.content.trim().is_empty()));
        // 但工具名与输出不能丢：模型要记得上一轮读过什么
        let joined: Vec<&str> = out.iter().map(|m| m.content.as_str()).collect();
        let joined = joined.join("\n");
        assert!(joined.contains("read_file"), "{joined}");
        assert!(joined.contains("FILE_BODY"), "{joined}");
        // 顺序不能乱
        assert_eq!(out[0].content, "读一下 a.rs");
        assert_eq!(out.last().unwrap().content, "那 b.rs 呢");
    }

    #[test]
    fn already_plain_history_is_unchanged() {
        let history = vec![ChatMessage::user("q"), ChatMessage::assistant("a")];
        let out = flatten_tool_history(&history);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].content, "q");
        assert_eq!(out[1].content, "a");
    }
}

#[cfg(test)]
mod role_provider_tests {
    use super::*;

    fn role(model: &str) -> Role {
        let mut r = Role::new("r", "R", "R");
        r.model = model.into();
        r
    }

    fn base() -> ProviderConfig {
        ProviderConfig {
            api_key: "sk-deepseek".into(),
            base_url: "https://api.deepseek.com/v1".into(),
            model: "deepseek-flash".into(),
            temperature: 0.7,
            thinking: neverwork_core::ThinkingLevel::Auto,
        }
    }

    #[test]
    fn display_label_does_not_switch_provider() {
        // 默认团队里就是这个值，之前会被误判成 anthropic
        let (p, cfg) = role_provider("openai", &base(), &role("Claude Opus"));
        assert_eq!(p, "openai", "展示标签不应切换厂商");
        assert_eq!(cfg.model, "deepseek-flash", "展示标签不应覆盖模型名");
        assert_eq!(cfg.api_key, "sk-deepseek", "Key 必须保持原厂商的");
    }

    #[test]
    fn gpt_label_does_not_override_model() {
        let (_, cfg) = role_provider("openai", &base(), &role("GPT-5"));
        assert_eq!(cfg.model, "deepseek-flash");
    }

    #[test]
    fn real_claude_id_switches_to_anthropic() {
        let (p, cfg) = role_provider("openai", &base(), &role("claude-sonnet-4-5"));
        assert_eq!(p, "anthropic");
        assert_eq!(cfg.model, "claude-sonnet-4-5");
    }

    #[test]
    fn real_openai_id_overrides_model_keeps_provider() {
        let (p, cfg) = role_provider("openai", &base(), &role("gpt-4o-mini"));
        assert_eq!(p, "openai");
        assert_eq!(cfg.model, "gpt-4o-mini");
    }

    #[test]
    fn empty_model_falls_back_to_base() {
        let (p, cfg) = role_provider("openai", &base(), &role(""));
        assert_eq!(p, "openai");
        assert_eq!(cfg.model, "deepseek-flash");
    }
}
