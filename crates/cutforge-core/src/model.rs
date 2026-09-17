//! 领域模型(计划书 1.2/1.3):Project/Timeline/Track/Clip 与 schema v2 逐字段对齐。
//! 时间统一毫秒(`*Ms`);剪映微秒只存在于适配层,不得进入本层。

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn one() -> u8 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    Video,
    Audio,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Voice,
    Sfx,
    Music,
    Ambient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Ffmpeg,
    Jianying,
    Cutforge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ratio {
    #[serde(rename = "9x16")]
    NineBySixteen,
    #[serde(rename = "3x4")]
    ThreeByFour,
    #[serde(rename = "16x9")]
    SixteenByNine,
}

/// 封闭枚举(videoType):`talking-head` / `talking-head+animation` / `pure-animation`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipKind {
    #[serde(rename = "talking-head")]
    TalkingHead,
    #[serde(rename = "talking-head+animation")]
    TalkingHeadAnimation,
    #[serde(rename = "pure-animation")]
    PureAnimation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bgm {
    pub src: String,
    #[serde(default = "default_gain")]
    pub gain_db: f64,
    #[serde(default = "yes")]
    pub ducking: bool,
    #[serde(default = "yes", rename = "loop")]
    pub loop_: bool,
}

fn default_gain() -> f64 {
    -18.0
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    pub ms: u64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Subtitle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ass: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

/// 片段(轨道上占据时间区间的元素)。稳定 `id`(如 V1-001)是锚点与 Op target 的引用基础,
/// 禁止依赖数组下标(计划书 1.1 原则 4)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
    pub start_ms: u64,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_in_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reframe: Option<Reframe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay: Option<Overlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade: Option<Fade>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "loop")]
    pub loop_: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub punch_in: Option<PunchIn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeze_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reframe {
    pub anchor_y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Motion {
    #[serde(rename = "in", default, skip_serializing_if = "Option::is_none")]
    pub in_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dur_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Overlay {
    pub x: u64,
    pub y: u64,
    pub w: u64,
    pub h: u64,
    #[serde(default = "one_f", skip_serializing_if = "is_one_f")]
    pub opacity: f64,
}

fn one_f() -> f64 {
    1.0
}
fn is_one_f(v: &f64) -> bool {
    *v == 1.0
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fade {
    #[serde(default)]
    pub in_ms: f64,
    #[serde(default)]
    pub out_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PunchIn {
    #[serde(default = "default_factor")]
    pub factor: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

fn default_factor() -> f64 {
    1.4
}

/// 轨道:同类型元素的容器;`id`(如 V1)首次生成后写回并不再变。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub kind: TrackKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

/// 工程(Project):IR 的序列化形式即 05_ir/project.json;`version` 恒为 1 兼容旧读法,
/// 契约演进由 `schemaVersion` 表达(计划书 3.3 第 5 项)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    #[serde(default = "one")]
    pub version: u8,
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub slug: String,
    pub fps: u32,
    pub canvas: Canvas,
    #[serde(default = "default_backends")]
    pub backends: Vec<Backend>,
    #[serde(default = "default_notes_path")]
    pub notes: String,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bgm: Option<Bgm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<Ratio>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markers: Option<Vec<Marker>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<Subtitle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_crossfade_ms: Option<f64>,
}

fn default_schema_version() -> String {
    "2.0.0".into()
}
fn default_backends() -> Vec<Backend> {
    vec![Backend::Ffmpeg]
}
fn default_notes_path() -> String {
    "notes.json".into()
}

impl Project {
    /// 反序列化 + schema v2 校验(契约优先:没有 schema 支撑的字段不存在)。
    pub fn from_value(v: &Value) -> Result<Self, Vec<String>> {
        let p: Project = serde_json::from_value(v.clone())
            .map_err(|e| vec![format!("反序列化失败: {e}")])?;
        let errors = cutforge_schema::validate("project", v);
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(p)
    }

    /// 序列化后必须仍然通过 schema(命令通道在每次变更后强制走这一步)。
    pub fn to_validated_value(&self) -> Result<Value, Vec<String>> {
        let v = serde_json::to_value(self).map_err(|e| vec![format!("序列化失败: {e}")])?;
        let errors = cutforge_schema::validate("project", &v);
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(v)
    }

    /// 按 id 找片段,返回 (轨道下标, 片段下标)。
    pub fn find_clip(&self, clip_id: &str) -> Option<(usize, usize)> {
        for (ti, t) in self.tracks.iter().enumerate() {
            for (ci, c) in t.clips.iter().enumerate() {
                if c.id == clip_id {
                    return Some((ti, ci));
                }
            }
        }
        None
    }

    pub fn find_track(&self, track_id: &str) -> Option<usize> {
        self.tracks.iter().position(|t| t.id == track_id)
    }

    /// 片段在时间轴上的终点(ms)。
    pub fn clip_end(p: &Clip) -> u64 {
        p.start_ms.saturating_add(p.duration_ms)
    }

    /// 同轨道时间重叠检测(剪辑不变量;破坏即 CF-004 的前提)。
    pub fn overlaps(track: &Track) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        for i in 0..track.clips.len() {
            for j in (i + 1)..track.clips.len() {
                let a = &track.clips[i];
                let b = &track.clips[j];
                if a.start_ms < Self::clip_end(b) && b.start_ms < Self::clip_end(a) {
                    pairs.push((a.id.clone(), b.id.clone()));
                }
            }
        }
        pairs
    }

    /// 下一个可用片段 id:<轨道id>-<序号三位零填>。
    pub fn next_clip_id(track: &Track) -> String {
        let mut n = track.clips.len() + 1;
        loop {
            let id = format!("{}-{n:03}", track.id);
            if !track.clips.iter().any(|c| c.id == id) {
                return id;
            }
            n += 1;
        }
    }

    /// 轨道确定性 id:<kind 首字母大写><序号>。
    pub fn track_letter(kind: TrackKind) -> char {
        match kind {
            TrackKind::Video => 'V',
            TrackKind::Audio => 'A',
            TrackKind::Text => 'T',
        }
    }

    pub fn next_track_id(&self, kind: TrackKind) -> String {
        let letter = Self::track_letter(kind);
        let mut n = 1;
        loop {
            let id = format!("{letter}{n}");
            if !self.tracks.iter().any(|t| t.id == id) {
                return id;
            }
            n += 1;
        }
    }
}

/// v1→v2 迁移入口(委托 schema crate 的迁移器,再过一遍类型化模型)。
pub fn migrate_from_value(v: &Value) -> Result<Project, Vec<String>> {
    let migrated = cutforge_schema::migrate_project(v);
    Project::from_value(&migrated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "version": 1, "schemaVersion": "2.0.0",
            "slug": "测试工程", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "backends": ["ffmpeg", "cutforge"],
            "tracks": [
                {"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 8400,
                     "sourceInMs": 12000, "role": "voice"},
                    {"id": "V1-002", "src": "a.mp4", "startMs": 8400, "durationMs": 6200,
                     "transition": {"type": "fade", "durMs": 300, "reason": "topic"}}
                ]},
                {"id": "A1", "kind": "audio", "clips": [
                    {"id": "A1-001", "src": "sfx.mp3", "startMs": 8400, "durationMs": 400,
                     "role": "sfx", "volume": 0.8}
                ]}
            ]
        })
    }

    #[test]
    fn parse_and_validate_sample() {
        let v = sample();
        let p = Project::from_value(&v).expect("样本必须通过 v2 校验");
        assert_eq!(p.tracks.len(), 2);
        assert_eq!(p.find_clip("V1-002"), Some((0, 1)));
        assert_eq!(p.find_clip("不存在"), None);
        assert_eq!(p.find_track("A1"), Some(1));
    }

    #[test]
    fn roundtrip_value_preserves_semantics() {
        let v = sample();
        let p = Project::from_value(&v).unwrap();
        let back = p.to_validated_value().unwrap();
        let a: Project = serde_json::from_value(back).unwrap();
        assert_eq!(p, a);
    }

    #[test]
    fn rejects_schema_violation() {
        let mut v = sample();
        v["fps"] = json!(90); // 不在允许集
        assert!(Project::from_value(&v).is_err());
        v["fps"] = json!(30);
        v["幽灵字段"] = json!(1); // additionalProperties=false
        let errs = Project::from_value(&v).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("幽灵字段")));
    }

    #[test]
    fn migrate_v1_sample() {
        let mut v = sample();
        // 去掉 v2 字段并剥掉 id,构造 v1 形态
        v.as_object_mut().unwrap().remove("schemaVersion");
        v.as_object_mut().unwrap().remove("backends");
        v["tracks"][0].as_object_mut().unwrap().remove("id");
        v["tracks"][0]["clips"][0].as_object_mut().unwrap().remove("id");
        let p = migrate_from_value(&v).expect("迁移后必须合法");
        assert_eq!(p.schema_version, "2.0.0");
        assert_eq!(p.tracks[0].id, "V1");
        assert_eq!(p.tracks[0].clips[0].id, "V1-001");
        assert_eq!(p.backends, vec![Backend::Ffmpeg]);
    }

    #[test]
    fn overlap_detection() {
        let p = Project::from_value(&sample()).unwrap();
        assert!(Project::overlaps(&p.tracks[0]).is_empty());
        let mut t = p.tracks[0].clone();
        t.clips[1].start_ms = 100; // 与 V1-001 重叠
        let ov = Project::overlaps(&t);
        assert_eq!(ov, vec![("V1-001".to_string(), "V1-002".to_string())]);
    }

    #[test]
    fn id_generation() {
        let p = Project::from_value(&sample()).unwrap();
        assert_eq!(Project::next_clip_id(&p.tracks[0]), "V1-003");
        assert_eq!(p.next_track_id(TrackKind::Video), "V2");
        assert_eq!(p.next_track_id(TrackKind::Text), "T1");
    }
}
