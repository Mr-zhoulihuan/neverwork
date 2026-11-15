//! Anthropic（Claude）Provider。`/v1/messages` 流式，SSE 解析 `content_block_delta.delta.text`。

use serde::Serialize;
use serde_json::Value;

use neverwork_core::AppResult;

use crate::{ChatMessage, LlmProvider, ProviderConfig, Role, StreamEvent, TokenUsage};

pub struct AnthropicProvider {
    config: ProviderConfig,
}

impl AnthropicProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self { config }
    }
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    stream: bool,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<WireMessage<'a>>,
}

impl LlmProvider for AnthropicProvider {
    fn id(&self) -> &'static str {
        "anthropic"
    }

    fn stream_chat(
        &self,
        messages: &[ChatMessage],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> AppResult<()> {
        let url = format!(
            "{}/messages",
            self.config.base_url.trim_end_matches('/')
        );
        let system: Option<String> = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.clone())
            .reduce(|a, b| format!("{a}\n{b}"));
        let body = WireRequest {
            model: &self.config.model,
            max_tokens: 4096,
            stream: true,
            temperature: self.config.temperature.clamp(0.0, 2.0),
            system,
            messages: messages
                .iter()
                .filter(|m| m.role != Role::System)
                .map(|m| WireMessage {
                    role: match m.role {
                        Role::Assistant => "assistant",
                        _ => "user",
                    },
                    content: &m.content,
                })
                .collect(),
        };

        let resp = match ureq::post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .send_json(&body)
        {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(code)) => {
                on_event(StreamEvent::Error(format!("HTTP {code}")));
                return Ok(());
            }
            Err(e) => {
                on_event(StreamEvent::Error(format!("请求失败：{e}")));
                return Ok(());
            }
        };

        let reader = resp.into_body().into_reader();
        // Anthropic 把用量拆在两个事件里：`message_start` 给输入（含缓存字段），
        // `message_delta` 给最终输出。两边都取，最后合成一份。
        let mut usage = TokenUsage::default();
        crate::sse::for_each_data(reader, |data| {
            if data == "[DONE]" {
                on_event(StreamEvent::Done);
                return;
            }
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if ty == "content_block_delta" {
                    if let Some(text) = v
                        .get("delta")
                        .and_then(|d| d.get("text"))
                        .and_then(|t| t.as_str())
                    {
                        if !text.is_empty() {
                            on_event(StreamEvent::Delta(text.to_string()));
                        }
                    }
                } else if ty == "message_start" {
                    if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
                        usage.input = u
                            .get("input_tokens")
                            .and_then(|x| x.as_u64())
                            .unwrap_or(0);
                        // 缓存命中/写入另算，能把真实成本还原得更准
                        let cache_read = u
                            .get("cache_read_input_tokens")
                            .and_then(|x| x.as_u64())
                            .unwrap_or(0);
                        let cache_write = u
                            .get("cache_creation_input_tokens")
                            .and_then(|x| x.as_u64())
                            .unwrap_or(0);
                        usage.input += cache_read + cache_write;
                        usage.output = u
                            .get("output_tokens")
                            .and_then(|x| x.as_u64())
                            .unwrap_or(0);
                    }
                } else if ty == "message_delta" {
                    if let Some(n) = v
                        .get("usage")
                        .and_then(|u| u.get("output_tokens"))
                        .and_then(|x| x.as_u64())
                    {
                        usage.output = n;
                    }
                } else if ty == "message_stop" {
                    on_event(StreamEvent::Done);
                }
            }
        });

        if usage.total() > 0 {
            on_event(StreamEvent::Usage(usage));
        }
        on_event(StreamEvent::Done);
        Ok(())
    }
}
