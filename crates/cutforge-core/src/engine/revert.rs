// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! mutate 的产物面与回滚令牌(R-12①:apply 去全量 clone)。
//!
//! `mutate` 就地变更前捕获"将被触达的容器"的 before 值(与 Op 面的 before
//! 同源同刻),失败/拒绝/幂等时按令牌**逆向写回**——恢复的是 Rust 值本身,
//! 逐字节等价于原态(TC-CORE-APPLY-099),成本 O(受影响轨) 而非 O(全工程)。

use super::invariants::Reject;
use crate::model::{Bgm, Clip, Project, Track};
use crate::oplog::OpKind;
use serde_json::Value;

/// 回滚令牌:命令触达容器的变更前值(逆向回滚的最小充分集)。
#[derive(Debug, Clone)]
pub(super) enum Revert {
    /// 命令在任何修改前失败:无可回滚面。
    None,
    /// 恢复 ti 轨的 clips 数组(单轨增删/改值/重排类命令)。
    TrackClips { ti: usize, clips: Vec<Clip> },
    /// 恢复整条轨(track_update:轨字段 + clips 一起还原)。
    Track { ti: usize, track: Box<Track> },
    /// 恢复全部轨(跨轨 move、全轨 split_all、compound_create 等多轨命令)。
    Tracks(Vec<Track>),
    /// 撤销 track_add 的尾插(弹出该轨)。
    PopTrack,
    /// 恢复 bgm(bgm_set/bgm_clear;None = 变更前无 bgm)。
    Bgm(Option<Bgm>),
    /// 多容器批量(clips_patch 跨多轨;逐条独立还原,次序无关)。
    Multi(Vec<Revert>),
}

impl Revert {
    /// 捕获:ti 轨 clips 数组的变更前值。
    pub(super) fn track_clips(p: &Project, ti: usize) -> Self {
        Revert::TrackClips {
            ti,
            clips: p.tracks[ti].clips.clone(),
        }
    }

    /// 捕获:整条轨的变更前值。
    pub(super) fn track(p: &Project, ti: usize) -> Self {
        Revert::Track {
            ti,
            track: Box::new(p.tracks[ti].clone()),
        }
    }

    /// 捕获:全部轨的变更前值。
    pub(super) fn tracks(p: &Project) -> Self {
        Revert::Tracks(p.tracks.clone())
    }

    /// 捕获:bgm 的变更前值。
    pub(super) fn bgm(p: &Project) -> Self {
        Revert::Bgm(p.bgm.clone())
    }

    /// 逆向写回:把捕获值原样赋回(无重排、无归一化——逐字节还原的依据)。
    /// 多容器条目相互独立,任意次序等价;按逆序折叠以明确"逆向"语义。
    pub(super) fn restore(self, p: &mut Project) {
        match self {
            Revert::None => {}
            Revert::TrackClips { ti, clips } => p.tracks[ti].clips = clips,
            Revert::Track { ti, track } => p.tracks[ti] = *track,
            Revert::Tracks(tracks) => p.tracks = tracks,
            Revert::PopTrack => {
                p.tracks.pop();
            }
            Revert::Bgm(v) => p.bgm = v,
            Revert::Multi(list) => {
                for r in list.into_iter().rev() {
                    r.restore(p);
                }
            }
        }
    }
}

/// mutate 成功产物:Op 面五元组 + 稳定 id + 回滚令牌。
pub(super) struct Mutated {
    pub path: String,
    pub before: Value,
    pub after: Value,
    pub summary: String,
    pub kind: OpKind,
    pub target_id: Option<String>,
    /// 逆向回滚令牌(R-12①):schema 校验失败 / op_id 撞车 / 幂等无变更时
    /// 由 Engine::apply 据此还原工程。
    pub revert: Revert,
}

/// mutate 失败:拒绝原因 + 回滚令牌(失败可能发生在部分修改之后——
/// enforce_no_overlap / checked_source_in / slide 边界守卫都是改后裁决)。
/// 令牌装盒:错误面保持小体积(clippy::result_large_err;失败路径罕见,
/// 一次 Box 分配可忽略)。
pub(super) struct MutateError {
    pub reject: Reject,
    pub revert: Box<Revert>,
}

impl MutateError {
    /// 修改前失败的便捷构造(无回滚面)。
    pub fn bare(reject: Reject) -> Self {
        Self {
            reject,
            revert: Box::new(Revert::None),
        }
    }
}

/// 构造"携带令牌的失败映射"(供 `.map_err(fail_with(&revert))?`;
/// 令牌仅在失败路径 clone,成功路径零开销)。
pub(super) fn fail_with(revert: &Revert) -> impl Fn(Reject) -> MutateError + '_ {
    move |reject| MutateError {
        reject,
        revert: Box::new(revert.clone()),
    }
}
