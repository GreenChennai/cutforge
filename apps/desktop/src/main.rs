//! CutForge 桌面壳(剪映对标 docs/upstream/04;C-FE 骨架见 docs/upstream/03)。
//!
//! **壳铁律(docs/upstream/03 §1.1)**:真相在内核(`cutforge-cli serve`),
//! 本壳只做「投影 + 意图提交」——时间线吸附/时长推导/重叠判定等语义一律
//! 不落壳;编辑全部经 `/rpc` 工具(clip_update / clip_move / clip_trim …),
//! 回流经 `GET /events` 长轮询(2s 上限)+ 本地 rev 对账。
//!
//! 启动形态:
//! - `--root <工程目录>` → 编辑器(自拉内核子进程,随壳退出);
//! - 无 root → **开始界面**(Home;选择工程后 spawn 自身 --root,docs/04 S1);
//! - `--attach http://127.0.0.1:8787 --token T` 可接已运行的内核(与 Web 壳共存)。
//!
//! 运行:`cargo run -p cutforge-desktop -- --root <工程目录>`(或 start-desktop.cmd)

mod app;
mod home;
mod kernel;
mod panels;
mod playback;
mod rpc;
mod state;
mod ui;

use std::path::PathBuf;

use sable::gpui::{App, AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};
use sable::gpui_component::Root;

pub struct Args {
    /// 工程目录(内核 serve 的 --root);None = 开始界面
    pub root: Option<PathBuf>,
    pub port: u16,
    pub token: String,
    /// 已运行内核地址(给则不自拉子进程)
    pub attach: Option<String>,
    /// cutforge-cli 可执行文件(缺省按 PATH/target 搜索)
    pub cli: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut root = None;
    // 0 = 内核用系统临时端口(缺省)。固定端口在多实例/孤儿内核(taskkill /F
    // 不触发 Drop)场景必撞车,曾致多轮播放"冻结"假象——RPC 打到僵尸内核。
    let mut port = 0u16;
    let mut token = "cutforge-desktop-local".to_string();
    let mut attach = None;
    let mut cli = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = args.next().map(PathBuf::from),
            "--port" => port = args.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--token" => token = args.next().unwrap_or_else(|| token.clone()),
            "--attach" => attach = args.next(),
            "--cli" => cli = args.next().map(PathBuf::from),
            other => {
                eprintln!("未知参数 {other}(支持 --root/--port/--token/--attach/--cli)");
                std::process::exit(2);
            }
        }
    }
    Args {
        root,
        port,
        token,
        attach,
        cli,
    }
}

fn main() {
    let args = parse_args();

    // 无 --root → 开始界面(不拉内核;选工程后 spawn 自身 --root)
    eprintln!(
        "[route] root={:?} attach={:?} exe={:?}",
        args.root,
        args.attach,
        std::env::current_exe().ok()
    );
    if args.root.is_none() {
        eprintln!("[route] → 首页(root 缺失)");
        sable::gpui::Application::new()
            .with_assets(ui::icon::Assets)
            .run(move |cx: &mut App| {
                sable::dock::init(cx);
                // A-08:sable ColorTokens ← ui/theme.rs 语义层(跨壳同源桥接)
                ui::theme::inject(cx);
                sable::gpui_component::theme::Theme::change(
                    sable::gpui_component::ThemeMode::Dark,
                    None,
                    cx,
                );
                let bounds = Bounds::centered(None, size(px(1100.), px(700.)), cx);
                let options = WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(sable::gpui::TitlebarOptions {
                        title: Some("CutForge".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                cx.open_window(options, |window, cx| {
                    let shell = home::HomeApp::new(window, cx);
                    cx.new(|cx| Root::new(shell, window, cx))
                })
                .expect("开始界面开窗失败");
                cx.activate(true);
            });
        return;
    }

    let root_dir = args
        .root
        .clone()
        .expect("root 已分流,编辑器路径必有值")
        .display()
        .to_string();
    eprintln!("[route] → 编辑器 root={root_dir}");

    // 内核:自拉子进程(随壳退出)或附着已有实例
    let kernel = kernel::Kernel::start(&args);
    let base = match &kernel {
        Ok(k) => k.base_url(),
        Err(msg) => {
            eprintln!("内核启动失败:{msg}(仍开窗,可在界面重试)");
            format!("http://127.0.0.1:{}", args.port)
        }
    };

    // 内核看门狗句柄经 Shared 流向全面板(BUG-20:健康/重启次数/日志路径可消费)
    let (kernel_health, kernel_restarts, kernel_log) = match &kernel {
        Ok(k) => (
            Some(k.health_flag()),
            Some(k.restart_counter()),
            k.stderr_log(),
        ),
        Err(_) => (None, None, None),
    };

    sable::gpui::Application::new()
        .with_assets(ui::icon::Assets)
        .run(move |cx: &mut App| {
            // 两套主题全局各自初始化:sable tokens(widgets 面板自绘)+ gpui-component
            // (DockArea/tab/输入框 chrome)。sable::dock::init 已含 gpui_component::init
            // 与 sable theme::init,不能只调后者(丢 sable 主题即启动 panic)。
            // A-08:sable ColorTokens 由 ui/theme.rs 语义层派生注入(桥接而非替换),
            // 消除"两套主题并存"的一致性风险;gpui-component chrome 固定深色。
            sable::dock::init(cx);
            ui::theme::inject(cx);
            sable::gpui_component::theme::Theme::change(
                sable::gpui_component::ThemeMode::Dark,
                None,
                cx,
            );

            // 剪映 H3:打开即记录最近工程(recent.json,与开始界面共用)
            home::record_recent(&root_dir);

            let bounds = Bounds::centered(None, size(px(1560.), px(950.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(sable::gpui::TitlebarOptions {
                    title: Some("CutForge".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };

            let base = base.clone();
            let token = args.token.clone();
            let root_for_view = root_dir.clone();
            cx.open_window(options, move |window, cx| {
                let shell = app::DesktopApp::new(
                    base,
                    token,
                    root_for_view,
                    kernel_health.clone(),
                    kernel_restarts.clone(),
                    kernel_log.clone(),
                    window,
                    cx,
                );
                cx.new(|cx| Root::new(shell, window, cx))
            })
            .expect("桌面壳开窗失败:gpui 平台层初始化异常(显卡驱动/显示服务)");

            cx.activate(true);
        });
}
