//! NeverWork 核心类型与错误。

use thiserror::Error;

/// 应用统一错误类型。
#[derive(Debug, Error)]
pub enum AppError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("配置错误: {0}")]
    Config(String),

    #[error("未知错误: {0}")]
    Other(String),
}

pub type AppResult<T> = Result<T, AppError>;

pub mod config;
pub mod model;
pub mod sessions;
pub mod settings;
pub use config::{Config, ExportOptions};
pub use model::*;
pub use sessions::SessionStore;
pub use settings::{Settings, ThemeMode, ProviderPreset, ProviderKind, ProviderEntry, ModelEntry, Protocol, Skill, SkillKind, SkillCategory, McpServerConfig, APP_VERSION};
