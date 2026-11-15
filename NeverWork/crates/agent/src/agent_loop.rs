//! Agent 主循环：模型 → 工具 → 回填 → 再来一轮，直到模型不再调用工具。
//!
//! 关键约束：
//! - **写操作必须经用户批准**。批准前只计算 diff 预览，不落盘。
//! - 轮数有上限，避免模型陷入「改一点、再改一点」的死循环烧 token。
//! - 用户随时可以取消（`cancel`），每轮开头与每个工具执行前都检查。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::diff::DiffLine;
use crate::tools::{self, ToolCall, ToolContext, ToolResult};
use crate::{build_provider, ChatMessage, ProviderConfig, StreamEvent};

/// 单次请求最多跑几轮工具（一轮 = 一次模型调用 + 若干工具执行）。
const MAX_ROUNDS: usize = 8;
/// 等待用户批准的最长时间。超时按拒绝处理，避免线程永久挂住。
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

/// Agent 运行过程中的事件。
pub enum AgentEvent {
    /// 文本增量（模型说的话）。
    TextDelta(String),
    /// 开始执行一个工具（已决定执行或等待批准）。
    ToolStarted {
        call: ToolCall,
        requires_approval: bool,
        /// 写操作的 diff 预览，只读工具为空。
        preview: Vec<DiffLine>,
    },
    /// 需要用户批准写操作。调用方必须把 `respond` 交给界面，
    /// 用户点「允许」发 true、点「拒绝」发 false。
    NeedApproval {
        call: ToolCall,
        preview: Vec<DiffLine>,
        respond: flume::Sender<bool>,
    },
    /// 工具执行完成。
    ToolFinished { result: ToolResult },
    /// 用户拒绝了某次写操作。
    Denied { call: ToolCall },
    /// 达到轮数上限（模型还想继续调工具，但被我们停下）。
    RoundLimit,
    /// 本轮结束。
    Done,
    /// 出错。
    Error(String),
}

/// 运行一次带工具的 Agent。
///
/// 写操作的批准通过 `AgentEvent::NeedApproval` 里携带的 `respond` 通道完成：
/// 界面收到事件后展示 diff 与「允许 / 拒绝」，把用户选择发回该通道即可。
pub fn run_agent(
    provider_id: &str,
    config: ProviderConfig,
    system_prompt: &str,
    history: Vec<ChatMessage>,
    user_text: &str,
    root: PathBuf,
    emit: &mut dyn FnMut(AgentEvent),
    cancel: Arc<AtomicBool>,
) {
    let ctx = ToolContext::new(root.clone());
    let specs = tools::tool_specs();

    let mut messages: Vec<ChatMessage> = Vec::new();
    messages.push(ChatMessage::system(build_system_prompt(system_prompt, &root)));
    messages.extend(history);
    messages.push(ChatMessage::user(user_text.to_string()));

    let provider = build_provider(provider_id, config);
    let with_tools = provider.supports_tools();

    for round in 0..MAX_ROUNDS {
        if cancel.load(Ordering::Relaxed) {
            emit(AgentEvent::Done);
            return;
        }
        // 只有第一轮需要附上工具说明；模型记住上下文即可。
        let _ = round;

        let mut text = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut error: Option<String> = None;

        let mut on_event = |ev: StreamEvent| match ev {
            StreamEvent::Delta(d) => {
                text.push_str(&d);
                emit(AgentEvent::TextDelta(d));
            }
            StreamEvent::ToolCalls(c) => calls.extend(c),
            StreamEvent::Error(e) => error = Some(e),
            StreamEvent::Done => {}
        };

        let result = if with_tools {
            provider.stream_chat_with_tools(&messages, &specs, &mut on_event)
        } else {
            provider.stream_chat(&messages, &mut on_event)
        };
        if let Err(e) = result {
            error = Some(e.to_string());
        }
        if let Some(e) = error {
            emit(AgentEvent::Error(e));
            return;
        }

        if calls.is_empty() {
            emit(AgentEvent::Done);
            return;
        }

        // 把这一轮助手说了什么 + 要调什么记进上下文
        messages.push(ChatMessage::assistant_with_tools(text, calls.clone()));

        for call in calls {
            if cancel.load(Ordering::Relaxed) {
                emit(AgentEvent::Done);
                return;
            }

            let need_approval = tools::requires_approval(&call.name);
            let preview = if need_approval {
                tools::preview(&call, &ctx).unwrap_or_default()
            } else {
                Vec::new()
            };
            emit(AgentEvent::ToolStarted {
                call: call.clone(),
                requires_approval: need_approval,
                preview: preview.clone(),
            });

            if need_approval {
                // 预览本身失败（比如路径越界）也要让用户看到原因，但仍允许拒绝。
                let allowed = {
                    let (tx, rx) = flume::bounded::<bool>(1);
                    emit(AgentEvent::NeedApproval {
                        call: call.clone(),
                        preview: preview.clone(),
                        respond: tx,
                    });
                    // 取消时也要能解开阻塞：用带超时的等待并顺带轮询 cancel。
                    let mut decided: Option<bool> = None;
                    let deadline = std::time::Instant::now() + APPROVAL_TIMEOUT;
                    while std::time::Instant::now() < deadline {
                        if cancel.load(Ordering::Relaxed) {
                            decided = Some(false);
                            break;
                        }
                        match rx.recv_timeout(Duration::from_millis(200)) {
                            Ok(v) => {
                                decided = Some(v);
                                break;
                            }
                            Err(flume::RecvTimeoutError::Timeout) => continue,
                            Err(flume::RecvTimeoutError::Disconnected) => {
                                decided = Some(false);
                                break;
                            }
                        }
                    }
                    decided.unwrap_or(false)
                };

                if !allowed {
                    emit(AgentEvent::Denied { call: call.clone() });
                    messages.push(ChatMessage::tool_result(
                        &call.id,
                        "用户拒绝了这个修改，文件未改动。请说明你原本想改什么，或换个方案。",
                    ));
                    continue;
                }
            }

            let result = tools::execute(&call, &ctx);
            messages.push(ChatMessage::tool_result(&call.id, result.content.clone()));
            emit(AgentEvent::ToolFinished { result });
        }
    }

    emit(AgentEvent::RoundLimit);
    emit(AgentEvent::Done);
}

/// 系统提示词：工具约定 + 工作方式。
fn build_system_prompt(persona: &str, root: &std::path::Path) -> String {
    let persona = if persona.trim().is_empty() {
        "你是一个在 IDE 里工作的编程助手。"
    } else {
        persona
    };
    let root_p = root.display();
    format!(
        "{persona}

你正在用户的工作区里工作，工作区根目录：{root_p}

你可以使用这些工具直接访问文件（路径一律用相对工作区根的路径）：
- read_file(path, offset?, limit?)：读文件，返回带行号的内容
- list_dir(path?)：列目录
- search_files(pattern, path?)：按文本搜索，定位改动位置
- write_file(path, content)：整体写入（新建或覆盖）
- apply_patch(path, old_string, new_string)：把一段**唯一存在**的原文替换掉
- multi_patch(edits[])：一次改多个文件，整批当作事务（全部校验通过才写，失败回滚），
  只弹一次审批。跨文件重构（改签名、重命名、批量替换）优先用它，而不是逐个 apply_patch

工作方式要求：
1. 改文件之前先用 read_file 看真实内容，不要凭猜测写代码。
2. 小改动优先用 apply_patch；要动 2 个以上文件时用 multi_patch 一次提交，
   old_string 都要带足够上下文保证唯一。
3. 写操作会弹给用户批准，批准后才会落盘；被拒绝就换方案，不要重复同样的写入。
4. 回答简洁，说清「改了什么、为什么」，不要复述整个文件。
5. 不确定的事实不要编造；需要更多信息就继续调用工具。"
    )
}
