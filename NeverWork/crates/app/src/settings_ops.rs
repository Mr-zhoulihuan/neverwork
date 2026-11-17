//! 设置相关行为：改设置、切主题、表单字段编辑、技能安装、升级检查、配置导出导入。
//!
//! 视图在 `views/settings.rs` 与 `views/skills.rs`，这里只负责状态变更与落盘。

use std::path::PathBuf;

use gpui::Context;

use neverwork_core::{
    config, AppResult, ProviderEntry, ProviderPreset, Settings, Skill, ThemeMode,
};

use crate::marketplace::{MarketEntry, MarketSource};
use crate::state::{AppState, CenterView, FormField, InputTarget, ProviderView, SettingsTab};

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

    /// 点击落到正在编辑的输入框外面时自动提交。
    ///
    /// 之前字段只在回车时提交：粘贴 Key 后直接点别处，编辑内容被静默丢弃，
    /// 输入框却还显示着已粘贴的文本——「看起来配好了，其实没保存」。
    /// 点击在输入框内部（挪光标、选文本）时不提交。
    pub(crate) fn commit_field_clicked_away(
        &mut self,
        pos: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.active_field.is_none() {
            return;
        }
        if let Some(b) = self.field_area_bounds.get() {
            if b.contains(&pos) {
                return;
            }
        }
        self.commit_field(cx);
    }

    /// 测试当前厂商连通性：用已保存的配置发一条最小对话请求。
    ///
    /// 网络请求在后台线程跑（`stream_chat` 是阻塞的），结果经 flume 送回 UI。
    pub(crate) fn test_provider(&mut self, idx: usize, cx: &mut Context<Self>) {
        if self.provider_test_busy {
            return;
        }
        let Some(p) = self.settings.providers.get(idx) else {
            return;
        };
        if !p.ready() {
            self.action_msg = Some("先填好 API Key 再测试".into());
            cx.notify();
            return;
        }
        let protocol = protocol_name(p.protocol());
        let model = p.effective_model();
        let config = neverwork_agent::ProviderConfig {
            api_key: p.api_key.clone(),
            base_url: p.base_url.clone(),
            model: model.clone(),
            temperature: p.temperature,
        };
        self.action_msg = Some(format!("测试中：{} …", model));
        self.provider_test_busy = true;
        cx.notify();

        let (tx, rx) = flume::unbounded::<Result<String, String>>();
        std::thread::spawn(move || {
            let provider = neverwork_agent::build_provider(&protocol, config);
            let mut first_err: Option<String> = None;
            let res = provider.stream_chat(
                &[neverwork_agent::ChatMessage::user("ping")],
                &mut |ev| {
                    if let neverwork_agent::StreamEvent::Error(e) = ev {
                        if first_err.is_none() {
                            first_err = Some(e);
                        }
                    }
                },
            );
            let outcome = match (first_err, res) {
                (Some(e), _) => Err(e),
                (None, Ok(())) => Ok(model),
                (None, Err(e)) => Err(e.to_string()),
            };
            let _ = tx.send(outcome);
        });
        cx.spawn(async move |weak, async_cx| {
            if let Ok(r) = rx.recv_async().await {
                let _ = weak.update(async_cx, |this, cx| {
                    this.provider_test_busy = false;
                    this.action_msg = Some(match r {
                        Ok(m) => format!("✓ 测试成功：模型 {} 已连通", m),
                        Err(e) => format!("✗ 测试失败：{}", e),
                    });
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// 读字段当前值（用于装填缓冲区）。
    pub(crate) fn field_value(&self, field: FormField) -> String {
        match field {
            FormField::SkillSearch => self.skill_search.clone(),
            FormField::RoleSkillSearch => self.role_skill_search.clone(),
            FormField::ProviderKey(i) => self.provider_at(i).map(|p| p.api_key.clone()).unwrap_or_default(),
            FormField::ProviderUrl(i) => self.provider_at(i).map(|p| p.base_url.clone()).unwrap_or_default(),
            FormField::ProviderName(i) => self.provider_at(i).map(|p| p.name.clone()).unwrap_or_default(),
            FormField::ProviderNewModel(_) => String::new(),
            FormField::ProviderModelFilter => self.provider_model_filter.clone(),
            FormField::ProviderSearch => self.provider_query.clone(),
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
            FormField::ProviderSearch => {
                self.provider_query = value;
                cx.notify();
                return;
            }
            FormField::ProviderModelFilter => {
                self.provider_model_filter = value;
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
        // 「手动添加模型」不是写回已有值，而是追加一条。
        if let FormField::ProviderNewModel(i) = field {
            self.add_provider_model(i as usize, &value, cx);
            return;
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

    /// 打开厂商详情。
    pub(crate) fn open_provider_detail(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.settings.providers.len() {
            return;
        }
        self.provider_view = ProviderView::Detail(idx);
        self.provider_model_filter.clear();
        self.provider_remove_confirm = None;
        self.active_field = None;
        cx.notify();
    }

    /// 从详情 / 添加面板回到厂商列表。
    pub(crate) fn close_provider_detail(&mut self, cx: &mut Context<Self>) {
        self.provider_view = ProviderView::List;
        self.provider_model_filter.clear();
        self.provider_remove_confirm = None;
        self.active_field = None;
        cx.notify();
    }

    /// 打开「添加厂商」面板。
    pub(crate) fn open_provider_add(&mut self, cx: &mut Context<Self>) {
        self.provider_view = ProviderView::Add;
        self.provider_query.clear();
        self.provider_remove_confirm = None;
        self.active_field = None;
        cx.notify();
    }

    /// 从预设添加一个厂商；已在列表里则直接跳到它的详情。
    pub(crate) fn add_provider_from_preset(
        &mut self,
        preset: ProviderPreset,
        cx: &mut Context<Self>,
    ) {
        if let Some(idx) = self
            .settings
            .providers
            .iter()
            .position(|p| p.preset == preset)
        {
            self.open_provider_detail(idx, cx);
            self.action_msg = Some("这个厂商已经在列表里了".into());
            cx.notify();
            return;
        }
        let mut p = ProviderEntry::from_preset(preset);
        if preset == ProviderPreset::Custom {
            let id = self.settings.next_custom_id();
            p.id = id.clone();
            p.name = format!("自定义厂商 {}", id.trim_start_matches("custom-"));
            p.enabled = true;
            p.models = vec![neverwork_core::ModelEntry::new("gpt-4o")];
            p.model = "gpt-4o".into();
        }
        let name = p.name.clone();
        self.settings.providers.push(p);
        self.update_settings(|_| {}, cx);
        let idx = self.settings.providers.len() - 1;
        self.provider_view = ProviderView::Detail(idx);
        self.action_msg = Some(if preset.is_local() {
            format!("已添加「{name}」，确认地址后点「拉取模型」或手动添加")
        } else {
            format!("已添加「{name}」，填入 Key 即可用")
        });
        cx.notify();
    }

    /// 移除厂商（两步：第一次点进入确认态，第二次才真删）。
    ///
    /// 已填的 Key 会随配置一起删掉，误点一下就得重新去官网申请，
    /// 所以和删会话一样要求再点一次。
    pub(crate) fn click_remove_provider(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.settings.providers.len() {
            return;
        }
        if self.provider_remove_confirm != Some(idx) {
            self.provider_remove_confirm = Some(idx);
            self.action_msg = Some(format!(
                "再点一次「确认移除」：{}的 API Key 会一起删掉",
                self.settings.providers[idx].name
            ));
            cx.notify();
            return;
        }
        self.provider_remove_confirm = None;
        let p = self.settings.providers.remove(idx);
        let removed_id = p.id.clone();
        // 会话里还指着这个厂商的，清掉回到「跟随全局」。
        for s in &mut self.sessions {
            if s.provider_id == removed_id {
                s.provider_id.clear();
                s.model.clear();
            }
        }
        if self.settings.active_provider_id == removed_id {
            self.settings.active_provider_id = self
                .settings
                .providers
                .iter()
                .find(|p| p.ready())
                .map(|p| p.id.clone())
                .unwrap_or_default();
        }
        self.provider_view = ProviderView::List;
        self.update_settings(|_| {}, cx);
        self.action_msg = Some(format!("已移除「{}」，可在添加面板重新加入", p.name));
        self.save_sessions();
        cx.notify();
    }

    pub(crate) fn set_provider_temperature(&mut self, idx: usize, t: f32, cx: &mut Context<Self>) {
        if let Some(p) = self.settings.providers.get_mut(idx) {
            p.temperature = t.clamp(0.0, 2.0);
        }
        self.update_settings(|_| {}, cx);
    }

    /// 详情页：逐条启停模型。
    pub(crate) fn toggle_provider_model(
        &mut self,
        idx: usize,
        model: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(p) = self.settings.providers.get_mut(idx) {
            p.toggle_model(model);
        }
        self.update_settings(|_| {}, cx);
    }

    /// 详情页：把某个模型设为该厂商的默认模型。
    pub(crate) fn set_provider_default_model(
        &mut self,
        idx: usize,
        model: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(p) = self.settings.providers.get_mut(idx) {
            p.select_model(model);
        }
        self.update_settings(|_| {}, cx);
    }

    /// 详情页：移除模型。
    pub(crate) fn remove_provider_model(
        &mut self,
        idx: usize,
        model: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(p) = self.settings.providers.get_mut(idx) {
            p.remove_model(model);
        }
        self.update_settings(|_| {}, cx);
    }

    /// 详情页：手动添加模型；新加的模型直接设为默认。
    pub(crate) fn add_provider_model(
        &mut self,
        idx: usize,
        model: &str,
        cx: &mut Context<Self>,
    ) {
        let id = model.trim().to_string();
        if id.is_empty() {
            return;
        }
        if let Some(p) = self.settings.providers.get_mut(idx) {
            if p.add_model(&id) {
                p.model = id.clone();
            }
        }
        self.update_settings(|_| {}, cx);
        if self.active_field == Some(FormField::ProviderNewModel(idx as u8)) {
            self.active_field = None;
        }
    }

    /// 详情页：按协议拉取厂商模型列表（后台线程，失败保留现有列表）。
    pub(crate) fn fetch_provider_models(&mut self, idx: usize, cx: &mut Context<Self>) {
        if self.provider_models_busy {
            return;
        }
        let Some(p) = self.settings.providers.get(idx) else {
            return;
        };
        if p.base_url.trim().is_empty() {
            self.action_msg = Some("先填写 API 基址".into());
            cx.notify();
            return;
        }
        if p.protocol() != neverwork_core::Protocol::Gemini && p.api_key.trim().is_empty() {
            self.action_msg = Some("先填写 API Key".into());
            cx.notify();
            return;
        }
        let protocol = protocol_name(p.protocol());
        let config = neverwork_agent::ProviderConfig {
            api_key: p.api_key.clone(),
            base_url: p.base_url.clone(),
            model: p.effective_model(),
            temperature: p.temperature,
        };
        let name = p.name.clone();
        self.provider_models_busy = true;
        self.action_msg = Some(format!("正在从 {name} 拉取模型列表…"));
        cx.notify();

        let (tx, rx) = flume::unbounded::<Result<Vec<String>, String>>();
        std::thread::spawn(move || {
            let _ = tx.send(neverwork_agent::list_models(protocol, &config));
        });
        cx.spawn(async move |weak, async_cx| {
            let Ok(result) = rx.recv_async().await else {
                return;
            };
            let _ = weak.update(async_cx, move |this, cx| {
                this.provider_models_busy = false;
                match result {
                    Ok(ids) => {
                        let total = ids.len();
                        let added = this
                            .settings
                            .providers
                            .get_mut(idx)
                            .map(|p| p.merge_models(ids))
                            .unwrap_or(0);
                        this.action_msg = Some(format!(
                            "已拉取 {total} 个模型，新增 {added} 个（已有条目保留启停设置）"
                        ));
                        this.update_settings(|_| {}, cx);
                    }
                    Err(e) => {
                        this.action_msg = Some(format!("拉取失败：{e}"));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// 指定会话实际使用的厂商：会话指定且存在就用它，否则回落全局当前厂商。
    pub(crate) fn provider_for_session(&self, session_idx: usize) -> Option<&ProviderEntry> {
        if let Some(s) = self.sessions.get(session_idx) {
            let id = s.provider_id.trim();
            if !id.is_empty() {
                if let Some(p) = self.settings.providers.iter().find(|p| p.id == id) {
                    return Some(p);
                }
            }
        }
        self.settings.active_provider()
    }

    /// 给 agent 用的指定会话厂商配置。
    pub(crate) fn provider_config_for(
        &self,
        session_idx: usize,
    ) -> Option<(String, neverwork_agent::ProviderConfig)> {
        // 环境变量优先，保持与原有 `NW_*` 行为兼容。
        if let Some((id, cfg)) = neverwork_agent::config_from_env() {
            return Some((id, cfg));
        }
        let p = self.provider_for_session(session_idx)?;
        if !p.ready() {
            return None;
        }
        let model = self
            .sessions
            .get(session_idx)
            .map(|s| s.model.trim().to_string())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| p.effective_model());
        Some((
            protocol_name(p.protocol()).to_string(),
            neverwork_agent::ProviderConfig {
                api_key: p.api_key.clone(),
                base_url: p.base_url.clone(),
                model,
                temperature: p.temperature,
            },
        ))
    }

    /// 指定会话的（厂商展示名, 模型 id）：头部 chip 与回答页脚共用。
    /// 会话没选过厂商就回落全局当前厂商，模型空则用它当前的默认模型。
    pub(crate) fn session_model_labels(&self, session_idx: usize) -> (String, String) {
        let Some(p) = self.provider_for_session(session_idx) else {
            return (String::new(), String::new());
        };
        let model = self
            .sessions
            .get(session_idx)
            .map(|s| s.model.trim().to_string())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| p.effective_model());
        (p.name.clone(), model)
    }

    // ===== 技能 =====

    pub(crate) fn set_market_tag(&mut self, tag: Option<String>, cx: &mut Context<Self>) {
        self.market_tag = tag;
        cx.notify();
    }

    pub(crate) fn set_skill_sort(&mut self, sort: u8, cx: &mut Context<Self>) {
        self.skill_sort = sort.min(2);
        cx.notify();
    }

    /// 切换 MCP / Skill 两列。
    ///
    /// 来源跟着 Tab 归一：Skill 只展示 GitHub 技能；MCP 回到上次用的
    /// MCP 来源（默认官方）。搜索词与标签随来源一起清。
    pub(crate) fn set_skills_tab(&mut self, tab: u8, cx: &mut Context<Self>) {
        let tab = tab.min(1);
        if self.skills_tab == tab {
            return;
        }
        self.skills_tab = tab;
        let target = if tab == 1 {
            MarketSource::GitHub
        } else if self.market_source == MarketSource::GitHub {
            self.mcp_source
        } else {
            self.market_source
        };
        self.set_market_source(target, cx);
        cx.notify();
    }

    /// 打开条目详情抽屉（点卡片主体触发，安装按钮不触发）。
    pub(crate) fn open_market_detail(&mut self, entry: MarketEntry, cx: &mut Context<Self>) {
        self.market_detail = Some(entry);
        // 市场页打开时底栏是安装 / 卸载，不带角色绑定。
        self.market_detail_role = None;
        cx.notify();
    }

    /// 从团队角色面板打开详情：底栏主按钮变成「绑定到角色」。
    pub(crate) fn open_market_detail_for_role(
        &mut self,
        entry: MarketEntry,
        role_idx: usize,
        cx: &mut Context<Self>,
    ) {
        self.market_detail = Some(entry);
        self.market_detail_role = Some(role_idx);
        cx.notify();
    }

    pub(crate) fn close_market_detail(&mut self, cx: &mut Context<Self>) {
        self.market_detail = None;
        self.market_detail_role = None;
        cx.notify();
    }

    /// 是否已安装：内存标记 + 磁盘文件（文本技能以磁盘为准，卸载后不会再误报）。
    pub(crate) fn is_skill_installed(&self, id: &str) -> bool {
        self.settings.skills.iter().any(|s| s.id == id && s.installed)
            || crate::skills_ops::is_installed(id)
    }

    /// 安装技能。
    ///
    /// - 文本技能：把正文落盘到 `skills/<id>/SKILL.md`，发送时整段注入提示词。
    /// - MCP 技能：先置 `pending_mcp_install` 等用户确认（安装 = 引入外部进程）。
    pub(crate) fn install_skill(&mut self, skill: Skill, cx: &mut Context<Self>) {
        if self.is_skill_installed(&skill.id) {
            return;
        }
        if skill.is_mcp() {
            let cmd = skill
                .mcp
                .as_ref()
                .map(|m| m.command.clone())
                .unwrap_or_default();
            self.pending_mcp_install = Some(skill.id.clone());
            self.notice = Some(format!(
                "「{}」是 MCP 插件：安装会写入 mcp_servers 并在首次使用时启动 `{cmd}`，需要你确认",
                skill.name
            ));
            cx.notify();
            return;
        }
        // 文本技能：真正落盘
        if let Err(e) = crate::skills_ops::install_text(&skill) {
            self.action_msg = Some(format!("安装失败：{e}"));
            cx.notify();
            return;
        }
        let mut s = skill.clone();
        s.installed = true;
        self.settings.skills.retain(|x| x.id != s.id);
        self.settings.skills.push(s.clone());
        self.update_settings(|_| {}, cx);
        self.action_msg = Some(format!(
            "已安装「{}」→ {}",
            s.name,
            crate::skills_ops::skill_file(&s.id).display()
        ));
        cx.notify();
    }

    /// 确认安装 MCP 插件：写 `mcp_servers` 并重配 agent（下次工具列表生效）。
    pub(crate) fn confirm_mcp_install(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.pending_mcp_install.take() else {
            return;
        };
        let Some(skill) = self.find_skill(&id) else {
            self.action_msg = Some("找不到该插件条目".into());
            cx.notify();
            return;
        };
        let Some(cfg) = skill.mcp.clone() else {
            self.action_msg = Some(format!("「{}」没有 MCP 启动配置", skill.name));
            cx.notify();
            return;
        };
        if !self.settings.skills.iter().any(|s| s.id == id) {
            let mut s = skill.clone();
            s.installed = true;
            self.settings.skills.push(s);
        }
        // 同名 server 只保留一份，重复安装视为覆盖启动参数
        if let Some(existing) = self.settings.mcp_servers.iter_mut().find(|m| m.name == cfg.name) {
            existing.command = cfg.command.clone();
            existing.args = cfg.args.clone();
            existing.enabled = true;
        } else {
            self.settings.mcp_servers.push(cfg.clone());
        }
        let servers = self.settings.mcp_servers.clone();
        self.update_settings(|_| {}, cx);
        neverwork_agent::mcp::configure(servers);
        self.action_msg = Some(format!(
            "已安装 MCP 插件「{}」：工具前缀 mcp__{}__，首次调用会启动外部进程并逐次审批",
            skill.name,
            cfg.sanitized()
        ));
        // 从团队编辑器发起时，装完顺手绑给那个角色
        if let Some(role_idx) = self.pending_mcp_attach.take() {
            self.attach_skill_to_role(role_idx, skill, cx);
            return;
        }
        cx.notify();
    }

    pub(crate) fn cancel_mcp_install(&mut self, cx: &mut Context<Self>) {
        self.pending_mcp_install = None;
        self.pending_mcp_attach = None;
        cx.notify();
    }

    // ===== 真实市场 =====

    /// 切换市场来源；没有缓存且没在拉时自动拉一次。
    ///
    /// 来源与 Tab 绑定：GitHub 只属于 Skill 页，其余属于 MCP 页。
    pub(crate) fn set_market_source(&mut self, source: MarketSource, cx: &mut Context<Self>) {
        if source == MarketSource::GitHub {
            self.skills_tab = 1;
        } else {
            self.skills_tab = 0;
            self.mcp_source = source;
        }
        if self.market_source == source {
            cx.notify();
            return;
        }
        self.market_source = source;
        self.skill_search.clear();
        self.market_tag = None;
        self.field_buf.reset("");
        cx.notify();
        let empty = self
            .market_entries
            .get(&source)
            .map(|e| e.is_empty())
            .unwrap_or(true);
        if empty {
            self.refresh_market(cx);
        }
    }

    /// 拉当前来源的真实条目（后台线程 + 写缓存；失败保留旧缓存）。
    pub(crate) fn refresh_market(&mut self, cx: &mut Context<Self>) {
        if self.market_loading {
            return;
        }
        let source = self.market_source;
        self.market_loading = true;
        self.market_error = None;
        cx.notify();
        let (tx, rx) =
            flume::unbounded::<(MarketSource, Result<(Vec<MarketEntry>, u64), String>)>();
        std::thread::spawn(move || {
            let r = crate::marketplace::fetch_and_cache(source);
            let _ = tx.send((source, r));
        });
        cx.spawn(async move |weak, async_cx| {
            let Ok((source, result)) = rx.recv_async().await else {
                return;
            };
            let _ = weak.update(async_cx, |this, cx| {
                this.apply_market_refresh(source, result, cx);
            });
        })
        .detach();
    }

    fn apply_market_refresh(
        &mut self,
        source: MarketSource,
        result: Result<(Vec<MarketEntry>, u64), String>,
        cx: &mut Context<Self>,
    ) {
        self.market_loading = false;
        match result {
            Ok((entries, at)) => {
                self.market_entries.insert(source, entries);
                self.market_fetched_at.insert(source, at);
                self.market_error = None;
            }
            // 有缓存就继续展示缓存，只把错误显示在状态行
            Err(e) => self.market_error = Some(e),
        }
        cx.notify();
    }

    /// 安装市场条目：文本技能直接落盘；MCP 插件先取启动配置，再进入确认流程。
    pub(crate) fn install_market_entry(&mut self, entry: MarketEntry, cx: &mut Context<Self>) {
        if entry.remote_only {
            self.action_msg = Some(
                "该条目只提供远程端点（SSE/HTTP），当前版本只支持 stdio 安装".into(),
            );
            cx.notify();
            return;
        }
        let skill = entry.skill.clone();
        if skill.is_mcp() && skill.mcp.is_none() {
            // 腾讯云等来源的列表不带启动配置：安装时再抓详情页
            let Some(mcp_id) = skill
                .id
                .strip_prefix("tencent.mcp.")
                .and_then(|s| s.parse::<u64>().ok())
            else {
                self.action_msg = Some(format!("「{}」缺少安装配置", skill.name));
                cx.notify();
                return;
            };
            if self.market_installing.is_some() {
                return;
            }
            self.market_installing = Some(skill.id.clone());
            self.notice = Some(format!("正在获取「{}」的安装配置…", skill.name));
            cx.notify();
            let (tx, rx) = flume::unbounded::<Result<neverwork_core::McpServerConfig, String>>();
            std::thread::spawn(move || {
                let _ = tx.send(crate::marketplace::fetch_tencent_detail(mcp_id));
            });
            cx.spawn(async move |weak, async_cx| {
                let Ok(result) = rx.recv_async().await else {
                    return;
                };
                let _ = weak.update(async_cx, move |this, cx| {
                    this.market_installing = None;
                    match result {
                        Ok(cfg) => {
                            let mut s = skill.clone();
                            s.mcp = Some(cfg.clone());
                            // 回写市场条目：确认条要显示真实命令
                            if let Some(list) = this.market_entries.get_mut(&MarketSource::Tencent)
                            {
                                if let Some(e) = list.iter_mut().find(|e| e.skill.id == s.id) {
                                    e.skill.mcp = Some(cfg);
                                }
                            }
                            this.install_skill(s, cx);
                        }
                        Err(e) => {
                            this.action_msg = Some(format!("获取安装配置失败：{e}"));
                            cx.notify();
                        }
                    }
                });
            })
            .detach();
            return;
        }
        self.install_skill(skill, cx);
    }

    /// 国内镜像开关：MCP 子进程启动时注入 npm / PyPI 镜像。
    pub(crate) fn toggle_mcp_mirror(&mut self, cx: &mut Context<Self>) {
        self.settings.mcp_mirror = !self.settings.mcp_mirror;
        neverwork_agent::mcp::set_mirror(self.settings.mcp_mirror);
        self.update_settings(|_| {}, cx);
        self.action_msg = Some(if self.settings.mcp_mirror {
            "已开启国内镜像：MCP 进程用 npmmirror / 清华 PyPI 拉包".into()
        } else {
            "已关闭国内镜像：MCP 进程使用系统默认源".into()
        });
        cx.notify();
    }

    /// 卸载：文本技能删目录；MCP 插件从 `mcp_servers` 移除并重配（进程随之退出）。
    pub(crate) fn uninstall_skill(&mut self, id: &str, cx: &mut Context<Self>) {
        let found = self.settings.skills.iter().find(|s| s.id == id).cloned();
        let name = found.as_ref().map(|s| s.name.clone()).unwrap_or_default();
        if let Some(skill) = &found {
            if let Some(cfg) = &skill.mcp {
                self.settings.mcp_servers.retain(|m| m.name != cfg.name);
                let servers = self.settings.mcp_servers.clone();
                neverwork_agent::mcp::configure(servers);
            }
        }
        let mut err = None;
        if crate::skills_ops::skill_dir(id).exists() {
            if let Err(e) = crate::skills_ops::uninstall_text(id) {
                err = Some(e);
            }
        }
        self.settings.skills.retain(|s| s.id != id);
        self.update_settings(|_| {}, cx);
        self.action_msg = Some(match err {
            Some(e) => format!("已清除标记，但文件删除失败：{e}"),
            None => format!("已卸载「{name}」"),
        });
        cx.notify();
    }

    /// 从本地 `.md` 导入技能：解析 front-matter → 落盘安装（同名覆盖）。
    pub(crate) fn import_skill_from_path(&mut self, cx: &mut Context<Self>) {
        let raw = self.import_path.trim().to_string();
        if raw.is_empty() {
            self.action_msg = Some("请先填写 .md 文件路径".into());
            cx.notify();
            return;
        }
        if !raw.to_ascii_lowercase().ends_with(".md") {
            self.action_msg = Some("技能导入需要 .md 文件；JSON 配置请在「设置 → 同步」导入".into());
            cx.notify();
            return;
        }
        let path = std::path::Path::new(&raw);
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                self.action_msg = Some(format!("读取失败：{e}"));
                cx.notify();
                return;
            }
        };
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "imported".into());
        let skill = crate::skills_ops::parse_md(&text, &stem);
        // 覆盖导入：先去掉旧标记，再走正常安装（会重写文件）
        self.settings.skills.retain(|s| s.id != skill.id);
        self.install_skill(skill, cx);
        self.skill_import_open = false;
        self.active_field = None;
        self.field_buf.reset("");
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

/// 协议枚举 → `neverwork_agent` 的字符串标识。
fn protocol_name(p: neverwork_core::Protocol) -> &'static str {
    match p {
        neverwork_core::Protocol::Anthropic => "anthropic",
        neverwork_core::Protocol::Gemini => "gemini",
        neverwork_core::Protocol::OpenAiCompatible => "openai",
    }
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
