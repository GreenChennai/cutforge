//! 桌面壳根视图:顶部工具栏(撤销/重做 · 分割/复制/删除 · 加轨)+ sable-dock
//! 工作台(左媒体库 / 中预览 / 右检查器 / 下时间轴)+ 底部状态栏 +
//! 网络线程 + UI 泵 + 键盘快捷键。
//!
//! 数据流(与 Web 壳同构):手势 → `submit()`(/rpc)→ 内核 Op → 事件回流
//! (`GET /events` 长轮询线程置 dirty)→ UI 泵(100ms)重投影 → 视图刷新。
//! 壳不缓存真相:`Snapshot` 只是最近一次投影的只读副本。
//!
//! 播放 = 壳侧推进播放头(工程 fps 步进)+ 逐帧 `render_frame`(稳态
//! ~0.17s/帧,内核有帧缓存;这是幻灯片式预览,精确播放走 render 导出)。

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use sable::dock::{SablePanel, WorkspacePresets};
use sable::gpui::prelude::FluentBuilder as _;
use sable::gpui::{
    App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use sable::gpui_component::dock::DockArea;
use sable::widgets::prelude::{SpacingTokens, h_flex};
use sable::widgets::theme::theme;
use sable::widgets::tokens::{ColorTokens, FONT_SIZE_CAPTION, FONT_SIZE_HEADING};

use crate::panels::inspector::InspectorPanel;
use crate::panels::library::LibraryPanel;
use crate::panels::preview::PreviewPanel;
use crate::panels::timeline::TimelineHost;
use crate::rpc::Rpc;
use crate::state::Shared;

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
            // UI 泵:100ms 对账 shared.rev/dirty → 重投影;播放推进
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
            Self {
                rpc,
                shared,
                seen_rev: 0,
                timeline,
                id_map: Default::default(),
                selection: None,
                playhead_ms: 0,
                playing: false,
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
            let preview = SablePanel::create(
                "预览",
                PreviewPanel::new(shared.clone(), rpc.clone(), &this, cx).into(),
                cx,
            );
            let inspector = SablePanel::create("检查器", InspectorPanel::new(&this, cx).into(), cx);
            let timeline_host = TimelineHost::new(&this, app.timeline.clone(), cx);
            let timeline_panel = SablePanel::create("时间轴", timeline_host.clone().into(), cx);

            let dock = WorkspacePresets::build_workspace(
                "cutforge-desktop",
                vec![library],
                preview,
                vec![inspector],
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
        // 播放推进:按真实流逝时间步进(幻灯片式:出帧速度跟上为准)
        if self.playing {
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
        self.status = format!("就绪 · rev {}", snap.rev);
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
        let at = self.playhead_ms.min(self.duration_ms.saturating_sub(100));
        *self.shared.preview_request.lock().unwrap() = Some(at);
        cx.notify();
    }

    /// 播放/暂停(到尾自动停;从 0 重播)。
    pub fn toggle_play(&mut self, cx: &mut Context<Self>) {
        if self.playing {
            self.playing = false;
            self.status = "已暂停".to_string();
        } else {
            if self.duration_ms > 0 && self.playhead_ms >= self.duration_ms {
                self.playhead_ms = 0;
            }
            self.playing = true;
            self.last_frame = Instant::now();
            self.status = "播放中(单帧幻灯片模式)".to_string();
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

impl Focusable for DesktopApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DesktopApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme(cx).colors;
        let status = self.status.clone();
        let connected = self.shared.connected.load(Ordering::Relaxed);
        let rev = self.shared.rev.load(Ordering::Relaxed);
        let snap = self.shared.snapshot();
        let n_clips = snap.clips.len();

        let toolbar = self.toolbar(&colors, cx);
        let shortcut_hint =
            "空格 播放 · ←→ 步帧 · Shift+←→ 1s · S 分割 · Del 删除 · Ctrl+Z 撤销 · ± 缩放";
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
            .child(toolbar)
            .child(div().flex_1().min_h_0().child(match self.dock.clone() {
                Some(dock) => dock.into_any_element(),
                None => div().into_any_element(),
            }))
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
            ("home", false, _) => self.set_playhead(0, cx),
            ("end", false, _) => self.set_playhead(self.duration_ms, cx),
            ("delete", false, _) | ("backspace", false, _) => self.delete_selected(cx),
            ("s", false, false) => self.split_selected(cx),
            ("d", false, false) => self.duplicate_selected(cx),
            ("z", true, false) => self.submit("undo", serde_json::json!({}), cx),
            ("z", true, true) | ("y", true, _) => self.submit("redo", serde_json::json!({}), cx),
            ("=", false, _) | ("+", false, _) => self.zoom(1.3, cx),
            ("-", false, _) => self.zoom(1.0 / 1.3, cx),
            _ => {}
        }
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

/// 工具栏:品牌 + 编辑组 + 片段组 + 轨道组(右留白给窗口拖拽区)。
impl DesktopApp {
    fn toolbar(&self, colors: &ColorTokens, cx: &mut Context<Self>) -> sable::gpui::AnyElement {
        let has_sel = self.selection.is_some();
        let weak = cx.entity().downgrade();
        let tb = |id: &'static str, label: &'static str, enabled: bool| {
            toolbar_button(id, label, enabled, colors)
        };
        h_flex()
            .w_full()
            .h(px(44.0))
            .px(px(SpacingTokens::SM))
            .gap(px(SpacingTokens::XS))
            .items_center()
            .bg(colors.surface_1)
            .border_b_1()
            .border_color(colors.border_subtle)
            .child(
                div()
                    .px(px(SpacingTokens::XS + 2.0))
                    .text_size(px(FONT_SIZE_HEADING))
                    .text_color(colors.accent)
                    .child("CutForge"),
            )
            .child(sep(colors))
            .child(tb("tb-undo", "↶ 撤销", true).on_click(submit_handler(
                weak.clone(),
                "undo",
                serde_json::json!({}),
            )))
            .child(tb("tb-redo", "↷ 重做", true).on_click(submit_handler(
                weak.clone(),
                "redo",
                serde_json::json!({}),
            )))
            .child(sep(colors))
            .child(
                tb("tb-split", "✂ 分割", has_sel).on_click(act_handler(
                    weak.clone(),
                    |app| {
                        (
                            "clip_split",
                            serde_json::json!({ "clipId": app.selection.clone(), "tMs": app.playhead_ms }),
                        )
                    },
                )),
            )
            .child(
                tb("tb-dup", "⧉ 副本", has_sel).on_click(act_handler(
                    weak.clone(),
                    |app| {
                        (
                            "clip_duplicate",
                            serde_json::json!({ "clipId": app.selection.clone(), "startMs": app.playhead_ms }),
                        )
                    },
                )),
            )
            .child(
                tb("tb-del", "✕ 删除", has_sel).on_click(act_handler(
                    weak.clone(),
                    |app| {
                        (
                            "clip_delete",
                            serde_json::json!({ "clipId": app.selection.clone() }),
                        )
                    },
                )),
            )
            .child(sep(colors))
            .child(
                tb("tb-add-v", "+ 视频轨", true).on_click(add_track_handler(
                    weak.clone(),
                    "video",
                )),
            )
            .child(
                tb("tb-add-a", "+ 音频轨", true).on_click(add_track_handler(
                    weak.clone(),
                    "audio",
                )),
            )
            .child(
                tb("tb-add-t", "+ 字幕轨", true).on_click(add_track_handler(
                    weak.clone(),
                    "text",
                )),
            )
            // 中段:居中工程名(剪映 H1;slug 缺省回落目录名)
            .child(
                h_flex()
                    .flex_1()
                    .justify_center()
                    .overflow_hidden()
                    .child(
                        div()
                            .max_w(px(420.0))
                            .text_size(px(FONT_SIZE_HEADING))
                            .text_color(colors.text_primary)
                            .child(self.project_title.clone())
                            .truncate(),
                    ),
            )
            // 右段:保存状态 + 导出主按钮(剪映 H2/H3)
            .child(
                h_flex()
                    .flex_1()
                    .justify_end()
                    .gap(px(SpacingTokens::SM))
                    .items_center()
                    .child(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child(format!("已保存 rev {}", self.shared.rev.load(Ordering::Relaxed))),
                    )
                    .child(
                        div()
                            .id(sable::gpui::ElementId::Name("tb-export".into()))
                            .px(px(SpacingTokens::SM + 4.0))
                            .py(px(5.0))
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

/// 分隔线。
fn sep(colors: &ColorTokens) -> impl IntoElement + use<> {
    div()
        .w(px(1.0))
        .h(px(18.0))
        .mx(px(SpacingTokens::XS))
        .bg(colors.border_subtle)
}

/// 工具栏按钮基座(enabled=false 置灰;danger = 删除类 hover 红)。
fn toolbar_button(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    colors: &ColorTokens,
) -> sable::gpui::Stateful<sable::gpui::Div> {
    div()
        .id(sable::gpui::ElementId::Name(id.into()))
        .px(px(SpacingTokens::SM + 2.0))
        .py(px(5.0))
        .rounded(px(5.0))
        .text_size(px(FONT_SIZE_CAPTION + 1.0))
        .when(enabled, |s| {
            s.bg(colors.surface_2)
                .text_color(colors.text_primary)
                .hover(|s| s.bg(colors.border_subtle))
                .cursor_pointer()
        })
        .when(!enabled, |s| {
            s.bg(colors.surface_1).text_color(colors.text_secondary)
        })
        .child(label)
}

/// 工具栏:静态工具提交(undo/redo)。
fn submit_handler(
    weak: sable::gpui::WeakEntity<DesktopApp>,
    tool: &'static str,
    params: serde_json::Value,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + use<> {
    move |_, _, cx: &mut App| {
        if let Some(app) = weak.upgrade() {
            app.update(cx, |app, cx| app.submit(tool, params.clone(), cx));
        }
    }
}

/// 工具栏点击处理器(Box 化:泛型闭包藏不进 `use<>` 返回位)。
type ClickHandler = std::boxed::Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// 工具栏:按当前应用状态现算工具+参数(选中类操作;泛型闭包藏不进
/// `use<>`,落 Box<dyn Fn>)。
fn act_handler(
    weak: sable::gpui::WeakEntity<DesktopApp>,
    f: impl Fn(&DesktopApp) -> (&'static str, serde_json::Value) + 'static,
) -> ClickHandler {
    std::boxed::Box::new(move |_, _, cx: &mut App| {
        if let Some(app) = weak.upgrade() {
            app.update(cx, |app, cx| {
                let (tool, params) = f(app);
                app.submit(tool, params, cx);
            });
        }
    })
}

/// 工具栏:加轨。
fn add_track_handler(
    weak: sable::gpui::WeakEntity<DesktopApp>,
    kind: &'static str,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + use<> {
    move |_, _, cx: &mut App| {
        if let Some(app) = weak.upgrade() {
            app.update(cx, |app, cx| {
                app.submit("track_add", serde_json::json!({ "kind": kind }), cx);
            });
        }
    }
}
