use super::*;
use gpui::{img, ClickEvent, CursorStyle, KeyDownEvent};
use neverwork_core::MessageKind;
use crate::state::InputTarget;
use crate::views::text_fit::fit_text;

/// 对话正文基准字号。折行估宽与渲染必须用同一个值：
/// 估窄了行会顶出气泡右侧被父容器裁掉半截。
const BODY_PX: f32 = 16.0;

/// 思考档位滑杆的格子宽度（4 档 × 64px，不依赖百分比计算）。
const THINK_CELL: f32 = 64.0;

/// 列表标记（`•` / `1.`）连同 `gap_2` 占掉的横向宽度。
fn marker_w(label: &str) -> f32 {
    label.chars().count() as f32 * BODY_PX * crate::views::text_fit::MONO_ASCII_RATIO + 8.0
}

/// 点击回调（`cx.listener` 的返回类型）：自由函数要接住它必须装箱。
pub(crate) type ClickCb =
    Box<dyn Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static>;

/// 消息级「复制」按钮：常驻但很淡，悬停变亮。
///
/// gpui 0.2.2 的文本元素没有选中能力（无 `InteractiveText`），
/// 所以「复制一句话」只能由消息自带的按钮驱动。
/// 用文字而不是 `⧉` 图标：图标太隐晦，用户根本不知道哪里能点。
fn copy_chip(
    id: impl Into<gpui::ElementId>,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_shrink_0()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .text_xs()
        .text_color(theme::dim())
        .cursor_pointer()
        .hover(|d| d.bg(theme::raised()).text_color(theme::accent()))
        .on_click(on_click)
        .child("复制")
}

/// 产物「另存为」按钮：弹系统保存对话框，默认目录工作区 `artifacts/`
/// （无工作区落到配置目录 exports/），可改到任意位置。
///
/// 必须与产物卡片/行**并排**而不是嵌套：嵌套时点击区重叠，点保存会
/// 顺带触发「打开产物」。右栏产物面板与对话底部产物条共用这一个按钮。
pub(crate) fn artifact_save_button(
    id: impl Into<gpui::ElementId>,
    path: String,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_shrink_0()
        .px_1()
        .rounded_sm()
        .text_size(px(12.0))
        .text_color(theme::dim())
        .cursor_pointer()
        .hover(|d| d.bg(theme::raised()).text_color(theme::accent()))
        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
            this.export_artifact(&path, cx);
        }))
        .child("⤓")
}

impl AppState {
    /// 把一段对话内容写进系统剪贴板，并给一条轻提示。
    pub(crate) fn copy_message(&mut self, text: String, what: &str, cx: &mut Context<Self>) {
        if text.trim().is_empty() {
            return;
        }
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        self.notice = Some(format!("已复制{what}"));
        cx.notify();
    }

    pub(crate) fn chat_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session();
        // 本会话实际用的厂商/模型（会话没选过就回落全局当前厂商）。
        let (model_provider, model_id) = self.session_model_labels(self.active_session);
        // 本会话对应的团队：切换「当前团队」不影响已建的旧会话。
        let session_team = self.team_for_session(self.active_session);
        let mut col = div().flex().flex_col().flex_1().h_full().overflow_hidden().bg(theme::surface());

        col = col.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .h(px(56.0))
                .px_5()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    // 窄窗口优先保右侧 chips：标题列允许收缩到 0，
                    // 否则标题的 min-content 宽度会把页签/模式 chip 顶出屏幕。
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(SharedString::from(session.title.clone())),
                        )
                        // 团队模式才显示角色点；单助手模式不放任何说明文字。
                        // 「能问答也能读文件」这种事说了也没用：用户直接问就行，
                        // 挂在标题下面只会一直占着一行。
                        .when(self.is_team_mode(), |d| d.child(role_dots_row(session_team.as_ref()))),
                )
                .child(div().flex_1())
                .child(self.model_chip(&model_provider, &model_id, cx))
                .child(self.trace_tabs(cx))
                .child(
                    // 模式在新建会话时选定，这里只展示、不提供切换。
                    // 要另一种模式就点侧栏「＋」新建一个会话。
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_3()
                        .py_1p5()
                        .rounded_md()
                        .bg(theme::panel())
                        .border_1()
                        .border_color(theme::border())
                        .text_xs()
                        .text_color(theme::muted())
                        .child(if self.is_team_mode() {
                            SharedString::from(format!(
                                "◈ {} · 团队",
                                session_team
                                    .as_ref()
                                    .map(|t| t.name.clone())
                                    .unwrap_or_default()
                            ))
                        } else {
                            SharedString::from("◎ 单助手")
                        }),
                ),
        );

        // 选择器就地嵌在标题行下面（不做浮层：窄窗口 / 高 DPI 下不易算歪）。
        if self.model_picker {
            col = col.child(self.chat_model_picker(cx));
        }

        // 轨迹页签：整区替换对话内容（状态条、消息流、输入框都不渲染），
        // 与参考稿（DeepSeek session log）一致；下面的代码全部跳过。
        if self.trace_open {
            return col.child(self.trace_view(cx));
        }

        // 顶部状态条：团队会话显示流水线（按当前团队的角色动态生成），
        // 单助手会话默认整条不出现。
        // 之前单助手也会挂一条「需要时会读文件，改文件前会问你」，
        // 属于解释了也不影响操作的废话提示，还白占一行高度。
        // 只有正在生成时才给一条，让用户知道它在干活、能停止。
        if self.is_team_mode() {
            let mut steps = div().flex().items_center().gap_2().flex_wrap();
            match session_team.as_ref() {
                Some(t) if !t.roles.is_empty() => {
                    for (i, role) in t.roles.iter().enumerate() {
                        if i > 0 {
                            steps = steps
                                .child(div().text_xs().text_color(theme::dim()).child("→"));
                        }
                        // 正在发言的角色高亮，空闲时全部正常显示。
                        let on = self.active_role.map_or(true, |a| a == i);
                        let label = if role.name.trim().is_empty() {
                            role.kind_label.as_str()
                        } else {
                            role.name.as_str()
                        };
                        steps =
                            steps.child(flow_step(&role.initial, role.color_index, label, on));
                    }
                }
                // 团队里没有任何角色：别硬编一条假流水线
                _ => {
                    steps = steps.child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .child("团队里还没有角色，去「团队」页添加"),
                    );
                }
            }
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_5()
                    .py_2()
                    .bg(theme::accent_soft())
                    .border_b_1()
                    .border_color(theme::border())
                    .child(steps)
                    .child(div().flex_1())
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded_md()
                            .bg(theme::accent_soft())
                            .text_xs()
                            .text_color(theme::accent())
                            .child(if self.is_running() { "生成中…" } else { "协作团队" }),
                    ),
            );
        } else if self.is_running() {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_5()
                    .py_2()
                    .bg(theme::accent_soft())
                    .border_b_1()
                    .border_color(theme::border())
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::accent())
                            .child("生成中…"),
                    ),
            );
        }

        // 整个对话流用等宽字体：参考稿（opencode）是终端风格，
        // 等宽让工具行、代码块、正文的行首对齐在同一条竖线上。
        let mut thread = div()
            .flex()
            .flex_col()
            .gap_4()
            .px_5()
            .py_5()
            .font_family(theme::mono_family())
            // 显式钉死字号：gpui 默认是 1rem（16px），但折行估宽用的是
            // `BODY_PX`，两者必须一致，不能靠默认值巧合相等。
            .text_size(px(BODY_PX));
        // 正文折行宽度：中栏真实可用宽度减去左右 px_5 内边距。
        let avail = (self.chat_w - 40.0).max(240.0);
        let sidx = self.active_session;
        let total = session.messages.len();
        let mut i = 0usize;
        while i < total {
            // 连续的工具调用聚合成一行，展开才看细节（对齐参考稿的步骤条）
            if matches!(session.messages[i].kind, MessageKind::ToolCall { .. }) {
                let mut j = i + 1;
                while j < total
                    && matches!(session.messages[j].kind, MessageKind::ToolCall { .. })
                {
                    j += 1;
                }
                let pending_here = self
                    .pending
                    .as_ref()
                    .map(|p| p.session_idx == sidx && p.msg_idx >= i && p.msg_idx < j)
                    .unwrap_or(false);
                thread = thread.child(self.tool_steps_group(i, j, pending_here, cx));
                i = j;
                continue;
            }
            let m = session.messages[i].clone();
            thread = thread.child(self.render_msg(&m, avail, i, cx));
            i += 1;
        }
        if self.is_running() {
            thread = thread.child(
                div()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("▍正在生成…"),
            );
        }
        // 会话产物（Agent 写入的文件 / 生成的图片）贴在对话流末尾
        thread = thread.child(self.artifacts_footer(cx));

        let mut body = div().flex().flex_col().flex_1().min_h(px(0.0));
        body = body.child(
            div()
                .id("chat-scroll")
                .flex_1()
                .overflow_scroll()
                .track_scroll(&self.chat_scroll)
                .child(thread),
        );

        // 内容有更新时滚到底部。必须在这里（内容已装进容器之后）请求，
        // 否则滚动会作用在上一帧的布局上，表现为「停在中途」。
        if self.chat_scroll_dirty.take() {
            self.chat_scroll.scroll_to_bottom();
        }
        body = body.child(self.composer(cx));
        col = col.child(body);
        col
    }

    /// 会话产物条：Agent 写过的文件与生成的图片，贴在对话流末尾。
    ///
    /// 与右栏「产物」面板同源（`Session.artifacts`）；这里只展示最新几个，
    /// 完整列表在右栏。没有产物时返回空元素，不占空间。
    fn artifacts_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sidx = self.active_session;
        let list = self
            .sessions
            .get(sidx)
            .map(|s| s.artifacts.as_slice())
            .unwrap_or(&[]);
        let mut col = div().flex().flex_col().gap_2().pt_2();
        if list.is_empty() {
            return col;
        }
        // 最新在前
        let mut ordered: Vec<&neverwork_core::Artifact> = list.iter().collect();
        ordered.reverse();
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().h(px(1.0)).flex_1().bg(theme::border()))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::dim())
                        .child(SharedString::from(format!("产物 · {}", list.len()))),
                )
                .child(div().h(px(1.0)).flex_1().bg(theme::border())),
        );
        let mut row = div().flex().items_center().gap_2().flex_wrap();
        for (i, a) in ordered.iter().take(8).enumerate() {
            let path = a.path.clone();
            let mut chip = div()
                .id(SharedString::from(format!("artifact-{i}")))
                .flex()
                .items_center()
                .gap_2()
                .h(px(34.0))
                .pl_1()
                .pr_2()
                .rounded_md()
                .border_1()
                .border_color(theme::border())
                .bg(theme::raised())
                .cursor_pointer()
                .hover(|d| d.bg(theme::panel()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                    this.open_artifact(&path, cx);
                }));
            // 缩略图只给确实还在盘上的图片，文件被删了退回类型徽标
            if a.kind == neverwork_core::ArtifactKind::Image
                && std::path::Path::new(&a.path).exists()
            {
                chip = chip.child(
                    div()
                        .size(px(26.0))
                        .flex_shrink_0()
                        .rounded_sm()
                        .overflow_hidden()
                        .child(
                            img(std::path::PathBuf::from(&a.path))
                                .size(px(26.0))
                                .object_fit(gpui::ObjectFit::Cover),
                        ),
                );
            } else {
                chip = chip.child(super::files::file_badge(&a.name));
            }
            chip = chip.child(
                div()
                    .max_w(px(150.0))
                    .text_xs()
                    .text_color(theme::muted())
                    .whitespace_nowrap()
                    .child(SharedString::from(fit_text(&a.name, 140.0, 11.0))),
            );
            // chip 与保存按钮并排（不嵌套）：点⤓只保存，不会顺带打开产物
            let save = artifact_save_button(
                SharedString::from(format!("artifact-save-{i}")),
                a.path.clone(),
                cx,
            );
            row = row.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(chip)
                    .child(save),
            );
        }
        if let Some(rest) = list.len().checked_sub(8).filter(|n| *n > 0) {
            row = row.child(
                div()
                    .text_xs()
                    .text_color(theme::dim())
                    .child(SharedString::from(format!("+{rest}"))),
            );
        }
        col.child(row)
    }

    /// 「对话 | 轨迹」两段页签：轨迹是整区切换（对齐参考稿的 session log），
    /// 打开时对话滚动区与输入框都不渲染，切回「对话」或 Esc 返回。
    fn trace_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let seg = |id: &'static str, label: &'static str, active: bool, to_trace: bool| {
            div()
                .id(id)
                .px_3()
                .py_1p5()
                .rounded_md()
                .text_xs()
                .text_color(if active { theme::fg() } else { theme::muted() })
                .bg(if active { theme::raised() } else { theme::panel() })
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                    if this.trace_open != to_trace {
                        this.trace_open = to_trace;
                        if !to_trace {
                            this.trace_selected = None;
                        }
                        cx.notify();
                    }
                }))
                .child(label)
        };
        div()
            .flex()
            .items_center()
            .gap_0p5()
            .p_0p5()
            .rounded_md()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .child(seg("tab-chat", "对话", !self.trace_open, false))
            .child(seg("tab-trace", "◷ 轨迹", self.trace_open, true))
    }

    /// 头部模型 chip：显示本会话的「厂商 · 模型」，点开按厂商分组的模型选择器。
    fn model_chip(
        &self,
        provider: &str,
        model: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let label = match (provider.is_empty(), model.is_empty()) {
            (true, _) => "未配置模型".to_string(),
            (false, true) => provider.to_string(),
            (false, false) => format!("{provider} · {model}"),
        };
        let open = self.model_picker;
        div()
            .id("chat-model-chip")
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1p5()
            .rounded_md()
            .border_1()
            .border_color(if open {
                theme::accent_line()
            } else {
                theme::border()
            })
            .bg(if open { theme::accent_soft() } else { theme::panel() })
            .text_xs()
            .text_color(if open { theme::fg() } else { theme::muted() })
            .cursor_pointer()
            .hover(|d| d.bg(theme::raised()))
            .on_click(cx.listener(|this, _: &gpui::ClickEvent, window, cx| {
                if this.model_picker {
                    this.close_chat_model_picker(cx);
                } else {
                    this.open_chat_model_picker(cx);
                    window.focus(&this.editor_focus);
                    cx.notify();
                }
            }))
            .child(
                div()
                    .size(px(6.0))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(if provider.is_empty() {
                        theme::red()
                    } else {
                        theme::green()
                    }),
            )
            .child(SharedString::from(fit_text(&label, 240.0, 12.0)))
            .child(div().text_color(theme::dim()).child("▾"))
    }

    /// 当前模型是否支持工具调用（团队自动交接的前提）。
    fn tools_supported(&self) -> bool {
        if let Ok(p) = std::env::var("NW_PROVIDER") {
            return p.is_empty() || p.eq_ignore_ascii_case("openai");
        }
        matches!(
            self.provider_for_session(self.active_session)
                .map(|p| p.protocol()),
            Some(neverwork_core::Protocol::OpenAiCompatible)
        )
    }

    pub(crate) fn composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.chat.text().is_empty();
        let has_open_file = self.open_path.is_some() || self.remote_open_rel.is_some();

        let mut col = div()
            .px_6()
            .pt_4()
            .pb_4()
            .border_t_1()
            .border_color(theme::border())
            .bg(theme::surface())
            .flex()
            .flex_col()
            .gap_2();

        // 有待确认的写操作时，输入框上方给出显式提示（卡片里也有按钮）。
        // 只提示「当前会话自己的」：生成中点「＋新建会话」后，旧会话的审批
        // 卡不属于这里，不能顶到新会话的输入框上方。
        if let Some(p) = self
            .pending
            .as_ref()
            .filter(|p| p.session_idx == self.active_session)
        {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(theme::with_alpha(0xf59e0b, 0.12))
                    .border_1()
                    .border_color(theme::with_alpha(0xf59e0b, 0.4))
                    .text_xs()
                    .text_color(theme::yellow())
                    .child("⚠")
                    .child(SharedString::from(format!(
                        "等待你确认：{} {}",
                        p.tool, p.summary
                    )))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .child(if p.tool == "run_command" {
                                "在上方卡片里点「允许并执行」或「拒绝」"
                            } else {
                                "在上方卡片里点「允许并写入」或「拒绝」"
                            }),
                    ),
            );
        }

        if let Some(notice) = &self.notice {
            col = col.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(theme::green_soft())
                    .border_1()
                    .border_color(theme::green_line())
                    .text_xs()
                    .text_color(theme::yellow())
                    .child(SharedString::from(notice.clone())),
            );
        } else if !self.ai_ready {
            col = col.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(theme::accent_soft())
                    .text_xs()
                    .text_color(theme::muted())
                    .child("未检测到 AI 配置。设置环境变量 NW_API_KEY 后重启即可对话。"),
            );
        }

        // ===== 团队 @ 入口：角色 chips 与「本轮交给」提示 =====
        if self.is_team_mode() {
            if let Some(team) = self.team_for_session(self.active_session) {
                if !self.is_running() {
                    let mut chips = div().flex().items_center().gap_2().flex_wrap();
                    for (i, role) in team.roles.iter().enumerate() {
                        let name = role.name.clone();
                        chips = chips.child(
                            div()
                                .id(SharedString::from(format!("role-chip-{i}")))
                                .flex()
                                .items_center()
                                .gap_1()
                                .px_2()
                                .py_0p5()
                                .rounded_md()
                                .border_1()
                                .border_color(theme::border())
                                .bg(theme::panel())
                                .text_xs()
                                .text_color(theme::muted())
                                .cursor_pointer()
                                .hover(|d| d.bg(theme::raised()).text_color(theme::accent()))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                                    this.chat.insert(&format!("@{name} "));
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .size(px(6.0))
                                        .rounded_md()
                                        .bg(theme::role_color(role.color_index)),
                                )
                                .child(SharedString::from(role.name.clone())),
                        );
                    }
                    col = col.child(chips);
                }
                // 命中 @角色 时告诉用户这一轮会交给谁；降级原因写在后面，
                // 否则用户会以为是角色没响应。
                if let Some(ri) =
                    neverwork_core::parse_role_mention(self.chat.text(), &team.roles)
                {
                    if let Some(role) = team.roles.get(ri) {
                        let mut hint = format!("本轮交给：{}", role.name);
                        if !self.tools_enabled {
                            hint.push_str("（工具已关：角色间不会自动交接）");
                        } else if !self.tools_supported() {
                            hint.push_str("（当前模型不支持工具，角色间不会自动交接）");
                        }
                        col = col.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .py_1p5()
                                .rounded_md()
                                .bg(theme::accent_soft())
                                .border_1()
                                .border_color(theme::accent_line())
                                .text_xs()
                                .text_color(theme::accent())
                                .child(SharedString::from(hint)),
                        );
                    }
                }
            }
        }

        // ===== 输入行：逐字符渲染，能看到光标，也能点任意位置改字 =====
        // 输入框真实位置：IME 候选框要贴住这里，而不是窗口左上角。
        // 之前 GPUI 拿到的 bounds 来自那个 1x1 的输入桥，候选框永远飘在角落。
        let area_slot = self.input_area_bounds.clone();
        let mut input_el = div()
            .id("composer-input")
            .flex()
            .flex_row()
            // 单行内容在加高后的框里垂直居中；多行时从顶端自然排开
            .items_center()
            .flex_wrap()
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            // 正文用 text_base：原来 text_sm 偏小，长句在宽窗口里很难读
            .text_base()
            .line_height(px(24.0))
            .min_h(px(48.0))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.switch_input_target(InputTarget::Chat);
                let end = this.chat.text().len();
                this.chat.move_to(end, false);
                window.focus(&this.focus_handle);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _window, cx| {
                this.on_chat_key(ev, cx);
            }))
            // 1x1 画布只为记录这块区域的真实坐标（渲染期拿 bounds 的唯一可靠时机）。
            .child(
                gpui::canvas(
                    |_b, _w, _cx| {},
                    move |bounds, _st, _w, _cx| {
                        area_slot.set(Some(bounds));
                    },
                )
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .size(px(1.0)),
            );

        if empty {
            // 空输入框也要画光标：否则默认状态只有一个占位提示，
            // 看不出「这里已经可以打字了」，用户得先点一下才敢输入。
            // 光标画在占位文字左侧，符合「插入点在文本起始处」的直觉。
            input_el = input_el
                .child(chat_caret())
                .child(
                    div()
                        .text_color(theme::dim())
                        .child("在这里提问，或下达要执行的任务…"),
                );
        } else {
            let cursor = self.chat.cursor();
            let sel = self.chat.selection();
            let mut caret_drawn = false;
            let chars: Vec<(usize, char)> = self.chat.text().char_indices().collect();
            for (i, ch) in chars {
                if cursor == i {
                    input_el = input_el.child(chat_caret());
                    caret_drawn = true;
                }
                let selected = sel
                    .as_ref()
                    .map(|r| i >= r.start && i < r.end)
                    .unwrap_or(false);
                let display: SharedString = if ch == ' ' {
                    SharedString::from("\u{00A0}")
                } else {
                    SharedString::from(ch.to_string())
                };
                let cell = div()
                    .flex_shrink_0()
                    .text_color(theme::fg())
                    .when(selected, |d| d.bg(gpui::rgb(0x264f78)))
                    .child(display)
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, ev: &gpui::MouseDownEvent, window, cx| {
                            this.switch_input_target(InputTarget::Chat);
                            this.chat.move_to(i, ev.modifiers.shift);
                            window.focus(&this.focus_handle);
                            cx.notify();
                        }),
                    );
                input_el = input_el.child(cell);
            }
            if !caret_drawn {
                input_el = input_el.child(chat_caret());
            }
        }

        // ===== 思考等级面板：内联在输入框上方（与模型选择器同一种做法，不做浮层）=====
        if self.thinking_panel {
            col = col.child(self.thinking_level_panel(cx));
        }

        // ===== 权限面板：同样内联在输入框上方，与思考面板互斥 =====
        if self.permission_panel {
            col = col.child(self.permission_level_panel(cx));
        }

        // ===== 待发附件：粘贴 / 上传 / 拖拽进来的文件都汇总在这里 =====
        if !self.chat_attachments.is_empty() {
            col = col.child(self.attachment_strip(cx));
        }

        // ===== 输入框：大圆角容器 + 左下「＋」+ 右下权限 chip =====
        let mut input_box = div()
            .rounded_lg()
            .border_1()
            .border_color(if self.is_running() {
                theme::accent_line()
            } else {
                theme::border()
            })
            .bg(theme::panel())
            .px_4()
            .pt_3()
            .pb_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(input_el);

        // 「＋」菜单：上传附件 / 引用文件 / 工具开关 / 清空。
        // 比在底栏摆一排文字按钮干净，也不会挤压窄窗口下的输入区。
        if self.composer_menu {
            input_box = input_box.child(self.composer_menu_row(has_open_file, cx));
        }

        // 拖拽文件到输入框 = 上传附件（与「＋ → 上传文件」同一条路径）。
        let entity = cx.entity();
        input_box = input_box.on_drop::<gpui::ExternalPaths>(move |paths, _window, cx| {
            let files = paths.paths().to_vec();
            entity.update(cx, |this, cx| this.attach_paths(files, cx));
        });

        col = col.child(
            input_box
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        // 左：＋ 菜单（引用文件 / 工具 / 清空）
                        .child(self.composer_plus_button(cx))
                        .child(div().flex_1())
                        // 右：权限 chip（常驻，显示当前会话档位）
                        .child(self.permission_chip(cx))
                        // 发送走 Enter，不再占一个按钮。
                        // 只有生成中才出现「停止」，否则没有任何中断入口。
                        .when(self.is_running(), |d| {
                            d.child(
                                div()
                                    .id("stop-btn")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .h(px(28.0))
                                    .px_3()
                                    .rounded_md()
                                    .bg(theme::raised())
                                    .text_xs()
                                    .text_color(theme::yellow())
                                    .cursor_pointer()
                                    .hover(|d| d.opacity(0.85))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                        this.stop_streaming(cx);
                                    }))
                                    .child("■ 停止"),
                            )
                        })
                        // 生成中已经有「停止」按钮和顶部状态，这里不再重复提示。
                        .when(!self.is_running() && self.layout_w.get() >= 980.0, |d| {
                            d.child(
                                div()
                                    .text_xs()
                                    .text_color(theme::dim())
                                    .child("Enter 发送 · Shift+Enter 换行"),
                            )
                        }),
                ),
        );

        col
    }

    /// 把当前打开的文件名以 @ 形式插入输入框。
    pub(crate) fn insert_file_reference(&mut self, cx: &mut Context<Self>) {
        if self.open_path.is_none() && self.remote_open_rel.is_none() {
            self.notice = Some("还没有打开文件".into());
            cx.notify();
            return;
        }
        let name = self.open_file_name();
        self.switch_input_target(InputTarget::Chat);
        self.chat.insert(&format!("@{name} "));
        cx.notify();
    }

    /// 输入框上方的待发附件条：图片给缩略图，文本文件给类型徽标。
    fn attachment_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div().flex().items_center().gap_2().flex_wrap();
        for (i, a) in self.chat_attachments.iter().enumerate() {
            let is_image = a.kind == neverwork_core::AttachmentKind::Image;
            let mut card = div()
                .id(SharedString::from(format!("attach-{i}")))
                .flex()
                .items_center()
                .gap_2()
                .h(px(40.0))
                .pl_1()
                .pr_2()
                .rounded_md()
                .border_1()
                .border_color(theme::border())
                .bg(theme::raised());
            if is_image {
                card = card.child(
                    div()
                        .size(px(32.0))
                        .flex_shrink_0()
                        .rounded_sm()
                        .overflow_hidden()
                        .child(
                            img(std::path::PathBuf::from(&a.path))
                                .size(px(32.0))
                                .object_fit(gpui::ObjectFit::Cover),
                        ),
                );
            } else {
                card = card.child(super::files::file_badge(&a.name));
            }
            card = card
                .child(
                    div()
                        .max_w(px(150.0))
                        .text_xs()
                        .text_color(theme::muted())
                        .whitespace_nowrap()
                        .child(SharedString::from(fit_text(&a.name, 140.0, 11.0))),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("attach-x-{i}")))
                        .px_1()
                        .rounded_sm()
                        .text_xs()
                        .text_color(theme::dim())
                        .cursor_pointer()
                        .hover(|d| d.text_color(theme::fg()))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                            this.remove_attachment(i, cx);
                        }))
                        .child("✕"),
                );
            row = row.child(card);
        }
        row
    }

    /// 输入框左下角的圆形「＋」：展开/收起工具菜单。
    fn composer_plus_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.composer_menu;
        div()
            .id("composer-plus")
            .flex()
            .items_center()
            .justify_center()
            .size(px(26.0))
            .flex_shrink_0()
            .rounded_full()
            .border_1()
            .border_color(if open {
                theme::accent_line()
            } else {
                theme::border()
            })
            .bg(if open { theme::accent_soft() } else { theme::raised() })
            .text_sm()
            .text_color(if open { theme::accent() } else { theme::muted() })
            .cursor_pointer()
            .hover(|d| d.border_color(theme::accent_line()).text_color(theme::fg()))
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.composer_menu = !this.composer_menu;
                // 三个面板互斥，否则叠在一起把输入框顶得很高
                if this.composer_menu {
                    this.thinking_panel = false;
                    this.thinking_dragging = false;
                    this.permission_panel = false;
                    this.permission_confirm = false;
                }
                cx.notify();
            }))
            .child(if open { "−" } else { "+" })
    }

    /// 底栏权限 chip：显示当前会话的权限档位，点击展开/收起权限面板。
    fn permission_chip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use neverwork_core::PermissionMode;
        let mode = self.permission();
        let open = self.permission_panel;
        let enabled = self.tools_enabled;
        let color = match mode {
            PermissionMode::ReadOnly => theme::muted(),
            PermissionMode::Plan => theme::accent(),
            PermissionMode::Standard => theme::green(),
            PermissionMode::Full => theme::yellow(),
        };
        div()
            .id("permission-chip")
            .flex()
            .items_center()
            .h(px(28.0))
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(if open {
                theme::accent_line()
            } else {
                theme::border()
            })
            .bg(if open {
                theme::accent_soft()
            } else {
                theme::raised()
            })
            .text_xs()
            .text_color(if enabled { color } else { theme::dim() })
            .cursor_pointer()
            .hover(|d| d.border_color(theme::accent_line()))
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                if !this.tools_enabled {
                    this.notice = Some("工具已关闭：先在「＋」菜单开启工具再调权限".into());
                    cx.notify();
                    return;
                }
                this.permission_panel = !this.permission_panel;
                this.permission_confirm = false;
                if this.permission_panel {
                    this.composer_menu = false;
                    this.thinking_panel = false;
                    this.thinking_dragging = false;
                }
                cx.notify();
            }))
            .child(SharedString::from(format!(
                "{} {}权限",
                mode.glyph(),
                mode.label()
            )))
    }

    /// 权限面板：四档卡片 + 「完整权限」确认条。
    fn permission_level_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use neverwork_core::PermissionMode;
        let current = self.permission();
        let mut col = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .py_3()
            .rounded_lg()
            .border_1()
            .border_color(theme::accent_line())
            .bg(theme::panel())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::fg())
                            .child(SharedString::from(format!(
                                "{} 权限 · {}",
                                current.glyph(),
                                current.label()
                            ))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(theme::dim())
                            .child("随会话保存；切换后下一条消息生效"),
                    )
                    .child(
                        div()
                            .id("permission-close")
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(theme::raised())
                            .text_xs()
                            .text_color(theme::muted())
                            .cursor_pointer()
                            .hover(|d| d.text_color(theme::fg()))
                            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                this.permission_panel = false;
                                this.permission_confirm = false;
                                cx.notify();
                            }))
                            .child("收起"),
                    ),
            );
        for (i, mode) in PermissionMode::ALL.iter().enumerate() {
            let mode = *mode;
            let active = mode == current;
            let color = match mode {
                PermissionMode::ReadOnly => theme::muted(),
                PermissionMode::Plan => theme::accent(),
                PermissionMode::Standard => theme::green(),
                PermissionMode::Full => theme::yellow(),
            };
            col = col.child(
                div()
                    .id(SharedString::from(format!("permission-mode-{i}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(if active {
                        theme::accent_line()
                    } else {
                        theme::border()
                    })
                    .bg(if active {
                        theme::accent_soft()
                    } else {
                        theme::raised()
                    })
                    .cursor_pointer()
                    .hover(|d| d.border_color(theme::accent_line()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        // 完整权限风险最高：先确认，确认后才真正切过去。
                        if mode == PermissionMode::Full
                            && this.permission() != PermissionMode::Full
                        {
                            this.permission_confirm = true;
                            cx.notify();
                        } else {
                            this.set_permission(mode, cx);
                        }
                    }))
                    .child(
                        div()
                            .w(px(20.0))
                            .text_sm()
                            .text_color(color)
                            .child(mode.glyph()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .when(active, |d| d.text_color(theme::accent()))
                                    .when(!active, |d| d.text_color(theme::fg()))
                                    .child(mode.label()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::dim())
                                    .child(mode.desc()),
                            ),
                    )
                    .child(
                        div()
                            .w(px(18.0))
                            .text_sm()
                            .text_color(if active {
                                theme::accent()
                            } else {
                                theme::dim()
                            })
                            .child(if active { "✓" } else { "" }),
                    ),
            );
        }
        if self.permission_confirm {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(theme::yellow())
                    .bg(theme::raised())
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(theme::yellow())
                            .child("完整权限会跳过写操作与命令的审批，本会话内生效。确认开启？"),
                    )
                    .child(
                        div()
                            .id("permission-cancel")
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(theme::panel())
                            .text_xs()
                            .text_color(theme::muted())
                            .cursor_pointer()
                            .hover(|d| d.text_color(theme::fg()))
                            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                this.permission_confirm = false;
                                cx.notify();
                            }))
                            .child("取消"),
                    )
                    .child(
                        div()
                            .id("permission-confirm")
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::yellow())
                            .bg(theme::raised())
                            .text_xs()
                            .text_color(theme::yellow())
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::accent_soft()))
                            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                this.set_permission(neverwork_core::PermissionMode::Full, cx);
                            }))
                            .child("确认开启"),
                    ),
            );
        }
        col.child(
            div()
                .text_xs()
                .text_color(theme::dim())
                .child("只读/计划会隐藏写文件与命令工具；完整权限只免审批，路径沙箱与超时限制不变。"),
        )
    }

    /// 「＋」展开的那一行：引用文件 / 工具开关 / 思考档位 / 清空。
    fn composer_menu_row(
        &self,
        has_open_file: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tools_on = self.tools_enabled;
        let level = self.thinking_level();
        let thinking_on = !level.is_auto();
        div()
            .flex()
            .items_center()
            .gap_2()
            .flex_wrap()
            // 上传附件（图片 / 文本文件，多选）
            .child(
                div()
                    .id("composer-attach")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(theme::raised())
                    .text_xs()
                    .text_color(theme::muted())
                    .cursor_pointer()
                    .hover(|d| d.text_color(theme::accent()))
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.pick_attachments(cx);
                    }))
                    .child("＋ 上传图片/文件"),
            )
            // 引用当前打开的文件
            .child(
                div()
                    .id("composer-ref")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(theme::raised())
                    .text_xs()
                    .when(has_open_file, |d| d.text_color(theme::muted()))
                    .when(!has_open_file, |d| d.text_color(theme::dim()))
                    .cursor_pointer()
                    .hover(|d| d.text_color(theme::accent()))
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.insert_file_reference(cx);
                    }))
                    .child("@ 引用文件"),
            )
            // 工具开关
            .child(
                div()
                    .id("toggle-tools")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_xs()
                    .cursor_pointer()
                    .when(tools_on, |d| {
                        d.bg(theme::accent_soft())
                            .text_color(theme::accent())
                            .hover(|d| d.opacity(0.85))
                    })
                    .when(!tools_on, |d| {
                        d.bg(theme::raised())
                            .text_color(theme::dim())
                            .hover(|d| d.text_color(theme::fg()))
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.toggle_composer_tools(cx);
                    }))
                    .child(if tools_on { "⚒ 工具已开" } else { "⚒ 工具已关" }),
            )
            // 思考档位：入口收进「＋」菜单，输入框底栏不再常驻。
            .child(
                div()
                    .id("composer-thinking")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_xs()
                    .cursor_pointer()
                    .when(thinking_on, |d| {
                        d.bg(theme::accent_soft())
                            .text_color(theme::accent())
                            .hover(|d| d.opacity(0.85))
                    })
                    .when(!thinking_on, |d| {
                        d.bg(theme::raised())
                            .text_color(theme::muted())
                            .hover(|d| d.text_color(theme::fg()))
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.thinking_panel = !this.thinking_panel;
                        this.composer_menu = false;
                        this.thinking_dragging = false;
                        this.permission_panel = false;
                        this.permission_confirm = false;
                        cx.notify();
                    }))
                    .child(SharedString::from(format!("⚙ 思考 · {}", level.label()))),
            )
            .child(
                div()
                    .id("composer-clear")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(theme::raised())
                    .text_xs()
                    .text_color(theme::muted())
                    .cursor_pointer()
                    .hover(|d| d.text_color(theme::fg()))
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.chat.reset("");
                        this.notice = None;
                        cx.notify();
                    }))
                    .child("清空"),
            )
    }

    /// 工具开关（从输入框菜单触发）。
    pub(crate) fn toggle_composer_tools(&mut self, cx: &mut Context<Self>) {
        self.tools_enabled = !self.tools_enabled;
        self.notice = Some(if self.tools_enabled {
            format!(
                "已开启工具：当前「{}」权限，点底栏权限 chip 可切换",
                self.permission().label()
            )
        } else {
            "已关闭工具：只做纯对话".into()
        });
        cx.notify();
    }

    /// 思考等级面板：当前档位 / 当前模型 / 4 档滑杆。
    fn thinking_level_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let level = self.thinking_level();
        let idx = level.index();
        let (provider, model) = self.session_model_labels(self.active_session);
        let model_label = if model.is_empty() {
            "未配置模型".to_string()
        } else {
            format!("{provider} · {model}")
        };
        // 档位停在的格子中心，滑杆填充到那里（4 格等宽）
        let track_w = THINK_CELL * 3.0;
        let fill_w = idx as f32 * THINK_CELL;
        // 轨道范围给 cursor 换算档位；拖动用窗口级监听（见 canvas）
        let track_slot = self.think_track.clone();
        let weak = cx.entity().downgrade();

        let mut slider = div()
            .id("think-track")
            .relative()
            .h(px(30.0))
            .w(px(track_w + THINK_CELL))
            .cursor_pointer()
            .child(
                // 记录轨道坐标；拖动监听必须走窗口级：
                // 元素级 on_mouse_move 只在悬停时触发，光标一滑出这条轨道就断，
                // 滑杆会「粘」在半路（尤其是往两端外拖的时候）。
                gpui::canvas(
                    |_b, _w, _cx| {},
                    move |bounds, _w, window, _cx| {
                        track_slot.set(Some(bounds));
                        let weak_move = weak.clone();
                        window.on_mouse_event(move |ev: &gpui::MouseMoveEvent, phase, _w, cx| {
                            if phase != gpui::DispatchPhase::Bubble {
                                return;
                            }
                            let Some(this) = weak_move.upgrade() else {
                                return;
                            };
                            let x = ev.position.x;
                            let pressed = ev.pressed_button == Some(gpui::MouseButton::Left);
                            let _ = this.update(cx, |this, cx| {
                                if !this.thinking_dragging {
                                    return;
                                }
                                if pressed {
                                    this.set_thinking_from_x(x, cx);
                                } else {
                                    // 在窗口外 / 输入桥外松开时收不到 mouse up，靠这里兜底落盘
                                    this.thinking_dragging = false;
                                    this.save_sessions();
                                    cx.notify();
                                }
                            });
                        });
                        let weak_up = weak.clone();
                        window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _w, cx| {
                            if phase != gpui::DispatchPhase::Bubble {
                                return;
                            }
                            let Some(this) = weak_up.upgrade() else {
                                return;
                            };
                            let _ = this.update(cx, |this, cx| {
                                if this.thinking_dragging {
                                    this.thinking_dragging = false;
                                    this.save_sessions();
                                    cx.notify();
                                }
                            });
                        });
                    },
                )
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .w(px(track_w + THINK_CELL))
                .h(px(30.0)),
            )
            // 按下即吸到最近一档；之后的移动由上面的窗口级监听接管
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, ev: &gpui::MouseDownEvent, _w, cx| {
                    this.thinking_dragging = true;
                    this.set_thinking_from_x(ev.position.x, cx);
                }),
            )
            .child(
                // 底轨
                div()
                    .absolute()
                    .left(px(THINK_CELL / 2.0))
                    .top(px(13.0))
                    .h(px(4.0))
                    .w(px(track_w))
                    .rounded_full()
                    .bg(theme::raised()),
            )
            .child(
                // 已选段（绿色，与图里的滑杆一致）
                div()
                    .absolute()
                    .left(px(THINK_CELL / 2.0))
                    .top(px(13.0))
                    .h(px(4.0))
                    .w(px(fill_w))
                    .rounded_full()
                    .bg(if idx == 0 {
                        theme::raised()
                    } else {
                        theme::green()
                    }),
            );

        for (i, lv) in neverwork_core::ThinkingLevel::ALL.iter().enumerate() {
            let active = i == idx;
            let passed = i <= idx;
            slider = slider.child(
                div()
                    .id(SharedString::from(format!("think-stop-{i}")))
                    .absolute()
                    .left(px((i as f32 * THINK_CELL) + (THINK_CELL - 24.0) / 2.0))
                    .top(px(3.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(24.0))
                    .rounded_full()
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::raised()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.set_thinking_level(*lv, cx);
                    }))
                    .child(
                        div()
                            .size(if active { px(14.0) } else { px(8.0) })
                            .rounded_full()
                            .bg(if active {
                                theme::fg()
                            } else if passed {
                                theme::green()
                            } else {
                                theme::raised()
                            })
                            .border_1()
                            .border_color(if passed {
                                theme::green()
                            } else {
                                theme::border()
                            }),
                    ),
            );
        }

        let mut labels = div().flex().w(px(track_w + THINK_CELL));
        for (i, lv) in neverwork_core::ThinkingLevel::ALL.iter().enumerate() {
            labels = labels.child(
                div()
                    .id(SharedString::from(format!("think-label-{i}")))
                    .w(px(THINK_CELL))
                    .flex()
                    .justify_center()
                    .text_xs()
                    .text_color(if i == idx { theme::fg() } else { theme::dim() })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.set_thinking_level(*lv, cx);
                    }))
                    .child(lv.label()),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .py_3()
            .rounded_lg()
            .border_1()
            .border_color(theme::accent_line())
            .bg(theme::panel())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if level.is_auto() {
                                theme::muted()
                            } else {
                                theme::accent()
                            })
                            .child(level.label()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(theme::dim())
                            .child(SharedString::from(fit_text(&model_label, 320.0, 11.0))),
                    )
                    .child(
                        div()
                            .id("think-reset")
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(theme::raised())
                            .text_xs()
                            .text_color(theme::muted())
                            .cursor_pointer()
                            .hover(|d| d.text_color(theme::accent()))
                            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                this.set_thinking_level(neverwork_core::ThinkingLevel::Auto, cx);
                            }))
                            .child("重置为自动"),
                    )
                    .child(
                        div()
                            .id("think-close")
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(theme::raised())
                            .text_xs()
                            .text_color(theme::muted())
                            .cursor_pointer()
                            .hover(|d| d.text_color(theme::fg()))
                            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                this.thinking_panel = false;
                                this.thinking_dragging = false;
                                cx.notify();
                            }))
                            .child("收起"),
                    ),
            )
            .child(slider)
            .child(labels)
            .child(
                div()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("可拖动滑杆（或点刻度）；随会话保存；自动 = 不向厂商发送思考参数。网关不支持时会自动去掉重试。"),
            )
    }

    /// 按鼠标横坐标把思考档位吸到最近一格（点滑杆与拖动共用）。
    ///
    /// 拖动中只改内存（`preview_thinking_level`），松开鼠标才落盘：
    /// 一次拖过两三档时每档都写一遍全部会话会卡手。
    fn set_thinking_from_x(&mut self, x: gpui::Pixels, cx: &mut Context<Self>) {
        let Some(bounds) = self.think_track.get() else {
            return;
        };
        // 轨道两端各留半格：在端点外侧按下也能吸到端点档
        let local = f32::from(x - bounds.left());
        let idx = ((local - THINK_CELL / 2.0) / THINK_CELL).round().clamp(0.0, 3.0) as usize;
        self.preview_thinking_level(neverwork_core::ThinkingLevel::from_index(idx), cx);
    }

    /// 当前会话的思考等级（没选过就是自动）。
    pub(crate) fn thinking_level(&self) -> neverwork_core::ThinkingLevel {
        self.sessions
            .get(self.active_session)
            .map(|s| s.thinking)
            .unwrap_or_default()
    }

    /// 改当前会话的思考等级并落盘（点刻度 / 标签 / 重置按钮走这里）。
    pub(crate) fn set_thinking_level(
        &mut self,
        level: neverwork_core::ThinkingLevel,
        cx: &mut Context<Self>,
    ) {
        let idx = self.active_session.min(self.sessions.len().saturating_sub(1));
        if let Some(s) = self.sessions.get_mut(idx) {
            s.thinking = level;
        }
        self.save_sessions();
        cx.notify();
    }

    /// 拖动中的档位：只改内存 + 重绘，不落盘。
    ///
    /// 拖一次会连续跨过好几个档位，每一档都把全部会话写一遍盘会卡手；
    /// 松开鼠标再由滑杆的 mouse up 监听 `save_sessions` 一次。
    pub(crate) fn preview_thinking_level(
        &mut self,
        level: neverwork_core::ThinkingLevel,
        cx: &mut Context<Self>,
    ) {
        if self.thinking_level() == level {
            return;
        }
        let idx = self.active_session.min(self.sessions.len().saturating_sub(1));
        if let Some(s) = self.sessions.get_mut(idx) {
            s.thinking = level;
        }
        cx.notify();
    }
    /// 当前会话的权限档位（没设置过就是标准）。
    pub(crate) fn permission(&self) -> neverwork_core::PermissionMode {
        self.sessions
            .get(self.active_session)
            .map(|s| s.permission)
            .unwrap_or_default()
    }

    /// 改当前会话的权限档位并落盘；面板随即收起。
    pub(crate) fn set_permission(
        &mut self,
        mode: neverwork_core::PermissionMode,
        cx: &mut Context<Self>,
    ) {
        let idx = self.active_session.min(self.sessions.len().saturating_sub(1));
        if let Some(s) = self.sessions.get_mut(idx) {
            s.permission = mode;
        }
        self.permission_panel = false;
        self.permission_confirm = false;
        self.save_sessions();
        self.notice = Some(match mode {
            neverwork_core::PermissionMode::ReadOnly => {
                "已切到只读权限：写文件与执行命令的工具不会提供".into()
            }
            neverwork_core::PermissionMode::Plan => {
                "已切到计划权限：先出实施计划，确认后切到标准或完整执行".into()
            }
            neverwork_core::PermissionMode::Standard => {
                "已切到标准权限：写操作与命令会逐次弹审批".into()
            }
            neverwork_core::PermissionMode::Full => {
                "已切到完整权限：写操作与命令直接执行，不再逐次审批".into()
            }
        });
        cx.notify();
    }
}

fn chat_caret() -> impl IntoElement {
    div()
        .w(px(2.0))
        .h(px(20.0))
        .flex_shrink_0()
        .bg(theme::accent())
}

pub(crate) fn flow_step(initial: &str, idx: usize, label: &str, active: bool) -> impl IntoElement {
    let color = theme::role_color(idx);
    let text = if active { theme::fg() } else { theme::dim() };
    // 角色没填头像字时退回名字首字，别渲染一个空色块
    let initial = if initial.trim().is_empty() {
        label.chars().next().map(|c| c.to_string()).unwrap_or_default()
    } else {
        initial.to_string()
    };
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(18.0))
                .rounded_md()
                .bg(color)
                .text_xs()
                .text_color(theme::bg())
                .child(SharedString::from(initial)),
        )
        .child(div().text_xs().text_color(text).child(SharedString::from(label.to_string())))
}

impl AppState {
    /// 渲染一条会话消息。`pending_here` 为真时给写操作卡片加上允许/拒绝按钮。
    pub(crate) fn render_msg(
        &self,
        m: &neverwork_core::Message,
        avail: f32,
        idx: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match &m.kind {
            MessageKind::User => {
                let body = m.body.clone();
                user_bubble(
                    &m.body,
                    &m.attachments,
                    copy_chip(
                        SharedString::from(format!("copy-user-{idx}")),
                        cx.listener(move |this, _: &ClickEvent, _w, cx| {
                            this.copy_message(body.clone(), "提问", cx);
                        }),
                    ),
                )
                .into_any_element()
            }
            MessageKind::Agent { participant } => {
                let (model, provider) = self.model_labels();
                let body = m.body.clone();
                if self.is_team_mode() {
                    // 角色类型标签是展示信息，跟着会话所属团队走；找不到就只显名字。
                    let kind_label = self
                        .team_for_session(self.active_session)
                        .and_then(|t| {
                            t.roles
                                .iter()
                                .find(|r| r.name.trim() == participant.role_name.trim())
                                .map(|r| r.kind_label.clone())
                        })
                        .unwrap_or_default();
                    team_bubble(
                        &participant.role_name,
                        &kind_label,
                        participant.initial.clone(),
                        participant.color_index,
                        &m.body,
                        m.code.as_ref(),
                        m.elapsed_ms,
                        (m.tokens_in, m.tokens_out),
                        &model,
                        &provider,
                        avail,
                        self.root.as_deref(),
                        Some((
                            gpui::ElementId::from(SharedString::from(format!(
                                "copy-agent-{idx}"
                            ))),
                            Box::new(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                                this.copy_message(body.clone(), "回答", cx);
                            })) as ClickCb,
                        )),
                        cx,
                    )
                    .into_any_element()
                } else {
                    agent_bubble(
                        &participant.role_name,
                        &participant.role_id,
                        participant.color_index,
                        &m.body,
                        m.code.as_ref(),
                        m.elapsed_ms,
                        (m.tokens_in, m.tokens_out),
                        &model,
                        &provider,
                        avail,
                        self.root.as_deref(),
                        Some((
                            gpui::ElementId::from(SharedString::from(format!(
                                "copy-agent-{idx}"
                            ))),
                            Box::new(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                                this.copy_message(body.clone(), "回答", cx);
                            })) as ClickCb,
                        )),
                        cx,
                    )
                    .into_any_element()
                }
            }
            MessageKind::Handoff {
                from,
                to,
                action,
                detail,
            } => {
                let expanded = self.expanded_handoff.contains(&idx);
                handoff_card(
                    idx,
                    from,
                    to,
                    action,
                    detail,
                    expanded,
                    Box::new(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        if !this.expanded_handoff.remove(&idx) {
                            this.expanded_handoff.insert(idx);
                        }
                        cx.notify();
                    })),
                )
                .into_any_element()
            }
            MessageKind::Tool { summary, .. } => tool_result(summary).into_any_element(),
            MessageKind::Parallel { items } => parallel_row(items).into_any_element(),
            // 工具调用统一由 `tool_steps_group` 聚合渲染。
            MessageKind::ToolCall { .. } => div().into_any_element(),
        }
    }

    /// 一组连续的工具调用：**只渲染一行统计**（线程 / 思考轮次 / 工具数 / 动作），
    /// 逐条的路径与输出不再铺开；写操作的审批仍单独展示 diff 与按钮。
    fn tool_steps_group(
        &self,
        start: usize,
        end: usize,
        pending_here: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        use neverwork_core::ToolRunStatus;

        let sidx = self.active_session;
        let session = match self.sessions.get(sidx) {
            Some(s) => s,
            None => return div().into_any_element(),
        };

        struct Step {
            idx: usize,
            name: String,
            path: String,
            summary: String,
            status: ToolRunStatus,
            thread: Option<usize>,
            round: Option<usize>,
            diff: Vec<neverwork_core::DiffLineView>,
        }

        let mut steps: Vec<Step> = Vec::new();
        for i in start..end.min(session.messages.len()) {
            if let MessageKind::ToolCall {
                name,
                path,
                summary,
                status,
                thread,
                round,
                diff,
                ..
            } = &session.messages[i].kind
            {
                steps.push(Step {
                    idx: i,
                    name: name.clone(),
                    path: path.clone(),
                    summary: summary.clone(),
                    status: *status,
                    thread: *thread,
                    round: *round,
                    diff: diff.clone(),
                });
            }
        }
        if steps.is_empty() {
            return div().into_any_element();
        }

        // 线程 = 参与这组工具调用的角色数（单 Agent 恒 1）；
        // 轮次 = 去重后的（线程, 轮序）对，一个角色的一轮模型思考算一轮。
        // 旧会话没有这两个字段（`None`）：宁可不显示，也不编一个错的数字。
        let mut threads: Vec<usize> = steps.iter().filter_map(|s| s.thread).collect();
        threads.sort_unstable();
        threads.dedup();
        let mut rounds: Vec<(usize, usize)> = steps
            .iter()
            .filter_map(|s| Some((s.thread?, s.round?)))
            .collect();
        rounds.sort_unstable();
        rounds.dedup();

        // 动作直方图：按次数降序聚合成一段轻量统计
        let mut actions: Vec<(&'static str, usize)> = Vec::new();
        for s in &steps {
            let action = tool_action(&s.name);
            match actions.iter_mut().find(|(name, _)| *name == action) {
                Some((_, count)) => *count += 1,
                None => actions.push((action, 1)),
            }
        }
        actions.sort_by(|a, b| b.1.cmp(&a.1));

        // 统计口径只拼实际有数据的项：旧会话只有工具数与动作
        let mut metrics: Vec<String> = Vec::new();
        if !threads.is_empty() {
            metrics.push(format!("{} 线程", threads.len()));
        }
        if !rounds.is_empty() {
            metrics.push(format!("{} 轮思考", rounds.len()));
        }
        metrics.push(format!("{} 个工具", steps.len()));

        let mut head = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .py_0p5()
            .text_xs()
            .text_color(theme::dim())
            .child(div().text_color(theme::muted()).child("⚒"))
            .child(
                div()
                    .text_color(theme::muted())
                    .child(SharedString::from(metrics.join(" · "))),
            );
        for (name, count) in actions.iter().take(4) {
            head = head
                .child(div().child("·"))
                .child(SharedString::from(format!("{name} {count}")));
        }
        if actions.len() > 4 {
            head = head.child(SharedString::from(format!("· +{}", actions.len() - 4)));
        }

        // 异常状态只给计数，不点名到具体步骤
        let running = steps.iter().any(|s| s.status == ToolRunStatus::Running);
        let awaiting = steps
            .iter()
            .filter(|s| s.status == ToolRunStatus::Awaiting)
            .count();
        let failed = steps
            .iter()
            .filter(|s| s.status == ToolRunStatus::Failed)
            .count();
        let stopped = steps
            .iter()
            .filter(|s| matches!(s.status, ToolRunStatus::Denied | ToolRunStatus::Cancelled))
            .count();
        if running {
            head = head.child(div().text_color(theme::accent()).child("· 执行中"));
        }
        if awaiting > 0 {
            head = head.child(
                div()
                    .text_color(theme::yellow())
                    .child(SharedString::from(format!("· 待确认 {awaiting}"))),
            );
        }
        if failed > 0 {
            head = head.child(
                div()
                    .text_color(theme::red())
                    .child(SharedString::from(format!("· 失败 {failed}"))),
            );
        }
        if stopped > 0 {
            head = head.child(SharedString::from(format!("· 中断 {stopped}")));
        }

        let mut col = div().flex().flex_col().gap_1().child(head);

        // 待审批：只有真正在等用户决定的写操作才展示内容与按钮，
        // 否则用户没地方点「允许」。
        let pending_idx = self
            .pending
            .as_ref()
            .filter(|p| p.session_idx == sidx)
            .map(|p| p.msg_idx);
        if let Some(step) = steps
            .iter()
            .find(|s| Some(s.idx) == pending_idx && s.status == ToolRunStatus::Awaiting)
        {
            let step_idx = step.idx;
            // 命令执行不是改文件：文案、参数与按钮都要跟着换，否则用户会以为
            // 「允许并写入」是往磁盘写东西，而不是跑一条命令。
            let is_command = step.name == "run_command";
            let arg = if is_command {
                step.summary.clone()
            } else if step.path.is_empty() {
                String::new()
            } else if let Some(t) = &self.remote {
                let mut p = t.root.trim_end_matches('/').to_string();
                if !step.path.starts_with('/') {
                    p.push('/');
                }
                p.push_str(&step.path);
                format!("{}:{p}", t.host)
            } else if let Some(root) = &self.root {
                root.join(&step.path).display().to_string()
            } else {
                neverwork_agent::tools::ToolContext::home_dir()
                    .join(&step.path)
                    .display()
                    .to_string()
            };
            let mut approve = div()
                .flex()
                .flex_col()
                .gap_1()
                .pl(px(14.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::yellow())
                                .child(if is_command {
                                    "会执行命令，确认后运行："
                                } else {
                                    "会改文件，确认后写入："
                                }),
                        )
                        .child(
                            div()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_xs()
                                .text_color(theme::muted())
                                .child(SharedString::from(format!(
                                    "{} {arg}",
                                    tool_action(&step.name)
                                ))),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .id(SharedString::from(format!("deny-{step_idx}")))
                                .px_2()
                                .py_0p5()
                                .rounded_sm()
                                .bg(theme::raised())
                                .text_color(theme::muted())
                                .cursor_pointer()
                                .hover(|d| d.bg(theme::border()))
                                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                                    this.answer_approval(false, cx);
                                }))
                                .child("拒绝"),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("allow-{step_idx}")))
                                .px_2()
                                .py_0p5()
                                .rounded_sm()
                                .bg(theme::green())
                                .text_color(gpui::rgb(0x000000))
                                .cursor_pointer()
                                .hover(|d| d.opacity(0.88))
                                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                                    this.answer_approval(true, cx);
                                }))
                                .child(if is_command { "允许并执行" } else { "允许并写入" }),
                        ),
                );
            if !step.diff.is_empty() {
                approve = approve.child(self.diff_box(step_idx, &step.diff));
            }
            col = col.child(approve);
        }

        // ask_user 待回答：问题 + 推荐选项 + 自定义输入行。
        // 与审批卡同位置渲染（统计行下方），否则用户没地方点。
        //
        // 只在自己的工具组后渲染：先前只按 session 过滤，历史里每一组工具调用
        // 后面都会重复画一张卡（截图里同一问题出现两次），两份卡片的元素 id
        // 也完全相同，点击/焦点状态在 GlobalElementId 上互相踩，按钮就点不动了。
        let last_tool_group = !session
            .messages
            .get(end..)
            .unwrap_or_default()
            .iter()
            .any(|m| matches!(m.kind, MessageKind::ToolCall { .. }));
        if let Some(ask) = self.pending_ask.as_ref().filter(|p| {
            p.session_idx == sidx && ask_card_in_group(p.msg_idx, start, end, last_tool_group)
        }) {
            let ask_focused = self.input_target == InputTarget::AskCustom;
            let custom = self.ask_custom.text().to_string();
            let custom_placeholder = custom.is_empty();
            let mut ask_box = div()
                .flex()
                .flex_col()
                .gap_1p5()
                .pl(px(14.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::accent())
                                .child("❓ 需要你决定"),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .id(SharedString::from(format!("ask-skip-{start}")))
                                .px_2()
                                .py_0p5()
                                .rounded_sm()
                                .bg(theme::raised())
                                .text_color(theme::muted())
                                .cursor_pointer()
                                .hover(|d| d.bg(theme::border()))
                                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                    this.skip_ask(cx);
                                }))
                                .child("跳过"),
                        ),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::fg())
                        .child(SharedString::from(ask.question.clone())),
                );
            // 选项按钮：第一个是推荐项（绿底），其余灰底；description 小字附在下面
            let mut opts = div().flex().flex_wrap().items_start().gap_1p5();
            for (i, opt) in ask.options.iter().enumerate() {
                let label = opt.label.clone();
                let mut btn = div()
                    .id(SharedString::from(format!("ask-opt-{start}-{i}")))
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .px_2p5()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.answer_ask(label.clone(), cx);
                    }));
                btn = if i == 0 {
                    btn.bg(theme::green())
                        .text_color(gpui::rgb(0x000000))
                        .hover(|d| d.opacity(0.88))
                } else {
                    btn.bg(theme::raised())
                        .text_color(theme::fg())
                        .hover(|d| d.bg(theme::border()))
                };
                btn = btn.child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(SharedString::from(opt.label.clone())),
                );
                if !opt.description.is_empty() {
                    btn = btn.child(
                        div()
                            .text_size(px(10.0))
                            .opacity(0.8)
                            .max_w(px(220.0))
                            .child(SharedString::from(opt.description.clone())),
                    );
                }
                opts = opts.child(btn);
            }
            ask_box = ask_box.child(opts);
            if ask.allow_custom {
                let mut input_row = div().flex().items_center().gap_1p5().child(
                    div()
                        .id(SharedString::from(format!("ask-custom-{start}")))
                        .flex_1()
                        .min_w(px(0.0))
                        .px_2p5()
                        .py_1()
                        .rounded_md()
                        .bg(theme::raised())
                        .border_1()
                        .border_color(if ask_focused {
                            theme::accent_line()
                        } else {
                            theme::border()
                        })
                        .cursor(CursorStyle::IBeam)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(12.0))
                        .text_color(if custom_placeholder {
                            theme::dim()
                        } else {
                            theme::fg()
                        })
                        .track_focus(&self.focus_handle)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.switch_input_target(InputTarget::AskCustom);
                            let end = this.ask_custom.text().len();
                            this.ask_custom.move_to(end, false);
                            window.focus(&this.focus_handle);
                            cx.notify();
                        }))
                        .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _w, cx| {
                            this.on_ask_custom_key(ev, cx);
                        }))
                        .child(SharedString::from(if custom_placeholder {
                            "或输入自定义答案…".to_string()
                        } else {
                            fit_text(&custom, 380.0, 12.0)
                        })),
                );
                // 有内容才显示「发送」，否则只有输入框（回车同效）
                if !custom_placeholder {
                    input_row = input_row.child(
                        div()
                            .id(SharedString::from(format!("ask-send-{start}")))
                            .px_2p5()
                            .py_1()
                            .rounded_md()
                            .bg(theme::accent())
                            .text_color(gpui::rgb(0x000000))
                            .cursor_pointer()
                            .hover(|d| d.opacity(0.88))
                            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                let text = this.ask_custom.text().trim().to_string();
                                if !text.is_empty() {
                                    this.answer_ask(text, cx);
                                }
                            }))
                            .child(SharedString::from("发送")),
                    );
                }
                ask_box = ask_box.child(input_row);
            }
            col = col.child(ask_box);
        }

        // 待确认时给整组一圈提示色，避免混在灰字里看不见
        if pending_here {
            return div()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(theme::with_alpha(0xf59e0b, 0.45))
                .child(col)
                .into_any_element();
        }
        col.into_any_element()
    }

    /// diff 展示块（`+` 绿 / `-` 红 / 上下文灰）。
    fn diff_box(&self, key: usize, diff: &[neverwork_core::DiffLineView]) -> impl IntoElement {
        let mut boxel = div()
            .id(SharedString::from(format!("diff-{key}")))
            .flex()
            .flex_col()
            .max_h(px(200.0))
            .overflow_scroll()
            .rounded_md()
            .bg(theme::bg())
            .p_2();

        for line in diff.iter() {
            let (fg, bg) = match line.marker.as_str() {
                "+" => (theme::green(), theme::with_alpha(0x22c55e, 0.10)),
                "-" => (theme::red(), theme::with_alpha(0xef4444, 0.10)),
                _ => (theme::dim(), theme::with_alpha(0x000000, 0.0)),
            };
            boxel = boxel.child(
                div()
                    .flex()
                    .gap_1()
                    .px_1()
                    .rounded_sm()
                    .bg(bg)
                    .text_xs()
                    .child(
                        div()
                            .w(px(10.0))
                            .flex_shrink_0()
                            .text_color(fg)
                            .child(SharedString::from(line.marker.clone())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_color(if line.marker == " " {
                                theme::muted()
                            } else {
                                fg
                            })
                            .child(SharedString::from(line.text.clone())),
                    ),
            );
        }
        boxel
    }
}

/// 动作词（中文）：统计行与审批行共用。用短词而不是叙述句，
/// 一行统计里才能塞下线程、轮次、工具数与动作分布。
fn tool_action(name: &str) -> &'static str {
    match name {
        "read_file" => "读取",
        "list_dir" => "列目录",
        "search_files" | "search_code" => "搜索",
        "write_file" => "写入",
        "apply_patch" => "打补丁",
        "multi_patch" => "批量修改",
        "handoff" => "交接",
        "web_search" => "联网搜索",
        "web_fetch" => "抓取网页",
        "run_command" => "执行命令",
        "ask_user" => "提问",
        _ => "其他",
    }
}

/// 用户发言：整块浅底 + 左侧强调条，等宽字体，不画气泡和头像。
///
/// 对齐参考稿（opencode）：用户输入是一段被引用的文本块，
/// 左右留白对齐正文，一眼能分辨「谁说的」但不抢视觉。
pub(crate) fn user_bubble(
    text: &str,
    attachments: &[neverwork_core::Attachment],
    copy: impl IntoElement,
) -> impl IntoElement {
    // 附件在正文上方：截图提问时图才是主体，文字只是补充。
    let mut body = div().flex().flex_col().gap_2();
    for a in attachments {
        if a.kind == neverwork_core::AttachmentKind::Image {
            body = body.child(
                div()
                    .w(px(140.0))
                    .rounded_md()
                    .overflow_hidden()
                    .border_1()
                    .border_color(theme::border())
                    .child(
                        img(std::path::PathBuf::from(&a.path))
                            .size(px(140.0))
                            .object_fit(gpui::ObjectFit::Cover),
                    ),
            );
        } else {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(theme::raised())
                    .child(super::files::file_badge(&a.name))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted())
                            .whitespace_nowrap()
                            .child(SharedString::from(fit_text(&a.name, 180.0, 11.0))),
                    ),
            );
        }
    }
    body = body.child(
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_color(theme::fg())
                    .line_height(px(22.0))
                    .child(SharedString::from(text.to_string())),
            )
            .child(copy),
    );
    div()
        .w_full()
        .pl_3()
        .pr_3()
        .py_2()
        .border_l_2()
        .border_color(theme::accent())
        .bg(theme::with_alpha(0xffffff, 0.035))
        .child(body)
}

/// AI 回答（单助手）：没有头像、没有气泡框，就是一段文档流 + 一行页脚。
///
/// 参考稿（opencode）里回答本身完全平铺，`▣ Build · 模型 · 厂商 · 4.2s`
/// 这条页脚承担「这是谁说的、花了多久」的信息，比头像更省地方。
/// 团队模式不要用它 —— 多角色平铺会连成一片、分不清谁说了什么，
/// 那边走 `team_bubble`。
pub(crate) fn agent_bubble(
    name: &str,
    role_id: &str,
    color_idx: usize,
    text: &str,
    code: Option<&neverwork_core::CodeBlock>,
    elapsed_ms: u64,
    // 本轮真实用量 `(输入, 输出)`；Provider 没报就是 `(0, 0)`，不显示。
    tokens: (u64, u64),
    model_label: &str,
    provider_label: &str,
    avail: f32,
    // 本地工作区根：解析 Markdown 图片的相对路径。
    root: Option<&std::path::Path>,
    // 复制按钮（元素 id + 回调）；`None` 时不渲染。
    copy: Option<(gpui::ElementId, ClickCb)>,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let mut col = div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .text_color(theme::fg());

    if text.trim().is_empty() && code.is_none() {
        col = col.child(div().text_color(theme::dim()).child("…"));
    } else {
        for b in crate::markdown::parse(text) {
            col = col.child(markdown_block(b, avail, root, cx));
        }
    }
    if let Some(c) = code {
        col = col.child(code_block(c));
    }

    col = col.child(agent_footer(
        name,
        role_id,
        color_idx,
        elapsed_ms,
        tokens,
        model_label,
        provider_label,
        copy,
    ));
    col
}

/// 回答页脚：`▣ 角色 · 模型 · 厂商 · 4.2s · ↑1.2k ↓345 [复制]`。
fn agent_footer(
    name: &str,
    role_id: &str,
    color_idx: usize,
    elapsed_ms: u64,
    tokens: (u64, u64),
    model_label: &str,
    provider_label: &str,
    copy: Option<(gpui::ElementId, ClickCb)>,
) -> impl IntoElement {
    // 多角色时带上角色名，单助手时只写模型与厂商。
    let who = if role_id == "agent" {
        String::new()
    } else if name.is_empty() {
        String::new()
    } else {
        name.to_string()
    };
    let mut parts: Vec<String> = Vec::new();
    if !who.is_empty() {
        parts.push(who);
    }
    if !model_label.is_empty() {
        parts.push(model_label.to_string());
    }
    if !provider_label.is_empty() {
        parts.push(provider_label.to_string());
    }
    if elapsed_ms > 0 {
        parts.push(format_elapsed(elapsed_ms));
    }
    // 用量记到页脚：没有这个，用户只能拍脑袋判断「这次贵不贵」，
    // token 预算超了也不知道超在哪里。
    if tokens.0 > 0 || tokens.1 > 0 {
        parts.push(format!("↑{} ↓{}", fmt_tokens(tokens.0), fmt_tokens(tokens.1)));
    }
    // 页脚文字统一走弱化色令牌：之前多角色用「白色 45% 透明」，
    // 浅色主题下白底白字几乎看不见；主题令牌两套底色都安全。
    let mut foot = div()
        .flex()
        .items_center()
        .gap_2()
        .pt_1()
        .child(
            div()
                .text_xs()
                .text_color(theme::role_color(color_idx))
                .child("▣"),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme::muted())
                .child(SharedString::from(parts.join(" · "))),
        )
        .child(div().flex_1());
    if let Some((id, cb)) = copy {
        foot = foot.child(copy_chip(id, cb));
    }
    foot
}

/// 团队模式的角色气泡：头像 + 名字行 + 气泡框 + 页脚。
///
/// 团队一轮里可能连着好几个角色的发言，纯文档流会连成一片；
/// 头像和气泡框给出「谁说完了、这句是谁的」的空间边界。
#[allow(clippy::too_many_arguments)]
pub(crate) fn team_bubble(
    name: &str,
    kind_label: &str,
    initial: String,
    color_idx: usize,
    text: &str,
    code: Option<&neverwork_core::CodeBlock>,
    elapsed_ms: u64,
    tokens: (u64, u64),
    model_label: &str,
    provider_label: &str,
    avail: f32,
    root: Option<&std::path::Path>,
    copy: Option<(gpui::ElementId, ClickCb)>,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let color = theme::role_color(color_idx);
    let initial = if initial.trim().is_empty() {
        name.chars().next().map(|c| c.to_string()).unwrap_or_default()
    } else {
        initial
    };

    let mut head = div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(SharedString::from(name.to_string())),
        );
    if !kind_label.trim().is_empty() {
        head = head.child(
            div()
                .px_1p5()
                .py_0p5()
                .rounded_md()
                .bg(theme::raised())
                .text_xs()
                .text_color(theme::dim())
                .child(SharedString::from(kind_label.trim().to_string())),
        );
    }

    // 气泡内宽 = 中栏可用宽度 - 头像 28 - 间距 12 - 左右内边距 32。
    // 折行预算按它算，否则文字会顶出气泡右边被裁。
    let inner_w = (avail - 72.0).max(160.0);
    let mut body = div().flex().flex_col().gap_2().text_color(theme::fg());
    if text.trim().is_empty() && code.is_none() {
        body = body.child(div().text_color(theme::dim()).child("…"));
    } else {
        for b in crate::markdown::parse(text) {
            body = body.child(markdown_block(b, inner_w, root, cx));
        }
    }
    if let Some(c) = code {
        body = body.child(code_block(c));
    }

    let foot = agent_footer(
        name,
        "",
        color_idx,
        elapsed_ms,
        tokens,
        model_label,
        provider_label,
        copy,
    );

    div()
        .w_full()
        .flex()
        .flex_row()
        .items_start()
        .gap_3()
        .child(
            div()
                .flex_shrink_0()
                .size(px(28.0))
                .rounded_md()
                .bg(color)
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::bg())
                .child(SharedString::from(initial)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .flex_1()
                .min_w(px(0.0))
                .child(head)
                .child(
                    div()
                        .rounded_lg()
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::panel())
                        .px_4()
                        .py_3()
                        .child(body),
                )
                .child(foot),
        )
}

/// 毫秒 → `272ms` / `4.2s` / `1m 03s`。
pub(crate) fn format_elapsed(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let total = ms / 1000;
        format!("{}m {:02}s", total / 60, total % 60)
    }
}

/// token 数 → `340` / `1.2k` / `1.3M`。
///
/// 页脚宽度寸土寸金，精确到个位没人看；量级到了就缩写。
fn fmt_tokens(n: u64) -> String {
    if n < 1000 {
        format!("{n}")
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

/// 渲染一个 Markdown 块。
///
/// `pub(crate)`：技能详情抽屉与 Markdown 预览复用同一套渲染器。
/// `root`：本地工作区根，用于解析图片的相对路径（可为 `None`）。
/// `cx`：图片点击要开放大查看器，截图/生成的图在这里接回调。
pub(crate) fn markdown_block(
    block: crate::markdown::Block,
    avail: f32,
    root: Option<&std::path::Path>,
    cx: &mut Context<AppState>,
) -> gpui::AnyElement {
    use crate::markdown::Block;
    match block {
        Block::Heading(level, text) => {
            // 标题按等级递减字号，并配一条细分隔线，让长回答有清晰层次
            let size = match level {
                1 => 18.0,
                2 => 16.0,
                _ => 14.5,
            };
            div()
                .flex()
                .flex_col()
                .gap_1()
                .pt_1()
                .child(
                    div()
                        .text_size(px(size))
                        .font_weight(FontWeight::BOLD)
                        // 参考稿里小标题是琥珀色，比正文更亮一档
                        .text_color(theme::yellow())
                        .child(inline_row(&text, avail, size)),
                )
                .when(level <= 2, |d| d.child(div().h(px(1.0)).bg(theme::border())))
                .into_any_element()
        }
        Block::Paragraph(text) => div()
            .text_color(theme::fg())
            .line_height(px(22.0))
            .child(inline_row(&text, avail, BODY_PX))
            .into_any_element(),
        Block::Code { lang, lines } => code_fence_block(&lang, &lines).into_any_element(),
        Block::Bullet(items) => {
            let mut col = div().flex().flex_col().gap_1();
            // 标记占掉的宽度要从折行预算里扣掉，否则每行多算一截、行尾被裁。
            let inner = (avail - marker_w("•")).max(120.0);
            for item in items {
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .items_start()
                        .child(div().text_color(theme::dim()).child("•"))
                        .child(div().flex_1().line_height(px(22.0)).child(inline_row(&item, inner, BODY_PX))),
                );
            }
            col.into_any_element()
        }
        Block::Numbered(start, items) => {
            let mut col = div().flex().flex_col().gap_1();
            for (i, item) in items.into_iter().enumerate() {
                // 序号宽度随位数变化（`9.` 比 `10.` 窄），按实际标签算。
                let label = format!("{}.", start + i as u32);
                let inner = (avail - marker_w(&label)).max(120.0);
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .items_start()
                        .child(
                            div()
                                .text_color(theme::dim())
                                .child(SharedString::from(label)),
                        )
                        .child(div().flex_1().line_height(px(22.0)).child(inline_row(&item, inner, BODY_PX))),
                );
            }
            col.into_any_element()
        }
        Block::Quote(items) => {
            let mut col = div().flex().flex_col().gap_1();
            // 左侧 pl_3（12）+ 竖线（2）会吃掉内容宽度
            let inner = (avail - 14.0).max(120.0);
            for line in items {
                col = col.child(div().line_height(px(22.0)).child(inline_row(&line, inner, BODY_PX)));
            }
            div()
                .pl_3()
                .border_l_2()
                .border_color(theme::accent_line())
                .text_color(theme::muted())
                .child(col)
                .into_any_element()
        }
        Block::Rule => div().h(px(1.0)).bg(theme::border()).into_any_element(),
        Block::Table { headers, aligns, rows } => {
            table_block(&headers, &aligns, &rows, avail).into_any_element()
        }
        Block::Image { alt, src } => image_block(&alt, &src, avail, root, cx),
    }
}

/// Markdown 图片：本地文件 / data URI 直接渲染，点击进放大查看器；
/// 网络图片与找不到的路径给一行占位（不报错、不吞掉后面的内容）。
fn image_block(
    alt: &str,
    src: &str,
    avail: f32,
    root: Option<&std::path::Path>,
    cx: &mut Context<AppState>,
) -> gpui::AnyElement {
    let max_w = avail.min(560.0);
    let caption = alt.trim();
    match crate::artifacts::resolve_image(src, root) {
        Some(path) => {
            // 元素 id 只用内容哈希：data URI 可能有几十 KB，不能拼进 id
            let id = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                src.hash(&mut h);
                format!("chat-image-{:016x}", h.finish())
            };
            let click_path = path.clone();
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .id(SharedString::from(id))
                        .rounded_md()
                        .border_1()
                        .border_color(theme::border())
                        .overflow_hidden()
                        .max_w(px(max_w))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                            this.open_image_view(click_path.clone(), cx);
                        }))
                        .child(
                            img(path)
                                .max_w_full()
                                .max_h(px(400.0))
                                .object_fit(gpui::ObjectFit::Contain),
                        ),
                )
                .when(!caption.is_empty(), |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .whitespace_nowrap()
                            .child(SharedString::from(fit_text(caption, max_w, 11.0))),
                    )
                })
                .into_any_element()
        }
        None => div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(theme::border())
            .bg(theme::surface())
            .text_xs()
            .text_color(theme::dim())
            .child("▧")
            .child(SharedString::from(fit_text(
                if caption.is_empty() { src } else { caption },
                (max_w - 40.0).max(80.0),
                11.0,
            )))
            .into_any_element(),
    }
}

/// Markdown 表格：表头底色 + 行列分隔线 + 隔行斑马纹。
///
/// 列宽按「该列最长内容」估算，总宽超出可用宽度时等比压缩（每列保底
/// 88px）；单元格内文字复用行内样式与折行，长内容在格内换行。
fn table_block(
    headers: &[String],
    aligns: &[crate::markdown::Align],
    rows: &[Vec<String>],
    avail: f32,
) -> gpui::AnyElement {
    use crate::markdown::Align;
    use crate::views::text_fit::char_width_mono;

    let cols = headers
        .len()
        .max(rows.iter().map(|r| r.len()).max().unwrap_or(0))
        .max(1);
    // 单元格左右内边距 px_2 = 8*2，与下面的 px_2() 保持一致
    const PAD_X: f32 = 16.0;
    // 保底宽要能放下「Python」这类 6 个 ASCII 或 3 个汉字，
    // 否则压缩后短列会被拆词（实测 Rust→Rus/t、Python→Pytho/n）。
    const MIN_COL: f32 = 88.0;

    let text_at = |list: &[String], c: usize| -> String { list.get(c).cloned().unwrap_or_default() };

    // 自然宽度：内容估算 + 内边距；估算只用可见字符宽度，加粗影响忽略
    let mut natural = vec![0.0f32; cols];
    for (c, w) in natural.iter_mut().enumerate() {
        let mut max_w = 0.0f32;
        let mut measure = |text: &str| {
            let t: f32 = text.chars().map(|ch| char_width_mono(ch, BODY_PX)).sum();
            max_w = max_w.max(t);
        };
        measure(&text_at(headers, c));
        for r in rows {
            measure(&text_at(r, c));
        }
        // 估算值加 12% 余量：等宽字体实际 advance 略高于 0.6em 估算，
        // 压到刚好等于内容宽时会出现莫名其妙的拆行
        *w = (max_w * 1.12 + PAD_X).max(MIN_COL);
    }
    let natural_total: f32 = natural.iter().sum();
    let budget = avail.max(240.0);
    let widths: Vec<f32> = if natural_total > budget {
        // 等比压缩；压到保底宽仍放不下时接受横向溢出（外层已 overflow_hidden）
        let scale = budget / natural_total;
        natural.iter().map(|w| (w * scale).max(MIN_COL)).collect()
    } else {
        natural
    };
    let total_w: f32 = widths.iter().sum();

    let cell = |text: &str, w: f32, c: usize| {
        let inner = (w - PAD_X).max(24.0);
        div()
            .w(px(w))
            .px_2()
            .py_1()
            .when(c > 0, |d| d.border_l_1().border_color(theme::border()))
            .child(inline_row_layout(
                text,
                inner,
                BODY_PX,
                24.0,
                aligns.get(c).copied().unwrap_or(Align::Left),
            ))
    };

    let mut head = div()
        .flex()
        .flex_row()
        .bg(theme::raised())
        .border_b_1()
        .border_color(theme::border())
        .font_weight(FontWeight::BOLD);
    for c in 0..cols {
        let w = widths[c];
        head = head.child(cell(&text_at(headers, c), w, c));
    }

    let mut body = div().flex().flex_col();
    for (ri, r) in rows.iter().enumerate() {
        let mut row = div().flex().flex_row();
        if ri % 2 == 1 {
            row = row.bg(theme::surface());
        }
        if ri + 1 < rows.len() {
            row = row.border_b_1().border_color(theme::border());
        }
        for c in 0..cols {
            let w = widths[c];
            row = row.child(cell(&text_at(r, c), w, c));
        }
        body = body.child(row);
    }

    div()
        .flex()
        .flex_col()
        .w_full()
        .max_w(px(total_w))
        .rounded_md()
        .border_1()
        .border_color(theme::border())
        .overflow_hidden()
        .child(head)
        .child(body)
        .into_any_element()
}

/// 行内样式（`**粗体**` / `` `代码` ``）渲染。
///
/// 折行自己做（`markdown::wrap_spans`）而不是交给 flex `flex_wrap`：
/// 气泡宽度按内容撑开，flex 拿不到确定宽度，长片段会顶出气泡边界；
/// 即使给上限，flex 也只在片段之间断行，长段落会挤成「首行只剩一个词」。
fn inline_row(text: &str, avail: f32, font_px: f32) -> gpui::AnyElement {
    inline_row_layout(text, avail, font_px, 200.0, crate::markdown::Align::Left)
}

/// 同 [`inline_row`]，额外控制最小折行宽度与横向对齐。
/// 表格单元格用：窄列不能被 `max(200)` 的保底宽度顶出格子。
fn inline_row_layout(
    text: &str,
    avail: f32,
    font_px: f32,
    min_avail: f32,
    align: crate::markdown::Align,
) -> gpui::AnyElement {
    use crate::markdown::InlineKind;
    // 折行宽度由调用方按中栏真实可用宽度传入。之前写死 `CHAT_MAX_W - 8`：
    // 右栏收起时比可用宽度窄一截（每行右侧留一大块空白），展开时又比可用
    // 宽度宽，文字被二次折行后裁掉右边。
    let avail = avail.max(min_avail);
    let spans = crate::markdown::inline_spans(text);
    // 字号取调用方的实际渲染字号：估窄了行会超宽被裁，估宽了又会提前折行。
    let lines = crate::markdown::wrap_spans(&spans, avail, font_px);

    let mut col = div()
        .flex()
        .flex_col()
        .gap_0p5()
        // 必须给确定宽度：只给 `max_w` 时父容器的宽度传到这一层仍然是
        // auto，taffy 会把它归约成最小内容宽（≈1 个字），正文因此竖排
        // 成一列并与后面的元素重叠。`w_full` 把父宽坐实，`max_w` 只做上限。
        .w_full()
        .max_w(px(avail));
    // 单元格文本对齐：行已按 avail 预折好，改列对齐即可
    col = match align {
        crate::markdown::Align::Left => col,
        crate::markdown::Align::Center => col.items_center(),
        crate::markdown::Align::Right => col.items_end(),
    };
    for line in lines {
        // 行内不换行、不压缩：本行已经按 `avail` 预折好，若再让 flex 去
        // 压缩每个 span，各 span 会各自内部折行，行高与列宽全乱、互相重叠。
        let mut row = div()
            .flex()
            .flex_row()
            .items_start()
            .max_w(px(avail))
            .min_w(px(0.0));
        for span in line {
            let el = match span.kind {
                InlineKind::Plain => div().child(SharedString::from(span.text)),
                InlineKind::Bold => div()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme::fg())
                    .child(SharedString::from(span.text)),
                InlineKind::Code => div()
                    .text_color(theme::green())
                    .child(SharedString::from(span.text)),
                InlineKind::Italic => div()
                    .italic()
                    .text_color(theme::muted())
                    .child(SharedString::from(span.text)),
                // 链接：accent 色可点击，走系统浏览器打开。
                // `on_click` 只挂在 Stateful 元素上，所以内层 div 先给一个
                // 按 url+文字哈希的稳定 id（重复链接共享 id 也无副作用）；
                // 外层包一层普通 div，让 match 各分支类型一致。
                InlineKind::Link => {
                    let url = span.url.clone();
                    let id = {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        span.text.hash(&mut h);
                        url.hash(&mut h);
                        SharedString::from(format!("md-link-{:x}", h.finish()))
                    };
                    div().child(
                        div()
                            .id(id)
                            .text_color(theme::accent())
                            .cursor_pointer()
                            .when_some(url, |d, u| {
                                d.on_click(
                                    move |_: &gpui::ClickEvent, _: &mut gpui::Window, cx: &mut gpui::App| {
                                        cx.open_url(&u);
                                    },
                                )
                            })
                            .child(SharedString::from(span.text)),
                    )
                }
            };
            // 不参与收缩：折行已在上面按可用宽度算好，
            // 这里再让 span 被压窄只会把它自己的文字挤成竖排。
            row = row.child(el.flex_shrink_0().whitespace_nowrap());
        }
        col = col.child(row);
    }
    col.into_any_element()
}

/// 代码围栏：带语言标签的顶栏 + 语法高亮内容。
fn code_fence_block(lang: &str, lines: &[String]) -> impl IntoElement {
    let lang = crate::highlight::lang_from_tag(lang);
    let mut body = div().flex().flex_col();
    for line in lines {
        // 空行也要占位，否则代码块的空行会被压掉
        if line.trim().is_empty() {
            body = body.child(div().h(px(18.0)));
            continue;
        }
        let mut row = div().flex().flex_row().flex_wrap();
        for span in crate::highlight::highlight_line(line, lang) {
            row = row.child(
                div()
                    .text_color(crate::highlight::color_for(span.tok))
                    .child(SharedString::from(span.text)),
            );
        }
        body = body.child(row);
    }
    div()
        .rounded_md()
        .border_1()
        .border_color(theme::border())
        .overflow_hidden()
        .bg(theme::bg())
        .child(
            div()
                .px_3()
                .py_2()
                .text_xs()
                .font_family(theme::mono_family())
                .child(body),
        )
}

impl AppState {
    /// 当前会话的厂商 / 模型展示名（头部 chip 与回答页脚共用）。
    pub(crate) fn model_labels(&self) -> (String, String) {
        let (provider, model) = self.session_model_labels(self.active_session);
        (model, provider)
    }
}

pub(crate) fn code_block(c: &neverwork_core::CodeBlock) -> impl IntoElement {
    div()
        .mt_2()
        .rounded_lg()
        .border_1()
        .border_color(theme::border())
        .overflow_hidden()
        .bg(theme::bg())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .bg(theme::raised())
                .border_b_1()
                .border_color(theme::border())
                .child(div().text_xs().text_color(theme::orange()).child("⬢"))
                .child(div().text_xs().text_color(theme::muted()).child(SharedString::from(c.filename.clone())))
                .child(div().flex_1())
                .child(div().text_xs().text_color(theme::green()).child(SharedString::from(format!("+{}", c.added))))
                .child(div().text_xs().text_color(theme::red()).child(SharedString::from(format!("−{}", c.removed)))),
        )
        .child(div().px_3().py_2().text_xs().text_color(theme::fg()).child(SharedString::from(c.code.clone())))
}

/// 角色交接卡片：默认一行「需求验证 → 接口成员 · 需要修改 3 处」，
/// 点击展开完整交接载荷。居中显示在两条角色气泡之间。
pub(crate) fn handoff_card(
    card_id: usize,
    from: &str,
    to: &str,
    action: &str,
    detail: &str,
    expanded: bool,
    on_click: ClickCb,
) -> impl IntoElement {
    let expandable = !detail.trim().is_empty();
    let mut head = div()
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .text_color(theme::muted())
        .child(div().text_color(theme::accent()).child("⇄"))
        .child(SharedString::from(from.to_string()))
        .child(div().text_color(theme::dim()).child("→"))
        .child(
            div()
                .text_color(theme::fg())
                .child(SharedString::from(to.to_string())),
        )
        .child(div().text_color(theme::dim()).child("·"))
        .child(SharedString::from(action.to_string()));
    if expandable {
        head = head.child(div().flex_1()).child(
            div()
                .text_color(theme::accent())
                .child(if expanded { "▾ 收起" } else { "▸ 详情" }),
        );
    }
    let mut card = div()
        .id(SharedString::from(format!("handoff-card-{card_id}")))
        .flex()
        .flex_col()
        .gap_2()
        .px_3()
        .py_2()
        .rounded_lg()
        .bg(theme::panel())
        .border_1()
        .border_color(theme::border())
        .child(head)
        // 带 cursor_pointer 的元素必须有 on_click，否则是死点击
        .when(expandable, |d| {
            d.cursor_pointer()
                .on_click(move |ev, w, cx| on_click(ev, w, cx))
        });
    if expandable && expanded {
        let mut body = div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_2()
            .border_t_1()
            .border_color(theme::border())
            .text_xs()
            .text_color(theme::muted());
        for line in detail.trim().lines() {
            body = body.child(if line.trim().is_empty() {
                div().h(px(6.0))
            } else {
                div().child(SharedString::from(line.to_string()))
            });
        }
        card = card.child(body);
    }
    div().flex().justify_center().child(card)
}

pub(crate) fn tool_result(summary: &str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(theme::green_soft())
        .border_1()
        .border_color(theme::green_line())
        .text_xs()
        .text_color(theme::muted())
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(15.0))
                .rounded_sm()
                .bg(theme::green_soft())
                .text_color(theme::green())
                .child("✓"),
        )
        .child(SharedString::from(summary.to_string()))
}

pub(crate) fn parallel_row(items: &[neverwork_core::ParallelItem]) -> impl IntoElement {
    let mut row = div().flex().items_center().justify_center().gap_2();
    for it in items {
        let status = if it.done {
            div().text_color(theme::green()).child("✓")
        } else {
            div().text_color(theme::accent()).child("…")
        };
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .rounded_md()
                .bg(theme::panel())
                .border_1()
                .border_color(theme::border())
                .text_xs()
                .text_color(theme::muted())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(15.0))
                        .rounded_sm()
                        .bg(theme::role_color(it.color_index))
                        .text_color(theme::fg())
                        .child(SharedString::from(it.initial.clone())),
                )
                .child(SharedString::from(format!("{} {}", it.role_name, it.task)))
                .child(status),
        );
    }
    div().flex().justify_center().child(row)
}



pub(crate) fn role_dot(name: &str, idx: usize) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(div().size(px(6.0)).rounded_md().bg(theme::role_color(idx)))
        .child(div().text_xs().text_color(theme::muted()).child(SharedString::from(name.to_string())))
}

/// 顶栏角色圆点行（来自会话所属团队）。
pub(crate) fn role_dots_row(team: Option<&neverwork_core::Team>) -> impl IntoElement {
    let mut row = div().flex().items_center().gap_2();
    if let Some(team) = team {
        for role in &team.roles {
            row = row.child(role_dot(&role.name, role.color_index));
        }
    }
    row
}

/// `pending_ask` 卡片归属哪一组工具调用：[start, end) 且同一会话。
///
/// 索引未知（`usize::MAX`，工具消息还没落进消息表）时只有最后一组能画，
/// 否则历史里每组工具后面都会重复一张同样的卡片。
pub(crate) fn ask_card_in_group(msg_idx: usize, start: usize, end: usize, last_group: bool) -> bool {
    if msg_idx == usize::MAX {
        last_group
    } else {
        msg_idx >= start && msg_idx < end
    }
}

#[cfg(test)]
mod tests {
    use super::ask_card_in_group;

    #[test]
    fn ask_card_renders_only_in_its_tool_group() {
        // 消息落在第一组：只有第一组画卡片
        assert!(ask_card_in_group(3, 2, 7, false));
        assert!(!ask_card_in_group(3, 7, 12, true));
        // 消息落在第二组
        assert!(!ask_card_in_group(9, 2, 7, false));
        assert!(ask_card_in_group(9, 7, 12, true));
        // 组边界：end 是开区间
        assert!(ask_card_in_group(2, 2, 7, false));
        assert!(!ask_card_in_group(7, 2, 7, false));
        // 索引未知：只放在最后一组，只有一张
        assert!(!ask_card_in_group(usize::MAX, 2, 7, false));
        assert!(ask_card_in_group(usize::MAX, 7, 12, true));
    }
}

