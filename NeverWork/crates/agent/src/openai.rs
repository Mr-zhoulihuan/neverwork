//! OpenAI 兼容 Provider（覆盖 OpenAI / DeepSeek / Qwen / GLM / Moonshot / Ollama 等）。
//!
//! 使用 `/chat/completions` 的流式接口（`stream: true`），SSE 解析：
//! - `choices[0].delta.content` → 文本增量
//! - `choices[0].delta.tool_calls[]` → 工具调用增量（`function.arguments` 会被切成碎片，需按 index 累积）
//! - `choices[0].finish_reason == "tool_calls"` → 本轮工具调用结束

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use neverwork_core::AppResult;

use crate::{
    ChatMessage, LlmProvider, ProviderConfig, Role, StreamEvent, TokenUsage, ToolCall, ToolSpec,
};

pub struct OpenAiProvider {
    config: ProviderConfig,
}

impl OpenAiProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self { config }
    }
}

#[derive(Serialize)]
struct WireToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireFunction,
}

#[derive(Serialize)]
struct WireFunction {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct WireMessage {
    role: &'static str,
    /// 仅在有 tool_calls 时可以为 null。
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    messages: Vec<WireMessage>,
    stream: bool,
    temperature: f32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    /// 不加这个，大多数网关不会在流里返回 token 用量。
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    /// 思考等级（OpenAI 兼容用）。不认识该参数的网关会 400，见下方重试。
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
}

#[derive(Serialize)]
struct WireTool {
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireToolDef,
}

#[derive(Serialize)]
struct WireToolDef {
    name: String,
    description: String,
    parameters: Value,
}

fn wire_role(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn to_wire(m: &ChatMessage) -> WireMessage {
    WireMessage {
        role: wire_role(m.role),
        content: if m.content.is_empty() && !m.tool_calls.is_empty() {
            None
        } else {
            Some(m.content.clone())
        },
        tool_calls: m
            .tool_calls
            .iter()
            .map(|c| WireToolCall {
                id: c.id.clone(),
                kind: "function",
                function: WireFunction {
                    name: c.name.clone(),
                    arguments: if c.arguments.trim().is_empty() {
                        "{}".to_string()
                    } else {
                        c.arguments.clone()
                    },
                },
            })
            .collect(),
        tool_call_id: m.tool_call_id.clone(),
    }
}

/// 累积中的工具调用。
#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

/// 把一个流式碎片并进已累积的值。
///
/// 不能无脑 `push_str`：`function.name` 按规范只在首片出现一次、之后靠拼接，
/// 但不少兼容网关/代理**每片都重发完整 name 或累计值**。这样拼出来的
/// `read_file` 会变成 `read_fileread_file`，随后按名分发时找不到工具。
/// 三种行为都要兼容：
/// - 增量碎片：`read_` + `file` → 追加
/// - 累计值：`r` → `re` → `rea`（每片带全部前缀）→ 只追加前缀之后的部分
/// - 重复完整值：每片都是 `read_file` → 忽略
///
/// 「只追加前缀之后的部分」对任意文本（含 JSON arguments）都等价于正确拼接，
/// 不会丢字符；不要用「已累积值结尾等于碎片」来判重 —— 那会把合法的
/// 重复内容（如 `"a":"a"` 的尾部）吃掉，产出非法 JSON。
fn accumulate(slot: &mut String, piece: &str) {
    if piece.is_empty() {
        return;
    }
    if slot.is_empty() {
        slot.push_str(piece);
        return;
    }
    if piece == slot.as_str() {
        return;
    }
    if piece.starts_with(slot.as_str()) {
        // `starts_with` 保证 slot.len() 落在字符边界上
        slot.push_str(&piece[slot.len()..]);
        return;
    }
    slot.push_str(piece);
}

impl OpenAiProvider {
    fn stream_impl(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> AppResult<()> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let mut body = WireRequest {
            model: &self.config.model,
            messages: messages.iter().map(to_wire).collect(),
            stream: true,
            temperature: self.config.temperature.clamp(0.0, 2.0),
            tools: tools
                .iter()
                .map(|t| WireTool {
                    kind: "function",
                    function: WireToolDef {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        parameters: t.parameters.clone(),
                    },
                })
                .collect(),
            tool_choice: if tools.is_empty() { None } else { Some("auto") },
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
            reasoning_effort: self.config.thinking.effort(),
        };

        let send = |body: &WireRequest| {
            ureq::post(&url)
                .header("Authorization", &format!("Bearer {}", self.config.api_key))
                .header("Content-Type", "application/json")
                .send_json(body)
        };

        // 可选字段（思考参数 / 用量流选项）都可能被网关当成非法参数回 400。
        // 不要为了可选项把整轮对话搞挂：先去掉思考参数重试，再去掉 stream_options。
        let resp = loop {
            match send(&body) {
                Ok(r) => break r,
                Err(ureq::Error::StatusCode(400)) if body.reasoning_effort.is_some() => {
                    body.reasoning_effort = None;
                }
                Err(ureq::Error::StatusCode(400)) if body.stream_options.is_some() => {
                    body.stream_options = None;
                }
                Err(ureq::Error::StatusCode(code)) => {
                    // 尽力把服务端的错误正文带出来，否则用户只会看到 HTTP 401 不知道原因
                    on_event(StreamEvent::Error(format!("HTTP {code}")));
                    return Ok(());
                }
                Err(e) => {
                    on_event(StreamEvent::Error(format!("请求失败：{e}")));
                    return Ok(());
                }
            }
        };

        // 按 index 累积工具调用碎片
        let mut calls: BTreeMap<usize, PartialCall> = BTreeMap::new();
        let mut saw_tool_calls = false;
        // 用量在流末尾单独一个 chunk 里，且那时 choices 是空数组
        let mut round_usage: Option<TokenUsage> = None;

        let reader = resp.into_body().into_reader();
        crate::sse::for_each_data(reader, |data| {
            if data == "[DONE]" {
                return;
            }
            let Ok(v) = serde_json::from_str::<Value>(data) else {
                return;
            };

            // 服务端错误对象
            if let Some(err) = v.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("未知错误");
                on_event(StreamEvent::Error(msg.to_string()));
                return;
            }

            // 用量：必须在取 choices 之前读 —— 带 usage 的那个 chunk 里 choices 为空。
            if let Some(u) = parse_usage_frame(data) {
                round_usage = Some(u);
            }

            let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else {
                return;
            };

            if let Some(delta) = choice.get("delta") {
                // 文本
                if let Some(text) = delta.get("content").and_then(|t| t.as_str()) {
                    if !text.is_empty() {
                        on_event(StreamEvent::Delta(text.to_string()));
                    }
                }
                // 推理模型（如 deepseek-reasoner）会先吐 reasoning_content，忽略它
                // 但保持连接继续读，避免误判为空回复。

                // 工具调用碎片
                if let Some(arr) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                    saw_tool_calls = true;
                    for item in arr {
                        let idx = item
                            .get("index")
                            .and_then(|i| i.as_u64())
                            .unwrap_or(0) as usize;
                        let slot = calls.entry(idx).or_default();
                        if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                            if !id.is_empty() {
                                slot.id = id.to_string();
                            }
                        }
                        if let Some(f) = item.get("function") {
                            if let Some(n) = f.get("name").and_then(|x| x.as_str()) {
                                accumulate(&mut slot.name, n);
                            }
                            if let Some(a) = f.get("arguments").and_then(|x| x.as_str()) {
                                accumulate(&mut slot.arguments, a);
                            }
                        }
                    }
                }
            }

            // finish_reason 出现即本轮结束
            if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                if reason == "tool_calls" || (saw_tool_calls && reason == "stop") {
                    let finished: Vec<ToolCall> = std::mem::take(&mut calls)
                        .into_iter()
                        .filter(|(_, c)| !c.name.is_empty())
                        .map(|(i, c)| ToolCall {
                            id: if c.id.is_empty() {
                                format!("call_{i}")
                            } else {
                                c.id
                            },
                            name: c.name,
                            arguments: c.arguments,
                        })
                        .collect();
                    if !finished.is_empty() {
                        on_event(StreamEvent::ToolCalls(finished));
                    }
                }
            }
        });

        // 有些服务端不发 finish_reason，靠流结束兜底
        if !calls.is_empty() {
            let finished: Vec<ToolCall> = calls
                .into_iter()
                .filter(|(_, c)| !c.name.is_empty())
                .map(|(i, c)| ToolCall {
                    id: if c.id.is_empty() {
                        format!("call_{i}")
                    } else {
                        c.id
                    },
                    name: c.name,
                    arguments: c.arguments,
                })
                .collect();
            if !finished.is_empty() {
                on_event(StreamEvent::ToolCalls(finished));
            }
        }

        if let Some(u) = round_usage {
            on_event(StreamEvent::Usage(u));
        }
        on_event(StreamEvent::Done);
        Ok(())
    }
}

impl LlmProvider for OpenAiProvider {
    fn id(&self) -> &'static str {
        "openai"
    }

    fn stream_chat(
        &self,
        messages: &[ChatMessage],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> AppResult<()> {
        self.stream_impl(messages, &[], on_event)
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn stream_chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> AppResult<()> {
        self.stream_impl(messages, tools, on_event)
    }
}

/// 从一帧 SSE 数据里取真实用量。
///
/// OpenAI 把 usage 放在流的最后一个 chunk，而那个 chunk 的 `choices` 是空数组：
/// 先取 choices 再处理 usage 的写法会把它整条丢掉，用量永远是 0。
pub(crate) fn parse_usage_frame(data: &str) -> Option<TokenUsage> {
    let v: Value = serde_json::from_str(data).ok()?;
    let u = v.get("usage").filter(|u| u.is_object())?;
    let input = u
        .get("prompt_tokens")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let output = u
        .get("completion_tokens")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    if input == 0 && output == 0 {
        return None;
    }
    Some(TokenUsage::new(input, output))
}

/// 给测试用：把内部消息转成请求体，便于断言字段。
///
/// 与实际请求保持一致（包括 `stream_options`）：形状对不上，就只会测出一个
/// 线上根本不存在的请求。
#[cfg(test)]
pub(crate) fn debug_request_body(
    model: &str,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    thinking: neverwork_core::ThinkingLevel,
) -> Value {
    use serde_json::json;
    let body = WireRequest {
        model,
        messages: messages.iter().map(to_wire).collect(),
        stream: true,
        temperature: 0.7,
        tools: tools
            .iter()
            .map(|t| WireTool {
                kind: "function",
                function: WireToolDef {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect(),
        tool_choice: if tools.is_empty() { None } else { Some("auto") },
        stream_options: Some(StreamOptions {
            include_usage: true,
        }),
        reasoning_effort: thinking.effort(),
    };
    serde_json::to_value(&body).unwrap_or(json!({}))
}

#[cfg(test)]
mod usage_tests {
    use super::{debug_request_body, parse_usage_frame};
    use crate::{ChatMessage, TokenUsage};
    use neverwork_core::ThinkingLevel;

    #[test]
    fn request_asks_for_stream_usage() {
        // 不带 `stream_options.include_usage`，大多数网关不在流里回 usage，
        // token 预算与用量记账就全是 0。
        let body = debug_request_body("gpt-4o", &[ChatMessage::user("hi")], &[], ThinkingLevel::Auto);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn reasoning_effort_only_when_level_chosen() {
        let auto = debug_request_body("o3", &[ChatMessage::user("hi")], &[], ThinkingLevel::Auto);
        assert!(auto.get("reasoning_effort").is_none(), "自动档不能发 reasoning_effort");
        let high = debug_request_body("o3", &[ChatMessage::user("hi")], &[], ThinkingLevel::High);
        assert_eq!(high["reasoning_effort"], "high");
    }

    #[test]
    fn usage_with_empty_choices_is_parsed() {
        // 真实形状：最后一个 chunk 的 choices 是空数组
        let frame = r#"{"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":34}}"#;
        assert_eq!(parse_usage_frame(frame), Some(TokenUsage::new(12, 34)));
    }

    #[test]
    fn missing_or_zero_usage_is_none() {
        assert_eq!(parse_usage_frame(r#"{"choices":[{"delta":{}}]}"#), None);
        assert_eq!(
            parse_usage_frame(r#"{"usage":{"prompt_tokens":0,"completion_tokens":0}}"#),
            None,
            "全 0 的用量当作没报，避免把预算记成 0"
        );
        assert_eq!(parse_usage_frame("[DONE]"), None);
    }
}

#[cfg(test)]
mod accumulate_tests {
    use super::accumulate;

    #[test]
    fn incremental_fragments_are_appended() {
        // OpenAI 规范行为：name 首片完整，arguments 被切成碎片
        let mut name = String::new();
        accumulate(&mut name, "read_file");
        assert_eq!(name, "read_file");

        let mut args = String::new();
        accumulate(&mut args, "{\"path\":");
        accumulate(&mut args, "\"src/lib.rs\"}");
        assert_eq!(args, "{\"path\":\"src/lib.rs\"}");
    }

    #[test]
    fn repeated_full_value_is_not_duplicated() {
        // 实测踩到过：兼容网关每片都重发完整 name，拼成 read_fileread_file
        let mut name = String::new();
        for _ in 0..3 {
            accumulate(&mut name, "read_file");
        }
        assert_eq!(name, "read_file");
    }

    #[test]
    fn cumulative_value_replaces_prefix() {
        // 有的网关发的不是增量而是「累计值」
        let mut name = String::new();
        for piece in ["r", "re", "rea", "read"] {
            accumulate(&mut name, piece);
        }
        assert_eq!(name, "read");

        let mut args = String::new();
        accumulate(&mut args, "{\"a\":");
        accumulate(&mut args, "{\"a\":\"b\"}");
        assert_eq!(args, "{\"a\":\"b\"}");
    }

    #[test]
    fn repeated_text_inside_arguments_is_kept() {
        // 判重不能用「已累积值以碎片结尾」：`"a":"a` 的尾部会跟下一片重叠
        let mut args = String::new();
        accumulate(&mut args, "{\"a\":\"a");
        accumulate(&mut args, "a\"}");
        assert_eq!(args, "{\"a\":\"aa\"}");
    }

    #[test]
    fn empty_piece_is_ignored() {
        let mut s = String::from("keep");
        accumulate(&mut s, "");
        assert_eq!(s, "keep");
    }
}
