// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 阶段脏传播(计划书 4.10):CutForge 改文件后只标记"下游变脏",**不自动重跑**;
//! 与 CutFlow 既有阶段缓存语义对齐(S8 只重烧不碰 IR 等)。
//!
//! 路径匹配走 `paths` 契约常量(0.5 目录中文化):新旧两套相对路径都认——
//! 同一版本要伺候两种盘面(0.4.x 旧工程兼容读写);提示串一律给新目录
//! (CutFlow 侧已按同一契约改名,rebuild 脚本随之落新目录)。

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
    // IR 变更:IR 在 S3 由 cutlist 生成 → S3 及下游脏,上游 S0–S2 不脏
    if norm == crate::paths::PROJECT_REL || norm == crate::paths::LEGACY_PROJECT_REL {
        return Some(StageImpact {
            first_dirty: "S3".into(),
            dirty: downstream(3),
            hint: format!("python {}/rebuild.py", crate::paths::TIMELINE),
            b8_guardrail: true,
        });
    }
    // 字幕 ASS:只重烧(S8),绝不重开 IR(S8 护栏)
    if norm == format!("{}/subtitles.ass", crate::paths::OUTPUT)
        || norm == format!("{}/subtitles.ass", crate::paths::LEGACY_OUTPUT)
    {
        return Some(StageImpact {
            first_dirty: "S8".into(),
            dirty: vec!["S8".into()],
            hint: format!("python {}/rebuild.py", crate::paths::OUTPUT),
            b8_guardrail: false,
        });
    }
    // 粗剪决策:S2 起重算(S3 IR 由 cutlist 派生,级联)
    if norm == crate::paths::CUTLIST_REL
        || norm == crate::paths::CUTLIST_APPLIED_REL
        || norm == crate::paths::LEGACY_CUTLIST_REL
        || norm == crate::paths::LEGACY_CUTLIST_APPLIED_REL
    {
        return Some(StageImpact {
            first_dirty: "S2".into(),
            dirty: downstream(2),
            hint: format!("python {}/rebuild.py", crate::paths::CUT),
            b8_guardrail: false,
        });
    }
    // artboard 卡片:S4 动画卡消费 → 创作素材 rebuild(artboard 子目录名保留英文)
    let artboard_prefixes = [
        format!("{}/{}/", crate::paths::ASSETS, crate::paths::ARTBOARD),
        format!("{}/{}/", crate::paths::LEGACY_ASSETS, crate::paths::ARTBOARD),
    ];
    if artboard_prefixes.iter().any(|p| norm.starts_with(p.as_str())) {
        return Some(StageImpact {
            first_dirty: "S4".into(),
            dirty: downstream(4),
            hint: format!(
                "python {}/{}/rebuild.py",
                crate::paths::ASSETS,
                crate::paths::ARTBOARD
            ),
            b8_guardrail: false,
        });
    }
    // wordline:全片时间真相源(S1 产物)→ S2 起全部重算
    if norm == crate::paths::WORDLINE_REL || norm == crate::paths::LEGACY_WORDLINE_REL {
        return Some(StageImpact {
            first_dirty: "S2".into(),
            dirty: downstream(2),
            hint: "python rebuild.py --from S2".into(),
            b8_guardrail: false,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_change_marks_s3_downstream_only() {
        // 新布局(0.5 中文目录)
        let imp = impact_for(crate::paths::PROJECT_REL).expect("project.json 必须有影响");
        assert_eq!(imp.first_dirty, "S3");
        assert!(!imp.dirty.iter().any(|s| s == "S0" || s == "S1" || s == "S2"), "上游不得标脏");
        assert_eq!(imp.dirty.len(), 9, "S3..S11 共 9 个阶段");
        assert_eq!(imp.dirty.first().unwrap(), "S3");
        assert_eq!(imp.dirty.last().unwrap(), "S11");
        assert!(imp.hint.contains("05_时间线工程/rebuild.py"));
        // 旧布局(0.4.x 英文目录)同样标脏(兼容伺候旧工程)
        let legacy = impact_for(crate::paths::LEGACY_PROJECT_REL).expect("旧布局 project.json 必须有影响");
        assert_eq!(legacy.first_dirty, "S3");
        assert!(legacy.hint.contains("05_时间线工程/rebuild.py"), "提示串一律指新目录");
    }

    #[test]
    fn subtitle_ass_marks_only_s8() {
        let imp = impact_for(&format!("{}/subtitles.ass", crate::paths::OUTPUT)).expect("subtitles.ass 必须有影响");
        assert_eq!(imp.dirty, vec!["S8".to_string()], "S8 护栏:只重烧,不碰 IR 阶段");
        assert!(!imp.dirty.iter().any(|s| s == "S3" || s == "S7" || s == "S9"));
        assert!(imp.hint.contains("06_成片输出/rebuild.py"));
        let legacy = impact_for("06_output/subtitles.ass").unwrap();
        assert_eq!(legacy.dirty, vec!["S8".to_string()], "旧布局同口径");
    }

    #[test]
    fn cutlist_and_artboard_and_wordline() {
        let cutlist = impact_for(crate::paths::CUTLIST_APPLIED_REL).unwrap();
        assert_eq!(cutlist.first_dirty, "S2");
        assert!(cutlist.hint.contains("04_粗剪决策/rebuild.py"));
        let legacy_cutlist = impact_for("04_cut/cutlist.applied.json").unwrap();
        assert_eq!(legacy_cutlist.first_dirty, "S2", "旧布局同口径");

        let card = impact_for(&format!("{}/artboard/cards/kc01.mp4", crate::paths::ASSETS)).unwrap();
        assert_eq!(card.first_dirty, "S4");
        assert!(card.hint.contains("03_创作素材/artboard/rebuild.py"));
        let legacy_card = impact_for("03_assets/artboard/cards/kc01.mp4").unwrap();
        assert_eq!(legacy_card.first_dirty, "S4", "旧布局同口径");

        let wl = impact_for(crate::paths::WORDLINE_REL).unwrap();
        assert_eq!(wl.first_dirty, "S2");
    }

    #[test]
    fn unknown_file_is_neutral() {
        assert!(impact_for(&format!("{}/brief.md", crate::paths::BRIEF)).is_none());
        assert!(impact_for("00_brief/brief.md").is_none(), "旧布局简报同样不标脏");
        assert!(impact_for(".cutforge/rev").is_none());
        assert!(impact_for(&format!("{}/final_x_916.mp4", crate::paths::OUTPUT)).is_none(), "成片产物不标脏");
        assert!(impact_for("06_output/final_x_916.mp4").is_none());
        assert!(impact_for("01_materials/a.mp4").is_none());
        assert!(impact_for("01_原始素材/a.mp4").is_none());
    }
}
