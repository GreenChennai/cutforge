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
pub mod watcher;

use cutforge_core::command::Command;
use cutforge_core::engine::{ApplyOpts, Engine, OpReceipt};
use cutforge_core::model::Project;
use cutforge_core::oplog::{Actor, Op, OpLog};
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
        let undo_stack: Vec<String> = log.ops().iter()
            .filter(|o| !matches!(o.op_kind, cutforge_core::oplog::OpKind::Undo | cutforge_core::oplog::OpKind::Redo))
            .map(|o| o.op_id.clone())
            .collect();
        let persisted = log.len();
        let engine = Engine::restore(project, log, rev, undo_stack)
            .map_err(|errs| io::Error::other(errs.join("; ")))?;
        Ok(Self { root: root.to_path_buf(), engine, persisted })
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

    /// 命令接口(唯一写入口的 IO 编排,4.2 八步)。
    pub fn apply(&mut self, cmd: Command, actor: Actor, opts: ApplyOpts) -> io::Result<OpReceipt> {
        let _guard = lock::acquire(&self.root, 30_000, 20)?;
        let receipt = self.engine.apply(cmd, actor, opts).map_err(reject_to_io)?;
        self.persist()?;
        Ok(receipt)
    }

    pub fn undo(&mut self, actor: Actor) -> io::Result<OpReceipt> {
        let _guard = lock::acquire(&self.root, 30_000, 20)?;
        let receipt = self.engine.undo(actor).map_err(reject_to_io)?;
        self.persist()?;
        Ok(receipt)
    }

    pub fn redo(&mut self, actor: Actor) -> io::Result<OpReceipt> {
        let _guard = lock::acquire(&self.root, 30_000, 20)?;
        let receipt = self.engine.redo(actor).map_err(reject_to_io)?;
        self.persist()?;
        Ok(receipt)
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
        Ok(())
    }

    /// 三路合并(M2 骨架的直接消费入口):磁盘文件与内存模型按 baseRev 合并。
    /// 返回 Ok(Some(rev)) 表示已合并采纳;Ok(None) 表示无差异;冲突以 Err 返回。
    pub fn merge_from_disk(&mut self) -> Result<Option<u64>, Vec<cutforge_core::merge::Conflict>> {
        let disk_text = match std::fs::read_to_string(self.root.join(PROJECT_REL)) {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };
        let disk: serde_json::Value = serde_json::from_str(&disk_text).map_err(|e| {
            vec![cutforge_core::merge::Conflict {
                code: cutforge_core::merge::ConflictCode::FieldConflict,
                pointer: "$".into(),
                base: None,
                disk: Some(serde_json::Value::String(e.to_string())),
                local: None,
            }]
        })?;
        let local = match self.engine.query(cutforge_core::engine::Query::ProjectView) {
            cutforge_core::engine::Answer::Project(v) => v,
            _ => unreachable!(),
        };
        // 共同祖先 = 本地当前状态在最近一次外部读取时的样子;M2 骨架以 rev 记录的
        // 快照占位(完整 baseRev 快照链在 M3 引入)。此处以"本地"同时充当 base,
        // 行为等价于"磁盘无本地未见的修改时无操作;有修改则以磁盘为冲突候选"。
        let base = local.clone();
        match cutforge_core::merge::three_way_merge(&base, &disk, &local) {
            cutforge_core::merge::MergeOutcome::Merged(v) => {
                if v == local {
                    Ok(None)
                } else {
                    let project = Project::from_value(&v).map_err(|errs| {
                        vec![cutforge_core::merge::Conflict {
                            code: cutforge_core::merge::ConflictCode::FieldConflict,
                            pointer: "$".into(),
                            disk: Some(v),
                            local: None,
                            base: Some(serde_json::Value::String(errs.join("; "))),
                        }]
                    })?;
                    self.engine = Engine::new(project).map_err(|errs| {
                        vec![cutforge_core::merge::Conflict {
                            code: cutforge_core::merge::ConflictCode::FieldConflict,
                            pointer: "$".into(),
                            base: None,
                            disk: None,
                            local: Some(serde_json::Value::String(errs.join("; "))),
                        }]
                    })?;
                    self.persisted = self.engine.oplog().len();
                    let _ = self.persist();
                    Ok(Some(self.engine.rev()))
                }
            }
            cutforge_core::merge::MergeOutcome::Conflicts(c) => Err(c),
        }
    }
}

fn reject_to_io(r: cutforge_core::engine::Reject) -> io::Error {
    use cutforge_core::engine::Reject::*;
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
