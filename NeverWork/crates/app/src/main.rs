//! NeverWork 应用入口。

mod filesystem;
mod brackets;
mod find;
mod folding;
mod git;
mod git_ops;
mod highlight;
mod input_handler;
mod keymap;
mod markdown;
mod line_cache;
mod marketplace;
mod settings_ops;
mod skills_catalog;
mod skills_ops;
mod state;
mod text_buffer;
mod views;

use gpui::prelude::*;
use gpui::{px, size, App, AppContext, Application, Bounds, Context, Render, Window, WindowBounds, WindowOptions, point};

use neverwork_ui::theme;
use state::AppState;

/// 计算窗口尺寸与位置。
///
/// 关键约束：**窗口不能超出屏幕**。默认按屏幕可用区域的 92% 开窗并居中，
/// 再留出任务栏高度；`NW_WIN_W` / `NW_WIN_H` 可覆盖（覆盖值也会被夹回屏幕范围，
/// 否则在高 DPI 环境下窗口会大到看不见边缘，用户没法拖回来）。
///
/// 说明：本机是 GameViewer 远程会话 + 200% 缩放，逻辑屏 1280x800、物理 2560x1600。
/// `display.bounds()` 报的是逻辑尺寸，所以这里全程用逻辑值计算，不用再除缩放。
fn window_bounds(cx: &App) -> Bounds<gpui::Pixels> {
    // 逻辑屏幕尺寸与原点
    let (screen_w, screen_h, screen_x, screen_y) = match cx.primary_display() {
        Some(display) => {
            let b = display.bounds();
            (
                f32::from(b.size.width),
                f32::from(b.size.height),
                f32::from(b.origin.x),
                f32::from(b.origin.y),
            )
        }
        None => (1200.0, 760.0, 0.0, 0.0),
    };

    // 任务栏等占位：逻辑 40px 足够，最多占屏幕的 8%
    let chrome = (screen_h * 0.08).min(40.0);
    let usable_w = (screen_w * 0.92).max(560.0);
    let usable_h = (screen_h * 0.92 - chrome).max(400.0);

    let mut w = 1200.0f32.min(usable_w);
    let mut h = 740.0f32.min(usable_h);

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
    // 覆盖值也不能把窗口开出屏幕
    w = w.min(usable_w);
    h = h.min(usable_h);

    // 居中到屏幕，并夹在屏幕内（右下角不越界）
    let x = (screen_x + (screen_w - w) / 2.0).clamp(screen_x, screen_x + screen_w - w);
    let y = (screen_y + (screen_h - h - chrome) / 2.0).clamp(screen_y, screen_y + screen_h - h - chrome);

    Bounds {
        origin: point(px(x), px(y)),
        size: size(px(w), px(h)),
    }
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
    // 远程密码认证的 askpass 端点：ssh 用 `SSH_ASKPASS` 调回本程序要密码。
    // 必须最先判断，不能先建窗口 / 初始化日志；密码只从环境变量读一次，
    // 打印到 stdout 后立即退出（不落盘、不进命令行）。
    if std::env::var("NW_ASKPASS_MODE").as_deref() == Ok("1") {
        if let Ok(secret) = std::env::var("NW_ASKPASS_SECRET") {
            println!("{secret}");
        }
        return;
    }

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
                // 最小尺寸：再小布局会碎成一团，也没有继续拖的意义。
                window_min_size: Some(size(px(660.0), px(440.0))),
                // 必须显式声明透明标题栏：gpui 的默认值是 `appears_transparent: false`，
                // 那样 Windows 会在窗口最上面叠一条系统原生标题栏（浅色横条 + 左上角
                // 默认白色窗口图标，右键弹「还原/移动/关闭」系统菜单）。界面本身已经
                // 自绘标题栏，拖动与最小化/最大化/关闭由 `views::titlebar` 注册的热区接管。
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("NeverWork".into()),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                ..Default::default()
            },
            move |window, cx| {
                // 只在显式要求时最大化。之前在高 DPI 下默认自动最大化，
                // 会让窗口启动就贴满整屏、且无法用「还原」回到正常大小，
                // 观感上就是「一启动就超出屏幕」。
                if std::env::var("NW_MAXIMIZE")
                    .map(|v| v != "0")
                    .unwrap_or(false)
                {
                    window.zoom_window();
                }
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
