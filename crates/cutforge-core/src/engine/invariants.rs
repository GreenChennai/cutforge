// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 不变量与拒绝原因:引擎写路径的守门面。
//! 冲突码(CF-001~006)在 M3 的合并器中细化,此处承载同轨重叠等硬约束。

use crate::model::Project;

/// 撤销/重做以外的拒绝原因;冲突码(CF-001~006)在 M3 的合并器中细化,
/// 此处 InvariantViolation 预留 CF-004(同轨时间重叠)。
#[derive(Debug, Clone, PartialEq)]
pub enum Reject {
    UnknownClip(String),
    UnknownTrack(String),
    UnknownOp(String),
    SplitOutside { clip_id: String, t_ms: u64 },
    NotAdjacent { left_id: String, right_id: String },
    DuplicateClipId(String),
    EmptyPatch(String),
    PreconditionFailed { expected: u64, actual: u64 },
    SchemaInvalid(Vec<String>),
    InvariantViolation(String),
    /// bgm_set 要求工程已有 doc.bgm,或 patch 携带 src(新建须有音源)。
    MissingBgm,
    NothingToUndo,
    NothingToRedo,
}

/// 同轨时间重叠校验(可见性 pub(super):供 apply 子模块的 mutate 调用)。
pub(super) fn enforce_no_overlap(p: &Project, ti: usize) -> Result<(), Reject> {
    let ov = Project::overlaps(&p.tracks[ti]);
    if ov.is_empty() {
        Ok(())
    } else {
        Err(Reject::InvariantViolation(format!(
            "同轨时间重叠(CF-004): {:?}", ov)))
    }
}
