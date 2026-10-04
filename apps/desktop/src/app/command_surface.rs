//! 命令面(A-02 + BUG-22):键位命令注册表与 MCP 命令拼装,不认识 GPUI 组件
//! (只经事件参数与 `Context` 通知,不构造任何视图元素)。
//!
//! 单一真相(BUG-22):命令 id / 标题 / 快捷键 / 入口上下文全部收口在
//! `keymap::REGISTRY` 一张表——`on_key` 查表分发(替换原巨型内联 match),
//! 设置页速查表由 `keymap::shortcut_rows` 生成(消灭双维护);
//! 表结构与冲突检测见子模块 `keymap`。
//!
//! 上下键 = 跨轨片段遍历(对齐 Web `nav.selectSibling` 语义:轨序 + startMs
//! 稳定序 ±1 夹取),替换原「占位清选」。
//!
//! 命令拼装(close_gap/paste 的轨道反查等既有手测路径)抽为纯函数,可脱离
//! 桌面壳单测(A-07 / TC-DESK-CMD-002)。

use sable::gpui::{Context, KeyDownEvent, Window};
use sable::video::model::TrackKind;

use super::DesktopApp;
use crate::state::Snapshot;

mod keymap;

use self::keymap::resolve;
pub(crate) use self::keymap::shortcut_rows;

// ---------------------------------------------------------------------------
// 命令拼装纯函数(TC-DESK-CMD-002:既有手测路径,脱离桌面壳可测)
// ---------------------------------------------------------------------------

/// 帧步进毫秒数(Shift = 1 秒;fps 由快照供给,缺省 30)。
pub(crate) fn step_ms(fps: f64, shift: bool) -> u64 {
    if shift {
        1000
    } else {
        (1000.0 / fps).round() as u64
    }
}

/// 时间轴缩放目标值(钳 12..600 px/s,与原 zoom 内联一致)。
pub(crate) fn zoom_next(cur: f64, factor: f64) -> f64 {
    (cur * factor).clamp(12.0, 600.0)
}

/// 粘贴目标轨反查:剪贴板片段的源轨(快照按 id 反查);查不到回落首视频轨,
/// 再查不到落 "V1"(与 Web `serverPaste` 同语义;同 kind 校验在内核)。
pub(crate) fn paste_track_id(snap: &Snapshot, clip_id: &str) -> String {
    snap.clips
        .iter()
        .find(|c| c.get("id").and_then(serde_json::Value::as_str) == Some(clip_id))
        .and_then(|c| c.get("track").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| {
            snap.first_track_of(&[TrackKind::Video])
                .unwrap_or_else(|| "V1".to_string())
        })
}

/// 空隙关闭轨道反查:取选中片段的轨,否则首视频轨,再回落 "V1"。
pub(crate) fn gap_track_id(selected: Option<&str>, snap: &Snapshot) -> String {
    selected
        .and_then(|id| {
            snap.clips
                .iter()
                .find(|c| c.get("id").and_then(serde_json::Value::as_str) == Some(id))
                .and_then(|c| c.get("track").and_then(serde_json::Value::as_str))
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            snap.first_track_of(&[TrackKind::Video])
                .unwrap_or_else(|| "V1".to_string())
        })
}

/// 媒体插入轨型路由(按素材类型;非音频一律视频轨,与原 insert_media 一致)。
pub(crate) fn insert_kinds(kind: &str) -> &'static [TrackKind] {
    match kind {
        "audio" => &[TrackKind::Audio],
        _ => &[TrackKind::Video],
    }
}

/// 跨轨遍历(上/下键;对齐 Web `nav.selectSibling` 语义):
/// 全片段按「轨序 + startMs」稳定排序后 ±1 夹取;无选中时向下取首、向上取尾;
/// 空时间线 None。
pub(crate) fn select_sibling(snap: &Snapshot, current: Option<&str>, dir: i64) -> Option<String> {
    fn json_id(c: &serde_json::Value) -> Option<&str> {
        c.get("id").and_then(serde_json::Value::as_str)
    }
    let order: Vec<&str> = snap.tracks.iter().map(|t| t.id.as_str()).collect();
    let mut list: Vec<&serde_json::Value> = snap.clips.iter().collect();
    list.sort_by_key(|c| {
        // 排序键:(轨序, startMs)——Web clipsOrdered 同口径
        let track = c
            .get("track")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let rank = order
            .iter()
            .position(|id| *id == track)
            .unwrap_or(usize::MAX);
        (
            rank,
            c.get("startMs")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
    });
    if list.is_empty() {
        return None;
    }
    let idx = current.and_then(|id| list.iter().position(|c| json_id(c) == Some(id)));
    let next = match idx {
        Some(i) => (i as i64 + dir).clamp(0, list.len() as i64 - 1) as usize,
        None => {
            if dir > 0 {
                0
            } else {
                list.len() - 1
            }
        }
    };
    json_id(list[next]).map(str::to_string)
}

// ---------------------------------------------------------------------------
// 键盘分发(查表)与命令执行
// ---------------------------------------------------------------------------

impl DesktopApp {
    /// 键盘快捷键(文本输入聚焦时跳过,避免吞输入);原内联 match 改查表。
    pub(crate) fn on_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 输入框聚焦(gpui-component InputState 持自己的焦点)时不劫持按键
        if let Some(focused) = window.focused(cx)
            && focused != self.focus
        {
            return; // 输入框等持有焦点的控件优先,不劫持全局快捷键
        }
        let key = ev.keystroke.key.as_str();
        let ctrl = ev.keystroke.modifiers.control;
        let shift = ev.keystroke.modifiers.shift;
        if let Some(b) = resolve(key, ctrl, shift) {
            self.run_command(b.id, cx);
        }
    }

    /// 注册表 id → 执行(重绑定只改组合,不改语义;对齐 Web RUN 表思路)。
    pub(crate) fn run_command(&mut self, id: &str, cx: &mut Context<Self>) {
        match id {
            "play.toggle" => self.toggle_play(cx),
            "play.k" => self.pause_transport(cx),
            "play.l" => self.transport_faster(cx),
            "play.slow" => self.transport_reverse(cx),
            "play.j" => self.transport_step_down(cx),
            "play.frameBack" => self.step_playhead(-1, cx),
            "play.frameFwd" => self.step_playhead(1, cx),
            "play.secBack" => self.set_playhead(self.playhead_ms.saturating_sub(1000), cx),
            "play.secFwd" => self.set_playhead(self.playhead_ms.saturating_add(1000), cx),
            "play.home" => self.set_playhead(0, cx),
            "play.end" => self.set_playhead(self.duration_ms, cx),
            "clip.siblingPrev" => self.step_sibling(-1, cx),
            "clip.siblingNext" => self.step_sibling(1, cx),
            "clip.split" => self.split_selected(cx),
            // 全轨分割(剪映 Ctrl+B 同义;S=单片段,T=全轨)
            "clip.splitAll" => self.submit(
                "clip_split_all",
                serde_json::json!({ "tMs": self.playhead_ms }),
                cx,
            ),
            "clip.duplicate" => self.duplicate_selected(cx),
            "edit.copy" => self.copy_selected(cx),
            "edit.cut" => {
                self.copy_selected(cx);
                self.delete_selected(cx);
            }
            "edit.paste" => self.paste_at_playhead(cx),
            "edit.closeGap" => self.close_gap_at_playhead(cx),
            "edit.delete" | "edit.deleteBksp" => self.delete_selected(cx),
            "edit.undo" => self.submit("undo", serde_json::json!({}), cx),
            "edit.redo" => self.submit("redo", serde_json::json!({}), cx),
            "view.immersiveExit" => {
                if self.immersive {
                    self.immersive = false;
                    self.status = "已退出沉浸预览".to_string();
                    cx.notify();
                }
            }
            "view.zoomIn" => self.zoom(1.3, cx),
            "view.zoomOut" => self.zoom(1.0 / 1.3, cx),
            _ => {}
        }
    }

    /// 帧步进(按快照帧率;Shift 档由注册表拆分为秒级行)。
    fn step_playhead(&mut self, dir: i64, cx: &mut Context<Self>) {
        let fps = self.shared.snapshot().fps();
        let step = step_ms(fps, false);
        if dir < 0 {
            self.set_playhead(self.playhead_ms.saturating_sub(step), cx);
        } else {
            self.set_playhead(self.playhead_ms.saturating_add(step), cx);
        }
    }

    /// 上/下键:跨轨遍历选中相邻片段(Web nav.selectSibling 同语义)。
    pub(crate) fn step_sibling(&mut self, dir: i64, cx: &mut Context<Self>) {
        let snap = self.shared.snapshot();
        match select_sibling(&snap, self.selection.as_deref(), dir) {
            Some(id) => {
                self.selection = Some(id);
                self.status = format!("选中 {}", self.selection.as_deref().unwrap_or("?"));
            }
            None => {
                self.status = "时间线没有片段".to_string();
            }
        }
        cx.notify();
    }

    /// 顶栏导出按钮(唯一入口;工作台不重复)。
    pub fn request_export(&mut self, cx: &mut Context<Self>) {
        self.submit("render", serde_json::json!({}), cx);
    }

    /// 媒体插入:按素材类型路由到首个同类未锁轨(播放头处)。
    pub fn insert_media(&mut self, path: &str, cx: &mut Context<Self>) {
        let snap = self.shared.snapshot();
        let Some(entry) = snap.media.iter().find(|m| m.path == path) else {
            return;
        };
        let Some(track_id) = snap.first_track_of(insert_kinds(&entry.kind)) else {
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
            .find(|c| c.get("id").and_then(serde_json::Value::as_str) == Some(want))
            .cloned()
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
        // 目标轨:剪贴板片段的源轨(纯函数反查;查不到回落首视频轨)
        let snap = self.shared.snapshot();
        let track_id = paste_track_id(&snap, &clip_id);
        self.submit(
            "clip_paste_at",
            serde_json::json!({ "trackId": track_id, "startMs": self.playhead_ms }),
            cx,
        );
    }

    /// 关闭播放头所在空隙(需片段 id 定位轨道;取选中片段的轨,否则首视频轨)。
    pub fn close_gap_at_playhead(&mut self, cx: &mut Context<Self>) {
        let snap = self.shared.snapshot();
        let track_id = gap_track_id(self.selection.as_deref(), &snap);
        self.submit(
            "clip_gap_delete",
            serde_json::json!({ "trackId": track_id, "tMs": self.playhead_ms }),
            cx,
        );
    }

    fn zoom(&mut self, factor: f64, cx: &mut Context<Self>) {
        let cur = self.timeline.read(cx).px_per_second;
        let next = zoom_next(cur, factor);
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

// ---------------------------------------------------------------------------
// 单测(A-07 / TC-DESK-KEY / TC-DESK-CMD)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snap_with(tracks: serde_json::Value, clips: serde_json::Value) -> Snapshot {
        Snapshot {
            tracks: crate::state::track_metas(&json!({ "tracks": tracks })),
            clips: clips.as_array().cloned().unwrap_or_default(),
            ..Default::default()
        }
    }

    // ---- 命令拼装纯函数(TC-DESK-CMD-002)----

    #[test]
    fn paste_track_id_prefers_source_track() {
        let snap = snap_with(
            json!([{ "id": "V1", "kind": "video" }, { "id": "V2", "kind": "video" }]),
            json!([{ "id": "c1", "track": "V2" }]),
        );
        assert_eq!(paste_track_id(&snap, "c1"), "V2");
    }

    #[test]
    fn paste_track_id_falls_back_to_first_video_then_v1() {
        let empty = snap_with(json!([]), json!([]));
        assert_eq!(paste_track_id(&empty, "gone"), "V1");
        let no_tracks = snap_with(json!([{ "id": "A1", "kind": "audio" }]), json!([]));
        assert_eq!(paste_track_id(&no_tracks, "gone"), "V1");
        let with_v = snap_with(json!([{ "id": "V7", "kind": "video" }]), json!([]));
        assert_eq!(paste_track_id(&with_v, "gone"), "V7");
    }

    #[test]
    fn gap_track_id_uses_selection_then_first_video() {
        let snap = snap_with(
            json!([{ "id": "V1", "kind": "video" }, { "id": "V2", "kind": "video" }]),
            json!([{ "id": "c1", "track": "V2" }]),
        );
        assert_eq!(gap_track_id(Some("c1"), &snap), "V2");
        assert_eq!(gap_track_id(Some("missing"), &snap), "V1");
        assert_eq!(gap_track_id(None, &snap), "V1");
    }

    #[test]
    fn insert_kinds_routes_audio_only() {
        assert_eq!(insert_kinds("audio"), &[TrackKind::Audio]);
        assert_eq!(insert_kinds("video"), &[TrackKind::Video]);
        assert_eq!(insert_kinds("image"), &[TrackKind::Video]);
        assert_eq!(insert_kinds("whatever"), &[TrackKind::Video]);
    }

    #[test]
    fn step_ms_frame_and_second() {
        assert_eq!(step_ms(30.0, false), 33);
        assert_eq!(step_ms(25.0, false), 40);
        assert_eq!(step_ms(30.0, true), 1000);
    }

    #[test]
    fn zoom_next_clamps_to_gate() {
        assert!((zoom_next(60.0, 1.3) - 78.0).abs() < 1e-9);
        assert!((zoom_next(500.0, 1.3) - 600.0).abs() < 1e-9); // 上钳
        assert!((zoom_next(13.0, 1.0 / 1.3) - 12.0).abs() < 1e-9); // 下钳
    }

    // ---- 跨轨遍历(上/下键新语义)----

    #[test]
    fn select_sibling_orders_by_track_then_start() {
        let snap = snap_with(
            json!([
                { "id": "V1", "kind": "video" },
                { "id": "V2", "kind": "video" },
                { "id": "A1", "kind": "audio" }
            ]),
            json!([
                { "id": "c-v2", "track": "V2", "startMs": 500 },
                { "id": "c-v1b", "track": "V1", "startMs": 100 },
                { "id": "c-a1", "track": "A1", "startMs": 50 },
                { "id": "c-v1a", "track": "V1", "startMs": 0 }
            ]),
        );
        // 稳定序:V1@0 → V1@100 → V2@500 → A1@50(跨轨)
        assert_eq!(select_sibling(&snap, None, 1).as_deref(), Some("c-v1a"));
        assert_eq!(
            select_sibling(&snap, Some("c-v1a"), 1).as_deref(),
            Some("c-v1b")
        );
        assert_eq!(
            select_sibling(&snap, Some("c-v1b"), 1).as_deref(),
            Some("c-v2")
        );
        assert_eq!(
            select_sibling(&snap, Some("c-v2"), 1).as_deref(),
            Some("c-a1")
        );
        // 尾部夹取
        assert_eq!(
            select_sibling(&snap, Some("c-a1"), 1).as_deref(),
            Some("c-a1")
        );
        // 反向逐级回走 + 头部夹取
        assert_eq!(
            select_sibling(&snap, Some("c-a1"), -1).as_deref(),
            Some("c-v2")
        );
        assert_eq!(
            select_sibling(&snap, Some("c-v1a"), -1).as_deref(),
            Some("c-v1a")
        );
        // 无选中向上 = 最后一个
        assert_eq!(select_sibling(&snap, None, -1).as_deref(), Some("c-a1"));
    }

    #[test]
    fn select_sibling_empty_timeline_is_none() {
        let snap = snap_with(json!([]), json!([]));
        assert_eq!(select_sibling(&snap, None, 1), None);
    }

    #[test]
    fn select_sibling_unknown_track_sorts_last() {
        let snap = snap_with(
            json!([{ "id": "V1", "kind": "video" }]),
            json!([
                { "id": "c-ghost", "track": "GX", "startMs": 0 },
                { "id": "c-v1", "track": "V1", "startMs": 0 }
            ]),
        );
        assert_eq!(
            select_sibling(&snap, Some("c-v1"), 1).as_deref(),
            Some("c-ghost")
        );
    }
}
