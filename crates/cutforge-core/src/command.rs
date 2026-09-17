//! 命令定义(计划书 2.6):命令是唯一的写入口,任何状态变更都表达为一条 Command,
//! 由 Engine::apply 翻译为 Op。不存在"直接赋值"的旁路。

use crate::model::Clip;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 片段属性 patch(对应 MCP `clip_update`):只改出现的字段;
/// 字段级 Op 的 before/after 由此派生。
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
}

impl ClipPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
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
        changes
    }
}

fn json_num(v: u64) -> Value {
    Value::from(v as i64)
}

fn opt_json_num(v: Option<u64>) -> Value {
    v.map(json_num).unwrap_or(Value::Null)
}

fn json_f64(v: f64) -> Value {
    serde_json::Number::from_f64(v).map(Value::Number).unwrap_or(Value::Null)
}

fn opt_json_f64(v: Option<f64>) -> Value {
    v.map(json_f64).unwrap_or(Value::Null)
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
            volume: Some(0.8),
            opacity: Some(0.5),
            scale: Some(1.1),
            text: Some("字幕".into()),
            freeze_ms: Some(300),
        }
        .apply_to(&mut c);
        assert_eq!(changes.len(), 9);
        assert_eq!(c.start_ms, 100);
        assert_eq!(c.duration_ms, 8000);
        assert_eq!(c.source_in_ms, Some(12100));
        assert_eq!(c.speed, Some(1.25));
        assert_eq!(c.volume, Some(0.8));
        assert_eq!(c.opacity, Some(0.5));
        assert_eq!(c.scale, Some(1.1));
        assert_eq!(c.text.as_deref(), Some("字幕"));
        assert_eq!(c.freeze_ms, Some(300));
        // 指针路径与 before/after 成对
        let m: std::collections::BTreeMap<String, (serde_json::Value, serde_json::Value)> =
            changes.into_iter().map(|(p, o, n)| (p, (o, n))).collect();
        let (o, n) = m.get("/volume").unwrap();
        assert_eq!(*o, serde_json::json!(1.0));
        assert_eq!(*n, serde_json::json!(0.8));
    }

    #[test]
    fn same_value_patch_yields_no_changes() {
        let mut c = clip();
        let changes = ClipPatch { volume: Some(1.0), ..Default::default() }.apply_to(&mut c);
        assert!(changes.is_empty(), "同值字段不得计入变更");
    }
}
