//! 桌面壳根视图:工具栏(状态/undo/redo/播放头) + sable-dock 工作台
//! (左媒体库 / 中预览 / 右检查器 / 下时间轴) + 网络线程 + UI 泵。
//!
//! 数据流(与 Web 壳同构):手势 → `submit()`(/rpc)→ 内核 Op → 事件回流
//! (`GET /events` 长轮询线程置 dirty)→ UI 泵(400ms)重投影 → 视图刷新。
//! 壳不缓存真相:`Snapshot` 只是最近一次投影的只读副本。

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use sable::dock::{SablePanel, WorkspacePresets};
use sable::gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
    div, px,
};
use sable::gpui_component::dock::DockArea;
use sable::widgets::prelude::{SpacingTokens, h_flex};
use sable::widgets::theme::theme;
use sable::widgets::tokens::FONT_SIZE_CAPTION;

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
    pub status: String,
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
        let app = cx.new(|cx| {
            // UI 泵:400ms 对账 shared.rev/dirty → 重投影
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(400))
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
                status: format!("连接中…(工程 {root_dir})"),
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
                LibraryPanel::new(shared.clone(), rpc.clone(), cx).into(),
                cx,
            );
            let preview = SablePanel::create(
                "预览",
                PreviewPanel::new(shared.clone(), rpc.clone(), cx).into(),
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
                    Some(px(260.)),
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
                since = max_seq_of(&events).unwrap_or(since);
                shared.dirty.store(true, Ordering::Relaxed);
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
    let project = rpc.call(
        "project_get",
        serde_json::json!({}),
        Duration::from_secs(10),
    )?;
    let timeline = rpc.call(
        "timeline_get",
        serde_json::json!({}),
        Duration::from_secs(10),
    )?;
    let ui_fields = rpc
        .get("/ui-fields", Duration::from_secs(10))
        .unwrap_or(serde_json::Value::Null);
    let rev = timeline.get("rev").and_then(|v| v.as_u64()).unwrap_or(0);
    let clips = timeline
        .get("clips")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    shared.store_snapshot(crate::state::Snapshot {
        project,
        clips,
        ui_fields,
        media: Vec::new(),
        rev,
    });
    Ok(())
}

impl DesktopApp {
    /// UI 泵(400ms):错误横幅 / rev 对账重投影。
    fn pump(&mut self, cx: &mut Context<Self>) {
        if let Some(err) = self.shared.take_error() {
            self.status = format!("连接异常:{err}");
            cx.notify();
            return;
        }
        let rev = self.shared.rev.load(Ordering::Relaxed);
        let dirty = self.shared.dirty.swap(false, Ordering::Relaxed);
        if rev != self.seen_rev || dirty {
            self.reproject(cx);
        }
    }

    /// 重投影:快照 → Sable Timeline 视图(壳零语义,见 state.rs)。
    fn reproject(&mut self, cx: &mut Context<Self>) {
        let snap = self.shared.snapshot();
        self.seen_rev = snap.rev;
        let (tl, id_map) = crate::state::project_timeline(&snap);
        self.id_map = id_map;
        self.timeline.update(cx, |view, _| *view = tl);
        self.status = format!(
            "已加载 rev {}(clips {})· 深色主题跟 sable tokens",
            snap.rev,
            snap.clips.len()
        );
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
        self.playhead_ms = ms;
        *self.shared.preview_request.lock().unwrap() = Some(ms);
        cx.notify();
    }

    /// 选中 clip(时间轴回调;Sable ClipId → 内核字符串 id)。
    pub fn select_clip(&mut self, id: sable::video::model::ClipId, cx: &mut Context<Self>) {
        self.selection = self.id_map.get(&id.value()).cloned();
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
        let playhead = self.playhead_ms;

        let toolbar = h_flex()
            .w_full()
            .h(px(40.0))
            .px(px(SpacingTokens::MD))
            .gap(px(SpacingTokens::SM))
            .bg(colors.surface_1)
            .border_b_1()
            .border_color(colors.border_subtle)
            .child(
                div()
                    .text_size(px(FONT_SIZE_CAPTION))
                    .text_color(colors.text_secondary)
                    .child(status),
            )
            .child(self.toolbar_buttons(cx))
            .child(
                div()
                    .text_size(px(FONT_SIZE_CAPTION))
                    .text_color(colors.text_primary)
                    .child(format!("播放头 {:.2}s", playhead as f64 / 1000.0)),
            );

        div()
            .id("cutforge-desktop-root")
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.surface_0)
            .text_color(colors.text_primary)
            .child(toolbar)
            .child(div().flex_1().min_h_0().child(match self.dock.clone() {
                Some(dock) => dock.into_any_element(),
                None => div().into_any_element(),
            }))
    }
}

impl DesktopApp {
    fn toolbar_buttons(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let colors = theme(cx).colors;
        let weak = cx.entity().downgrade();
        let btn = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px(px(SpacingTokens::SM))
                .py(px(2.0))
                .rounded_sm()
                .bg(colors.surface_2)
                .text_size(px(FONT_SIZE_CAPTION))
                .text_color(colors.text_primary)
                .hover(|s| s.bg(colors.border_subtle))
                .cursor_pointer()
                .child(label)
        };
        h_flex()
            .gap(px(SpacingTokens::SM))
            .child(btn("tb-undo", "撤销").on_click(submit_handler(
                weak.clone(),
                "undo",
                serde_json::json!({}),
            )))
            .child(btn("tb-redo", "重做").on_click(submit_handler(
                weak.clone(),
                "redo",
                serde_json::json!({}),
            )))
            .child(btn("tb-back", "◀ 1s").on_click(seek_handler(weak.clone(), -1000)))
            .child(btn("tb-fwd", "1s ▶").on_click(seek_handler(weak.clone(), 1000)))
            .child(btn("tb-preview", "预览此帧").on_click(seek_handler(weak, 0)))
    }
}

/// 工具栏:提交内核工具(undo/redo;具名 fn 保 HRTB 推断)。
fn submit_handler(
    weak: sable::gpui::WeakEntity<DesktopApp>,
    tool: &'static str,
    params: serde_json::Value,
) -> impl Fn(&sable::gpui::ClickEvent, &mut Window, &mut App) + use<> {
    move |_, _, cx: &mut App| {
        if let Some(app) = weak.upgrade() {
            app.update(cx, |app, cx| app.submit(tool, params.clone(), cx));
        }
    }
}

/// 工具栏:播放头步进(delta=0 = 当前帧重渲)。
fn seek_handler(
    weak: sable::gpui::WeakEntity<DesktopApp>,
    delta: i64,
) -> impl Fn(&sable::gpui::ClickEvent, &mut Window, &mut App) + use<> {
    move |_, _, cx: &mut App| {
        if let Some(app) = weak.upgrade() {
            app.update(cx, |app, cx| {
                let next = if delta == 0 {
                    app.playhead_ms
                } else if delta > 0 {
                    app.playhead_ms.saturating_add(delta as u64)
                } else {
                    app.playhead_ms.saturating_sub((-delta) as u64)
                };
                app.set_playhead(next, cx);
            });
        }
    }
}
