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
    content: WireContent<'a>,
}

/// 纯文本消息用字符串（体积最小），带图时用内容块数组。
#[derive(Serialize)]
#[serde(untagged)]
enum WireContent<'a> {
    Text(&'a str),
    Blocks(Vec<WireBlock<'a>>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireBlock<'a> {
    Text { text: &'a str },
    Image { source: WireImageSource<'a> },
}

#[derive(Serialize)]
struct WireImageSource<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    media_type: &'a str,
    data: &'a str,
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
    /// 思考等级；`Auto` 时不发。
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingCfg>,
}

#[derive(Serialize)]
struct ThinkingCfg {
    #[serde(rename = "type")]
    kind: &'static str,
    budget_tokens: u32,
}

impl AnthropicProvider {
    /// 拼请求体。开思考时有两个硬约束（不满足直接 400）：
    /// - `temperature` 必须是 1
    /// - `max_tokens` 必须大于 `budget_tokens`
    fn wire<'a>(&'a self, messages: &'a [ChatMessage]) -> WireRequest<'a> {
        let system: Option<String> = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.clone())
            .reduce(|a, b| format!("{a}\n{b}"));
        let budget = self.config.thinking.budget();
        WireRequest {
            model: &self.config.model,
            max_tokens: budget.map_or(4096, |b| b + 4096),
            stream: true,
            temperature: if budget.is_some() {
                1.0
            } else {
                self.config.temperature.clamp(0.0, 2.0)
            },
            system,
            messages: messages
                .iter()
                .filter(|m| m.role != Role::System)
                .map(|m| WireMessage {
                    role: match m.role {
                        Role::Assistant => "assistant",
                        _ => "user",
                    },
                    content: if m.images.is_empty() {
                        WireContent::Text(&m.content)
                    } else {
                        let mut blocks: Vec<WireBlock> = m
                            .images
                            .iter()
                            .map(|img| WireBlock::Image {
                                source: WireImageSource {
                                    kind: "base64",
                                    media_type: &img.mime,
                                    data: &img.data_base64,
                                },
                            })
                            .collect();
                        if !m.content.is_empty() {
                            blocks.push(WireBlock::Text { text: &m.content });
                        }
                        WireContent::Blocks(blocks)
                    },
                })
                .collect(),
            thinking: budget.map(|b| ThinkingCfg {
                kind: "enabled",
                budget_tokens: b,
            }),
        }
    }
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
        let body = self.wire(messages);

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

#[cfg(test)]
mod tests {
    use super::*;
    use neverwork_core::ThinkingLevel;

    fn cfg(thinking: ThinkingLevel) -> ProviderConfig {
        ProviderConfig {
            api_key: "k".into(),
            base_url: "https://api.anthropic.com/v1".into(),
            model: "claude-sonnet-4-5".into(),
            temperature: 0.7,
            thinking,
        }
    }

    fn body(thinking: ThinkingLevel) -> Value {
        serde_json::to_value(AnthropicProvider::new(cfg(thinking)).wire(&[ChatMessage::user("hi")]))
            .unwrap()
    }

    #[test]
    fn auto_thinking_sends_nothing() {
        let v = body(ThinkingLevel::Auto);
        assert!(v.get("thinking").is_none());
        assert_eq!(v["max_tokens"], 4096);
        assert!((v["temperature"].as_f64().unwrap() - 0.7).abs() < 1e-6);
    }

    #[test]
    fn thinking_enabled_respects_anthropic_constraints() {
        let v = body(ThinkingLevel::High);
        assert_eq!(v["thinking"]["type"], "enabled");
        let budget = v["thinking"]["budget_tokens"].as_u64().unwrap();
        assert_eq!(budget, 16384);
        // 开思考时 temperature 必须为 1，max_tokens 必须大于 budget
        assert_eq!(v["temperature"], 1.0);
        assert!(v["max_tokens"].as_u64().unwrap() > budget);
    }
}
