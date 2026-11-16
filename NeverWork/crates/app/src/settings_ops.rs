//! 设置相关行为：改设置、切主题、表单字段编辑、技能安装、升级检查、配置导出导入。
//!
//! 视图在 `views/settings.rs` 与 `views/skills.rs`，这里只负责状态变更与落盘。

use std::path::PathBuf;

use gpui::Context;

use neverwork_core::{
    config, AppResult, ProviderEntry, ProviderPreset, Settings, Skill, ThemeMode,
};

use crate::state::{AppState, CenterView, FormField, InputTarget, SettingsTab};

impl AppState {
    // ===== 打开 / 关闭 =====

    pub(crate) fn open_settings(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        self.center = CenterView::Settings;
        self.settings_tab = tab;
        self.active_field = None;
        self.action_msg = None;
        self.palette_open = false;
        if tab == SettingsTab::Sync && self.update_status.is_none() {
            self.check_update(cx);
        }
        cx.notify();
    }

    pub(crate) fn open_skills(&mut self, cx: &mut Context<Self>) {
        self.center = CenterView::Skills;
        self.active_field = None;
        self.action_msg = None;
        self.palette_open = false;
        cx.notify();
    }

    pub(crate) fn set_settings_tab(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        self.settings_tab = tab;
        self.active_field = None;
        self.action_msg = None;
        // 进入「同步与升级」时自动查一次版本，省得用户还要再点一下。
        if tab == SettingsTab::Sync && self.update_status.is_none() {
            self.check_update(cx);
        }
        cx.notify();
    }

    /// 保存设置到 `~/.neverwork/settings.json`。
    pub(crate) fn persist_settings(&mut self) -> AppResult<()> {
        config::save_settings(&self.settings)?;
        // 开了同步就顺手留一份带时间戳的备份（默认脱敏）。
        if self.settings.sync_enabled {
            self.settings.last_sync_at = config::now_secs();
            let with_secrets = self.export_with_secrets;
            if let Some(path) =
                write_snapshot_with(&self.config, &self.settings, with_secrets).ok()
            {
                self.action_msg = Some(format!("已同步到 {}", short_path(&path)));
            }
        }
        Ok(())
    }

    /// 改设置并立即落盘（设置项都是即时生效的，没有"应用"按钮）。
    pub(crate) fn update_settings(
        &mut self,
        f: impl FnOnce(&mut Settings),
        cx: &mut Context<Self>,
    ) {
        f(&mut self.settings);
        match self.persist_settings() {
            Ok(()) => {
                if self.action_msg.is_none() {
                    self.action_msg = Some("已保存".into());
                }
            }
            Err(e) => self.action_msg = Some(format!("保存失败：{e}")),
        }
        self.ai_ready =
            self.settings.usable_provider_count() > 0 || neverwork_agent::config_from_env().is_some();
        cx.notify();
    }

    // ===== 主题 / 外观 =====

    pub(crate) fn set_theme(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        neverwork_ui::theme::set_theme_mode(mode);
        self.update_settings(|s| s.theme = mode, cx);
    }

    // ===== 编辑器相关（被编辑器视图读取） =====

    pub(crate) fn editor_font_size(&self) -> f32 {
        self.settings.editor_font_size.clamp(9.0, 24.0)
    }

    pub(crate) fn line_height(&self) -> f32 {
        (self.editor_font_size() + 8.0).round()
    }

    pub(crate) fn indent_str(&self) -> String {
        self.settings.indent()
    }

    // ===== 表单字段 =====

    /// 开始编辑某个表单字段：把当前值装进缓冲区。
    pub(crate) fn begin_field(&mut self, field: FormField, cx: &mut Context<Self>) {
        let current = self.field_value(field);
        self.active_field = Some(field);
        self.input_target = InputTarget::Field(field);
        self.field_buf.reset(current);
        // 光标放到末尾，符合"点进去接着改"的预期。
        self.field_buf.move_doc_end(false);
        self.marked_range = None;
        cx.notify();
    }

    /// 提交当前字段的编辑结果到设置里。
    pub(crate) fn commit_field(&mut self, cx: &mut Context<Self>) {
        let Some(field) = self.active_field else { return };
        let value = self.field_buf.text().to_string();
        self.apply_field(field, value, cx);
        self.active_field = None;
        cx.notify();
    }

    /// 读字段当前值（用于装填缓冲区）。
    pub(crate) fn field_value(&self, field: FormField) -> String {
        match field {
            FormField::SkillSearch => self.skill_search.clone(),
            FormField::RoleSkillSearch => self.role_skill_search.clone(),
            FormField::ProviderKey(i) => self.provider_at(i).map(|p| p.api_key.clone()).unwrap_or_default(),
            FormField::ProviderUrl(i) => self.provider_at(i).map(|p| p.base_url.clone()).unwrap_or_default(),
            FormField::ProviderModel(i) => self.provider_at(i).map(|p| p.model.clone()).unwrap_or_default(),
            FormField::ProviderName(i) => self.provider_at(i).map(|p| p.name.clone()).unwrap_or_default(),
            FormField::ImportPath => self.import_path.clone(),
            FormField::ModelSearch => self.model_query.clone(),
            FormField::FindQuery => self.find.query.clone(),
            FormField::FindReplace => self.find.replace.clone(),
        }
    }

    fn provider_at(&self, idx: u8) -> Option<&ProviderEntry> {
        self.settings.providers.get(idx as usize)
    }

    fn apply_field(&mut self, field: FormField, value: String, cx: &mut Context<Self>) {
        match field {
            FormField::SkillSearch => {
                self.skill_search = value;
                cx.notify();
                return;
            }
            FormField::RoleSkillSearch => {
                self.role_skill_search = value;
                cx.notify();
                return;
            }
            FormField::ImportPath => {
                self.import_path = value;
                cx.notify();
                return;
            }
            FormField::ModelSearch => {
                self.model_query = value;
                self.model_highlight = 0;
                cx.notify();
                return;
            }
            FormField::FindQuery => {
                self.set_find_query(value, cx);
                return;
            }
            FormField::FindReplace => {
                self.find.replace = value;
                cx.notify();
                return;
            }
            _ => {}
        }
        let Some(idx) = field.index().map(|i| i as usize) else {
            return;
        };
        let Some(p) = self.settings.providers.get_mut(idx) else {
            return;
        };
        match field {
            FormField::ProviderKey(_) => p.api_key = value.trim().to_string(),
            FormField::ProviderUrl(_) => p.base_url = value.trim().to_string(),
            FormField::ProviderModel(_) => {
                let v = value.trim().to_string();
                p.model = v.clone();
                // 用户手输的模型也进候选列表，方便下次直接选。
                if !v.is_empty() && !p.models.iter().any(|m| m == &v) {
                    p.models.insert(0, v);
                }
            }
            FormField::ProviderName(_) => p.name = value.trim().to_string(),
            _ => {}
        }
        let ok = self.persist_settings();
        self.action_msg = Some(match ok {
            Ok(()) => "已保存".into(),
            Err(e) => format!("保存失败：{e}"),
        });
        cx.notify();
    }

    // ===== 厂商 =====

    pub(crate) fn set_active_provider(&mut self, id: String, cx: &mut Context<Self>) {
        self.update_settings(|s| s.active_provider_id = id, cx);
    }

    pub(crate) fn toggle_provider(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(p) = self.settings.providers.get_mut(idx) {
            p.enabled = !p.enabled;
        }
        self.update_settings(|_| {}, cx);
    }


    /// 加一个自定义厂商。
    pub(crate) fn add_custom_provider(&mut self, cx: &mut Context<Self>) {
        let id = self.settings.next_custom_id();
        let mut p = ProviderEntry::from_preset(ProviderPreset::Custom);
        p.id = id.clone();
        p.name = format!("自定义厂商 {}", id.trim_start_matches("custom-"));
        p.enabled = true;
        p.models = vec!["gpt-4o".into()];
        p.model = "gpt-4o".into();
        self.settings.providers.push(p);
        self.update_settings(|_| {}, cx);
        self.action_msg = Some("已添加自定义厂商，填入地址与 Key 即可用".into());
        cx.notify();
    }

    pub(crate) fn remove_provider(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.settings.providers.len() {
            return;
        }
        // 内置厂商不允许删除，只允许关掉；自定义可以删。
        if self.settings.providers[idx].preset != ProviderPreset::Custom {
            self.action_msg = Some("内置厂商不能删除，可以用「启用」开关关掉".into());
            cx.notify();
            return;
        }
        self.settings.providers.remove(idx);
        if self.settings.active_provider_id.is_empty() && !self.settings.providers.is_empty() {
            self.settings.active_provider_id = self.settings.providers[0].id.clone();
        }
        self.update_settings(|_| {}, cx);
    }

    pub(crate) fn set_provider_temperature(&mut self, idx: usize, t: f32, cx: &mut Context<Self>) {
        if let Some(p) = self.settings.providers.get_mut(idx) {
            p.temperature = t.clamp(0.0, 2.0);
        }
        self.update_settings(|_| {}, cx);
    }

    /// 给 agent 用的当前厂商配置。
    pub(crate) fn active_provider_config(
        &self,
    ) -> Option<(String, neverwork_agent::ProviderConfig)> {
        // 环境变量优先，保持与原有 `NW_*` 行为兼容。
        if let Some((id, cfg)) = neverwork_agent::config_from_env() {
            return Some((id, cfg));
        }
        let p = self.settings.active_provider()?;
        if !p.ready() {
            return None;
        }
        let protocol = match p.protocol() {
            neverwork_core::Protocol::Anthropic => "anthropic",
            neverwork_core::Protocol::Gemini => "gemini",
            neverwork_core::Protocol::OpenAiCompatible => "openai",
        };
        Some((
            protocol.to_string(),
            neverwork_agent::ProviderConfig {
                api_key: p.api_key.clone(),
                base_url: p.base_url.clone(),
                model: p.model.clone(),
                temperature: p.temperature,
            },
        ))
    }

    // ===== 技能 =====

    pub(crate) fn set_skill_filter(
        &mut self,
        cat: Option<neverwork_core::SkillCategory>,
        cx: &mut Context<Self>,
    ) {
        self.skill_filter = cat;
        cx.notify();
    }

    pub(crate) fn set_skill_sort(&mut self, sort: u8, cx: &mut Context<Self>) {
        self.skill_sort = sort.min(2);
        cx.notify();
    }

    pub(crate) fn set_skills_tab(&mut self, tab: u8, cx: &mut Context<Self>) {
        self.skills_tab = tab.min(1);
        cx.notify();
    }

    pub(crate) fn is_skill_installed(&self, id: &str) -> bool {
        self.settings.skills.iter().any(|s| s.id == id && s.installed)
    }

    pub(crate) fn install_skill(&mut self, skill: Skill, cx: &mut Context<Self>) {
        if self.is_skill_installed(&skill.id) {
            return;
        }
        let mut s = skill.clone();
        s.installed = true;
        self.settings.skills.retain(|x| x.id != s.id);
        self.settings.skills.push(s.clone());
        let name = s.name.clone();
        self.update_settings(|_| {}, cx);
        self.action_msg = Some(format!("已安装「{name}」"));
        cx.notify();
    }

    pub(crate) fn uninstall_skill(&mut self, id: &str, cx: &mut Context<Self>) {
        let name = self
            .settings
            .skills
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        self.settings.skills.retain(|s| s.id != id);
        self.update_settings(|_| {}, cx);
        self.action_msg = Some(format!("已卸载「{name}」"));
        cx.notify();
    }

    // ===== 升级 =====

    /// 检查更新：对比本地 HEAD 与远程 `origin/main` 的提交数。
    /// 不是 git 仓库时给出明确说明，而不是假装"已是最新"。
    pub(crate) fn check_update(&mut self, cx: &mut Context<Self>) {
        self.update_status = Some("正在检查…".into());
        cx.notify();

        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .map(|p| p.to_path_buf());
        let Some(repo) = repo else {
            self.update_status = Some("无法定位仓库目录".into());
            cx.notify();
            return;
        };

        let out = std::process::Command::new("git")
            .args(["-C", &repo.to_string_lossy(), "rev-list", "--count", "HEAD..origin/main"])
            .output();

        let status = match out {
            Ok(o) if o.status.success() => {
                let n = String::from_utf8_lossy(&o.stdout).trim().to_string();
                match n.parse::<u32>() {
                    Ok(0) => format!("已是最新版本 v{}", neverwork_core::APP_VERSION),
                    Ok(n) => format!(
                        "发现 {n} 个新提交（当前 v{}），可用「拉取更新」拉到本地",
                        neverwork_core::APP_VERSION
                    ),
                    Err(_) => "无法解析提交数".to_string(),
                }
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                if err.contains("not a git repository") {
                    "当前目录不是 git 仓库，无法检查更新".to_string()
                } else if err.contains("unknown revision") || err.contains("ambiguous argument") {
                    "本地没有 origin/main，先执行 git fetch origin".to_string()
                } else {
                    format!("检查失败：{}", err.lines().next().unwrap_or("未知错误"))
                }
            }
            Err(_) => "未找到 git 命令，无法检查更新".to_string(),
        };
        self.settings.last_update_check = config::now_secs();
        let _ = self.persist_settings();
        self.update_status = Some(status);
        cx.notify();
    }

    /// 拉取更新（`git pull --ff-only`），失败时保留现场并展示原因。
    pub(crate) fn pull_update(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .map(|p| p.to_path_buf())
        else {
            self.update_status = Some("无法定位仓库目录".into());
            cx.notify();
            return;
        };
        let out = std::process::Command::new("git")
            .args(["-C", &repo.to_string_lossy(), "pull", "--ff-only"])
            .output();
        let msg = match out {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                format!("已更新：{}", text.lines().next().unwrap_or("完成"))
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                format!("更新失败：{}", err.lines().next().unwrap_or("未知错误"))
            }
            Err(_) => "未找到 git 命令".to_string(),
        };
        self.update_status = Some(msg);
        cx.notify();
    }

    // ===== 同步（本地备份） =====

    pub(crate) fn toggle_sync(&mut self, cx: &mut Context<Self>) {
        let on = !self.settings.sync_enabled;
        self.update_settings(|s| s.sync_enabled = on, cx);
        if !on {
            self.action_msg = Some("已关闭自动同步".into());
        }
        cx.notify();
    }

    pub(crate) fn sync_now(&mut self, cx: &mut Context<Self>) {
        let with_secrets = self.export_with_secrets;
        match write_snapshot_with(&self.config, &self.settings, with_secrets) {
            Ok(path) => {
                self.settings.last_sync_at = config::now_secs();
                let _ = self.persist_settings();
                self.action_msg = Some(if with_secrets {
                    format!("已导出（含密钥）到 {}", short_path(&path))
                } else {
                    format!("已导出（已脱敏）到 {}", short_path(&path))
                });
            }
            Err(e) => self.action_msg = Some(format!("导出失败：{e}")),
        }
        cx.notify();
    }

    /// 导入：读 `import_path` 指向的文件并覆盖当前配置。
    pub(crate) fn import_from_path(&mut self, cx: &mut Context<Self>) {
        let raw = self.import_path.trim().to_string();
        if raw.is_empty() {
            self.action_msg = Some("请先填写要导入的 JSON 路径".into());
            cx.notify();
            return;
        }
        let text = match std::fs::read_to_string(&raw) {
            Ok(t) => t,
            Err(e) => {
                self.action_msg = Some(format!("读取失败：{e}"));
                cx.notify();
                return;
            }
        };
        match config::import_all(&text) {
            Ok((cfg, mut st, includes_secrets)) => {
                // 脱敏导出里 Key 是空的：用本地已有的 Key 补齐，
                // 否则用户点一次导入就把自己填好的密钥全清了。
                let mut kept = 0usize;
                if !includes_secrets {
                    let before: Vec<String> = st
                        .providers
                        .iter()
                        .filter(|p| !p.api_key.trim().is_empty())
                        .map(|p| p.id.clone())
                        .collect();
                    config::merge_keeping_local_keys(&mut st, &self.settings);
                    kept = st
                        .providers
                        .iter()
                        .filter(|p| {
                            !p.api_key.trim().is_empty() && !before.contains(&p.id)
                        })
                        .count();
                }
                self.config = cfg;
                neverwork_ui::theme::set_theme_mode(st.theme);
                self.settings = st;
                let _ = config::save(&self.config);
                let _ = config::save_settings(&self.settings);
                self.ai_ready = self.settings.usable_provider_count() > 0;
                let skills = self.settings.skills.len();
                self.action_msg = Some(if includes_secrets {
                    format!(
                        "已导入：{} 个团队、{} 个厂商（含密钥）、{} 个技能",
                        self.config.teams.len(),
                        self.settings.providers.len(),
                        skills
                    )
                } else {
                    format!(
                        "已导入：{} 个团队、{} 个厂商、{} 个技能；文件不含密钥，已保留本地 {} 个 Key",
                        self.config.teams.len(),
                        self.settings.providers.len(),
                        skills,
                        kept
                    )
                });
            }
            Err(e) => self.action_msg = Some(format!("格式不对：{e}")),
        }
        cx.notify();
    }

    /// 打开同步目录（资源管理器）。
    pub(crate) fn reveal_sync_dir(&mut self, cx: &mut Context<Self>) {
        let dir = sync_dir();
        let _ = std::fs::create_dir_all(&dir);
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("explorer").arg(&dir).spawn();
        }
        self.action_msg = Some(format!("同步目录：{}", dir.display()));
        cx.notify();
    }

    /// 已有的同步快照（新的在前）。
    pub(crate) fn sync_snapshots(&self) -> Vec<(String, u64)> {
        let dir = sync_dir();
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("json") {
                    continue;
                }
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let mtime = e
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                out.push((name, mtime));
            }
        }
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out
    }
}

/// 同步目录：`~/.neverwork/sync`。
pub(crate) fn sync_dir() -> PathBuf {
    config::config_dir().join("sync")
}

/// 写一份带时间戳的快照。

/// 写快照；`include_secrets` 决定是否把 API Key 一起写进去。
pub(crate) fn write_snapshot_with(
    cfg: &neverwork_core::Config,
    st: &neverwork_core::Settings,
    include_secrets: bool,
) -> AppResult<PathBuf> {
    let dir = sync_dir();
    std::fs::create_dir_all(&dir)?;
    let stamp = format_timestamp(config::now_secs());
    let path = dir.join(format!("neverwork-{stamp}.json"));
    let opts = if include_secrets {
        config::ExportOptions::with_secrets()
    } else {
        config::ExportOptions::default()
    };
    let text = config::export_all(cfg, st, opts)?;
    std::fs::write(&path, text)?;
    Ok(path)
}

/// `20261115-154230` 形式的本地时间戳。
fn format_timestamp(secs: u64) -> String {
    // 不引 chrono，用 UTC 秒数手工拆（展示用足够）。
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant 的 days->civil 算法。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn short_path(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    if s.chars().count() > 48 {
        let tail: String = s.chars().rev().take(44).collect::<Vec<_>>().into_iter().rev().collect();
        format!("…{tail}")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_format_is_stable() {
        // 2026-11-15 15:42:30 UTC = 1794757350
        let s = format_timestamp(1_794_757_350);
        assert_eq!(s, "20261115-154230", "时间戳格式不对: {s}");
    }

    #[test]
    fn civil_from_days_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-11-15 是 epoch 起第 20772 天
        assert_eq!(civil_from_days(20_772), (2026, 11, 15));
        // 闰年边界：2024-02-29
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[test]
    fn short_path_truncates_long() {
        let long = "a".repeat(80);
        let p = std::path::Path::new(&long);
        let s = short_path(p);
        assert!(s.starts_with('…'));
        assert!(s.chars().count() <= 45);
    }
}
