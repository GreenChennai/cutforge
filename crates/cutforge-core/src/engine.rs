//! 引擎(计划书 2.6/4.2/4.4):查询接口与命令接口严格分离。
//!
//! - `query`:纯投影,无副作用,可并发;
//! - `apply`/`undo`/`redo`:唯一写入口——校验 → 变更 → schema 验证 →
//!   生成 Op(含 baseRev)入 OpLog → rev 递增。
//!   步骤"校验前置条件"是'所见即所得'能成立的唯一原因:相对 baseRev
//!   已失效的写入会被拒绝,不存在静默覆盖(北极星指标)。

use crate::command::{ClipPatch, Command};
use crate::model::Project;
use crate::oplog::{Actor, Op, OpKind, OpLog, OpTarget};
use serde_json::{json, Value};

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
    NothingToUndo,
    NothingToRedo,
}

/// apply 选项:op_id/request_id 支持幂等;caused_by 把改动与标注绑定(4.9);
/// expect_rev 是 baseRev 前置检查(调用方声明"我基于哪一版改");
/// non_undoable 标记自动簿记类登记(锚点重定位):进审计链但不入撤销栈(ADR-0001)。
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

#[derive(Debug, Clone)]
pub enum Query {
    /// 全工程视图(序列化 Value)。
    ProjectView,
    /// 单片段。
    Clip { id: String },
    /// 单轨道。
    Track { id: String },
    /// 时间线概览:(clip_id, start_ms, end_ms, track_id)。
    Timeline,
    /// OpLog tail。
    OpLogTail { since_rev: Option<u64>, actor_kind: Option<crate::oplog::ActorKind> },
    /// 当前 rev。
    Rev,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Project(Value),
    Clip(Option<Value>),
    Track(Option<Value>),
    Timeline(Vec<(String, u64, u64, String)>),
    Ops(Vec<Op>),
    Rev(u64),
}

pub struct Engine {
    project: Project,
    log: OpLog,
    rev: u64,
    undo_stack: Vec<String>,
    redo_stack: Vec<String>,
    /// 非工程真相源(notes.json/cutlist.json 等)的内存态:文件级 Op 的
    /// 撤销/重做路由目标(ADR-0001:按 target.file 逆写,而非伪造指针回写)。
    file_states: std::collections::BTreeMap<String, Value>,
    /// 自上次 drain 以来被写脏的真相源文件(IO 层据此落盘,先文件后记账)。
    dirty_files: std::collections::BTreeSet<String>,
}

impl Engine {
    pub fn new(project: Project) -> Result<Self, Vec<String>> {
        project.to_validated_value()?;
        Ok(Self {
            project, log: OpLog::new(), rev: 0,
            undo_stack: Vec::new(), redo_stack: Vec::new(),
            file_states: std::collections::BTreeMap::new(),
            dirty_files: std::collections::BTreeSet::new(),
        })
    }

    /// IO 层加载既有工程时用:恢复(项目, OpLog, rev, 撤销栈, 文件态)五元组。
    pub fn restore(
        project: Project,
        log: OpLog,
        rev: u64,
        undo_stack: Vec<String>,
        file_states: std::collections::BTreeMap<String, Value>,
    ) -> Result<Self, Vec<String>> {
        project.to_validated_value()?;
        Ok(Self { project, log, rev, undo_stack, redo_stack: Vec::new(), file_states, dirty_files: std::collections::BTreeSet::new() })
    }

    pub fn rev(&self) -> u64 {
        self.rev
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn oplog(&self) -> &OpLog {
        &self.log
    }

    /// 某真相源文件的当前内存态(撤销/重做路由与 IO 落盘的依据)。
    pub fn file_state(&self, file: &str) -> Option<&Value> {
        self.file_states.get(file)
    }

    /// 全部文件态快照(打开/合并时传递给新 Engine)。
    pub fn file_states(&self) -> &std::collections::BTreeMap<String, Value> {
        &self.file_states
    }

    /// 取走并清空脏文件清单(先文件后记账:IO 层先落盘这些文件再追加 oplog)。
    pub fn take_dirty_files(&mut self) -> Vec<String> {
        let files: Vec<String> = self.dirty_files.iter().cloned().collect();
        self.dirty_files.clear();
        files
    }

    /// 查询接口:纯投影。
    pub fn query(&self, q: Query) -> Answer {
        match q {
            Query::ProjectView => Answer::Project(
                serde_json::to_value(&self.project).unwrap_or(Value::Null),
            ),
            Query::Clip { id } => Answer::Clip(
                self.project.find_clip(&id).map(|(ti, ci)| {
                    serde_json::to_value(&self.project.tracks[ti].clips[ci]).unwrap_or(Value::Null)
                }),
            ),
            Query::Track { id } => Answer::Track(
                self.project.find_track(&id)
                    .map(|ti| serde_json::to_value(&self.project.tracks[ti]).unwrap_or(Value::Null)),
            ),
            Query::Timeline => Answer::Timeline(
                self.project.tracks.iter().flat_map(|t| {
                    t.clips.iter().map(move |c| {
                        (c.id.clone(), c.start_ms, c.start_ms + c.duration_ms, t.id.clone())
                    })
                }).collect(),
            ),
            Query::OpLogTail { since_rev, actor_kind } => {
                Answer::Ops(self.log.tail(since_rev, actor_kind).into_iter().cloned().collect())
            }
            Query::Rev => Answer::Rev(self.rev),
        }
    }

    /// 命令接口:唯一写入口。
    pub fn apply(&mut self, cmd: Command, actor: Actor, opts: ApplyOpts) -> Result<OpReceipt, Reject> {
        // 幂等 1:request_id 去重(非幂等写操作,计划书 5.2)
        if let Some(rid) = opts.request_id.as_deref()
            && self.log.has_request_id(rid) {
                return Ok(OpReceipt { op_ids: Vec::new(), rev: self.rev, idempotent: true });
            }
        // 前置条件:相对 baseRev 已失效 → 拒绝(不存在静默覆盖)
        if let Some(expect) = opts.expect_rev
            && expect != self.rev {
                return Err(Reject::PreconditionFailed { expected: expect, actual: self.rev });
            }
        // 幂等 2:显式 op_id 已存在 → 原样回执
        if let Some(id) = opts.op_id.as_deref()
            && self.log.ops().iter().any(|o| o.op_id == id) {
                return Ok(OpReceipt { op_ids: vec![id.to_string()], rev: self.rev, idempotent: true });
            }

        let snapshot = self.project.clone();
        let outcome = self.mutate(cmd.clone());
        let (path, before, after, summary, kind) = match outcome {
            Ok(o) => o,
            Err(rej) => {
                self.project = snapshot;
                return Err(rej);
            }
        };
        // 通用幂等:命令未产生任何变化 → 不升 rev、不产 Op,回执标注 idempotent
        if before == after {
            self.project = snapshot;
            return Ok(OpReceipt { op_ids: Vec::new(), rev: self.rev, idempotent: true });
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
            target: OpTarget { file: "project.json".into(), path },
            op_kind: kind,
            before,
            after,
            base_rev: crate::format_rev(self.rev - 1),
            rev: Some(self.rev),
            caused_by: if opts.caused_by.is_empty() { None } else { Some(opts.caused_by) },
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
                Ok(OpReceipt { op_ids: vec![op_id], rev: self.rev, idempotent: false })
            }
            // op_id 撞车(并发分配同一 id):本次不生效,按幂等回执
            None => {
                self.rev -= 1;
                self.project = snapshot;
                Ok(OpReceipt { op_ids: vec![op_id], rev: self.rev, idempotent: true })
            }
        }
    }

    /// 撤销:按 target.file 路由逆写(ADR-0001)——project.json 走指针回写,
    /// 文件级真相源回滚其在 file_states 的内存态(由 IO 层落盘),产生 opKind=undo 的新 Op。
    pub fn undo(&mut self, actor: Actor) -> Result<OpReceipt, Reject> {
        let target_id = self.undo_stack.last().cloned().ok_or(Reject::NothingToUndo)?;
        let original = self.log.ops().iter().find(|o| o.op_id == target_id).cloned().ok_or(Reject::UnknownOp(target_id))?;
        self.rev += 1;
        let is_project = original.target.file == "project.json";
        if !is_project {
            // 文件级:仅当当前态恰为该 Op 的 after 才允许逆写(LIFO 语义被外部扰动时如实拒绝)
            match self.file_states.get(&original.target.file) {
                Some(cur) if *cur == original.after => {}
                _ => {
                    self.rev -= 1;
                    return Err(Reject::InvariantViolation(format!(
                        "撤销基准不一致:{} 的当前态已偏离待撤销 Op 的 after(外部改动或重放),拒绝盲写",
                        original.target.file)));
                }
            }
        }
        let undo_op = Op {
            op_id: self.log.next_op_id(),
            ts: crate::timeutil::now_rfc3339(),
            actor,
            target: original.target.clone(),
            op_kind: OpKind::Undo,
            before: original.after.clone(),
            after: original.before.clone(),
            base_rev: crate::format_rev(self.rev - 1),
            rev: Some(self.rev),
            caused_by: None,
            summary: format!("撤销 {}", original.summary),
            request_id: None,
            auto: None,
        };
        if is_project {
            let mut project = self.project.clone();
            apply_value_at(&mut project, &original.target.path, original.before.clone())
                .map_err(|e| { self.rev -= 1; Reject::InvariantViolation(e) })?;
            if let Err(errs) = project.to_validated_value() {
                self.rev -= 1;
                return Err(Reject::SchemaInvalid(errs));
            }
            self.project = project;
        } else {
            self.file_states.insert(original.target.file.clone(), original.before.clone());
            self.dirty_files.insert(original.target.file.clone());
        }
        self.log.push(undo_op);
        self.undo_stack.pop();
        self.redo_stack.push(original.op_id.clone());
        Ok(OpReceipt { op_ids: self.log.ops().last().map(|o| vec![o.op_id.clone()]).unwrap_or_default(), rev: self.rev, idempotent: false })
    }

    /// 重做:按 target.file 路由恢复被撤销 Op 的 after,产生 opKind=redo 的新 Op。
    pub fn redo(&mut self, actor: Actor) -> Result<OpReceipt, Reject> {
        let target_id = self.redo_stack.last().cloned().ok_or(Reject::NothingToRedo)?;
        let original = self.log.ops().iter().find(|o| o.op_id == target_id).cloned().ok_or(Reject::UnknownOp(target_id))?;
        self.rev += 1;
        let is_project = original.target.file == "project.json";
        if !is_project {
            match self.file_states.get(&original.target.file) {
                Some(cur) if *cur == original.before => {}
                _ => {
                    self.rev -= 1;
                    return Err(Reject::InvariantViolation(format!(
                        "重做基准不一致:{} 的当前态已偏离待重做 Op 的 before,拒绝盲写",
                        original.target.file)));
                }
            }
        }
        let redo_op = Op {
            op_id: self.log.next_op_id(),
            ts: crate::timeutil::now_rfc3339(),
            actor,
            target: original.target.clone(),
            op_kind: OpKind::Redo,
            before: original.before.clone(),
            after: original.after.clone(),
            base_rev: crate::format_rev(self.rev - 1),
            rev: Some(self.rev),
            caused_by: None,
            summary: format!("重做 {}", original.summary),
            request_id: None,
            auto: None,
        };
        if is_project {
            let mut project = self.project.clone();
            apply_value_at(&mut project, &original.target.path, original.after.clone())
                .map_err(|e| { self.rev -= 1; Reject::InvariantViolation(e) })?;
            if let Err(errs) = project.to_validated_value() {
                self.rev -= 1;
                return Err(Reject::SchemaInvalid(errs));
            }
            self.project = project;
        } else {
            self.file_states.insert(original.target.file.clone(), original.after.clone());
            self.dirty_files.insert(original.target.file.clone());
        }
        self.log.push(redo_op);
        self.redo_stack.pop();
        self.undo_stack.push(original.op_id.clone());
        Ok(OpReceipt { op_ids: self.log.ops().last().map(|o| vec![o.op_id.clone()]).unwrap_or_default(), rev: self.rev, idempotent: false })
    }

    /// 从工程 + 完整 OpLog 回放,得到与逐步 apply 语义一致的状态(计划书 4.4 可回放)。
    /// undo/redo Op 的 after 本身就是当时的状态转移,故全部照序应用;
    /// 文件级 Op 路由到 file_states(ADR-0001),不与工程文档混淆;
    /// 撤销栈按日志语义模拟重建(Undo 弹栈、Redo 压回,auto 类跳过)。
    pub fn replay(base: Project, ops: &[Op]) -> Result<Self, Reject> {
        let mut eng = Engine::new(base).map_err(Reject::SchemaInvalid)?;
        eng.rev = 0;
        for op in ops {
            if op.target.file == "project.json" {
                let mut project = eng.project.clone();
                apply_value_at(&mut project, &op.target.path, op.after.clone())
                    .map_err(Reject::InvariantViolation)?;
                if let Err(errs) = project.to_validated_value() {
                    return Err(Reject::SchemaInvalid(errs));
                }
                eng.project = project;
            } else {
                eng.file_states.insert(op.target.file.clone(), op.after.clone());
            }
            eng.rev = op.rev.unwrap_or(eng.rev + 1);
            eng.log.push_loaded(op.clone());
        }
        eng.undo_stack = rebuild_undo_stack(ops);
        eng.redo_stack = Vec::new();
        Ok(eng)
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
            "project.json", "wordline.json", "cutlist.json", "cutlist.applied.json", "notes.json",
        ];
        if !KNOWN.contains(&file) {
            return Err(Reject::InvariantViolation(format!("未知真相源文件: {file}")));
        }
        if let Some(rid) = opts.request_id.as_deref()
            && self.log.has_request_id(rid) {
                return Ok(OpReceipt { op_ids: Vec::new(), rev: self.rev, idempotent: true });
            }
        if let Some(expect) = opts.expect_rev
            && expect != self.rev {
                return Err(Reject::PreconditionFailed { expected: expect, actual: self.rev });
            }
        if before == after {
            return Ok(OpReceipt { op_ids: Vec::new(), rev: self.rev, idempotent: true });
        }
        self.rev += 1;
        let op = Op {
            op_id: opts.op_id.unwrap_or_else(|| self.log.next_op_id()),
            ts: crate::timeutil::now_rfc3339(),
            actor,
            target: OpTarget { file: file.to_string(), path: path.to_string() },
            op_kind: kind,
            before,
            after: after.clone(),
            base_rev: crate::format_rev(self.rev - 1),
            rev: Some(self.rev),
            caused_by: if opts.caused_by.is_empty() { None } else { Some(opts.caused_by) },
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
                Ok(OpReceipt { op_ids: vec![op_id], rev: self.rev, idempotent: false })
            }
            None => {
                self.rev -= 1;
                Ok(OpReceipt { op_ids: vec![op_id], rev: self.rev, idempotent: true })
            }
        }
    }

    /// 状态语义 hash(测试与 M3 回放等价门禁的基础)。
    pub fn state_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let v = serde_json::to_value(&self.project).unwrap_or(Value::Null);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        canonical_json(&v).hash(&mut h);
        h.finish()
    }

    /// 命令 → (路径, before, after, 摘要)。变更就地生效;失败时调用方回滚快照。
    #[allow(clippy::type_complexity)]
    fn mutate(&mut self, cmd: Command) -> Result<(String, Value, Value, String, OpKind), Reject> {
        let p = &mut self.project;
        match cmd {
            Command::ClipUpdate { clip_id, patch } => {
                if patch.is_empty() {
                    return Err(Reject::EmptyPatch(clip_id));
                }
                let (ti, ci) = p.find_clip(&clip_id).ok_or(Reject::UnknownClip(clip_id.clone()))?;
                let path = clip_pointer(ti, ci);
                let before = serde_json::to_value(&p.tracks[ti].clips[ci]).unwrap();
                let mut clip = p.tracks[ti].clips[ci].clone();
                let changes = patch.apply_to(&mut clip);
                let summary = if changes.is_empty() {
                    "无实际变更".to_string()
                } else {
                    changes.iter().map(|(ptr, o, n)| format!("{ptr}: {o}→{n}")).collect::<Vec<_>>().join(", ")
                };
                p.tracks[ti].clips[ci] = clip;
                let after = serde_json::to_value(&p.tracks[ti].clips[ci]).unwrap();
                enforce_no_overlap(p, ti)?;
                Ok((path, before, after, format!("clip_update {clip_id}: {summary}"), OpKind::Set))
            }
            Command::ClipSplit { clip_id, t_ms } => {
                let (ti, ci) = p.find_clip(&clip_id).ok_or(Reject::UnknownClip(clip_id.clone()))?;
                let clip = &p.tracks[ti].clips[ci];
                let end = clip.start_ms + clip.duration_ms;
                // 幂等语义(计划书 5.2:已切则返回既有结果):切点边界已存在 → 无操作
                if p.tracks[ti].clips.iter().any(|c| c.start_ms == t_ms && c.id != clip_id) {
                    let arr = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                    let before = arr.clone();
                    return Ok((clips_pointer(ti), before, arr, format!("clip_split {clip_id}@{t_ms}(已切,幂等)"), OpKind::Split));
                }
                // 越界判定:t_ms 不在片段内部一律拒绝
                if t_ms <= clip.start_ms || t_ms >= end {
                    return Err(Reject::SplitOutside { clip_id, t_ms });
                }
                let path = clips_pointer(ti);
                let before = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                let mut left = p.tracks[ti].clips[ci].clone();
                let mut right = left.clone();
                left.duration_ms = t_ms - left.start_ms;
                right.id = Project::next_clip_id(&p.tracks[ti]);
                right.start_ms = t_ms;
                right.duration_ms = end - t_ms;
                right.source_in_ms = right.source_in_ms.map(|s| s + (t_ms - clip.start_ms));
                right.text = None; // 文本归属左段
                p.tracks[ti].clips[ci] = left;
                p.tracks[ti].clips.insert(ci + 1, right);
                let after = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                Ok((path, before.clone(), after, format!("clip_split {clip_id}@{t_ms}"), OpKind::Split))
            }
            Command::ClipDelete { clip_id } => {
                let (ti, ci) = p.find_clip(&clip_id).ok_or(Reject::UnknownClip(clip_id.clone()))?;
                let path = clips_pointer(ti);
                let before = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                p.tracks[ti].clips.remove(ci);
                let after = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                Ok((path, before, after, format!("clip_delete {clip_id}"), OpKind::Delete))
            }
            Command::ClipMove { clip_id, new_start_ms, to_track } => match to_track {
                None => {
                    let (ti, ci) = p.find_clip(&clip_id).ok_or(Reject::UnknownClip(clip_id.clone()))?;
                    let path = clip_pointer(ti, ci);
                    let before = serde_json::to_value(&p.tracks[ti].clips[ci]).unwrap();
                    let mut clip = p.tracks[ti].clips[ci].clone();
                    clip.start_ms = new_start_ms;
                    p.tracks[ti].clips[ci] = clip;
                    let after = serde_json::to_value(&p.tracks[ti].clips[ci]).unwrap();
                    enforce_no_overlap(p, ti)?;
                    Ok((path, before, after, format!("clip_move {clip_id}→{new_start_ms}ms"), OpKind::Move))
                }
                Some(track_id) => {
                    let ti = p.find_track(&track_id).ok_or(Reject::UnknownTrack(track_id.clone()))?;
                    let (src_ti, ci) = p.find_clip(&clip_id).ok_or(Reject::UnknownClip(clip_id.clone()))?;
                    if p.tracks[src_ti].kind != p.tracks[ti].kind {
                        return Err(Reject::InvariantViolation(format!(
                            "跨轨移动必须同 kind:{}→{}",
                            p.tracks[src_ti].kind.kind_json(), p.tracks[ti].kind.kind_json())));
                    }
                    let path = "/tracks".to_string();
                    let before = serde_json::to_value(&p.tracks).unwrap();
                    let mut clip = p.tracks[src_ti].clips.remove(ci);
                    clip.start_ms = new_start_ms;
                    clip.id = Project::next_clip_id(&p.tracks[ti]);
                    p.tracks[ti].clips.push(clip);
                    let after = serde_json::to_value(&p.tracks).unwrap();
                    enforce_no_overlap(p, ti)?;
                    Ok((path, before, after, format!("clip_move {clip_id}→{track_id}@{new_start_ms}ms"), OpKind::Move))
                }
            },
            Command::ClipInsert { to_track, clip, .. } => {
                let ti = p.find_track(&to_track).ok_or(Reject::UnknownTrack(to_track.clone()))?;
                if p.tracks.iter().any(|t| t.clips.iter().any(|c| c.id == clip.id)) {
                    return Err(Reject::DuplicateClipId(clip.id));
                }
                let path = clips_pointer(ti);
                let before = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                p.tracks[ti].clips.push(clip.clone());
                let after = serde_json::to_value(&p.tracks[ti].clips).unwrap();
                enforce_no_overlap(p, ti)?;
                Ok((path, before, after, format!("clip_insert {}→{to_track}", clip.id), OpKind::Insert))
            }
            Command::ClipMerge { left_id, right_id } => {
                let (lt, li) = p.find_clip(&left_id).ok_or(Reject::UnknownClip(left_id.clone()))?;
                let (rt, ri) = p.find_clip(&right_id).ok_or(Reject::UnknownClip(right_id.clone()))?;
                if lt != rt || ri != li + 1 {
                    return Err(Reject::NotAdjacent { left_id, right_id });
                }
                let left_end = p.tracks[lt].clips[li].start_ms + p.tracks[lt].clips[li].duration_ms;
                if left_end != p.tracks[rt].clips[ri].start_ms {
                    return Err(Reject::NotAdjacent { left_id, right_id });
                }
                let path = clips_pointer(lt);
                let before = serde_json::to_value(&p.tracks[lt].clips).unwrap();
                let right_end = p.tracks[rt].clips[ri].start_ms + p.tracks[rt].clips[ri].duration_ms;
                p.tracks[lt].clips[li].duration_ms = right_end - p.tracks[lt].clips[li].start_ms;
                p.tracks[lt].clips.remove(ri);
                let after = serde_json::to_value(&p.tracks[lt].clips).unwrap();
                Ok((path, before, after, format!("clip_merge {left_id}+{right_id}"), OpKind::Merge))
            }
        }
    }
}

fn clip_pointer(ti: usize, ci: usize) -> String {
    format!("/tracks/{ti}/clips/{ci}")
}

fn clips_pointer(ti: usize) -> String {
    format!("/tracks/{ti}/clips")
}

/// 在 Project 上按 JSON Pointer 路径设值(undo/redo/replay 的通用机制)。
fn apply_value_at(project: &mut Project, pointer: &str, value: Value) -> Result<(), String> {
    let mut v = serde_json::to_value(&*project).map_err(|e| e.to_string())?;
    let segs: Vec<&str> = pointer.trim_start_matches('/').split('/').filter(|s| !s.is_empty()).collect();
    let mut payload = Some(value);
    let mut cur: &mut serde_json::Value = &mut v;
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        if last {
            match cur {
                serde_json::Value::Object(m) => {
                    m.insert((*seg).to_string(), payload.take().unwrap_or(serde_json::Value::Null));
                }
                serde_json::Value::Array(a) => {
                    let idx: usize = seg.parse().map_err(|_| format!("指针段 '{seg}' 非数组下标"))?;
                    if idx >= a.len() {
                        return Err(format!("指针 '{pointer}' 越界({idx} ≥ {})", a.len()));
                    }
                    a[idx] = payload.take().unwrap_or(serde_json::Value::Null);
                }
                _ => return Err(format!("指针 '{pointer}' 终点不是容器")),
            }
        } else {
            cur = match cur {
                serde_json::Value::Object(m) => {
                    m.get_mut(*seg).ok_or_else(|| format!("指针 '{pointer}' 缺键 '{seg}'"))?
                }
                serde_json::Value::Array(a) => {
                    let idx: usize = seg.parse().map_err(|_| format!("指针段 '{seg}' 非数组下标"))?;
                    if idx >= a.len() {
                        return Err(format!("指针 '{pointer}' 越界({idx} ≥ {})", a.len()));
                    }
                    &mut a[idx]
                }
                _ => return Err(format!("指针 '{pointer}' 中段 '{seg}' 处不是容器")),
            };
        }
    }
    *project = serde_json::from_value(v).map_err(|e| format!("回放反序列化失败: {e}"))?;
    Ok(())
}

fn enforce_no_overlap(p: &Project, ti: usize) -> Result<(), Reject> {
    let ov = Project::overlaps(&p.tracks[ti]);
    if ov.is_empty() {
        Ok(())
    } else {
        Err(Reject::InvariantViolation(format!(
            "同轨时间重叠(CF-004): {:?}", ov)))
    }
}

/// 按日志语义重建撤销栈(Undo 弹栈、Redo 压回;io 打开工程与 replay 共用)。
/// auto 类 Op(锚点重定位等自动簿记)不入栈——撤销深度 = 真实用户手势数(ADR-0001)。
pub fn rebuild_undo_stack(ops: &[Op]) -> Vec<String> {
    let mut undo_stack: Vec<String> = Vec::new();
    let mut redo_stack: Vec<String> = Vec::new();
    for op in ops {
        match op.op_kind {
            OpKind::Undo => {
                if let Some(x) = undo_stack.pop() {
                    redo_stack.push(x);
                }
            }
            OpKind::Redo => {
                if let Some(x) = redo_stack.pop() {
                    undo_stack.push(x);
                }
            }
            _ if op.auto == Some(true) => {}
            _ => undo_stack.push(op.op_id.clone()),
        }
    }
    undo_stack
}

/// 规范化 JSON 文本(键序无关,数值保持 Value 语义)。
pub fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", serde_json::to_string(k).unwrap(), canonical_json(&m[k])))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(a) => {
            let inner: Vec<String> = a.iter().map(canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        other => other.to_string(),
    }
}

impl crate::model::TrackKind {
    fn kind_json(&self) -> &'static str {
        match self {
            crate::model::TrackKind::Video => "video",
            crate::model::TrackKind::Audio => "audio",
            crate::model::TrackKind::Text => "text",
        }
    }
}

/// 便捷:构造测试样本工程。
pub fn sample_project() -> Project {
    let v = json!({
        "version": 1, "schemaVersion": "2.0.0",
        "slug": "engine-样本", "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "tracks": [
            {"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 8400,
                 "sourceInMs": 12000, "role": "voice", "volume": 1.0},
                {"id": "V1-002", "src": "a.mp4", "startMs": 8400, "durationMs": 6200}
            ]},
            {"id": "A1", "kind": "audio", "clips": [
                {"id": "A1-001", "src": "sfx.mp3", "startMs": 8400, "durationMs": 400,
                 "role": "sfx", "volume": 0.8}
            ]}
        ]
    });
    Project::from_value(&v).expect("样本工程必须合法")
}

/// 命令便捷构造(测试用)。
pub fn patch(duration_ms: u64) -> ClipPatch {
    ClipPatch { duration_ms: Some(duration_ms), ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oplog::ActorKind;

    fn agent() -> Actor {
        Actor::agent("test")
    }

    #[test]
    fn apply_update_sets_rev_and_op() {
        let mut eng = Engine::new(sample_project()).unwrap();
        let r = eng.apply(Command::ClipUpdate { clip_id: "V1-001".into(), patch: patch(8000) }, agent(), ApplyOpts::default()).unwrap();
        assert_eq!(r.rev, 1);
        assert_eq!(eng.rev(), 1);
        let clip = eng.query(Query::Clip { id: "V1-001".into() });
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
        let before = eng.query(Query::Clip { id: "V1-001".into() });
        // 移动 V1-001 到 9000ms(与 V1-002[8400..14600] 重叠)→ 必须拒绝
        let r = eng.apply(Command::ClipUpdate { clip_id: "V1-001".into(), patch: ClipPatch { start_ms: Some(9000), duration_ms: Some(2000), ..Default::default() } }, agent(), ApplyOpts::default());
        assert!(matches!(r, Err(Reject::InvariantViolation(_))));
        assert_eq!(eng.query(Query::Clip { id: "V1-001".into() }), before, "失败必须回滚");
        assert_eq!(eng.oplog().len(), 0, "失败不得产生 Op");
        assert_eq!(eng.rev(), 0);
    }

    #[test]
    fn empty_patch_rejected() {
        let mut eng = Engine::new(sample_project()).unwrap();
        let r = eng.apply(Command::ClipUpdate { clip_id: "V1-001".into(), patch: ClipPatch::default() }, agent(), ApplyOpts::default());
        assert!(matches!(r, Err(Reject::EmptyPatch(_))));
    }

    #[test]
    fn precondition_and_unknown_targets() {
        let mut eng = Engine::new(sample_project()).unwrap();
        let r = eng.apply(Command::ClipDelete { clip_id: "V1-001".into() }, agent(), ApplyOpts { expect_rev: Some(5), ..Default::default() });
        assert!(matches!(r, Err(Reject::PreconditionFailed { expected: 5, actual: 0 })));
        let r = eng.apply(Command::ClipDelete { clip_id: "不存在".into() }, agent(), ApplyOpts::default());
        assert!(matches!(r, Err(Reject::UnknownClip(_))));
        let r = eng.apply(Command::ClipInsert { to_track: "T9".into(), clip: sample_project().tracks[0].clips[0].clone(), request_id: None }, agent(), ApplyOpts::default());
        assert!(matches!(r, Err(Reject::UnknownTrack(_))));
    }

    #[test]
    fn split_merge_and_idempotent_split() {
        let mut eng = Engine::new(sample_project()).unwrap();
        let r = eng.apply(Command::ClipSplit { clip_id: "V1-001".into(), t_ms: 4000 }, agent(), ApplyOpts::default()).unwrap();
        assert_eq!(r.rev, 1);
        match eng.query(Query::Timeline) {
            Answer::Timeline(tl) => {
                assert_eq!(tl.len(), 4, "V1 三个片段 + A1 一个音效");
                assert!(tl.iter().any(|(id, s, e, _)| id == "V1-003" && *s == 4000 && *e == 8400));
            }
            other => panic!("意外: {other:?}"),
        }
        // 再切同一点:V1-003 起点已是 4000 → 幂等
        let r2 = eng.apply(Command::ClipSplit { clip_id: "V1-001".into(), t_ms: 4000 }, agent(), ApplyOpts::default()).unwrap();
        assert_eq!(r2.rev, 1, "幂等切分不得再升 rev");
        // 合并回去
        let r3 = eng.apply(Command::ClipMerge { left_id: "V1-001".into(), right_id: "V1-003".into() }, agent(), ApplyOpts::default()).unwrap();
        assert_eq!(r3.rev, 2);
        match eng.query(Query::Clip { id: "V1-001".into() }) {
            Answer::Clip(Some(c)) => assert_eq!(c["durationMs"], json!(8400)),
            other => panic!("意外: {other:?}"),
        }
        // 非相邻合并拒绝:先重切出中间片段,V1-001 与 V1-002 隔着它
        let _ = eng.apply(Command::ClipSplit { clip_id: "V1-001".into(), t_ms: 4000 }, agent(), ApplyOpts::default()).unwrap();
        let r4 = eng.apply(Command::ClipMerge { left_id: "V1-001".into(), right_id: "V1-002".into() }, agent(), ApplyOpts::default());
        assert!(matches!(r4, Err(Reject::NotAdjacent { .. })));
        // 切分越界拒绝(5000 不是既有边界,V1-001 已是 [0..4000])
        let r5 = eng.apply(Command::ClipSplit { clip_id: "V1-001".into(), t_ms: 5000 }, agent(), ApplyOpts::default());
        assert!(matches!(r5, Err(Reject::SplitOutside { .. })));
    }

    #[test]
    fn delete_insert_move_and_request_id_dedup() {
        let mut eng = Engine::new(sample_project()).unwrap();
        let r = eng.apply(Command::ClipDelete { clip_id: "A1-001".into() }, agent(), ApplyOpts::default()).unwrap();
        assert_eq!(r.rev, 1);
        let mut clip = sample_project().tracks[1].clips[0].clone();
        clip.id = "A1-002".into();
        let opts = ApplyOpts { request_id: Some("req-1".into()), ..Default::default() };
        let r2 = eng.apply(Command::ClipInsert { to_track: "A1".into(), clip, request_id: Some("req-1".into()) }, agent(), opts).unwrap();
        assert!(!r2.idempotent);
        assert_eq!(eng.rev(), 2);
        // 同 request_id 再来 → 幂等回执,rev 不动
        let clip2 = { let mut c = sample_project().tracks[1].clips[0].clone(); c.id = "A1-003".into(); c };
        let r3 = eng.apply(Command::ClipInsert { to_track: "A1".into(), clip: clip2, request_id: Some("req-1".into()) }, agent(), ApplyOpts { request_id: Some("req-1".into()), ..Default::default() }).unwrap();
        assert!(r3.idempotent);
        assert_eq!(eng.rev(), 2);
        // 跨 kind 移动拒绝
        let r4 = eng.apply(Command::ClipMove { clip_id: "A1-002".into(), new_start_ms: 0, to_track: Some("V1".into()) }, agent(), ApplyOpts::default());
        assert!(matches!(r4, Err(Reject::InvariantViolation(_))));
    }

    #[test]
    fn duplicate_clip_id_rejected() {
        let mut eng = Engine::new(sample_project()).unwrap();
        let clip = sample_project().tracks[0].clips[0].clone();
        let r = eng.apply(Command::ClipInsert { to_track: "V1".into(), clip, request_id: None }, agent(), ApplyOpts::default());
        assert!(matches!(r, Err(Reject::DuplicateClipId(_))));
    }

    #[test]
    fn oplog_tail_filters() {
        let mut eng = Engine::new(sample_project()).unwrap();
        eng.apply(Command::ClipDelete { clip_id: "A1-001".into() }, Actor::user("用户"), ApplyOpts::default()).unwrap();
        eng.apply(Command::ClipDelete { clip_id: "V1-002".into() }, Actor::agent("AI"), ApplyOpts::default()).unwrap();
        match eng.query(Query::OpLogTail { since_rev: Some(0), actor_kind: Some(ActorKind::Agent) }) {
            Answer::Ops(ops) => {
                assert_eq!(ops.len(), 1);
                assert_eq!(ops[0].actor.id, "AI");
            }
            other => panic!("意外: {other:?}"),
        }
        match eng.query(Query::OpLogTail { since_rev: None, actor_kind: None }) {
            Answer::Ops(ops) => assert_eq!(ops.len(), 2),
            other => panic!("意外: {other:?}"),
        }
    }

    #[test]
    fn caused_by_and_summary_recorded() {
        let mut eng = Engine::new(sample_project()).unwrap();
        eng.apply(
            Command::ClipUpdate { clip_id: "V1-001".into(), patch: patch(8000) },
            agent(),
            ApplyOpts { caused_by: vec!["n-0001".into()], summary: Some("按标注缩短".into()), ..Default::default() },
        ).unwrap();
        let op = &eng.oplog().ops()[0];
        assert_eq!(op.caused_by.as_deref(), Some(&["n-0001".to_string()][..]));
        assert_eq!(op.summary, "按标注缩短");
    }
}
