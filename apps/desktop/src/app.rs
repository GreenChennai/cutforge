//! 桌面壳根视图:顶部工具栏(撤销/重做 · 分割/复制/删除 · 加轨)+ sable-dock
//! 工作台(左媒体库 / 中预览 / 右检查器 / 下时间轴)+ 底部状态栏 +
//! 网络线程 + UI 泵 + 播放引擎泵 + 键盘快捷键。
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

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use sable::dock::{SablePanel, WorkspacePresets};
use sable::gpui::prelude::FluentBuilder as _;
use sable::gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, StatefulInteractiveElement as _,
    Styled as _, Window, div, px,
};
use sable::gpui_component::WindowExt as _;
use sable::gpui_component::dock::DockArea;
use sable::gpui_component::notification::Notification;
use sable::widgets::prelude::{SpacingTokens, h_flex};
use sable::widgets::theme::theme;
use sable::widgets::tokens::{ColorTokens, FONT_SIZE_CAPTION, FONT_SIZE_HEADING};

use crate::panels::inspector::InspectorPanel;
use crate::panels::library::LibraryPanel;
use crate::panels::preview::PreviewPanel;
use crate::panels::timeline::TimelineHost;
use crate::playback::{PlaybackEngine, zone as zone_cache};
use crate::rpc::Rpc;
use crate::state::{EngineFrame, Shared, ZoneRendered};

/// zone 请求跨度(播放头起向后 8s;钳内容长度,100ms 网格量化在内核)。
const ZONE_SPAN_MS: u64 = 8_000;
/// 播放中编辑 → 重同步防抖。
const RESYNC_DEBOUNCE: Duration = Duration::from_millis(300);
/// JKL 变速挡:L 正向 / Shift+L 反向 / J 全梯减速。
const SPEED_LADDER_UP: [f32; 3] = [1.0, 2.0, 4.0];
const SPEED_LADDER_DOWN: [f32; 3] = [1.0, 0.5, 0.25];
const SPEED_LADDER_ALL: [f32; 5] = [4.0, 2.0, 1.0, 0.5, 0.25];

/// 桌面壳业务根视图。
pub struct DesktopApp {
    pub rpc: Arc<Rpc>,
    pub shared: Arc<Shared>,
    /// 内核投影 rev 的 UI 侧对账值(≠ shared.rev 即待重投影)
    seen_rev: u64,
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
    last_frame: Instant,
    pub duration_ms: u64,
    pub status: String,
    /// 状态栏工程目录展示
    pub project_dir: String,
    /// 顶栏居中工程名(project.slug,重投影时刷新)
    pub project_title: String,
    pub dock: Option<Entity<DockArea>>,
    pub focus: FocusHandle,

    // ---- I1 播放引擎(M1 直解码 / M2 zone / M3 UI) ----
    /// 播放引擎(惰性建;构造失败不持有,回幻灯片)
    engine: Option<PlaybackEngine>,
    /// 引擎当前加载的播放源;None = 幻灯片路径(引擎未参与)
    pub engine_source: Option<EngineSource>,
    /// 引擎回落幻灯片的原因(状态栏 + 设置页「播放」小节;None = 正常)
    pub engine_note: Option<String>,
    /// 引擎源解析时依据的快照 rev(播放中编辑重同步对账)
    engine_source_rev: u64,
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
    zone_ready: Option<ZoneReady>,
    /// 拖进度条中(记原播放态;松手 seek 后恢复)
    scrubbing: Option<bool>,
    /// 播放中编辑 → 防抖重同步时刻
    resync_at: Option<Instant>,
    /// 待 toast(后台线程完成无 window;render 期 flush)
    pending_toasts: Vec<String>,
    /// 帧率统计窗口起点(播放日志证据)
    stat_at: Instant,
    /// 上次按钟消费帧时的引擎位置(帧步进门控;NEG_INF = 下帧立即可消费)
    frame_gate: f64,
    /// 帧供给落空已记日志(每供给断段只记一次)
    pop_miss: bool,
    /// 窗口内搬运帧数
    stat_frames: u64,
}

/// 引擎当前播放源(M1 直解码片段 / M2 zone 预渲段)。
#[derive(Clone, Debug)]
pub enum EngineSource {
    /// 直解码:播放头所在视频片段的原素材(工程内绝对路径)
    Direct {
        clip_id: String,
        src: PathBuf,
        clip_start_ms: u64,
        clip_end_ms: u64,
        /// 源内入点(媒体绝对域;时间映射基准 t = clip_start + (m − source_in)/s)
        source_in: f64,
        /// 本次流覆盖的媒体内区间
        media_in: f64,
        media_out: f64,
        /// 片段恒速(speed 字段;speedCurve 暂按 1.0,已知局限)
        clip_speed: f64,
    },
    /// zone 预渲段(preview_zone_render 产物;1:1 工程时间)
    Zone {
        file: PathBuf,
        start_ms: u64,
        end_ms: u64,
    },
}

/// 直解码源解析结果(播放头所在视频片段;媒体域为源内绝对时间)。
struct DirectSource {
    clip_id: String,
    src: PathBuf,
    clip_start_ms: u64,
    clip_end_ms: u64,
    /// 源内入点(sourceInMs;时间映射基准)
    source_in: f64,
    /// 本次流覆盖的媒体内区间
    media_in: f64,
    media_out: f64,
    /// 片段恒速(speed 字段;speedCurve 暂按 1.0,已知局限)
    clip_speed: f64,
}

impl DirectSource {
    /// 与当前引擎源逐字段对账(播放中编辑重同步:变了才重启流)。
    fn matches_engine(&self, engine_source: &EngineSource) -> bool {
        match engine_source {
            EngineSource::Direct {
                clip_id,
                src,
                clip_start_ms,
                clip_end_ms,
                source_in,
                media_in,
                media_out,
                ..
            } => {
                self.clip_id == *clip_id
                    && self.src == *src
                    && self.clip_start_ms == *clip_start_ms
                    && self.clip_end_ms == *clip_end_ms
                    && self.source_in == *source_in
                    && self.media_in == *media_in
                    && self.media_out == *media_out
            }
            _ => false,
        }
    }
}

/// 已渲染可用的 zone(暂停后原地重播免重渲;rev/marker 失配即作废)。
#[derive(Clone)]
struct ZoneReady {
    file: PathBuf,
    start_ms: u64,
    end_ms: u64,
    /// 内核内容寻址键(指纹 marker 判定用)
    key: String,
    rev: u64,
}

/// 播放路径日志(工单验收:播放路径有日志证据——帧率/回落事件)。
/// GUI 形态下 stderr 常不可见:默认落系统临时目录,CUTFORGE_PLAY_LOG 可覆盖路径。
fn play_log(msg: &str) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    eprintln!("[cutforge-play @{stamp}] {msg}");
    let path = std::env::var("CUTFORGE_PLAY_LOG").unwrap_or_else(|_| {
        std::env::temp_dir()
            .join("cutforge-play.log")
            .display()
            .to_string()
    });
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write as _;
        let _ = writeln!(f, "[cutforge-play @{stamp}] {msg}");
    }
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
            let this = cx.entity();
            let shared = app.shared.clone();
            let rpc = app.rpc.clone();
            let library = SablePanel::create(
                "媒体库",
                LibraryPanel::new(shared.clone(), rpc.clone(), &this, window, cx).into(),
                cx,
            );
            let preview_view = PreviewPanel::new(shared.clone(), rpc.clone(), &this, cx);
            let preview = SablePanel::create("预览", preview_view.clone().into(), cx);
            let inspector = SablePanel::create("检查器", InspectorPanel::new(&this, cx).into(), cx);
            let settings = SablePanel::create(
                "设置",
                crate::panels::settings::SettingsPanel::new(&this, cx).into(),
                cx,
            );
            let timeline_host = TimelineHost::new(&this, app.timeline.clone(), cx);
            let timeline_panel = SablePanel::create("时间轴", timeline_host.clone().into(), cx);

            let dock = WorkspacePresets::build_workspace(
                "cutforge-desktop",
                vec![library],
                preview,
                vec![inspector, settings],
                window,
                cx,
            );
            dock.update(cx, |area, cx| {
                let dock_weak = cx.entity().downgrade();
                area.set_bottom_dock(
                    sable::dock::tab_group(vec![timeline_panel], &dock_weak, window, cx),
                    Some(px(320.)),
                    true,
                    window,
                    cx,
                );
            });
            app.dock = Some(dock);
            app.preview = Some(preview_view);
        });

        window.focus(&focus_for_window);
        // 网络线程:初载 + 事件长轮询(置 dirty/rev,UI 泵对账)
        let net_shared = app.read(cx).shared.clone();
        let net_rpc = app.read(cx).rpc.clone();
        std::thread::Builder::new()
            .name("cutforge-net".into())
            .spawn(move || net_loop(net_rpc, net_shared))
            .expect("网络线程启动失败");

        app
    }
}

/// 网络线程:初载全量 → 事件长轮询循环(任何成功响应置 connected)。
fn net_loop(rpc: Arc<Rpc>, shared: Arc<Shared>) {
    loop {
        match load_once(&rpc, &shared) {
            Ok(()) => break,
            Err(e) => {
                shared.set_error(e);
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
    let mut since: u64 = 0;
    loop {
        match rpc.poll_events(since) {
            Ok(events) => {
                shared.connected.store(true, Ordering::Relaxed);
                let next = max_seq_of(&events);
                // 空轮询不置 dirty:否则每秒一次无谓重投影(状态栏闪烁)
                if next.map(|seq| seq > since).unwrap_or(false) {
                    since = next.unwrap_or(since);
                    shared.dirty.store(true, Ordering::Relaxed);
                }
            }
            Err(_) => {
                // 长轮询超时/瞬时网络错属正常,静默重试(Web 壳同口径)
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

/// 宽容解析事件响应:`{seq, events:[…]}` 或数组,取能看到的最大 seq。
fn max_seq_of(events: &serde_json::Value) -> Option<u64> {
    let top_seq = events.get("seq").and_then(serde_json::Value::as_u64);
    let arr = events
        .get("events")
        .and_then(|e| e.as_array())
        .or_else(|| events.as_array());
    let max_in = arr.and_then(|a| {
        a.iter()
            .filter_map(|e| e.get("seq").and_then(serde_json::Value::as_u64))
            .max()
    });
    max_in.or(top_seq)
}

fn load_once(rpc: &Rpc, shared: &Arc<Shared>) -> Result<(), String> {
    let project_env = rpc.call(
        "project_get",
        serde_json::json!({}),
        Duration::from_secs(10),
    )?;
    // project_get data 形状 = {project: <工程文档>, rev}(实测探针);
    // 工程文档在内层 project 键,宽容兼容平铺形状
    let project = project_env
        .get("project")
        .cloned()
        .unwrap_or_else(|| project_env.clone());
    let timeline = rpc.call(
        "timeline_get",
        serde_json::json!({}),
        Duration::from_secs(10),
    )?;
    let ui_fields = rpc
        .get("/ui-fields", Duration::from_secs(10))
        .unwrap_or(serde_json::Value::Null);
    let catalogs = rpc
        .get("/catalogs", Duration::from_secs(10))
        .unwrap_or(serde_json::Value::Null);
    let media = rpc
        .call(
            "media_browse",
            serde_json::json!({}),
            Duration::from_secs(30),
        )
        .map(|data| crate::state::media_entries(&data))
        .unwrap_or_default();
    let tracks = crate::state::track_metas(&project);
    let rev = timeline.get("rev").and_then(|v| v.as_u64()).unwrap_or(0);
    let clips = timeline
        .get("clips")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    shared.store_snapshot(crate::state::Snapshot {
        project,
        tracks,
        clips,
        ui_fields,
        catalogs,
        media,
        rev,
    });
    Ok(())
}

impl DesktopApp {
    /// UI 泵(100ms):错误横幅 / 播放推进 / rev 对账重投影。
    fn pump(&mut self, cx: &mut Context<Self>) {
        if let Some(err) = self.shared.take_error() {
            let prefix = if err.contains("失败[") {
                "操作失败"
            } else {
                "连接异常"
            };
            self.status = format!("{prefix}:{err}");
            if self.playing {
                self.playing = false;
            }
            cx.notify();
            return;
        }
        // 播放推进(幻灯片路径):引擎在场时由播放泵推进(16ms),此处不抢。
        // 按真实流逝时间步进(幻灯片式:出帧速度跟上为准)
        if self.playing && !self.engine_active() && self.scrubbing.is_none() {
            let elapsed = self.last_frame.elapsed().as_millis() as u64;
            self.last_frame = Instant::now();
            if elapsed > 0 {
                let next = (self.playhead_ms + elapsed)
                    .min(self.duration_ms.saturating_sub(100).max(self.playhead_ms));
                self.playhead_ms = next;
                *self.shared.preview_request.lock().unwrap() = Some(next);
                if next >= self.duration_ms {
                    self.playing = false;
                    self.status = "播放结束".to_string();
                }
            }
        }
        // 截图完成 → toast(render 期经 window 推送)
        if let Ok(mut g) = self.shared.screenshot_result.lock()
            && let Some(res) = g.take()
        {
            match res {
                Ok(msg) => self.pending_toasts.push(msg),
                Err(e) => self.pending_toasts.push(format!("截图失败:{e}")),
            }
            cx.notify();
        }
        // 对账两信号,刻意分离:
        // - dirty(事件回流/提交成功)= 内核可能变了 → 后台**重拉**全量快照
        //   (快照只是副本,原地重投影是陈旧数据的幻觉——实测插入片段不刷新
        //   的根因);reloading 守卫防重入;
        // - snapshot_rev ≠ seen_rev = 内有新快照 → 本地重投影(廉价,克隆)。
        let dirty = self.shared.dirty.swap(false, Ordering::Relaxed);
        let reloading = self.shared.reloading.load(Ordering::Relaxed);
        if dirty && !reloading {
            self.shared.reloading.store(true, Ordering::Relaxed);
            let rpc = self.rpc.clone();
            let shared = self.shared.clone();
            cx.background_executor()
                .spawn(async move {
                    let result = load_once(&rpc, &shared);
                    shared.reloading.store(false, Ordering::Relaxed);
                    if let Err(e) = result {
                        shared.set_error(e);
                    }
                })
                .detach();
        }
        if !reloading && self.shared.snapshot_rev() != self.seen_rev {
            self.reproject(cx);
        }
    }

    /// 重投影:快照 → Sable Timeline 视图(壳零语义,见 state.rs)。
    fn reproject(&mut self, cx: &mut Context<Self>) {
        let snap = self.shared.snapshot();
        self.seen_rev = snap.rev;
        let (tl, id_map) = crate::state::project_timeline(&snap);
        self.id_map = id_map;
        self.duration_ms = snap.duration_ms();
        self.timeline.update(cx, |view, _| *view = tl);
        // 引擎源随内容作废:任何提交 → zone/直解码流都基于旧工程,
        // 暂停态直接弃源(静帧走 render_frame 反映编辑结果);
        // 播放态交播放泵 300ms 防抖重同步(避免每次提交重启流)
        if let Some(zr) = &self.zone_ready
            && zr.rev != snap.rev
        {
            self.zone_ready = None;
        }
        if self.engine_source.is_some() && !self.playing {
            self.drop_engine_source(cx);
        }
        // 编辑回流 → 预览随播放头刷新(播放中由推进逻辑自刷);
        // 请求点钳在最后一帧之前(精确 =duration 抽帧会落空,内核无帧可出)
        if !self.playing {
            let at = self.playhead_ms.min(self.duration_ms.saturating_sub(100));
            *self.shared.preview_request.lock().unwrap() = Some(at);
        }
        self.project_title = snap
            .project
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                std::path::Path::new(&self.project_dir)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| self.project_dir.clone())
            });
        // 播放中不覆盖「播放中…」状态(rev 刷新常态发生)
        if !self.playing {
            self.status = format!("就绪 · rev {}", snap.rev);
        }
        cx.notify();
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
                        shared.dirty.store(true, Ordering::Relaxed);
                    }
                    Err(e) => {
                        shared.set_error(e);
                    }
                }
            })
            .detach();
        cx.notify();
    }

    pub fn set_playhead(&mut self, ms: u64, cx: &mut Context<Self>) {
        let mut ms = ms;
        if self.duration_ms > 0 {
            ms = ms.min(self.duration_ms);
        }
        self.playhead_ms = ms;
        // 引擎在场:源内 seek(重启流,约百 ms;离散按键可接受),越界弃源。
        // 播放态保持——引擎 seek 不改播放态,恢复由调用方(如 end_scrub)处理
        if let Some(src) = self.engine_source.clone() {
            match &src {
                EngineSource::Zone {
                    start_ms, end_ms, ..
                } => {
                    let (zs, ze) = (*start_ms, *end_ms);
                    if ms >= zs && ms < ze {
                        if let Some(e) = self.engine.as_mut() {
                            e.seek_ms((ms - zs) as f64);
                            self.frame_gate = f64::NEG_INFINITY;
                        }
                    } else {
                        self.drop_engine_source(cx);
                    }
                }
                EngineSource::Direct { .. } => {
                    let ds = direct_params_at(&src, ms);
                    match ds {
                        Some(media_t) => {
                            if let Some(e) = self.engine.as_mut() {
                                e.seek_ms(media_t);
                                self.frame_gate = f64::NEG_INFINITY;
                            }
                        }
                        None => self.drop_engine_source(cx),
                    }
                }
            }
        }
        if !self.engine_active() {
            let at = self.playhead_ms.min(self.duration_ms.saturating_sub(100));
            *self.shared.preview_request.lock().unwrap() = Some(at);
        }
        cx.notify();
    }

    /// 吸附开关切换(时间线工具行/设置页同源)。
    pub fn toggle_snap(&mut self, cx: &mut Context<Self>) {
        self.snap_enabled = !self.snap_enabled;
        self.status = format!("吸附 {}", if self.snap_enabled { "开" } else { "关" });
        cx.notify();
    }

    /// 播放/暂停(到尾自动停;从 0 重播)。
    /// 流畅画质(默认)走引擎(M2 zone 优先,M1 直解码),精确画质走幻灯片。
    pub fn toggle_play(&mut self, cx: &mut Context<Self>) {
        if self.playing {
            self.pause_playback(cx);
        } else {
            self.start_play(cx);
        }
        cx.notify();
    }

    /// 媒体插入:按素材类型路由到首个同类未锁轨(播放头处)。
    pub fn insert_media(&mut self, path: &str, cx: &mut Context<Self>) {
        let snap = self.shared.snapshot();
        let Some(entry) = snap.media.iter().find(|m| m.path == path) else {
            return;
        };
        let kinds: &[sable::video::model::TrackKind] = match entry.kind.as_str() {
            "audio" => &[sable::video::model::TrackKind::Audio],
            _ => &[sable::video::model::TrackKind::Video],
        };
        let Some(track_id) = snap.first_track_of(kinds) else {
            self.status = format!("没有可用的{}轨道(先加轨)", entry.kind);
            cx.notify();
            return;
        };
        let params = serde_json::json!({
            "trackId": track_id,
            "src": entry.path,
            "startMs": self.playhead_ms,
        });
        self.submit("clip_add", params, cx);
    }

    pub fn select_clip(&mut self, id: sable::video::model::ClipId, cx: &mut Context<Self>) {
        self.selection = self.id_map.get(&id.value()).cloned();
        self.status = format!("选中 {}", self.selection.as_deref().unwrap_or("?"));
        cx.notify();
    }

    /// 选中 clip 的投影 JSON(检查器展示用)。
    pub fn selected_clip(&self) -> Option<serde_json::Value> {
        let want = self.selection.as_deref()?;
        let snap = self.shared.snapshot();
        snap.clips
            .iter()
            .find(|c| c.get("id").and_then(|v| v.as_str()) == Some(want))
            .cloned()
    }
}

// ---------------------------------------------------------------------------
// I1 播放引擎接线(M1 直解码 / M2 zone / M3 UI;docs/tickets/I1-S2)
// ---------------------------------------------------------------------------

/// Direct 源内工程时刻 → 媒体内时刻(seek 换算;越界 None)。
fn direct_params_at(src: &EngineSource, t: u64) -> Option<f64> {
    match src {
        EngineSource::Direct {
            clip_start_ms,
            clip_end_ms,
            source_in,
            clip_speed,
            ..
        } => {
            if t < *clip_start_ms || t >= *clip_end_ms {
                return None;
            }
            Some(source_in + t.saturating_sub(*clip_start_ms) as f64 * clip_speed)
        }
        _ => None,
    }
}

/// 引擎媒体位置 → 工程播放头(ms;Direct 经片段速度折算,zone 1:1)。
fn project_time_of(src: &EngineSource, media_pos: f64) -> u64 {
    match src {
        EngineSource::Direct {
            clip_start_ms,
            source_in,
            clip_speed,
            ..
        } => {
            // pos 是媒体绝对域(引擎时钟锚在 in_ms),映射与载入点无关
            let dt = ((media_pos - source_in) / clip_speed).max(0.0);
            clip_start_ms + dt as u64
        }
        EngineSource::Zone { start_ms, .. } => start_ms + media_pos.max(0.0) as u64,
    }
}

/// 引擎流终点(媒体域;到尾由引擎自动 pause)。
fn source_end_ms(src: &EngineSource) -> f64 {
    match src {
        EngineSource::Direct { media_out, .. } => *media_out,
        EngineSource::Zone {
            start_ms, end_ms, ..
        } => end_ms.saturating_sub(*start_ms) as f64,
    }
}

/// preview_zone_render 响应 → ZoneRendered(宽容解析;file/media 任一)。
fn parse_zone_data(
    data: &serde_json::Value,
    req_start: u64,
    req_end: u64,
    rev: u64,
) -> Result<ZoneRendered, String> {
    let file = data
        .get("file")
        .and_then(serde_json::Value::as_str)
        .or_else(|| data.get("media").and_then(serde_json::Value::as_str))
        .ok_or_else(|| "响应缺 file".to_string())?
        .to_string();
    if file.is_empty() {
        return Err("响应 file 为空".into());
    }
    // startMs/endMs 为内核量化后实际区间(时间映射基准;缺失回落请求值)
    let start_ms = data
        .get("startMs")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(req_start);
    let end_ms = data
        .get("endMs")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(req_end);
    Ok(ZoneRendered {
        file,
        start_ms,
        end_ms,
        key: data
            .get("key")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        rev,
    })
}

/// 本机是否有音频输出设备(无声卡 = WASAPI 0 设备;引擎本就自动回落墙钟)。
fn has_audio_device() -> bool {
    use cpal::traits::HostTrait as _;
    cpal::default_host().default_output_device().is_some()
}

/// 变速挡展示(整数不带小数)。
pub fn fmt_speed(s: f32) -> String {
    if (s - s.round()).abs() < 1e-3 {
        format!("{}x", s.round() as i32)
    } else {
        format!("{s}x")
    }
}

impl DesktopApp {
    /// 引擎路径在场(帧由播放泵供给;预览面板据此跳过 render_frame)。
    pub fn engine_active(&self) -> bool {
        self.engine_source.is_some() || self.zone_loading.is_some()
    }

    /// 拖进度条进行中(预览面板 mouse-move/up 收尾判定用)。
    pub fn scrubbing(&self) -> bool {
        self.scrubbing.is_some()
    }

    /// 引擎有效速度(片段速度 × 变速挡)与有效静音(≠1x 自动静音:音频不跟速)。
    pub fn transport_params(&self) -> (f32, bool) {
        let spd = self.engine_speed_effective();
        (spd, self.user_muted || (spd - 1.0).abs() > 1e-3)
    }

    /// 引擎有效速度(检查器/设置页展示)。
    pub fn engine_speed_effective(&self) -> f32 {
        let clip_speed = match &self.engine_source {
            Some(EngineSource::Direct { clip_speed, .. }) => *clip_speed,
            _ => 1.0,
        };
        (clip_speed * self.transport_speed as f64) as f32
    }

    /// 播放泵(16ms):zone 结果搬运 / 引擎推进与帧搬运 / 段尾推进 /
    /// 播放中编辑重同步(300ms 防抖)。幻灯片推进仍在 100ms 泵。
    fn engine_pump(&mut self, cx: &mut Context<Self>) {
        let mut notify = false;
        // 1) zone 渲染结果(后台任务写;失败静默回落 M1 直解码)
        let zone_arrival = self
            .shared
            .zone_result
            .lock()
            .ok()
            .and_then(|mut g| g.take());
        if let Some(res) = zone_arrival {
            self.zone_loading = None;
            match res {
                Ok(z) => {
                    let file = self.rpc.absolutize(&z.file);
                    // 指纹 marker:渲染编排方(壳)职责——成功渲染后落
                    // `<key>.fresh`,复用判定走 playback::zone::is_fresh
                    let session = zone_cache::zone_key(&z.key, z.start_ms, z.end_ms);
                    if !z.key.is_empty() {
                        let dir =
                            zone_cache::preview_cache_dir(std::path::Path::new(&self.project_dir));
                        let _ = std::fs::create_dir_all(&dir);
                        let _ = std::fs::write(dir.join(format!("{}.fresh", z.key)), "");
                    }
                    play_log(&format!(
                        "zone 就绪 {session} [{}, {}) file={}",
                        z.start_ms, z.end_ms, z.file
                    ));
                    self.zone_ready = Some(ZoneReady {
                        file,
                        start_ms: z.start_ms,
                        end_ms: z.end_ms,
                        key: z.key,
                        rev: z.rev,
                    });
                    if self.playing && !self.quality_precise {
                        self.play_zone_ready(cx);
                    }
                }
                Err(e) => {
                    play_log(&format!("zone 失败,回落直解码:{e}"));
                    if self.playing && !self.quality_precise && self.engine_source.is_none() {
                        self.start_direct_play(cx);
                    }
                }
            }
            notify = true;
        }
        // 2) 引擎推进 + 帧搬运(故障 → 回落;段尾 → 推进下一源)
        let mut frame_out: Option<EngineFrame> = None;
        if self.playing
            && let Some(src) = self.engine_source.clone()
        {
            let mut fallback_reason: Option<String> = None;
            let mut ended = false;
            if let Some(engine) = self.engine.as_mut() {
                let pos = engine.position_ms();
                if engine.is_faulted() {
                    fallback_reason = Some(
                        engine
                            .last_error()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "未知故障".into()),
                    );
                } else {
                    self.playhead_ms = project_time_of(&src, pos).min(self.duration_ms);
                    if pos + 0.5 >= source_end_ms(&src) && !engine.is_playing() {
                        ended = true; // 引擎到尾自动 pause
                    }
                    // 帧步进按主时钟:背压下解码被消费速度拉平,消费节奏必须
                    // 与帧边界对账——用**累计帧序号**而非 pos 差值:16ms 泵 tick
                    // 走 16/32/48ms,差值门控会在每次弹帧后重置基准,实际
                    // 48ms/帧 = 20.8fps 天花板(实测 17fps);序号对账后每跨一
                    // 条帧边界消费一帧,30fps 内容即 30fps 消费(fps 防御 ≤0)
                    let fps = self.shared.snapshot().fps();
                    let frame_dur = (1000.0 / if fps > 0.0 { fps } else { 30.0 }).max(1.0);
                    let frame_idx = (pos / frame_dur).floor();
                    if frame_idx != self.frame_gate {
                        if let Some(f) = engine.poll_frame() {
                            self.frame_gate = frame_idx;
                            self.pop_miss = false;
                            let t = project_time_of(&src, f.pts_ms).min(self.duration_ms);
                            frame_out = Some(EngineFrame::new(t, f.rgba, f.width, f.height));
                        } else if !self.pop_miss {
                            // 门开而环空:解码供给断(eof/丢帧/停流),每窗记一次
                            self.pop_miss = true;
                            play_log(&format!(
                                "帧供给落空 pos={pos:.0} fault={} err={:?}",
                                engine.is_faulted(),
                                engine.last_error()
                            ));
                        }
                    }
                }
            }
            notify = true;
            if let Some(reason) = fallback_reason {
                self.fallback_to_slideshow(Some(reason), cx);
            } else if ended {
                self.advance_after_source_end(cx);
            }
        }
        // 3) 播放中编辑重同步:rev 变化 → 300ms 防抖后重解析,变了才重启流
        if self.playing && self.engine_source.is_some() {
            let rev = self.shared.snapshot_rev();
            if rev != self.engine_source_rev {
                if self.resync_at.is_none() {
                    self.resync_at = Some(Instant::now() + RESYNC_DEBOUNCE);
                } else if Instant::now() >= self.resync_at.expect("刚置的防抖时刻必有值")
                {
                    self.resync_at = None;
                    self.resync_engine(cx);
                    notify = true;
                }
            } else {
                self.resync_at = None;
            }
        }
        // 4) 帧上屏(shared 槽 → 预览面板泵 → RenderImage)
        // 注:此刻 DesktopApp 正在更新中,面板不可回读宿主(gpui 实体借规,
        // 回读即 panic——冒烟实测),状态由这里算好传入
        if let Some(f) = frame_out {
            self.stat_frames += 1;
            *self.shared.engine_frame.lock().unwrap() = Some(f);
            let zone_loading_now = self.zone_loading.is_some();
            if let Some(p) = self.preview.clone() {
                p.update(cx, |panel, cx| panel.pump_from_host(cx, zone_loading_now));
            }
            notify = true;
        }
        // 帧率日志(2s 窗口;验收要求播放路径有日志证据)
        if self.playing && self.engine_source.is_some() {
            let elapsed = self.stat_at.elapsed();
            if elapsed >= Duration::from_secs(2) {
                let fps = self.stat_frames as f64 / elapsed.as_secs_f64();
                play_log(&format!(
                    "帧率 {:.1} fps(窗口 {} 帧 / {:.1}s)播放头 {}ms",
                    fps,
                    self.stat_frames,
                    elapsed.as_secs_f64(),
                    self.playhead_ms
                ));
                self.stat_at = Instant::now();
                self.stat_frames = 0;
            }
        }
        if notify {
            cx.notify();
        }
    }

    /// 引擎段尾推进:zone 播完续直解码;直解码片段播完按循环/下一段续走;
    /// 到工程尾停机。下一段非视频(图片/文本/空窗)→ 幻灯片续走。
    fn advance_after_source_end(&mut self, cx: &mut Context<Self>) {
        let Some(src) = self.engine_source.clone() else {
            return;
        };
        let t = self.playhead_ms;
        match &src {
            EngineSource::Zone { file, .. } => {
                play_log(&format!(
                    "zone 段播完 t={t} file={} → 续直解码",
                    file.display()
                ));
                self.engine_source = None;
                if t >= self.duration_ms {
                    self.playing = false;
                    self.status = "播放结束".into();
                } else {
                    self.start_direct_play(cx);
                    return;
                }
            }
            EngineSource::Direct {
                clip_start_ms,
                media_in,
                ..
            } => {
                let (cs, mi) = (*clip_start_ms, *media_in);
                if self.loop_clip && t >= cs {
                    // A→B 循环:回片段头续播(引擎 seek = 重启流,约百 ms)
                    let params = self.transport_params();
                    if let Some(e) = self.engine.as_mut() {
                        e.seek_ms(mi);
                        let (spd, muted) = params;
                        e.set_speed(spd);
                        e.set_muted(muted);
                        e.play();
                        self.frame_gate = f64::NEG_INFINITY;
                    }
                    self.playhead_ms = cs;
                    self.status = "循环播放".into();
                    return;
                }
                self.engine_source = None;
                if t >= self.duration_ms {
                    self.playing = false;
                    self.status = "播放结束".into();
                    if let Some(e) = self.engine.as_mut() {
                        e.pause();
                    }
                } else {
                    self.start_direct_play(cx);
                    return;
                }
            }
        }
        cx.notify();
    }

    /// 载入已就绪 zone 并播放(rev/marker/文件失配 → 静默转直解码)。
    fn play_zone_ready(&mut self, cx: &mut Context<Self>) {
        let Some(z) = self.zone_ready.clone() else {
            return;
        };
        let rev = self.shared.snapshot_rev();
        // 新鲜度:rev 对账(提交即作废)+ 壳侧指纹 marker(渲染编排方职责)+
        // 文件存在(防 gc 半途);三者齐备才免重渲复用
        let marker_fresh = !z.key.is_empty()
            && zone_cache::is_fresh(
                &zone_cache::preview_cache_dir(std::path::Path::new(&self.project_dir)),
                &z.key,
            );
        let drifted = self.playhead_ms < z.start_ms || self.playhead_ms >= z.end_ms;
        if z.rev != rev || drifted || !marker_fresh || !z.file.exists() {
            play_log(&format!(
                "zone 作废(rev 失配={}, 播放头漂移={drifted}, marker 失鲜={mf}, 文件缺失={fe}),直解码",
                z.rev != rev,
                mf = !marker_fresh,
                fe = !z.file.exists()
            ));
            self.zone_ready = None;
            self.start_direct_play(cx);
            return;
        }
        let fps = self.shared.snapshot().fps();
        let in_ms = (self.playhead_ms - z.start_ms) as f64;
        let out_ms = (z.end_ms - z.start_ms) as f64;
        let params = self.transport_params();
        let loaded = self
            .engine
            .as_mut()
            .map(|e| e.load_clip(&z.file, in_ms, out_ms, fps));
        match loaded {
            Some(Ok(())) => {
                let (spd, muted) = params;
                let e = self.engine.as_mut().expect("engine 刚加载成功必在场");
                e.set_speed(spd);
                e.set_muted(muted);
                e.play();
                self.engine_source = Some(EngineSource::Zone {
                    file: z.file,
                    start_ms: z.start_ms,
                    end_ms: z.end_ms,
                });
                self.engine_source_rev = rev;
                self.engine_note = None;
                self.playing = true;
                self.frame_gate = f64::NEG_INFINITY;
                play_log(&format!(
                    "zone 起播 [{}, {}) in={in_ms} out={out_ms} fps={fps} 倍速={spd}",
                    z.start_ms, z.end_ms
                ));
                self.status = "播放中(zone 预览)".into();
            }
            Some(Err(e)) => {
                play_log(&format!("zone 加载失败:{e}"));
                self.engine_note = Some(format!("zone 加载失败:{e}"));
                self.start_direct_play(cx);
            }
            None => self.fallback_to_slideshow(Some("引擎未初始化".into()), cx),
        }
        cx.notify();
    }

    /// M1 直解码起播:播放头所在视频片段 → load_clip(绝对 src,媒体内偏移,fps)。
    fn start_direct_play(&mut self, cx: &mut Context<Self>) {
        let t = self.playhead_ms;
        let rev = self.shared.snapshot_rev();
        let fps = self.shared.snapshot().fps();
        let params = self.transport_params();
        let resolved = self.resolve_direct_source(t);
        match resolved {
            Some(ds) => {
                let DirectSource {
                    clip_id,
                    src,
                    clip_start_ms: cs,
                    clip_end_ms: ce,
                    source_in,
                    media_in,
                    media_out,
                    clip_speed,
                } = ds;
                play_log(&format!(
                    "直解码起播 clip={clip_id} src={} media=[{media_in:.0}, {media_out:.0}) 速度={clip_speed} 倍速={}",
                    src.display(),
                    params.0
                ));
                let loaded = self
                    .engine
                    .as_mut()
                    .map(|e| e.load_clip(&src, media_in, media_out, fps));
                match loaded {
                    Some(Ok(())) => {
                        let (spd, muted) = params;
                        let e = self.engine.as_mut().expect("engine 刚加载成功必在场");
                        e.set_speed(spd);
                        e.set_muted(muted);
                        e.play();
                        self.engine_source = Some(EngineSource::Direct {
                            clip_id,
                            src,
                            clip_start_ms: cs,
                            clip_end_ms: ce,
                            source_in,
                            media_in,
                            media_out,
                            clip_speed,
                        });
                        self.engine_source_rev = rev;
                        self.engine_note = None;
                        self.playing = true;
                        self.frame_gate = f64::NEG_INFINITY;
                        self.status = "播放中(直解码)".into();
                    }
                    Some(Err(e)) => self.fallback_to_slideshow(Some(format!("加载失败:{e}")), cx),
                    None => self.fallback_to_slideshow(Some("引擎未初始化".into()), cx),
                }
            }
            // 图片/文本片段/空窗本就无流 → 幻灯片路径,不提示错误
            None => self.fallback_to_slideshow(None, cx),
        }
        cx.notify();
    }

    /// 解析播放头所在视频片段为直解码参数:
    /// 媒体偏移 = sourceInMs + (播放头 − 片头) × 片段速度(含 trim 修正)。
    fn resolve_direct_source(&self, t: u64) -> Option<DirectSource> {
        let snap = self.shared.snapshot();
        let clip = snap.video_clip_at(t)?;
        let clip_id = clip
            .get("id")
            .and_then(serde_json::Value::as_str)?
            .to_string();
        let src_rel = clip.get("src").and_then(serde_json::Value::as_str)?;
        let cs = clip.get("startMs").and_then(serde_json::Value::as_u64)?;
        let ce = clip.get("endMs").and_then(serde_json::Value::as_u64)?;
        let src = self.rpc.absolutize(src_rel);
        let clip_speed = crate::state::clip_speed(&clip);
        let source_in = clip
            .get("sourceInMs")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as f64;
        let media_in = source_in + t.saturating_sub(cs) as f64 * clip_speed;
        let media_out = source_in + ce.saturating_sub(cs) as f64 * clip_speed;
        if media_out - media_in < 1.0 {
            return None;
        }
        Some(DirectSource {
            clip_id,
            src,
            clip_start_ms: cs,
            clip_end_ms: ce,
            source_in,
            media_in,
            media_out,
            clip_speed,
        })
    }

    /// 降级链终点:回落既有幻灯片模式(引擎不可用不白屏不 panic)。
    /// reason = None 是正常路径切换(图片/文本片段无流),不落错误横幅。
    fn fallback_to_slideshow(&mut self, reason: Option<String>, cx: &mut Context<Self>) {
        self.engine_source = None;
        self.zone_loading = None;
        self.resync_at = None;
        if let Some(r) = &reason {
            play_log(&format!("回落幻灯片:{r}"));
            self.engine_note = Some(r.clone());
            self.status = format!("已回落幻灯片模式:{r}");
        } else {
            play_log("回落幻灯片(非视频段,正常路径)");
            self.status = "播放中(单帧幻灯片模式)".into();
        }
        self.playing = true;
        self.last_frame = Instant::now();
        let at = self
            .playhead_ms
            .min(self.duration_ms.saturating_sub(100).max(self.playhead_ms));
        *self.shared.preview_request.lock().unwrap() = Some(at);
        cx.notify();
    }

    /// 弃引擎源(停流回 Idle;幻灯片静帧接管反映最新工程)。
    fn drop_engine_source(&mut self, cx: &mut Context<Self>) {
        self.engine_source = None;
        self.resync_at = None;
        if let Some(e) = self.engine.as_mut() {
            e.shutdown();
        }
        let at = self
            .playhead_ms
            .min(self.duration_ms.saturating_sub(100).max(self.playhead_ms));
        *self.shared.preview_request.lock().unwrap() = Some(at);
        cx.notify();
    }

    /// 引擎惰性构造(ffmpeg/ffprobe 路径:CUTFORGE_FFMPEG / CUTFORGE_FFPROBE,
    /// 缺省 "ffmpeg" / "ffprobe",与内核 bundle 口径一致)。
    fn ensure_engine(&mut self) -> Result<(), String> {
        if self.engine.is_none() {
            let ffmpeg = std::env::var("CUTFORGE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
            let ffprobe = std::env::var("CUTFORGE_FFPROBE").unwrap_or_else(|_| "ffprobe".into());
            let engine = PlaybackEngine::new(PathBuf::from(ffmpeg), PathBuf::from(ffprobe))
                .map_err(|e| e.to_string())?;
            self.engine = Some(engine);
        }
        Ok(())
    }

    fn pause_playback(&mut self, cx: &mut Context<Self>) {
        self.playing = false;
        if let Some(e) = self.engine.as_mut() {
            e.pause();
        }
        self.status = "已暂停".into();
        cx.notify();
    }

    /// 起播:精确画质 → 幻灯片;流畅 → zone 复用 → zone 请求 → 直解码。
    fn start_play(&mut self, cx: &mut Context<Self>) {
        if self.duration_ms == 0 {
            self.status = "空时间线,没有可播放内容".into();
            return;
        }
        if self.playhead_ms >= self.duration_ms {
            self.playhead_ms = 0;
        }
        if self.quality_precise {
            // 精确:引擎不参与,幻灯片逐帧
            self.engine_source = None;
            self.zone_loading = None;
            if let Some(e) = self.engine.as_mut() {
                e.shutdown();
            }
            self.playing = true;
            self.last_frame = Instant::now();
            self.status = "播放中(单帧精确模式)".into();
            return;
        }
        if let Err(e) = self.ensure_engine() {
            self.fallback_to_slideshow(Some(e), cx);
            return;
        }
        // zone 复用:rev/marker 未变且播放头仍在区间内 → 免重渲直载(原地重播)
        let rev = self.shared.snapshot_rev();
        let reuse = self.zone_ready.as_ref().is_some_and(|z| {
            z.rev == rev
                && self.playhead_ms >= z.start_ms
                && self.playhead_ms < z.end_ms
                && zone_cache::is_fresh(
                    &zone_cache::preview_cache_dir(std::path::Path::new(&self.project_dir)),
                    &z.key,
                )
                && z.file.exists()
        });
        if reuse {
            self.play_zone_ready(cx);
            return;
        }
        // M2:后台请求 zone,发起即角标,返回即切换
        if self.request_zone(cx) {
            self.playing = true; // 等渲染完成;引擎泵收结果后续播
            self.last_frame = Instant::now();
            self.status = "预览渲染中,完成后自动播放".into();
            return;
        }
        // 区间过短/无视频片段 → M1 直解码(再回落幻灯片)
        self.start_direct_play(cx);
    }

    /// 发起 preview_zone_render([播放头, +8s] 钳内容长度;100ms 量化在内核)。
    /// 同步工具无 runId —— 发起即角标,返回即切换。
    fn request_zone(&mut self, cx: &mut Context<Self>) -> bool {
        let snap = self.shared.snapshot();
        let has_video = snap.clips.iter().any(|c| {
            c.get("trackKind").and_then(serde_json::Value::as_str) == Some("video")
                && c.get("src")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|s| !s.is_empty())
        });
        if !has_video {
            return false;
        }
        let rev = self.shared.snapshot_rev();
        let start = (self.playhead_ms / 100 * 100).min(self.duration_ms.saturating_sub(100));
        let end =
            ((self.playhead_ms + ZONE_SPAN_MS).min(self.duration_ms) / 100 * 100).max(start + 100);
        if end <= start {
            return false;
        }
        self.zone_loading = Some((start, end, rev));
        play_log(&format!(
            "zone 请求 [{start}, {end}) rev={rev}(同步工具,无 runId;完成即切换)"
        ));
        let rpc = self.rpc.clone();
        let shared = self.shared.clone();
        cx.background_executor()
            .spawn(async move {
                let res = rpc
                    .call(
                        "preview_zone_render",
                        serde_json::json!({ "startMs": start, "endMs": end }),
                        crate::rpc::render_timeout("preview_zone_render"),
                    )
                    .and_then(|data| parse_zone_data(&data, start, end, rev));
                *shared.zone_result.lock().unwrap() = Some(res);
            })
            .detach();
        true
    }

    /// 播放中编辑重同步(300ms 防抖后调用):重解析当前源,变了才重启流。
    fn resync_engine(&mut self, cx: &mut Context<Self>) {
        let rev = self.shared.snapshot_rev();
        let Some(src) = self.engine_source.clone() else {
            return;
        };
        match src {
            EngineSource::Zone { .. } => {
                // zone 内容寻址随提交失效 → 切直解码续播(下轮播放重新请求 zone)
                self.zone_ready = None;
                self.start_direct_play(cx);
            }
            EngineSource::Direct { .. } => {
                let t = self.playhead_ms;
                let fresh = self.resolve_direct_source(t);
                let changed = match (&fresh, &self.engine_source) {
                    (Some(ds), Some(cur)) => !ds.matches_engine(cur),
                    (None, Some(EngineSource::Direct { .. })) => true,
                    _ => false,
                };
                if !changed {
                    self.engine_source_rev = rev;
                    return;
                }
                play_log(&format!("编辑重同步:片段参数变化,重启流 t={t}"));
                match fresh {
                    Some(_) => self.start_direct_play(cx),
                    None => self.fallback_to_slideshow(None, cx),
                }
            }
        }
    }

    // ---- M3:JKL / 循环 / 静音 / 画质 / 截图 / 沉浸 / 拖拽 scrub ----

    /// L:正向倍速循环 1→2→4→1(≠1x 自动静音,UI 显示静音图标)。
    pub fn transport_faster(&mut self, cx: &mut Context<Self>) {
        let cur = self.transport_speed;
        let idx = SPEED_LADDER_UP.iter().position(|s| (*s - cur).abs() < 1e-3);
        let next = match idx {
            Some(i) => SPEED_LADDER_UP[(i + 1) % SPEED_LADDER_UP.len()],
            None => 1.0,
        };
        self.set_transport_speed(next, cx);
    }

    /// Shift+L:反向倍速循环 1→0.5→0.25。
    pub fn transport_reverse(&mut self, cx: &mut Context<Self>) {
        let cur = self.transport_speed;
        let idx = SPEED_LADDER_DOWN
            .iter()
            .position(|s| (*s - cur).abs() < 1e-3);
        let next = match idx {
            Some(i) => SPEED_LADDER_DOWN[(i + 1) % SPEED_LADDER_DOWN.len()],
            None => 1.0,
        };
        self.set_transport_speed(next, cx);
    }

    /// J:减速方向(4→2→1→0.5→0.25,到 0.25 停)。
    pub fn transport_step_down(&mut self, cx: &mut Context<Self>) {
        let cur = self.transport_speed;
        let idx = SPEED_LADDER_ALL
            .iter()
            .position(|s| (*s - cur).abs() < 1e-3);
        let next = match idx {
            Some(i) if i + 1 < SPEED_LADDER_ALL.len() => SPEED_LADDER_ALL[i + 1],
            _ => 0.25,
        };
        self.set_transport_speed(next, cx);
    }

    /// K:暂停(空格才是播放/暂停切换)。
    pub fn pause_transport(&mut self, cx: &mut Context<Self>) {
        if self.playing {
            self.pause_playback(cx);
        } else {
            self.status = "已暂停".into();
            cx.notify();
        }
    }

    fn set_transport_speed(&mut self, speed: f32, cx: &mut Context<Self>) {
        self.transport_speed = speed;
        let (spd, muted) = self.transport_params();
        if let Some(e) = self.engine.as_mut() {
            e.set_speed(spd);
            e.set_muted(muted);
        }
        let note = if muted && (spd - 1.0).abs() > 1e-3 {
            "(静音)"
        } else {
            ""
        };
        self.status = format!("倍速 {}{}", fmt_speed(speed), note);
        cx.notify();
    }

    /// 静音开关(无声卡时 toast 提示;引擎对静音是丢弃式消费,不回跳)。
    pub fn toggle_mute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.user_muted = !self.user_muted;
        let muted = self.transport_params().1;
        if let Some(e) = self.engine.as_mut() {
            e.set_muted(muted);
        }
        if !has_audio_device() {
            window.push_notification(Notification::info("本机无音频设备,已按静音处理"), cx);
        }
        self.status = format!("静音 {}", if self.user_muted { "开" } else { "关" });
        cx.notify();
    }

    /// 当前片段 A→B 循环开关(引擎播到片段尾自动 seek+play)。
    pub fn toggle_loop(&mut self, cx: &mut Context<Self>) {
        self.loop_clip = !self.loop_clip;
        self.status = if self.loop_clip {
            "循环开:当前片段 A→B".into()
        } else {
            "循环关".into()
        };
        cx.notify();
    }

    /// 画质切换:精确 = 既有 render_frame 路径;流畅 = 引擎(默认)。
    pub fn toggle_quality(&mut self, cx: &mut Context<Self>) {
        self.quality_precise = !self.quality_precise;
        if self.quality_precise {
            if let Some(e) = self.engine.as_mut() {
                e.shutdown();
            }
            self.engine_source = None;
            self.zone_loading = None;
            self.status = "画质:精确(逐帧 render_frame)".into();
            if self.playing {
                self.last_frame = Instant::now();
                let at = self
                    .playhead_ms
                    .min(self.duration_ms.saturating_sub(100).max(self.playhead_ms));
                *self.shared.preview_request.lock().unwrap() = Some(at);
            }
        } else {
            self.status = "画质:流畅(引擎)".into();
            if self.playing {
                self.start_play(cx);
            }
        }
        cx.notify();
    }

    /// 沉浸预览(收起其他面板只留预览;Esc 退出;非系统级全屏)。
    pub fn toggle_immersive(&mut self, cx: &mut Context<Self>) {
        self.immersive = !self.immersive;
        cx.notify();
    }

    /// 截图:render_frame 当前播放头 → 复制到 <工程>/screenshots/<时间戳>.png。
    /// (watcher 会触发一次无害重拉,接受;toast 显示相对路径)
    pub fn screenshot(&mut self, cx: &mut Context<Self>) {
        // 钳在最后一帧之前(=duration 抽帧会落空,内核无帧可出,与预览同口径)
        let t = self.playhead_ms.min(self.duration_ms.saturating_sub(100));
        let rpc = self.rpc.clone();
        let shared = self.shared.clone();
        let root = self.project_dir.clone();
        self.status = "截图渲染中…".into();
        cx.background_executor()
            .spawn(async move {
                let res = screenshot_once(&rpc, &root, t);
                *shared.screenshot_result.lock().unwrap() = Some(res);
            })
            .detach();
        cx.notify();
    }

    /// 拖进度条:按下开始(引擎暂停记忆播放态;拖拽零动画直接映射)。
    pub fn begin_scrub(&mut self, cx: &mut Context<Self>) {
        if self.scrubbing.is_some() {
            return;
        }
        let was = self.playing;
        if was && let Some(e) = self.engine.as_mut() {
            e.pause();
        }
        self.playing = false;
        self.scrubbing = Some(was);
        cx.notify();
    }

    /// 拖动中:只更新播放头显示(不 seek 不出帧请求;松手才 seek)。
    pub fn scrub_to(&mut self, ms: u64, cx: &mut Context<Self>) {
        if self.scrubbing.is_none() {
            return;
        }
        self.playhead_ms = if self.duration_ms > 0 {
            ms.min(self.duration_ms)
        } else {
            ms
        };
        cx.notify();
    }

    /// 松手:提交 seek(引擎重启流),原播放态恢复。
    pub fn end_scrub(&mut self, ms: u64, cx: &mut Context<Self>) {
        let Some(was) = self.scrubbing.take() else {
            return;
        };
        self.set_playhead(ms, cx);
        if was {
            if self.engine_source.is_some() {
                let params = self.transport_params();
                if let Some(e) = self.engine.as_mut() {
                    let (spd, muted) = params;
                    e.set_speed(spd);
                    e.set_muted(muted);
                    e.play();
                }
                self.playing = true;
            } else {
                self.start_play(cx);
            }
        }
        cx.notify();
    }

    /// toast 冲刷(后台完成无 window;render 期统一推送)。
    fn flush_toasts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_toasts.is_empty() {
            return;
        }
        for msg in self.pending_toasts.drain(..) {
            window.push_notification(Notification::info(msg), cx);
        }
    }
}

/// 截图单发:render_frame → 复制到 <root>/screenshots/(后台线程)。
fn screenshot_once(rpc: &Rpc, root: &str, t_ms: u64) -> Result<String, String> {
    let data = rpc.call(
        "render_frame",
        serde_json::json!({ "atMs": t_ms }),
        crate::rpc::render_timeout("render_frame"),
    )?;
    let png = crate::panels::preview::find_png_path(&data).ok_or("响应中未找到 PNG 路径")?;
    let src = rpc.absolutize(&png);
    let dir = PathBuf::from(root).join("screenshots");
    std::fs::create_dir_all(&dir).map_err(|e| format!("建目录失败:{e}"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let name = format!("cutforge-shot-{stamp}.png");
    let dest = dir.join(&name);
    std::fs::copy(&src, &dest).map_err(|e| format!("复制失败:{e}"))?;
    Ok(format!("screenshots/{name}(播放头 {t_ms}ms)"))
}

impl Focusable for DesktopApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DesktopApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 后台完成的截图/静音提示经 window 推 toast(gpui-component 通知面)
        self.flush_toasts(window, cx);
        let colors = theme(cx).colors;
        let status = self.status.clone();
        let connected = self.shared.connected.load(Ordering::Relaxed);
        let rev = self.shared.rev.load(Ordering::Relaxed);
        let snap = self.shared.snapshot();
        let n_clips = snap.clips.len();

        let shortcut_hint = "空格 播放 · J/K/L 变速 · ←→ 步帧 · Shift+←→ 1s · S 分割 · Del 删除 · Ctrl+Z 撤销 · ± 缩放";
        // 沉浸预览:收起工具栏与 dock,只留预览(Esc 退出;按钮文案「沉浸」如实)
        let toolbar = if self.immersive {
            None
        } else {
            Some(self.toolbar(&colors, cx))
        };
        let body = if self.immersive {
            match self.preview.clone() {
                Some(p) => p.into_any_element(),
                None => div().into_any_element(),
            }
        } else {
            match self.dock.clone() {
                Some(dock) => dock.into_any_element(),
                None => div().into_any_element(),
            }
        };
        let project_name = self
            .project_dir
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&self.project_dir)
            .to_string();
        let statusbar = h_flex()
            .w_full()
            .h(px(26.0))
            .px(px(SpacingTokens::SM))
            .gap(px(SpacingTokens::SM))
            .items_center()
            .bg(colors.surface_1)
            .border_t_1()
            .border_color(colors.border_subtle)
            .child(div().w(px(8.0)).h(px(8.0)).rounded_full().bg(if connected {
                colors.success
            } else {
                colors.danger
            }))
            .child(
                div()
                    .text_size(px(FONT_SIZE_CAPTION))
                    .text_color(colors.text_secondary)
                    .child(if connected {
                        format!("已连接 · rev {rev} · {n_clips} clips")
                    } else {
                        "未连接".to_string()
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(FONT_SIZE_CAPTION))
                    .text_color(if connected {
                        colors.text_secondary
                    } else {
                        colors.danger
                    })
                    .child(status)
                    .truncate(),
            )
            .child(
                div()
                    .text_size(px(FONT_SIZE_CAPTION))
                    .text_color(colors.text_secondary)
                    .child(shortcut_hint),
            )
            .child(
                div()
                    .text_size(px(FONT_SIZE_CAPTION))
                    .text_color(colors.text_secondary)
                    .child(project_name),
            );

        div()
            .id("cutforge-desktop-root")
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.surface_0)
            .text_color(colors.text_primary)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .when(toolbar.is_some(), |s| {
                s.child(toolbar.expect("刚判定 Some"))
            })
            .child(div().flex_1().min_h_0().child(body))
            .child(statusbar)
    }
}

/// 键盘快捷键(文本输入聚焦时跳过,避免吞输入)。
impl DesktopApp {
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // 输入框聚焦(gpui-component InputState 持自己的焦点)时不劫持按键
        if let Some(focused) = window.focused(cx)
            && focused != self.focus
        {
            return; // 输入框等持有焦点的控件优先,不劫持全局快捷键
        }
        let key = ev.keystroke.key.as_str();
        let ctrl = ev.keystroke.modifiers.control;
        let shift = ev.keystroke.modifiers.shift;
        let fps = self.shared.snapshot().fps();
        let step = if shift {
            1000
        } else {
            (1000.0 / fps).round() as u64
        };
        match (key, ctrl, shift) {
            (" ", false, false) => self.toggle_play(cx),
            ("left", false, _) => self.set_playhead(self.playhead_ms.saturating_sub(step), cx),
            ("right", false, _) => self.set_playhead(self.playhead_ms.saturating_add(step), cx),
            // JKL 变速(L 正向快放 / Shift+L 慢放 / J 减速 / K 暂停)
            ("l", false, false) => self.transport_faster(cx),
            ("l", false, true) => self.transport_reverse(cx),
            ("j", false, false) => self.transport_step_down(cx),
            ("k", false, false) => self.pause_transport(cx),
            ("escape", false, false) => {
                if self.immersive {
                    self.immersive = false;
                    self.status = "已退出沉浸预览".to_string();
                    cx.notify();
                }
            }
            ("up", false, false) | ("down", false, false) => {
                // 上/下:相邻轨同名位置片段选择(简化:清选;多轨遍历候后)
                self.selection = None;
                self.status = "取消选中".to_string();
                cx.notify();
            }
            ("home", false, _) => self.set_playhead(0, cx),
            ("end", false, _) => self.set_playhead(self.duration_ms, cx),
            ("delete", false, _) | ("backspace", false, _) => self.delete_selected(cx),
            ("s", false, false) => self.split_selected(cx),
            ("t", false, false) => {
                // 全轨分割(剪映 Ctrl+B 同义;S=单片段,T=全轨)
                self.submit(
                    "clip_split_all",
                    serde_json::json!({ "tMs": self.playhead_ms }),
                    cx,
                );
            }
            ("d", false, false) => self.duplicate_selected(cx),
            ("c", true, false) => self.copy_selected(cx),
            ("x", true, false) => {
                self.copy_selected(cx);
                self.delete_selected(cx);
            }
            ("v", true, false) => self.paste_at_playhead(cx),
            ("g", false, false) => self.close_gap_at_playhead(cx),
            ("z", true, false) => self.submit("undo", serde_json::json!({}), cx),
            ("z", true, true) | ("y", true, _) => self.submit("redo", serde_json::json!({}), cx),
            ("=", false, _) | ("+", false, _) => self.zoom(1.3, cx),
            ("-", false, _) => self.zoom(1.0 / 1.3, cx),
            _ => {}
        }
    }

    /// 复制选中片段到壳侧剪贴板(内核 clip_copy 幂等,但壳只存 id 即可;
    /// 直接存 id:paste 用 clip_paste_at 需 trackId+startMs,不依赖内核剪贴板)。
    pub fn copy_selected(&mut self, cx: &mut Context<Self>) {
        match self.selection.clone() {
            Some(clip) => {
                self.clipboard = Some(clip);
                self.status = "已复制".to_string();
                cx.notify();
            }
            None => {
                self.status = "未选中片段".to_string();
                cx.notify();
            }
        }
    }

    /// 粘贴:剪贴板片段的源轨 + 播放头落点(clip_paste_at;同 kind 校验在内核)。
    pub fn paste_at_playhead(&mut self, cx: &mut Context<Self>) {
        let Some(clip_id) = self.clipboard.clone() else {
            self.status = "剪贴板为空".to_string();
            cx.notify();
            return;
        };
        // 目标轨:剪贴板片段的源轨(从快照按 id 反查;查不到回落首视频轨)
        let snap = self.shared.snapshot();
        let track_id = snap
            .clips
            .iter()
            .find(|c| c.get("id").and_then(serde_json::Value::as_str) == Some(clip_id.as_str()))
            .and_then(|c| c.get("track").and_then(serde_json::Value::as_str))
            .map(str::to_string)
            .unwrap_or_else(|| {
                snap.first_track_of(&[sable::video::model::TrackKind::Video])
                    .unwrap_or_else(|| "V1".to_string())
            });
        self.submit(
            "clip_paste_at",
            serde_json::json!({ "trackId": track_id, "startMs": self.playhead_ms }),
            cx,
        );
    }

    /// 关闭播放头所在空隙(需片段 id 定位轨道;取选中片段的轨,否则首视频轨)。
    pub fn close_gap_at_playhead(&mut self, cx: &mut Context<Self>) {
        let track_id = self
            .selected_clip()
            .map(|c| {
                c.get("track")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("V1")
                    .to_string()
            })
            .unwrap_or_else(|| {
                self.shared
                    .snapshot()
                    .first_track_of(&[sable::video::model::TrackKind::Video])
                    .unwrap_or_else(|| "V1".to_string())
            });
        self.submit(
            "clip_gap_delete",
            serde_json::json!({ "trackId": track_id, "tMs": self.playhead_ms }),
            cx,
        );
    }

    fn zoom(&mut self, factor: f64, cx: &mut Context<Self>) {
        let cur = self.timeline.read(cx).px_per_second;
        let next = (cur * factor).clamp(12.0, 600.0);
        self.timeline.update(cx, |tl, _| tl.px_per_second = next);
        self.status = format!("时间轴缩放 {next:.0} px/s");
        cx.notify();
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(clip) = self.selection.clone() {
            self.submit("clip_delete", serde_json::json!({ "clipId": clip }), cx);
        }
    }

    fn split_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(clip) = self.selection.clone() {
            self.submit(
                "clip_split",
                serde_json::json!({ "clipId": clip, "tMs": self.playhead_ms }),
                cx,
            );
        }
    }

    fn duplicate_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(clip) = self.selection.clone() {
            self.submit(
                "clip_duplicate",
                serde_json::json!({ "clipId": clip, "startMs": self.playhead_ms }),
                cx,
            );
        }
    }
}

/// 顶栏(剪映 IA:工具栏只放工程级操作——品牌 | 居中工程名 | rev+导出;
/// 编辑操作唯一入口 = 时间线工具行 + 快捷键,不再重复)。
impl DesktopApp {
    fn toolbar(&self, colors: &ColorTokens, cx: &mut Context<Self>) -> sable::gpui::AnyElement {
        let weak = cx.entity().downgrade();
        h_flex()
            .w_full()
            .h(px(40.0))
            .px(px(SpacingTokens::SM))
            .items_center()
            .bg(colors.surface_1)
            .border_b_1()
            .border_color(colors.border_subtle)
            .child(
                h_flex()
                    .w(px(220.0))
                    .flex_shrink_0()
                    .gap(px(SpacingTokens::XS))
                    .items_center()
                    .child(
                        div()
                            .px(px(SpacingTokens::XS + 2.0))
                            .text_size(px(FONT_SIZE_HEADING))
                            .text_color(colors.accent)
                            .child("CutForge"),
                    ),
            )
            // 中段:居中工程名(slug 优先,重投影刷新)
            .child(
                h_flex().flex_1().justify_center().overflow_hidden().child(
                    div()
                        .max_w(px(420.0))
                        .text_size(px(FONT_SIZE_HEADING))
                        .text_color(colors.text_primary)
                        .child(self.project_title.clone())
                        .truncate(),
                ),
            )
            // 右段:保存状态 + 导出主按钮
            .child(
                h_flex()
                    .w(px(220.0))
                    .flex_shrink_0()
                    .justify_end()
                    .gap(px(SpacingTokens::SM))
                    .items_center()
                    .child(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child(format!(
                                "已保存 rev {}",
                                self.shared.rev.load(Ordering::Relaxed)
                            )),
                    )
                    .child(
                        div()
                            .id(sable::gpui::ElementId::Name("tb-export".into()))
                            .px(px(SpacingTokens::SM + 4.0))
                            .py(px(4.0))
                            .rounded(px(5.0))
                            .bg(colors.accent)
                            .text_size(px(FONT_SIZE_CAPTION + 1.0))
                            .text_color(colors.surface_0)
                            .hover(|s| s.bg(colors.text_secondary))
                            .cursor_pointer()
                            .child("导出")
                            .on_click({
                                let weak = weak.clone();
                                move |_, _, cx: &mut App| {
                                    if let Some(app) = weak.upgrade() {
                                        app.update(cx, |app, cx| {
                                            app.submit("render", serde_json::json!({}), cx);
                                        });
                                    }
                                }
                            }),
                    ),
            )
            .into_any_element()
    }
}
