// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 专业编辑命令实现(册五 T5.4/ADR-0019;拆分自 engine::apply——行数红线 A1-3,
//! 纯移动):[`Command::CompoundCreate`] / [`Command::CompoundUnbind`] /
//! [`Command::TrackSplitAt`] 的 mutate 体。语义裁决与 Reject 面与拆分前逐字一致
//! (单 Op 原子,enforce_no_overlap 守卫,失败由 Engine::apply 统一回滚快照)。

use super::invariants::{enforce_no_overlap, Reject};
use crate::model::{CompoundSpec, Project};
use crate::oplog::OpKind;
use serde_json::Value;

/// 命令执行产物:(指针, before, after, 摘要, OpKind)——与 Engine::mutate 同形。
pub type Mutated = (String, Value, Value, String, OpKind);

/// clips 指针(与 apply::clips_pointer 同式;可见性桥)。
pub fn clips_pointer_pub(ti: usize) -> String {
    format!("/tracks/{ti}/clips")
}


// ---- 复合片段打包(册五 T5.4/ADR-0019;单 Op 原子) ----
#[allow(clippy::too_many_lines)]
pub fn compound_create(
    p: &mut Project,
    clip_ids: Vec<String>,
    to_track: String,
    start_ms: u64,
) -> Result<Mutated, Reject> {
    {
                if clip_ids.len() < 2 {
                    return Err(Reject::InvariantViolation(format!(
                        "compound_create 至少选 2 个片段(收到 {})",
                        clip_ids.len()
                    )));
                }
                let ti = p.find_track(&to_track).ok_or(Reject::UnknownTrack(to_track.clone()))?;
                if p.tracks[ti].kind != crate::model::TrackKind::Video {
                    return Err(Reject::InvariantViolation(format!(
                        "复合片段只能落视频轨(目标 {to_track} 是 {})",
                        p.tracks[ti].kind.kind_json()
                    )));
                }
                // 选中片段收集(全部必须存在;全部视频轨;无复合嵌套)
                let mut picked: Vec<crate::model::Clip> = Vec::with_capacity(clip_ids.len());
                for id in &clip_ids {
                    let (src_ti, ci) = p.find_clip(id).ok_or(Reject::UnknownClip(id.clone()))?;
                    if p.tracks[src_ti].kind != crate::model::TrackKind::Video {
                        return Err(Reject::InvariantViolation(format!(
                            "复合打包只收视频片段({id} 在 {} 轨)",
                            p.tracks[src_ti].kind.kind_json()
                        )));
                    }
                    let c = &p.tracks[src_ti].clips[ci];
                    if c.compound.is_some() {
                        return Err(Reject::InvariantViolation(format!(
                            "{id} 已是复合片段(深度上限两级,拒绝三层;编辑请先解包)"
                        )));
                    }
                    picked.push(c.clone());
                }
                picked.sort_by_key(|c| (c.start_ms, c.id.clone()));
                // 两两不重叠且首尾相接(子时间线单轨 concat 语义,与模型校验同契约)
                for w in picked.windows(2) {
                    let end = w[0].start_ms + w[0].duration_ms;
                    if w[1].start_ms < end {
                        return Err(Reject::InvariantViolation(format!(
                            "复合选区重叠: {}[{}, {}) 与 {}[{}, {})",
                            w[0].id, w[0].start_ms, end, w[1].id, w[1].start_ms, w[1].start_ms + w[1].duration_ms
                        )));
                    }
                    if w[1].start_ms > end {
                        return Err(Reject::InvariantViolation(format!(
                            "复合选区须首尾相接(先闭合间隙): {} 终点 {end}ms 与 {} 起点 {}ms 之间有间隙",
                            w[0].id, w[1].id, w[1].start_ms
                        )));
                    }
                }
                let base = picked[0].start_ms;
                let inner: Vec<crate::model::Clip> = picked
                    .iter()
                    .map(|c| {
                        let mut c = c.clone();
                        c.start_ms -= base; // 全局 → 局部时间域
                        c
                    })
                    .collect();
                let duration = inner.iter().map(|c| c.start_ms + c.duration_ms).max().unwrap_or(0);
                let shell = crate::model::Clip {
                    id: Project::next_clip_id(&p.tracks[ti]),
                    source_hash: None,
                    src: None,
                    start_ms,
                    duration_ms: duration,
                    source_in_ms: None,
                    speed: None,
                    speed_curve: None,
                    reverse: None,
                    rotation: None,
                    crop: None,
                    flip: None,
                    volume: None,
                    role: None,
                    text: None,
                    text_style: None,
                    huazi: None,
                    font: None,
                    denoise: None,
                    pitch: None,
                    position: None,
                    scale: None,
                    reframe: None,
                    motion: None,
                    transition: None,
                    overlay: None,
                    opacity: None,
                    fade: None,
                    loop_: None,
                    punch_in: None,
                    freeze_ms: None,
                    fx: None,
                    keyframes: None,
                    grade: None,
                    compound: Some(CompoundSpec { canvas: None, clips: inner }),
                };
                let path = "/tracks".to_string();
                let before = serde_json::to_value(&p.tracks).unwrap();
                for id in &clip_ids {
                    let (src_ti, ci) = p.find_clip(id).expect("上文已确认存在");
                    p.tracks[src_ti].clips.remove(ci);
                }
                p.tracks[ti].clips.push(shell.clone());
                let after = serde_json::to_value(&p.tracks).unwrap();
                enforce_no_overlap(p, ti)?;
                Ok((path, before, after,
                    format!("compound_create {}→{to_track}@{start_ms}ms({} 段打包)", shell.id, clip_ids.len()),
                    OpKind::Insert))
            }
}

// ---- 复合片段解包(册五 T5.4/ADR-0019;单 Op 原子;打包的逆操作) ----
pub fn compound_unbind(p: &mut Project, clip_id: String) -> Result<Mutated, Reject> {
    {
                let (ti, ci) = p.find_clip(&clip_id).ok_or(Reject::UnknownClip(clip_id.clone()))?;
                let shell = &p.tracks[ti].clips[ci];
                let Some(cp) = &shell.compound else {
                    return Err(Reject::InvariantViolation(format!(
                        "{clip_id} 不是复合片段(无 compound 字段)"
                    )));
                };
                if p.tracks[ti].kind != crate::model::TrackKind::Video {
                    return Err(Reject::InvariantViolation(format!(
                        "复合片段只能在视频轨解包({clip_id} 在 {} 轨)",
                        p.tracks[ti].kind.kind_json()
                    )));
                }
                // 探针轨本地推进 id 分配(与 subtitle_import 同口径;不触工作区)
                let mut probe = p.tracks[ti].clone();
                probe.clips = Vec::new();
                let mut restored: Vec<crate::model::Clip> = Vec::with_capacity(cp.clips.len());
                let mut inner: Vec<&crate::model::Clip> = cp.clips.iter().collect();
                inner.sort_by_key(|c| (c.start_ms, c.id.clone()));
                for c in inner {
                    let mut c = c.clone();
                    c.id = Project::next_clip_id(&probe);
                    c.start_ms += shell.start_ms; // 局部 → 全局时间域
                    probe.clips.push(c.clone());
                    restored.push(c);
                }
                let path = clips_pointer_pub(ti);
                let before = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                let n_restored = restored.len();
                p.tracks[ti].clips.remove(ci);
                for (offset, c) in restored.into_iter().enumerate() {
                    p.tracks[ti].clips.insert(ci + offset, c);
                }
                let after = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                enforce_no_overlap(p, ti)?;
                Ok((path, before, after,
                    format!("compound_unbind {clip_id}({n_restored} 段还原)"),
                    OpKind::Split))
            }
}

// ---- 单轨多点分割(册五 T5.4 scene_detect 自动切段;单 Op 原子) ----
pub fn track_split_at(p: &mut Project, track_id: String, t_points: Vec<u64>) -> Result<Mutated, Reject> {
    {
                let ti = p.find_track(&track_id).ok_or(Reject::UnknownTrack(track_id.clone()))?;
                let path = clips_pointer_pub(ti);
                let before = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                let mut points = t_points.clone();
                points.sort_unstable();
                points.dedup();
                let mut n = 0usize;
                for t_ms in points {
                    // 先收集命中下标再改,避免插队导致的下标漂移
                    let hits: Vec<usize> = p.tracks[ti].clips.iter().enumerate()
                        .filter(|(_, c)| c.start_ms < t_ms && t_ms < c.start_ms + c.duration_ms)
                        .map(|(i, _)| i)
                        .collect();
                    for (offset, i) in hits.into_iter().enumerate() {
                        let ci = i + offset;
                        let start = p.tracks[ti].clips[ci].start_ms;
                        let end = start + p.tracks[ti].clips[ci].duration_ms;
                        let src_shift = t_ms - start;
                        let mut right = p.tracks[ti].clips[ci].clone();
                        right.id = Project::next_clip_id(&p.tracks[ti]);
                        right.start_ms = t_ms;
                        right.duration_ms = end - t_ms;
                        right.source_in_ms = right.source_in_ms.map(|s| s + src_shift);
                        right.text = None; // 文本归属左段(与 clip_split 同语义)
                        p.tracks[ti].clips[ci].duration_ms = t_ms - start;
                        p.tracks[ti].clips.insert(ci + 1, right);
                        n += 1;
                    }
                }
                if n == 0 {
                    return Err(Reject::InvariantViolation(format!(
                        "{track_id} 轨上无片段严格包含任何切点({t_points:?}),无切可分"
                    )));
                }
                let after = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                Ok((path, before, after,
                    format!("track_split_at {track_id}@{t_points:?}({n} 段)"),
                    OpKind::Split))
    }
}
