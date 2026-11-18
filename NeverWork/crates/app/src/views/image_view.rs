//! 图片放大查看器：对话 / 产物里的图片点开后盖满窗口。
//!
//! 交互：滚轮缩放（以中心为基准）、按住拖拽平移、双击复位；
//! Esc、点背景或右上角 ✕ 关闭。`zoom = 1.0` 是「适应窗口」。

use super::*;
use crate::views::text_fit::fit_text;
use gpui::{img, MouseButton, ScrollDelta};

/// 顶部工具条 / 底部操作提示的高度。
const HEAD_H: f32 = 48.0;
const HINT_H: f32 = 34.0;

/// 渲染放大查看器（`state.image_view` 为 `None` 时返回空元素）。
///
/// `visible_w/h` 用 render 里算好的窗口逻辑尺寸，决定适配比例与拖拽边界。
pub(crate) fn image_view_overlay(
    state: &AppState,
    visible_w: f32,
    visible_h: f32,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(view) = state.image_view.as_ref() else {
        return div().into_any_element();
    };
    let (nat_w, nat_h) = view.natural;
    // 标题栏不盖：上面铺着系统的拖动/最小化/最大化/关闭热区，
    // 顶栏工具条叠上去时点「＋」会变成最小化窗口（真实踩过）。
    let content_h = (visible_h - theme::TITLEBAR_H).max(120.0);
    let avail_w = (visible_w - 96.0).max(160.0);
    let avail_h = (content_h - HEAD_H - HINT_H - 24.0).max(120.0);
    // 小图不放大：fit 最多 1.0（100% = 原图像素）
    let fit = (avail_w / nat_w).min(avail_h / nat_h).min(1.0);
    let scale = (fit * view.zoom).max(0.01);
    let disp_w = (nat_w * scale).max(24.0);
    let disp_h = (nat_h * scale).max(24.0);
    // 平移边界：超出可视区域的部分 + 48px 余量；再保证至少 80px 图片留在窗口内
    let lim_x = ((disp_w - avail_w) / 2.0).max(0.0) + 48.0;
    let lim_y = ((disp_h - avail_h) / 2.0).max(0.0) + 48.0;
    let ox = view.offset.0.clamp(-lim_x, lim_x);
    let oy = view.offset.1.clamp(-lim_y, lim_y);
    let area_h = (content_h - HEAD_H - HINT_H).max(80.0);
    let left = ((visible_w - disp_w) / 2.0 + ox).clamp(80.0 - disp_w, visible_w - 80.0);
    let top = (HEAD_H + (area_h - disp_h) / 2.0 + oy)
        .clamp(HEAD_H + 24.0 - disp_h, content_h - HINT_H - 24.0);
    let percent = (scale * 100.0).round() as i32;
    let name = view.name();
    let path = view.path.clone();

    // ===== 顶部工具条 =====
    let tools = div()
        .flex()
        .items_center()
        .gap_1()
        .child(ctl_button("image-zoom-out", "－", cx, |this, cx| {
            this.zoom_image_view(1.0 / 1.25, cx);
        }))
        .child(
            div()
                .min_w(px(44.0))
                .text_center()
                .text_size(px(theme::text_caption()))
                .text_color(theme::fg())
                .child(SharedString::from(format!("{percent}%"))),
        )
        .child(ctl_button("image-zoom-in", "＋", cx, |this, cx| {
            this.zoom_image_view(1.25, cx);
        }))
        .child(ctl_button("image-zoom-fit", "适应", cx, |this, cx| {
            this.reset_image_view(cx);
        }))
        // 图片产物常从这里保存：与产物面板的⤓同一套导出逻辑
        .child(ctl_button("image-save", "保存", cx, |this, cx| {
            let Some(p) = this.image_view.as_ref().map(|v| v.path.clone()) else {
                return;
            };
            this.export_artifact(&p.to_string_lossy(), cx);
        }))
        .child(ctl_button("image-close", "✕", cx, |this, cx| {
            this.close_image_view(cx);
        }));

    let header = div()
        .absolute()
        .left_0()
        .right_0()
        .top_0()
        .h(px(HEAD_H))
        .px_4()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .text_size(px(theme::text_ui()))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::fg())
                .child(SharedString::from(fit_text(
                    &name,
                    (visible_w * 0.45).max(160.0),
                    theme::text_ui(),
                ))),
        )
        .child(
            div()
                .text_size(px(theme::text_caption()))
                .text_color(theme::muted())
                .child(SharedString::from(format!("{nat_w:.0}×{nat_h:.0}"))),
        )
        .child(div().flex_1())
        .child(tools);

    // ===== 图片本体：拖拽平移 / 双击复位 =====
    let canvas = div()
        .id("image-view-canvas")
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(disp_w))
        .h(px(disp_h))
        .rounded(px(theme::radius_md()))
        .border_1()
        .border_color(theme::border())
        .bg(theme::surface())
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, ev: &gpui::MouseDownEvent, _w, cx| {
                cx.stop_propagation();
                if ev.click_count >= 2 {
                    this.reset_image_view(cx);
                    return;
                }
                if let Some(v) = &mut this.image_view {
                    let p: (f32, f32) = (ev.position.x.into(), ev.position.y.into());
                    v.drag = Some((p, v.offset));
                }
            }),
        )
        .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, _w, cx| {
            if ev.pressed_button != Some(MouseButton::Left) {
                return;
            }
            let Some(v) = &mut this.image_view else { return };
            let Some((start, base)) = v.drag else { return };
            let p: (f32, f32) = (ev.position.x.into(), ev.position.y.into());
            v.offset = (base.0 + (p.0 - start.0), base.1 + (p.1 - start.1));
            cx.notify();
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _: &gpui::MouseUpEvent, _w, cx| {
                if let Some(v) = &mut this.image_view {
                    if v.drag.take().is_some() {
                        cx.notify();
                    }
                }
            }),
        )
        .child(
            img(path)
                .size_full()
                .object_fit(gpui::ObjectFit::Contain),
        );

    // ===== 底部提示 =====
    let hint = div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(px(HINT_H))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(theme::text_caption()))
        .text_color(theme::muted())
        .child("滚轮缩放 · 拖拽平移 · 双击复位 · Esc / 点背景关闭");

    // 点击遮罩关闭改到 mask 上（与 header/canvas/hint 互为兄弟），
    // 不能在根节点挂 on_mouse_down：实测根节点会先于按钮收到按下事件，
    // 点「＋」和「适应」会直接把查看器关掉。
    // mask 必须 `occlude`（同 market_detail），否则滚轮会同时滚背后的聊天流。
    let mask = div()
        .id("image-view-mask")
        .absolute()
        .inset_0()
        .occlude()
        .bg(theme::with_alpha(0x000000, 0.82))
        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
            this.close_image_view(cx);
        }))
        .on_scroll_wheel(cx.listener(|this, ev: &gpui::ScrollWheelEvent, _w, cx| {
            let dy: f32 = match ev.delta {
                ScrollDelta::Lines(p) => p.y,
                ScrollDelta::Pixels(p) => f32::from(p.y) / 60.0,
            };
            if dy.abs() < 0.05 {
                return;
            }
            let factor = if dy > 0.0 { 1.1 } else { 1.0 / 1.1 };
            this.zoom_image_view(factor, cx);
        }));

    div()
        .id("image-viewer")
        .absolute()
        .top(px(theme::TITLEBAR_H))
        .left_0()
        .right_0()
        .bottom_0()
        .child(mask)
        .child(header)
        .child(canvas)
        .child(hint)
        .into_any_element()
}

/// 工具条小按钮。
///
/// 只挂 `on_click`：遮罩是兄弟节点，点击按钮不会冒泡到 mask 的关闭回调。
fn ctl_button(
    id: &'static str,
    label: &'static str,
    cx: &mut Context<AppState>,
    action: impl Fn(&mut AppState, &mut Context<AppState>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(26.0))
        .px_2()
        .rounded(px(theme::radius_md()))
        .bg(theme::raised())
        .text_size(px(theme::text_caption()))
        .text_color(theme::fg())
        .cursor_pointer()
        .hover(|d| d.bg(theme::border()))
        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _w, cx| {
            action(this, cx);
        }))
        .child(label)
}
