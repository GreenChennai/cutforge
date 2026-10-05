//! 工作台视图(A-02 拆分):面板装配 + 工程事件订阅(SyncHub 长轮询消费)+
//! UI 对账泵 + 根渲染。**不认识命令协议**——本模块不写任何 MCP 工具名,
//! 用户动作一律回调 `DesktopApp` 的命令面方法(工具栏导出 → `request_export`)。
//!
//! 数据流:网络线程 `GET /events` 长轮询置 dirty/rev → 100ms 泵对账重投影
//! (dirty = 后台重拉全量;snapshot_rev 变 = 本地重投影)→ `Render` 刷新。

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use sable::dock::{SablePanel, WorkspacePresets};
use sable::gpui::prelude::FluentBuilder as _;
use sable::gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use sable::gpui_component::WindowExt as _;
use sable::gpui_component::notification::Notification;
use sable::widgets::prelude::{SpacingTokens, h_flex};
use sable::widgets::theme::theme;
use sable::widgets::tokens::{ColorTokens, FONT_SIZE_CAPTION, FONT_SIZE_HEADING};

use crate::panels::inspector::InspectorPanel;
use crate::panels::library::LibraryPanel;
use crate::panels::preview::PreviewPanel;
use crate::panels::timeline::TimelineHost;
use crate::rpc::Rpc;
use crate::state::Shared;

use super::DesktopApp;

// ---------------------------------------------------------------------------
// 工程事件订阅(网络线程:初载全量 + /events 长轮询)
// ---------------------------------------------------------------------------

/// 网络线程:初载全量 → 事件长轮询循环(任何成功响应置 connected)。
pub(crate) fn net_loop(rpc: Arc<Rpc>, shared: Arc<Shared>) {
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

// ---------------------------------------------------------------------------
// 面板装配(sable-dock 工作台;两阶段组装的第二阶段)
// ---------------------------------------------------------------------------

/// 工作台装配:左媒体库 / 中预览 / 右检查器+设置 / 底部时间轴 dock。
pub(crate) fn assemble_dock(
    app: &mut DesktopApp,
    window: &mut Window,
    cx: &mut Context<DesktopApp>,
) {
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
}

// ---------------------------------------------------------------------------
// UI 对账泵(100ms)
// ---------------------------------------------------------------------------

impl DesktopApp {
    /// UI 泵(100ms):错误横幅 / 播放推进 / rev 对账重投影。
    pub(crate) fn pump(&mut self, cx: &mut Context<Self>) {
        // R-17:壳态 10s 防抖落盘(播放头/选择/吸附;小文件,损坏由 load 丢弃)
        if self.shell_last_save.elapsed() >= std::time::Duration::from_secs(10) {
            self.shell_last_save = std::time::Instant::now();
            crate::app::shell_state::save(
                Path::new(&self.project_dir),
                &crate::app::shell_state::ShellState {
                    playhead_ms: self.playhead_ms,
                    selection: self.selection.clone().into_iter().collect(),
                    snap_on: self.snap_enabled,
                },
            );
        }
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
            self.last_frame = std::time::Instant::now();
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
        if let Some(zr) = self.zone_ready.as_ref()
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

// ---------------------------------------------------------------------------
// 根渲染(顶栏 + 工作台 + 状态栏)
// ---------------------------------------------------------------------------

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
                                move |_, _, cx: &mut sable::gpui::App| {
                                    if let Some(app) = weak.upgrade() {
                                        app.update(cx, |app, cx| {
                                            app.request_export(cx);
                                        });
                                    }
                                }
                            }),
                    ),
            )
            .into_any_element()
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

// ---------------------------------------------------------------------------
// 单测(A-07):事件响应解析(表驱动)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn max_seq_of_object_with_events_array() {
        let v = json!({"seq": 9, "events": [{"seq": 3}, {"seq": 7}]});
        assert_eq!(max_seq_of(&v), Some(7)); // 事件内最大者优先
    }

    #[test]
    fn max_seq_of_bare_array() {
        let v = json!([{"seq": 4}, {"seq": 12}]);
        assert_eq!(max_seq_of(&v), Some(12));
    }

    #[test]
    fn max_seq_of_top_seq_only() {
        assert_eq!(max_seq_of(&json!({"seq": 41})), Some(41));
    }

    #[test]
    fn max_seq_of_empty_and_absent() {
        assert_eq!(max_seq_of(&json!({})), None);
        assert_eq!(max_seq_of(&json!({"events": []})), None);
        assert_eq!(max_seq_of(&json!({"seq": 5, "events": []})), Some(5)); // 空数组回落顶层
    }

    #[test]
    fn max_seq_of_ignores_non_u64_seq() {
        assert_eq!(
            max_seq_of(&json!({"seq": "x", "events": [{"seq": 2}]})),
            Some(2)
        );
        assert_eq!(max_seq_of(&json!({"seq": "x"})), None);
    }
}
