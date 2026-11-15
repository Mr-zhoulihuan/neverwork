//! NeverWork 应用入口。

mod data;
mod filesystem;
mod find;
mod folding;
mod highlight;
mod input_handler;
mod keymap;
mod line_cache;
mod settings_ops;
mod skills_catalog;
mod state;
mod text_buffer;
mod views;

use gpui::prelude::*;
use gpui::{px, size, App, AppContext, Application, Bounds, Context, Render, Window, WindowBounds, WindowOptions, point};

use neverwork_ui::theme;
use state::AppState;

/// 计算窗口尺寸。
/// 计算窗口尺寸。
///
/// 说明：本机是 GameViewer 远程会话 + 200% 缩放，GPUI 得到的 scale=2，
/// 而窗口按 1:1 物理像素创建，导致内容被放大一倍后裁掉右半边。
/// 窗口尺寸本身仍按常规逻辑像素给出，布局侧的响应式折叠会处理可见范围
/// （见 `AppState::render`）。这里支持 NW_WIN_W / NW_WIN_H 覆盖，便于异常
/// DPI 环境下验证。
fn window_bounds(cx: &App) -> Bounds<gpui::Pixels> {
    let mut w: f32 = 1200.0;
    let mut h: f32 = 740.0;

    if let Some(display) = cx.primary_display() {
        let db = display.bounds();
        let dw: f32 = db.size.width.into();
        let dh: f32 = db.size.height.into();
        if dw > 0.0 && dh > 0.0 {
            // display 尺寸与窗口逻辑尺寸同单位时，这个上限才是安全的。
            w = w.min((dw * 0.94).max(560.0));
            h = h.min((dh * 0.94 - 24.0).max(360.0));
        }
    }

    // 显式覆盖，便于在异常 DPI 环境下验证界面。
    if let Ok(v) = std::env::var("NW_WIN_W") {
        if let Ok(v) = v.parse::<f32>() {
            w = v.max(480.0);
        }
    }
    if let Ok(v) = std::env::var("NW_WIN_H") {
        if let Ok(v) = v.parse::<f32>() {
            h = v.max(320.0);
        }
    }

    let mut bounds = Bounds::centered(None, size(px(w), px(h)), cx);
    if let Some(display) = cx.primary_display() {
        let db = display.bounds();
        let dw: f32 = db.size.width.into();
        let dh: f32 = db.size.height.into();
        let ox: f32 = db.origin.x.into();
        let oy: f32 = db.origin.y.into();
        bounds.origin = point(px(ox + ((dw - w) / 2.0).max(0.0)), px(oy + ((dh - h) / 2.0).max(0.0)));
    }
    bounds
}

/// 根视图：包裹主界面。
enum Root {
    Main(gpui::Entity<AppState>),
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        match self {
            Root::Main(e) => e.clone().into_any_element(),
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "neverwork=info,info".into()),
        )
        .init();

    let page = std::env::var("NW_PAGE").unwrap_or_default();
    tracing::info!("NeverWork 启动");
    // 统一入口：团队页复用主界面里的可交互团队编辑器，不再使用静态展示页。
    let start_team = page == "team";

    Application::new().run(move |cx: &mut App| {
        let bounds = window_bounds(cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_window, cx| {
                if start_team {
                    std::env::set_var("NW_VIEW", "team");
                }
                let main = cx.new(AppState::new);
                cx.new(|_| Root::Main(main))
            },
        )
        .expect("无法打开主窗口");

        cx.activate(true);
        let _ = (theme::SIDEBAR_W, theme::FILES_W);
    });
}
