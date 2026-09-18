// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 阶段脏传播(计划书 4.10):CutForge 改文件后只标记"下游变脏",**不自动重跑**;
//! 与 CutFlow 既有阶段缓存语义对齐(S8 只重烧不碰 IR 等)。

/// CutFlow 管线的 12 个阶段(S0–S11)。
pub const STAGES: [&str; 12] = [
    "S0", "S1", "S2", "S3", "S4", "S5", "S6", "S7", "S8", "S9", "S10", "S11",
];

#[derive(Debug, Clone, PartialEq)]
pub struct StageImpact {
    /// 变脏的起始阶段(该阶段及其下游全部变脏;`dirty` 恰为 [S8] 时例外,见下)。
    pub first_dirty: String,
    /// 变脏的阶段集合。
    pub dirty: Vec<String>,
    /// 人话提示:该跑哪个 rebuild(4.10 衔接表)。
    pub hint: String,
    /// 是否触发 CutFlow 的 B8 护栏(手注单 clip 音频/转场场景)。
    pub b8_guardrail: bool,
}

fn downstream(first: usize) -> Vec<String> {
    STAGES[first..].iter().map(|s| s.to_string()).collect()
}

/// 工程内相对路径 → 阶段影响(4.10 衔接表;未列出的路径返回 None,不标脏)。
pub fn impact_for(rel: &str) -> Option<StageImpact> {
    let norm = rel.replace('\\', "/");
    match norm.as_str() {
        // IR 变更:IR 在 S3 由 cutlist 生成 → S3 及下游脏,上游 S0–S2 不脏
        "05_ir/project.json" => Some(StageImpact {
            first_dirty: "S3".into(),
            dirty: downstream(3),
            hint: "python 05_ir/rebuild.py".into(),
            b8_guardrail: true,
        }),
        // 字幕 ASS:只重烧(S8),绝不重开 IR(S8 护栏)
        "06_output/subtitles.ass" => Some(StageImpact {
            first_dirty: "S8".into(),
            dirty: vec!["S8".into()],
            hint: "python 06_output/rebuild.py".into(),
            b8_guardrail: false,
        }),
        // 粗剪决策:S2 起重算(S3 IR 由 cutlist 派生,级联)
        p if p == "04_cut/cutlist.json" || p == "04_cut/cutlist.applied.json" => {
            Some(StageImpact {
                first_dirty: "S2".into(),
                dirty: downstream(2),
                hint: "python 04_cut/rebuild.py".into(),
                b8_guardrail: false,
            })
        }
        // artboard 卡片:S4 动画卡消费 → 03_assets rebuild
        p if p.starts_with("03_assets/artboard/") => Some(StageImpact {
            first_dirty: "S4".into(),
            dirty: downstream(4),
            hint: "python 03_assets/artboard/rebuild.py".into(),
            b8_guardrail: false,
        }),
        // wordline:全片时间真相源(S1 产物)→ S2 起全部重算
        "05_ir/wordline.json" => Some(StageImpact {
            first_dirty: "S2".into(),
            dirty: downstream(2),
            hint: "python rebuild.py --from S2".into(),
            b8_guardrail: false,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_change_marks_s3_downstream_only() {
        let imp = impact_for("05_ir/project.json").expect("project.json 必须有影响");
        assert_eq!(imp.first_dirty, "S3");
        assert!(!imp.dirty.iter().any(|s| s == "S0" || s == "S1" || s == "S2"), "上游不得标脏");
        assert_eq!(imp.dirty.len(), 9, "S3..S11 共 9 个阶段");
        assert_eq!(imp.dirty.first().unwrap(), "S3");
        assert_eq!(imp.dirty.last().unwrap(), "S11");
        assert!(imp.hint.contains("05_ir/rebuild.py"));
    }

    #[test]
    fn subtitle_ass_marks_only_s8() {
        let imp = impact_for("06_output/subtitles.ass").expect("subtitles.ass 必须有影响");
        assert_eq!(imp.dirty, vec!["S8".to_string()], "S8 护栏:只重烧,不碰 IR 阶段");
        assert!(!imp.dirty.iter().any(|s| s == "S3" || s == "S7" || s == "S9"));
        assert!(imp.hint.contains("06_output/rebuild.py"));
    }

    #[test]
    fn cutlist_and_artboard_and_wordline() {
        let cutlist = impact_for("04_cut/cutlist.applied.json").unwrap();
        assert_eq!(cutlist.first_dirty, "S2");
        assert!(cutlist.hint.contains("04_cut/rebuild.py"));

        let card = impact_for("03_assets/artboard/cards/kc01.mp4").unwrap();
        assert_eq!(card.first_dirty, "S4");
        assert!(card.hint.contains("03_assets/artboard/rebuild.py"));

        let wl = impact_for("05_ir/wordline.json").unwrap();
        assert_eq!(wl.first_dirty, "S2");
    }

    #[test]
    fn unknown_file_is_neutral() {
        assert!(impact_for("00_brief/brief.md").is_none());
        assert!(impact_for(".cutforge/rev").is_none());
        assert!(impact_for("06_output/final_x_916.mp4").is_none(), "成片产物不标脏");
    }
}
