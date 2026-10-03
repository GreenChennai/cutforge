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
    /// 预览渲染结果槽(后台任务写,UI 泵取;Err 亦占位防卡死)
    pub preview_result: Mutex<Option<Result<PreviewFrame, String>>>,
    /// 期望预览的播放头(壳请求,渲染泵消费)
    pub preview_request: Mutex<Option<u64>>,
    /// 最近一次网络错误(状态栏展示)
    pub last_error: Mutex<Option<String>>,
    /// 后台重拉进行中(防重入;UI 泵守卫)
    pub reloading: AtomicBool,
}

#[derive(Clone)]
pub struct PreviewFrame {
    pub t_ms: u64,
    /// 解码后的 RGBA8(非预乘;由后台线程从 PNG 解出)
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// 轨道元数据(轨道头展示/插入路由用;来自 project.tracks 形状搬运)。
#[derive(Clone)]
pub struct TrackMeta {
    pub id: String,
    pub kind: TrackKind,
    pub name: String,
    pub mute: bool,
    pub locked: bool,
}

/// 媒体库条目(media_browse `files` 形状搬运;.cutforge 缓存已滤除)。
#[derive(Clone)]
pub struct MediaEntry {
    pub name: String,
    /// 工程内相对路径(clip_add src / media_thumbnail src 直通)
    pub path: String,
    /// 内核给的 kind:video/audio/image
    pub kind: String,
    /// 文件大小(展示用;当前面板未展示,保留字段)
    #[allow(dead_code)]
    pub bytes: u64,
    pub duration_ms: Option<u64>,
}

/// 一次完整投影的快照(后台线程整帧替换,UI 读时克隆)。
#[derive(Clone, Default)]
pub struct Snapshot {
    pub project: Value,
    pub tracks: Vec<TrackMeta>,
    pub clips: Vec<Value>,
    pub ui_fields: Value,
    /// 转场/动效/花字目录(GET /catalogs;缺省 Null)
    pub catalogs: Value,
    pub media: Vec<MediaEntry>,
    pub rev: u64,
}

impl Snapshot {
    /// 工程时长(clips endMs 最大值;ms)。
    pub fn duration_ms(&self) -> u64 {
        self.clips
            .iter()
            .filter_map(|c| c.get("endMs").and_then(Value::as_u64))
            .max()
            .unwrap_or(0)
    }

    /// 工程帧率(缺省 30)。
    pub fn fps(&self) -> f64 {
        let fps = self
            .project
            .get("fps")
            .and_then(Value::as_f64)
            .unwrap_or(30.0);
        if fps <= 0.0 { 30.0 } else { fps }
    }

    /// 第一条匹配轨道类型的轨道 id(媒体插入路由;文本素材回落文本轨)。
    pub fn first_track_of(&self, kinds: &[TrackKind]) -> Option<String> {
        self.tracks
            .iter()
            .find(|t| kinds.contains(&t.kind) && !t.locked)
            .map(|t| t.id.clone())
    }
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

    /// 快照数据 rev(轻量读,不克隆;重投影对账用)。
    pub fn snapshot_rev(&self) -> u64 {
        self.inner.lock().unwrap().rev
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

/// project.tracks → 轨道元数据(名称缺省回落轨道 id)。
pub fn track_metas(project: &Value) -> Vec<TrackMeta> {
    project
        .get("tracks")
        .and_then(Value::as_array)
        .map(|tracks| {
            tracks
                .iter()
                .map(|t| {
                    let id = str_of(t, "id");
                    let name = {
                        let n = str_of(t, "name");
                        if n.is_empty() { id.clone() } else { n }
                    };
                    TrackMeta {
                        kind: track_kind(&str_of(t, "kind")),
                        name,
                        mute: t.get("mute").and_then(Value::as_bool).unwrap_or(false),
                        locked: t.get("locked").and_then(Value::as_bool).unwrap_or(false),
                        id,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// media_browse 响应 → 媒体条目;滤除 `.cutforge` 缓存与隐藏文件。
pub fn media_entries(data: &Value) -> Vec<MediaEntry> {
    data.get("files")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .filter_map(|f| {
                    let path = str_of(f, "path");
                    if path.is_empty() || path.starts_with(".cutforge") || path.starts_with('.') {
                        return None;
                    }
                    let name = {
                        let n = str_of(f, "name");
                        if n.is_empty() {
                            path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string()
                        } else {
                            n
                        }
                    };
                    Some(MediaEntry {
                        name,
                        kind: str_of(f, "kind"),
                        bytes: u64_of(f, "bytes"),
                        duration_ms: f.get("durationMs").and_then(Value::as_u64),
                        path,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
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

    for track in &snap.tracks {
        tl.add_track(track.kind);
    }
    let track_kinds: Vec<(String, TrackKind)> =
        snap.tracks.iter().map(|t| (t.id.clone(), t.kind)).collect();

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
        // 显示名:视频/音频 = 素材文件名;文本片段 = 字幕文本(去空,截 16 字)
        // ——asset.path 在视图模型里只作展示名消费(TimelineView clip_label),
        // 不回写、零语义。
        let display = {
            let text = str_of(clip, "text");
            if text.is_empty() {
                let src = str_of(clip, "src");
                src.rsplit(['/', '\\']).next().unwrap_or(&src).to_string()
            } else {
                text.chars().take(16).collect::<String>()
            }
        };
        let asset = AssetRef::new(display, 0);
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
