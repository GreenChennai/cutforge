// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 命令定义(计划书 2.6):命令是唯一的写入口,任何状态变更都表达为一条 Command,
//! 由 Engine::apply 翻译为 Op。不存在"直接赋值"的旁路。

use crate::model::{Clip, Crop, FxSpec, SpeedPoint};
use serde::{Deserialize, Serialize};

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
    /// 入场直通别名 mo.<id>(册四 T4.6;声明时优先于 in 枚举)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_fx: Option<String>,
    /// 出场直通别名 mo.<id>(册四 T4.6)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_fx: Option<String>,
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
    /// ducking 侧链参数(册五 T5.3):缺省 = 既有常量(threshold 0.03 线性域 /
    /// ratio 8 / attack 80ms / release 500ms),不给即行为零变化。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duck_threshold: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duck_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duck_attack_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duck_release_ms: Option<f64>,
}

/// 片段属性 patch(对应 MCP `clip_update`):只改出现的字段,字段级 Op 的
/// before/after 由此派生;transition/motion 嵌套子 patch(Some 承接,内部按字段合并)。
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
    /// 降噪档(册四 A4 T4.8):off/low/mid/high(混音链 afftdn)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denoise: Option<String>,
    /// 保速变调半音档(册四 A4 T4.8;±12,混音链 asetrate+atempo 补偿)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// 文本样式(册四 T4.7):整对象替换(渲染端 ADR-0016 ASS 生成消费)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_style: Option<crate::text_style::TextStyle>,
    /// 花字挂载(册四 T4.7):整对象替换(template+params)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub huazi: Option<crate::text_style::Huazi>,
    /// 花字显式清除(册四收口):patch.huazi = null 或空对象承接位——Option<Huazi>
    /// 表达不了「从有到无」,独立布尔承载(serde default 保 oplog 回放零迁移);
    /// 与 huazi 同现时清除胜出(派发层互斥构造)。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub huazi_clear: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeze_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<TransitionPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<MotionPatch>,
    /// 片段特效(册四 T4.6):整对象替换;combo 上限 3 在 schema 层界,顺序即应用序。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx: Option<FxSpec>,
    /// 关键帧(IR v3,T5.1):整组替换;白名单/互斥裁决 schema + 语义校验双闸。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyframes: Option<Vec<crate::keyframes::Keyframe>>,
    /// 片段调色(册五 T5.2):整对象替换(与 crop/fx 同口径原子操作);
    /// 显式清除走 grade_clear(patch.grade 为 null 或空对象即清除,与 huazi 同模式)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<crate::model::Grade>,
    /// 调色显式清除(册五 T5.2):patch.grade = null 或空对象承接位
    /// (Option<Grade> 表达不了「从有到无」;与 huazi_clear 同模式)。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub grade_clear: bool,
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
    /// 轨道 EQ(册五 T5.3):整组替换;显式清除走 eq_clear。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<Vec<crate::model::EqBand>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub eq_clear: bool,
    /// 轨道动态(册五 T5.3):整对象替换;显式清除走 dyn_clear。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dyn_: Option<crate::model::TrackDyn>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dyn_clear: bool,
}

impl TrackPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// 高层命令(M2 集合;编排类命令 stage_run/render 等属于 MCP 层,不进内核)。
// 变体尺寸差大,但命令按值传递、调用频率为人类编辑量级,装箱反而增分配,保留内联。
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
    /// 批量插入片段(册四 A4 T4.7 subtitle_import):单 Op 原子——任一片段
    /// id 重复或落点重叠则整批拒绝;id 已由派发层确定性分配。
    ClipsInsert { to_track: String, clips: Vec<Clip>, request_id: Option<String> },
    /// 批量改片段属性(册四 A4 T4.7 subtitle_replace 批量替换):单 Op 原子,
    /// 每个 (clipId, patch) 独立按字段合并;任一 clipId 不存在则整批拒绝。
    ClipsPatch { updates: Vec<(String, ClipPatch)> },
}

/// `clip_trim` 模式(册四 A4 T4.2):trim 单边/roll 双边联动/slip 内容/slide 位置。
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use crate::model::FxEntry;

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
            denoise: Some("high".into()),
            pitch: Some(3.0),
            opacity: Some(0.5),
            scale: Some(1.1),
            text: Some("字幕".into()),
            text_style: Some(crate::text_style::TextStyle { color: Some("#FFCC00".into()), ..Default::default() }),
            huazi: Some(crate::text_style::Huazi { template: "hz.pop".into(), params: None }),
            huazi_clear: false,
            freeze_ms: Some(300),
            transition: Some(TransitionPatch { type_: Some("fade".into()), dur_ms: Some(300.0), ..Default::default() }),
            motion: Some(MotionPatch { in_: Some("fadeIn".into()), ..Default::default() }),
            fx: None,
            keyframes: None,
            grade: None,
            grade_clear: false,
        }
        .apply_to(&mut c);
        assert_eq!(changes.len(), 21);
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
        assert_eq!(c.denoise.as_deref(), Some("high"));
        assert_eq!(c.pitch, Some(3.0));
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

    /// fx patch(册四 T4.6):整对象替换;先建后换;指针恒为 /fx;
    /// combo 数组顺序即应用顺序;同值不产变更。
    #[test]
    fn fx_patch_replaces_whole_object() {
        let mut c = clip();
        assert!(c.fx.is_none());
        // 首次:新建 + 一条变更
        let changes = ClipPatch {
            fx: Some(FxSpec {
                combo: Some(vec![
                    FxEntry { fx: "fx.mono".into(), params: None },
                    FxEntry {
                        fx: "fx.grain".into(),
                        params: Some(serde_json::from_str(r#"{"strength":24}"#).unwrap()),
                    },
                ]),
                ..Default::default()
            }),
            ..Default::default()
        }
        .apply_to(&mut c);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].0, "/fx");
        let fx = c.fx.as_ref().unwrap();
        assert_eq!(fx.combo.as_ref().unwrap()[0].fx, "fx.mono");
        assert_eq!(fx.combo.as_ref().unwrap()[1].fx, "fx.grain");
        // 覆盖:整对象替换(不逐项合并),combo 被换掉、in 槽位出现
        let changes = ClipPatch {
            fx: Some(FxSpec { in_: Some(FxEntry { fx: "mo.fadeIn".into(), params: None }), ..Default::default() }),
            ..Default::default()
        }
        .apply_to(&mut c);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].0, "/fx");
        let fx = c.fx.as_ref().unwrap();
        assert!(fx.combo.is_none(), "整对象替换:旧 combo 不得残留");
        assert_eq!(fx.in_.as_ref().unwrap().fx, "mo.fadeIn");
        // 同值不产变更
        let changes = ClipPatch {
            fx: Some(FxSpec { in_: Some(FxEntry { fx: "mo.fadeIn".into(), params: None }), ..Default::default() }),
            ..Default::default()
        }
        .apply_to(&mut c);
        assert!(changes.is_empty(), "同值 fx 不得计入变更: {changes:?}");
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

}
