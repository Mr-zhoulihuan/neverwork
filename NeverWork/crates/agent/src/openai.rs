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

use crate::{ChatMessage, LlmProvider, ProviderConfig, Role, StreamEvent, ToolCall, ToolSpec};

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
struct WireRequest<'a> {
    model: &'a str,
    messages: Vec<WireMessage>,
    stream: bool,
    temperature: f32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
}

#[derive(Serialize)]
struct WireTool {
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireToolDef,
}

#[derive(Serialize)]
struct WireToolDef {
    name: &'static str,
    description: &'static str,
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
        let body = WireRequest {
            model: &self.config.model,
            messages: messages.iter().map(to_wire).collect(),
            stream: true,
            temperature: self.config.temperature.clamp(0.0, 2.0),
            tools: tools
                .iter()
                .map(|t| WireTool {
                    kind: "function",
                    function: WireToolDef {
                        name: t.name,
                        description: t.description,
                        parameters: t.parameters.clone(),
                    },
                })
                .collect(),
            tool_choice: if tools.is_empty() { None } else { Some("auto") },
        };

        let resp = match ureq::post(&url)
            .header("Authorization", &format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .send_json(&body)
        {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(code)) => {
                // 尽力把服务端的错误正文带出来，否则用户只会看到 HTTP 401 不知道原因
                on_event(StreamEvent::Error(format!("HTTP {code}")));
                return Ok(());
            }
            Err(e) => {
                on_event(StreamEvent::Error(format!("请求失败：{e}")));
                return Ok(());
            }
        };

        // 按 index 累积工具调用碎片
        let mut calls: BTreeMap<usize, PartialCall> = BTreeMap::new();
        let mut saw_tool_calls = false;

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
                                if !n.is_empty() {
                                    slot.name.push_str(n);
                                }
                            }
                            if let Some(a) = f.get("arguments").and_then(|x| x.as_str()) {
                                slot.arguments.push_str(a);
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

/// 给测试用：把内部消息转成请求体，便于断言字段。
#[cfg(test)]
pub(crate) fn debug_request_body(model: &str, messages: &[ChatMessage], tools: &[ToolSpec]) -> Value {
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
                    name: t.name,
                    description: t.description,
                    parameters: t.parameters.clone(),
                },
            })
            .collect(),
        tool_choice: if tools.is_empty() { None } else { Some("auto") },
    };
    serde_json::to_value(&body).unwrap_or(json!({}))
}
