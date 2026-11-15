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

impl AppState {
    pub(crate) fn right_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                .child(self.tab_button("改动", RightTab::Changes, cx))
                .child(self.tab_button("预览", RightTab::Preview, cx)),
        );

        col = col.child(match self.right_tab {
            RightTab::Files => self.files_panel(cx).into_any_element(),
            RightTab::Changes => self.changes_panel(cx).into_any_element(),
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

        col
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

    fn changes_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col().flex_1().min_h(px(0.0));

        col = col.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(px(34.0))
                .px_3()
                .border_b_1()
                .border_color(theme::border())
                .child(div().text_xs().text_color(theme::dim()).child("最近修改"))
                .child(div().flex_1())
                .child(
                    div()
                        .id("refresh-changes")
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
                            this.refresh_recent(cx);
                        }))
                        .child("⟳"),
                ),
        );

        let mut list = div()
            .id("changes-list")
            .flex()
            .flex_col()
            .gap_0p5()
            .p_2()
            .flex_1()
            .overflow_scroll();

        if self.recent.is_empty() {
            list = list.child(
                div()
                    .p_3()
                    .text_xs()
                    .text_color(theme::dim())
                    .child("点右上角 ⟳ 扫描最近修改的文件"),
            );
        }
        for (rel, when) in &self.recent {
            let abs = self.root.join(rel);
            let is_open = self.open_path.as_deref() == Some(abs.as_path());
            list = list.child(
                div()
                    .id(SharedString::from(format!("chg-{rel}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(28.0))
                    .px_2()
                    .rounded_sm()
                    .text_sm()
                    .cursor_pointer()
                    .hover(|d| d.bg(theme::raised()))
                    .when(is_open, |d| d.bg(theme::raised()).text_color(theme::fg()))
                    .when(!is_open, |d| d.text_color(theme::muted()))
                    .child(div().text_xs().text_color(theme::orange()).child("◆"))
                    .child(div().flex_1().child(SharedString::from(rel.clone())))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::dim())
                            .child(SharedString::from(human_time(*when))),
                    )
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
                        this.open_file(abs.clone(), cx);
                    })),
            );
        }
        col.child(list)
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

fn human_time(t: std::time::SystemTime) -> String {
    let Ok(d) = std::time::SystemTime::now().duration_since(t) else {
        return "刚刚".into();
    };
    let secs = d.as_secs();
    match secs {
        0..=59 => "刚刚".into(),
        60..=3599 => format!("{} 分钟前", secs / 60),
        3600..=86399 => format!("{} 小时前", secs / 3600),
        86400..=2_592_000 => format!("{} 天前", secs / 86400),
        _ => format!("{} 个月前", secs / 2_592_000),
    }
}
