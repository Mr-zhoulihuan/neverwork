//! 本地配置持久化：团队列表保存到用户目录下的 JSON 文件。
//!
//! 本地优先：使用可移植的 JSON，不引入数据库。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{AppError, AppResult, Settings, Team};

/// 持久化的配置根。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub teams: Vec<Team>,
    /// 当前选中的团队 id。
    pub active_team_id: String,
}

impl Default for Config {
    fn default() -> Self {
        let team = Team::default_team();
        Self {
            active_team_id: team.id.clone(),
            teams: vec![team],
        }
    }
}

/// 配置目录：`~/.neverwork`，可用 `NW_CONFIG_DIR` 覆盖。
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NW_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".neverwork")
}

fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

/// 加载配置；不存在则返回默认。
pub fn load() -> Config {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

/// 原子保存配置（临时文件 + rename）。
pub fn save(config: &Config) -> AppResult<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir).map_err(AppError::Io)?;
    let path = config_path();
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(config)
        .map_err(|e| AppError::Config(e.to_string()))?;
    std::fs::write(&tmp, text).map_err(AppError::Io)?;
    std::fs::rename(&tmp, &path).map_err(AppError::Io)?;
    Ok(())
}

impl Config {
    /// 当前活动团队。
    pub fn active_team(&self) -> Option<&Team> {
        self.teams.iter().find(|t| t.id == self.active_team_id).or_else(|| self.teams.first())
    }
}

// ============ 设置（settings.json） ============

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

/// 加载设置；文件不存在或损坏时返回默认值。
pub fn load_settings() -> Settings {
    match std::fs::read_to_string(settings_path()) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

/// 原子保存设置。
pub fn save_settings(settings: &Settings) -> AppResult<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir).map_err(AppError::Io)?;
    let path = settings_path();
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| AppError::Config(e.to_string()))?;
    std::fs::write(&tmp, text).map_err(AppError::Io)?;
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    std::fs::rename(&tmp, &path).map_err(AppError::Io)?;
    Ok(())
}

/// 导出全部配置（团队 + 设置）为一段 JSON，用于同步/备份。
pub fn export_all(config: &Config, settings: &Settings) -> AppResult<String> {
    #[derive(serde::Serialize)]
    struct Bundle<'a> {
        version: &'a str,
        exported_at: u64,
        config: &'a Config,
        settings: &'a Settings,
    }
    serde_json::to_string_pretty(&Bundle {
        version: crate::APP_VERSION,
        exported_at: now_secs(),
        config,
        settings,
    })
    .map_err(|e| AppError::Config(e.to_string()))
}

/// 从导出的 JSON 恢复；返回 (config, settings)。
pub fn import_all(text: &str) -> AppResult<(Config, Settings)> {
    #[derive(serde::Deserialize)]
    struct Bundle {
        #[serde(default)]
        config: Option<Config>,
        #[serde(default)]
        settings: Option<Settings>,
    }
    let b: Bundle = serde_json::from_str(text).map_err(|e| AppError::Config(e.to_string()))?;
    Ok((b.config.unwrap_or_default(), b.settings.unwrap_or_default()))
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
