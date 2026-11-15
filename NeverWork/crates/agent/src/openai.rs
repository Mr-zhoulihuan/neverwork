//! OpenAI 兼容 Provider（覆盖 OpenAI / DeepSeek / Qwen / GLM / Moonshot / Ollama 等）。
//!
//! 使用 `/chat/completions` 的流式接口（`stream: true`），SSE 解析 `choices[0].delta.content`。

use serde::Serialize;
use serde_json::Value;

use neverwork_core::AppResult;

use crate::{ChatMessage, LlmProvider, ProviderConfig, Role, StreamEvent};

pub struct OpenAiProvider {
    config: ProviderConfig,
}

impl OpenAiProvider {
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
    messages: Vec<WireMessage<'a>>,
    stream: bool,
}

fn wire_role(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
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
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let body = WireRequest {
            model: &self.config.model,
            messages: messages
                .iter()
                .map(|m| WireMessage { role: wire_role(m.role), content: &m.content })
                .collect(),
            stream: true,
        };

        let resp = match ureq::post(&url)
            .header("Authorization", &format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .send_json(&body)
        {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(code)) => {
                on_event(StreamEvent::Error(format!("HTTP {code}")));
                return Ok(());
            }
            Err(e) => {
                on_event(StreamEvent::Error(format!("request failed: {e}")));
                return Ok(());
            }
        };

        let reader = resp.into_body().into_reader();
        crate::sse::for_each_data(reader, |data| {
            if data == "[DONE]" {
                on_event(StreamEvent::Done);
                return;
            }
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                if let Some(text) = v
                    .get("choices")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("delta"))
                    .and_then(|d| d.get("content"))
                    .and_then(|t| t.as_str())
                {
                    if !text.is_empty() {
                        on_event(StreamEvent::Delta(text.to_string()));
                    }
                }
            }
        });

        on_event(StreamEvent::Done);
        Ok(())
    }
}
