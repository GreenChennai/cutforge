// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 批量命令的 mutate 实现(册四 A4 T4.7;apply.rs 行数红线,本模块承载两个
//! 批量变体,语义与单片段命令逐一同源):
//! - `ClipsInsert`:批量插入片段,单 Op 原子——任一 id 重复 / 落点重叠则整批拒绝
//!   (subtitle_import 的原子性根基:半批落盘比失败更糟);
//! - `ClipsPatch`:批量按字段合并 patch,单 Op 原子——任一 clipId 不存在则整批拒绝
//!   (subtitle_replace 批量替换;未来"批量变速/音量"复用同一通道)。
//!
//! R-12①:失败面统一携带 Revert 回滚令牌(触达轨的 clips 变更前值),
//! 由 Engine::apply 逆向写回,与单片段命令同一回滚口径。

use super::Reject;
use super::invariants::enforce_no_overlap;
use super::revert::{MutateError, Mutated, Revert, fail_with};
use crate::command::{ClipPatch, Command};
use crate::engine::Engine;
use crate::oplog::OpKind;
use serde_json::Value;

/// mutate 的统一返回面——与 apply.rs 同形(R-12① 起为 Mutated/MutateError)。
type MutateOutcome = Result<Mutated, MutateError>;

impl Engine {
    pub(super) fn mutate_clips_insert(
        &mut self,
        to_track: String,
        clips: Vec<crate::model::Clip>,
    ) -> MutateOutcome {
        let p = &mut self.project;
        let ti = p
            .find_track(&to_track)
            .ok_or_else(|| MutateError::bare(Reject::UnknownTrack(to_track.clone())))?;
        for c in &clips {
            if p.tracks
                .iter()
                .any(|t| t.clips.iter().any(|x| x.id == c.id))
            {
                return Err(MutateError::bare(Reject::DuplicateClipId(c.id.clone())));
            }
        }
        let path = format!("/tracks/{ti}/clips");
        let before = serde_json::to_value(&p.tracks[ti].clips).unwrap();
        let revert = Revert::track_clips(p, ti);
        // BUG-05 补口:逐个按 startMs 二分插入(与 clip_insert 同款)——乱序批次
        // 不得 extend 尾部破坏"轨道 startMs 升序"不变量;批内相对次序保持
        for c in &clips {
            let pos = p.tracks[ti]
                .clips
                .partition_point(|x| x.start_ms < c.start_ms);
            p.tracks[ti].clips.insert(pos, c.clone());
        }
        let after = serde_json::to_value(&p.tracks[ti].clips).unwrap();
        enforce_no_overlap(p, ti).map_err(fail_with(&revert))?;
        Ok(Mutated {
            path,
            before,
            after,
            summary: format!(
                "clips_insert {}→{to_track}({} 段)",
                clips.first().map(|c| c.id.as_str()).unwrap_or("-"),
                clips.len()
            ),
            kind: OpKind::Insert,
            target_id: None,
            revert,
        })
    }

    pub(super) fn mutate_clips_patch(&mut self, updates: &[(String, ClipPatch)]) -> MutateOutcome {
        for (clip_id, patch) in updates {
            if patch.is_empty() {
                return Err(MutateError::bare(Reject::EmptyPatch(clip_id.clone())));
            }
        }
        let p = &mut self.project;
        // 先全部定位再改:任一 clipId 不存在 → 整批拒绝(原子)
        let mut locs: Vec<(usize, usize)> = Vec::with_capacity(updates.len());
        for (clip_id, _) in updates {
            let (ti, ci) = p
                .find_clip(clip_id)
                .ok_or_else(|| MutateError::bare(Reject::UnknownClip(clip_id.clone())))?;
            locs.push((ti, ci));
        }
        // Op 面按轨归并:同一轨多次修改 → 该轨 clips 数组一个 before/after
        let mut touched: Vec<usize> = locs.iter().map(|(ti, _)| *ti).collect();
        touched.sort_unstable();
        touched.dedup();
        let before: Vec<(usize, Value)> = touched
            .iter()
            .map(|&ti| (ti, serde_json::to_value(&p.tracks[ti].clips).unwrap()))
            .collect();
        // R-12①:批量回滚令牌 = 各触达轨的 clips 变更前值(条目独立,次序无关)
        let revert = Revert::Multi(
            touched
                .iter()
                .map(|&ti| Revert::track_clips(p, ti))
                .collect(),
        );
        let mut n_changes = 0usize;
        for ((_, patch), (ti, ci)) in updates.iter().zip(locs.iter()) {
            let mut clip = p.tracks[*ti].clips[*ci].clone();
            n_changes += patch.clone().apply_to(&mut clip).len();
            let old_start = p.tracks[*ti].clips[*ci].start_ms;
            p.tracks[*ti].clips[*ci] = clip;
            // BUG-05 补口(与 clip_update 同款):patch 改 startMs 且跨兄弟位置时
            // 原地更新破坏轨道升序不变量 → remove + 二分重插
            if p.tracks[*ti].clips[*ci].start_ms != old_start {
                let moved = p.tracks[*ti].clips.remove(*ci);
                let pos = p.tracks[*ti]
                    .clips
                    .partition_point(|c| c.start_ms < moved.start_ms);
                p.tracks[*ti].clips.insert(pos, moved);
            }
        }
        for &ti in &touched {
            enforce_no_overlap(p, ti).map_err(fail_with(&revert))?;
        }
        // 摘要带首个非空变更示例(批量摘要不逐条展开)
        let after: Vec<(usize, Value)> = touched
            .iter()
            .map(|&ti| (ti, serde_json::to_value(&p.tracks[ti].clips).unwrap()))
            .collect();
        let first_track = touched[0];
        let path = format!("/tracks/{first_track}/clips");
        let before_v = before
            .iter()
            .find(|(ti, _)| *ti == first_track)
            .map(|(_, v)| v.clone())
            .unwrap();
        let after_v = after
            .iter()
            .find(|(ti, _)| *ti == first_track)
            .map(|(_, v)| v.clone())
            .unwrap();
        Ok(Mutated {
            path,
            before: before_v,
            after: after_v,
            summary: format!("clips_patch({} clip,{} 字段)", updates.len(), n_changes),
            kind: OpKind::Set,
            target_id: None,
            revert,
        })
    }

    /// mutate 主分派接入(apply.rs 的 match 末尾两分支委托至此,保持行数红线)。
    pub(super) fn mutate_dispatch_batch(&mut self, cmd: &Command) -> Option<MutateOutcome> {
        match cmd {
            Command::ClipsInsert {
                to_track, clips, ..
            } => Some(self.mutate_clips_insert(to_track.clone(), clips.clone())),
            Command::ClipsPatch { updates } => Some(self.mutate_clips_patch(updates)),
            _ => None,
        }
    }
}
