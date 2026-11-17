//! NeverWork 应用入口。

// Release 版不能弹控制台：Rust 默认按控制台子系统编译，双击 exe（或从快捷方式启动）
// 时 Windows 会额外挂一个终端窗口，tracing 日志全打在里面。debug 保留控制台，
// 方便 `cargo run` 直接看到启动日志。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod filesystem;
mod brackets;
mod find;
mod folding;
mod git;
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

/// 单实例：命名互斥量 + 把已有实例的窗口带到前台。
///
/// 第二个实例不再开窗口（配置/会话文件、SSH 监听都不该跑两份），
/// 而是找到第一个进程的主窗口激活后自己退出。互斥量名用 `Local\` 前缀，
/// 每个登录会话一份，多用户互不影响。
#[cfg(windows)]
mod single_instance {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(attr: *mut c_void, owner: i32, name: *const u16) -> *mut c_void;
        fn GetLastError() -> u32;
        fn CloseHandle(h: *mut c_void) -> i32;
        fn EnumWindows(cb: extern "system" fn(*mut c_void, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: *mut c_void, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: *mut c_void) -> i32;
        fn ShowWindow(hwnd: *mut c_void, cmd: i32) -> i32;
        fn SetForegroundWindow(hwnd: *mut c_void) -> i32;
        fn GetCurrentProcessId() -> u32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn QueryFullProcessImageNameW(
            proc: *mut c_void,
            flags: u32,
            buf: *mut u16,
            size: *mut u32,
        ) -> i32;
    }

    const ERROR_ALREADY_EXISTS: u32 = 183;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const SW_RESTORE: i32 = 9;

    struct FindCtx {
        self_pid: u32,
        found: *mut c_void,
    }

    /// 枚举顶层可见窗口，按进程 exe 名（neverwork.exe）认自己人。
    /// 不靠窗口标题：无边框自绘窗口的标题不可靠，进程名才稳。
    extern "system" fn enum_cb(hwnd: *mut c_void, lparam: isize) -> i32 {
        unsafe {
            let ctx = &mut *(lparam as *mut FindCtx);
            if IsWindowVisible(hwnd) == 0 {
                return 1;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == 0 || pid == ctx.self_pid {
                return 1;
            }
            let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if proc.is_null() {
                return 1;
            }
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(proc, 0, buf.as_mut_ptr(), &mut len);
            CloseHandle(proc);
            if ok == 0 {
                return 1;
            }
            let name = String::from_utf16_lossy(&buf[..len as usize]).to_ascii_lowercase();
            if name.ends_with("neverwork.exe") {
                ctx.found = hwnd;
                return 0; // 找到就停
            }
            1
        }
    }

    /// `true` = 首个实例，继续启动；`false` = 已有实例（已尽量激活其窗口），本进程退出。
    pub fn acquire_or_activate() -> bool {
        let name: Vec<u16> = OsStr::new("Local\\NeverWork-SingleInstance-v1")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            // handle 故意不关：进程活着互斥量就在，退出时系统自动回收。
            let h = CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr());
            if h.is_null() || GetLastError() != ERROR_ALREADY_EXISTS {
                return true;
            }
            let mut ctx = FindCtx {
                self_pid: GetCurrentProcessId(),
                found: std::ptr::null_mut(),
            };
            EnumWindows(enum_cb, &mut ctx as *mut FindCtx as isize);
            if !ctx.found.is_null() {
                ShowWindow(ctx.found, SW_RESTORE);
                SetForegroundWindow(ctx.found);
            }
            false
        }
    }
}

#[cfg(not(windows))]
mod single_instance {
    /// 非 Windows 暂不限制多开（主目标平台是 Windows）。
    pub fn acquire_or_activate() -> bool {
        true
    }
}

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

/// 崩溃兜底：把 panic 消息与调用栈追加到配置目录下的 `crash.log`。
///
/// release 版没有控制台（`windows_subsystem = "windows"`），panic 默认只写 stderr，
/// 进程一死信息就跟着没了，用户只能反馈「闪退」。落盘一份之后，
/// 窗口拖动/缩放这类偶发崩溃下次可以直接看到 panic 位置。
/// 注意：栈溢出 / abort（0xc0000409）不经过 panic 钩子，那类崩溃得靠系统 dump。
fn install_crash_log() {
    let dir = std::env::var("NW_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .unwrap_or_else(|_| ".".into());
            std::path::Path::new(&home).join(".neverwork")
        });
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let bt = std::backtrace::Backtrace::force_capture();
        let text = format!("=== panic @ unix {ts} ===\n{info}\n--- backtrace ---\n{bt}\n\n");
        if std::fs::create_dir_all(&dir).is_ok() {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("crash.log"))
            {
                let _ = f.write_all(text.as_bytes());
            }
        }
        default_hook(info);
    }));
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

    // 单实例：已有实例在跑就激活它的窗口并退出。必须在 askpass 分支之后
    // （ssh 密码回调会反复启动本进程）。`NW_ALLOW_MULTI=1` 仅供自动化验证并行多开。
    if std::env::var("NW_ALLOW_MULTI").as_deref() != Ok("1")
        && !single_instance::acquire_or_activate()
    {
        return;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "neverwork=info,info".into()),
        )
        .init();
    // 崩溃信息落盘：release 没有控制台，闪退时靠 `~/.neverwork/crash.log` 定位。
    install_crash_log();

    let page = std::env::var("NW_PAGE").unwrap_or_default();
    tracing::info!("NeverWork 启动");
    // 统一入口：团队页复用主界面里的可交互团队编辑器，不再使用静态展示页。
    let start_team = page == "team";

    // `.with_assets`：`svg()` 元素要通过资源源取文件；默认的 `()` 资源源永远返回
    // None，图标栏的矢量图标会直接不显示。
    Application::new()
        .with_assets(assets::EmbeddedAssets)
        .run(move |cx: &mut App| {
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
