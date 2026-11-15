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
/// 跑一次 Agent 循环（模型 → 工具 → 回填，直到模型不再要工具）。
///
/// `enable_tools = false` 时**不把工具清单发给模型**：模型看不到工具就不会
/// 试着调用，纯文本作答。关工具的场景（比如纯问答）也因此更省 token。
#[allow(clippy::too_many_arguments)]
pub fn run_agent(
    provider_id: &str,
    config: ProviderConfig,
    system_prompt: &str,
    history: Vec<ChatMessage>,
    user_text: &str,
    root: PathBuf,
    enable_tools: bool,
    emit: &mut dyn FnMut(AgentEvent),
    cancel: Arc<AtomicBool>,
) {
    let ctx = ToolContext::new(root.clone());
    let specs = if enable_tools {
        tools::tool_specs()
    } else {
        Vec::new()
    };

    let mut messages: Vec<ChatMessage> = Vec::new();
    let mut sys = build_system_prompt(system_prompt, &root);
    if !enable_tools {
        sys.push_str(
            "

注意：本次会话没有开启文件工具。只做回答与解释，             不要声称已经读过或改过任何文件。",
        );
    }
    messages.push(ChatMessage::system(sys));
    messages.extend(history);
    messages.push(ChatMessage::user(user_text.to_string()));

    let provider = build_provider(provider_id, config);
    let with_tools = enable_tools && provider.supports_tools();

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

/// 系统提示词：先分清「回答问题」和「改代码」，再决定要不要用工具。
///
/// 之前的提示词通篇在讲怎么读写文件、还要求「改文件之前先用 read_file」，
/// 模型于是把**任何输入**都当成代码任务：问一句常识也先去翻目录、
/// 解释一个概念也去搜文件，既慢又答不到点上。
pub(crate) fn build_system_prompt(persona: &str, root: &std::path::Path) -> String {
    let persona = if persona.trim().is_empty() {
        "你是一个在 IDE 里工作的编程助手。"
    } else {
        persona
    };
    let root_p = root.display();
    format!(
        "{persona}

你工作在一个代码编辑器里，当前工作区根目录：{root_p}

## 先判断用户要什么

用户的输入大致分两类，处理方式完全不同：

**A. 提问 / 解释 / 讨论** —— 直接回答，不要动工具。
包括：概念解释、技术选型、报错原因分析、代码该怎么写、方案对比、
闲聊与自我介绍、翻译、写作、常识问答、学习建议。
这类问题**不要**调用任何工具，不要读文件、不要列目录。
凭你已有的知识直接答；确实需要看代码才能回答时才读。
不确定的事实就说不知道，不要编造，也不要靠翻文件掩盖不确定性。

**B. 在本工作区里动手** —— 才使用工具。
包括：读某个真实文件、查找某段代码在哪、修改/新建/删除文件、重构。
这类任务先看清真实内容再动手，不要凭猜测改代码。

判断依据是**用户想要结果，还是想要信息**：
要结果（「把这个函数改成异步」「加个单测」）→ B；
要信息（「这个报错啥意思」「Rust 的 lifetime 怎么理解」）→ A。

## 可用工具（仅在 B 类任务时使用）

路径一律用相对工作区根的路径。

- read_file(path, offset?, limit?)：读文件，返回带行号的内容
- list_dir(path?)：列目录
- search_files(pattern, path?)：按文本搜索
- write_file(path, content)：整体写入（新建或覆盖）
- apply_patch(path, old_string, new_string)：把一段**唯一存在**的原文替换掉
- multi_patch(edits[])：一次改多个文件，整批当作事务（全部校验通过才写，失败回滚），
  只弹一次审批。跨文件重构（改签名、重命名、批量替换）优先用它

## 工作方式

1. B 类任务先 read_file 看真实内容，再决定改什么。
2. 小改动用 apply_patch；动 2 个以上文件用 multi_patch 一次提交，
   old_string 都要带足够上下文保证唯一。
3. 写操作会弹给用户批准，批准后才落盘；被拒绝就换方案，不要重复同样的写入。
4. 回答简洁：说清结论、改了什么、为什么。不要复述整个文件，不要为了显得勤奋而
   堆叠工具调用。
5. A 类问题用自然语言答完即可，不需要「下一步建议」式的清单。"
    )
}
