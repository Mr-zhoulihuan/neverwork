//! NeverWork Agent：LLM Provider 抽象、流式对话与工具调用。
//!
//! 设计：统一内部消息模型 → 各 Provider 适配 → 通过回调逐块流式输出。
//! 使用 `ureq`（阻塞 + 流式读取）在后台线程运行，避免与 GPUI 的异步执行器冲突。

mod agent_loop;
pub mod multi_patch;
mod anthropic;
mod gemini;
pub mod diff;
mod openai;
mod sse;
mod supervisor;
mod team;
pub mod tools;
#[cfg(test)]
mod tests;

pub use agent_loop::{run_agent, AgentEvent};
pub use openai::OpenAiProvider;
pub use anthropic::AnthropicProvider;
pub use gemini::GeminiProvider;
pub use supervisor::{parse_decision, Decision};
pub use team::{run_team, TeamEvent};
pub use tools::{ToolCall, ToolResult, ToolSpec};

use neverwork_core::AppResult;

/// 对话角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    /// 工具执行结果（回填给模型）。
    Tool,
}

/// 一条对话消息（内部统一格式）。
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// 助手发起的工具调用（仅 `Role::Assistant` 可能非空）。
    pub tool_calls: Vec<ToolCall>,
    /// 工具结果消息对应的调用 id（仅 `Role::Tool` 使用）。
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    fn base(role: Role, content: String) -> Self {
        Self {
            role,
            content,
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn system(s: impl Into<String>) -> Self {
        Self::base(Role::System, s.into())
    }
    pub fn user(s: impl Into<String>) -> Self {
        Self::base(Role::User, s.into())
    }
    pub fn assistant(s: impl Into<String>) -> Self {
        Self::base(Role::Assistant, s.into())
    }

    /// 带工具调用的助手消息。
    pub fn assistant_with_tools(content: impl Into<String>, calls: Vec<ToolCall>) -> Self {
        let mut m = Self::base(Role::Assistant, content.into());
        m.tool_calls = calls;
        m
    }

    /// 工具结果消息。
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        let mut m = Self::base(Role::Tool, content.into());
        m.tool_call_id = Some(call_id.into());
        m
    }
}

/// 流式响应的一个增量事件。
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// 文本增量。
    Delta(String),
    /// 模型请求调用这些工具（本轮文本已结束）。
    ToolCalls(Vec<ToolCall>),
    /// 结束。
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

    /// 发起流式对话（不带工具）。`on_event` 会在每个增量时被调用（在调用线程上）。
    fn stream_chat(
        &self,
        messages: &[ChatMessage],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> AppResult<()>;

    /// 是否支持工具调用。
    fn supports_tools(&self) -> bool {
        false
    }

    /// 带工具的流式对话。不支持工具的 Provider 默认退化为普通对话。
    fn stream_chat_with_tools(
        &self,
        messages: &[ChatMessage],
        _tools: &[ToolSpec],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> AppResult<()> {
        self.stream_chat(messages, on_event)
    }
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
        "gemini" => Box::new(GeminiProvider::new(config)),
        _ => Box::new(OpenAiProvider::new(config)),
    }
}
