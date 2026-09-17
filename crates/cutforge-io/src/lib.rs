//! CutForge IO 层:Workspace 把"文件真相源"(计划书 4.2)与内核命令通道接在一起。
//!
//! 唯一写入路径(4.2 八步,本文件为编排者;所有落盘经 `atomic::atomic_write`):
//! 1. 申请工程锁 → 2/3. Engine 校验前置与不变量 → 4. Op 追加 oplog →
//! 5. 新内容生成 → 6. 原子写入 project.json → 7. rev 落盘 → 8. 释放锁。
//! `.cutforge/` 可安全删除重建(仅丢同步历史)——`workspace_state_rebuildable` 测试保证。

pub mod atomic;
pub mod backup;
pub mod fsutil;
pub mod lock;
pub mod probe;
pub mod stage;
pub mod watcher;

use cutforge_core::command::Command;
use cutforge_core::engine::{ApplyOpts, Engine, OpReceipt};
use cutforge_core::merge::{Conflict, ConflictCode, MergeOutcome};
use cutforge_core::model::Project;
use cutforge_core::notes::{Note, NoteAuthor, NoteReject, NotesStore};
use cutforge_core::oplog::{Actor, Op, OpKind, OpLog};
use cutforge_core::anchor::Anchor;
use std::io;
use std::path::{Path, PathBuf};

pub const PROJECT_REL: &str = "05_ir/project.json";
pub const WORDLINE_REL: &str = "05_ir/wordline.json";
pub const CUTLIST_REL: &str = "04_cut/cutlist.json";
pub const NOTES_REL: &str = "notes.json";

/// 测试夹具:从仓库回归样本搭建完整工程目录(v1 形态,顺带覆盖迁移路径)。
#[doc(hidden)]
pub fn tests_fixture(tag: &str) -> io::Result<PathBuf> {
    let sample =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/regression/talking-head");
    let root = fsutil::temp_dir(tag);
    fsutil::ensure(&root.join("05_ir"))?;
    fsutil::ensure(&root.join("04_cut"))?;
    fsutil::copy_file(&sample.join("project.json"), &root.join(PROJECT_REL))?;
    fsutil::copy_file(&sample.join("wordline.json"), &root.join(WORDLINE_REL))?;
    fsutil::copy_file(&sample.join("cutlist.json"), &root.join(CUTLIST_REL))?;
    fsutil::copy_file(&sample.join("notes.json"), &root.join(NOTES_REL))?;
    Ok(root)
}

pub struct Workspace {
    root: PathBuf,
    engine: Engine,
    /// 已持久化的 Op 数(oplog 追增用)。
    persisted: usize,
    notes: NotesStore,
    notes_dirty: bool,
}

impl Workspace {
    /// 打开工程目录:读取 → v1 迁移 → 校验 → 恢复 OpLog/rev(含崩溃修复,4.5)。
    pub fn open(root: &Path) -> io::Result<Self> {
        let project_path = root.join(PROJECT_REL);
        let text = std::fs::read_to_string(&project_path).map_err(|e| {
            io::Error::new(e.kind(), format!("打开工程失败({project_path:?}): {e}"))
        })?;
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| io::Error::other(format!("project.json 非法 JSON: {e}")))?;
        let project = Project::from_value(&value)
            .or_else(|_| cutforge_core::model::migrate_from_value(&value))
            .map_err(|errs| io::Error::other(format!("project.json 未通过 v2 契约: {}", errs.join("; "))))?;

        // OpLog 恢复(按天分文件,文件名升序 = 时间升序)
        let mut log = OpLog::new();
        let oplog_dir = root.join(".cutforge/oplog");
        if let Ok(entries) = std::fs::read_dir(&oplog_dir) {
            let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect();
            files.sort();
            for file in files {
                let content = std::fs::read_to_string(&file)?;
                for line in content.lines().filter(|l| !l.trim().is_empty()) {
                    match serde_json::from_str::<Op>(line) {
                        Ok(op) => {
                            log.push_loaded(op);
                        }
                        Err(e) => {
                            // 半行(上次写入中断):截断处之后不再可信,停止加载
                            eprintln!("oplog 半行(截断恢复): {file:?}: {e}");
                            break;
                        }
                    }
                }
            }
        }

        // rev 对账:文件 rev < oplog rev → 上次写入中断,以 oplog 为准修复
        let rev_path = root.join(".cutforge/rev");
        let mut rev = log.last_rev().unwrap_or(0);
        if let Ok(text) = std::fs::read_to_string(&rev_path) {
            if let Ok(disk_rev) = text.trim().parse::<u64>() {
                if disk_rev > rev {
                    rev = disk_rev;
                }
            }
        }
        let undo_stack: Vec<String> = cutforge_core::engine::rebuild_undo_stack(log.ops());
        let persisted = log.len();
        let engine = Engine::restore(project, log, rev, undo_stack)
            .map_err(|errs| io::Error::other(errs.join("; ")))?;

        // notes.json(标注):缺失 = 空存储;存在则必须过 notes.schema
        let (notes, notes_dirty) = match std::fs::read_to_string(root.join(NOTES_REL)) {
            Ok(text) => {
                let v: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| io::Error::other(format!("notes.json 非法 JSON: {e}")))?;
                let store = NotesStore::from_value(&v)
                    .map_err(|errs| io::Error::other(format!("CF-005 SCHEMA_DRIFT(notes.json): {}", errs.join("; "))))?;
                (store, false)
            }
            Err(_) => (NotesStore::new(), false),
        };
        Ok(Self { root: root.to_path_buf(), engine, persisted, notes, notes_dirty })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn rev(&self) -> u64 {
        self.engine.rev()
    }

    /// 命令接口(唯一写入口的 IO 编排,4.2 八步);成功后联动标注重定位(4.9)。
    pub fn apply(&mut self, cmd: Command, actor: Actor, opts: ApplyOpts) -> io::Result<OpReceipt> {
        let _guard = lock::acquire(&self.root, 30_000, 20)?;
        let before_project = self.engine.project().clone();
        let receipt = self.engine.apply(cmd, actor.clone(), opts).map_err(reject_to_io)?;
        self.sync_notes_after_change(&before_project, actor)?;
        self.persist()?;
        Ok(receipt)
    }

    pub fn undo(&mut self, actor: Actor) -> io::Result<OpReceipt> {
        let _guard = lock::acquire(&self.root, 30_000, 20)?;
        let before_project = self.engine.project().clone();
        let receipt = self.engine.undo(actor.clone()).map_err(reject_to_io)?;
        self.sync_notes_after_change(&before_project, actor)?;
        self.persist()?;
        Ok(receipt)
    }

    pub fn redo(&mut self, actor: Actor) -> io::Result<OpReceipt> {
        let _guard = lock::acquire(&self.root, 30_000, 20)?;
        let before_project = self.engine.project().clone();
        let receipt = self.engine.redo(actor.clone()).map_err(reject_to_io)?;
        self.sync_notes_after_change(&before_project, actor)?;
        self.persist()?;
        Ok(receipt)
    }

    // ---------- 标注(4.9) ----------

    pub fn notes(&self) -> &NotesStore {
        &self.notes
    }

    /// 创建标注(user 提需求 / agent 反向提问),写入 notes.json 并登记 Op。
    pub fn notes_add(
        &mut self,
        anchor: Anchor,
        body: String,
        author: NoteAuthor,
        tags: Vec<String>,
        actor: Actor,
    ) -> io::Result<String> {
        let before = self.notes.to_value();
        let note_id = self.notes.next_id();
        self.notes.add(anchor, body, author, tags);
        self.record_notes_change(&before, OpKind::Insert, actor, format!("创建标注 {note_id}"), None)?;
        Ok(note_id)
    }

    /// 结案回执(绑定 opIds;幂等)。
    pub fn notes_resolve(
        &mut self,
        note_id: &str,
        reply: String,
        op_ids: Vec<String>,
        actor: Actor,
    ) -> io::Result<()> {
        let before = self.notes.to_value();
        match self.notes.resolve(note_id, reply, op_ids) {
            Ok(_) => {}
            Err(NoteReject::UnknownNote(_)) => {
                return Err(io::Error::new(io::ErrorKind::NotFound, format!("标注不存在: {note_id}")))
            }
            Err(_) => return Ok(()), // 已按相同内容结案 → 幂等
        }
        self.record_notes_change(&before, OpKind::Set, actor, format!("标注 {note_id} 结案"), None)?;
        Ok(())
    }

    pub fn notes_reject(&mut self, note_id: &str, reason: String, actor: Actor) -> io::Result<()> {
        let before = self.notes.to_value();
        self.notes.reject(note_id, reason).map_err(|e| io::Error::new(io::ErrorKind::NotFound, format!("{e:?}")))?;
        self.record_notes_change(&before, OpKind::Set, actor, format!("标注 {note_id} 否决"), None)?;
        Ok(())
    }

    /// apply/undo/redo 成功后:open 态标注按 3.6 规则重定位;有变化则落盘并留痕。
    fn sync_notes_after_change(&mut self, before_project: &Project, actor: Actor) -> io::Result<()> {
        let _ = before_project;
        let before = self.notes.to_value();
        let (moved, orphaned) = self.notes.relocate_all(self.engine.project(), 500);
        if moved > 0 || orphaned > 0 {
            self.record_notes_change(
                &before,
                OpKind::Set,
                actor,
                format!("锚点重定位:跟随/重挂 {moved},转孤儿 {orphaned}"),
                None,
            )?;
        } else {
            self.notes_dirty = false;
        }
        Ok(())
    }

    fn record_notes_change(
        &mut self,
        before: &serde_json::Value,
        kind: OpKind,
        actor: Actor,
        summary: String,
        caused_by: Option<Vec<String>>,
    ) -> io::Result<()> {
        let after = self.notes.to_value();
        let opts = ApplyOpts { caused_by: caused_by.unwrap_or_default(), ..Default::default() };
        self.engine
            .record_file_change("notes.json", "/items", before.clone(), after, kind, actor, opts)
            .map_err(reject_to_io)?;
        self.notes_dirty = true;
        self.persist()?;
        Ok(())
    }

    // ---------- 冲突(4.7) ----------

    /// 当前持久化的冲突清单。
    pub fn conflict_list(&self) -> io::Result<Vec<(String, Conflict)>> {
        let dir = self.root.join(".cutforge/conflicts");
        let mut out = Vec::new();
        if !dir.is_dir() {
            return Ok(out);
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        files.sort();
        for f in files {
            let text = std::fs::read_to_string(&f)?;
            let v: serde_json::Value = serde_json::from_str(&text)?;
            let id = f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let conflict = serde_json::from_value::<PersistedConflict>(v)?.into_conflict();
            out.push((id, conflict));
        }
        Ok(out)
    }

    /// 三路合并(M2 骨架的直接消费入口):磁盘文件与内存模型按 baseRev 合并。
    /// 冲突时写入 `.cutforge/conflicts/<id>.json` 三方快照并停写(4.7)。
    pub fn merge_from_disk(&mut self) -> Result<Option<u64>, Vec<(String, Conflict)>> {
        let disk_text = match std::fs::read_to_string(self.root.join(PROJECT_REL)) {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };
        let disk: serde_json::Value = match serde_json::from_str(&disk_text) {
            Ok(v) => v,
            Err(e) => {
                let c = Conflict {
                    code: ConflictCode::FieldConflict,
                    pointer: "$".into(),
                    base: None,
                    disk: Some(serde_json::Value::String(format!("CF-005 SCHEMA_DRIFT: {e}"))),
                    local: None,
                };
                let id = self.persist_conflict(&c);
                return Err(vec![(id, c)]);
            }
        };
        let local = match self.engine.query(cutforge_core::engine::Query::ProjectView) {
            cutforge_core::engine::Answer::Project(v) => v,
            _ => unreachable!(),
        };
        // M2 骨架以"本地"同时充当 base;完整 baseRev 快照链在 M4 引入。
        let base = local.clone();
        match cutforge_core::merge::three_way_merge(&base, &disk, &local) {
            MergeOutcome::Merged(v) => {
                if v == local {
                    Ok(None)
                } else {
                    let project = match Project::from_value(&v) {
                        Ok(p) => p,
                        Err(errs) => {
                            let c = Conflict {
                                code: ConflictCode::FieldConflict,
                                pointer: "$".into(),
                                disk: Some(v),
                                local: None,
                                base: Some(serde_json::Value::String(format!("CF-005 SCHEMA_DRIFT: {}", errs.join("; ")))),
                            };
                            let id = self.persist_conflict(&c);
                            return Err(vec![(id, c)]);
                        }
                    };
                    let new_engine = match Engine::new(project) {
                        Ok(e) => e,
                        Err(errs) => {
                            let c = Conflict {
                                code: ConflictCode::FieldConflict,
                                pointer: "$".into(),
                                base: None,
                                disk: None,
                                local: Some(serde_json::Value::String(format!("CF-005 SCHEMA_DRIFT: {}", errs.join("; ")))),
                            };
                            let id = self.persist_conflict(&c);
                            return Err(vec![(id, c)]);
                        }
                    };
                    self.engine = new_engine;
                    self.notes = NotesStore::new();
                    self.persisted = self.engine.oplog().len();
                    let _ = self.persist();
                    Ok(Some(self.engine.rev()))
                }
            }
            MergeOutcome::Conflicts(conflicts) => {
                let persisted: Vec<(String, Conflict)> = conflicts
                    .iter()
                    .map(|c| (self.persist_conflict(c), c.clone()))
                    .collect();
                Err(persisted)
            }
        }
    }

    /// 冲突三方快照落盘(4.7:冲突产生时在任何写入发生之前停止)。
    fn persist_conflict(&self, c: &Conflict) -> String {
        let id = format!(
            "cf-{}-{}",
            self.engine.rev(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let payload = serde_json::json!({
            "conflictId": id,
            "code": c.code.code(),
            "pointer": c.pointer,
            "base": c.base,
            "disk": c.disk,
            "local": c.local,
            "createdAt": cutforge_core::timeutil::now_rfc3339(),
        });
        let dir = self.root.join(".cutforge/conflicts");
        let _ = std::fs::create_dir_all(&dir);
        let _ = atomic::atomic_write(&dir.join(format!("{id}.json")), payload.to_string().as_bytes());
        id
    }

    /// 步骤 4-7:备份 → 原子写 project.json → 追加 oplog → rev 落盘。
    fn persist(&mut self) -> io::Result<()> {
        let ops = self.engine.oplog().ops();
        // 备份旧 project.json(全局约定 B.8:可回滚)
        if self.persisted == 0 || self.persisted < ops.len() {
            if let Ok(old) = std::fs::read(self.root.join(PROJECT_REL)) {
                backup::backup_file(&self.root, PROJECT_REL, &old)?;
            }
        }
        let value = self.engine.query(cutforge_core::engine::Query::ProjectView);
        let cutforge_core::engine::Answer::Project(ref v) = value else { unreachable!() };
        let mut buf = serde_json::to_vec_pretty(v)?;
        buf.push(b'\n');
        atomic::atomic_write(&self.root.join(PROJECT_REL), &buf)?;

        // 新 Op 追加 .jsonl(按天切分;append-only)
        let day = today_compact();
        let oplog_file = self.root.join(".cutforge/oplog").join(format!("{day}.jsonl"));
        while self.persisted < ops.len() {
            let op = &ops[self.persisted];
            let line = serde_json::to_string(op)?;
            atomic::append_line(&oplog_file, &format!("{line}\n"))?;
            self.persisted += 1;
        }
            atomic::atomic_write(&self.root.join(".cutforge/rev"), format!("{}\n", self.engine.rev()).as_bytes())?;
        // 标注落盘(有变化才写)
        if self.notes_dirty {
            let mut buf = serde_json::to_vec_pretty(&self.notes.to_value())?;
            buf.push(b'\n');
            atomic::atomic_write(&self.root.join(NOTES_REL), &buf)?;
            self.notes_dirty = false;
        }
        Ok(())
    }
}

/// 持久化冲突快照的磁盘形态(`.cutforge/conflicts/<id>.json`)。
#[derive(serde::Deserialize)]
struct PersistedConflict {
    code: String,
    pointer: String,
    #[serde(default)]
    base: Option<serde_json::Value>,
    #[serde(default)]
    disk: Option<serde_json::Value>,
    #[serde(default)]
    local: Option<serde_json::Value>,
}

impl PersistedConflict {
    fn into_conflict(self) -> Conflict {
        let code = match self.code.as_str() {
            "CF-002" => ConflictCode::DeleteModify,
            "CF-003" => ConflictCode::DupId,
            _ => ConflictCode::FieldConflict,
        };
        Conflict { code, pointer: self.pointer, base: self.base, disk: self.disk, local: self.local }
    }
}

fn reject_to_io(r: cutforge_core::engine::Reject) -> io::Error {    use cutforge_core::engine::Reject::*;
    let (kind, msg) = match r {
        PreconditionFailed { expected, actual } => (
            io::ErrorKind::InvalidInput,
            format!("PRECONDITION_FAILED: 基于 rev-{expected} 的写入已失效(当前 rev-{actual})"),
        ),
        SchemaInvalid(errs) => (io::ErrorKind::InvalidInput, format!("SCHEMA_INVALID: {}", errs.join("; "))),
        InvariantViolation(m) => (io::ErrorKind::InvalidInput, format!("GUARD_FAILED: {m}")),
        NothingToUndo => (io::ErrorKind::InvalidInput, "NOTHING_TO_UNDO".into()),
        NothingToRedo => (io::ErrorKind::InvalidInput, "NOTHING_TO_REDO".into()),
        other => (io::ErrorKind::InvalidInput, format!("{other:?}")),
    };
    io::Error::new(kind, msg)
}

/// UTC 紧凑日期 YYYYMMDD(oplog 按天切分)。
fn today_compact() -> String {
    cutforge_core::timeutil::now_rfc3339()[..10].replace('-', "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil;

    #[test]
    fn open_migrates_v1_and_persists_commands() {
        let root = tests_fixture("ws-open").unwrap();
        let mut ws = Workspace::open(&root).unwrap();
        assert_eq!(ws.rev(), 0);
        // v1 样本经迁移后可正常应用命令
        let r = ws.apply(
            Command::ClipUpdate {
                clip_id: "V1-001".into(),
                patch: cutforge_core::command::ClipPatch { duration_ms: Some(8000), ..Default::default() },
            },
            Actor::agent("io-test"),
            ApplyOpts::default(),
        )
        .unwrap();
        assert_eq!(r.rev, 1);
        assert_eq!(ws.rev(), 1);
        // 落盘验证:project.json/rev/oplog 都存在
        assert!(root.join(".cutforge/rev").exists());
        assert!(root.join(".cutforge/oplog").read_dir().unwrap().next().is_some());
        fsutil::cleanup(&root);
    }

    #[test]
    fn reopen_restores_state_and_undo_stack() {
        let root = tests_fixture("ws-reopen").unwrap();
        {
            let mut ws = Workspace::open(&root).unwrap();
            ws.apply(
                Command::ClipUpdate {
                    clip_id: "V1-001".into(),
                    patch: cutforge_core::command::ClipPatch { duration_ms: Some(8000), ..Default::default() },
                },
                Actor::user("人"),
                ApplyOpts::default(),
            )
            .unwrap();
        }
        let mut ws2 = Workspace::open(&root).unwrap();
        assert_eq!(ws2.rev(), 1, "rev 必须从盘面恢复");
        assert_eq!(ws2.engine().oplog().len(), 1, "OpLog 必须从 jsonl 恢复");
        // 恢复后的撤销栈仍然可用
        ws2.undo(Actor::user("人")).unwrap();
        let disk = std::fs::read_to_string(root.join(PROJECT_REL)).unwrap();
        assert!(disk.contains("8400"), "撤销后盘面应回到 8400");
        fsutil::cleanup(&root);
    }
}
