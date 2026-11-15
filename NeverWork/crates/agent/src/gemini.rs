//! Google Gemini 原生 Provider（`generateContent` / `streamGenerateContent`）。
//!
//! 与 OpenAI 兼容路径的差别（所以必须单独实现，不能凑合）：
//! - 端点是 `/models/{model}:streamGenerateContent?alt=sse`
//! - 消息结构是 `contents: [{role, parts: [{text}]}]`，role 只有 `user` / `model`
//! - system prompt 单独放在 `systemInstruction`
//! - 流式返回仍是 SSE，但负载是完整 `candidates[0].content.parts[0].text` 增量
//! - 认证用 `x-goog-api-key` 头，不是 Bearer
//!
//! 工具调用（functionDeclarations）暂不接：Gemini 的 function calling 参数结构
//! 与 OpenAI 不同，混着做容易出错；纯对话先跑通，工具留给后续单独一轮。

use serde::Serialize;
use serde_json::Value;

use neverwork_core::AppResult;

use crate::{ChatMessage, LlmProvider, ProviderConfig, Role, StreamEvent, TokenUsage, ToolSpec};

pub struct GeminiProvider {
    config: ProviderConfig,
}

impl GeminiProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self { config }
    }
}

#[derive(Serialize)]
struct Part {
    text: String,
}

#[derive(Serialize)]
struct Content {
    /// Gemini 只认 `user` / `model`。
    role: &'static str,
    parts: Vec<Part>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SystemInstruction {
    parts: Vec<Part>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRequest {
    contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<SystemInstruction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    temperature: f32,
    max_output_tokens: u32,
}

/// 把内部消息拆成 (system 文本, contents)。
fn to_wire(messages: &[ChatMessage]) -> (Option<String>, Vec<Content>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut contents: Vec<Content> = Vec::new();

    for m in messages {
        match m.role {
            Role::System => {
                if !m.content.trim().is_empty() {
                    system_parts.push(m.content.clone());
                }
            }
            Role::User | Role::Tool => {
                // Tool 结果对 Gemini 来说也是用户侧输入
                contents.push(Content {
                    role: "user",
                    parts: vec![Part {
                        text: m.content.clone(),
                    }],
                });
            }
            Role::Assistant => {
                if m.content.trim().is_empty() {
                    continue; // 纯工具调用轮次没有文本，跳过
                }
                contents.push(Content {
                    role: "model",
                    parts: vec![Part {
                        text: m.content.clone(),
                    }],
                });
            }
        }
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };
    (system, contents)
}

/// 拼出流式端点。
pub fn endpoint(base_url: &str, model: &str, api_key: &str) -> String {
    let base = base_url.trim_end_matches('/');
    // 允许用户把 base 写成 `.../v1beta` 或 `.../v1`，都拼到 models 上
    format!(
        "{base}/models/{model}:streamGenerateContent?alt=sse&key={api_key}"
    )
}

/// 从一条 SSE data 里取出文本增量。
///
/// Gemini 每帧返回的是累积到目前的完整 candidates，所以取 `parts[].text`
/// 拼接；调用方按增量语义处理即可（实测每帧只带新片段）。
pub fn extract_text(data: &str) -> Option<String> {
    let v: Value = serde_json::from_str(data).ok()?;
    let mut out = String::new();
    let cands = v.get("candidates")?.as_array()?;
    for c in cands {
        let Some(parts) = c
            .get("content")
            .and_then(|x| x.get("parts"))
            .and_then(|x| x.as_array())
        else {
            continue;
        };
        for p in parts {
            if let Some(t) = p.get("text").and_then(|x| x.as_str()) {
                out.push_str(t);
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 从流帧里取用量。
///
/// `usageMetadata` 是累计值（每帧可能都带，取最后一次即可），
/// 字段名是 camelCase：`promptTokenCount` / `candidatesTokenCount`。
pub fn extract_usage(data: &str) -> Option<TokenUsage> {
    let v: Value = serde_json::from_str(data).ok()?;
    let u = v.get("usageMetadata")?;
    let input = u
        .get("promptTokenCount")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let output = u
        .get("candidatesTokenCount")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    if input == 0 && output == 0 {
        return None;
    }
    Some(TokenUsage::new(input, output))
}

/// 从流帧里识别错误对象。
pub fn extract_error(data: &str) -> Option<String> {
    let v: Value = serde_json::from_str(data).ok()?;
    let e = v.get("error")?;
    Some(
        e.get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误")
            .to_string(),
    )
}

impl LlmProvider for GeminiProvider {
    fn id(&self) -> &'static str {
        "gemini"
    }

    fn stream_chat(
        &self,
        messages: &[ChatMessage],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> AppResult<()> {
        self.stream_impl(messages, on_event)
    }

    /// Gemini 原生支持函数调用，但参数结构与 OpenAI 不同，
    /// 这一轮先只做纯对话，避免半成品工具调用让模型行为不可预期。
    fn supports_tools(&self) -> bool {
        false
    }

    fn stream_chat_with_tools(
        &self,
        messages: &[ChatMessage],
        _tools: &[ToolSpec],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> AppResult<()> {
        self.stream_impl(messages, on_event)
    }
}

impl GeminiProvider {
    fn stream_impl(
        &self,
        messages: &[ChatMessage],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> AppResult<()> {
        let url = endpoint(&self.config.base_url, &self.config.model, &self.config.api_key);
        let (system, contents) = to_wire(messages);
        if contents.is_empty() {
            on_event(StreamEvent::Error("没有可发送的消息".into()));
            return Ok(());
        }

        let body = WireRequest {
            contents,
            system_instruction: system.map(|text| SystemInstruction {
                parts: vec![Part { text }],
            }),
            generation_config: Some(GenerationConfig {
                temperature: self.config.temperature.clamp(0.0, 2.0),
                max_output_tokens: 8192,
            }),
        };

        let resp = match ureq::post(&url)
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
        let mut usage: Option<TokenUsage> = None;
        crate::sse::for_each_data(reader, |data| {
            if data == "[DONE]" {
                return;
            }
            if let Some(err) = extract_error(data) {
                on_event(StreamEvent::Error(err));
                return;
            }
            if let Some(u) = extract_usage(data) {
                usage = Some(u);
            }
            if let Some(text) = extract_text(data) {
                on_event(StreamEvent::Delta(text));
            }
        });

        if let Some(u) = usage {
            on_event(StreamEvent::Usage(u));
        }
        on_event(StreamEvent::Done);
        Ok(())
    }
}

/// 给测试用：构造请求体。
#[cfg(test)]
pub(crate) fn debug_body(messages: &[ChatMessage], temperature: f32) -> Value {
    let (system, contents) = to_wire(messages);
    let body = WireRequest {
        contents,
        system_instruction: system.map(|text| SystemInstruction {
            parts: vec![Part { text }],
        }),
        generation_config: Some(GenerationConfig {
            temperature,
            max_output_tokens: 8192,
        }),
    };
    serde_json::to_value(&body).unwrap_or_else(|_| serde_json::Value::Object(Default::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_has_model_and_alt_sse() {
        let u = endpoint("https://generativelanguage.googleapis.com/v1beta", "gemini-2.5-pro", "K");
        assert!(u.contains("models/gemini-2.5-pro:streamGenerateContent"));
        assert!(u.contains("alt=sse"));
        assert!(u.contains("key=K"));
    }

    #[test]
    fn endpoint_trims_trailing_slash() {
        let u = endpoint("https://x/v1beta/", "m", "k");
        assert!(!u.contains("v1beta//models"), "{u}");
    }

    #[test]
    fn roles_map_to_user_and_model() {
        let msgs = vec![
            ChatMessage::user("hi"),
            ChatMessage::assistant("hello"),
            ChatMessage::user("again"),
        ];
        let v = debug_body(&msgs, 0.7);
        let contents = v.get("contents").unwrap().as_array().unwrap();
        assert_eq!(contents.len(), 3);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[1]["role"], "model", "assistant 应映射成 model");
        assert_eq!(contents[2]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "hi");
    }

    #[test]
    fn system_goes_to_system_instruction() {
        let msgs = vec![
            ChatMessage::system("你是助手"),
            ChatMessage::user("hi"),
        ];
        let v = debug_body(&msgs, 0.7);
        let contents = v.get("contents").unwrap().as_array().unwrap();
        assert_eq!(contents.len(), 1, "system 不应出现在 contents 里");
        assert_eq!(
            v["systemInstruction"]["parts"][0]["text"],
            "你是助手"
        );
    }

    #[test]
    fn multiple_system_messages_joined() {
        let msgs = vec![
            ChatMessage::system("A"),
            ChatMessage::system("B"),
            ChatMessage::user("hi"),
        ];
        let v = debug_body(&msgs, 0.7);
        let text = v["systemInstruction"]["parts"][0]["text"].as_str().unwrap();
        assert!(text.contains('A') && text.contains('B'));
    }

    #[test]
    fn empty_assistant_turn_skipped() {
        // 纯工具调用的 assistant 消息没有文本，不该发给 Gemini
        let msgs = vec![
            ChatMessage::user("hi"),
            ChatMessage::assistant(""),
            ChatMessage::user("again"),
        ];
        let v = debug_body(&msgs, 0.7);
        let contents = v.get("contents").unwrap().as_array().unwrap();
        assert_eq!(contents.len(), 2);
    }

    #[test]
    fn no_system_omits_field() {
        let msgs = vec![ChatMessage::user("hi")];
        let v = debug_body(&msgs, 0.7);
        assert!(v.get("systemInstruction").is_none());
    }

    #[test]
    fn extracts_text_from_stream_frame() {
        let data = r#"{"candidates":[{"content":{"parts":[{"text":"你好"}],"role":"model"}}]}"#;
        assert_eq!(extract_text(data).as_deref(), Some("你好"));
    }

    #[test]
    fn concatenates_multiple_parts() {
        let data = r#"{"candidates":[{"content":{"parts":[{"text":"a"},{"text":"b"}]}}]}"#;
        assert_eq!(extract_text(data).as_deref(), Some("ab"));
    }

    #[test]
    fn empty_parts_return_none() {
        let data = r#"{"candidates":[{"content":{"parts":[{"text":""}]}}]}"#;
        assert_eq!(extract_text(data), None);
    }

    #[test]
    fn malformed_json_is_none() {
        assert_eq!(extract_text("not json"), None);
        assert_eq!(extract_text(r#"{"candidates":null}"#), None);
    }

    #[test]
    fn extracts_error_message() {
        let data = r#"{"error":{"code":400,"message":"API key not valid"}}"#;
        assert_eq!(extract_error(data).as_deref(), Some("API key not valid"));
        assert_eq!(extract_error(r#"{"candidates":[]}"#), None);
    }

    #[test]
    fn extracts_usage_metadata() {
        let data = r#"{"usageMetadata":{"promptTokenCount":120,"candidatesTokenCount":34,"totalTokenCount":154}}"#;
        let u = extract_usage(data).expect("应能解析出用量");
        assert_eq!(u.input, 120);
        assert_eq!(u.output, 34);
        assert_eq!(u.total(), 154);
    }

    #[test]
    fn zero_usage_is_none() {
        let data = r#"{"usageMetadata":{"promptTokenCount":0,"candidatesTokenCount":0}}"#;
        assert_eq!(extract_usage(data), None);
        assert_eq!(extract_usage(r#"{"candidates":[]}"#), None);
    }
}
