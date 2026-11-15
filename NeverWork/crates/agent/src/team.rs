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
    match team.flow {
        FlowMode::Pipeline => run_pipeline(team, user_text, base_config, emit, cancel),
        FlowMode::Parallel | FlowMode::Supervisor => {
            run_parallel(team, user_text, base_config, emit, cancel)
        }
    }
    emit(TeamEvent::AllDone);
}

/// 解析角色模型名，若看起来是跨 provider 的模型（如 claude*）则切换到 anthropic。
/// 返回 (provider 名称, 应用于该角色的配置)。
fn role_provider(base_provider: &str, base: &ProviderConfig, role: &Role) -> (String, ProviderConfig) {
    let model = role.model.to_lowercase();
    let provider = if model.starts_with("claude") {
        "anthropic".to_string()
    } else {
        base_provider.to_string()
    };
    let mut cfg = base.clone();
    if !role.model.trim().is_empty() {
        cfg.model = role.model.clone();
    }
    (provider, cfg)
}

/// 流水线：顺序执行，累积上下文。
fn run_pipeline(
    team: &Team,
    user_text: &str,
    base_config: ProviderConfig,
    emit: &mut dyn FnMut(TeamEvent),
    cancel: Arc<AtomicBool>,
) {
    let provider_id = std::env::var("NW_PROVIDER").unwrap_or_else(|_| "openai".into());
    // 累积的对话上下文（每个角色的最终产出会加进来）。
    let mut conversation: Vec<ChatMessage> = vec![ChatMessage::user(user_text.to_string())];

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
        handles.push(std::thread::spawn(move || {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let (prov, cfg) = role_provider(&pid, &base, &role);
            let provider = crate::build_provider(&prov, cfg);
            let msgs = vec![
                ChatMessage::system(role_system_prompt(&role)),
                ChatMessage::user(user_text),
            ];
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

fn role_system_prompt(role: &Role) -> String {
    if role.prompt.trim().is_empty() {
        format!(
            "你是团队中的 {}（{}）。请专注你的职责，简洁、专业地完成分配给你的部分。",
            role.name, role.kind_label
        )
    } else {
        role.prompt.clone()
    }
}
