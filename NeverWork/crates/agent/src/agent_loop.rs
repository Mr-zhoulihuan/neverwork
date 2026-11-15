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
use crate::{build_provider, ChatMessage, ProviderConfig, Role, StreamEvent};

/// 单次请求最多跑几轮工具（一轮 = 一次模型调用 + 若干工具执行）。
///
/// 8 轮对「阅读这个项目」这类任务不够：模型每轮读一个文件，
/// 十几轮就用完了，用户只看到一堆读文件然后什么都没答。
const MAX_ROUNDS: usize = 16;
/// 等待用户批准的最长时间。超时按拒绝处理，避免线程永久挂住。
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

/// 发给模型的上下文字符预算（粗估 4 字符/token，约 40k token）。
///
/// 一次 `read_file` 默认吐 400 行、约 1.5 万字符。多轮读下来，光工具结果
/// 就能到几十万字符，远超模型上下文窗口——provider 直接返回 context-length
/// 错误。用户看到的现象就是「一直在读取，然后报错」。
const CONTEXT_BUDGET_CHARS: usize = 160_000;
/// 压缩早期工具结果时，每条保留的开头长度（保住开头才能认出是哪个文件）。
const TOOL_RESULT_HEAD: usize = 3_000;
/// 最近这么多条工具结果保持原样：模型当前推理正靠它们。
const RECENT_TOOL_RESULTS_KEPT: usize = 2;
/// 截短后拼在末尾的标记，让模型知道这里缺了东西、可以重读。
const TOOL_OMITTED_MARK: &str =
    "\n…（这段早期内容已省略以控制上下文；需要细节请重新 read_file）";

/// 把字符下标收敛到合法的 UTF-8 边界（`str::truncate` 落在字符中间会 panic）。
fn char_boundary_at(s: &str, want: usize) -> usize {
    if want >= s.len() {
        return s.len();
    }
    let mut i = want;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// 截短一条工具结果，返回实际省下的字符数。
fn truncate_tool(m: &mut ChatMessage) -> usize {
    let cut = char_boundary_at(&m.content, TOOL_RESULT_HEAD);
    let freed = (m.content.len() - cut).saturating_sub(TOOL_OMITTED_MARK.len());
    m.content.truncate(cut);
    m.content.push_str(TOOL_OMITTED_MARK);
    freed
}

/// 上下文超预算时，从**最早**的工具结果开始截短，直到回到预算内。
///
/// 只改内容、不删消息：OpenAI 协议要求每个 `assistant.tool_calls` 后面
/// 都必须有对应的 tool 消息，少一条就整轮 400。用户消息与助手正文也不动，
/// 那是对话本身，压掉会让模型串不上前因。
///
/// 先跳过最近 `RECENT_TOOL_RESULTS_KEPT` 条；若只靠早期内容仍回不到预算内
/// （比如最后几条自身就很大），再无视这个偏好继续压——宁可丢掉细节，
/// 也不能让整轮请求被 provider 拒掉。
fn shrink_tool_results(messages: &mut [ChatMessage]) {
    let total: usize = messages.iter().map(|m| m.content.len()).sum();
    if total <= CONTEXT_BUDGET_CHARS {
        return;
    }
    let mut need = total - CONTEXT_BUDGET_CHARS;
    // 正序压：丢掉最旧的，模型当前推理靠的是最新的。
    let shrinkable = messages
        .iter()
        .filter(|m| m.role == Role::Tool && m.content.len() > TOOL_RESULT_HEAD)
        .count()
        .saturating_sub(RECENT_TOOL_RESULTS_KEPT);
    let mut seen = 0usize;
    for m in messages.iter_mut() {
        if need == 0 || seen >= shrinkable {
            break;
        }
        if m.role != Role::Tool || m.content.len() <= TOOL_RESULT_HEAD {
            continue;
        }
        seen += 1;
        need = need.saturating_sub(truncate_tool(m));
    }
    if need == 0 {
        return;
    }
    for m in messages.iter_mut().rev() {
        if need == 0 {
            break;
        }
        if m.role != Role::Tool || m.content.len() <= TOOL_RESULT_HEAD {
            continue;
        }
        need = need.saturating_sub(truncate_tool(m));
    }
}

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
        // 每轮开始前先压一遍历史工具结果，别让上下文一路涨到 provider 拒绝。
        shrink_tool_results(&mut messages);

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

    // 预算用尽时**必须让模型基于已读到的内容作答**。之前直接结束：
    // 用户等了十几轮读取，最后只看到一句「已达上限」，一条有用的回答都没有。
    // 这里不再附工具清单，模型没有工具可调，只能总结。
    // 收尾这一次同样要压上下文——否则最容易在最后一步被 provider 拒绝。
    shrink_tool_results(&mut messages);
    emit(AgentEvent::TextDelta("\n\n".into()));
    let mut on_event = |ev: StreamEvent| {
        if let StreamEvent::Delta(d) = ev {
            emit(AgentEvent::TextDelta(d));
        }
    };
    if let Err(e) = provider.stream_chat(&messages, &mut on_event) {
        emit(AgentEvent::Error(e.to_string()));
    }
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
   **需要看多个文件时，在同一轮里并行发起多个 read_file / list_dir 调用**，
   不要一个文件等一轮：轮数预算是有限的，一个个来很快就会用完。
2. 小改动用 apply_patch；动 2 个以上文件用 multi_patch 一次提交，
   old_string 都要带足够上下文保证唯一。
3. 写操作会弹给用户批准，批准后才落盘；被拒绝就换方案，不要重复同样的写入。
4. 回答简洁：说清结论、改了什么、为什么。不要复述整个文件，不要为了显得勤奋而
   堆叠工具调用。
5. A 类问题用自然语言答完即可，不需要「下一步建议」式的清单。"
    )
}

#[cfg(test)]
mod context_budget_tests {
    use super::*;

    fn tool(i: usize, content: String) -> ChatMessage {
        ChatMessage::tool_result(format!("c{i}"), content)
    }

    #[test]
    fn within_budget_is_untouched() {
        let mut msgs = vec![
            ChatMessage::system("sys"),
            ChatMessage::user("u"),
            tool(0, "短内容".into()),
        ];
        let before: Vec<usize> = msgs.iter().map(|m| m.content.len()).collect();
        shrink_tool_results(&mut msgs);
        let after: Vec<usize> = msgs.iter().map(|m| m.content.len()).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn oldest_results_shrink_first_and_message_count_is_stable() {
        // 5 × 50000 = 250000，超出 160000 预算 90000
        let big = "x".repeat(50_000);
        let mut msgs = vec![ChatMessage::system("sys")];
        for i in 0..5 {
            msgs.push(ChatMessage::assistant_with_tools("", vec![]));
            msgs.push(tool(i, big.clone()));
        }
        let n = msgs.len();
        shrink_tool_results(&mut msgs);
        assert_eq!(msgs.len(), n, "不得删除消息，否则 tool_calls 配对会断");
        let tools: Vec<&ChatMessage> = msgs.iter().filter(|m| m.role == Role::Tool).collect();
        assert_eq!(tools.len(), 5);
        // 最早两条必被压
        assert!(tools[0].content.len() < 50_000);
        assert!(tools[1].content.len() < 50_000);
        // 最近两条保持完整
        assert_eq!(tools[3].content.len(), 50_000);
        assert_eq!(tools[4].content.len(), 50_000);
        let total: usize = msgs.iter().map(|m| m.content.len()).sum();
        assert!(total <= CONTEXT_BUDGET_CHARS, "total={total}");
    }

    #[test]
    fn oversized_recent_results_also_shrink() {
        // 3 条各 100000 = 300000，即使保留最近两条也回不到 160000
        let big = "y".repeat(100_000);
        let mut msgs = vec![ChatMessage::user("u")];
        for i in 0..3 {
            msgs.push(tool(i, big.clone()));
        }
        shrink_tool_results(&mut msgs);
        let total: usize = msgs.iter().map(|m| m.content.len()).sum();
        assert!(total <= CONTEXT_BUDGET_CHARS, "total={total}");
    }

    #[test]
    fn multibyte_content_never_panics_and_is_marked() {
        // 汉字 3 字节：截断点必然落在字符中间
        let big = "中".repeat(20_000);
        let mut msgs = vec![ChatMessage::user("u")];
        for i in 0..5 {
            msgs.push(tool(i, big.clone()));
        }
        shrink_tool_results(&mut msgs);
        let tools: Vec<&ChatMessage> = msgs.iter().filter(|m| m.role == Role::Tool).collect();
        assert!(
            tools[0].content.ends_with("read_file）"),
            "被截短的结果要带上省略标记"
        );
    }

    #[test]
    fn user_and_assistant_text_is_preserved() {
        let big = "z".repeat(80_000);
        let mut msgs = vec![
            ChatMessage::user(big.clone()),
            ChatMessage::assistant(big.clone()),
            tool(0, big.clone()),
            tool(1, big.clone()),
            tool(2, big.clone()),
        ];
        shrink_tool_results(&mut msgs);
        assert_eq!(msgs[0].content.len(), 80_000, "用户消息不压");
        assert_eq!(msgs[1].content.len(), 80_000, "助手正文不压");
    }
}
