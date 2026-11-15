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
pub use config::Config;
pub use model::*;
