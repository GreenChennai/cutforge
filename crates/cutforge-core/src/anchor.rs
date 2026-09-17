//! 锚点模型(计划书 1.2/3.6/4.9):标注指向时间轴上某个点或某个元素的稳定引用。
//! 重定位规则(3.6,禁止静默丢弃):
//! 1. 元素仍在、仅时间位移 → 锚点跟随(保持相对偏移),relocated=true;
//! 2. id 消失但有 ≤nearest_ms 邻近元素 → 重挂到最近元素,relocated=true;
//! 3. id 消失且无邻近元素 → state=orphan(编辑器"孤儿标注"面板可见)。

use crate::model::Project;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnchorKind {
    Clip,
    Track,
    Time,
    Word,
    SubtitleCard,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    pub kind: AnchorKind,
    /// 目标对象的稳定 id;kind=Time 时为 null。
    pub ref_: Option<String>,
    pub t_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorState {
    /// 锚点原样命中。
    Resolved,
    /// 跟随或重挂成功。
    Relocated,
    /// 无法重定位(必须显式保留,不得丢弃)。
    Orphan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Relocation {
    pub state: AnchorState,
    pub anchor: Anchor,
    /// 转为 orphan 时的原因(写进 notes.json 的 orphanReason)。
    pub orphan_reason: Option<String>,
}

/// 缺省邻近阈值 500ms(计划书 3.6,可配)。
pub const DEFAULT_NEAREST_MS: u64 = 500;

/// 解析锚点在工程中的当前指向;kind=Time/Word 恒 Resolved(Time 只报时间,Word 属 wordline 域)。
pub fn resolve(project: &Project, anchor: &Anchor) -> Option<String> {
    match anchor.kind {
        AnchorKind::Clip | AnchorKind::SubtitleCard => project
            .find_clip(anchor.ref_.as_deref()?)
            .map(|(ti, ci)| format!("/tracks/{ti}/clips/{ci}")),
        AnchorKind::Track => {
            let id = anchor.ref_.as_deref()?;
            project.find_track(id).map(|ti| format!("/tracks/{ti}"))
        }
        AnchorKind::Time | AnchorKind::Word => None,
    }
}

/// 重定位(3.6 规则表):元素位移→跟随;消失→≤nearest_ms 重挂最近;否则 orphan。
pub fn relocate(project: &Project, anchor: &Anchor, nearest_ms: u64) -> Relocation {
    match anchor.kind {
        AnchorKind::Time | AnchorKind::Word => {
            return Relocation { state: AnchorState::Resolved, anchor: anchor.clone(), orphan_reason: None };
        }
        AnchorKind::Track => {
            if let Some(id) = anchor.ref_.as_deref() {
                if project.find_track(id).is_some() {
                    return Relocation { state: AnchorState::Resolved, anchor: anchor.clone(), orphan_reason: None };
                }
            }
            return Relocation {
                state: AnchorState::Orphan,
                anchor: anchor.clone(),
                orphan_reason: Some(format!("轨道 {:?} 已不存在", anchor.ref_)),
            };
        }
        AnchorKind::Clip | AnchorKind::SubtitleCard => {}
    }
    let Some(id) = anchor.ref_.clone() else {
        return Relocation {
            state: AnchorState::Orphan,
            anchor: anchor.clone(),
            orphan_reason: Some("锚点缺少目标 id".into()),
        };
    };
    // 规则 1:元素仍在(可能整体位移)→ 跟随;点锚点吸附进元素当前范围
    if let Some((ti, ci)) = project.find_clip(&id) {
        let clip = &project.tracks[ti].clips[ci];
        let mut relocated = anchor.clone();
        relocated.t_ms = anchor.t_ms.clamp(clip.start_ms, clip.start_ms + clip.duration_ms);
        return Relocation { state: AnchorState::Relocated, anchor: relocated, orphan_reason: None };
    }
    // 规则 2:id 消失,找 ≤nearest_ms 的最近片段重挂(片段区间内距离记 0)
    let mut best: Option<(u64, (usize, usize))> = None;
    for (ti, t) in project.tracks.iter().enumerate() {
        for (ci, c) in t.clips.iter().enumerate() {
            let end = c.start_ms + c.duration_ms;
            let dist = if anchor.t_ms < c.start_ms {
                c.start_ms - anchor.t_ms
            } else if anchor.t_ms > end {
                anchor.t_ms - end
            } else {
                0
            };
            if dist <= nearest_ms && best.map(|(d, _)| dist < d).unwrap_or(true) {
                best = Some((dist, (ti, ci)));
            }
        }
    }
    if let Some((_, (ti, ci))) = best {
        let clip = &project.tracks[ti].clips[ci];
        let mut relocated = anchor.clone();
        relocated.ref_ = Some(clip.id.clone());
        relocated.t_ms = anchor.t_ms.clamp(clip.start_ms, clip.start_ms + clip.duration_ms);
        return Relocation { state: AnchorState::Relocated, anchor: relocated, orphan_reason: None };
    }
    // 规则 3:orphan(显式保留,不删除)
    Relocation {
        state: AnchorState::Orphan,
        anchor: anchor.clone(),
        orphan_reason: Some(format!("id {id} 已消失且 {nearest_ms}ms 内无邻近片段")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::sample_project;

    #[test]
    fn resolve_clip_track_time() {
        let p = sample_project();
        let a = Anchor { kind: AnchorKind::Clip, ref_: Some("V1-002".into()), t_ms: 9000, span: None };
        assert_eq!(resolve(&p, &a), Some("/tracks/0/clips/1".into()));
        let t = Anchor { kind: AnchorKind::Track, ref_: Some("A1".into()), t_ms: 0, span: None };
        assert_eq!(resolve(&p, &t), Some("/tracks/1".into()));
        let time = Anchor { kind: AnchorKind::Time, ref_: None, t_ms: 12340, span: None };
        assert_eq!(resolve(&p, &time), None);
    }

    #[test]
    fn relocate_follows_moved_clip() {
        let mut p = sample_project();
        // V1-002 整体后移 1000ms(模拟 AI 改了上游时长)
        p.tracks[0].clips[1].start_ms = 9400;
        let anchor = Anchor { kind: AnchorKind::Clip, ref_: Some("V1-002".into()), t_ms: 9000, span: None };
        let r = relocate(&p, &anchor, DEFAULT_NEAREST_MS);
        assert_eq!(r.state, AnchorState::Relocated);
        assert_eq!(r.anchor.ref_.as_deref(), Some("V1-002"));
        assert_eq!(r.anchor.t_ms, 9400, "点锚点吸附进元素范围");
    }

    #[test]
    fn relocate_rehangs_to_nearest_then_orphans() {
        let mut p = sample_project();
        p.tracks[0].clips.remove(1); // V1-002 消失(V1-001 = 0..8400,A1 = 8400..8800)
        let near = Anchor { kind: AnchorKind::Clip, ref_: Some("V1-002".into()), t_ms: 8600, span: None };
        let r = relocate(&p, &near, DEFAULT_NEAREST_MS);
        assert_eq!(r.state, AnchorState::Relocated);
        assert_eq!(r.anchor.ref_.as_deref(), Some("A1-001"), "8600ms 距 A1[8400..8800] 距离 0,重挂最近");
        assert_eq!(r.anchor.t_ms, 8600);
        // 20000ms 处(全部片段之后)500ms 内无任何片段 → orphan(显式保留)
        let far = Anchor { kind: AnchorKind::Clip, ref_: Some("V1-002".into()), t_ms: 20000, span: None };
        let r2 = relocate(&p, &far, DEFAULT_NEAREST_MS);
        assert_eq!(r2.state, AnchorState::Orphan);
        assert!(r2.orphan_reason.unwrap().contains("V1-002"));
    }
}
