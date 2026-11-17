use super::*;
use gpui::{img, ClickEvent};

use crate::state::{FormField, InputTarget, SettingsTab};
use crate::views::text_fit::fit_text;

impl AppState {
    pub(crate) fn team_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().flex_1().h_full().overflow_hidden().bg(theme::surface());

        // 顶部栏
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(px(38.0))
                .px_3()
                .bg(theme::panel())
                .border_b_1()
                .border_color(theme::border())
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Agent 团队编辑器"))
                .child(if let Some(n) = &self.notice {
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_md()
                        .bg(theme::green_soft())
                        .text_xs()
                        .text_color(theme::green())
                        .child(SharedString::from(n.clone()))
                } else {
                    div()
                })
                .child(div().flex_1())
                .child(
                    div()
                        .id("team-close")
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .text_xs()
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.show_chat(cx);
                        }))
                        .child("← 对话"),
                ),
        );

        let team = self.config.active_team().cloned().unwrap_or_else(neverwork_core::Team::default_team);

        let mut body = div().id("team-scroll").flex().flex_col().gap_4().px_6().py_5().flex_1().overflow_scroll();

        // ---- 团队名 + 切换下拉 + 管理操作 ----
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(40.0))
                        .rounded_lg()
                        .bg(theme::accent())
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .child("◈"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .id("team-switch")
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .px_1()
                                        .py_0p5()
                                        .rounded_md()
                                        .text_sm()
                                        .font_weight(FontWeight::BOLD)
                                        .cursor_pointer()
                                        .hover(|d| d.bg(theme::raised()))
                                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                            this.team_menu_open = !this.team_menu_open;
                                            this.team_delete_confirm = false;
                                            this.commit_role_prompt();
                                            cx.notify();
                                        }))
                                        .child(SharedString::from(team.name.clone()))
                                        .child(div().text_xs().text_color(theme::dim()).child("▾")),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::dim())
                                        .child(SharedString::from(format!("{} 个角色", team.roles.len()))),
                                ),
                        )
                        .child(div().text_xs().text_color(theme::muted()).child("协作：@ 点名或首角色先行")),
                )
                .child(
                    div()
                        .id("team-rename")
                        .flex_shrink_0()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(theme::raised())
                        .text_xs()
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::border()))
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.rename_team(window, cx);
                        }))
                        .child("重命名"),
                )
                .child(
                    div()
                        .id("team-delete")
                        .flex_shrink_0()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .text_xs()
                        .text_color(theme::red())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::border()))
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.delete_team(cx);
                        }))
                        .child(if self.team_delete_confirm { "确认删除？" } else { "删除" }),
                ),
        );

        // 重命名输入行（回车 / 失焦提交，立即写盘）
        if self.active_field == Some(FormField::TeamName) {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(52.0))
                    .child(crate::views::settings::input_box(
                        SharedString::from("team-name-input"),
                        self.field_buf.text().to_string(),
                        "团队名称",
                        true,
                        self.editor_focus.clone(),
                        self.field_area_bounds.clone(),
                        cx,
                        FormField::TeamName,
                    ))
                    .child(div().text_xs().text_color(theme::dim()).child("回车保存")),
            );
        }

        // 团队切换下拉（内联展开，不遮住下面的角色卡）
        if self.team_menu_open {
            let mut menu = div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded_lg()
                .border_1()
                .border_color(theme::border())
                .bg(theme::panel());
            for (ti, t) in self.config.teams.iter().enumerate() {
                let on = t.id == self.config.active_team_id;
                let id = t.id.clone();
                let name = t.name.clone();
                let n = t.roles.len();
                menu = menu.child(
                    div()
                        .id(SharedString::from(format!("team-option-{ti}")))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1p5()
                        .rounded_md()
                        .when(on, |d| d.bg(theme::accent_soft()))
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::raised()))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                            this.select_team(id.clone(), cx);
                        }))
                        .child(
                            div()
                                .text_sm()
                                .text_color(if on { theme::accent() } else { theme::fg() })
                                .child(SharedString::from(name)),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::dim())
                                .child(SharedString::from(format!("{n} 个角色"))),
                        ),
                );
            }
            menu = menu.child(
                div()
                    .id("team-new")
                    .flex()
                    .items_center()
                    .justify_center()
                    .py_1p5()
                    .rounded_md()
                    .border_1()
                    .border_color(theme::border())
                    .text_xs()
                    .text_color(theme::accent())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::raised()))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.add_team(window, cx);
                    }))
                    .child("＋ 新建团队"),
            );
            body = body.child(menu);
        }

        // 角色卡
        let mut roles = div().flex().flex_col().gap_2();
        for (idx, role) in team.roles.iter().enumerate() {
            roles = roles.child(self.role_edit_card(idx, role, cx));
        }
        roles = roles.child(
            div()
                .id("add-role")
                .flex()
                .items_center()
                .justify_center()
                .py_3()
                .rounded_lg()
                .border_1()
                .border_color(theme::border())
                .text_sm()
                .text_color(theme::dim())
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                    this.add_role(cx);
                }))
                .child("＋ 添加角色"),
        );
        body = body.child(roles);
        col = col.child(body);

        // 底部保存
        col = col.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_6()
                .py_3()
                .border_t_1()
                .border_color(theme::border())
                .text_xs()
                .text_color(theme::dim())
                .child(SharedString::from(format!("{} 个角色", team.roles.len())))
                .child(
                    div()
                        .id("save-team")
                        .px_4()
                        .py_1p5()
                        .rounded_md()
                        .bg(theme::accent())
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.save_team(cx);
                        }))
                        .child("保存"),
                ),
        );

        col
    }

    pub(crate) fn role_edit_card(&self, idx: usize, role: &neverwork_core::Role, cx: &mut Context<Self>) -> impl IntoElement {
        let mut card = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel());

        // ---- 顶行：头像 / 名称与模型 / 换模型 / 删除 ----
        let head = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(34.0))
                    .rounded_md()
                    .bg(theme::role_color(role.color_index))
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child(SharedString::from(role.initial.clone())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(SharedString::from(role.name.clone())))
                            .child(
                                div()
                                    .px_1()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(theme::raised())
                                    .text_xs()
                                    .text_color(theme::dim())
                                    .child(SharedString::from(role.kind_label.clone())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .text_color(theme::muted())
                            .child(div().text_color(theme::accent()).child("◆"))
                            .child(SharedString::from(format!("模型：{}", self.role_model_label(role))))
                            .child(div().flex_1())
                            .child(div().text_xs().text_color(theme::dim()).child(SharedString::from(truncate(&role.prompt, 28)))),
                    ),
            )
            .child(
                div()
                    .id(SharedString::from(format!("edit-role-{idx}")))
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(if self.role_edit_index == Some(idx) { theme::accent_soft() } else { theme::raised() })
                    .text_xs()
                    .text_color(if self.role_edit_index == Some(idx) { theme::accent() } else { theme::muted() })
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::border()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.toggle_role_edit(idx, cx);
                    }))
                    .child(if self.role_edit_index == Some(idx) { "收起" } else { "编辑" }),
            )
            .child(
                div()
                    .id(SharedString::from(format!("del-role-{idx}")))
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_xs()
                    .text_color(theme::red())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::border()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.remove_role(idx, cx);
                    }))
                    .child("删除"),
            );
        card = card.child(head);

        // ---- 插件行：已绑定插件（技能）chips + 从市场添加 ----
        // 角色绑定的是技能市场里的插件；发消息时其说明会拼进角色提示词。
        let mut plugins = div().flex().flex_wrap().items_center().gap_1p5().pl(px(46.0));
        plugins = plugins.child(div().text_xs().text_color(theme::dim()).child("插件"));
        if role.skills.is_empty() {
            plugins = plugins.child(div().text_xs().text_color(theme::dim()).child("（未绑定）"));
        }
        for id in &role.skills {
            let name = self.find_skill(id).map(|s| s.name).unwrap_or_else(|| id.clone());
            plugins = plugins.child(plugin_chip(idx, id, name, cx));
        }
        let open = self.role_skill_picker == Some(idx);
        plugins = plugins.child(
            div()
                .id(SharedString::from(format!("role-plugin-add-{idx}")))
                .px_2()
                .py_0p5()
                .rounded_full()
                .border_1()
                .border_color(if open { theme::accent_line() } else { theme::border() })
                .text_xs()
                .text_color(theme::accent())
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                    this.toggle_role_skill_picker(idx, cx);
                }))
                .child(if open { "收起插件市场" } else { "＋ 插件" }),
        );
        card = card.child(plugins);

        // ---- 内联展开的角色编辑 ----
        if self.role_edit_index == Some(idx) {
            card = card.child(self.role_edit_panel(idx, role, cx));
        }

        // ---- 展开的插件市场面板 ----
        if open {
            card = card.child(self.role_skill_panel(idx, role, cx));
        }
        card
    }

    pub(crate) fn active_team_index(&self) -> usize {
        self.config
            .teams
            .iter()
            .position(|t| t.id == self.config.active_team_id)
            .unwrap_or(0)
    }

    pub(crate) fn add_role(&mut self, cx: &mut Context<Self>) {
        // 先提交上一个角色的提示词/字段，再改列表（下标会变）。
        self.close_role_edit(cx);
        self.close_role_skill_picker(cx);
        let ti = self.active_team_index();
        let mut new_idx = None;
        if let Some(team) = self.config.teams.get_mut(ti) {
            let n = team.roles.len();
            let id = format!("role-{n}");
            let mut role = neverwork_core::Role::new(id, format!("角色{}", n + 1), "A");
            role.kind_label = "自定义".into();
            role.color_index = n % 6;
            role.prompt = "负责你分配的职责。".into();
            team.roles.push(role);
            new_idx = Some(n);
        }
        // 新角色直接展开编辑，省一次点击。
        if let Some(i) = new_idx {
            self.role_edit_index = Some(i);
            let prompt = self.edited_role().map(|r| r.prompt.clone()).unwrap_or_default();
            self.role_prompt_buf.reset(prompt);
            self.role_prompt_buf.move_doc_end(false);
        }
        cx.notify();
    }

    pub(crate) fn remove_role(&mut self, idx: usize, cx: &mut Context<Self>) {
        self.close_role_edit(cx);
        self.close_role_skill_picker(cx);
        let ti = self.active_team_index();
        if let Some(team) = self.config.teams.get_mut(ti) {
            if team.roles.len() > 1 && idx < team.roles.len() {
                team.roles.remove(idx);
            } else {
                self.notice = Some("至少保留一个角色".into());
            }
        }
        cx.notify();
    }

    /// 当前正在内联编辑的角色。
    pub(crate) fn edited_role(&self) -> Option<&neverwork_core::Role> {
        let idx = self.role_edit_index?;
        self.config.active_team().and_then(|t| t.roles.get(idx))
    }

    pub(crate) fn edited_role_mut(&mut self) -> Option<&mut neverwork_core::Role> {
        let idx = self.role_edit_index?;
        let ti = self.active_team_index();
        self.config.teams.get_mut(ti).and_then(|t| t.roles.get_mut(idx))
    }

    /// 角色当前生效模型的展示标签。
    pub(crate) fn role_model_label(&self, role: &neverwork_core::Role) -> String {
        if !role.provider_id.trim().is_empty() {
            if let Some(p) = self
                .settings
                .providers
                .iter()
                .find(|p| p.id == role.provider_id && p.enabled && p.ready())
            {
                let model = if role.model.trim().is_empty() {
                    p.effective_model()
                } else {
                    role.model.clone()
                };
                return format!("{} · {}", p.name, model);
            }
        }
        if role.model.trim().is_empty() {
            "跟随会话默认".into()
        } else {
            format!("跟随会话 · {}", role.model)
        }
    }

    /// 展开 / 收起角色的内联编辑（与插件面板互斥）。
    pub(crate) fn toggle_role_edit(&mut self, idx: usize, cx: &mut Context<Self>) {
        if self.role_edit_index == Some(idx) {
            self.close_role_edit(cx);
            return;
        }
        self.close_role_edit(cx);
        let Some(role) = self.config.active_team().and_then(|t| t.roles.get(idx)) else {
            return;
        };
        let prompt = role.prompt.clone();
        self.role_edit_index = Some(idx);
        self.role_model_picker = None;
        self.role_prompt_buf.reset(prompt);
        self.role_prompt_buf.move_doc_end(false);
        self.close_role_skill_picker(cx);
        self.team_delete_confirm = false;
        cx.notify();
    }

    /// 收起编辑：先提交提示词缓冲与单行字段，再清状态。
    pub(crate) fn close_role_edit(&mut self, cx: &mut Context<Self>) {
        if self.input_target == InputTarget::RolePrompt {
            let text = self.role_prompt_buf.text().to_string();
            if !text.trim().is_empty() {
                if let Some(role) = self.edited_role_mut() {
                    role.prompt = text;
                }
            }
            self.input_target = InputTarget::Chat;
            self.marked_range = None;
        }
        if matches!(
            self.active_field,
            Some(FormField::RoleName) | Some(FormField::RoleKind) | Some(FormField::RoleInitial)
        ) {
            self.commit_field(cx);
        }
        self.role_edit_index = None;
        self.role_model_picker = None;
        self.role_prompt_buf.reset("");
        self.team_delete_confirm = false;
        cx.notify();
    }

    /// 点击提示词块：装入缓冲并切换输入目标。
    pub(crate) fn begin_role_prompt(&mut self, cx: &mut Context<Self>) {
        if self.role_edit_index.is_none() || self.input_target == InputTarget::RolePrompt {
            return;
        }
        if self.active_field.is_some() {
            self.commit_field(cx);
        }
        let text = self.edited_role().map(|r| r.prompt.clone()).unwrap_or_default();
        self.role_prompt_buf.reset(text);
        self.role_prompt_buf.move_doc_end(false);
        self.switch_input_target(InputTarget::RolePrompt);
        cx.notify();
    }

    /// 把提示词缓冲写回角色（不收起面板；保存 / 切团队前调用）。
    pub(crate) fn commit_role_prompt(&mut self) {
        if self.input_target != InputTarget::RolePrompt {
            return;
        }
        let text = self.role_prompt_buf.text().to_string();
        if !text.trim().is_empty() {
            if let Some(role) = self.edited_role_mut() {
                role.prompt = text;
            }
        }
    }

    pub(crate) fn toggle_role_model_picker(&mut self, idx: usize, cx: &mut Context<Self>) {
        if self.role_edit_index != Some(idx) {
            return;
        }
        self.role_model_picker = if self.role_model_picker == Some(idx) {
            None
        } else {
            Some(idx)
        };
        cx.notify();
    }

    /// 选中已配置模型；厂商 id 与模型 id 一起写入角色。
    pub(crate) fn select_role_model(
        &mut self,
        idx: usize,
        provider_id: String,
        model: String,
        cx: &mut Context<Self>,
    ) {
        let ti = self.active_team_index();
        if let Some(role) = self.config.teams.get_mut(ti).and_then(|t| t.roles.get_mut(idx)) {
            role.provider_id = provider_id;
            role.model = model;
        }
        self.role_model_picker = None;
        cx.notify();
    }

    // ===== 团队 CRUD =====

    /// 切换团队/删除前后的统一清场：所有面板下标都失去意义。
    fn clear_team_edit_state(&mut self) {
        self.role_skill_picker = None;
        self.role_skill_search.clear();
        self.role_skill_filter = 0;
        self.role_edit_index = None;
        self.role_model_picker = None;
        self.role_prompt_buf.reset("");
        if matches!(
            self.active_field,
            Some(FormField::RoleName)
                | Some(FormField::RoleKind)
                | Some(FormField::RoleInitial)
                | Some(FormField::RoleSkillSearch)
                | Some(FormField::TeamName)
        ) {
            self.active_field = None;
        }
        if self.input_target == InputTarget::RolePrompt {
            self.input_target = InputTarget::Chat;
        }
        self.marked_range = None;
        self.field_buf.reset("");
        self.market_detail_role = None;
        self.pending_mcp_attach = None;
    }

    pub(crate) fn select_team(&mut self, id: String, cx: &mut Context<Self>) {
        self.team_menu_open = false;
        self.team_delete_confirm = false;
        if self.config.active_team_id == id {
            cx.notify();
            return;
        }
        if self.active_field.is_some() {
            self.commit_field(cx);
        }
        self.commit_role_prompt();
        self.clear_team_edit_state();
        self.config.active_team_id = id;
        match neverwork_core::config::save(&self.config) {
            Ok(()) => {}
            Err(e) => self.notice = Some(format!("切换团队保存失败：{e}")),
        }
        cx.notify();
    }

    pub(crate) fn add_team(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if self.active_field.is_some() {
            self.commit_field(cx);
        }
        self.commit_role_prompt();
        self.clear_team_edit_state();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut id = format!("team-{now}");
        let mut n = 2;
        while self.config.teams.iter().any(|t| t.id == id) {
            id = format!("team-{now}-{n}");
            n += 1;
        }
        let team_no = self.config.teams.len() + 1;
        let mut role = neverwork_core::Role::new(format!("role-{now}"), "助手", "A");
        role.kind_label = "自定义".into();
        role.prompt = "负责你分配的职责。".into();
        role.color_index = 0;
        self.config.teams.push(neverwork_core::Team {
            id: id.clone(),
            name: format!("新团队 {team_no}"),
            flow: neverwork_core::FlowMode::Pipeline,
            roles: vec![role],
        });
        self.config.active_team_id = id;
        match neverwork_core::config::save(&self.config) {
            Ok(()) => self.notice = Some("已新建团队，接着给它起个名字".into()),
            Err(e) => self.notice = Some(format!("保存失败：{e}")),
        }
        self.team_menu_open = false;
        self.begin_field(FormField::TeamName, cx);
        window.focus(&self.editor_focus);
        cx.notify();
    }

    pub(crate) fn rename_team(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        self.team_menu_open = false;
        self.team_delete_confirm = false;
        self.begin_field(FormField::TeamName, cx);
        window.focus(&self.editor_focus);
        cx.notify();
    }

    pub(crate) fn delete_team(&mut self, cx: &mut Context<Self>) {
        self.team_menu_open = false;
        if self.config.teams.len() <= 1 {
            self.team_delete_confirm = false;
            self.notice = Some("至少保留一个团队".into());
            cx.notify();
            return;
        }
        if !self.team_delete_confirm {
            self.team_delete_confirm = true;
            cx.notify();
            return;
        }
        if self.active_field.is_some() {
            self.commit_field(cx);
        }
        self.commit_role_prompt();
        self.clear_team_edit_state();
        let ti = self.active_team_index();
        let removed = self.config.teams.remove(ti);
        self.config.active_team_id = self
            .config
            .teams
            .first()
            .map(|t| t.id.clone())
            .unwrap_or_default();
        self.team_delete_confirm = false;
        match neverwork_core::config::save(&self.config) {
            Ok(()) => self.notice = Some(format!("已删除团队「{}」", removed.name)),
            Err(e) => self.notice = Some(format!("保存失败：{e}")),
        }
        cx.notify();
    }

    pub(crate) fn save_team(&mut self, cx: &mut Context<Self>) {
        // 先把正在编辑的字段与提示词缓冲写回内存，再落盘。
        if self.active_field.is_some() {
            self.commit_field(cx);
        }
        self.commit_role_prompt();
        // 确保 active_team_id 指向存在的团队。
        if !self.config.teams.iter().any(|t| t.id == self.config.active_team_id) {
            if let Some(first) = self.config.teams.first() {
                self.config.active_team_id = first.id.clone();
            }
        }
        match neverwork_core::config::save(&self.config) {
            Ok(()) => self.notice = Some("团队已保存".into()),
            Err(e) => self.notice = Some(format!("保存失败：{e}")),
        }
        cx.notify();
    }

    /// 展开 / 收起某个角色的「插件市场」面板。
    pub(crate) fn toggle_role_skill_picker(&mut self, idx: usize, cx: &mut Context<Self>) {
        if self.role_skill_picker == Some(idx) {
            self.close_role_skill_picker(cx);
            return;
        }
        // 同一张卡上两个大面板会互相挤，编辑面板先收起来。
        if self.role_edit_index.is_some() {
            self.close_role_edit(cx);
        }
        self.role_skill_picker = Some(idx);
        self.role_skill_search.clear();
        self.role_skill_filter = 0;
        self.field_buf.reset("");
        if self.active_field == Some(FormField::RoleSkillSearch) {
            self.active_field = None;
        }
        cx.notify();
    }

    pub(crate) fn close_role_skill_picker(&mut self, cx: &mut Context<Self>) {
        self.role_skill_picker = None;
        self.role_skill_search.clear();
        self.role_skill_filter = 0;
        self.field_buf.reset("");
        if self.active_field == Some(FormField::RoleSkillSearch) {
            self.active_field = None;
        }
        cx.notify();
    }

    /// 角色卡内联编辑面板：名称 / 类型 / 首字母 / 模型下拉 / 多行提示词。
    fn role_edit_panel(&self, idx: usize, role: &neverwork_core::Role, cx: &mut Context<Self>) -> impl IntoElement {
        // 单行字段：编辑态显示 field_buf（边打边看），否则显示角色现值。
        let field_text = |field: FormField, cur: String| -> String {
            if self.active_field == Some(field) {
                self.field_buf.text().to_string()
            } else {
                cur
            }
        };
        let editing_prompt = self.input_target == InputTarget::RolePrompt;
        let label_w = px(44.0);

        let name_row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().w(label_w).flex_shrink_0().text_xs().text_color(theme::dim()).child("名称"))
            .child(crate::views::settings::input_box(
                SharedString::from(format!("role-name-input-{idx}")),
                field_text(FormField::RoleName, role.name.clone()),
                "角色名称",
                self.active_field == Some(FormField::RoleName),
                self.editor_focus.clone(),
                self.field_area_bounds.clone(),
                cx,
                FormField::RoleName,
            ));

        let kind_row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().w(label_w).flex_shrink_0().text_xs().text_color(theme::dim()).child("类型"))
            .child(crate::views::settings::input_box(
                SharedString::from(format!("role-kind-input-{idx}")),
                field_text(FormField::RoleKind, role.kind_label.clone()),
                "如：架构 / 编码 / 审查",
                self.active_field == Some(FormField::RoleKind),
                self.editor_focus.clone(),
                self.field_area_bounds.clone(),
                cx,
                FormField::RoleKind,
            ))
            .child(div().flex_shrink_0().text_xs().text_color(theme::dim()).child("首字母"))
            .child(
                div().w(px(56.0)).flex_shrink_0().child(crate::views::settings::input_box(
                    SharedString::from(format!("role-initial-input-{idx}")),
                    field_text(FormField::RoleInitial, role.initial.clone()),
                    "A",
                    self.active_field == Some(FormField::RoleInitial),
                    self.editor_focus.clone(),
                    self.field_area_bounds.clone(),
                    cx,
                    FormField::RoleInitial,
                )),
            );

        // 模型行：显示当前生效标签，点「选择」展开已配置模型下拉。
        let model_row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().w(label_w).flex_shrink_0().text_xs().text_color(theme::dim()).child("模型"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(div().text_xs().text_color(theme::accent()).child("◆"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::fg())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(SharedString::from(self.role_model_label(role))),
                    ),
            )
            .child(
                div()
                    .id(SharedString::from(format!("role-model-btn-{idx}")))
                    .flex_shrink_0()
                    .px_2()
                    .py_0p5()
                    .rounded_full()
                    .border_1()
                    .border_color(if self.role_model_picker == Some(idx) {
                        theme::accent_line()
                    } else {
                        theme::border()
                    })
                    .text_xs()
                    .text_color(theme::accent())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::panel()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.toggle_role_model_picker(idx, cx);
                    }))
                    .child("选择 ▾"),
            );

        // ---- 提示词：逐字符渲染，能看到光标，也能点任意位置改字 ----
        let prompt_text = if editing_prompt {
            self.role_prompt_buf.text().to_string()
        } else {
            role.prompt.clone()
        };
        let mut prompt_box = div()
            .id(SharedString::from(format!("role-prompt-{idx}")))
            .flex()
            .flex_row()
            .items_center()
            .flex_wrap()
            .min_h(px(64.0))
            .p_2()
            .rounded_md()
            .bg(theme::panel())
            .border_1()
            .border_color(if editing_prompt {
                theme::accent_line()
            } else {
                theme::border()
            })
            .text_xs()
            .line_height(px(18.0))
            .cursor(gpui::CursorStyle::IBeam)
            .when(editing_prompt, |d| d.track_focus(&self.editor_focus))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.begin_role_prompt(cx);
                window.focus(&this.editor_focus);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
                this.on_role_prompt_key(ev, cx);
            }));

        if prompt_text.is_empty() {
            if editing_prompt {
                prompt_box = prompt_box.child(role_caret());
            } else {
                prompt_box = prompt_box
                    .child(div().text_color(theme::dim()).child("点这里编写角色提示词"));
            }
        } else {
            let cursor = self.role_prompt_buf.cursor();
            let mut caret_drawn = !editing_prompt;
            for (i, ch) in prompt_text.char_indices() {
                if editing_prompt && cursor == i {
                    prompt_box = prompt_box.child(role_caret());
                    caret_drawn = true;
                }
                if ch == '\n' {
                    // 换行：零高整行元素在 flex_wrap 里强制断行。
                    prompt_box = prompt_box.child(div().w_full().h(px(0.0)));
                    continue;
                }
                let display: SharedString = if ch == ' ' {
                    SharedString::from("\u{00A0}")
                } else {
                    SharedString::from(ch.to_string())
                };
                prompt_box = prompt_box.child(
                    div()
                        .flex_shrink_0()
                        .text_color(theme::fg())
                        .child(display)
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, ev: &gpui::MouseDownEvent, window, cx| {
                                this.begin_role_prompt(cx);
                                this.role_prompt_buf.move_to(i, ev.modifiers.shift);
                                window.focus(&this.editor_focus);
                                cx.notify();
                            }),
                        ),
                );
            }
            if !caret_drawn {
                prompt_box = prompt_box.child(role_caret());
            }
        }

        let mut panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded_md()
            .bg(theme::raised())
            .border_1()
            .border_color(theme::accent_line())
            .child(name_row)
            .child(kind_row)
            .child(model_row);
        if self.role_model_picker == Some(idx) {
            panel = panel.child(self.role_model_menu(idx, cx));
        }
        panel
            .child(
                div()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("提示词（角色指令，运行时拼在团队协作说明之后）"),
            )
            .child(prompt_box)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .child("改动先留在内存，点页面底部「保存」写盘"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id(SharedString::from(format!("role-edit-done-{idx}")))
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(theme::accent())
                            .text_xs()
                            .text_color(gpui::rgb(0xffffff))
                            .cursor_pointer()
                            .hover(|d| d.opacity(0.88))
                            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                this.close_role_edit(cx);
                            }))
                            .child("完成"),
                    ),
            )
    }

    /// 角色编辑里的模型下拉：只列设置里启用且就绪的厂商与模型。
    fn role_model_menu(&self, idx: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let (cur_pid, cur_model) = self
            .config
            .active_team()
            .and_then(|t| t.roles.get(idx))
            .map(|r| (r.provider_id.clone(), r.model.clone()))
            .unwrap_or_default();
        let groups = self.picker_groups("");

        let mut list = div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded_md()
            .bg(theme::panel())
            .border_1()
            .border_color(theme::border());

        // 第一行：跟随会话默认（清空角色的厂商与模型绑定）。
        let on_default = cur_pid.is_empty() && cur_model.is_empty();
        list = list.child(
            div()
                .id(SharedString::from(format!("role-model-follow-{idx}")))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_md()
                .when(on_default, |d| d.bg(theme::accent_soft()))
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                    this.select_role_model(idx, String::new(), String::new(), cx);
                }))
                .child(
                    div()
                        .text_xs()
                        .text_color(if on_default { theme::accent() } else { theme::fg() })
                        .child("跟随会话默认"),
                )
                .child(div().flex_1())
                .when(on_default, |d| {
                    d.child(div().text_xs().text_color(theme::accent()).child("✓"))
                }),
        );

        if groups.is_empty() {
            list = list
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(theme::dim())
                        .child("没有可用模型：先到设置里添加厂商并勾选模型"),
                )
                .child(
                    div()
                        .id("role-model-go-settings")
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(theme::raised())
                        .text_xs()
                        .text_color(theme::accent())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::border()))
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.open_settings(SettingsTab::Models, cx);
                        }))
                        .child("去设置模型 →"),
                );
        }

        for g in groups {
            let provider_id = g.provider_id.clone();
            list = list.child(
                div()
                    .px_2()
                    .pt_1()
                    .text_xs()
                    .text_color(theme::dim())
                    .child(SharedString::from(format!("{} · {}", g.name, g.kind))),
            );
            for m in g.models {
                let on = provider_id == cur_pid && m == cur_model;
                let pid = provider_id.clone();
                let model = m.clone();
                list = list.child(
                    div()
                        .id(SharedString::from(format!("role-model-{idx}-{provider_id}-{m}")))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .when(on, |d| d.bg(theme::accent_soft()))
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::raised()))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                            this.select_role_model(idx, pid.clone(), model.clone(), cx);
                        }))
                        .child(
                            div()
                                .text_xs()
                                .text_color(if on { theme::accent() } else { theme::fg() })
                                .child(SharedString::from(m)),
                        )
                        .child(div().flex_1())
                        .when(on, |d| {
                            d.child(div().text_xs().text_color(theme::accent()).child("✓"))
                        }),
                );
            }
        }

        div()
            .id(SharedString::from(format!("role-model-menu-{idx}")))
            .max_h(px(220.0))
            .overflow_y_scroll()
            .child(list)
    }

    /// 角色卡展开的插件市场：搜索框 + 候选行（取自离线技能市场目录）。
    fn role_skill_panel(&self, idx: usize, role: &neverwork_core::Role, cx: &mut Context<Self>) -> impl IntoElement {
        let editing = self.active_field == Some(FormField::RoleSkillSearch);
        let display = if editing {
            self.field_buf.text().to_string()
        } else {
            self.role_skill_search.clone()
        };
        let empty = display.is_empty();

        // 搜索框：样式与技能市场一致，输入目标走 FormField。
        let search = div()
            .id(SharedString::from(format!("role-skill-search-{idx}")))
            .flex()
            .items_center()
            .gap_2()
            .flex_1()
            .h(px(28.0))
            .px_2()
            .rounded_full()
            .bg(theme::panel())
            .border_1()
            .border_color(if editing { theme::accent_line() } else { theme::border() })
            .cursor(gpui::CursorStyle::IBeam)
            .when(editing, |d| d.track_focus(&self.editor_focus))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.begin_field(FormField::RoleSkillSearch, cx);
                window.focus(&this.editor_focus);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _w, cx| {
                if this.active_field != Some(FormField::RoleSkillSearch) {
                    return;
                }
                if ev.keystroke.key == "escape" {
                    this.close_role_skill_picker(cx);
                    return;
                }
                if ev.keystroke.key == "enter" {
                    return;
                }
                if ev.keystroke.key == "backspace" && this.field_buf.text().is_empty() {
                    this.role_skill_search.clear();
                    cx.notify();
                    return;
                }
                this.on_field_key(ev, cx);
                this.role_skill_search = this.field_buf.text().to_string();
                cx.notify();
            }))
            .child(div().text_xs().text_color(theme::dim()).child("⌕"))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(if empty { theme::dim() } else { theme::fg() })
                    .child(SharedString::from(if empty {
                        "搜索插件，如「PDF」「云开发」".to_string()
                    } else {
                        fit_text(&display, 240.0, 12.0)
                    })),
            )
            .when(!empty, |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!("role-skill-search-clear-{idx}")))
                        .text_xs()
                        .text_color(theme::dim())
                        .cursor_pointer()
                        .hover(|d| d.text_color(theme::fg()))
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.role_skill_search.clear();
                            this.field_buf.reset("");
                            cx.notify();
                        }))
                        .child("✕"),
                )
            });

        // 候选：已知插件（已安装 + 内建 + 各来源市场），带市场条目以取真实图标与来源。
        let q = self.role_skill_search.trim().to_lowercase();
        let mut list: Vec<(neverwork_core::Skill, Option<crate::marketplace::MarketEntry>, bool)> = self
            .known_skills()
            .into_iter()
            .map(|s| {
                let entry = self.market_entry_for(&s.id);
                let installed = self.is_skill_installed(&s.id);
                (s, entry, installed)
            })
            .collect();
        // 计数取筛选前的全集：页签上的数字不随搜索 / 筛选跳动。
        let total_n = list.len();
        let mcp_n = list.iter().filter(|(s, _, _)| s.is_mcp()).count();
        let installed_n = list.iter().filter(|(_, _, on)| *on).count();
        if !q.is_empty() {
            list.retain(|(s, _, _)| {
                s.name.to_lowercase().contains(&q)
                    || s.id.to_lowercase().contains(&q)
                    || s.summary.to_lowercase().contains(&q)
                    || s.author.to_lowercase().contains(&q)
            });
        }
        match self.role_skill_filter {
            1 => list.retain(|(s, _, _)| s.is_mcp()),
            2 => list.retain(|(s, _, _)| !s.is_mcp()),
            3 => list.retain(|(_, _, on)| *on),
            _ => {}
        }
        list.sort_by(|a, b| {
            b.0.score()
                .partial_cmp(&a.0.score())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // 面板内滚动，不再砍到 8 条；上限避免极端数据把首帧拖慢。
        list.truncate(60);

        // 筛选页签：全部 / MCP / 技能 / 已安装。
        let filters: [(&str, usize, u8); 4] = [
            ("全部", total_n, 0),
            ("MCP", mcp_n, 1),
            ("技能", total_n - mcp_n, 2),
            ("已安装", installed_n, 3),
        ];
        let mut chips = div().flex().items_center().gap_1().pl_1();
        for (label, n, val) in filters {
            let on = self.role_skill_filter == val;
            chips = chips.child(
                div()
                    .id(SharedString::from(format!("role-skill-filter-{idx}-{val}")))
                    .px_2()
                    .py_0p5()
                    .rounded_full()
                    .text_xs()
                    .when(on, |d| d.bg(theme::fg()).text_color(theme::surface()))
                    .when(!on, |d| {
                        d.bg(theme::raised())
                            .text_color(theme::muted())
                            .hover(|d| d.text_color(theme::fg()))
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.role_skill_filter = val;
                        cx.notify();
                    }))
                    .child(SharedString::from(format!("{label} {n}"))),
            );
        }

        // 来源切换 + 刷新 + 拉取状态：绑定插件前先把市场数据拉顺。
        let mut sources = div().flex().items_center().gap_1().pl_1();
        for src in crate::marketplace::MarketSource::all() {
            let on = self.market_source == src;
            let has_cache = self
                .market_entries
                .get(&src)
                .map(|l| !l.is_empty())
                .unwrap_or(false);
            sources = sources.child(
                div()
                    .id(SharedString::from(format!("role-market-src-{idx}-{}", src.key())))
                    .px_2()
                    .py_0p5()
                    .rounded_full()
                    .text_xs()
                    .when(on, |d| d.bg(theme::fg()).text_color(theme::surface()))
                    .when(!on, |d| {
                        d.border_1()
                            .border_color(theme::border())
                            .text_color(theme::muted())
                            .hover(|d| d.text_color(theme::fg()))
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.set_market_source(src, cx);
                        let has = this
                            .market_entries
                            .get(&src)
                            .map(|l| !l.is_empty())
                            .unwrap_or(false);
                        if !has && !this.market_loading {
                            this.refresh_market(cx);
                        }
                    }))
                    .child(SharedString::from(if has_cache {
                        format!("{} ✓", src.label())
                    } else {
                        src.label().to_string()
                    })),
            );
        }
        let loading = self.market_loading;
        sources = sources.child(div().flex_1()).child(
            div()
                .id(SharedString::from(format!("role-market-refresh-{idx}")))
                .flex_shrink_0()
                .px_2()
                .py_0p5()
                .rounded_full()
                .text_xs()
                .text_color(if loading { theme::dim() } else { theme::accent() })
                .cursor_pointer()
                .hover(|d| d.bg(theme::border()))
                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                    this.refresh_market(cx);
                }))
                .child(if loading { "拉取中…" } else { "⟳ 刷新市场" }),
        );
        let error_line: Option<AnyElement> = self.market_error.as_ref().map(|e| {
            div()
                .px_1()
                .text_xs()
                .text_color(theme::red())
                .child(SharedString::from(fit_text(
                    &format!("市场拉取失败：{e}"),
                    520.0,
                    12.0,
                )))
                .into_any_element()
        });

        // MCP 安装确认条：注册外部进程前让用户点头（确认后自动绑到本角色）
        let pending_bar: Option<gpui::AnyElement> =
            self.pending_mcp_install.as_ref().map(|pid| {
                let sk = self.find_skill(pid);
                let name = sk
                    .as_ref()
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| pid.clone());
                let cmd = sk
                    .as_ref()
                    .and_then(|s| {
                        s.mcp
                            .as_ref()
                            .map(|m| format!("{} {}", m.command, m.args.join(" ")))
                    })
                    .unwrap_or_default();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .rounded_md()
                    .bg(theme::accent_soft())
                    .border_1()
                    .border_color(theme::accent_line())
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::fg())
                            .child(SharedString::from(format!(
                                "「{name}」是 MCP 插件，将注册并启动外部进程：{cmd}"
                            ))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .justify_end()
                            .child(
                                div()
                                    .id(SharedString::from(format!("role-mcp-cancel-{idx}")))
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .text_xs()
                                    .text_color(theme::muted())
                                    .cursor_pointer()
                                    .hover(|d| d.bg(theme::border()))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                        this.cancel_mcp_install(cx);
                                    }))
                                    .child("取消"),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("role-mcp-confirm-{idx}")))
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .bg(theme::accent())
                                    .text_xs()
                                    .text_color(gpui::rgb(0xffffff))
                                    .cursor_pointer()
                                    .hover(|d| d.opacity(0.88))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                        this.confirm_mcp_install(cx);
                                    }))
                                    .child("确认并绑定"),
                            ),
                    )
                    .into_any_element()
            });

        let mut rows = div().flex().flex_col().gap_1();
        if list.is_empty() {
            rows = rows.child(
                div()
                    .px_2()
                    .py_2()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("市场里没有匹配的插件，换个关键词试试"),
            );
        }
        for (skill, entry, _) in list {
            let added = role.skills.iter().any(|x| x == &skill.id);
            rows = rows.child(self.role_plugin_row(idx, skill, entry, added, cx));
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded_md()
            .bg(theme::raised())
            .border_1()
            .border_color(theme::accent_line())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(search)
                    .child(
                        div()
                            .id(SharedString::from(format!("role-skill-done-{idx}")))
                            .px_2()
                            .py_0p5()
                            .rounded_md()
                            .text_xs()
                            .text_color(theme::muted())
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::border()))
                            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                this.close_role_skill_picker(cx);
                            }))
                            .child("完成"),
                    ),
            )
            .child(chips)
            .child(sources)
            .children(error_line)
            .child(
                div()
                    .id(SharedString::from(format!("role-skill-list-{idx}")))
                    .max_h(px(300.0))
                    .overflow_y_scroll()
                    .child(rows),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("文本插件安装后落盘到 skills/<id>/SKILL.md，绑定后正文拼进该角色提示词；MCP 插件会启动外部进程，需确认。点条目看详情，点底部「保存」写盘。"),
            )
            .children(pending_bar)
    }

    /// 插件市场面板里的一行候选：真实图标 + 名称/来源/形态 + 摘要。
    ///
    /// 点行主体打开详情抽屉（底栏是「绑定到角色」）；右侧「添加」直接绑定。
    /// `added` 为真时右侧是「已添加」（不可再点）。
    fn role_plugin_row(
        &self,
        idx: usize,
        skill: neverwork_core::Skill,
        entry: Option<crate::marketplace::MarketEntry>,
        added: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let glyph = if skill.glyph.trim().is_empty() {
            "✦".to_string()
        } else {
            skill.glyph.clone()
        };
        let name = skill.name.clone();
        let summary = fit_text(&skill.summary, 520.0, 12.0);
        let source = crate::marketplace::source_label(&skill.id);
        let mcp = skill.is_mcp();
        // 正在抓详情 / 解析正文的条目：按钮位显示「安装中…」，防重复点。
        let busy_here = self.market_installing.as_deref() == Some(skill.id.as_str());
        // 远程托管条目没有本地启动配置，不能安装 / 绑定。
        let remote_only = entry.as_ref().map(|e| e.remote_only).unwrap_or(false);

        // 图标：优先真实 logo（已缓存），否则回退字形方块。
        let icon: AnyElement = match entry.as_ref().and_then(crate::marketplace::icon_path) {
            Some(path) => div()
                .size(px(26.0))
                .flex_shrink_0()
                .rounded_md()
                .overflow_hidden()
                .child(img(path).size(px(26.0)).object_fit(gpui::ObjectFit::Cover))
                .into_any_element(),
            None => div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(26.0))
                .flex_shrink_0()
                .rounded_md()
                .bg(theme::accent_soft())
                .text_xs()
                .text_color(theme::accent())
                .child(SharedString::from(glyph))
                .into_any_element(),
        };

        let action = if busy_here {
            div()
                .px_2()
                .py_0p5()
                .rounded_full()
                .bg(theme::with_alpha(0xf59e0b, 0.14))
                .text_xs()
                .text_color(theme::yellow())
                .child("安装中…")
                .into_any_element()
        } else if added {
            div()
                .px_2()
                .py_0p5()
                .rounded_full()
                .bg(theme::accent_soft())
                .text_xs()
                .text_color(theme::accent())
                .child("已添加")
                .into_any_element()
        } else if remote_only {
            div()
                .px_2()
                .py_0p5()
                .rounded_full()
                .border_1()
                .border_color(theme::border())
                .text_xs()
                .text_color(theme::dim())
                .child("仅远程")
                .into_any_element()
        } else {
            let add = skill.clone();
            div()
                .id(SharedString::from(format!("role-{idx}-add-{}", skill.id)))
                .px_2()
                .py_0p5()
                .rounded_full()
                .border_1()
                .border_color(theme::accent_line())
                .text_xs()
                .text_color(theme::accent())
                .cursor_pointer()
                .hover(|d| d.bg(theme::accent_soft()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                    // 不让点击冒泡到整行（否则会顺手打开详情抽屉）。
                    cx.stop_propagation();
                    this.attach_skill_to_role(idx, add.clone(), cx);
                }))
                .child("＋ 添加")
                .into_any_element()
        };

        // 本地导入 / 内建条目没有市场条目，用技能本体补一个：详情抽屉照常可开。
        let entry_for_open = entry.unwrap_or_else(|| {
            let mut e = crate::marketplace::MarketEntry::new(skill.clone());
            e.remote_only = false;
            e.tags = Vec::new();
            e
        });

        div()
            .id(SharedString::from(format!("role-{idx}-plugin-row-{}", skill.id)))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1p5()
            .rounded_md()
            .bg(theme::panel())
            .cursor_pointer()
            .hover(|d| d.bg(theme::raised()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                this.open_market_detail_for_role(entry_for_open.clone(), idx, cx);
            }))
            .child(icon)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::fg())
                                    .child(SharedString::from(name)),
                            )
                            .child(
                                div()
                                    .px_1()
                                    .rounded_sm()
                                    .bg(theme::raised())
                                    .text_xs()
                                    .text_color(theme::dim())
                                    .child(source),
                            )
                            .child(if mcp {
                                div()
                                    .px_1()
                                    .rounded_sm()
                                    .bg(theme::accent_soft())
                                    .text_xs()
                                    .text_color(theme::accent())
                                    .child("MCP")
                                    .into_any_element()
                            } else {
                                div()
                                    .px_1()
                                    .rounded_sm()
                                    .bg(theme::raised())
                                    .text_xs()
                                    .text_color(theme::muted())
                                    .child("文本")
                                    .into_any_element()
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(SharedString::from(summary)),
                    ),
            )
            .child(action)
    }

}

/// 角色提示词框里的光标。
fn role_caret() -> impl IntoElement {
    div()
        .w(px(2.0))
        .h(px(16.0))
        .flex_shrink_0()
        .bg(theme::accent())
}

/// 已绑定插件 chip：`✦ 名称 ✕`，点 ✕ 解除绑定。
pub(crate) fn plugin_chip(
    idx: usize,
    id: &str,
    name: String,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let remove_id = id.to_string();
    div()
        .id(SharedString::from(format!("role-{idx}-plugin-{id}")))
        .flex()
        .items_center()
        .gap_1()
        .pl_2()
        .pr_1()
        .py_0p5()
        .rounded_full()
        .bg(theme::accent_soft())
        .text_xs()
        .text_color(theme::accent())
        .child("✦")
        .child(SharedString::from(name))
        .child(
            div()
                .id(SharedString::from(format!("role-{idx}-unplug-{id}")))
                .px_1()
                .rounded_full()
                .text_xs()
                .text_color(theme::muted())
                .cursor_pointer()
                .hover(|d| d.bg(theme::border()).text_color(theme::fg()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                    this.detach_skill_from_role(idx, &remove_id, cx);
                }))
                .child("✕"),
        )
}
