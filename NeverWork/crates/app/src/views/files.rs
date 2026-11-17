//! 右侧面板：文件树 / 改动 / 预览。
//!
//! 修复点：
//! - 之前三个 Tab（文件/改动/预览）只是样式，点了没反应 → 现在真的切换。
//! - 之前目录行没有点击处理，`FileNode.expanded` 写了从来没人读 → 现在可展开/折叠。
//! - 之前点文件靠「按文件名递归搜索」，同名文件会打开错的那个 → 现在用相对路径。

use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, SharedString};

use neverwork_ui::theme;

use crate::state::{AppState, MediaPreview, RightTab};
use crate::views::text_fit::fit_text;

/// 文件类型 →（徽标文字, 主题色）。
///
/// 颜色全部取自主题语义色（深浅两套自动跟随），徽标用半透明底 + 同色字，
/// 两个主题下对比度都够；只认常见扩展名，其余走中性灰。
fn badge_of(name: &str) -> (&'static str, gpui::Rgba) {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "rs" => ("RS", theme::orange()),
        "toml" | "lock" => ("TOML", theme::dim()),
        "md" | "markdown" => ("MD", theme::indigo()),
        "json" => ("{}", theme::yellow()),
        "yaml" | "yml" => ("YML", theme::purple()),
        "py" => ("PY", theme::cyan()),
        "js" | "mjs" | "cjs" => ("JS", theme::yellow()),
        "ts" | "tsx" => ("TS", theme::indigo()),
        "jsx" => ("JSX", theme::yellow()),
        "html" | "htm" => ("<>", theme::red()),
        "css" | "scss" | "less" => ("#", theme::pink()),
        "sh" | "bash" | "zsh" | "ps1" | "bat" | "cmd" => ("SH", theme::green()),
        "go" => ("GO", theme::cyan()),
        "c" | "h" => ("C", theme::indigo()),
        "cpp" | "cc" | "hpp" | "hxx" => ("C++", theme::indigo()),
        "java" => ("JAVA", theme::red()),
        "kt" => ("KT", theme::purple()),
        "pdf" => ("PDF", theme::red()),
        "txt" | "log" => ("TXT", theme::muted()),
        "sql" => ("SQL", theme::cyan()),
        "xml" => ("XML", theme::orange()),
        _ => {
            // 图片格式集合与媒体查看器共用一份，新增格式不会两边不同步。
            if crate::state::is_image_extension(&ext) {
                ("IMG", theme::pink())
            } else {
                ("·", theme::muted())
            }
        }
    }
}

/// 文件类型彩色徽标（文件树与对话附件 chip 共用）。
pub(crate) fn file_badge(name: &str) -> impl IntoElement {
    let (label, color) = badge_of(name);
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(26.0))
        .h(px(15.0))
        .flex_shrink_0()
        .rounded_sm()
        .bg(theme::tint(color, 0.16))
        .border_1()
        .border_color(theme::tint(color, 0.45))
        .text_size(px(9.0))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(color)
        .child(SharedString::from(label))
}

impl AppState {
    pub(crate) fn right_panel(
        &self,
        force_collapsed: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 收起态：只留一条窄边，点一下重新展开。
        // 这样在窄窗口里也能腾出宽度给中栏，而不是只能靠「专注模式」全收起。
        if self.right_collapsed || force_collapsed {
            return div()
                .flex()
                .flex_col()
                .items_center()
                .w(px(16.0))
                .flex_shrink_0()
                .h_full()
                .pt_2()
                .bg(theme::panel())
                .border_l_1()
                .border_color(theme::border())
                .child(
                    div()
                        .id("expand-right")
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(14.0))
                        .h(px(44.0))
                        .rounded_sm()
                        .text_xs()
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                            this.toggle_right_panel(cx);
                        }))
                        .child("◀"),
                )
                .into_any_element();
        }

        let mut col = div()
            .flex()
            .flex_col()
            .w(px(theme::FILES_W))
            .flex_shrink_0()
            .h_full()
            .bg(theme::panel())
            .border_l_1()
            .border_color(theme::border());

        // ===== Tab 头 =====
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .h(px(56.0))
                .px_3()
                .border_b_1()
                .border_color(theme::border())
                .child(self.tab_button("文件", RightTab::Files, cx))
                .child(self.tab_button("改动", RightTab::Git, cx))
                .child(self.tab_button("产物", RightTab::Artifacts, cx))
                .child(self.tab_button("预览", RightTab::Preview, cx))
                .child(div().flex_1())
                .child(
                    div()
                        .id("collapse-right")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(22.0))
                        .rounded_md()
                        .text_xs()
                        .text_color(theme::dim())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                            this.toggle_right_panel(cx);
                        }))
                        .child("▶"),
                ),
        );

        col = col.child(match self.right_tab {
            RightTab::Files => self.files_panel(cx).into_any_element(),
            RightTab::Git => self.git_panel(cx).into_any_element(),
            RightTab::Artifacts => self.artifacts_panel(cx).into_any_element(),
            RightTab::Preview => self.preview_panel(cx).into_any_element(),
        });

        // ===== 底部状态 =====
        col = col.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(theme::border())
                .text_xs()
                .text_color(theme::dim())
                .child(if self.remote.is_some() {
                    "⇅ 远程"
                } else {
                    "▤ 本地"
                })
                .child(SharedString::from(format!("{} 项", self.files.len()))),
        );

        col.into_any_element()
    }

    fn tab_button(
        &self,
        label: &'static str,
        tab: RightTab,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let on = self.right_tab == tab;
        div()
            .id(SharedString::from(format!("rt-{label}")))
            .flex()
            .items_center()
            .h(px(28.0))
            .px_2()
            .rounded_md()
            .text_sm()
            .cursor_pointer()
            .when(on, |d| d.bg(theme::raised()).text_color(theme::fg()))
            .when(!on, |d| d.text_color(theme::muted()).hover(|d| d.bg(theme::raised())))
            .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                this.set_right_tab(tab, cx);
            }))
            .child(SharedString::from(label))
    }

    /// 右栏「产物」：当前会话 Agent 写入的文件与生成的图片。
    ///
    /// 与对话流末尾的产物条同源（`Session.artifacts`）；这里给完整列表，
    /// 点一行就打开：图片走只读查看器，其余走编辑器。
    fn artifacts_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use neverwork_core::ArtifactKind;

        let list = self
            .sessions
            .get(self.active_session)
            .map(|s| s.artifacts.as_slice())
            .unwrap_or(&[]);
        let mut col = div().flex().flex_col().flex_1().min_h(px(0.0));
        col = col.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(34.0))
                .px_3()
                .border_b_1()
                .border_color(theme::border())
                .text_xs()
                .text_color(theme::dim())
                .child("本会话产物")
                .child(SharedString::from(format!("{} 项", list.len()))),
        );
        if list.is_empty() {
            return col.child(
                div()
                    .p_3()
                    .text_xs()
                    .line_height(px(20.0))
                    .text_color(theme::dim())
                    .child("暂无产物。Agent 写入的文件、生成的图片会自动收集到这里。"),
            );
        }
        let mut rows = div()
            .id("artifacts-list")
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .flex_1()
            .overflow_scroll();
        let mut ordered: Vec<&neverwork_core::Artifact> = list.iter().collect();
        ordered.reverse();
        for (i, a) in ordered.iter().enumerate() {
            let path = a.path.clone();
            let exists = std::path::Path::new(&a.path).exists();
            let mut row = div()
                .id(SharedString::from(format!("art-row-{i}")))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()))
                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                    this.open_artifact(&path, cx);
                }));
            if a.kind == ArtifactKind::Image && exists {
                row = row.child(
                    div()
                        .size(px(30.0))
                        .flex_shrink_0()
                        .rounded_sm()
                        .overflow_hidden()
                        .child(
                            gpui::img(std::path::PathBuf::from(&a.path))
                                .size(px(30.0))
                                .object_fit(gpui::ObjectFit::Cover),
                        ),
                );
            } else {
                row = row.child(file_badge(&a.name));
            }
            let sub = if self.remote.is_some() {
                "远程工作区产物，暂不能打开".to_string()
            } else if !exists {
                "文件已不存在".to_string()
            } else {
                String::new()
            };
            let mut info = div()
                .flex()
                .flex_col()
                .flex_1()
                .overflow_hidden()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::muted())
                        .whitespace_nowrap()
                        .child(SharedString::from(fit_text(&a.name, 180.0, 11.0))),
                );
            if !sub.is_empty() {
                info = info.child(
                    div()
                        .text_size(px(10.0))
                        .text_color(theme::dim())
                        .child(SharedString::from(sub)),
                );
            }
            // 行与「保存」按钮并排（不嵌套）：点⤓只导出，不打开文件
            let save = crate::views::chat::artifact_save_button(
                SharedString::from(format!("art-row-save-{i}")),
                a.path.clone(),
                cx,
            );
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(row.child(info).flex_1())
                    .child(save),
            );
        }
        col.child(rows)
    }

    fn files_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // 无工作区：文件树没得显示，给一扇门
        if !self.has_workspace() {
            return div()
                .flex()
                .flex_col()
                .items_start()
                .gap_2()
                .p_3()
                .text_xs()
                .text_color(theme::dim())
                .child("还没有选择工作区")
                .child(
                    div()
                        .id("files-open-folder")
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(theme::raised())
                        .border_1()
                        .border_color(theme::border())
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::border()).text_color(theme::fg()))
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                            this.pick_workspace_folder(cx);
                        }))
                        .child("＋ 打开文件夹…"),
                )
                .into_any_element();
        }
        let mut col = div().flex().flex_col().flex_1().min_h(px(0.0));

        col = col.child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .h(px(34.0))
                .px_3()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .id("refresh-tree")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(24.0))
                        .rounded_md()
                        .text_sm()
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::raised()))
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                            this.refresh_tree(cx);
                        }))
                        .child("⟳"),
                )
                .child(
                    div()
                        .id("collapse-all")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(24.0))
                        .rounded_md()
                        .text_sm()
                        .text_color(theme::muted())
                        .cursor_pointer()
                        .hover(|d| d.bg(theme::raised()))
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                            this.collapse_all(cx);
                        }))
                        .child("⊟"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::dim())
                        .child(SharedString::from(if let Some(t) = &self.remote {
                            fit_text(&t.display(), 140.0, 12.0)
                        } else {
                            self.workspace
                                .as_ref()
                                .map(|w| w.name.clone())
                                .unwrap_or_else(|| "无工作区".into())
                        })),
                ),
        );

        let mut tree = div()
            .id("file-tree")
            .flex()
            .flex_col()
            .gap_0p5()
            .p_2()
            .flex_1()
            .overflow_scroll();

        let remote = self.remote.is_some();
        for f in &self.files {
            let indent = 8.0 + (f.level as f32) * 12.0;
            let abs = self.abs_path(&f.path);
            let is_open = if remote {
                !f.is_dir && self.remote_open_rel.as_deref() == Some(f.path.as_str())
            } else {
                !f.is_dir && self.open_path.as_deref() == Some(abs.as_path())
            };

            // 文件行：目录继续用箭头，文件按类型给彩色徽标（一眼看出是什么文件）。
            let icon: AnyElement = if f.is_dir {
                div()
                    .text_xs()
                    .text_color(theme::accent())
                    .child(SharedString::from(if f.expanded { "▾" } else { "▸" }))
                    .into_any_element()
            } else {
                file_badge(&f.name).into_any_element()
            };

            let row = div()
                .id(SharedString::from(format!("frow-{}-{}", f.level, f.path)))
                .flex()
                .items_center()
                .gap_2()
                .h(px(28.0))
                .pl(px(indent))
                .pr_2()
                .rounded_sm()
                .text_sm()
                .cursor_pointer()
                .hover(|d| d.bg(theme::raised()))
                .when(is_open, |d| d.bg(theme::raised()).text_color(theme::fg()))
                .when(!is_open, |d| d.text_color(theme::muted()))
                .child(icon)
                .child(div().flex_1().child(SharedString::from(f.name.clone())));

            let row = if f.is_dir {
                if remote {
                    let rel = f.path.clone();
                    row.on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                        this.toggle_remote_dir(rel.clone(), cx);
                    }))
                } else {
                    let path = abs.clone();
                    row.on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                        this.toggle_dir(path.clone(), cx);
                    }))
                }
            } else if remote {
                let rel = f.path.clone();
                let rel_menu = f.path.clone();
                row.on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                    this.open_remote_file(rel.clone(), cx);
                }))
                // 右键：添加到会话 / 复制路径（远程文件没有本地副本，添加项置灰）
                .on_mouse_down(
                    gpui::MouseButton::Right,
                    cx.listener(move |this, ev: &gpui::MouseDownEvent, _w, cx| {
                        cx.stop_propagation();
                        let at = (ev.position.x.into(), ev.position.y.into());
                        this.open_file_menu(None, Some(rel_menu.clone()), false, at, cx);
                    }),
                )
            } else {
                let path = abs.clone();
                let abs_menu = abs.clone();
                row.on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                    this.open_file(path.clone(), cx);
                }))
                .on_mouse_down(
                    gpui::MouseButton::Right,
                    cx.listener(move |this, ev: &gpui::MouseDownEvent, _w, cx| {
                        cx.stop_propagation();
                        let at = (ev.position.x.into(), ev.position.y.into());
                        this.open_file_menu(Some(abs_menu.clone()), None, false, at, cx);
                    }),
                )
            };
            tree = tree.child(row);
        }

        if self.files.is_empty() {
            tree = tree.child(
                div()
                    .p_3()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("这个目录里没有可显示的文件"),
            );
        }

        col.child(tree).into_any_element()
    }

    /// Git 面板：分支 + 改动文件 + diff + 提交历史。
    fn git_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // 无工作区：没有仓库可看，直接给占位
        if !self.has_workspace() {
            return div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .text_xs()
                .text_color(theme::dim())
                .child("还没有选择工作区")
                .child(
                    div()
                        .text_color(theme::muted())
                        .child("点左侧会话栏顶部的工作区条，打开一个文件夹"),
                )
                .into_any_element();
        }
        let st = &self.git;

        // 远程工作区的 Git 面板还没接：说明清楚，别让用户以为仓库坏了
        if self.remote.is_some() {
            return div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .text_xs()
                .text_color(theme::dim())
                .child("远程工作区暂不支持改动面板")
                .child(
                    div()
                        .text_color(theme::muted())
                        .child("文件树、编辑器与 Agent 工具照常可用"),
                )
                .into_any_element();
        }

        let mut col = div().flex().flex_col().flex_1().min_h(px(0.0));

        // ---- 头部：分支 + 领先/落后 + 刷新 ----
        let mut head = div()
            .flex()
            .items_center()
            .gap_2()
            .h(px(34.0))
            .px_3()
            .border_b_1()
            .border_color(theme::border())
            .child(div().text_xs().text_color(theme::accent()).child("⎇"));

        // 分支名只读展示（改动面板只列文件，不提供分支操作）
        head = head.child(
            div()
                .px_1()
                .text_xs()
                .flex()
                .items_center()
                .whitespace_nowrap()
                .text_color(theme::fg())
                .child(SharedString::from(if st.branch.is_empty() {
                    "—".to_string()
                } else {
                    fit_text(&st.branch, 120.0, 12.0)
                })),
        );

        if st.ahead > 0 {
            head = head.child(
                div()
                    .text_xs()
                    .text_color(theme::green())
                    .child(SharedString::from(format!("↑{}", st.ahead))),
            );
        }
        if st.behind > 0 {
            head = head.child(
                div()
                    .text_xs()
                    .text_color(theme::yellow())
                    .child(SharedString::from(format!("↓{}", st.behind))),
            );
        }

        head = head
            .child(div().flex_1())
            .child(
                div()
                    .id("git-refresh")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(22.0))
                    .rounded_md()
                    .text_xs()
                    .text_color(theme::muted())
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::raised()).text_color(theme::fg()))
                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                        this.refresh_git(cx);
                    }))
                    .child("⟳"),
            );

        col = col.child(head);

        // ---- 多仓库切换（工作区里有多个仓库时才显示） ----
        if self.git_repos.len() > 1 {
            let mut bar = div()
                .id("git-repos")
                .flex()
                .flex_wrap()
                .gap_1()
                .px_3()
                .pb_2()
                .text_xs();
            bar = bar.child(
                div()
                    .w_full()
                    .pb_1()
                    .text_color(theme::dim())
                    .child(SharedString::from(format!(
                        "工作区内有 {} 个仓库",
                        self.git_repos.len()
                    ))),
            );
            let active = self.git_dir();
            for (name, path) in self.git_repos.iter() {
                let on = active.as_ref() == Some(path);
                let p2 = path.clone();
                bar = bar.child(
                    div()
                        .id(SharedString::from(format!("gitrepo-{name}")))
                        .px_2()
                        .py_0p5()
                        .rounded_sm()
                        .cursor_pointer()
                        .when(on, |d| {
                            d.bg(theme::accent_soft()).text_color(theme::accent())
                        })
                        .when(!on, |d| {
                            d.bg(theme::raised())
                                .text_color(theme::muted())
                                .hover(|d| d.bg(theme::border()))
                        })
                        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                            this.select_git_repo(p2.clone(), cx);
                        }))
                        .child(SharedString::from(fit_text(name, 90.0, 12.0))),
                );
            }
            col = col.child(bar);
        }

        // ---- 多仓库但还没选：明确提示（只看改动也要先选仓库） ----
        if self.git_active.is_none() && self.git_repos.len() > 1 {
            return col
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_3()
                        .text_xs()
                        .text_color(theme::dim())
                        .child("请先在上方选一个仓库")
                        .child(
                            div()
                                .text_color(theme::muted())
                                .child("工作区里有多个仓库，选好后才能查看改动"),
                        ),
                )
                .into_any_element();
        }

        // ---- 不是仓库：给出明确提示 ----
        if !self.git_is_repo {
            let children = self
                .root
                .as_ref()
                .map(|r| crate::git::child_repos(r))
                .unwrap_or_default();
            let mut wrap = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .text_xs()
                .text_color(theme::dim())
                .child("当前工作区不是 git 仓库");

            if children.is_empty() {
                wrap = wrap.child(
                    div()
                        .text_color(theme::muted())
                        .child("这里也没有子仓库。可以用 git init 初始化后再刷新。"),
                );
            } else {
                wrap = wrap.child(
                    div()
                        .text_color(theme::muted())
                        .child(format!("下面这些子目录是仓库（{} 个）", children.len())),
                );
                for name in children {
                    let path = self.abs_path(&name);
                    let label = name.clone();
                    wrap = wrap.child(
                        div()
                            .id(SharedString::from(format!("subrepo-{name}")))
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(26.0))
                            .px_2()
                            .rounded_sm()
                            .text_xs()
                            .cursor_pointer()
                            .hover(|d| d.bg(theme::raised()))
                            .child(div().text_color(theme::accent()).child("⎇"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_color(theme::fg())
                                    .child(SharedString::from(fit_text(&label, 180.0, 12.0))),
                            )
                            .child(
                                div()
                                    .text_color(theme::dim())
                                    .child("作为工作区 →"),
                            )
                            .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                                this.switch_workspace(path.clone(), cx);
                            })),
                    );
                }
            }
            return col.child(wrap).into_any_element();
        }

        // ---- 改动摘要 ----
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .text_xs()
                .text_color(theme::dim())
                .child(SharedString::from(crate::git::summary_line(st)))
                .when(st.has_conflicts(), |d| {
                    d.child(
                        div()
                            .px_1p5()
                            .py_0p5()
                            .rounded_sm()
                            .bg(theme::with_alpha(0xef4444, 0.18))
                            .text_color(theme::red())
                            .child("有冲突"),
                    )
                }),
        );

        // ---- 改动文件列表 ----
        let mut list = div()
            .id("git-changes")
            .flex()
            .flex_col()
            .gap_0p5()
            .px_2()
            // 必须 flex_shrink_0：否则在纵向 flex 里会被后面的 diff 挤成 0 高度，
            // 表现为「摘要说有 N 个文件，但列表一个都不显示」。
            .flex_shrink_0()
            .max_h(px(220.0))
            .overflow_scroll();

        if st.changes.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_2()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("工作区干净，没有未提交改动"),
            );
        }

        for (i, c) in st.changes.iter().enumerate() {
            let path = c.path.clone();
            let open_path = c.path.clone();
            let deleted = matches!(c.kind, crate::git::ChangeKind::Deleted);
            let color = match c.kind {
                crate::git::ChangeKind::Added => theme::green(),
                crate::git::ChangeKind::Modified => theme::yellow(),
                crate::git::ChangeKind::Deleted => theme::red(),
                crate::git::ChangeKind::Renamed => theme::cyan(),
                crate::git::ChangeKind::Conflicted => theme::red(),
            };
            let marker = c.kind.marker();
            list = list.child(
                div()
                    .id(SharedString::from(format!("gitchg-{i}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(24.0))
                    .px_2()
                    .rounded_sm()
                    .text_xs()
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::with_alpha(0xffffff, 0.04)))
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                        // 已删除的文件没有内容可打开，给个提示而不是静默失败
                        if deleted {
                            this.notice = Some("文件已删除，无法打开".into());
                            cx.notify();
                            return;
                        }
                        let Some(dir) = this.git_dir() else { return };
                        this.open_file(dir.join(&open_path), cx);
                    }))
                    .child(
                        div()
                            .w(px(12.0))
                            .flex_shrink_0()
                            .text_color(color)
                            .child(SharedString::from(marker)),
                    )
                    .when(c.staged, |d| {
                        d.child(
                            div()
                                .w(px(6.0))
                                .h(px(6.0))
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(theme::green()),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_color(theme::muted())
                            .child(SharedString::from(fit_text(&path, 190.0, 12.0))),
                    ),
            );
        }
        col = col.child(list);

        col.into_any_element()
    }

    fn preview_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().flex_1().min_h(px(0.0));

        let name = self.open_file_name();
        // 图片 / PDF / Markdown 走媒体分支；文本走行级只读预览。
        let media = self.media_preview.clone();
        let header_hint = match &media {
            Some(MediaPreview::Image(_)) => "图片预览",
            Some(MediaPreview::Pdf(_)) => "PDF 预览",
            Some(MediaPreview::Markdown(_)) => "Markdown 预览",
            None => "只读预览",
        };
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(px(34.0))
                .px_3()
                .border_b_1()
                .border_color(theme::border())
                .child(div().text_xs().text_color(theme::dim()).child(header_hint))
                .child(div().flex_1())
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::muted())
                        .child(SharedString::from(name.clone())),
                ),
        );

        let mut body = div()
            .id("preview-body")
            .flex()
            .flex_col()
            .p_2()
            .gap_0p5()
            .flex_1()
            .overflow_scroll()
            .text_xs();

        if let Some(media) = &media {
            match media {
                MediaPreview::Image(p) => {
                    let click = p.clone();
                    let thumb = gpui::img(p.clone())
                        .w_full()
                        .max_h(px(360.0))
                        .object_fit(gpui::ObjectFit::Contain);
                    let size = std::fs::metadata(p)
                        .map(|m| crate::state::human_size(m.len()))
                        .unwrap_or_default();
                    body = body.child(
                        div()
                            .id("preview-image")
                            .w_full()
                            .rounded_md()
                            .overflow_hidden()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::surface())
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                                this.open_image_view(click.clone(), cx);
                            }))
                            .child(thumb),
                    );
                    body = body.child(
                        div()
                            .pt_1()
                            .text_color(theme::dim())
                            .child(SharedString::from(format!("{name} · {size} · 图片"))),
                    );
                }
                MediaPreview::Pdf(_) => {
                    let size = media
                        .path()
                        .metadata()
                        .map(|m| crate::state::human_size(m.len()))
                        .unwrap_or_default();
                    body = body.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .p_2()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::raised())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .w(px(34.0))
                                            .h(px(42.0))
                                            .flex_shrink_0()
                                            .rounded_sm()
                                            .bg(theme::tint(theme::red(), 0.14))
                                            .border_1()
                                            .border_color(theme::tint(theme::red(), 0.45))
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(theme::red())
                                            .child("PDF"),
                                    )
                                    .child(
                                        div()
                                            .text_color(theme::fg())
                                            .child(SharedString::from(name.clone())),
                                    ),
                            )
                            .child(div().text_color(theme::dim()).child(format!(
                                "{size} · 内置查看暂不支持 PDF 排版渲染"
                            )))
                            .child(
                                div()
                                    .id("preview-open-external")
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .bg(theme::accent_soft())
                                    .text_color(theme::accent())
                                    .cursor_pointer()
                                    .hover(|d| d.bg(theme::border()))
                                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                                        this.open_media_externally(cx);
                                    }))
                                    .child("↗ 用默认应用打开"),
                            ),
                    );
                }
                MediaPreview::Markdown(p) => {
                    // 右栏窄：宽度按面板实际可用宽度扣除内边距。
                    let text = crate::filesystem::read_file(p).unwrap_or_default();
                    let avail = (theme::FILES_W - 48.0).max(160.0);
                    let mut col = div().flex().flex_col().gap_2().w_full().text_color(theme::fg());
                    if text.trim().is_empty() {
                        col = col.child(div().text_color(theme::dim()).child("（空文件）"));
                    } else {
                        for b in crate::markdown::parse(&text) {
                            col = col.child(crate::views::chat::markdown_block(
                                b,
                                avail,
                                self.root.as_deref(),
                                cx,
                            ));
                        }
                    }
                    body = body.child(col);
                }
            }
        } else if self.open_path.is_none() && self.remote_open_rel.is_none() {
            body = body.child(
                div()
                    .p_3()
                    .text_color(theme::dim())
                    .child("还没有打开文件。在「文件」里点一个文件即可预览。"),
            );
        } else {
            let lang = crate::highlight::lang_for(&name);
            let cache = self.line_cache.borrow_mut().sync(self.editor.text(), lang);
            let limit = cache.line_count().min(400);
            for i in 0..limit {
                let mut line_el = div().flex().flex_row().gap_2();
                line_el = line_el.child(
                    div()
                        .w(px(28.0))
                        .flex_shrink_0()
                        .text_color(theme::dim())
                        .child(SharedString::from(format!("{}", i + 1))),
                );
                let mut text_el = div().flex().flex_row().flex_1().min_w(px(0.0)).overflow_hidden();
                for span in cache.spans[i].iter() {
                    text_el = text_el.child(
                        div()
                            .text_color(crate::highlight::color_for(span.tok))
                            .child(SharedString::from(span.text.clone())),
                    );
                }
                if cache.texts[i].is_empty() {
                    text_el = text_el.child(div().child(" "));
                }
                body = body.child(line_el.child(text_el));
            }
            if cache.line_count() > limit {
                body = body.child(
                    div()
                        .p_2()
                        .text_color(theme::dim())
                        .child(SharedString::from(format!(
                            "… 还有 {} 行，请在编辑器中查看",
                            cache.line_count() - limit
                        ))),
                );
            }
        }

        let _ = cx;
        col.child(body)
    }
}

