//! 条目详情抽屉：从右侧抽出 420px 面板，遮罩或 Esc 关闭。
//!
//! 内容全部来自条目自身（`Skill.detail` 是上游 Markdown 正文），
//! 安装 / 卸载直接复用市场那套状态机，不另开一条写入路径。

use gpui::prelude::*;
use gpui::{div, img, px, AnyElement, Context, FontWeight, SharedString};

use neverwork_ui::theme;

use crate::state::AppState;

/// 抽屉宽度（窄窗口按比例收）。
const DETAIL_W: f32 = 420.0;
/// 正文最多渲染多少字符：`SKILL.md` 可能有几十 KB，
/// 全量构树会把首帧拖慢，这里只预览开头。
const DETAIL_PREVIEW_CHARS: usize = 6000;

impl AppState {
    pub(crate) fn market_detail_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(entry) = self.market_detail.clone() else {
            // 调用方只在 `is_some()` 时渲染；空态给个零尺寸兜底。
            return div().into_any_element();
        };
        let skill = entry.skill.clone();
        let installed = self.is_skill_installed(&skill.id);
        let mcp = skill.is_mcp();
        let remote_only = entry.remote_only;
        let busy_here = self.market_installing.as_deref() == Some(skill.id.as_str());
        let pending_here = self.pending_mcp_install.as_deref() == Some(skill.id.as_str());
        let mcp_cmd = skill
            .mcp
            .as_ref()
            .map(|m| format!("{} {}", m.command, m.args.join(" ")))
            .unwrap_or_default();
        let hue = self.market_source.hue();
        let width = (self.layout_w.get() * 0.92).min(DETAIL_W).max(300.0);
        // 左右 px_5 内边距要扣掉，否则 Markdown 折行会算多一截。
        let avail = (width - 40.0).max(200.0);

        // ---- 头部：图标 + 名称 / 作者 + 关闭 ----
        let icon: AnyElement = match crate::marketplace::icon_path(&entry) {
            Some(path) => div()
                .size(px(40.0))
                .flex_shrink_0()
                .rounded_lg()
                .overflow_hidden()
                .child(img(path).size(px(40.0)).object_fit(gpui::ObjectFit::Cover))
                .into_any_element(),
            None => div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(40.0))
                .flex_shrink_0()
                .rounded_lg()
                .bg(theme::with_alpha(hue, 0.16))
                .text_sm()
                .text_color(gpui::rgb(hue))
                .child(SharedString::from(skill.glyph.clone()))
                .into_any_element(),
        };

        let mut panel = div()
            .id("market-detail")
            .flex()
            .flex_col()
            .w(px(width))
            .h_full()
            .bg(theme::panel())
            .border_l_1()
            .border_color(theme::border())
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .px_5()
                    .py_4()
                    .border_b_1()
                    .border_color(theme::border())
                    .child(icon)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .text_size(px(16.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme::fg())
                                    .child(SharedString::from(skill.name.clone())),
                            )
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_xs()
                                    .text_color(theme::dim())
                                    .child(SharedString::from(skill.author.clone())),
                            ),
                    )
                    .child(
                        div()
                            .id("detail-close")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(26.0))
                            .flex_shrink_0()
                            .rounded_md()
                            .text_sm()
                            .text_color(theme::muted())
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                                this.close_market_detail(cx);
                            }))
                            .child("✕"),
                    ),
            );

        // ---- 形态 / 来源 / 标签 ----
        // 来源从 id 前缀推断（与团队插件面板共用同一份文案）。
        let source_label = crate::marketplace::source_label(&skill.id);
        let mut badges = div()
            .flex()
            .items_center()
            .gap_1p5()
            .flex_wrap()
            .px_5()
            .pt_3()
            .child(badge(source_label, theme::accent_soft(), theme::accent()))
            .child(badge(
                if mcp { "MCP" } else { "文本技能" },
                theme::raised(),
                theme::muted(),
            ));
        if remote_only {
            badges = badges.child(badge("仅远程", theme::raised(), theme::dim()));
        }
        if skill.installs > 0 {
            badges = badges.child(badge(
                &format!("累计安装 {}", skill.installs_label()),
                theme::raised(),
                theme::dim(),
            ));
        }
        for t in entry.tags.iter().filter(|t| {
            t.as_str() != "仅远程" && t.as_str() != skill.author && t.as_str() != source_label
        }) {
            badges = badges.child(badge(t, theme::raised(), theme::muted()));
        }
        panel = panel.child(badges);

        // ---- 正文（Markdown，复用对话区的渲染器） ----
        // skills.sh 列表接口不带正文：抽屉打开的瞬间先显示「正在拉取」，
        // 正文到达后由 state 回填并重绘（`detail_loaded`）。
        let mut body = div()
            .id("detail-body")
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .min_h(px(0.0))
            .px_5()
            .py_4()
            .overflow_y_scroll()
            .text_size(px(13.0));
        let loading = self.market_detail_loading && !entry.detail_loaded;
        if loading {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(theme::muted())
                    .child("⟳ 正在拉取上游说明…"),
            );
        } else {
            if let Some(err) = self
                .market_detail_error
                .as_ref()
                .filter(|_| !entry.detail_loaded)
            {
                body = body.child(
                    div()
                        .text_sm()
                        .text_color(theme::red())
                        .child(SharedString::from(format!("说明拉取失败：{err}"))),
                );
            }
            if skill.detail.trim().is_empty() {
                body = body.child(
                    div()
                        .text_sm()
                        .text_color(theme::dim())
                        .child("上游没有提供详细说明"),
                );
            } else {
                for block in crate::markdown::parse(&preview_detail(&skill.detail)) {
                    body = body.child(crate::views::chat::markdown_block(
                        block,
                        avail,
                        self.root.as_deref(),
                        cx,
                    ));
                }
            }
        }
        if pending_here {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .bg(theme::accent_soft())
                    .border_1()
                    .border_color(theme::accent_line())
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::fg())
                            .child(SharedString::from(format!(
                                "将注册外部进程：{mcp_cmd}（工具调用逐次审批）"
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
                                    .id("detail-mcp-cancel")
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .text_xs()
                                    .text_color(theme::muted())
                                    .cursor_pointer()
                                    .hover(|d| d.bg(theme::raised()))
                                    .on_click(cx.listener(
                                        |this, _: &gpui::ClickEvent, _w, cx| {
                                            this.cancel_mcp_install(cx);
                                        },
                                    ))
                                    .child("取消"),
                            )
                            .child(
                                div()
                                    .id("detail-mcp-confirm")
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .bg(theme::accent())
                                    .text_xs()
                                    .text_color(gpui::rgb(0xffffff))
                                    .cursor_pointer()
                                    .hover(|d| d.opacity(0.88))
                                    .on_click(cx.listener(
                                        |this, _: &gpui::ClickEvent, _w, cx| {
                                            this.confirm_mcp_install(cx);
                                        },
                                    ))
                                    .child("确认安装"),
                            ),
                    ),
            );
        }
        if mcp && skill.mcp.is_none() && !remote_only {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("安装时会从上游详情页抓取启动命令"),
            );
        }
        panel = panel.child(body);

        // ---- 底栏：绑定 / 安装 / 卸载 + 上游链接 ----
        let id = skill.id.clone();
        let entry_for_btn = entry.clone();
        let mut install_btn = div()
            .id("detail-install")
            .flex()
            .items_center()
            .justify_center()
            .min_w(px(96.0))
            .px_3()
            .py_1p5()
            .rounded_md()
            .text_xs();
        // 从团队角色面板进来时，主按钮是「绑定到角色」（绑定会先确保安装，
        // MCP 未装则走确认流程，确认后自动绑），不再给卸载入口。
        let bound_role = self.market_detail_role.and_then(|(ti, ri)| {
            let team = self.config.teams.get(ti)?;
            let role = team.roles.get(ri)?;
            let bound = role.skills.iter().any(|x| x == &skill.id);
            Some((ti, ri, role.name.clone(), bound))
        });
        if let Some((team_idx, role_idx, role_name, bound)) = bound_role {
            install_btn = if bound {
                install_btn
                    .bg(theme::with_alpha(0x22c55e, 0.12))
                    .text_color(theme::green())
                    .child(SharedString::from(format!("✓ 已绑定到「{role_name}」")))
            } else if remote_only {
                // 绑定会先确保安装；仅远程条目装不了，自然也不能绑
                install_btn
                    .border_1()
                    .border_color(theme::border())
                    .text_color(theme::dim())
                    .child("仅远程，暂不支持绑定")
            } else {
                let skill_for_bind = skill.clone();
                let label = format!("＋ 绑定到「{role_name}」");
                install_btn
                    .bg(theme::accent())
                    .text_color(gpui::rgb(0xffffff))
                    .cursor_pointer()
                    .hover(|d| d.opacity(0.88))
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                        this.attach_skill_to_role(team_idx, role_idx, skill_for_bind.clone(), cx);
                    }))
                    .child(SharedString::from(label))
            };
        } else if installed {
            install_btn = install_btn
                .bg(theme::with_alpha(0x22c55e, 0.12))
                .text_color(theme::green())
                .cursor_pointer()
                .hover(|d| d.bg(theme::with_alpha(0x22c55e, 0.2)))
                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                    this.uninstall_skill(&id, cx);
                }))
                .child("✓ 已安装（点击卸载）");
        } else if remote_only {
            install_btn = install_btn
                .border_1()
                .border_color(theme::border())
                .text_color(theme::dim())
                .child("仅远程，暂不支持安装");
        } else if busy_here {
            install_btn = install_btn
                .bg(theme::raised())
                .text_color(theme::muted())
                .child("安装中…");
        } else {
            install_btn = install_btn
                .bg(theme::accent())
                .text_color(gpui::rgb(0xffffff))
                .cursor_pointer()
                .hover(|d| d.opacity(0.88))
                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                    this.install_market_entry(entry_for_btn.clone(), cx);
                }))
                .child(if mcp { "安装 MCP" } else { "安装" });
        }

        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .px_5()
            .py_4()
            .border_t_1()
            .border_color(theme::border())
            .child(install_btn)
            .child(div().flex_1())
            // 已安装技能（本地导入）没有上游链接，不占位
            .when_some(entry.source_url.clone(), |d, url| {
                d.child(
                    div()
                        .id("detail-open-source")
                        .px_3()
                        .py_1p5()
                        .rounded_md()
                        .border_1()
                        .border_color(theme::border())
                        .text_xs()
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                        .on_click(cx.listener(move |_this, _: &gpui::ClickEvent, _w, cx| {
                            cx.open_url(&url);
                        }))
                        .child("↗ 打开来源"),
                )
            });
        panel = panel.child(footer);

        // ---- 遮罩 + 面板 ----
        // 必须是 flex 行布局：块级流会把面板排到左边（`flex_1` 占位也不生效）。
        div()
            .absolute()
            .flex()
            // 标题栏不盖：窗口按钮还要能点
            .top(px(theme::TITLEBAR_H))
            .left(px(0.0))
            .right(px(0.0))
            .bottom(px(0.0))
            .child(
                div()
                    .id("market-detail-mask")
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .size_full()
                    .bg(theme::with_alpha(0x000000, 0.35))
                    // 必须 occlude：gpui 的滚轮命中拿的是「鼠标下所有 hitbox」，
                    // 不阻断的话滚抽屉正文时，底层页面的 `skills-scroll`
                    // 也会跟着滚，看起来像「整个窗口往下拉」。
                    .occlude()
                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                        this.close_market_detail(cx);
                    })),
            )
            .child(div().flex_1())
            .child(panel)
            .into_any_element()
    }
}

/// 小徽标：标签行与形态行共用。
fn badge(label: &str, bg: gpui::Rgba, fg: gpui::Rgba) -> AnyElement {
    div()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(bg)
        .text_xs()
        .text_color(fg)
        .child(SharedString::from(label.to_string()))
        .into_any_element()
}

/// 正文预览：超长截断，避免一次构出上千个元素。
fn preview_detail(text: &str) -> String {
    let trimmed = text.trim();
    let truncated: String = trimmed.chars().take(DETAIL_PREVIEW_CHARS).collect();
    if truncated.chars().count() < trimmed.chars().count() {
        format!("{truncated}\n\n…（正文过长，仅预览前 {DETAIL_PREVIEW_CHARS} 字符）")
    } else {
        truncated
    }
}
