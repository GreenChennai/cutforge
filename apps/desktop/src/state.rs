//! 壳状态:后台线程写入的快照(共享) + UI 侧投影(Sable Timeline 视图)。
//!
//! **壳纯度纪律**:本模块对内核 JSON 只做**形状搬运**(字段拷贝进 Sable
//! Timeline 的视图模型),不推导任何时间线语义——`in/out` 恒取投影原值,
//! 不做速度换算;时长合计由 Sable 侧 `duration_ms` 只作视图标尺,不回写。

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use sable::video::model::{AssetRef, Timeline, TrackKind};
use serde_json::Value;

/// 后台网络线程 ↔ UI 泵的共享面。
#[derive(Default)]
pub struct Shared {
    /// 内核投影 rev(事件回流/提交成功时更新;0 = 未加载)
    pub rev: AtomicU64,
    /// 需要重投影(事件 rev 变化 / 本地提交成功)
    pub dirty: AtomicBool,
    /// 连接状态(健康探测/任意成功请求置真)
    pub connected: AtomicBool,
    pub inner: Mutex<Snapshot>,
    /// 预览帧:<(tMs, PNG 路径 或 RGBA 字节)> 由渲染任务写入
    pub preview: Mutex<Option<PreviewFrame>>,
    /// 期望预览的播放头(壳请求,渲染任务消费后清零)
    pub preview_request: Mutex<Option<u64>>,
    /// 最近一次网络错误(状态栏展示)
    pub last_error: Mutex<Option<String>>,
}

#[derive(Clone)]
pub struct PreviewFrame {
    pub t_ms: u64,
    /// 解码后的 RGBA8(非预乘;由后台线程从 PNG 解出)
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// 一次完整投影的快照(后台线程整帧替换,UI 读时克隆)。
#[derive(Clone, Default)]
pub struct Snapshot {
    pub project: Value,
    pub clips: Vec<Value>,
    pub ui_fields: Value,
    pub media: Vec<Value>,
    pub rev: u64,
}

impl Shared {
    pub fn set_error(&self, msg: String) {
        *self.last_error.lock().unwrap() = Some(msg);
        self.connected.store(false, Ordering::Relaxed);
    }

    pub fn take_error(&self) -> Option<String> {
        self.last_error.lock().unwrap().take()
    }

    pub fn snapshot(&self) -> Snapshot {
        self.inner.lock().unwrap().clone()
    }

    pub fn store_snapshot(&self, snap: Snapshot) {
        let rev = snap.rev;
        *self.inner.lock().unwrap() = snap;
        self.rev.store(rev, Ordering::Relaxed);
        self.connected.store(true, Ordering::Relaxed);
        self.dirty.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// 投影:内核 JSON → Sable Timeline(视图模型;形状搬运,零语义)
// ---------------------------------------------------------------------------

fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn u64_of(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn track_kind(kind: &str) -> TrackKind {
    match kind {
        "audio" => TrackKind::Audio,
        "text" => TrackKind::Subtitle,
        _ => TrackKind::Video,
    }
}

/// 快照 → (Sable Timeline 视图, clip id 映射表)。
///
/// 轨道清单来自 `project.tracks`(id/kind),片段来自 `timeline_get.clips`
/// (服务端算好的投影:endMs 等;壳零推导)。Sable 侧排序仅为满足视图
/// 不变量(start_ms 升序),内核本身保证不重叠。
pub fn project_timeline(snap: &Snapshot) -> (Timeline, HashMap<u64, String>) {
    let mut tl = Timeline::new();
    tl.px_per_second = 60.0;
    let mut id_map = HashMap::new();

    let track_kinds: Vec<(String, TrackKind)> = snap
        .project
        .get("tracks")
        .and_then(Value::as_array)
        .map(|tracks| {
            tracks
                .iter()
                .map(|t| (str_of(t, "id"), track_kind(&str_of(t, "kind"))))
                .collect()
        })
        .unwrap_or_default();
    for (_, kind) in &track_kinds {
        tl.add_track(*kind);
    }

    let mut rows: Vec<&Value> = snap.clips.iter().collect();
    rows.sort_by_key(|c| u64_of(c, "startMs"));
    for clip in rows {
        let clip_id_str = str_of(clip, "id");
        let track_id = str_of(clip, "track");
        let Some(track_idx) = track_kinds.iter().position(|(id, _)| *id == track_id) else {
            continue;
        };
        let start_ms = u64_of(clip, "startMs");
        let duration_ms = u64_of(clip, "durationMs");
        let asset = AssetRef::new(str_of(clip, "src"), 0);
        // place_clip 铸造视图侧 id(ClipId 外部不可构造);行已按 startMs
        // 升序、内核保证同轨不重叠 → 插入即满足视图不变量
        if let Ok(id) = tl.place_clip(track_idx, asset, start_ms, duration_ms) {
            id_map.insert(id.value(), clip_id_str);
        }
    }
    for track in &mut tl.tracks {
        track.clips.sort_by_key(|c| c.start_ms);
    }
    tl.duration_ms = tl
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .map(|c| c.end_ms())
        .max()
        .unwrap_or(0);
    (tl, id_map)
}
