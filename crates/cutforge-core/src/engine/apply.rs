// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 命令接口:唯一写入口(计划书 2.6)。`apply`/`record_file_change` 负责幂等/
//! 前置检查 → 变更 → schema 验证 → 产出 Op 入 OpLog;`mutate` 承担 Command →
//! (路径, before, after, 摘要) 的就地变更;复合/多轨分割体在 engine::pro_cmds。

use super::invariants::enforce_no_overlap;
use super::{Engine, Reject};
use crate::command::{Command, TrimEdge, TrimMode};
use crate::model::Project;
use crate::oplog::{Actor, Op, OpKind, OpTarget};
use serde_json::Value;

/// apply 选项:op_id/request_id 幂等;caused_by 绑定标注(4.9);expect_rev =
/// baseRev 前置("我基于哪一版改");non_undoable = 自动簿记登记(进审计链
/// 不入撤销栈,ADR-0001)。
#[derive(Debug, Clone, Default)]
pub struct ApplyOpts {
    pub op_id: Option<String>,
    pub request_id: Option<String>,
    pub caused_by: Vec<String>,
    pub summary: Option<String>,
    pub expect_rev: Option<u64>,
    pub non_undoable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpReceipt {
    pub op_ids: Vec<String>,
    pub rev: u64,
    /// true = 同 opId/request_id 重复调用,状态未变(幂等回执)。
    pub idempotent: bool,
}

impl Engine {
    /// 命令接口:唯一写入口。
    pub fn apply(
        &mut self,
        cmd: Command,
        actor: Actor,
        opts: ApplyOpts,
    ) -> Result<OpReceipt, Reject> {
        // 幂等 1:request_id 去重(非幂等写操作,计划书 5.2)
        if let Some(rid) = opts.request_id.as_deref()
            && self.log.has_request_id(rid)
        {
            return Ok(OpReceipt {
                op_ids: Vec::new(),
                rev: self.rev,
                idempotent: true,
            });
        }
        // 前置条件:相对 baseRev 已失效 → 拒绝(不存在静默覆盖)
        if let Some(expect) = opts.expect_rev
            && expect != self.rev
        {
            return Err(Reject::PreconditionFailed {
                expected: expect,
                actual: self.rev,
            });
        }
        // 幂等 2:显式 op_id 已存在 → 原样回执
        if let Some(id) = opts.op_id.as_deref()
            && self.log.ops().iter().any(|o| o.op_id == id)
        {
            return Ok(OpReceipt {
                op_ids: vec![id.to_string()],
                rev: self.rev,
                idempotent: true,
            });
        }

        // 轨道有序不变量(BUG-05):断言口径 = 引擎不得**破坏**既有有序——
        // 变更前已无序的轨(三路合并写回的外部真相)如实放行,不冤枉引擎。
        let order_before: Vec<bool> = self
            .project
            .tracks
            .iter()
            .map(Project::track_order_ok)
            .collect();
        let snapshot = self.project.clone();
        let outcome = self.mutate(cmd.clone());
        let (path, before, after, summary, kind, target_id) = match outcome {
            Ok(o) => o,
            Err(rej) => {
                self.project = snapshot;
                return Err(rej);
            }
        };
        debug_assert!(
            self.project
                .tracks
                .iter()
                .zip(order_before)
                .all(|(t, was)| !was || Project::track_order_ok(t)),
            "engine 变更破坏了轨道 startMs 升序不变量: {:?}",
            self.project
                .tracks
                .iter()
                .map(|t| (t.id.clone(), Project::track_order_ok(t)))
                .collect::<Vec<_>>()
        );
        // 通用幂等:命令未产生任何变化 → 不升 rev、不产 Op,回执标注 idempotent
        if before == after {
            self.project = snapshot;
            return Ok(OpReceipt {
                op_ids: Vec::new(),
                rev: self.rev,
                idempotent: true,
            });
        }
        // 变更后强制 schema 验证(契约优先;失败即回滚)
        if let Err(errs) = self.project.to_validated_value() {
            self.project = snapshot;
            return Err(Reject::SchemaInvalid(errs));
        }

        self.rev += 1;
        let op = Op {
            op_id: opts.op_id.unwrap_or_else(|| self.log.next_op_id()),
            ts: crate::timeutil::now_rfc3339(),
            actor,
            target: OpTarget {
                file: "project.json".into(),
                path,
            },
            op_kind: kind,
            before,
            after,
            base_rev: crate::format_rev(self.rev - 1),
            rev: Some(self.rev),
            target_id,
            caused_by: if opts.caused_by.is_empty() {
                None
            } else {
                Some(opts.caused_by)
            },
            summary: opts.summary.unwrap_or(summary),
            request_id: opts.request_id,
            auto: None,
        };
        let op_id = op.op_id.clone();
        match self.log.push(op) {
            Some(_) => {
                if !opts.non_undoable {
                    self.undo_stack.push(op_id.clone());
                    self.redo_stack.clear();
                }
                Ok(OpReceipt {
                    op_ids: vec![op_id],
                    rev: self.rev,
                    idempotent: false,
                })
            }
            // op_id 撞车(并发分配同一 id):本次不生效,按幂等回执
            None => {
                self.rev -= 1;
                self.project = snapshot;
                Ok(OpReceipt {
                    op_ids: vec![op_id],
                    rev: self.rev,
                    idempotent: true,
                })
            }
        }
    }

    /// 非 project.json 真相源(notes.json 等)的变更登记:进 OpLog 审计链、
    /// 升 rev,但不改工程文档(工程文档只能走 `apply`)。
    // 参数与 Op 字段一一对应(显式契约面),收拢成结构体反而遮蔽字段名。
    #[allow(clippy::too_many_arguments)]
    pub fn record_file_change(
        &mut self,
        file: &str,
        path: &str,
        before: Value,
        after: Value,
        kind: OpKind,
        actor: Actor,
        opts: ApplyOpts,
    ) -> Result<OpReceipt, Reject> {
        const KNOWN: [&str; 5] = [
            "project.json",
            "wordline.json",
            "cutlist.json",
            "cutlist.applied.json",
            "notes.json",
        ];
        if !KNOWN.contains(&file) {
            return Err(Reject::InvariantViolation(format!(
                "未知真相源文件: {file}"
            )));
        }
        if let Some(rid) = opts.request_id.as_deref()
            && self.log.has_request_id(rid)
        {
            return Ok(OpReceipt {
                op_ids: Vec::new(),
                rev: self.rev,
                idempotent: true,
            });
        }
        if let Some(expect) = opts.expect_rev
            && expect != self.rev
        {
            return Err(Reject::PreconditionFailed {
                expected: expect,
                actual: self.rev,
            });
        }
        if before == after {
            return Ok(OpReceipt {
                op_ids: Vec::new(),
                rev: self.rev,
                idempotent: true,
            });
        }
        self.rev += 1;
        let op = Op {
            op_id: opts.op_id.unwrap_or_else(|| self.log.next_op_id()),
            ts: crate::timeutil::now_rfc3339(),
            actor,
            target: OpTarget {
                file: file.to_string(),
                path: path.to_string(),
            },
            op_kind: kind,
            before,
            after: after.clone(),
            base_rev: crate::format_rev(self.rev - 1),
            rev: Some(self.rev),
            target_id: None,
            caused_by: if opts.caused_by.is_empty() {
                None
            } else {
                Some(opts.caused_by)
            },
            summary: opts.summary.unwrap_or_else(|| format!("{file} 变更")),
            request_id: opts.request_id,
            auto: if opts.non_undoable { Some(true) } else { None },
        };
        if file != "project.json" {
            self.file_states.insert(file.to_string(), after);
            self.dirty_files.insert(file.to_string());
        }
        let op_id = op.op_id.clone();
        match self.log.push(op) {
            Some(_) => {
                if !opts.non_undoable {
                    self.undo_stack.push(op_id.clone());
                    self.redo_stack.clear();
                }
                Ok(OpReceipt {
                    op_ids: vec![op_id],
                    rev: self.rev,
                    idempotent: false,
                })
            }
            None => {
                self.rev -= 1;
                Ok(OpReceipt {
                    op_ids: vec![op_id],
                    rev: self.rev,
                    idempotent: true,
                })
            }
        }
    }

    /// 命令 → (路径, before, after, 摘要, Op 类型, target_id)。变更就地生效;
    /// 失败时调用方回滚快照。target_id = clip 对象指针 Op 的稳定 id 寻址记录
    /// (BUG-06;撤销/重做/回放按 id 定位,对下标漂移免疫)。
    /// 批量命令(ClipsInsert/ClipsPatch,册四 T4.7)委托 engine::batch(行数红线)。
    #[allow(clippy::type_complexity)]
    fn mutate(
        &mut self,
        cmd: Command,
    ) -> Result<(String, Value, Value, String, OpKind, Option<String>), Reject> {
        if let Some(outcome) = self.mutate_dispatch_batch(&cmd) {
            return outcome;
        }
        let p = &mut self.project;
        match cmd {
            Command::ClipUpdate { clip_id, patch } => {
                if patch.is_empty() {
                    return Err(Reject::EmptyPatch(clip_id));
                }
                let (ti, ci) = p
                    .find_clip(&clip_id)
                    .ok_or(Reject::UnknownClip(clip_id.clone()))?;
                // Op 面保持 clip 对象指针(撤销/回放经 target_id 稳定寻址,对数组
                // 重排与外部漂移免疫);数组序由 write_project_op 写回后归位重排,
                // 与 mutate 内的重插同规则(BUG-05 补口)。
                let path = clip_pointer(ti, ci);
                let before = serde_json::to_value(&p.tracks[ti].clips[ci]).unwrap();
                let mut clip = p.tracks[ti].clips[ci].clone();
                let changes = patch.apply_to(&mut clip);
                let summary = if changes.is_empty() {
                    "无实际变更".to_string()
                } else {
                    changes
                        .iter()
                        .map(|(ptr, o, n)| format!("{ptr}: {o}→{n}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let old_start = p.tracks[ti].clips[ci].start_ms;
                p.tracks[ti].clips[ci] = clip;
                // after 必须在重排**前**取:重排后 ci 处已是别的 clip(顺序陷阱)
                let after = serde_json::to_value(&p.tracks[ti].clips[ci]).unwrap();
                if p.tracks[ti].clips[ci].start_ms != old_start {
                    let moved = p.tracks[ti].clips.remove(ci);
                    let pos = p.tracks[ti]
                        .clips
                        .partition_point(|c| c.start_ms < moved.start_ms);
                    p.tracks[ti].clips.insert(pos, moved);
                }
                enforce_no_overlap(p, ti)?;
                Ok((
                    path,
                    before,
                    after,
                    format!("clip_update {clip_id}: {summary}"),
                    OpKind::Set,
                    Some(clip_id),
                ))
            }
            Command::ClipSplit { clip_id, t_ms } => {
                let (ti, ci) = p
                    .find_clip(&clip_id)
                    .ok_or(Reject::UnknownClip(clip_id.clone()))?;
                // 幂等语义(计划书 5.2:已切则返回既有结果):切点边界已存在 → 无操作
                if p.tracks[ti]
                    .clips
                    .iter()
                    .any(|c| c.start_ms == t_ms && c.id != clip_id)
                {
                    let arr = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                    let before = arr.clone();
                    return Ok((
                        clips_pointer(ti),
                        before,
                        arr,
                        format!("clip_split {clip_id}@{t_ms}(已切,幂等)"),
                        OpKind::Split,
                        None,
                    ));
                }
                let path = clips_pointer(ti);
                let before = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                // 切分语义收口 Clip::split_at(BUG-01/02:sourceIn 走 speed 积分、
                // 关键帧/曲线 rebase、fade/transition 归属);engine 只补 id 与落位
                let right_id = Project::next_clip_id(&p.tracks[ti]);
                let (left, mut right) =
                    clip_ref(p, ti, ci)
                        .split_at(t_ms)
                        .map_err(|e| Reject::SplitOutside {
                            clip_id: e.clip_id,
                            t_ms: e.t_ms,
                        })?;
                right.id = right_id;
                p.tracks[ti].clips[ci] = left;
                p.tracks[ti].clips.insert(ci + 1, right);
                let after = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                Ok((
                    path,
                    before.clone(),
                    after,
                    format!("clip_split {clip_id}@{t_ms}"),
                    OpKind::Split,
                    None,
                ))
            }
            Command::ClipDelete { clip_id } => {
                let (ti, ci) = p
                    .find_clip(&clip_id)
                    .ok_or(Reject::UnknownClip(clip_id.clone()))?;
                let path = clips_pointer(ti);
                let before = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                p.tracks[ti].clips.remove(ci);
                let after = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                Ok((
                    path,
                    before,
                    after,
                    format!("clip_delete {clip_id}"),
                    OpKind::Delete,
                    None,
                ))
            }
            Command::ClipMove {
                clip_id,
                new_start_ms,
                to_track,
            } => match to_track {
                None => {
                    let (ti, ci) = p
                        .find_clip(&clip_id)
                        .ok_or(Reject::UnknownClip(clip_id.clone()))?;
                    let path = clips_pointer(ti);
                    let before = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                    // 同轨移动 = remove + 按 startMs 二分重插(BUG-05:轨道有序
                    // 不变量;Op 面为整轨 clips 数组,撤销/回放逐字节还原数组序)
                    let mut clip = p.tracks[ti].clips.remove(ci);
                    clip.start_ms = new_start_ms;
                    let pos = p.tracks[ti]
                        .clips
                        .partition_point(|c| c.start_ms < new_start_ms);
                    p.tracks[ti].clips.insert(pos, clip);
                    let after = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                    enforce_no_overlap(p, ti)?;
                    Ok((
                        path,
                        before,
                        after,
                        format!("clip_move {clip_id}→{new_start_ms}ms"),
                        OpKind::Move,
                        None,
                    ))
                }
                Some(track_id) => {
                    let ti = p
                        .find_track(&track_id)
                        .ok_or(Reject::UnknownTrack(track_id.clone()))?;
                    let (src_ti, ci) = p
                        .find_clip(&clip_id)
                        .ok_or(Reject::UnknownClip(clip_id.clone()))?;
                    if p.tracks[src_ti].kind != p.tracks[ti].kind {
                        return Err(Reject::InvariantViolation(format!(
                            "跨轨移动必须同 kind:{}→{}",
                            p.tracks[src_ti].kind.kind_json(),
                            p.tracks[ti].kind.kind_json()
                        )));
                    }
                    let path = "/tracks".to_string();
                    let before = serde_json::to_value(&p.tracks).unwrap();
                    // BUG-05:clip id 是 Op target/锚点/缓存的身份基础——移动轨道是
                    // 位置变更不是身份变更,保 id;目标轨按 startMs 二分插入保有序
                    let mut clip = p.tracks[src_ti].clips.remove(ci);
                    clip.start_ms = new_start_ms;
                    let pos = p.tracks[ti]
                        .clips
                        .partition_point(|c| c.start_ms < new_start_ms);
                    p.tracks[ti].clips.insert(pos, clip);
                    let after = serde_json::to_value(&p.tracks).unwrap();
                    enforce_no_overlap(p, ti)?;
                    Ok((
                        path,
                        before,
                        after,
                        format!("clip_move {clip_id}→{track_id}@{new_start_ms}ms"),
                        OpKind::Move,
                        None,
                    ))
                }
            },
            Command::ClipInsert { to_track, clip, .. } => {
                let ti = p
                    .find_track(&to_track)
                    .ok_or(Reject::UnknownTrack(to_track.clone()))?;
                if p.tracks
                    .iter()
                    .any(|t| t.clips.iter().any(|c| c.id == clip.id))
                {
                    return Err(Reject::DuplicateClipId(clip.id));
                }
                let path = clips_pointer(ti);
                let before = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                // BUG-05 补口:按 startMs 二分插入(与 clip_move 同款)——text_add 等
                // 早位置插入不得 push 尾部破坏"轨道 startMs 升序"不变量
                let pos = p.tracks[ti]
                    .clips
                    .partition_point(|c| c.start_ms < clip.start_ms);
                p.tracks[ti].clips.insert(pos, clip.clone());
                let after = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                enforce_no_overlap(p, ti)?;
                Ok((
                    path,
                    before,
                    after,
                    format!("clip_insert {}→{to_track}", clip.id),
                    OpKind::Insert,
                    None,
                ))
            }
            Command::TrackAdd {
                kind,
                request_id: _,
            } => {
                let id = p.next_track_id(kind);
                let path = "/tracks".to_string();
                let before = serde_json::to_value(&p.tracks).unwrap();
                p.tracks.push(crate::model::Track {
                    id: id.clone(),
                    kind,
                    name: None,
                    locked: None,
                    mute: None,
                    solo: None,
                    hidden: None,
                    height_px: None,
                    color: None,
                    eq: None,
                    dyn_: None,
                    clips: Vec::new(),
                });
                let after = serde_json::to_value(&p.tracks).unwrap();
                Ok((
                    path,
                    before,
                    after,
                    format!("track_add {id}"),
                    OpKind::Insert,
                    None,
                ))
            }
            Command::ClipMerge { left_id, right_id } => {
                let (lt, li) = p
                    .find_clip(&left_id)
                    .ok_or(Reject::UnknownClip(left_id.clone()))?;
                let (rt, ri) = p
                    .find_clip(&right_id)
                    .ok_or(Reject::UnknownClip(right_id.clone()))?;
                if lt != rt || ri != li + 1 {
                    return Err(Reject::NotAdjacent { left_id, right_id });
                }
                let left_end = p.tracks[lt].clips[li].start_ms + p.tracks[lt].clips[li].duration_ms;
                if left_end != p.tracks[rt].clips[ri].start_ms {
                    return Err(Reject::NotAdjacent { left_id, right_id });
                }
                let path = clips_pointer(lt);
                let before = serde_json::to_value(&p.tracks[lt].clips).unwrap();
                // BUG-04:合并 = 声明"这两段本是一段"——语义连续性谓词(同源/同速/
                // 内容窗积分相接)在 Clip::merge_with;并同时完成关键帧/曲线/fade
                // 等字段并回(split 的精确逆,TC-CORE-MERGE-003 往返不变量)
                let left = p.tracks[lt].clips[li].clone();
                let right = p.tracks[rt].clips[ri].clone();
                let merged = left
                    .merge_with(&right)
                    .map_err(Reject::InvariantViolation)?;
                p.tracks[lt].clips[li] = merged;
                p.tracks[lt].clips.remove(ri);
                let after = serde_json::to_value(&p.tracks[lt].clips).unwrap();
                Ok((
                    path,
                    before,
                    after,
                    format!("clip_merge {left_id}+{right_id}"),
                    OpKind::Merge,
                    None,
                ))
            }
            Command::BgmSet { patch } => {
                // 新建前置:工程尚无 bgm 时必须携带 src(拒绝 src 为空的幽灵背景乐)
                if p.bgm.is_none() && patch.src.is_none() {
                    return Err(Reject::MissingBgm);
                }
                let before = serde_json::to_value(&p.bgm).unwrap_or(Value::Null);
                let changes = patch.apply_to(&mut p.bgm);
                let summary = if changes.is_empty() {
                    "无实际变更".to_string()
                } else {
                    changes
                        .iter()
                        .map(|(ptr, o, n)| format!("{ptr}: {o}→{n}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let after = serde_json::to_value(&p.bgm).unwrap_or(Value::Null);
                Ok((
                    "/bgm".to_string(),
                    before,
                    after,
                    format!("bgm_set: {summary}"),
                    OpKind::Set,
                    None,
                ))
            }
            Command::BgmClear => {
                let before = serde_json::to_value(&p.bgm).unwrap_or(Value::Null);
                p.bgm = None;
                Ok((
                    "/bgm".to_string(),
                    before,
                    Value::Null,
                    "bgm_clear".to_string(),
                    OpKind::Delete,
                    None,
                ))
            }
            Command::ClipTrim {
                clip_id,
                mode,
                edge,
                delta_ms,
            } => {
                let (ti, ci) = p
                    .find_clip(&clip_id)
                    .ok_or(Reject::UnknownClip(clip_id.clone()))?;
                let d = delta_ms as i128;
                // roll/slide 联动多片段 → Op 面为整轨 clips 数组;trim/slip 单片段
                let multi = matches!(mode, TrimMode::Roll | TrimMode::Slide);
                let path = if multi {
                    clips_pointer(ti)
                } else {
                    clip_pointer(ti, ci)
                };
                let before = if multi {
                    serde_json::to_value(&p.tracks[ti].clips).unwrap()
                } else {
                    serde_json::to_value(&p.tracks[ti].clips[ci]).unwrap()
                };
                match mode {
                    TrimMode::Trim => {
                        let clip = &mut p.tracks[ti].clips[ci];
                        let (ns, nd) = match edge {
                            TrimEdge::In => {
                                (clip.start_ms as i128 + d, clip.duration_ms as i128 - d)
                            }
                            TrimEdge::Out => (clip.start_ms as i128, clip.duration_ms as i128 + d),
                        };
                        if ns < 0 {
                            return Err(Reject::InvariantViolation(format!(
                                "trim 越出时间轴起点: start {ns}ms"
                            )));
                        }
                        if nd <= 0 {
                            return Err(Reject::InvariantViolation(format!(
                                "trim 后时长必须 > 0: {nd}ms"
                            )));
                        }
                        if edge == TrimEdge::In
                            && let Some(si) = clip.source_in_ms
                        {
                            let nsi = si as i128 + d;
                            if nsi < 0 {
                                return Err(Reject::InvariantViolation(format!(
                                    "trim-in 越出素材入点: sourceIn {nsi}ms < 0"
                                )));
                            }
                            clip.source_in_ms = Some(nsi as u64);
                        }
                        clip.start_ms = ns as u64;
                        clip.duration_ms = nd as u64;
                        enforce_no_overlap(p, ti)?;
                    }
                    TrimMode::Slip => {
                        // 内容平移:sourceInMs 平移,时间线占位不变;素材时长约束
                        // (sourceIn+duration ≤ 素材长)在派发层经 ffprobe 承接(内核无媒体知识)
                        let clip = &mut p.tracks[ti].clips[ci];
                        let nsi = clip.source_in_ms.unwrap_or(0) as i128 + d;
                        if nsi < 0 {
                            return Err(Reject::InvariantViolation(format!(
                                "slip 越出素材入点: sourceIn {nsi}ms < 0"
                            )));
                        }
                        clip.source_in_ms = Some(nsi as u64);
                    }
                    TrimMode::Roll => {
                        // 双边联动:与 clip 在 edge 侧**贴合**的相邻片段边界同移,总时长不变;
                        // 无贴合邻居 → InvariantViolation(roll 需要相邻片段)
                        let clip_start = p.tracks[ti].clips[ci].start_ms as i128;
                        let clip_end = clip_start + p.tracks[ti].clips[ci].duration_ms as i128;
                        let neighbor = match edge {
                            TrimEdge::In => p.tracks[ti].clips.iter().position(|c| {
                                c.id != clip_id
                                    && c.start_ms < clip_start as u64
                                    && c.start_ms as i128 + c.duration_ms as i128 == clip_start
                            }),
                            TrimEdge::Out => p.tracks[ti]
                                .clips
                                .iter()
                                .position(|c| c.id != clip_id && c.start_ms as i128 == clip_end),
                        };
                        let Some(ni) = neighbor else {
                            return Err(Reject::InvariantViolation(format!(
                                "roll 需要 {} 侧贴合相邻片段({clip_id} 的 {} 边界无邻居)",
                                match edge {
                                    TrimEdge::In => "入点",
                                    TrimEdge::Out => "出点",
                                },
                                match edge {
                                    TrimEdge::In => "in",
                                    TrimEdge::Out => "out",
                                }
                            )));
                        };
                        let nb = match edge {
                            TrimEdge::In => clip_start + d,
                            TrimEdge::Out => clip_end + d,
                        };
                        let left_bound = match edge {
                            TrimEdge::In => p.tracks[ti].clips[ni].start_ms as i128,
                            TrimEdge::Out => clip_start,
                        };
                        let right_bound = match edge {
                            TrimEdge::In => clip_end,
                            TrimEdge::Out => {
                                let n = &p.tracks[ti].clips[ni];
                                n.start_ms as i128 + n.duration_ms as i128
                            }
                        };
                        if nb <= left_bound {
                            return Err(Reject::InvariantViolation(format!(
                                "roll 后边界一侧时长必须 > 0: 边界 {nb}ms ≤ {left_bound}ms"
                            )));
                        }
                        if nb >= right_bound {
                            return Err(Reject::InvariantViolation(format!(
                                "roll 后边界另一侧时长必须 > 0: 边界 {nb}ms ≥ {right_bound}ms"
                            )));
                        }
                        match edge {
                            TrimEdge::In => {
                                let shift = nb - clip_start;
                                let n = &mut p.tracks[ti].clips[ni];
                                n.duration_ms = (nb - n.start_ms as i128) as u64;
                                let cid = p.tracks[ti].clips[ci].id.clone();
                                let c = &mut p.tracks[ti].clips[ci];
                                c.start_ms = nb as u64;
                                c.duration_ms = (clip_end - nb) as u64;
                                if let Some(si) = c.source_in_ms {
                                    // BUG-03:拒绝式守卫(负 i128 as u64 会回绕成天文数字)
                                    c.source_in_ms = Some(super::invariants::checked_source_in(
                                        si, shift, &cid,
                                    )?);
                                }
                            }
                            TrimEdge::Out => {
                                let n = &mut p.tracks[ti].clips[ni];
                                let shift = nb - n.start_ms as i128;
                                n.duration_ms = ((n.start_ms + n.duration_ms) as i128 - nb) as u64;
                                n.start_ms = nb as u64;
                                let cid = n.id.clone();
                                if let Some(si) = n.source_in_ms {
                                    n.source_in_ms = Some(super::invariants::checked_source_in(
                                        si, shift, &cid,
                                    )?);
                                }
                                let c = &mut p.tracks[ti].clips[ci];
                                c.duration_ms = (nb - clip_start) as u64;
                            }
                        }
                    }
                    TrimMode::Slide => {
                        // 位置平移:内容窗不变,贴合邻居随之让位(邻居边界跟随闭合)/压缩;
                        // 非贴合邻居不动,越界相撞交 enforce_no_overlap 裁决
                        let clip = &p.tracks[ti].clips[ci];
                        let old_start = clip.start_ms as i128;
                        let dur = clip.duration_ms as i128;
                        let old_end = old_start + dur;
                        let ns = old_start + d;
                        if ns < 0 {
                            return Err(Reject::InvariantViolation(format!(
                                "slide 越出时间轴起点: start {ns}ms"
                            )));
                        }
                        let ne = ns + dur;
                        let li = p.tracks[ti].clips.iter().position(|c| {
                            c.id != clip_id
                                && c.start_ms as i128 + c.duration_ms as i128 == old_start
                        });
                        let ri = p.tracks[ti]
                            .clips
                            .iter()
                            .position(|c| c.id != clip_id && c.start_ms as i128 == old_end);
                        if let Some(li) = li {
                            let l = &mut p.tracks[ti].clips[li];
                            let l_start = l.start_ms as i128;
                            if ns <= l_start {
                                return Err(Reject::InvariantViolation(format!(
                                    "slide 后左邻时长必须 > 0: 新边界 {ns}ms ≤ 左邻起点 {l_start}ms"
                                )));
                            }
                            l.duration_ms = (ns - l_start) as u64;
                        }
                        if let Some(ri) = ri {
                            let r = &mut p.tracks[ti].clips[ri];
                            let r_end = (r.start_ms + r.duration_ms) as i128;
                            if ne >= r_end {
                                return Err(Reject::InvariantViolation(format!(
                                    "slide 后右邻时长必须 > 0: 新边界 {ne}ms ≥ 右邻终点 {r_end}ms"
                                )));
                            }
                            let shift = ne - r.start_ms as i128;
                            r.duration_ms = (r_end - ne) as u64;
                            r.start_ms = ne as u64;
                            let rid = r.id.clone();
                            if let Some(si) = r.source_in_ms {
                                // BUG-03:拒绝式守卫(负 i128 as u64 会回绕成天文数字)
                                r.source_in_ms =
                                    Some(super::invariants::checked_source_in(si, shift, &rid)?);
                            }
                        }
                        p.tracks[ti].clips[ci].start_ms = ns as u64;
                        enforce_no_overlap(p, ti)?;
                    }
                }
                let after = if multi {
                    serde_json::to_value(&p.tracks[ti].clips).unwrap()
                } else {
                    serde_json::to_value(&p.tracks[ti].clips[ci]).unwrap()
                };
                let label = match mode {
                    TrimMode::Trim => format!(
                        "clip_trim {clip_id} {} {delta_ms:+}ms",
                        match edge {
                            TrimEdge::In => "in",
                            TrimEdge::Out => "out",
                        }
                    ),
                    TrimMode::Roll => format!(
                        "clip_trim roll {clip_id} {} {delta_ms:+}ms(边界联动)",
                        match edge {
                            TrimEdge::In => "in",
                            TrimEdge::Out => "out",
                        }
                    ),
                    TrimMode::Slip => format!("clip_trim slip {clip_id} {delta_ms:+}ms(内容平移)"),
                    TrimMode::Slide => {
                        format!("clip_trim slide {clip_id} {delta_ms:+}ms(位置平移)")
                    }
                };
                // trim/slip 的 Op 面为单 clip 对象指针 → 记录稳定 id(BUG-06);
                // roll/slide 的 Op 面为整轨 clips 数组 → 无单点 target
                let target_id = match mode {
                    TrimMode::Trim | TrimMode::Slip => Some(clip_id),
                    TrimMode::Roll | TrimMode::Slide => None,
                };
                Ok((
                    path,
                    before,
                    after,
                    label,
                    if matches!(mode, TrimMode::Slide) {
                        OpKind::Move
                    } else {
                        OpKind::Set
                    },
                    target_id,
                ))
            }
            Command::ClipSplitAll { t_ms } => {
                let path = "/tracks".to_string();
                let before = serde_json::to_value(&p.tracks).unwrap();
                let mut n = 0usize;
                for track in p.tracks.iter_mut() {
                    // 先收集命中下标再改,避免插队导致的下标漂移
                    let hits: Vec<usize> = track
                        .clips
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| c.start_ms < t_ms && t_ms < c.start_ms + c.duration_ms)
                        .map(|(i, _)| i)
                        .collect();
                    for (offset, i) in hits.into_iter().enumerate() {
                        let ci = i + offset;
                        // 切分语义收口 Clip::split_at(与 clip_split 同口径,BUG-01/02)
                        let right_id = Project::next_clip_id(track);
                        let (left, mut right) = track.clips[ci].split_at(t_ms).map_err(|e| {
                            Reject::InvariantViolation(format!(
                                "clip_split_all 命中片段越界(不应发生): {e:?}"
                            ))
                        })?;
                        right.id = right_id;
                        track.clips[ci] = left;
                        track.clips.insert(ci + 1, right);
                        n += 1;
                    }
                }
                let after = serde_json::to_value(&p.tracks).unwrap();
                Ok((
                    path,
                    before,
                    after,
                    format!("clip_split_all @{t_ms}({n} 段)"),
                    OpKind::Split,
                    None,
                ))
            }
            Command::TrackUpdate { track_id, patch } => {
                if patch.is_empty() {
                    return Err(Reject::EmptyPatch(track_id));
                }
                let ti = p
                    .find_track(&track_id)
                    .ok_or(Reject::UnknownTrack(track_id.clone()))?;
                let path = format!("/tracks/{ti}");
                let before = serde_json::to_value(&p.tracks[ti]).unwrap();
                let changes = patch.apply_to(&mut p.tracks[ti]);
                let summary = if changes.is_empty() {
                    "无实际变更".to_string()
                } else {
                    changes
                        .iter()
                        .map(|(ptr, o, n)| format!("{ptr}: {o}→{n}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let after = serde_json::to_value(&p.tracks[ti]).unwrap();
                Ok((
                    path,
                    before,
                    after,
                    format!("track_update {track_id}: {summary}"),
                    OpKind::Set,
                    None,
                ))
            }
            Command::ClipGapDelete { track_id, t_ms } => {
                let ti = p
                    .find_track(&track_id)
                    .ok_or(Reject::UnknownTrack(track_id.clone()))?;
                let before = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                let clips = &mut p.tracks[ti].clips;
                if clips.is_empty() {
                    return Err(Reject::InvariantViolation(format!(
                        "{track_id} 轨上无片段,无间隙可删"
                    )));
                }
                // 按 start 排序的视图定位包含 t_ms 的间隙 [prev_end, next_start);
                // 首段之前的空档视作 [0, first.start)
                let mut order: Vec<usize> = (0..clips.len()).collect();
                order.sort_by_key(|&i| clips[i].start_ms);
                let mut prev_end: u64 = 0;
                let mut gap: Option<(u64, u64)> = None;
                for &i in &order {
                    let s = clips[i].start_ms;
                    if s > prev_end && prev_end <= t_ms && t_ms < s {
                        gap = Some((prev_end, s));
                        break;
                    }
                    prev_end = prev_end.max(s + clips[i].duration_ms);
                }
                let (gs, ge) = gap.ok_or_else(|| {
                    Reject::InvariantViolation(format!(
                        "t_ms={t_ms} 处无间隙可删(位于片段内部或全部片段之后)"
                    ))
                })?;
                let shift = ge - gs;
                for c in clips.iter_mut() {
                    if c.start_ms >= ge {
                        c.start_ms -= shift;
                    }
                }
                let after = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                Ok((
                    clips_pointer(ti),
                    before,
                    after,
                    format!("clip_gap_delete {track_id}@{t_ms} 闭合 {shift}ms"),
                    OpKind::Move,
                    None,
                ))
            }
            // 批量命令(册四 T4.7):mutate 入口已委托 engine::batch,此臂仅穷尽性
            Command::ClipsInsert { .. } | Command::ClipsPatch { .. } => {
                unreachable!("批量命令已在 mutate 入口分派(engine::batch)")
            }
            // 复合片段打包/解包与单轨多点分割(册五 T5.4;实现在 engine::pro_cmds
            // ——行数红线 A1-3 纯移动,mutate 臂只做薄委托,语义与 Reject 面零变化)
            Command::CompoundCreate {
                clip_ids,
                to_track,
                start_ms,
                request_id: _,
            } => super::pro_cmds::compound_create(p, clip_ids, to_track, start_ms),
            Command::CompoundUnbind { clip_id } => super::pro_cmds::compound_unbind(p, clip_id),
            Command::TrackSplitAt { track_id, t_points } => {
                super::pro_cmds::track_split_at(p, track_id, t_points)
            }
        }
    }
}

fn clip_pointer(ti: usize, ci: usize) -> String {
    format!("/tracks/{ti}/clips/{ci}")
}

/// 短借取片段(切分等按值消费语义的场景;借用到语句末即止)。
fn clip_ref(p: &Project, ti: usize, ci: usize) -> &crate::model::Clip {
    &p.tracks[ti].clips[ci]
}

fn clips_pointer(ti: usize) -> String {
    format!("/tracks/{ti}/clips")
}

impl crate::model::TrackKind {
    /// kind 字符串(crate 内共用;pro_cmds 经 kind_json_pub 桥取用)。
    pub(crate) fn kind_json(&self) -> &'static str {
        match self {
            crate::model::TrackKind::Video => "video",
            crate::model::TrackKind::Audio => "audio",
            crate::model::TrackKind::Text => "text",
            crate::model::TrackKind::Adjust => "adjust",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::ClipPatch;
    use crate::engine::{Answer, Query, Reject, patch, sample_project};
    use crate::oplog::ActorKind;
    use serde_json::json;

    fn agent() -> Actor {
        Actor::agent("test")
    }

    #[test]
    fn apply_update_sets_rev_and_op() {
        let mut eng = Engine::new(sample_project()).unwrap();
        let r = eng
            .apply(
                Command::ClipUpdate {
                    clip_id: "V1-001".into(),
                    patch: patch(8000),
                },
                agent(),
                ApplyOpts::default(),
            )
            .unwrap();
        assert_eq!(r.rev, 1);
        assert_eq!(eng.rev(), 1);
        let clip = eng.query(Query::Clip {
            id: "V1-001".into(),
        });
        match clip {
            Answer::Clip(Some(c)) => assert_eq!(c["durationMs"], json!(8000)),
            other => panic!("意外: {other:?}"),
        }
        assert_eq!(eng.oplog().len(), 1);
        let op = &eng.oplog().ops()[0];
        assert_eq!(op.base_rev, "rev-0");
        assert_eq!(op.op_kind, OpKind::Set);
        assert_eq!(op.actor.kind, ActorKind::Agent);
    }

    #[test]
    fn update_overlap_rejected_and_rolled_back() {
        let mut eng = Engine::new(sample_project()).unwrap();
        let before = eng.query(Query::Clip {
            id: "V1-001".into(),
        });
        // 移动 V1-001 到 9000ms(与 V1-002[8400..14600] 重叠)→ 必须拒绝
        let r = eng.apply(
            Command::ClipUpdate {
                clip_id: "V1-001".into(),
                patch: ClipPatch {
                    start_ms: Some(9000),
                    duration_ms: Some(2000),
                    ..Default::default()
                },
            },
            agent(),
            ApplyOpts::default(),
        );
        assert!(matches!(r, Err(Reject::InvariantViolation(_))));
        assert_eq!(
            eng.query(Query::Clip {
                id: "V1-001".into()
            }),
            before,
            "失败必须回滚"
        );
        assert_eq!(eng.oplog().len(), 0, "失败不得产生 Op");
        assert_eq!(eng.rev(), 0);
    }

    #[test]
    fn precondition_and_unknown_targets() {
        let mut eng = Engine::new(sample_project()).unwrap();
        let r = eng.apply(
            Command::ClipDelete {
                clip_id: "V1-001".into(),
            },
            agent(),
            ApplyOpts {
                expect_rev: Some(5),
                ..Default::default()
            },
        );
        assert!(matches!(
            r,
            Err(Reject::PreconditionFailed {
                expected: 5,
                actual: 0
            })
        ));
        let r = eng.apply(
            Command::ClipDelete {
                clip_id: "不存在".into(),
            },
            agent(),
            ApplyOpts::default(),
        );
        assert!(matches!(r, Err(Reject::UnknownClip(_))));
        let r = eng.apply(
            Command::ClipInsert {
                to_track: "T9".into(),
                clip: sample_project().tracks[0].clips[0].clone(),
                request_id: None,
            },
            agent(),
            ApplyOpts::default(),
        );
        assert!(matches!(r, Err(Reject::UnknownTrack(_))));
    }

    #[test]
    fn split_merge_and_idempotent_split() {
        let mut eng = Engine::new(sample_project()).unwrap();
        let r = eng
            .apply(
                Command::ClipSplit {
                    clip_id: "V1-001".into(),
                    t_ms: 4000,
                },
                agent(),
                ApplyOpts::default(),
            )
            .unwrap();
        assert_eq!(r.rev, 1);
        match eng.query(Query::Timeline) {
            Answer::Timeline(tl) => {
                assert_eq!(tl.len(), 4, "V1 三个片段 + A1 一个音效");
                assert!(
                    tl.iter()
                        .any(|(id, s, e, _)| id == "V1-003" && *s == 4000 && *e == 8400)
                );
            }
            other => panic!("意外: {other:?}"),
        }
        // 再切同一点:V1-003 起点已是 4000 → 幂等
        let r2 = eng
            .apply(
                Command::ClipSplit {
                    clip_id: "V1-001".into(),
                    t_ms: 4000,
                },
                agent(),
                ApplyOpts::default(),
            )
            .unwrap();
        assert_eq!(r2.rev, 1, "幂等切分不得再升 rev");
        // 合并回去
        let r3 = eng
            .apply(
                Command::ClipMerge {
                    left_id: "V1-001".into(),
                    right_id: "V1-003".into(),
                },
                agent(),
                ApplyOpts::default(),
            )
            .unwrap();
        assert_eq!(r3.rev, 2);
        match eng.query(Query::Clip {
            id: "V1-001".into(),
        }) {
            Answer::Clip(Some(c)) => assert_eq!(c["durationMs"], json!(8400)),
            other => panic!("意外: {other:?}"),
        }
        // 非相邻合并拒绝:先重切出中间片段,V1-001 与 V1-002 隔着它
        let _ = eng
            .apply(
                Command::ClipSplit {
                    clip_id: "V1-001".into(),
                    t_ms: 4000,
                },
                agent(),
                ApplyOpts::default(),
            )
            .unwrap();
        let r4 = eng.apply(
            Command::ClipMerge {
                left_id: "V1-001".into(),
                right_id: "V1-002".into(),
            },
            agent(),
            ApplyOpts::default(),
        );
        assert!(matches!(r4, Err(Reject::NotAdjacent { .. })));
        // 切分越界拒绝(5000 不是既有边界,V1-001 已是 [0..4000])
        let r5 = eng.apply(
            Command::ClipSplit {
                clip_id: "V1-001".into(),
                t_ms: 5000,
            },
            agent(),
            ApplyOpts::default(),
        );
        assert!(matches!(r5, Err(Reject::SplitOutside { .. })));
    }

    /// transition/motion 经 ClipUpdate 应用:合并语义 + 枚举外值由 schema 层拒(SCHEMA_INVALID 回滚)。
    #[test]
    fn clip_update_transition_motion_apply_and_schema_guard() {
        let mut eng = Engine::new(sample_project()).unwrap();
        let r = eng
            .apply(
                Command::ClipUpdate {
                    clip_id: "V1-002".into(),
                    patch: ClipPatch {
                        transition: Some(crate::command::TransitionPatch {
                            type_: Some("slideleft".into()),
                            dur_ms: Some(320.0),
                            ..Default::default()
                        }),
                        motion: Some(crate::command::MotionPatch {
                            in_: Some("zoomIn".into()),
                            out: Some("fadeOut".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                },
                agent(),
                ApplyOpts::default(),
            )
            .unwrap();
        assert_eq!(r.rev, 1);
        match eng.query(Query::Clip {
            id: "V1-002".into(),
        }) {
            Answer::Clip(Some(c)) => {
                assert_eq!(c["transition"]["type"], json!("slideleft"));
                assert_eq!(c["transition"]["durMs"], json!(320.0));
                assert_eq!(c["motion"]["in"], json!("zoomIn"));
                assert_eq!(c["motion"]["out"], json!("fadeOut"));
                assert!(c["motion"].get("inMs").is_none(), "未给出的字段不得臆造");
            }
            other => panic!("意外: {other:?}"),
        }
        // 部分合并:再给 reason,既有 type/durMs 保持
        eng.apply(
            Command::ClipUpdate {
                clip_id: "V1-002".into(),
                patch: ClipPatch {
                    transition: Some(crate::command::TransitionPatch {
                        reason: Some("topic".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            },
            agent(),
            ApplyOpts::default(),
        )
        .unwrap();
        match eng.query(Query::Clip {
            id: "V1-002".into(),
        }) {
            Answer::Clip(Some(c)) => {
                assert_eq!(
                    c["transition"]["type"],
                    json!("slideleft"),
                    "未给出的字段不得被清掉"
                );
                assert_eq!(c["transition"]["reason"], json!("topic"));
            }
            other => panic!("意外: {other:?}"),
        }
        // 枚举外值:内核不设枚举约束,由 schema 层拒绝并回滚
        let before = eng.query(Query::Clip {
            id: "V1-002".into(),
        });
        let r = eng.apply(
            Command::ClipUpdate {
                clip_id: "V1-002".into(),
                patch: ClipPatch {
                    transition: Some(crate::command::TransitionPatch {
                        type_: Some("爆闪".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            },
            agent(),
            ApplyOpts::default(),
        );
        assert!(
            matches!(r, Err(Reject::SchemaInvalid(_))),
            "枚举外转场必须被 schema 层拒: {r:?}"
        );
        assert_eq!(
            eng.query(Query::Clip {
                id: "V1-002".into()
            }),
            before,
            "拒绝必须回滚"
        );
        assert_eq!(eng.rev(), 2, "拒绝不得升 rev");
        let r = eng.apply(
            Command::ClipUpdate {
                clip_id: "V1-002".into(),
                patch: ClipPatch {
                    motion: Some(crate::command::MotionPatch {
                        in_: Some("乱入".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            },
            agent(),
            ApplyOpts::default(),
        );
        assert!(
            matches!(r, Err(Reject::SchemaInvalid(_))),
            "枚举外动效必须被 schema 层拒: {r:?}"
        );
    }

    #[cfg(test)]
    mod pro_tests {
        // include! 相对当前文件(engine/)解析,POSIX/Windows 一致;super = apply::tests,
        // 与原 #[path] 内联挂载同语义(#[path] 的相对链在内联模块下会命中不存在的
        // 目录段,POSIX 不做词法 .. 归一——CI ubuntu 实证)。
        include!("pro_tests.rs");
    }
}
