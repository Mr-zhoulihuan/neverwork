use super::*;
use gpui::ClickEvent;

use crate::state::FormField;
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
        // 下标会变，面板直接收起，避免后面的角色被误展开。
        self.close_role_skill_picker(cx);
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

    /// 展开 / 收起某个角色的「插件市场」面板。
    pub(crate) fn toggle_role_skill_picker(&mut self, idx: usize, cx: &mut Context<Self>) {
        if self.role_skill_picker == Some(idx) {
            self.close_role_skill_picker(cx);
            return;
        }
        self.role_skill_picker = Some(idx);
        self.role_skill_search.clear();
        self.field_buf.reset("");
        if self.active_field == Some(FormField::RoleSkillSearch) {
            self.active_field = None;
        }
        cx.notify();
    }

    pub(crate) fn close_role_skill_picker(&mut self, cx: &mut Context<Self>) {
        self.role_skill_picker = None;
        self.role_skill_search.clear();
        self.field_buf.reset("");
        if self.active_field == Some(FormField::RoleSkillSearch) {
            self.active_field = None;
        }
        cx.notify();
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
                        "搜索插件市场，如「需求分析」".to_string()
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

        // 候选：已知插件（已安装 + 内建 + 各来源市场），按综合评分排序，最多 8 条。
        let q = self.role_skill_search.trim().to_lowercase();
        let mut list: Vec<(neverwork_core::Skill, bool)> = self
            .known_skills()
            .into_iter()
            .map(|s| {
                let installed = self.is_skill_installed(&s.id);
                (s, installed)
            })
            .collect();
        if !q.is_empty() {
            list.retain(|(s, _)| {
                s.name.to_lowercase().contains(&q)
                    || s.id.to_lowercase().contains(&q)
                    || s.summary.to_lowercase().contains(&q)
                    || s.author.to_lowercase().contains(&q)
            });
        }
        list.sort_by(|a, b| {
            b.0.score()
                .partial_cmp(&a.0.score())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        list.truncate(8);

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
        for (skill, _) in list {
            let added = role.skills.iter().any(|x| x == &skill.id);
            rows = rows.child(self.role_plugin_row(idx, skill, added, cx));
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
            .child(rows)
            .child(
                div()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("文本插件安装后落盘到 skills/<id>/SKILL.md，正文会拼进角色提示词；MCP 插件会启动外部进程，需确认。点底部「保存」写盘。"),
            )
            .children(pending_bar)
    }

    /// 插件市场面板里的一行候选。`added` 为真时右侧是「已添加」（不可再点）。
    fn role_plugin_row(
        &self,
        idx: usize,
        skill: neverwork_core::Skill,
        added: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let glyph = if skill.glyph.trim().is_empty() {
            "✦".to_string()
        } else {
            skill.glyph.clone()
        };
        let name = skill.name.clone();
        let summary = fit_text(&skill.summary, 320.0, 12.0);
        let action = if added {
            div()
                .px_2()
                .py_0p5()
                .rounded_full()
                .bg(theme::accent_soft())
                .text_xs()
                .text_color(theme::accent())
                .child("已添加")
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
                    this.attach_skill_to_role(idx, add.clone(), cx);
                }))
                .child("＋ 添加")
                .into_any_element()
        };

        div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(theme::panel())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(22.0))
                    .rounded_md()
                    .bg(theme::accent_soft())
                    .text_xs()
                    .text_color(theme::accent())
                    .child(SharedString::from(glyph)),
            )
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
                                    .child(SharedString::from(name)),
                            )
                            .when(skill.is_mcp(), |d| {
                                d.child(
                                    div()
                                        .px_1()
                                        .rounded_sm()
                                        .bg(theme::accent_soft())
                                        .text_xs()
                                        .text_color(theme::accent())
                                        .child("MCP"),
                                )
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

pub(crate) fn flow_chip(cx: &mut Context<AppState>, mode: neverwork_core::FlowMode, current: neverwork_core::FlowMode) -> impl IntoElement {
    let on = mode == current;
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
