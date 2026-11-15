//! NeverWork Agent：LLM Provider 抽象与流式对话实现。
//!
//! 设计：统一内部消息模型 → 各 Provider 适配 → 通过回调逐块流式输出。
//! 使用 `ureq`（阻塞 + 流式读取）在后台线程运行，避免与 GPUI 的异步执行器冲突。

mod openai;
mod sse;
mod anthropic;
mod team;
#[cfg(test)]
mod tests;

pub use openai::OpenAiProvider;
pub use anthropic::AnthropicProvider;
pub use team::{run_team, TeamEvent};

use neverwork_core::AppResult;

/// 对话角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

/// 一条对话消息（内部统一格式）。
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn system(s: impl Into<String>) -> Self {
        Self { role: Role::System, content: s.into() }
    }
    pub fn user(s: impl Into<String>) -> Self {
        Self { role: Role::User, content: s.into() }
    }
    pub fn assistant(s: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: s.into() }
    }
}

/// 流式响应的一个增量事件。
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// 文本增量。
    Delta(String),
    /// 结束（可带结束原因）。
    Done,
    /// 错误信息（非 panic，用于 UI 友好展示）。
    Error(String),
}

/// Provider 配置。
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    /// 采样温度（0.0–2.0）。
    pub temperature: f32,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-4o".into(),
            temperature: 0.7,
        }
    }
}

/// LLM Provider 抽象。
pub trait LlmProvider: Send + Sync {
    /// Provider 名称（"openai" / "anthropic"）。
    fn id(&self) -> &'static str;
    /// 发起流式对话。`on_event` 会在每个增量时被调用（在调用线程上）。
    fn stream_chat(
        &self,
        messages: &[ChatMessage],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> AppResult<()>;
}

/// 从环境变量读取配置：
/// - `NW_PROVIDER` = openai（默认）| anthropic
/// - `NW_API_KEY`
/// - `NW_BASE_URL`（可选，缺省按 provider 给默认值）
/// - `NW_MODEL`（可选，缺省给默认模型）
pub fn config_from_env() -> Option<(String, ProviderConfig)> {
    let api_key = std::env::var("NW_API_KEY").ok()?;
    let provider = std::env::var("NW_PROVIDER").unwrap_or_else(|_| "openai".into());
    let (default_base, default_model) = match provider.as_str() {
        "anthropic" => ("https://api.anthropic.com/v1", "claude-sonnet-4-5"),
        _ => ("https://api.openai.com/v1", "gpt-4o"),
    };
    let base_url = std::env::var("NW_BASE_URL").unwrap_or_else(|_| default_base.into());
    let model = std::env::var("NW_MODEL").unwrap_or_else(|_| default_model.into());
    Some((
        provider,
        ProviderConfig {
            api_key,
            base_url,
            model,
            temperature: 0.7,
        },
    ))
}

/// 根据配置构建具体的 Provider。
pub fn build_provider(provider: &str, config: ProviderConfig) -> Box<dyn LlmProvider> {
    match provider {
        "anthropic" => Box::new(AnthropicProvider::new(config)),
        _ => Box::new(OpenAiProvider::new(config)),
    }
}
