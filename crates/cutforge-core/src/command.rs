// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 命令定义(计划书 2.6):命令是唯一的写入口,任何状态变更都表达为一条 Command,
//! 由 Engine::apply 翻译为 Op。不存在"直接赋值"的旁路。

use crate::model::{Clip, Crop, SpeedPoint};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 转场子 patch(对应 clip.transition;枚举约束在 schema 层)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionPatch {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dur_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx: Option<String>,
}

/// 动效子 patch(对应 clip.motion 的 in/inMs/out/outMs;枚举约束在 schema 层)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionPatch {
    #[serde(rename = "in", default, skip_serializing_if = "Option::is_none")]
    pub in_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_ms: Option<f64>,
}

/// 工程级背景乐 patch(对应 doc.bgm;项目级字段,不经 ClipPatch)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BgmPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ducking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "loop")]
    pub loop_: Option<bool>,
}

/// 片段属性 patch(对应 MCP `clip_update`):只改出现的字段;
/// 字段级 Op 的 before/after 由此派生。transition/motion 为嵌套子 patch:
/// 外层 Some = 承接该对象,内部再按字段合并(None 不改)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_in_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    /// 分段速度曲线(册四 A4 T4.4):整组替换(数组在三路合并中整体为一个值,语义一致)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_curve: Option<Vec<SpeedPoint>>,
    /// 倒放开关(册四 A4 T4.4)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse: Option<bool>,
    /// 旋转角度(度;册四 A4 T4.9)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    /// 源域裁剪矩形(册四 A4 T4.9):整对象替换(原子构图操作)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<Crop>,
    /// 翻转(none/h/v;册四 A4 T4.9)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeze_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<TransitionPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<MotionPatch>,
}

impl ClipPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl TransitionPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl MotionPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// 轨道属性 patch(对应 MCP `track_update`;册四 A4 T4.2):只改出现的字段
/// (None = 不改),字段级 Op 的 before/after 由此派生。静音/独奏/隐藏的
/// 渲染混音联动候 BE3,本册只保证 IR 字段 + 命令 + 契约链就位。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mute: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solo: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_px: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl TrackPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// 应用到 track 并返回字段级变更(指针相对 track 对象,即 /tracks/{i}/…)。
    pub fn apply_to(self, track: &mut crate::model::Track) -> Vec<FieldChange> {
        let mut changes: Vec<FieldChange> = Vec::new();
        let mut record = |name: &str, old: Value, new: Value| {
            if old != new {
                changes.push((format!("/{name}"), old, new));
            }
        };
        if let Some(v) = self.name {
            let old = track.name.replace(v.clone());
            record("name", old.map(Value::String).unwrap_or(Value::Null), Value::String(v));
        }
        if let Some(v) = self.locked {
            let old = track.locked.replace(v);
            record("locked", opt_json_bool(old), Value::from(v));
        }
        if let Some(v) = self.mute {
            let old = track.mute.replace(v);
            record("mute", opt_json_bool(old), Value::from(v));
        }
        if let Some(v) = self.solo {
            let old = track.solo.replace(v);
            record("solo", opt_json_bool(old), Value::from(v));
        }
        if let Some(v) = self.hidden {
            let old = track.hidden.replace(v);
            record("hidden", opt_json_bool(old), Value::from(v));
        }
        if let Some(v) = self.height_px {
            let old = track.height_px.replace(v);
            record("heightPx", opt_json_num(old), json_num(v));
        }
        if let Some(v) = self.color {
            let old = track.color.replace(v.clone());
            record("color", old.map(Value::String).unwrap_or(Value::Null), Value::String(v));
        }
        changes
    }
}

/// `clip_trim` 模式(册四 A4 T4.2):trim 单边伸缩 / roll 边界双边联动 /
/// slip 内容平移 / slide 位置平移。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimMode {
    Trim,
    Roll,
    Slip,
    Slide,
}

/// `clip_trim` 的作用边(边缘;trim/roll 必给,slip/slide 不适用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimEdge {
    In,
    Out,
}

impl BgmPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// 应用到 doc.bgm 并返回字段级变更(指针相对工程根,即 /bgm/…)。
    /// 工程尚无 bgm 时按 schema 默认值(gainDb=-18/ducking=true/loop=true)新建;
    /// 是否"无 bgm 且无 src"的前置拒绝由 Engine::mutate 承接。
    pub fn apply_to(self, bgm: &mut Option<crate::model::Bgm>) -> Vec<FieldChange> {
        let mut changes: Vec<FieldChange> = Vec::new();
        let mut record = |name: &str, old: Value, new: Value| {
            if old != new {
                changes.push((format!("/bgm/{name}"), old, new));
            }
        };
        let mut b = bgm.take().unwrap_or(crate::model::Bgm {
            src: String::new(),
            gain_db: -18.0,
            ducking: true,
            loop_: true,
        });
        if let Some(v) = self.src {
            let old = std::mem::replace(&mut b.src, v.clone());
            record("src", Value::String(old), Value::String(v));
        }
        if let Some(v) = self.gain_db {
            let old = b.gain_db;
            b.gain_db = v;
            record("gainDb", json_f64(old), json_f64(v));
        }
        if let Some(v) = self.ducking {
            let old = b.ducking;
            b.ducking = v;
            record("ducking", Value::from(old), Value::from(v));
        }
        if let Some(v) = self.loop_ {
            let old = b.loop_;
            b.loop_ = v;
            record("loop", Value::from(old), Value::from(v));
        }
        *bgm = Some(b);
        changes
    }
}

/// 高层命令(M2 集合;编排类命令 stage_run/render 等属于 MCP 层,不进内核)。
// ClipPatch 携带九个可选字段导致变体尺寸差;命令按值传递、调用频率为人类编辑量级,
// 装箱反而增加分配,故保留内联。
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// 改片段属性(幂等:同 patch 重复应用结果一致)。
    ClipUpdate { clip_id: String, patch: ClipPatch },
    /// 在时间点切分片段(已切过则返回既有结果的幂等语义)。
    ClipSplit { clip_id: String, t_ms: u64 },
    /// 删除片段。
    ClipDelete { clip_id: String },
    /// 移动片段(改起点,可选换轨;换轨必须同 kind)。
    ClipMove { clip_id: String, new_start_ms: u64, to_track: Option<String> },
    /// 新增片段(overlay_add/sfx_add 等非幂等写由此承接,须带 request_id 去重)。
    ClipInsert { to_track: String, clip: Clip, request_id: Option<String> },
    /// 合并相邻两片段(left 在前且边界相接;无损逆操作)。
    ClipMerge { left_id: String, right_id: String },
    /// 新增空轨道(M10 多轨管理;id 由 Engine 按 kind 确定性生成)。
    TrackAdd { kind: crate::model::TrackKind, request_id: Option<String> },
    /// 设置/合并工程级背景乐(doc.bgm;工程尚无 bgm 时 patch 必须携带 src)。
    BgmSet { patch: BgmPatch },
    /// 清除工程级背景乐(已无 bgm 时幂等)。
    BgmClear,
    /// 片段修剪(册四 A4 T4.2 四模式):trim 单边伸缩(edge 必给)/
    /// roll 相邻边界双边联动(总时长不变)/ slip 内容平移(sourceInMs 平移,
    /// 时间线占位不变)/ slide 位置平移(贴合邻居让位/压缩)。
    /// 硬约束(时长>0、不越 0、无重叠)违反返回既有 InvariantViolation/NotAdjacent。
    ClipTrim { clip_id: String, mode: TrimMode, edge: TrimEdge, delta_ms: i64 },
    /// 播放头处**所有轨**命中的片段一次全分割(单 Op;切点在片段内部才切)。
    ClipSplitAll { t_ms: u64 },
    /// 轨道属性 patch(幂等;None 字段不改)。
    TrackUpdate { track_id: String, patch: TrackPatch },
    /// 删除片段间间隙:定位轨上包含 t_ms 的间隙,后继片段整体左移闭合(单 Op 原子)。
    ClipGapDelete { track_id: String, t_ms: u64 },
}

/// 从 patch 派生的字段级变更(指针片段 → before/after),用于生成叶级 Op。
pub type FieldChange = (String, Value, Value);

impl ClipPatch {
    /// 应用到 clip 并返回字段级变更(指针相对 clip 对象)。
    pub fn apply_to(self, clip: &mut Clip) -> Vec<FieldChange> {
        let mut changes: Vec<FieldChange> = Vec::new();
        let mut record = |name: &str, old: Value, new: Value| {
            if old != new {
                changes.push((format!("/{name}"), old, new));
            }
        };
        if let Some(v) = self.start_ms {
            let old = clip.start_ms;
            clip.start_ms = v;
            record("startMs", json_num(old), json_num(v));
        }
        if let Some(v) = self.duration_ms {
            let old = clip.duration_ms;
            clip.duration_ms = v;
            record("durationMs", json_num(old), json_num(v));
        }
        if let Some(v) = self.source_in_ms {
            let old = clip.source_in_ms;
            clip.source_in_ms = Some(v);
            record("sourceInMs", opt_json_num(old), json_num(v));
        }
        if let Some(v) = self.speed {
            let old = clip.speed;
            clip.speed = Some(v);
            record("speed", opt_json_f64(old), json_f64(v));
        }
        if let Some(v) = self.speed_curve {
            let old = clip.speed_curve.replace(v.clone());
            record("speedCurve",
                   old.map(|ps| json_points(&ps)).unwrap_or(Value::Null),
                   json_points(&v));
        }
        if let Some(v) = self.reverse {
            let old = clip.reverse;
            clip.reverse = Some(v);
            record("reverse", opt_json_bool(old), Value::from(v));
        }
        if let Some(v) = self.rotation {
            let old = clip.rotation;
            clip.rotation = Some(v);
            record("rotation", opt_json_f64(old), json_f64(v));
        }
        if let Some(v) = self.crop {
            let old = clip.crop.replace(v);
            record("crop", old.map(|c| serde_json::to_value(c).unwrap_or(Value::Null)).unwrap_or(Value::Null),
                   serde_json::to_value(v).unwrap_or(Value::Null));
        }
        if let Some(v) = self.flip {
            let old = clip.flip.replace(v.clone());
            record("flip", old.map(Value::String).unwrap_or(Value::Null), Value::String(v));
        }
        if let Some(v) = self.volume {
            let old = clip.volume;
            clip.volume = Some(v);
            record("volume", opt_json_f64(old), json_f64(v));
        }
        if let Some(v) = self.opacity {
            let old = clip.opacity;
            clip.opacity = Some(v);
            record("opacity", opt_json_f64(old), json_f64(v));
        }
        if let Some(v) = self.scale {
            let old = clip.scale;
            clip.scale = Some(v);
            record("scale", opt_json_f64(old), json_f64(v));
        }
        if let Some(v) = self.text {
            let old = clip.text.take();
            clip.text = Some(v.clone());
            record("text", old.map(Value::String).unwrap_or(Value::Null), Value::String(v));
        }
        if let Some(v) = self.freeze_ms {
            let old = clip.freeze_ms;
            clip.freeze_ms = Some(v);
            record("freezeMs", opt_json_num(old), json_num(v));
        }
        if let Some(tp) = self.transition {
            // 按字段合并(None 不改;transition 对象本身不存在则按 schema 默认新建)
            let t = clip.transition.get_or_insert_with(Default::default);
            if let Some(v) = tp.type_ {
                let old = t.type_.replace(v.clone());
                record("transition/type", old.map(Value::String).unwrap_or(Value::Null), Value::String(v));
            }
            if let Some(v) = tp.dur_ms {
                let old = t.dur_ms.replace(v);
                record("transition/durMs", opt_json_f64(old), json_f64(v));
            }
            if let Some(v) = tp.reason {
                let old = t.reason.replace(v.clone());
                record("transition/reason", old.map(Value::String).unwrap_or(Value::Null), Value::String(v));
            }
            if let Some(v) = tp.fx {
                let old = t.fx.replace(v.clone());
                record("transition/fx", old.map(Value::String).unwrap_or(Value::Null), Value::String(v));
            }
        }
        if let Some(mp) = self.motion {
            let m = clip.motion.get_or_insert_with(Default::default);
            if let Some(v) = mp.in_ {
                let old = m.in_.replace(v.clone());
                record("motion/in", old.map(Value::String).unwrap_or(Value::Null), Value::String(v));
            }
            if let Some(v) = mp.in_ms {
                let old = m.in_ms.replace(v);
                record("motion/inMs", opt_json_f64(old), json_f64(v));
            }
            if let Some(v) = mp.out {
                let old = m.out.replace(v.clone());
                record("motion/out", old.map(Value::String).unwrap_or(Value::Null), Value::String(v));
            }
            if let Some(v) = mp.out_ms {
                let old = m.out_ms.replace(v);
                record("motion/outMs", opt_json_f64(old), json_f64(v));
            }
        }
        changes
    }
}

fn json_num(v: u64) -> Value {
    Value::from(v as i64)
}

fn opt_json_num(v: Option<u64>) -> Value {
    v.map(json_num).unwrap_or(Value::Null)
}

fn opt_json_bool(v: Option<bool>) -> Value {
    v.map(Value::from).unwrap_or(Value::Null)
}

fn json_f64(v: f64) -> Value {
    serde_json::Number::from_f64(v).map(Value::Number).unwrap_or(Value::Null)
}

fn opt_json_f64(v: Option<f64>) -> Value {
    v.map(json_f64).unwrap_or(Value::Null)
}

/// 速度曲线点集 → JSON 数组([{atMs, speed}, …];serde 形态与 Clip 落盘逐字一致)。
fn json_points(v: &[SpeedPoint]) -> Value {
    Value::Array(v.iter().map(|p| serde_json::to_value(p).unwrap_or(Value::Null)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip() -> Clip {
        serde_json::from_value(serde_json::json!({
            "id": "V1-001", "startMs": 0, "durationMs": 8400,
            "sourceInMs": 12000, "volume": 1.0
        }))
        .unwrap()
    }

    #[test]
    fn full_patch_records_changes() {
        let mut c = clip();
        let changes = ClipPatch {
            start_ms: Some(100),
            duration_ms: Some(8000),
            source_in_ms: Some(12100),
            speed: Some(1.25),
            speed_curve: Some(vec![SpeedPoint { at_ms: 0, speed: 1.25 }]),
            reverse: Some(false),
            rotation: Some(0.0),
            crop: Some(Crop { x: 0, y: 0, w: 320, h: 240 }),
            flip: Some("none".into()),
            volume: Some(0.8),
            opacity: Some(0.5),
            scale: Some(1.1),
            text: Some("字幕".into()),
            freeze_ms: Some(300),
            transition: Some(TransitionPatch { type_: Some("fade".into()), dur_ms: Some(300.0), ..Default::default() }),
            motion: Some(MotionPatch { in_: Some("fadeIn".into()), ..Default::default() }),
        }
        .apply_to(&mut c);
        assert_eq!(changes.len(), 17);
        assert_eq!(c.start_ms, 100);
        assert_eq!(c.duration_ms, 8000);
        assert_eq!(c.source_in_ms, Some(12100));
        assert_eq!(c.speed, Some(1.25));
        assert_eq!(c.speed_curve.as_ref().unwrap().len(), 1);
        assert_eq!(c.reverse, Some(false));
        assert_eq!(c.rotation, Some(0.0));
        assert_eq!(c.crop, Some(Crop { x: 0, y: 0, w: 320, h: 240 }));
        assert_eq!(c.flip.as_deref(), Some("none"));
        assert_eq!(c.volume, Some(0.8));
        assert_eq!(c.opacity, Some(0.5));
        assert_eq!(c.scale, Some(1.1));
        assert_eq!(c.text.as_deref(), Some("字幕"));
        assert_eq!(c.freeze_ms, Some(300));
        let tr = c.transition.as_ref().unwrap();
        assert_eq!(tr.type_.as_deref(), Some("fade"));
        assert_eq!(tr.dur_ms, Some(300.0));
        assert_eq!(c.motion.as_ref().unwrap().in_.as_deref(), Some("fadeIn"));
        // 指针路径与 before/after 成对
        let m: std::collections::BTreeMap<String, (serde_json::Value, serde_json::Value)> =
            changes.into_iter().map(|(p, o, n)| (p, (o, n))).collect();
        let (o, n) = m.get("/volume").unwrap();
        assert_eq!(*o, serde_json::json!(1.0));
        assert_eq!(*n, serde_json::json!(0.8));
        let (o, n) = m.get("/transition/type").unwrap();
        assert_eq!(*o, serde_json::Value::Null);
        assert_eq!(*n, serde_json::json!("fade"));
    }

    #[test]
    fn same_value_patch_yields_no_changes() {
        let mut c = clip();
        let changes = ClipPatch { volume: Some(1.0), ..Default::default() }.apply_to(&mut c);
        assert!(changes.is_empty(), "同值字段不得计入变更");
    }

    /// 册四 A4 T4.4/T4.9:速度/时间与变换五字段(speedCurve/reverse/rotation/crop/flip)
    /// 的 patch 合并语义:整组替换、同值不产变更、None 不改、undo 用的 before/after 成对。
    #[test]
    fn time_transform_patch_replaces_whole_value() {
        let mut c = clip(); // volume=1.0,无新字段
        let changes = ClipPatch {
            speed_curve: Some(vec![
                SpeedPoint { at_ms: 0, speed: 0.5 },
                SpeedPoint { at_ms: 1000, speed: 2.0 },
            ]),
            reverse: Some(true),
            rotation: Some(-90.0),
            crop: Some(Crop { x: 10, y: 0, w: 160, h: 120 }),
            flip: Some("h".into()),
            ..Default::default()
        }
        .apply_to(&mut c);
        assert_eq!(changes.len(), 5);
        let m: std::collections::BTreeMap<String, (Value, Value)> =
            changes.into_iter().map(|(p, o, n)| (p, (o, n))).collect();
        // before 均为 Null(字段此前缺席);after 与落盘形态逐字一致
        let (o, n) = m.get("/speedCurve").unwrap();
        assert_eq!(*o, Value::Null);
        assert_eq!(n[0]["atMs"], serde_json::json!(0));
        assert_eq!(n[1]["speed"], serde_json::json!(2.0));
        assert_eq!(m.get("/reverse").unwrap().1, serde_json::json!(true));
        assert_eq!(m.get("/rotation").unwrap().1, serde_json::json!(-90.0));
        let (_, cn) = m.get("/crop").unwrap();
        assert_eq!(cn["w"], serde_json::json!(160));
        assert_eq!(m.get("/flip").unwrap().1, serde_json::json!("h"));
        assert_eq!(c.speed_curve.as_ref().unwrap().len(), 2);
        assert_eq!(c.crop, Some(Crop { x: 10, y: 0, w: 160, h: 120 }));

        // 同值重复应用:零变更(幂等)
        let changes = ClipPatch {
            speed_curve: Some(vec![
                SpeedPoint { at_ms: 0, speed: 0.5 },
                SpeedPoint { at_ms: 1000, speed: 2.0 },
            ]),
            reverse: Some(true),
            rotation: Some(-90.0),
            crop: Some(Crop { x: 10, y: 0, w: 160, h: 120 }),
            flip: Some("h".into()),
            ..Default::default()
        }
        .apply_to(&mut c);
        assert!(changes.is_empty(), "同值整组替换不得计入变更: {changes:?}");

        // 部分合并:只改 rotation,其余保持(None 不改)
        let changes = ClipPatch { rotation: Some(45.0), ..Default::default() }.apply_to(&mut c);
        assert_eq!(changes.len(), 1);
        assert_eq!(c.rotation, Some(45.0));
        assert_eq!(c.flip.as_deref(), Some("h"), "未给出的字段不得被清掉");
    }

    /// 嵌套子 patch 合并语义:None 不改;Some 只覆盖给出的字段(部分合并);
    /// transition 对象不存在时按字段新建;同值不产变更。
    #[test]
    fn nested_transition_motion_patch_merges_by_field() {
        let mut c = clip();
        assert!(c.transition.is_none() && c.motion.is_none());

        // None = 不改:对象仍缺席
        let changes = ClipPatch::default().apply_to(&mut c);
        assert!(changes.is_empty());
        assert!(c.transition.is_none() && c.motion.is_none());

        // 首次给出:只给 type/fx,其余字段缺席 → 新建对象且只含给出的字段
        let changes = ClipPatch {
            transition: Some(TransitionPatch { type_: Some("wipeleft".into()), fx: Some("tr.demo".into()), ..Default::default() }),
            ..Default::default()
        }
        .apply_to(&mut c);
        assert_eq!(changes.len(), 2);
        let tr = c.transition.as_ref().unwrap();
        assert_eq!(tr.type_.as_deref(), Some("wipeleft"));
        assert_eq!(tr.fx.as_deref(), Some("tr.demo"));
        assert_eq!(tr.dur_ms, None, "未给出的字段不得臆造");
        assert!(tr.reason.is_none());

        // 部分覆盖:durMs/reason 合并进既有对象,type/fx 保持
        let changes = ClipPatch {
            transition: Some(TransitionPatch { dur_ms: Some(420.0), reason: Some("topic".into()), ..Default::default() }),
            ..Default::default()
        }
        .apply_to(&mut c);
        assert_eq!(changes.len(), 2);
        let tr = c.transition.as_ref().unwrap();
        assert_eq!(tr.type_.as_deref(), Some("wipeleft"), "未给出的字段不得被清掉");
        assert_eq!(tr.fx.as_deref(), Some("tr.demo"));
        assert_eq!(tr.dur_ms, Some(420.0));
        assert_eq!(tr.reason.as_deref(), Some("topic"));

        // Some 覆盖同名字段
        ClipPatch {
            transition: Some(TransitionPatch { type_: Some("circleopen".into()), ..Default::default() }),
            motion: Some(MotionPatch { in_: Some("slideInLeft".into()), in_ms: Some(280.0), ..Default::default() }),
            ..Default::default()
        }
        .apply_to(&mut c);
        assert_eq!(c.transition.as_ref().unwrap().type_.as_deref(), Some("circleopen"));
        let m = c.motion.as_ref().unwrap();
        assert_eq!(m.in_.as_deref(), Some("slideInLeft"));
        assert_eq!(m.in_ms, Some(280.0));
        assert!(m.out.is_none(), "未给出的 out 不得臆造");

        // 同值 patch 不产变更
        let changes = ClipPatch {
            transition: Some(TransitionPatch { type_: Some("circleopen".into()), ..Default::default() }),
            motion: Some(MotionPatch { in_ms: Some(280.0), ..Default::default() }),
            ..Default::default()
        }
        .apply_to(&mut c);
        assert!(changes.is_empty(), "同值嵌套字段不得计入变更: {changes:?}");
    }

    /// BgmPatch:合并语义 + 无 bgm 时按 schema 默认新建;is_empty 口径。
    #[test]
    fn bgm_patch_merges_and_creates_with_schema_defaults() {
        let mut bgm: Option<crate::model::Bgm> = None;
        // 无 bgm 时只给 gainDb → 新建但 src 为空串(Engine 层拒绝此前置,见 engine 测试)
        let changes = BgmPatch { gain_db: Some(-12.0), ..Default::default() }.apply_to(&mut bgm);
        let b = bgm.as_ref().unwrap();
        assert_eq!(b.gain_db, -12.0);
        assert_eq!(b.src, "");
        assert!(b.ducking && b.loop_, "新建必须落 schema 默认 ducking=true/loop=true");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].0, "/bgm/gainDb");

        // 合并:src/loop 覆盖,gainDb/ducking 保持
        let changes = BgmPatch {
            src: Some("02_音乐/bgm.mp3".into()),
            loop_: Some(false),
            ..Default::default()
        }
        .apply_to(&mut bgm);
        assert_eq!(changes.len(), 2);
        let b = bgm.as_ref().unwrap();
        assert_eq!(b.src, "02_音乐/bgm.mp3");
        assert!(!b.loop_);
        assert_eq!(b.gain_db, -12.0, "未给出的 gainDb 不得被重置");
        assert!(b.ducking);

        // 同值不产变更
        let changes = BgmPatch { src: Some("02_音乐/bgm.mp3".into()), ..Default::default() }.apply_to(&mut bgm);
        assert!(changes.is_empty(), "同值 bgm 字段不得计入变更");

        assert!(BgmPatch::default().is_empty());
        assert!(!BgmPatch { src: Some("x".into()), ..Default::default() }.is_empty());
    }

    /// TrackPatch(track_update 的合并语义):None=不改;同值不产变更;
    /// 指针相对 track 对象;缺省字段不臆造。
    #[test]
    fn track_patch_merges_by_field() {
        let mut t: crate::model::Track = serde_json::from_value(serde_json::json!({
            "id": "V1", "kind": "video", "clips": []
        }))
        .unwrap();
        // 全字段 patch:7 项变更,指针逐一对应
        let changes = TrackPatch {
            name: Some("主画面".into()), locked: Some(true), mute: Some(false),
            solo: Some(true), hidden: Some(false), height_px: Some(240),
            color: Some("#3D7EAF".into()),
        }
        .apply_to(&mut t);
        assert_eq!(changes.len(), 7);
        let m: std::collections::BTreeMap<String, (Value, Value)> =
            changes.into_iter().map(|(p, o, n)| (p, (o, n))).collect();
        assert_eq!(m["/name"].1, serde_json::json!("主画面"));
        assert_eq!(m["/heightPx"].1, serde_json::json!(240));
        assert_eq!(m["/color"].1, serde_json::json!("#3D7EAF"));
        assert_eq!(t.height_px, Some(240));
        // 部分合并:只改 mute,name/heightPx 保持
        let changes = TrackPatch { mute: Some(true), ..Default::default() }.apply_to(&mut t);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].0, "/mute");
        assert_eq!(t.name.as_deref(), Some("主画面"), "未给出的字段不得被清掉");
        assert_eq!(t.mute, Some(true));
        // 同值 patch 不产变更
        let changes = TrackPatch { mute: Some(true), ..Default::default() }.apply_to(&mut t);
        assert!(changes.is_empty(), "同值 track 字段不得计入变更");
        assert!(TrackPatch::default().is_empty());
        assert!(!TrackPatch { color: Some("#111111".into()), ..Default::default() }.is_empty());
    }
}
