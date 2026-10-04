// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 复合片段 IR(册五 T5.4/ADR-0019;拆分自 model.rs——行数红线 A1-3,纯移动;
//! model.rs `pub use` 保持 `crate::model::CompoundSpec` 路径逐字不变)。
//!
//! 决策口径(ADR-0019):复合片段 = **内联子时间线**——否决外部子工程引用
//! (单文件真相源纪律:引用即悬空断链/撤销跨文件无解/缓存键混入外部指纹);
//! 嵌套深度上限两级(子 clip 不得再带 compound,语义层拒绝三层);时间域为
//! 复合片段局部域;渲染递归展开见 cutforge-render::compound(内容寻址中间段);
//! 编辑 = 解包→改→重打包(个人自用心智,简单可审计)。

use serde::{Deserialize, Serialize};

/// 复合片段内联子时间线(对应 schema $defs/compound):`clips` 为局部时间域
/// 子片段(升序、不重叠、首尾相接,单轨 concat 语义,与主时间线视频轨同契约);
/// `canvas` 可省略(缺省 = 工程画幅)。整对象替换(与 crop/fx 同口径原子操作)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompoundSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<crate::model::Canvas>,
    pub clips: Vec<crate::model::Clip>,
}

impl CompoundSpec {
    /// 子时间线总时长(ms)= max(startMs+durationMs);渲染/打包共用同一口径。
    pub fn duration_ms(&self) -> u64 {
        self.clips
            .iter()
            .map(|c| c.start_ms + c.duration_ms)
            .max()
            .unwrap_or(0)
    }

    /// 子时间线语义校验(册五 T5.4):升序、两两不重叠且首尾相接(单轨 concat
    /// 语义,与主时间线视频轨同契约);子 clip 不得再带 compound(深度上限两级);
    /// 子 clip 关键帧走同一语义裁决(validate_clip_keyframes)。
    pub fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();
        let mut order: Vec<&crate::model::Clip> = self.clips.iter().collect();
        order.sort_by_key(|c| (c.start_ms, c.id.clone()));
        let mut prev_end: Option<u64> = None;
        for c in order {
            if c.compound.is_some() {
                errs.push(format!(
                    "clip {}: 复合嵌套超深(深度上限两级,拒绝三层;编辑请先解包)",
                    c.id
                ));
            }
            for e in crate::keyframes::validate_clip_keyframes(c) {
                errs.push(format!("clip {}: {e}", c.id));
            }
            let end = c.start_ms.saturating_add(c.duration_ms);
            if let Some(pe) = prev_end {
                if c.start_ms < pe {
                    errs.push(format!("clip {}: 子时间线重叠(上一段终点 {pe}ms)", c.id));
                } else if c.start_ms > pe {
                    errs.push(format!(
                        "clip {}: 子时间线有间隙(上一段终点 {pe}ms,本段起点 {}ms;子时间线为单轨 concat 语义,须首尾相接)",
                        c.id, c.start_ms
                    ));
                }
            }
            prev_end = Some(prev_end.map_or(end, |pe: u64| pe.max(end)));
        }
        errs
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    fn compound_project(inner: serde_json::Value) -> serde_json::Value {
        json!({
            "version": 1, "schemaVersion": "3.0.0", "slug": "compound", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": [{
                "id": "V1-001", "startMs": 0, "durationMs": 2000,
                "compound": {"clips": inner}
            }]}]
        })
    }

    /// 合法复合(子时间线升序/首尾相接)parse+roundtrip 零丢失;时长口径 = max end。
    #[test]
    fn compound_parse_roundtrip_and_duration() {
        let v = compound_project(json!([
            {"id": "V1-001", "src": "red.mp4", "startMs": 0, "durationMs": 1000},
            {"id": "V1-002", "src": "blue.mp4", "startMs": 1000, "durationMs": 1000,
             "transition": {"type": "fade", "durMs": 300}}
        ]));
        let p = crate::model::Project::from_value(&v).expect("合法复合必须通过校验");
        let shell = &p.tracks[0].clips[0];
        let cp = shell.compound.as_ref().unwrap();
        assert_eq!(cp.duration_ms(), 2000, "复合时长 = 子时间线总时长(max end)");
        assert_eq!(cp.canvas, None, "canvas 可省略");
        let back = p.to_validated_value().unwrap();
        assert_eq!(
            back["tracks"][0]["clips"][0]["compound"]["clips"]
                .as_array()
                .unwrap()
                .len(),
            2,
            "compound 子 clips 读写一轮不丢"
        );
    }

    /// 深度上限两级:子 clip 再带 compound → 拒绝三层(SCHEMA_INVALID)。
    #[test]
    fn compound_depth_beyond_two_rejected() {
        let v = compound_project(json!([
            {"id": "V1-001", "src": "red.mp4", "startMs": 0, "durationMs": 1000,
             "compound": {"clips": [
                {"id": "V1-002", "src": "x.mp4", "startMs": 0, "durationMs": 500}
             ]}}
        ]));
        let errs = crate::model::Project::from_value(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("嵌套超深")),
            "三层必须被拒: {errs:?}"
        );
    }

    /// 子时间线单轨语义:重叠/有间隙均拒绝(升序排序后首尾相接判定)。
    #[test]
    fn compound_inner_overlap_and_gap_rejected() {
        // 重叠
        let v = compound_project(json!([
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 1000},
            {"id": "V1-002", "src": "b.mp4", "startMs": 800, "durationMs": 1000}
        ]));
        let errs = crate::model::Project::from_value(&v).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("子时间线重叠")), "{errs:?}");
        // 间隙
        let v = compound_project(json!([
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 1000},
            {"id": "V1-002", "src": "b.mp4", "startMs": 1500, "durationMs": 1000}
        ]));
        let errs = crate::model::Project::from_value(&v).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("子时间线有间隙")),
            "{errs:?}"
        );
    }
}
