// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! Patch 应用实现(册五 T5.2/T5.3 拆分自 command.rs——行数红线 A1-3,纯移动):
//! [`crate::command::ClipPatch::apply_to`] / [`TrackPatch::apply_to`] / [`BgmPatch::apply_to`]
//! 与 json 叶级值小工具。同 crate 内 inherent impl 跨模块合法,类型与调用路径零变化。

use crate::command::{BgmPatch, ClipPatch, TrackPatch};
use crate::model::Clip;
use crate::model::SpeedPoint;
use serde_json::Value;

impl TrackPatch {
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
        if let Some(v) = self.eq {
            // 整组替换(册五 T5.3;数组在三路合并中整体为一个值)
            let old = track.eq.replace(v.clone());
            record(
                "eq",
                old.map(|b| serde_json::to_value(b).unwrap_or(Value::Null)).unwrap_or(Value::Null),
                serde_json::to_value(v).unwrap_or(Value::Null),
            );
        }
        if self.eq_clear {
            let old = track.eq.take();
            record(
                "eq",
                old.map(|b| serde_json::to_value(b).unwrap_or(Value::Null)).unwrap_or(Value::Null),
                Value::Null,
            );
        }
        if let Some(v) = self.dyn_ {
            let old = track.dyn_.replace(v.clone());
            record(
                "dyn",
                old.map(|d| serde_json::to_value(d).unwrap_or(Value::Null)).unwrap_or(Value::Null),
                serde_json::to_value(v).unwrap_or(Value::Null),
            );
        }
        if self.dyn_clear {
            let old = track.dyn_.take();
            record(
                "dyn",
                old.map(|d| serde_json::to_value(d).unwrap_or(Value::Null)).unwrap_or(Value::Null),
                Value::Null,
            );
        }
        changes
    }
}

impl BgmPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// 应用到 doc.bgm 并返回字段级变更(指针 /bgm/…);无 bgm 时按 schema 默认值
    /// 新建(gainDb=-18/ducking=true/loop=true);"无 bgm 无 src"拒绝在 Engine::mutate。
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
            duck_threshold: None,
            duck_ratio: None,
            duck_attack_ms: None,
            duck_release_ms: None,
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
        if let Some(v) = self.duck_threshold {
            let old = b.duck_threshold.replace(v);
            record("duckThreshold", opt_json_f64(old), json_f64(v));
        }
        if let Some(v) = self.duck_ratio {
            let old = b.duck_ratio.replace(v);
            record("duckRatio", opt_json_f64(old), json_f64(v));
        }
        if let Some(v) = self.duck_attack_ms {
            let old = b.duck_attack_ms.replace(v);
            record("duckAttackMs", opt_json_f64(old), json_f64(v));
        }
        if let Some(v) = self.duck_release_ms {
            let old = b.duck_release_ms.replace(v);
            record("duckReleaseMs", opt_json_f64(old), json_f64(v));
        }
        *bgm = Some(b);
        changes
    }
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
        if let Some(v) = self.denoise {
            let old = clip.denoise.replace(v.clone());
            record("denoise", old.map(Value::String).unwrap_or(Value::Null), Value::String(v));
        }
        if let Some(v) = self.pitch {
            let old = clip.pitch;
            clip.pitch = Some(v);
            record("pitch", opt_json_f64(old), json_f64(v));
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
        if let Some(v) = self.text_style {
            // 整对象替换(册四 T4.7;与 crop 同口径原子操作)
            let old = clip.text_style.replace(v.clone());
            record(
                "textStyle",
                old.map(|t| serde_json::to_value(t).unwrap_or(Value::Null)).unwrap_or(Value::Null),
                serde_json::to_value(v).unwrap_or(Value::Null),
            );
        }
        if let Some(v) = self.huazi {
            let old = clip.huazi.replace(v.clone());
            record(
                "huazi",
                old.map(|t| serde_json::to_value(t).unwrap_or(Value::Null)).unwrap_or(Value::Null),
                serde_json::to_value(v).unwrap_or(Value::Null),
            );
        }
        if self.huazi_clear {
            // 显式清除(册四收口;huazi_clear 与 huazi 同现时清除胜出,replay 稳健)
            let old = clip.huazi.take();
            record(
                "huazi",
                old.map(|t| serde_json::to_value(t).unwrap_or(Value::Null)).unwrap_or(Value::Null),
                Value::Null,
            );
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
            if let Some(v) = mp.in_fx {
                let old = m.in_fx.replace(v.clone());
                record("motion/inFx", old.map(Value::String).unwrap_or(Value::Null), Value::String(v));
            }
            if let Some(v) = mp.out_fx {
                let old = m.out_fx.replace(v.clone());
                record("motion/outFx", old.map(Value::String).unwrap_or(Value::Null), Value::String(v));
            }
        }
        if let Some(fx) = self.fx {
            // 整对象替换(册四 T4.6;与 crop 同口径原子操作)
            let old = clip.fx.replace(fx.clone());
            record(
                "fx",
                old.map(|f| serde_json::to_value(f).unwrap_or(Value::Null)).unwrap_or(Value::Null),
                serde_json::to_value(fx).unwrap_or(Value::Null),
            );
        }
        if let Some(v) = self.keyframes {
            // 整组替换(IR v3 T5.1;数组整体一个值,merge 语义一致)
            let old = clip.keyframes.replace(v.clone());
            record(
                "keyframes",
                old.map(|ks| serde_json::to_value(ks).unwrap_or(Value::Null)).unwrap_or(Value::Null),
                serde_json::to_value(v).unwrap_or(Value::Null),
            );
        }
        if let Some(v) = self.grade {
            // 整对象替换(册五 T5.2;与 crop/fx 同口径原子操作)
            let old = clip.grade.replace(v.clone());
            record(
                "grade",
                old.map(|g| serde_json::to_value(g).unwrap_or(Value::Null)).unwrap_or(Value::Null),
                serde_json::to_value(v).unwrap_or(Value::Null),
            );
        }
        if self.grade_clear {
            // 显式清除(册五 T5.2;grade_clear 与 grade 同现时清除胜出,replay 稳健)
            let old = clip.grade.take();
            record(
                "grade",
                old.map(|g| serde_json::to_value(g).unwrap_or(Value::Null)).unwrap_or(Value::Null),
                Value::Null,
            );
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

