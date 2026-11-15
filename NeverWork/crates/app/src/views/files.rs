//! 右侧面板：文件树 / 改动 / 预览。
//!
//! 修复点：
//! - 之前三个 Tab（文件/改动/预览）只是样式，点了没反应 → 现在真的切换。
//! - 之前目录行没有点击处理，`FileNode.expanded` 写了从来没人读 → 现在可展开/折叠。
//! - 之前点文件靠「按文件名递归搜索」，同名文件会打开错的那个 → 现在用相对路径。

use gpui::prelude::*;
use gpui::{div, px, Context, SharedString};

use neverwork_ui::theme;

use crate::state::{AppState, RightTab};
use crate::views::text_fit::fit_text;

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
                .child(self.tab_button("Git", RightTab::Git, cx))
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
                .child("⎇ 本地")
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

    fn files_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                        .child(SharedString::from(self.workspace.name.clone())),
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

        for f in &self.files {
            let indent = 8.0 + (f.level as f32) * 12.0;
            let abs = self.root.join(&f.path);
            let is_open = !f.is_dir && self.open_path.as_deref() == Some(abs.as_path());

            let icon = if f.is_dir {
                if f.expanded {
                    "▾"
                } else {
                    "▸"
                }
            } else {
                "⬢"
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
                .child(
                    div()
                        .text_xs()
                        .text_color(if f.is_dir { theme::accent() } else { theme::dim() })
                        .child(SharedString::from(icon.to_string())),
                )
                .child(div().flex_1().child(SharedString::from(f.name.clone())));

            let row = if f.is_dir {
                let path = abs.clone();
                row.on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                    this.toggle_dir(path.clone(), cx);
                }))
            } else {
                let path = abs.clone();
                row.on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                    this.open_file(path.clone(), cx);
                }))
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

        col.child(tree)
    }

    /// Git 面板：分支 + 改动文件 + diff + 提交历史。
    fn git_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let st = &self.git;

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

        head = head.child(
            div()
                .text_xs()
                .text_color(theme::fg())
                .child(SharedString::from(if st.branch.is_empty() {
                    "—".to_string()
                } else {
                    fit_text(&st.branch, 110.0, 12.0)
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

        // ---- 不是仓库：给出明确提示 ----
        if !self.git_is_repo {
            let children = crate::git::child_repos(&self.root);
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
                    let path = self.root.join(&name);
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
            let selected = self.git_selected.as_deref() == Some(c.path.as_str());
            let path = c.path.clone();
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
                    .when(selected, |d| d.bg(theme::raised()))
                    .when(!selected, |d| d.hover(|d| d.bg(theme::with_alpha(0xffffff, 0.04))))
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                        this.select_git_file(path.clone(), cx);
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
                            .text_color(if selected { theme::fg() } else { theme::muted() })
                            .child(SharedString::from(fit_text(&c.path, 190.0, 12.0))),
                    ),
            );
        }
        col = col.child(list);

        // ---- 选中文件的 diff ----
        if let Some(sel) = &self.git_selected {
            let diff = crate::git::file_diff(&self.root, sel);
            let mut boxel = div()
                .id("git-diff")
                .flex()
                .flex_col()
                .gap_0p5()
                .flex_1()
                .min_h(px(0.0))
                .px_2()
                .py_1()
                .border_t_1()
                .border_color(theme::border())
                .overflow_scroll()
                .text_xs();

            if diff.is_empty() {
                boxel = boxel.child(
                    div()
                        .px_1()
                        .py_2()
                        .text_color(theme::dim())
                        .child("没有可显示的差异（可能是空文件或仅权限变化）"),
                );
            }
            for line in diff.iter().take(400) {
                let (fg, bg) = match line.marker.as_str() {
                    "+" => (theme::green(), theme::with_alpha(0x22c55e, 0.10)),
                    "-" => (theme::red(), theme::with_alpha(0xef4444, 0.10)),
                    "@" => (theme::accent(), theme::with_alpha(0x3b82f6, 0.10)),
                    _ => (theme::dim(), theme::with_alpha(0x000000, 0.0)),
                };
                boxel = boxel.child(
                    div()
                        .flex()
                        .gap_1()
                        .rounded_sm()
                        .bg(bg)
                        .whitespace_nowrap()
                        .child(
                            div()
                                .w(px(8.0))
                                .flex_shrink_0()
                                .text_color(fg)
                                .child(SharedString::from(line.marker.clone())),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .text_color(if line.marker == " " { theme::muted() } else { fg })
                                .child(SharedString::from(fit_text(&line.text, 300.0, 11.0))),
                        ),
                );
            }
            col = col.child(boxel);
        }

        col.child(self.git_commits(cx)).into_any_element()
    }

    /// 最近提交历史（放在 Git 面板底部）。
    fn git_commits(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div()
            .id("git-commits")
            .flex()
            .flex_col()
            .gap_0p5()
            .px_2()
            .py_1()
            .border_t_1()
            .border_color(theme::border())
            .max_h(px(150.0))
            .overflow_scroll()
            .text_xs();

        col = col.child(
            div()
                .px_1()
                .pb_1()
                .text_color(theme::dim())
                .child("最近提交"),
        );

        if self.git.commits.is_empty() {
            col = col.child(
                div()
                    .px_1()
                    .text_color(theme::dim())
                    .child("还没有提交"),
            );
            return col;
        }

        for (i, c) in self.git.commits.iter().take(12).enumerate() {
            col = col.child(
                div()
                    .id(SharedString::from(format!("gitcommit-{i}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(22.0))
                    .px_1()
                    .rounded_sm()
                    .hover(|d| d.bg(theme::with_alpha(0xffffff, 0.04)))
                    .child(
                        div()
                            .w(px(52.0))
                            .flex_shrink_0()
                            .text_color(theme::accent())
                            .child(SharedString::from(c.hash.clone())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_color(theme::muted())
                            .child(SharedString::from(fit_text(&c.summary, 150.0, 11.0))),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(theme::dim())
                            .child(SharedString::from(fit_text(&c.when, 60.0, 11.0))),
                    ),
            );
        }
        let _ = cx;
        col
    }

    fn preview_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().flex_1().min_h(px(0.0));

        let name = self.open_file_name();
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(px(34.0))
                .px_3()
                .border_b_1()
                .border_color(theme::border())
                .child(div().text_xs().text_color(theme::dim()).child("只读预览"))
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

        if self.open_path.is_none() {
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

