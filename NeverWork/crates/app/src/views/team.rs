use super::*;
use gpui::ClickEvent;

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

        // 团队名 + 编排模式
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
                        .child(div().text_sm().font_weight(FontWeight::BOLD).child(SharedString::from(team.name.clone())))
                        .child(div().text_xs().text_color(theme::muted()).child(SharedString::from(format!("编排：{}", team.flow.label())))),
                ),
        );

        // 编排模式切换
        body = body.child(
            div()
                .flex()
                .gap_2()
                .child(flow_chip(cx, neverwork_core::FlowMode::Pipeline, team.flow))
                .child(flow_chip(cx, neverwork_core::FlowMode::Parallel, team.flow))
                .child(flow_chip(cx, neverwork_core::FlowMode::Supervisor, team.flow)),
        );

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
        div()
            .flex()
            .items_center()
            .gap_3()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
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
                            .child(SharedString::from(format!("模型：{}", role.model)))
                            .child(div().flex_1())
                            .child(div().text_xs().text_color(theme::dim()).child(SharedString::from(truncate(&role.prompt, 28)))),
                    ),
            )
            .child(
                div()
                    .id(SharedString::from(format!("toggle-model-{idx}")))
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(theme::raised())
                    .text_xs()
                    .text_color(theme::muted())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::border()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.cycle_role_model(idx, cx);
                    }))
                    .child("换模型"),
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
            )
    }

    pub(crate) fn active_team_index(&self) -> usize {
        self.config
            .teams
            .iter()
            .position(|t| t.id == self.config.active_team_id)
            .unwrap_or(0)
    }

    pub(crate) fn add_role(&mut self, cx: &mut Context<Self>) {
        let ti = self.active_team_index();
        if let Some(team) = self.config.teams.get_mut(ti) {
            let n = team.roles.len();
            let id = format!("role-{n}");
            let mut role = neverwork_core::Role::new(id, format!("角色{}", n + 1), "A");
            role.kind_label = "自定义".into();
            role.color_index = n % 6;
            role.model = "gpt-4o".into();
            role.prompt = "负责你分配的职责。".into();
            team.roles.push(role);
        }
        cx.notify();
    }

    pub(crate) fn remove_role(&mut self, idx: usize, cx: &mut Context<Self>) {
        let ti = self.active_team_index();
        if let Some(team) = self.config.teams.get_mut(ti) {
            if team.roles.len() > 1 && idx < team.roles.len() {
                team.roles.remove(idx);
            }
        }
        cx.notify();
    }

    pub(crate) fn cycle_role_model(&mut self, idx: usize, cx: &mut Context<Self>) {
        const MODELS: [&str; 4] = ["gpt-4o", "claude-sonnet-4-5", "deepseek-chat", "gpt-4o-mini"];
        let ti = self.active_team_index();
        if let Some(team) = self.config.teams.get_mut(ti) {
            if let Some(role) = team.roles.get_mut(idx) {
                let cur = role.model.clone();
                let pos = MODELS.iter().position(|m| *m == cur).unwrap_or(0);
                role.model = MODELS[(pos + 1) % MODELS.len()].to_string();
            }
        }
        cx.notify();
    }

    pub(crate) fn set_flow(&mut self, flow: neverwork_core::FlowMode, cx: &mut Context<Self>) {
        let ti = self.active_team_index();
        if let Some(team) = self.config.teams.get_mut(ti) {
            team.flow = flow;
        }
        cx.notify();
    }

    pub(crate) fn save_team(&mut self, cx: &mut Context<Self>) {
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

}

pub(crate) fn flow_chip(cx: &mut Context<AppState>, mode: neverwork_core::FlowMode, current: neverwork_core::FlowMode) -> impl IntoElement {    let on = mode == current;
    let base = div()
        .id(SharedString::from(format!("flow-{:?}", mode)))
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .flex_1()
        .py_2()
        .rounded_md()
        .border_1()
        .text_sm()
        .cursor_pointer();
    let el = if on {
        base.border_color(theme::accent_line()).bg(theme::accent_soft()).text_color(theme::accent())
    } else {
        base.border_color(theme::border()).bg(theme::panel()).text_color(theme::muted())
    };
    el.on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
        this.set_flow(mode, cx);
    }))
    .child(SharedString::from(mode.label()))
}
