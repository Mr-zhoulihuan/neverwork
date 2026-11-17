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

/// 去掉 UTF-8 BOM。Windows 记事本保存的 JSON 会带 BOM，
/// serde_json 不接受，会当成损坏文件→整份设置被静默重置成默认值。
fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// 加载配置；不存在则返回默认。
pub fn load() -> Config {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(strip_bom(&text)).unwrap_or_default(),
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
        Ok(text) => serde_json::from_str(strip_bom(&text)).unwrap_or_default(),
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

/// 导出选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportOptions {
    /// 是否把 API Key 一起导出。
    ///
    /// 默认 **false**：导出的文件常被放进云盘、发给同事、贴进聊天，
    /// 密钥一旦跟着走就收不回来了。需要连同密钥迁移时，用户必须显式勾选。
    pub include_secrets: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_secrets: false,
        }
    }
}

impl ExportOptions {
    pub fn with_secrets() -> Self {
        Self {
            include_secrets: true,
        }
    }
}

/// 导出全部配置（团队 + 设置）为一段 JSON，用于同步/备份。
///
/// 默认走脱敏：`settings.providers[].api_key` 全部清空。
pub fn export_all(config: &Config, settings: &Settings, opts: ExportOptions) -> AppResult<String> {
    #[derive(serde::Serialize)]
    struct Bundle<'a> {
        version: &'a str,
        exported_at: u64,
        /// 导出时是否包含密钥，导入侧据此判断要不要覆盖本地 Key。
        includes_secrets: bool,
        config: &'a Config,
        settings: Settings,
    }
    let settings = if opts.include_secrets {
        settings.clone()
    } else {
        settings.redacted()
    };
    serde_json::to_string_pretty(&Bundle {
        version: crate::APP_VERSION,
        exported_at: now_secs(),
        includes_secrets: opts.include_secrets,
        config,
        settings,
    })
    .map_err(|e| AppError::Config(e.to_string()))
}

/// 从导出的 JSON 恢复；返回 (config, settings, 文件里是否带密钥)。
///
/// 脱敏导出（`includes_secrets=false`）里的 Key 是空的，
/// 调用方应当用本地已有的 Key 补齐，而不是把用户已填的 Key 清掉。
pub fn import_all(text: &str) -> AppResult<(Config, Settings, bool)> {
    #[derive(serde::Deserialize)]
    struct Bundle {
        #[serde(default)]
        includes_secrets: bool,
        #[serde(default)]
        config: Option<Config>,
        #[serde(default)]
        settings: Option<Settings>,
    }
    let b: Bundle = serde_json::from_str(strip_bom(text)).map_err(|e| AppError::Config(e.to_string()))?;
    Ok((
        b.config.unwrap_or_default(),
        b.settings.unwrap_or_default(),
        b.includes_secrets,
    ))
}

/// 把导入的设置与本地设置合并：按厂商 id 保留本地 Key（导入文件没带 Key 时）。
pub fn merge_keeping_local_keys(imported: &mut Settings, local: &Settings) {
    for p in &mut imported.providers {
        if !p.api_key.trim().is_empty() {
            continue;
        }
        if let Some(local_p) = local.providers.iter().find(|x| x.id == p.id) {
            if !local_p.api_key.trim().is_empty() {
                p.api_key = local_p.api_key.clone();
            }
        }
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ThemeMode;

    #[test]
    fn bom_prefixed_settings_is_not_treated_as_corrupt() {
        let mut s = Settings::default();
        s.theme = ThemeMode::Light;
        let text = format!("\u{feff}{}", serde_json::to_string(&s).unwrap());
        let back: Settings = serde_json::from_str(strip_bom(&text)).expect("带 BOM 也要能读");
        assert_eq!(back.theme, ThemeMode::Light);
    }

    #[test]
    fn bom_prefixed_export_still_imports() {
        let text = export_all(&Config::default(), &Settings::default(), ExportOptions::default())
            .unwrap();
        let (_, back, secrets) = import_all(&format!("\u{feff}{text}")).expect("带 BOM 也要能导");
        assert!(!secrets);
        assert_eq!(back.theme, Settings::default().theme);
    }
}
