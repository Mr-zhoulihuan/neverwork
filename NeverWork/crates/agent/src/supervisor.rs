//! supervisor 编排：由 Lead 逐轮决定「下一步交给谁」，而不是机械轮流。
//!
//! 与 pipeline 的区别：pipeline 是固定顺序，supervisor 是**动态指派**——
//! Lead 看到当前进展后选下一个角色，也可以在信息足够时直接收尾。
//!
//! 决策用一次独立的、极短的模型调用（只让它输出角色名或 DONE），
//! 解析失败就退化成流水线顺序，保证不会卡死。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use neverwork_core::{Role, Team};

use crate::{ChatMessage, ProviderConfig, StreamEvent};

/// 最多指派几轮。Lead 自己也算一轮，避免无限循环烧 token。
pub const MAX_STEPS: usize = 6;

/// 一次指派决策。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// 交给这个下标的角色。
    Assign(usize),
    /// 认为任务已完成，结束。
    Done,
    /// 无法解析，调用方应退化处理。
    Unknown,
}

/// 从 Lead 的回复里解析出决策。
///
/// 容忍模型多说几句：优先找 `DONE`，其次找出现的第一个角色名（忽略大小写）。
pub fn parse_decision(reply: &str, roles: &[Role]) -> Decision {
    let up = reply.to_uppercase();
    // DONE 优先：出现 "DONE" 或「完成」且没有明确指派时收尾
    let says_done = up.contains("DONE")
        || reply.contains("已完成")
        || reply.contains("无需继续")
        || reply.contains("不必继续");

    // 角色名匹配（取最早出现的一个，这样「先 Coder 后 Reviewer」会选 Coder）
    let mut best: Option<(usize, usize)> = None;
    for (i, r) in roles.iter().enumerate() {
        if r.name.trim().is_empty() {
            continue;
        }
        if let Some(pos) = reply.to_lowercase().find(&r.name.to_lowercase()) {
            if best.map(|(_, p)| pos < p).unwrap_or(true) {
                best = Some((i, pos));
            }
        }
    }

    match (best, says_done) {
        // 明确点名了角色 → 指派（即便同时也说了"完成"，点名更具体）
        (Some((i, _)), _) => Decision::Assign(i),
        (None, true) => Decision::Done,
        (None, false) => Decision::Unknown,
    }
}

/// 拼接决策用的提示词。
pub fn decision_prompt(team: &Team, user_text: &str, progress: &[(String, String)]) -> String {
    let mut roster = String::new();
    for r in &team.roles {
        let kind = if r.kind_label.is_empty() {
            String::new()
        } else {
            format!("（{}）", r.kind_label)
        };
        let duty = if r.prompt.trim().is_empty() {
            String::new()
        } else {
            format!("：{}", r.prompt.trim())
        };
        roster.push_str(&format!("- {}{}{}\n", r.name, kind, duty));
    }

    let mut done = String::new();
    if progress.is_empty() {
        done.push_str("（还没有任何角色开始工作）\n");
    } else {
        for (name, summary) in progress {
            let one_line: String = summary.lines().next().unwrap_or("").chars().take(120).collect();
            done.push_str(&format!("- {name}：{one_line}\n"));
        }
    }

    format!(
        "你是这个团队的 Lead，负责决定下一步交给谁，而不是自己完成任务。

团队成员：
{roster}
用户任务：
{user_text}

目前进展：
{done}
请只回复一个角色名（从上面成员里的名字中选），表示下一步交给谁。
如果任务已经完成、或再继续也没有意义，只回复 DONE。不要输出任何其他内容。"
    )
}

/// 跑 supervisor 编排。
///
/// `lead_idx` 是负责决策的角色下标；通常就是第一个角色。
#[allow(clippy::too_many_arguments)]
pub fn run_supervisor(
    team: &Team,
    user_text: &str,
    base_config: ProviderConfig,
    provider_id: &str,
    emit: &mut dyn FnMut(crate::TeamEvent),
    cancel: Arc<AtomicBool>,
) {
    if team.roles.is_empty() {
        return;
    }
    // Lead 优先取名字/id 带 lead/supervisor 的角色，否则用第一个
    let lead_idx = team
        .roles
        .iter()
        .position(|r| {
            let n = format!("{} {}", r.name.to_lowercase(), r.id.to_lowercase());
            n.contains("lead") || n.contains("supervisor") || n.contains("主管")
        })
        .unwrap_or(0);

    let mut progress: Vec<(String, String)> = Vec::new();
    let mut used: Vec<usize> = Vec::new();

    for step in 0..MAX_STEPS {
        if cancel.load(Ordering::Relaxed) {
            return;
        }

        // ---- 1. 让 Lead 决策 ----
        let prompt = decision_prompt(team, user_text, &progress);
        let (prov, cfg) = crate::team::role_provider(provider_id, &base_config, &team.roles[lead_idx]);
        let decision_provider = crate::build_provider(&prov, cfg);

        let mut reply = String::new();
        {
            let msgs = vec![ChatMessage::user(prompt)];
            let mut on = |ev: StreamEvent| {
                if let StreamEvent::Delta(d) = ev {
                    reply.push_str(&d);
                }
            };
            let _ = decision_provider.stream_chat(&msgs, &mut on);
        }
        if cancel.load(Ordering::Relaxed) {
            return;
        }

        let decision = parse_decision(&reply, &team.roles);

        // ---- 2. 决定下一步 ----
        let target = match decision {
            Decision::Done => {
                emit(crate::TeamEvent::RoleStart { role_index: lead_idx });
                emit(crate::TeamEvent::Delta {
                    role_index: lead_idx,
                    text: format!(
                        "【Lead】判定任务已可收尾（共 {} 步）。",
                        progress.len()
                    ),
                });
                emit(crate::TeamEvent::RoleDone { role_index: lead_idx });
                return;
            }
            Decision::Assign(i) => i,
            Decision::Unknown => {
                // 解析不出就按「还没干过活」的顺序挑一个，保证能推进
                let fallback = team
                    .roles
                    .iter()
                    .enumerate()
                    .find(|(i, _)| *i != lead_idx && !used.contains(i))
                    .map(|(i, _)| i)
                    .unwrap_or(lead_idx);
                fallback
            }
        };

        // 同一个角色连续被指派两次以上说明在原地打转，收尾
        let same_twice = used.last() == Some(&target) && used.iter().filter(|x| **x == target).count() >= 2;
        if same_twice || step + 1 == MAX_STEPS {
            emit(crate::TeamEvent::RoleStart { role_index: lead_idx });
            emit(crate::TeamEvent::Delta {
                role_index: lead_idx,
                text: if same_twice {
                    format!(
                        "【Lead】{} 已连续处理两次，为避免空转先停在这里。",
                        team.roles[target].name
                    )
                } else {
                    "【Lead】已达单轮指派上限，先停在这里等你确认。".to_string()
                },
            });
            emit(crate::TeamEvent::RoleDone { role_index: lead_idx });
            return;
        }

        // ---- 3. 执行被指派的角色 ----
        used.push(target);
        emit(crate::TeamEvent::RoleStart { role_index: target });

        let role = &team.roles[target];
        let mut msgs = vec![ChatMessage::system(crate::team::role_system_prompt(role))];
        msgs.push(ChatMessage::user(user_text.to_string()));
        for (name, body) in &progress {
            msgs.push(ChatMessage::assistant(format!("[{name}] {body}")));
        }
        if target == lead_idx {
            msgs.push(ChatMessage::user(
                "（你同时是这个任务的执行者，请直接完成你负责的那部分。）",
            ));
        }

        let (prov, cfg) = crate::team::role_provider(provider_id, &base_config, role);
        let provider = crate::build_provider(&prov, cfg);

        let mut collected = String::new();
        {
            let mut on = |ev: StreamEvent| match ev {
                StreamEvent::Delta(d) => {
                    if !cancel.load(Ordering::Relaxed) {
                        collected.push_str(&d);
                        emit(crate::TeamEvent::Delta {
                            role_index: target,
                            text: d,
                        });
                    }
                }
                StreamEvent::Error(e) => emit(crate::TeamEvent::RoleError {
                    role_index: target,
                    error: e,
                }),
                StreamEvent::ToolCalls(_) => {}
                StreamEvent::Done => {}
            };
            let _ = provider.stream_chat(&msgs, &mut on);
        }
        emit(crate::TeamEvent::RoleDone { role_index: target });

        if !collected.trim().is_empty() {
            progress.push((role.name.clone(), collected));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neverwork_core::FlowMode;

    fn team(names: &[&str]) -> Team {
        Team {
            id: "t".into(),
            name: "T".into(),
            flow: FlowMode::Supervisor,
            roles: names
                .iter()
                .map(|n| {
                    let mut r = Role::new(n.to_lowercase(), *n, "X");
                    r.kind_label = "角色".into();
                    r
                })
                .collect(),
        }
    }

    #[test]
    fn parses_exact_role_name() {
        let t = team(&["Planner", "Coder", "Reviewer"]);
        assert_eq!(parse_decision("Coder", &t.roles), Decision::Assign(1));
    }

    #[test]
    fn parses_role_name_inside_sentence() {
        let t = team(&["Planner", "Coder", "Reviewer"]);
        assert_eq!(
            parse_decision("下一步应该交给 Reviewer 来检查。", &t.roles),
            Decision::Assign(2)
        );
    }

    #[test]
    fn case_insensitive() {
        let t = team(&["Planner", "Coder"]);
        assert_eq!(parse_decision("coder", &t.roles), Decision::Assign(1));
    }

    #[test]
    fn parses_done() {
        let t = team(&["Planner", "Coder"]);
        assert_eq!(parse_decision("DONE", &t.roles), Decision::Done);
        assert_eq!(parse_decision("任务已完成", &t.roles), Decision::Done);
        assert_eq!(parse_decision("无需继续", &t.roles), Decision::Done);
    }

    #[test]
    fn unknown_when_nothing_matches() {
        let t = team(&["Planner", "Coder"]);
        assert_eq!(parse_decision("随便说点什么", &t.roles), Decision::Unknown);
        assert_eq!(parse_decision("", &t.roles), Decision::Unknown);
    }

    #[test]
    fn name_beats_done() {
        // 既点名又说了完成 → 点名更具体，应指派
        let t = team(&["Planner", "Coder"]);
        assert_eq!(
            parse_decision("Coder 已完成这部分", &t.roles),
            Decision::Assign(1)
        );
    }

    #[test]
    fn earliest_name_wins() {
        let t = team(&["Planner", "Coder"]);
        let d = parse_decision("先让 Coder 做，然后 Planner 复核", &t.roles);
        assert_eq!(d, Decision::Assign(1), "Coder 出现得更早");
    }

    #[test]
    fn decision_prompt_lists_roles_and_progress() {
        let t = team(&["Planner", "Coder"]);
        let p = decision_prompt(&t, "重构支付", &[("Planner".into(), "拆成三步".into())]);
        assert!(p.contains("Planner"));
        assert!(p.contains("Coder"));
        assert!(p.contains("重构支付"));
        assert!(p.contains("拆成三步"));
        assert!(p.contains("DONE"));
    }

    #[test]
    fn decision_prompt_handles_empty_progress() {
        let t = team(&["Planner"]);
        let p = decision_prompt(&t, "任务", &[]);
        assert!(p.contains("还没有任何角色开始工作"));
    }
}
