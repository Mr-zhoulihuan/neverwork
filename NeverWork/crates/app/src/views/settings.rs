//! 设置页：常规 / 模型与厂商 / 技能 / 同步与升级 / 关于。
//!
//! 所有开关都是即时生效并立刻落盘的（没有"应用"按钮），
//! 因为设置项少、每次改动代价极低，这样用户不会忘记保存。

use gpui::prelude::*;
use gpui::{div, px, Context, FocusHandle, FontWeight, SharedString};

use neverwork_core::ThemeMode;
use neverwork_ui::theme;

use crate::state::{AppState, FormField, SettingsTab};
use crate::views::text_fit::fit_text;

/// 设置页左侧导航宽度。
const NAV_W: f32 = 150.0;

impl AppState {
    pub(crate) fn settings_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div()
            .flex()
            .flex_row()
            .flex_1()
            .h_full()
            .min_w(px(0.0))
            .bg(theme::surface());

        row = row.child(self.settings_nav(cx));

        let mut body = div()
            .id("settings-body")
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .px_6()
            .py_5()
            .overflow_y_scroll();

        // 顶部标题 + 操作反馈
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::fg())
                        .child(SharedString::from(self.settings_tab.label())),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::dim())
                        .child(SharedString::from(tab_hint(self.settings_tab))),
                ),
        );

        if let Some(msg) = &self.action_msg {
            body = body.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(theme::green_soft())
                    .border_1()
                    .border_color(theme::green_line())
                    .text_xs()
                    .text_color(theme::green())
                    .child(SharedString::from(msg.clone())),
            );
        }

        body = body.child(match self.settings_tab {
            SettingsTab::General => self.settings_general(cx).into_any_element(),
            SettingsTab::Models => self.settings_models(cx).into_any_element(),
            SettingsTab::Skills => self.settings_installed_skills(cx).into_any_element(),
            SettingsTab::Sync => self.settings_sync(cx).into_any_element(),
            SettingsTab::About => self.settings_about().into_any_element(),
        });

        row.child(body)
    }

    fn settings_nav(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut nav = div()
            .flex()
            .flex_col()
            .gap_1()
            .w(px(NAV_W))
            .flex_shrink_0()
            .h_full()
            .px_2()
            .py_3()
            .bg(theme::panel())
            .border_r_1()
            .border_color(theme::border());

        nav = nav.child(
            div()
                .id("settings-back")
                .flex()
                .items_center()
                .gap_2()
                .h(px(30.0))
                .px_2()
                .mb_2()
                .rounded_md()
                .text_xs()
                .text_color(theme::muted())
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()))
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                    this.show_chat(cx);
                }))
                .child("← 返回对话"),
        );

        for tab in SettingsTab::all() {
            let on = self.settings_tab == tab;
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("stab-{}", tab.label())))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(32.0))
                    .px_2()
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .when(on, |d| d.bg(theme::accent_soft()).text_color(theme::accent()))
                    .when(!on, |d| {
                        d.text_color(theme::muted()).hover(|d| d.bg(theme::raised()))
                    })
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                        this.set_settings_tab(tab, cx);
                    }))
                    .child(div().text_xs().child(SharedString::from(tab.glyph())))
                    .child(SharedString::from(tab.label())),
            );
        }

        nav = nav.child(div().flex_1()).child(
            div()
                .px_2()
                .text_xs()
                .text_color(theme::dim())
                .child(SharedString::from(format!("v{}", neverwork_core::APP_VERSION))),
        );

        nav
    }

    // ==================== 常规 ====================

    fn settings_general(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().gap_4().min_w(px(0.0));

        // ---- 外观 ----
        col = col.child(section_card(
            "外观",
            "更改后立即生效，无需重启。",
            {
                let mut g = div().flex().flex_col().gap_3();

                // 主题：每套是完整色板，色卡用它的背景 + 强调色预览。
                g = g.child(setting_row(
                    "主题",
                    "9 套完整配色；跟随系统读系统深浅",
                    div()
                        .flex()
                        .gap_1()
                        .flex_wrap()
                        .children(ThemeMode::all().iter().map(|m| {
                            let mode = *m;
                            let on = self.settings.theme == mode;
                            let (pbg, _pfg, pacc) = theme::palette_preview(mode);
                            div()
                                .id(SharedString::from(format!("theme-{}", mode.index())))
                                .flex()
                                .items_center()
                                .gap_1()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .text_xs()
                                .cursor_pointer()
                                .when(on, |d| {
                                    d.bg(theme::accent_soft())
                                        .text_color(theme::accent())
                                        .border_1()
                                        .border_color(theme::accent_line())
                                })
                                .when(!on, |d| {
                                    d.bg(theme::raised())
                                        .text_color(theme::muted())
                                        .hover(|d| d.bg(theme::border()))
                                })
                                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                                    this.set_theme(mode, cx);
                                }))
                                .child(
                                    div()
                                        .size(px(12.0))
                                        .rounded_sm()
                                        .bg(pbg)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(div().size(px(5.0)).rounded_full().bg(pacc)),
                                )
                                .child(SharedString::from(mode.label()))
                        })),
                ));

                g = g.child(setting_row(
                    "界面缩放",
                    format!("当前 {}%", self.settings.ui_scale as i32),
                    stepper(
                        "ui-scale",
                        cx,
                        self.settings.ui_scale as i32,
                        80,
                        160,
                        10,
                        "%",
                        |this, v, cx| this.update_settings(|s| s.ui_scale = v as f32, cx),
                    ),
                ));

                g
            },
        ));

        // ---- 编辑器 ----
        col = col.child(section_card(
            "编辑器",
            "作用于文件编辑器",
            {
                let mut g = div().flex().flex_col().gap_3();

                g = g.child(setting_row(
                    "字号",
                    format!("当前 {} px", self.editor_font_size() as i32),
                    stepper(
                        "font-size",
                        cx,
                        self.editor_font_size() as i32,
                        9,
                        24,
                        1,
                        "px",
                        |this, v, cx| this.update_settings(|s| s.editor_font_size = v as f32, cx),
                    ),
                ));

                g = g.child(setting_row(
                    "缩进宽度",
                    "Tab 与自动缩进的空格数",
                    stepper(
                        "tab-width",
                        cx,
                        self.settings.tab_width as i32,
                        1,
                        8,
                        1,
                        "空格",
                        |this, v, cx| this.update_settings(|s| s.tab_width = v as u8, cx),
                    ),
                ));

                g = g.child(toggle_row(
                    "显示行号",
                    "关闭后隐藏左侧行号栏",
                    self.settings.show_line_numbers,
                    cx,
                    |this, cx| {
                        let v = this.settings.show_line_numbers;
                        this.update_settings(|s| s.show_line_numbers = !v, cx);
                    },
                ));

                g = g.child(toggle_row(
                    "自动保存",
                    "停止输入 800ms 后自动写盘",
                    self.settings.auto_save,
                    cx,
                    |this, cx| {
                        let v = this.settings.auto_save;
                        this.update_settings(|s| s.auto_save = !v, cx);
                    },
                ));

                g = g.child(toggle_row(
                    "国内镜像",
                    "MCP 安装依赖走 npmmirror / 清华 PyPI，已设的环境变量不覆盖",
                    self.settings.mcp_mirror,
                    cx,
                    |this, cx| this.toggle_mcp_mirror(cx),
                ));

                g
            },
        ));

        // ---- 数据 ----
        col = col.child(section_card(
            "数据",
            "保存在本机用户目录，不上传",
            {
                let dir = neverwork_core::config::config_dir();
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(setting_row(
                        "配置目录",
                        dir.display().to_string(),
                        btn("reveal-config", "打开目录", cx, |this, cx| {
                            #[cfg(windows)]
                            {
                                let _ = std::process::Command::new("explorer")
                                    .arg(neverwork_core::config::config_dir())
                                    .spawn();
                            }
                            this.action_msg = Some("已打开配置目录".into());
                            cx.notify();
                        }),
                    ))
                    .child(setting_row(
                        "恢复默认设置",
                        "不删除团队、技能与 API Key",
                        btn_danger("reset-settings", "恢复默认", cx, |this, cx| {
                            let providers = this.settings.providers.clone();
                            let skills = this.settings.skills.clone();
                            let active = this.settings.active_provider_id.clone();
                            this.settings = neverwork_core::Settings::default();
                            this.settings.providers = providers;
                            this.settings.skills = skills;
                            this.settings.active_provider_id = active;
                            neverwork_ui::theme::set_theme_mode(this.settings.theme);
                            this.update_settings(|_| {}, cx);
                            this.action_msg = Some("已恢复默认偏好".into());
                            cx.notify();
                        }),
                    ))
            },
        ));

        col
    }

    // ==================== 已安装技能 ====================

    fn settings_installed_skills(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let installed: Vec<&neverwork_core::Skill> =
            self.settings.skills.iter().filter(|s| s.installed).collect();

        let mut col = div().flex().flex_col().gap_3().min_w(px(0.0));

        col = col.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded_md()
                .bg(theme::accent_soft())
                .text_xs()
                .text_color(theme::muted())
                .child(SharedString::from(format!(
                    "已安装 {} 个技能",
                    installed.len()
                )))
                .child(div().flex_1())
                .child(btn("open-market", "去技能市场 →", cx, |this, cx| {
                    this.open_skills(cx);
                })),
        );

        if installed.is_empty() {
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .py_8()
                    .text_sm()
                    .text_color(theme::dim())
                    .child("还没有安装任何技能")
                    .child(
                        div()
                            .text_xs()
                            .child("技能可以给 Agent 补上具体领域的做法，安装后即可在对话里引用"),
                    ),
            );
            return col;
        }

        for s in installed {
            let id = s.id.clone();
            let hue = s.category.hue();
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.0))
                            .flex_shrink_0()
                            .rounded_md()
                            .bg(theme::with_alpha(hue, 0.18))
                            .text_xs()
                            .text_color(gpui::rgb(hue))
                            .child(SharedString::from(s.glyph.clone())),
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
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(theme::fg())
                                            .child(SharedString::from(s.name.clone())),
                                    )
                                    .child(tag(s.category.label(), theme::dim())),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::dim())
                                    .child(SharedString::from(fit_text(&s.id, 340.0, 12.0))),
                            ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("uninstall-{id}")))
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .text_xs()
                            .text_color(theme::red())
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::raised()))
                            .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                                this.uninstall_skill(&id, cx);
                            }))
                            .child("卸载"),
                    ),
            );
        }

        col
    }

    // ==================== 同步与升级 ====================

    fn settings_sync(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().gap_4().min_w(px(0.0));

        // ---- 版本 ----
        col = col.child(section_card(
            "版本升级",
            "对比远端仓库的新提交",
            {
                let mut g = div().flex().flex_col().gap_3();
                g = g.child(setting_row(
                    "当前版本",
                    format!(
                        "v{} · 上次检查 {}",
                        neverwork_core::APP_VERSION,
                        fmt_time(self.settings.last_update_check)
                    ),
                    div()
                        .flex()
                        .gap_2()
                        .child(btn("check-update", "检查更新", cx, |this, cx| {
                            this.check_update(cx);
                        }))
                        .child(btn("pull-update", "拉取更新", cx, |this, cx| {
                            this.pull_update(cx);
                        })),
                ));
                if let Some(st) = &self.update_status {
                    g = g.child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(theme::raised())
                            .text_xs()
                            .text_color(theme::muted())
                            .child(SharedString::from(st.clone())),
                    );
                }
                g
            },
        ));

        // ---- 同步 ----
        col = col.child(section_card(
            "配置同步",
            "导出成 JSON，可在另一台机器导入恢复",
            {
                let mut g = div().flex().flex_col().gap_3();

                g = g.child(toggle_row(
                    "自动备份",
                    "改设置后自动留一份快照",
                    self.settings.sync_enabled,
                    cx,
                    |this, cx| this.toggle_sync(cx),
                ));

                g = g.child(toggle_row(
                    "导出包含 API Key",
                    "默认关闭：导出的文件不含密钥，换机器后需重新填写。仅在需要完整迁移时打开。",
                    self.export_with_secrets,
                    cx,
                    |this, cx| {
                        let v = this.export_with_secrets;
                        this.export_with_secrets = !v;
                        this.action_msg = Some(if !v {
                            "导出将包含明文 API Key，请勿把文件发给他人".into()
                        } else {
                            "导出将脱敏：文件里不含 API Key".into()
                        });
                        cx.notify();
                    },
                ));

                g = g.child(setting_row(
                    "立即导出",
                    format!(
                        "{}　上次同步 {}",
                        if self.export_with_secrets {
                            "将包含明文密钥（谨慎分享）"
                        } else {
                            "已脱敏：不含 API Key"
                        },
                        fmt_time(self.settings.last_sync_at)
                    ),
                    div()
                        .flex()
                        .gap_2()
                        .child(btn("sync-now", "导出快照", cx, |this, cx| {
                            this.sync_now(cx);
                        }))
                        .child(btn("reveal-sync", "打开目录", cx, |this, cx| {
                            this.reveal_sync_dir(cx);
                        })),
                ));

                // 导入
                let editing = self.active_field == Some(FormField::ImportPath);
                let path_display = if editing {
                    self.field_buf.text().to_string()
                } else {
                    self.import_path.clone()
                };
                g = g.child(setting_row(
                    "从文件导入",
                    "导入会覆盖当前团队与设置",
                    div()
                        .flex()
                        .gap_2()
                        .child(input_box(
                            SharedString::from("import-path"),
                            path_display,
                            "C:\\path\\to\\neverwork-2026….json",
                            editing,
                            self.editor_focus.clone(),
                            self.field_area_bounds.clone(),
                            cx,
                            FormField::ImportPath,
                        ))
                        .child(btn("do-import", "导入", cx, |this, cx| {
                            this.import_from_path(cx);
                        })),
                ));

                // 已有快照
                let snaps = self.sync_snapshots();
                if !snaps.is_empty() {
                    let mut list = div().flex().flex_col().gap_1();
                    list = list.child(label(&format!("已有快照（{}）", snaps.len())));
                    for (name, at) in snaps.iter().take(6) {
                        let full = crate::settings_ops::sync_dir().join(name);
                        let full_s = full.display().to_string();
                        list = list.child(
                            div()
                                .id(SharedString::from(format!("snap-{name}")))
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .text_xs()
                                .text_color(theme::muted())
                                .cursor_pointer()
                                .hover(|d| d.bg(theme::raised()))
                                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                                    this.import_path = full_s.clone();
                                    this.action_msg = Some("已填入导入路径，点「导入」即可恢复".into());
                                    cx.notify();
                                }))
                                .child(div().text_color(theme::accent()).child("◆"))
                                .child(SharedString::from(fit_text(name, 240.0, 12.0)))
                                .child(div().flex_1())
                                .child(SharedString::from(fmt_time(*at))),
                        );
                    }
                    g = g.child(list);
                }

                g
            },
        ));

        col
    }

    // ==================== 关于 ====================

    fn settings_about(&self) -> impl IntoElement {
        let mut col = div().flex().flex_col().gap_4().min_w(px(0.0));

        col = col.child(section_card(
            "NeverWork",
            "纯 Rust + GPUI 自绘的 AI 原生 IDE。",
            {
                let mut g = div().flex().flex_col().gap_2();
                for (k, v) in [
                    ("版本", format!("v{}", neverwork_core::APP_VERSION)),
                    ("界面", "GPUI 0.2（Zed 内核，GPU 渲染）".to_string()),
                    ("许可", "Apache-2.0".to_string()),
                    ("仓库", "gitee.com/ZLH1995123/neverwork".to_string()),
                    (
                        "配置",
                        neverwork_core::config::config_dir().display().to_string(),
                    ),
                ] {
                    g = g.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .py_1()
                            .child(
                                div()
                                    .w(px(72.0))
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(theme::dim())
                                    .child(SharedString::from(k)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::muted())
                                    .child(SharedString::from(v)),
                            ),
                    );
                }
                g
            },
        ));

        col = col.child(section_card(
            "统计",
            "",
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(stat_line("厂商", self.settings.providers.len().to_string()))
                .child(stat_line(
                    "已填 Key",
                    self.settings.keyed_provider_count().to_string(),
                ))
                .child(stat_line(
                    "已安装技能",
                    self.settings.installed_skill_count().to_string(),
                ))
                .child(stat_line("团队", self.config.teams.len().to_string()))
                .child(stat_line("会话", self.sessions.len().to_string())),
        ));

        col
    }
}

// ==================== 通用小组件 ====================

fn tab_hint(tab: SettingsTab) -> &'static str {
    match tab {
        SettingsTab::General => "外观、编辑器与数据。所有改动即时生效并自动保存。",
        SettingsTab::Models => "点开厂商填 Key、管模型；每个会话可单独选厂商与模型",
        SettingsTab::Skills => "已安装的技能",
        SettingsTab::Sync => "导出/导入配置，检查新版本",
        SettingsTab::About => "版本与统计信息。",
    }
}

pub(crate) fn section_card(
    title: &str,
    hint: impl Into<String>,
    body: impl IntoElement,
) -> impl IntoElement {
    let hint = hint.into();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(theme::border())
        .bg(theme::panel())
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::fg())
                        .child(SharedString::from(title.to_string())),
                )
                .when(!hint.is_empty(), |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .child(SharedString::from(hint)),
                    )
                }),
        )
        .child(body)
}

/// 左标题 + 右控件的设置行。
pub(crate) fn setting_row(
    title: impl Into<String>,
    hint: impl Into<String>,
    control: impl IntoElement,
) -> impl IntoElement {
    let hint = hint.into();
    // 固定标签列宽 + 控件不收缩：这样整行宽度可预测，窄窗口下控件也不会被挤出屏幕。
    div()
        .flex()
        .items_center()
        .gap_3()
        .py_1()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .w(px(180.0))
                .flex_shrink_0()
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::fg())
                        .child(SharedString::from(title.into())),
                )
                .when(!hint.is_empty(), |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .child(SharedString::from(fit_text(&hint, 180.0, 12.0))),
                    )
                }),
        )
        .child(div().flex_shrink_0().child(control))
}

/// 开关行。
fn toggle_row(
    title: &'static str,
    hint: &'static str,
    on: bool,
    cx: &mut Context<AppState>,
    f: impl Fn(&mut AppState, &mut Context<AppState>) + 'static,
) -> impl IntoElement {
    let control = div()
        .id(SharedString::from(format!("toggle-{title}")))
        .flex()
        .items_center()
        .w(px(44.0))
        .h(px(24.0))
        .px_0p5()
        .rounded_full()
        .cursor_pointer()
        .when(on, |d| d.bg(theme::accent()).justify_end())
        .when(!on, |d| d.bg(theme::border()).justify_start())
        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| f(this, cx)))
        .child(div().size(px(18.0)).rounded_full().bg(gpui::rgb(0xffffff)));
    setting_row(title, hint, control)
}

/// 标题在上、输入框在下的表单行（厂商详情：显示名 / Key / 基址）。
pub(crate) fn form_row(
    title: &str,
    hint: &str,
    control: impl IntoElement,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(label(title))
                .when(!hint.is_empty(), |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .child(SharedString::from(hint.to_string())),
                    )
                }),
        )
        .child(control)
}

/// 数值步进器。
pub(crate) fn stepper(
    id: &str,
    cx: &mut Context<AppState>,
    value: i32,
    min: i32,
    max: i32,
    step: i32,
    suffix: &'static str,
    f: impl Fn(&mut AppState, i32, &mut Context<AppState>) + Clone + 'static,
) -> impl IntoElement {
    let id_minus = SharedString::from(format!("{id}-minus"));
    let id_plus = SharedString::from(format!("{id}-plus"));
    let f1 = f.clone();
    let f2 = f;
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .id(id_minus)
                .flex()
                .items_center()
                .justify_center()
                .size(px(24.0))
                .rounded_md()
                .bg(theme::raised())
                .text_sm()
                .text_color(if value <= min { theme::dim() } else { theme::fg() })
                .cursor_pointer()
                .hover(|d| d.bg(theme::border()))
                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                    f1(this, (value - step).max(min), cx);
                }))
                .child("−"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .min_w(px(56.0))
                .text_sm()
                .text_color(theme::fg())
                .child(SharedString::from(if suffix.is_empty() {
                    format!("{value}")
                } else {
                    format!("{value}{suffix}")
                })),
        )
        .child(
            div()
                .id(id_plus)
                .flex()
                .items_center()
                .justify_center()
                .size(px(24.0))
                .rounded_md()
                .bg(theme::raised())
                .text_sm()
                .text_color(if value >= max { theme::dim() } else { theme::fg() })
                .cursor_pointer()
                .hover(|d| d.bg(theme::border()))
                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                    f2(this, (value + step).min(max), cx);
                }))
                .child("＋"),
        )
}

pub(crate) fn btn(
    id: &'static str,
    label_text: &'static str,
    cx: &mut Context<AppState>,
    f: impl Fn(&mut AppState, &mut Context<AppState>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded_md()
        .bg(theme::accent())
        .text_xs()
        .text_color(gpui::rgb(0xffffff))
        .cursor_pointer()
        .hover(|d| d.opacity(0.88))
        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| f(this, cx)))
        .child(SharedString::from(label_text))
}

fn btn_danger(
    id: &'static str,
    label_text: &'static str,
    cx: &mut Context<AppState>,
    f: impl Fn(&mut AppState, &mut Context<AppState>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded_md()
        .bg(theme::with_alpha(0xef4444, 0.14))
        .text_xs()
        .text_color(theme::red())
        .cursor_pointer()
        .hover(|d| d.opacity(0.85))
        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| f(this, cx)))
        .child(SharedString::from(label_text))
}

pub(crate) fn label(text: &str) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(theme::dim())
        .child(SharedString::from(text.to_string()))
}

pub(crate) fn tag(text: &str, color: gpui::Rgba) -> impl IntoElement {
    div()
        .px_1p5()
        .py_0p5()
        .rounded_sm()
        .bg(theme::with_alpha(0xffffff, 0.06))
        .text_xs()
        .text_color(color)
        .child(SharedString::from(text.to_string()))
}

fn stat_line(k: &str, v: String) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .py_0p5()
        .child(div().text_xs().text_color(theme::dim()).child(SharedString::from(k.to_string())))
        .child(div().text_xs().text_color(theme::fg()).child(SharedString::from(v)))
}

/// 可点击编辑的输入框。点击后进入 `FormField` 编辑态，回车提交。
#[allow(clippy::too_many_arguments)]
pub(crate) fn input_box(
    id: SharedString,
    value: String,
    placeholder: &'static str,
    editing: bool,
    focus: FocusHandle,
    bounds_slot: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    cx: &mut Context<AppState>,
    field: FormField,
) -> impl IntoElement {
    let empty = value.is_empty();
    let display = if empty {
        placeholder.to_string()
    } else {
        value
    };
    let color = if empty { theme::dim() } else { theme::fg() };

    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .flex_1()
        .min_w(px(0.0))
        .h(px(28.0))
        .px_2()
        .rounded_md()
        .bg(theme::raised())
        .border_1()
        .border_color(if editing {
            theme::accent_line()
        } else {
            theme::border()
        })
        .text_xs()
        .cursor(gpui::CursorStyle::IBeam)
        .when(editing, |d| d.track_focus(&focus))
        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, window, cx| {
            // 已在该字段编辑态时再点不要重置缓冲（否则会把刚打的字抹回旧值）。
            if !editing {
                this.begin_field(field, cx);
            }
            window.focus(&this.editor_focus);
            cx.notify();
        }))
        .on_key_down(cx.listener(move |this, ev: &gpui::KeyDownEvent, _w, cx| {
            if this.active_field != Some(field) {
                return;
            }
            if ev.keystroke.key == "enter" || ev.keystroke.key == "escape" {
                this.commit_field(cx);
                return;
            }
            this.on_field_key(ev, cx);
        }))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(color)
                .child(SharedString::from(fit_text(&display, 300.0, 12.0))),
        )
        .when(editing, |d| {
            d.child(div().text_color(theme::accent()).child("▏")).child(
                // 记录输入框在窗口中的真实位置，「点外面自动提交」靠它判定。
                gpui::canvas(
                    |_b, _w, _cx| {},
                    move |bounds, _st, _w, _cx| {
                        bounds_slot.set(Some(bounds));
                    },
                )
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .size(px(1.0)),
            )
        })
}

/// 相对时间：刚刚 / N 分钟前 / N 小时前 / 日期。
fn fmt_time(secs: u64) -> String {
    if secs == 0 {
        return "从未".into();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let d = now.saturating_sub(secs);
    match d {
        0..=59 => "刚刚".into(),
        60..=3599 => format!("{} 分钟前", d / 60),
        3600..=86399 => format!("{} 小时前", d / 3600),
        86400..=2_592_000 => format!("{} 天前", d / 86400),
        _ => format!("{} 个月前", d / 2_592_000),
    }
}
