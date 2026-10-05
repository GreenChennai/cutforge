//! 壳状态:后台线程写入的快照(共享) + UI 侧投影(Sable Timeline 视图)。
//!
//! **壳纯度纪律**:本模块对内核 JSON 只做**形状搬运**(字段拷贝进 Sable
//! Timeline 的视图模型),不推导任何时间线语义——`in/out` 恒取投影原值,
//! 不做速度换算;时长合计由 Sable 侧 `duration_ms` 只作视图标尺,不回写。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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
    /// 内核看门狗健康(BUG-20:kernel.rs 看门狗置位,设置页/状态栏消费;
    /// Arc 由 main 从 Kernel 克隆种入,附着/失败时为常真缺省)
    pub kernel_health: Arc<AtomicBool>,
    /// 内核看门狗重启次数(同上)
    pub kernel_restarts: Arc<AtomicU64>,
    pub inner: Mutex<Snapshot>,
    /// 预览渲染结果槽(后台任务写,UI 泵取;Err 亦占位防卡死)
    pub preview_result: Mutex<Option<Result<PreviewFrame, String>>>,
    /// 期望预览的播放头(壳请求,渲染泵消费)
    pub preview_request: Mutex<Option<u64>>,
    /// 最近一次网络错误(状态栏展示)
    pub last_error: Mutex<Option<String>>,
    /// 后台重拉进行中(防重入;UI 泵守卫)
    pub reloading: AtomicBool,
    /// 引擎解码帧(播放泵写,预览面板消费;seq 去重,I1-M1)
    pub engine_frame: Mutex<Option<EngineFrame>>,
    /// preview_zone_render 结果槽(后台任务写,引擎泵消费;I1-M2)
    pub zone_result: Mutex<Option<Result<ZoneRendered, String>>>,
    /// 截图完成消息(后台任务写,UI 泵转 toast;I1-M3)
    pub screenshot_result: Mutex<Option<Result<String, String>>>,
}

/// 引擎解码出的一帧(工程时间域;rgba 为 RGBA8888 直通)。
#[derive(Clone)]
pub struct EngineFrame {
    /// 全局递增序号(显示去重;新实例 = 新 gpui 纹理)
    pub seq: u64,
    /// 工程时间(播放头;ms)
    pub t_ms: u64,
    pub rgba: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

impl EngineFrame {
    pub fn new(t_ms: u64, rgba: Arc<Vec<u8>>, width: u32, height: u32) -> Self {
        static NEXT_SEQ: AtomicU64 = AtomicU64::new(1);
        EngineFrame {
            seq: NEXT_SEQ.fetch_add(1, Ordering::Relaxed),
            t_ms,
            rgba,
            width,
            height,
        }
    }
}

/// preview_zone_render 结果(M2;file 为工程内相对路径,壳负责挂 root)。
#[derive(Clone)]
pub struct ZoneRendered {
    pub file: String,
    /// 内核量化后的实际区间(100ms 网格;时间映射基准)
    pub start_ms: u64,
    pub end_ms: u64,
    /// 内容寻址键(shell 据此写/查指纹 marker,playback::zone 新鲜度辅助)
    pub key: String,
    /// 发起请求时的快照 rev(rev 变化即作废)
    pub rev: u64,
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

    /// 播放头所在视频类片段(有 src 的非文本片段;I1 播放解析用)。
    /// 多轨同点重叠时取轨道清单靠后者(内核渲染叠加在上者;形状读取,
    /// 不做合成语义推导)。空窗/纯图片/文本片段 → None(走幻灯片路径)。
    pub fn video_clip_at(&self, t_ms: u64) -> Option<Value> {
        self.clips
            .iter()
            .rev()
            .find(|c| {
                let s = c.get("startMs").and_then(Value::as_u64).unwrap_or(0);
                let e = c.get("endMs").and_then(Value::as_u64).unwrap_or(0);
                t_ms >= s
                    && t_ms < e
                    && c.get("trackKind").and_then(Value::as_str) == Some("video")
                    && c.get("src")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty())
            })
            .cloned()
    }
}

/// 片段恒速(speed 字段;缺省 1.0)。
/// 已知局限:`speedCurve` 分段变速暂按 1.0 处理(引擎直解码无逐段映射),
/// 变速曲线片段的预览节奏以导出为准。
pub fn clip_speed(clip: &Value) -> f64 {
    let curved = clip
        .get("speedCurve")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty());
    if curved {
        return 1.0;
    }
    let s = clip.get("speed").and_then(Value::as_f64).unwrap_or(1.0);
    if s.is_finite() && s > 0.0 { s } else { 1.0 }
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

// ---------------------------------------------------------------------------
// 单测(A-07 / TC-DESK-PROJ-001):投影纯函数 golden 对拍
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn clip(id: &str, track: &str, start: u64, dur: u64) -> Value {
        // 内核投影形状:endMs(时长合计消费)与 durationMs 并存
        json!({ "id": id, "track": track, "startMs": start, "durationMs": dur, "endMs": start + dur })
    }

    // ---- track_metas ----

    #[test]
    fn track_metas_golden_shape() {
        let metas = track_metas(&json!({ "tracks": [
            {"id": "V1", "kind": "video", "name": "主轨", "mute": true, "locked": false},
            {"id": "A1", "kind": "audio", "name": "", "locked": true}
        ]}));
        assert_eq!(metas.len(), 2);
        assert_eq!(
            (metas[0].id.as_str(), metas[0].name.as_str()),
            ("V1", "主轨")
        );
        assert!(metas[0].mute && !metas[0].locked);
        assert!(matches!(metas[0].kind, TrackKind::Video));
        // 空名回落轨道 id
        assert_eq!((metas[1].id.as_str(), metas[1].name.as_str()), ("A1", "A1"));
        assert!(matches!(metas[1].kind, TrackKind::Audio));
        assert!(metas[1].locked);
    }

    #[test]
    fn track_metas_kind_mapping_and_missing_tracks() {
        let metas = track_metas(&json!({ "tracks": [
            {"id": "T1", "kind": "text"}, {"id": "V1", "kind": "weird"}
        ]}));
        assert!(matches!(metas[0].kind, TrackKind::Subtitle)); // text → Subtitle
        assert!(matches!(metas[1].kind, TrackKind::Video)); // 未知 → Video 兜底
        assert!(track_metas(&json!({})).is_empty());
        assert!(track_metas(&Value::Null).is_empty());
    }

    // ---- media_entries ----

    #[test]
    fn media_entries_filters_cache_and_hidden() {
        let entries = media_entries(&json!({ "files": [
            {"path": "a.mp4", "name": "a.mp4", "kind": "video", "bytes": 10},
            {"path": ".cutforge/cache.png", "name": "cache.png", "kind": "image"},
            {"path": ".hidden", "name": ".hidden", "kind": "video"},
            {"path": "", "name": "empty", "kind": "video"},
            {"path": "music/a.mp3", "name": "", "kind": "audio", "durationMs": 3000}
        ]}));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "a.mp4");
        assert_eq!(entries[0].bytes, 10);
        // 空名回落路径末段
        assert_eq!(entries[1].name, "a.mp3");
        assert_eq!(entries[1].duration_ms, Some(3000));
    }

    #[test]
    fn media_entries_absent_files_is_empty() {
        assert!(media_entries(&json!({})).is_empty());
        assert!(media_entries(&Value::Null).is_empty());
    }

    // ---- 片段/快照纯函数 ----

    #[test]
    fn clip_speed_default_curved_and_degenerate() {
        assert!((clip_speed(&json!({"speed": 2.0})) - 2.0).abs() < 1e-9);
        assert!((clip_speed(&json!({})) - 1.0).abs() < 1e-9);
        // speedCurve 分段变速:暂按 1.0(已知局限,以导出为准)
        assert!((clip_speed(&json!({"speedCurve": [{"t": 0}]})) - 1.0).abs() < 1e-9);
        assert!((clip_speed(&json!({"speed": 0})) - 1.0).abs() < 1e-9);
        assert!((clip_speed(&json!({"speed": -3})) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn snapshot_duration_is_max_end() {
        let snap = Snapshot {
            clips: vec![clip("a", "V1", 0, 500), clip("b", "V1", 700, 900)],
            ..Default::default()
        };
        assert_eq!(snap.duration_ms(), 1600);
        assert_eq!(Snapshot::default().duration_ms(), 0);
    }

    #[test]
    fn snapshot_fps_defaults_and_guards() {
        assert!((Snapshot::default().fps() - 30.0).abs() < 1e-9);
        let snap = Snapshot {
            project: json!({"fps": 25}),
            ..Default::default()
        };
        assert!((snap.fps() - 25.0).abs() < 1e-9);
        let bad = Snapshot {
            project: json!({"fps": 0}),
            ..Default::default()
        };
        assert!((bad.fps() - 30.0).abs() < 1e-9);
    }

    #[test]
    fn first_track_of_skips_locked() {
        let snap = Snapshot {
            tracks: vec![
                TrackMeta {
                    id: "V1".into(),
                    kind: TrackKind::Video,
                    name: "V1".into(),
                    mute: false,
                    locked: true,
                },
                TrackMeta {
                    id: "V2".into(),
                    kind: TrackKind::Video,
                    name: "V2".into(),
                    mute: false,
                    locked: false,
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            snap.first_track_of(&[TrackKind::Video]).as_deref(),
            Some("V2")
        );
        let none = Snapshot {
            tracks: vec![TrackMeta {
                id: "A1".into(),
                kind: TrackKind::Audio,
                name: "A1".into(),
                mute: false,
                locked: false,
            }],
            ..Default::default()
        };
        assert_eq!(none.first_track_of(&[TrackKind::Video]), None);
    }

    #[test]
    fn video_clip_at_requires_video_with_src_and_half_open() {
        let snap = Snapshot {
            clips: vec![
                json!({"id": "t", "track": "T1", "trackKind": "video", "src": "", "startMs": 0, "endMs": 100}),
                json!({"id": "img", "track": "V1", "trackKind": "video", "src": "p.png", "startMs": 0, "endMs": 100}),
                json!({"id": "v", "track": "V1", "trackKind": "video", "src": "a.mp4", "startMs": 0, "endMs": 100}),
                json!({"id": "v2", "track": "V2", "trackKind": "video", "src": "b.mp4", "startMs": 0, "endMs": 100}),
            ],
            ..Default::default()
        };
        // 同点重叠取轨道清单靠后者(上层叠加)
        assert_eq!(snap.video_clip_at(50).unwrap().get("id").unwrap(), "v2");
        assert!(snap.video_clip_at(100).is_none()); // 半开区间
        assert!(snap.video_clip_at(999).is_none());
    }

    // ---- project_timeline golden ----

    #[test]
    fn project_timeline_golden_two_tracks_three_clips() {
        let snap = Snapshot {
            tracks: vec![
                TrackMeta {
                    id: "V1".into(),
                    kind: TrackKind::Video,
                    name: "V1".into(),
                    mute: false,
                    locked: false,
                },
                TrackMeta {
                    id: "A1".into(),
                    kind: TrackKind::Audio,
                    name: "A1".into(),
                    mute: false,
                    locked: false,
                },
            ],
            clips: vec![
                clip("c3", "A1", 100, 400),
                clip("c1", "V1", 200, 300),
                clip("c2", "V1", 0, 100),
                clip("ghost", "VX", 0, 100), // 未知轨 → 跳过
            ],
            ..Default::default()
        };
        let (tl, id_map) = project_timeline(&snap);
        assert_eq!(tl.tracks.len(), 2);
        assert!(matches!(tl.tracks[0].kind, TrackKind::Video));
        assert!(matches!(tl.tracks[1].kind, TrackKind::Audio));
        // 未知轨片段不计入 id_map
        assert_eq!(id_map.len(), 3);
        // 同轨按 startMs 升序(视图不变量),id 映射完整
        let v1: Vec<u64> = tl.tracks[0].clips.iter().map(|c| c.start_ms).collect();
        assert_eq!(v1, vec![0, 200]);
        assert_eq!(tl.duration_ms, 500); // A1@100+400
    }

    #[test]
    fn project_timeline_display_name_from_src_basename_or_text() {
        let snap = Snapshot {
            tracks: vec![TrackMeta {
                id: "V1".into(),
                kind: TrackKind::Video,
                name: "V1".into(),
                mute: false,
                locked: false,
            }],
            clips: vec![
                json!({"id": "c1", "track": "V1", "startMs": 0, "durationMs": 10, "src": "dir/clip 名.mp4"}),
                json!({"id": "c2", "track": "V1", "startMs": 20, "durationMs": 10, "text": "这是一条很长的字幕文本应当被截断到十六字"}),
            ],
            ..Default::default()
        };
        let (tl, _) = project_timeline(&snap);
        assert_eq!(tl.tracks[0].clips[0].asset.path, "clip 名.mp4");
        assert_eq!(tl.tracks[0].clips[1].asset.path.chars().count(), 16);
    }
}
