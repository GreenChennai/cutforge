//! 桌面壳根视图(A-02 拆分:原 app.rs 1971 行按依赖方向分四件,行为零变化):
//! - `mod.rs`(本件,<300 行):`DesktopApp` 状态结构 + 生命周期/窗口装配 +
//!   意图提交唯一出口 `submit`;
//! - `command_surface`:键位命令注册表(BUG-22,id/标题/快捷键/上下文同源)+
//!   菜单/快捷键 → MCP 命令的映射与拼装(纯函数可单测);
//! - `workspace_view`:面板装配 + 网络线程(SyncHub 长轮询消费)+ UI 对账泵 + 渲染;
//! - `playback_facade`:I1 播放状态机门面(`playback/` 的宿主接线,与 UI 解耦)。
//!
//! 数据流(与 Web 壳同构):手势 → `submit()`(/rpc)→ 内核 Op → 事件回流
//! (`GET /events` 长轮询线程置 dirty)→ UI 泵(100ms)重投影 → 视图刷新。
//! 壳不缓存真相:`Snapshot` 只是最近一次投影的只读副本。
//!
//! 播放(I1,docs/tickets/I1-S2):双通道——
//! - **流畅(默认)**:`PlaybackEngine` 直解码播放头所在 clip(M1),或
//!   `preview_zone_render` 预渲段优先(M2);引擎帧经播放泵(16ms)搬运上屏;
//! - **精确**:既有幻灯片路径(逐帧 `render_frame`)。
//!
//! 引擎构造/加载失败或运行中故障(is_faulted 含 Stalled)→ 自动回落幻灯片,
//! 原因进状态栏与设置页「播放」小节,不白屏不 panic。

use std::sync::Arc;
use std::time::{Duration, Instant};

use sable::gpui::{App, AppContext as _, Context, Entity, FocusHandle, Focusable, Window};

use crate::panels::preview::PreviewPanel;
use crate::playback::PlaybackEngine;
use crate::rpc::Rpc;
use crate::state::Shared;

mod command_surface;
mod playback_facade;
mod workspace_view;

/// 命令面对外消费口:设置页速查表从注册表生成(BUG-22 单一真相)。
pub(crate) use self::command_surface::shortcut_rows;
pub use self::playback_facade::source_map::fmt_speed;
use self::playback_facade::source_map::{EngineSource, ZoneReady};

/// 桌面壳业务根视图。
pub struct DesktopApp {
    pub rpc: Arc<Rpc>,
    pub shared: Arc<Shared>,
    /// 内核投影 rev 的 UI 侧对账值(≠ shared.rev 即待重投影)
    pub(crate) seen_rev: u64,
    pub timeline: Entity<sable::video::model::Timeline>,
    /// ClipId(u64)→ 内核字符串 id(回调还原用)
    pub id_map: std::collections::HashMap<u64, String>,
    /// 选中 clip 的内核字符串 id
    pub selection: Option<String>,
    pub playhead_ms: u64,
    /// 播放中(壳侧推进播放头)
    pub playing: bool,
    /// 吸附开关(拖拽/展示用;时间线工具行与设置页同源切换)
    pub snap_enabled: bool,
    /// 壳侧剪贴板(内核 clip id;clip_copy → Ctrl+V 经 clip_paste_at 落播放头)
    pub clipboard: Option<String>,
    /// 上一帧时刻(播放推进累加)
    pub(crate) last_frame: Instant,
    pub duration_ms: u64,
    pub status: String,
    /// 状态栏工程目录展示
    pub project_dir: String,
    /// 顶栏居中工程名(project.slug,重投影时刷新)
    pub project_title: String,
    pub dock: Option<Entity<sable::gpui_component::dock::DockArea>>,
    pub focus: FocusHandle,

    // ---- I1 播放引擎(M1 直解码 / M2 zone / M3 UI)----
    /// 播放引擎(惰性建;构造失败不持有,回幻灯片)
    pub(crate) engine: Option<PlaybackEngine>,
    /// 引擎当前加载的播放源;None = 幻灯片路径(引擎未参与)
    pub engine_source: Option<EngineSource>,
    /// 引擎回落幻灯片的原因(状态栏 + 设置页「播放」小节;None = 正常)
    pub engine_note: Option<String>,
    /// 引擎源解析时依据的快照 rev(播放中编辑重同步对账)
    pub(crate) engine_source_rev: u64,
    /// 变速挡(JKL;与片段速度相乘后驱动引擎)
    pub transport_speed: f32,
    /// 用户静音(≠变速自动静音)
    pub user_muted: bool,
    /// 当前片段 A→B 循环
    pub loop_clip: bool,
    /// 画质:true = 精确(逐帧 render_frame)/ false = 流畅(引擎,默认)
    pub quality_precise: bool,
    /// 沉浸预览(收起其他面板只留预览;非系统级全屏)
    pub immersive: bool,
    /// 预览面板句柄(引擎帧推送 / 沉浸态渲染)
    pub preview: Option<Entity<PreviewPanel>>,
    /// zone 请求进行中(from, to, rev)——角标显示 + 结果对账
    pub zone_loading: Option<(u64, u64, u64)>,
    /// 最近一次可用 zone(file, start, end, rev):暂停后原地重播免重渲
    pub(crate) zone_ready: Option<ZoneReady>,
    /// 拖进度条中(记原播放态;松手 seek 后恢复)
    pub(crate) scrubbing: Option<bool>,
    /// 播放中编辑 → 防抖重同步时刻
    pub(crate) resync_at: Option<Instant>,
    /// 待 toast(后台线程完成无 window;render 期 flush)
    pub(crate) pending_toasts: Vec<String>,
    /// 帧率统计窗口起点(播放日志证据)
    pub(crate) stat_at: Instant,
    /// 上次按钟消费帧时的引擎位置(帧步进门控;NEG_INF = 下帧立即可消费)
    pub(crate) frame_gate: f64,
    /// 帧供给落空已记日志(每供给断段只记一次)
    pub(crate) pop_miss: bool,
    /// 窗口内搬运帧数
    pub(crate) stat_frames: u64,
}

impl DesktopApp {
    /// 组装根视图(在 `cx.open_window` build 闭包里调用)。
    pub fn new(
        base: String,
        token: String,
        root_dir: String,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let rpc = Arc::new(Rpc::new(base, &token, root_dir.clone()));
        let shared = Arc::new(Shared::default());
        let timeline = cx.new(|_| sable::video::model::Timeline::new());

        let focus = cx.focus_handle();
        let focus_for_window = focus.clone();
        let project_dir = root_dir.clone();
        let app = cx.new(|cx| {
            // UI 泵:100ms 对账 shared.rev/dirty → 重投影;幻灯片播放推进
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    let _ = this
                        .update(cx, |app: &mut DesktopApp, cx: &mut Context<DesktopApp>| {
                            app.pump(cx)
                        });
                }
            })
            .detach();
            // 播放泵:16ms 引擎推进(播放头/帧搬运/段切换/重同步);
            // 空闲时只是读几个布尔,开销可忽略
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    if this
                        .update(cx, |app: &mut DesktopApp, cx: &mut Context<DesktopApp>| {
                            app.engine_pump(cx)
                        })
                        .is_err()
                    {
                        break; // 实体已释放(关窗),停泵
                    }
                }
            })
            .detach();
            Self {
                rpc,
                shared,
                seen_rev: 0,
                timeline,
                id_map: Default::default(),
                selection: None,
                playhead_ms: 0,
                playing: false,
                snap_enabled: true,
                clipboard: None,
                last_frame: Instant::now(),
                duration_ms: 0,
                status: format!("连接中…(工程 {root_dir})"),
                project_dir,
                project_title: std::path::Path::new(&root_dir)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root_dir.clone()),
                dock: None,
                focus,
                engine: None,
                engine_source: None,
                engine_note: None,
                engine_source_rev: 0,
                transport_speed: 1.0,
                user_muted: false,
                loop_clip: false,
                quality_precise: false,
                immersive: false,
                preview: None,
                zone_loading: None,
                zone_ready: None,
                scrubbing: None,
                resync_at: None,
                pending_toasts: Vec::new(),
                stat_at: Instant::now(),
                stat_frames: 0,
                frame_gate: f64::NEG_INFINITY,
                pop_miss: false,
            }
        });

        // 两阶段组装:面板回调需要 WeakEntity<Self>,先建根再建 dock
        app.update(cx, |app, cx| {
            workspace_view::assemble_dock(app, window, cx);
        });

        window.focus(&focus_for_window);
        // 网络线程:初载 + 事件长轮询(置 dirty/rev,UI 泵对账)
        let net_shared = app.read(cx).shared.clone();
        let net_rpc = app.read(cx).rpc.clone();
        std::thread::Builder::new()
            .name("cutforge-net".into())
            .spawn(move || workspace_view::net_loop(net_rpc, net_shared))
            .expect("网络线程启动失败");

        app
    }

    /// 意图提交(后台执行;成功置 dirty 由泵立即重投影,事件回流兜底)。
    pub fn submit(
        &mut self,
        tool: &'static str,
        params: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let rpc = self.rpc.clone();
        let shared = self.shared.clone();
        self.status = format!("{tool} …");
        cx.background_executor()
            .spawn(async move {
                match rpc.call(tool, params, crate::rpc::render_timeout(tool)) {
                    Ok(_data) => {
                        shared
                            .dirty
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    Err(e) => {
                        shared.set_error(e);
                    }
                }
            })
            .detach();
        cx.notify();
    }

    /// 吸附开关切换(时间线工具行/设置页同源)。
    pub fn toggle_snap(&mut self, cx: &mut Context<Self>) {
        self.snap_enabled = !self.snap_enabled;
        self.status = format!("吸附 {}", if self.snap_enabled { "开" } else { "关" });
        cx.notify();
    }
}

impl Focusable for DesktopApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}
