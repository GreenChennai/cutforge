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
    SplitOutside {
        clip_id: String,
        t_ms: u64,
    },
    NotAdjacent {
        left_id: String,
        right_id: String,
    },
    DuplicateClipId(String),
    EmptyPatch(String),
    PreconditionFailed {
        expected: u64,
        actual: u64,
    },
    SchemaInvalid(Vec<String>),
    InvariantViolation(String),
    /// bgm_set 要求工程已有 doc.bgm,或 patch 携带 src(新建须有音源)。
    MissingBgm,
    NothingToUndo,
    NothingToRedo,
    /// OpLog 回放 rev 链断链(BUG-07):expected = 顺推期望 rev,got = 实际遇到的
    /// rev,op_id = 断链处第一条 Op。open 层应把本错误冒泡为「工程需修复」的
    /// 用户可见状态(与 R-03 半行截断同路),而非 open 失败。
    RevGap {
        expected: u64,
        got: u64,
        op_id: String,
    },
}

/// BUG-03:roll/slide 的 sourceIn 平移守卫(**拒绝式**:负值/溢出拒绝而非钳制,
/// 语义明确、可撤销;工程零变更由 Engine::apply 的快照回滚统一保证)。
pub(super) fn checked_source_in(si: u64, shift: i128, clip_id: &str) -> Result<u64, Reject> {
    let v = si as i128 + shift;
    if v < 0 {
        return Err(Reject::InvariantViolation(format!(
            "越出素材入点: sourceIn {v}ms < 0({clip_id}),拒绝式守卫拦截,工程零变更"
        )));
    }
    u64::try_from(v)
        .map_err(|_| Reject::InvariantViolation(format!("sourceIn 溢出 u64: {v}({clip_id})")))
}

/// 同轨时间重叠校验(可见性 pub(super):供 apply 子模块的 mutate 调用)。
pub(super) fn enforce_no_overlap(p: &Project, ti: usize) -> Result<(), Reject> {
    let ov = Project::overlaps(&p.tracks[ti]);
    if ov.is_empty() {
        Ok(())
    } else {
        Err(Reject::InvariantViolation(format!(
            "同轨时间重叠(CF-004): {:?}",
            ov
        )))
    }
}
