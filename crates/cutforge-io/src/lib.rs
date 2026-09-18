// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! CutForge IO 层:Workspace 把"文件真相源"(计划书 4.2)与内核命令通道接在一起。
//!
//! 唯一写入路径(4.2 八步,本文件为编排者;所有落盘经 `atomic::atomic_write`):
//! 1. 申请工程锁(`open_exclusive` 可让锁覆盖 open→apply→persist 全程,P0-5)→
//! 2/3. Engine 校验前置与不变量 → 4. 新内容先落盘(先文件后记账)→
//! 5. Op 追加 oplog → 6. rev 落盘 → 7. 释放锁。
//!
//! `_meta` 旁路(ADR-0002):CutFlow 真实 IR 顶层带 `_meta`(实现细节字段,被
//! v2 schema `additionalProperties:false` 拒收)。open 时剥出入内存旁路,
//! persist 时原样回写——CutForge 打得开任何 CutFlow 工程,写回不丢它。
//!
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
use cutforge_core::notes::{NoteAuthor, NotesStore};
use cutforge_core::oplog::{Actor, Op, OpKind, OpLog};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

pub const PROJECT_REL: &str = "05_ir/project.json";
pub const WORDLINE_REL: &str = "05_ir/wordline.json";
pub const CUTLIST_REL: &str = "04_cut/cutlist.json";
pub const CUTLIST_APPLIED_REL: &str = "04_cut/cutlist.applied.json";
pub const NOTES_REL: &str = "notes.json";

/// baseRev 快照目录(M9-1):`.cutforge/bases/<rev>.json` = rev N 时刻的工程视图。
/// 三路合并的共同祖先由此取得;"本地充当 base"的死代码口径退役。
pub const BASES_REL: &str = ".cutforge/bases";
/// 快照保留上限(LRU 按 rev 淘汰,防膨胀;计划书 V2-R4)。
const BASES_KEEP: usize = 32;

/// 非工程真相源文件与其在工程目录内的相对路径(file_states 初始化与落盘的依据)。
const FILE_TRUTHS: [(&str, &str); 3] = [
    ("wordline.json", WORDLINE_REL),
    ("cutlist.json", CUTLIST_REL),
    ("cutlist.applied.json", CUTLIST_APPLIED_REL),
];

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
    /// `_meta` 旁路(ADR-0002):open 时剥出、persist 原样回写。
    meta_bypass: Option<serde_json::Value>,
    /// 非工程真相源文件最近一次落盘值(撤销/登记后 diff 落盘,避免无谓重写)。
    files: BTreeMap<String, serde_json::Value>,
    /// open_exclusive 持有的全程锁(含 Drop 自动释放)。
    lock: Option<lock::LockGuard>,
    /// 最近一次与磁盘同步时的 project 视图(去 `_meta`;写入窗口漂移检测的基准)。
    synced_disk: Option<serde_json::Value>,
}

impl Workspace {
    /// 打开工程目录(只读语义安全;写操作走 `open_exclusive` 或依赖方法内临时锁)。
    pub fn open(root: &Path) -> io::Result<Self> {
        let (engine, persisted, notes, meta_bypass, files, synced_disk) = Self::load(root)?;
        Ok(Self {
            root: root.to_path_buf(), engine, persisted, notes,
            notes_dirty: false, meta_bypass, files, lock: None, synced_disk,
        })
    }

    /// 独占打开:锁覆盖 open→apply→persist 全程(P0-5:跨进程并发写不再丢更新)。
    /// MCP 写通道与 CLI 变更子命令一律走本入口。
    pub fn open_exclusive(root: &Path) -> io::Result<Self> {
        let guard = lock::acquire(root, 30_000, 20)?;
        let (engine, persisted, notes, meta_bypass, files, synced_disk) = Self::load(root)?;
        let mut ws = Self {
            root: root.to_path_buf(), engine, persisted, notes,
            notes_dirty: false, meta_bypass, files, lock: Some(guard), synced_disk,
        };
        // 迁移升级:盘面为旧形态(v1/缺 id)时,独占打开即落规范形,
        // 使后续外部改动检测与守护合并都以 v2 规范形为基准。
        let view = ws.engine.query(cutforge_core::engine::Query::ProjectView);
        let cutforge_core::engine::Answer::Project(ref v) = view else { unreachable!() };
        if Some(v) != ws.synced_disk.as_ref() {
            ws.persist()?;
        }
        Ok(ws)
    }

    #[allow(clippy::type_complexity)]
    fn load(
        root: &Path,
    ) -> io::Result<(Engine, usize, NotesStore, Option<serde_json::Value>, BTreeMap<String, serde_json::Value>, Option<serde_json::Value>)> {
        let project_path = root.join(PROJECT_REL);
        let text = std::fs::read_to_string(&project_path).map_err(|e| {
            io::Error::new(e.kind(), format!("打开工程失败({project_path:?}): {e}"))
        })?;
        let mut value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| io::Error::other(format!("project.json 非法 JSON: {e}")))?;
        // _meta 旁路(ADR-0002):进内存旁路,不进契约校验
        let meta_bypass = value.as_object_mut().and_then(|o| o.remove("_meta"));
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
        if let Ok(text) = std::fs::read_to_string(&rev_path)
            && let Ok(disk_rev) = text.trim().parse::<u64>()
                && disk_rev > rev {
                    rev = disk_rev;
                }
        let (undo_stack, redo_stack) = cutforge_core::engine::rebuild_stacks(log.ops());
        let persisted = log.len();

        // notes.json(标注):缺失 = 空存储;存在则必须过 notes.schema
        let (notes, notes_value) = match std::fs::read_to_string(root.join(NOTES_REL)) {
            Ok(text) => {
                let v: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| io::Error::other(format!("notes.json 非法 JSON: {e}")))?;
                let store = NotesStore::from_value(&v)
                    .map_err(|errs| io::Error::other(format!("CF-005 SCHEMA_DRIFT(notes.json): {}", errs.join("; "))))?;
                (store, v)
            }
            Err(_) => {
                let store = NotesStore::new();
                let v = store.to_value();
                (store, v)
            }
        };

        // 文件态初始化(ADR-0001:文件级 Op 撤销/重做的路由目标)
        let mut file_states: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        for (name, rel) in FILE_TRUTHS {
            if let Ok(text) = std::fs::read_to_string(root.join(rel))
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    file_states.insert(name.to_string(), v);
                }
        }
        file_states.insert("notes.json".to_string(), notes_value);

        let engine = Engine::restore_with_stacks(project, log, rev, undo_stack, redo_stack, file_states)
            .map_err(|errs| io::Error::other(errs.join("; ")))?;

        // 最近落盘值快照(落盘 diff 用)
        let mut files = BTreeMap::new();
        for (name, rel) in FILE_TRUTHS {
            if let Ok(text) = std::fs::read_to_string(root.join(rel))
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    files.insert(name.to_string(), v);
                }
        }
        Ok((engine, persisted, notes, meta_bypass, files, value.clone().into()))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// 当前工程(只读视图)。
    pub fn project(&self) -> &Project {
        self.engine.project()
    }

    pub fn rev(&self) -> u64 {
        self.engine.rev()
    }

    /// 已持有的全程锁之外临时补锁(open_exclusive 打开时不重复加锁)。
    fn temp_lock(&self) -> io::Result<Option<lock::LockGuard>> {
        if self.lock.is_some() {
            Ok(None)
        } else {
            lock::acquire(&self.root, 30_000, 20).map(Some)
        }
    }

    /// 命令接口(唯一写入口的 IO 编排,4.2 八步);成功后联动标注重定位(4.9)。
    pub fn apply(&mut self, cmd: Command, actor: Actor, opts: ApplyOpts) -> io::Result<OpReceipt> {
        let _guard = self.temp_lock()?;
        self.pre_write_sync()?;
        let receipt = self.engine.apply(cmd, actor.clone(), opts).map_err(reject_to_io)?;
        self.reconcile_files()?;
        self.sync_notes_after_change(actor)?;
        self.persist()?;
        Ok(receipt)
    }

    pub fn undo(&mut self, actor: Actor) -> io::Result<OpReceipt> {
        let _guard = self.temp_lock()?;
        self.pre_write_sync()?;
        let receipt = self.engine.undo(actor.clone()).map_err(reject_to_io)?;
        self.reconcile_files()?;
        self.sync_notes_after_change(actor)?;
        self.persist()?;
        Ok(receipt)
    }

    pub fn redo(&mut self, actor: Actor) -> io::Result<OpReceipt> {
        let _guard = self.temp_lock()?;
        self.pre_write_sync()?;
        let receipt = self.engine.redo(actor.clone()).map_err(reject_to_io)?;
        self.reconcile_files()?;
        self.sync_notes_after_change(actor)?;
        self.persist()?;
        Ok(receipt)
    }

    /// 写前同步(M9-1):磁盘与本地已分叉时,先按真祖先三路合并;
    /// 不可自动合并 → 冲突落盘并停写(4.7:任何写入发生之前停止)。
    fn pre_write_sync(&mut self) -> io::Result<()> {
        if !self.conflict_list()?.is_empty() {
            return Err(io::Error::other("CONFLICT: 存在未裁决冲突(.cutforge/conflicts/),停写直至裁决"));
        }
        match self.sync_with_disk() {
            Ok(_) => Ok(()),
            Err(conflicts) => Err(io::Error::other(format!(
                "CONFLICT: 外部改动与本地不可自动合并({} 项),已落 .cutforge/conflicts/",
                conflicts.len()))),
        }
    }

    /// 与磁盘做一次三路合并(base = baseRev 快照链的真祖先;不再"本地充当 base")。
    /// 返回 Ok(true) = 采纳了外部改动;Err = 不可自动合并(冲突已落盘)。
    pub fn sync_with_disk(&mut self) -> Result<bool, Vec<(String, Conflict)>> {
        let disk_text = match std::fs::read_to_string(self.root.join(PROJECT_REL)) {
            Ok(t) => t,
            Err(_) => return Ok(false),
        };
        let mut disk: serde_json::Value = match serde_json::from_str(&disk_text) {
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
        let disk_meta = disk.as_object_mut().and_then(|o| o.remove("_meta"));
        if let Some(m) = disk_meta {
            self.meta_bypass = Some(m);
        }
        if Some(&disk) == self.synced_disk.as_ref() {
            return Ok(false); // 快路径:磁盘与装载时一致,无外部改动
        }
        let local = match self.engine.query(cutforge_core::engine::Query::ProjectView) {
            cutforge_core::engine::Answer::Project(v) => v,
            _ => unreachable!(),
        };
        let base = self.load_base()
            .or_else(|| self.synced_disk.clone())
            .unwrap_or_else(|| local.clone());
        match cutforge_core::merge::three_way_merge(&base, &disk, &local) {
            MergeOutcome::Merged(v) => {
                if v == local {
                    return Ok(false);
                }
                self.adopt_merged(v).map_err(|e| {
                    vec![(format!("cf-adopt-{}", self.engine.rev()), Conflict {
                        code: ConflictCode::FieldConflict,
                        pointer: "$".into(),
                        base: None,
                        disk: None,
                        local: Some(serde_json::Value::String(e.to_string())),
                    })]
                })?;
                Ok(true)
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

    /// 采纳合并结果:保留 OpLog/rev/撤销栈/文件态的历史连续性(不得重置 rev)。
    fn adopt_merged(&mut self, v: serde_json::Value) -> io::Result<()> {
        let project = Project::from_value(&v).map_err(|errs| {
            io::Error::other(format!("CF-005 SCHEMA_DRIFT(合并结果): {}", errs.join("; ")))
        })?;
        let log = self.engine.oplog().clone();
        let (undo_stack, redo_stack) = cutforge_core::engine::rebuild_stacks(log.ops());
        let file_states = self.engine.file_states().clone();
        let new_engine =
            Engine::restore_with_stacks(project, log, self.engine.rev(), undo_stack, redo_stack, file_states)
                .map_err(|errs| io::Error::other(errs.join("; ")))?;
        self.engine = new_engine;
        self.persisted = self.engine.oplog().len();
        self.persist()?;
        Ok(())
    }

    /// 从快照链取三路合并的祖先:精确 rev → ≤当前 rev 的最大者;链缺失返回 None
    /// (退化口径:以本地为 base——与 V1 兼容,但此时冲突检测天然不可触发)。
    fn load_base(&self) -> Option<serde_json::Value> {
        let dir = self.root.join(BASES_REL);
        let mut best: Option<(u64, PathBuf)> = None;
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(n) = name.strip_suffix(".json").and_then(|s| s.parse::<u64>().ok()) else { continue };
            if n <= self.engine.rev() && best.as_ref().map(|(b, _)| n > *b).unwrap_or(true) {
                best = Some((n, entry.path()));
            }
        }
        let (_, path) = best?;
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// 引擎脏文件 → 盘面(先文件后记账;P1-9:写失败时 oplog 尚未记账)。
    /// notes.json 特殊:重载 NotesStore(撤销/重做恢复标注态)。
    fn reconcile_files(&mut self) -> io::Result<()> {
        for file in self.engine.take_dirty_files() {
            if file == "notes.json" {
                let Some(v) = self.engine.file_state("notes.json").cloned() else { continue };
                if v != self.notes.to_value() {
                    let store = NotesStore::from_value(&v).map_err(|errs| {
                        io::Error::other(format!("CF-005 SCHEMA_DRIFT(notes.json): {}", errs.join("; ")))
                    })?;
                    self.notes = store;
                    self.notes_dirty = true;
                }
                continue;
            }
            let Some(rel) = FILE_TRUTHS.iter().find(|(n, _)| *n == file).map(|(_, r)| r) else { continue };
            let Some(v) = self.engine.file_state(&file).cloned() else { continue };
            if self.files.get(&file) != Some(&v) {
                let mut buf = serde_json::to_vec_pretty(&v)?;
                buf.push(b'\n');
                atomic::atomic_write(&self.root.join(rel), &buf)?;
                self.files.insert(file, v);
            }
        }
        Ok(())
    }

    // ---------- 标注(4.9) ----------

    pub fn notes(&self) -> &NotesStore {
        &self.notes
    }

    /// 创建标注(user 提需求 / agent 反向提问),写入 notes.json 并登记 Op。
    pub fn notes_add(
        &mut self,
        anchor: cutforge_core::anchor::Anchor,
        body: String,
        author: NoteAuthor,
        tags: Vec<String>,
        actor: Actor,
        request_id: Option<String>,
    ) -> io::Result<String> {
        // 幂等:同 request_id 已登记过 → 不再新增;从该 Op 的 after 恢复既有标注 id
        if let Some(rid) = request_id.as_deref()
            && self.engine.oplog().has_request_id(rid)
        {
            let id = self
                .engine
                .oplog()
                .ops()
                .iter()
                .rev()
                .find(|o| o.request_id.as_deref() == Some(rid))
                .and_then(|o| o.after["items"].as_array())
                .and_then(|items| items.last())
                .and_then(|n| n["id"].as_str())
                .unwrap_or_default()
                .to_string();
            return Ok(id);
        }
        self.pre_write_sync()?;
        let before = self.notes.to_value();
        let note_id = self.notes.next_id();
        self.notes.add(anchor, body, author, tags);
        self.record_notes_change(
            &before, OpKind::Insert, actor,
            format!("创建标注 {note_id}"), None, request_id, false,
        )?;
        Ok(note_id)
    }

    /// 结案回执(绑定 opIds;同内容重复结案视为幂等成功,其余错误如实上报)。
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
            Err(cutforge_core::notes::NoteReject::AlreadyResolved(_)) => return Ok(()), // 幂等回执
            Err(cutforge_core::notes::NoteReject::UnknownNote(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("PRECONDITION_FAILED: 标注 {note_id} 不存在"),
                ))
            }
            Err(e) => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("PRECONDITION_FAILED: {e:?}")))
            }
        }
        self.record_notes_change(&before, OpKind::Set, actor, format!("标注 {note_id} 结案"), None, None, false)?;
        Ok(())
    }

    pub fn notes_reject(&mut self, note_id: &str, reason: String, actor: Actor) -> io::Result<()> {
        let before = self.notes.to_value();
        match self.notes.reject(note_id, reason) {
            Ok(_) => {}
            Err(cutforge_core::notes::NoteReject::UnknownNote(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("PRECONDITION_FAILED: 标注 {note_id} 不存在"),
                ))
            }
            Err(e) => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("PRECONDITION_FAILED: {e:?}")))
            }
        }
        self.record_notes_change(&before, OpKind::Set, actor, format!("标注 {note_id} 否决"), None, None, false)?;
        Ok(())
    }

    /// apply/undo/redo 成功后:open 态标注按 3.6 规则重定位;有变化则落盘并留痕。
    /// 重定位是自动簿记:`auto` Op,不入撤销栈(ADR-0001)。
    fn sync_notes_after_change(&mut self, actor: Actor) -> io::Result<()> {
        let before = self.notes.to_value();
        let (moved, orphaned) = self.notes.relocate_all(self.engine.project(), 500);
        if moved > 0 || orphaned > 0 {
            self.record_notes_change(
                &before,
                OpKind::Set,
                actor,
                format!("锚点重定位:跟随/重挂 {moved},转孤儿 {orphaned}"),
                None, None, true,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn record_notes_change(
        &mut self,
        before: &serde_json::Value,
        kind: OpKind,
        actor: Actor,
        summary: String,
        caused_by: Option<Vec<String>>,
        request_id: Option<String>,
        non_undoable: bool,
    ) -> io::Result<()> {
        let after = self.notes.to_value();
        let opts = ApplyOpts {
            caused_by: caused_by.unwrap_or_default(),
            summary: Some(summary),
            request_id,
            non_undoable,
            ..Default::default()
        };
        self.engine
            .record_file_change("notes.json", "/items", before.clone(), after, kind, actor, opts)
            .map_err(reject_to_io)?;
        self.notes_dirty = true;
        self.persist()?;
        Ok(())
    }

    /// 非 project.json 真相源(如 cutlist.json)的复合写:锁内登记审计 Op 并持久化。
    /// 文件本体由 reconcile 按 file_states 落盘(先文件后记账)。
    // 参数与 Op 字段一一对应(同 record_file_change 的理由)。
    #[allow(clippy::too_many_arguments)]
    pub fn record_change(
        &mut self,
        file: &str,
        path: &str,
        before: serde_json::Value,
        after: serde_json::Value,
        kind: cutforge_core::oplog::OpKind,
        actor: Actor,
        opts: ApplyOpts,
    ) -> io::Result<OpReceipt> {
        let _guard = self.temp_lock()?;
        let receipt = self
            .engine
            .record_file_change(file, path, before, after, kind, actor, opts)
            .map_err(reject_to_io)?;
        self.reconcile_files()?;
        self.persist()?;
        Ok(receipt)
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

    /// 三路合并(兼容入口):磁盘文件与内存模型按 baseRev 快照合并。
    /// 冲突时写入 `.cutforge/conflicts/<id>.json` 三方快照并停写(4.7)。
    /// `_meta` 旁路:磁盘若带新版 `_meta`(CutFlow 重生成),采纳之(ADR-0002)。
    pub fn merge_from_disk(&mut self) -> Result<Option<u64>, Vec<(String, Conflict)>> {
        self.sync_with_disk()?;
        Ok(Some(self.engine.rev()))
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

    /// 步骤 5-7:备份 → 原子写 project.json(`_meta` 旁路回写)→ 追加 oplog →
    /// rev 落盘 → 标注落盘。真相源文件本体已在 reconcile_files 先行落盘。
    fn persist(&mut self) -> io::Result<()> {
        // 写入窗口漂移检测(M9-1):pre-merge 之后、落盘之前,磁盘若被外部改写
        // (外部写者不持锁),以同步点为 base 做三路;同字段异改 → CF-001,
        // 本地待写**弃用**并从磁盘重载——宁拒写,不静默覆盖(北极星指标)。
        self.check_window_drift()?;
        let ops = self.engine.oplog().ops();
        // 备份旧 project.json(全局约定 B.8:可回滚)
        if (self.persisted == 0 || self.persisted < ops.len())
            && let Ok(old) = std::fs::read(self.root.join(PROJECT_REL)) {
                backup::backup_file(&self.root, PROJECT_REL, &old)?;
            }
        let value = self.engine.query(cutforge_core::engine::Query::ProjectView);
        let cutforge_core::engine::Answer::Project(mut v) = value else { unreachable!() };
        if let Some(meta) = &self.meta_bypass
            && let Some(obj) = v.as_object_mut() {
                obj.insert("_meta".into(), meta.clone());
            }
        let mut buf = serde_json::to_vec_pretty(&v)?;
        buf.push(b'\n');
        atomic::atomic_write(&self.root.join(PROJECT_REL), &buf)?;
        self.synced_disk = Some(v);

        // baseRev 快照(M9-1):同步点的工程视图(不含 _meta,与契约面一致)
        let rev = self.engine.rev();
        let bases_dir = self.root.join(BASES_REL);
        let _ = std::fs::create_dir_all(&bases_dir);
        let base_value = self.engine.query(cutforge_core::engine::Query::ProjectView);
        let cutforge_core::engine::Answer::Project(ref bv) = base_value else { unreachable!() };
        let mut bb = serde_json::to_vec_pretty(bv)?;
        bb.push(b'\n');
        let _ = atomic::atomic_write(&bases_dir.join(format!("{rev}.json")), &bb);
        prune_bases(&bases_dir, BASES_KEEP);

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

    /// persist 前置:磁盘 != 上次同步视图 → 外部在窗口内写入。
    /// 可自动合并 → 采纳(继续写);冲突 → 冲突落盘、本地重载、报 CONFLICT。
    fn check_window_drift(&mut self) -> io::Result<()> {
        let Some(synced) = self.synced_disk.clone() else { return Ok(()) };
        let Ok(text) = std::fs::read_to_string(self.root.join(PROJECT_REL)) else { return Ok(()) };
        let Ok(mut cur) = serde_json::from_str::<serde_json::Value>(&text) else { return Ok(()) };
        let cur_meta = cur.as_object_mut().and_then(|o| o.remove("_meta"));
        if let Some(m) = cur_meta {
            self.meta_bypass = Some(m);
        }
        if cur == synced {
            return Ok(());
        }
        let local = match self.engine.query(cutforge_core::engine::Query::ProjectView) {
            cutforge_core::engine::Answer::Project(v) => v,
            _ => unreachable!(),
        };
        match cutforge_core::merge::three_way_merge(&synced, &cur, &local) {
            MergeOutcome::Merged(v) => {
                self.synced_disk = Some(cur);
                if v != local {
                    self.adopt_merged(v)?;
                }
                Ok(())
            }
            MergeOutcome::Conflicts(conflicts) => {
                for c in &conflicts {
                    self.persist_conflict(c);
                }
                self.reload_from_disk()?;
                Err(io::Error::other(format!(
                    "CONFLICT: 写入窗口内外部已改动同一工程({} 项冲突),本地待写已弃用,请裁决后重试",
                    conflicts.len())))
            }
        }
    }

    /// 弃用本地待写:从磁盘真相重载全部状态(外部改动获胜,4.7)。
    fn reload_from_disk(&mut self) -> io::Result<()> {
        let (engine, persisted, notes, meta_bypass, files, synced_disk) = Self::load(&self.root)?;
        self.engine = engine;
        self.persisted = persisted;
        self.notes = notes;
        self.notes_dirty = false;
        self.meta_bypass = meta_bypass;
        self.files = files;
        self.synced_disk = synced_disk;
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

/// baseRev 快照 LRU 淘汰:保留 rev 最大的 keep 份(计划书 V2-R4 防膨胀)。
fn prune_bases(dir: &Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut revs: Vec<(u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(".json").and_then(|x| x.parse::<u64>().ok()).map(|n| (n, e.path()))
        })
        .collect();
    revs.sort_by_key(|(n, _)| *n);
    while revs.len() > keep {
        let (_, path) = revs.remove(0);
        let _ = std::fs::remove_file(path);
    }
}

/// UTC 紧凑日期 YYYYMMDD(oplog 按天切分;算法唯一来源 core::timeutil)。
fn today_compact() -> String {
    cutforge_core::timeutil::now_date_compact()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil;

    #[test]
    fn open_migrates_v1_and_persists_commands() {
        let root = tests_fixture("ws-open").unwrap();
        let mut ws = Workspace::open_exclusive(&root).unwrap();
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
            let mut ws = Workspace::open_exclusive(&root).unwrap();
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
        let mut ws2 = Workspace::open_exclusive(&root).unwrap();
        assert_eq!(ws2.rev(), 1, "rev 必须从盘面恢复");
        assert_eq!(ws2.engine().oplog().len(), 1, "OpLog 必须从 jsonl 恢复");
        // 恢复后的撤销栈仍然可用
        ws2.undo(Actor::user("人")).unwrap();
        let disk = std::fs::read_to_string(root.join(PROJECT_REL)).unwrap();
        assert!(disk.contains("8400"), "撤销后盘面应回到 8400");
        fsutil::cleanup(&root);
    }

    /// M8-1 门禁(IO 侧):notes_add→undo,notes.json 盘面**语义**还原
    /// (canonical JSON 逐值相等;重序列化的缩进/键序不属语义域)。
    /// 撤销深度 = 真实用户手势数(auto 的重定位 Op 不入栈)。
    #[test]
    fn notes_add_undo_restores_disk_bytes() {
        let root = tests_fixture("ws-undo-notes").unwrap();
        let notes_before = std::fs::read_to_string(root.join(NOTES_REL)).unwrap();
        let mut ws = Workspace::open_exclusive(&root).unwrap();
        let anchor = cutforge_core::anchor::Anchor {
            kind: cutforge_core::anchor::AnchorKind::Clip,
            ref_: Some("V1-001".into()), t_ms: 4000, span: None,
        };
        ws.notes_add(anchor, "这里语速太快".into(), NoteAuthor::User, vec![], Actor::user("人"), None).unwrap();
        let after_add: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(NOTES_REL)).unwrap()).unwrap();
        assert_eq!(after_add["items"].as_array().unwrap().len(), 3, "新标注必须落盘");
        ws.undo(Actor::user("人")).unwrap();
        let notes_after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(NOTES_REL)).unwrap()).unwrap();
        let before: serde_json::Value = serde_json::from_str(&notes_before).unwrap();
        assert_eq!(notes_after, before, "undo 后 notes.json 必须还原到 notes_add 之前");
        fsutil::cleanup(&root);
    }

    /// M8-1 门禁(cutlist 侧):经 record_change 的 cutlist 编辑可 undo 还原盘面
    /// (修复前:op 记到 project 的 "/" 指针,cutlist 文件原封不动)。
    #[test]
    fn cutlist_record_change_undo_restores_disk() {
        let root = tests_fixture("ws-undo-cutlist").unwrap();
        let before_text = std::fs::read_to_string(root.join(CUTLIST_REL)).unwrap();
        let before: serde_json::Value = serde_json::from_str(&before_text).unwrap();
        let mut ws = Workspace::open_exclusive(&root).unwrap();
        let mut after = before.clone();
        after["cuts"][0]["action"] = serde_json::json!("review");
        ws.record_change(
            "cutlist.json", "/", before.clone(), after,
            OpKind::Set, Actor::script("m8-1"),
            ApplyOpts { summary: Some("cut_apply 测试".into()), ..Default::default() },
        ).unwrap();
        let mid: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(CUTLIST_REL)).unwrap()).unwrap();
        assert_eq!(mid["cuts"][0]["action"], serde_json::json!("review"), "编辑必须真实落盘");
        ws.undo(Actor::user("人")).unwrap();
        let restored: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(CUTLIST_REL)).unwrap()).unwrap();
        assert_eq!(restored, before, "undo 后 cutlist.json 必须还原");
        fsutil::cleanup(&root);
    }

    /// M8-4 门禁:双线程并发写同一工程,open_exclusive 锁覆盖 open→apply→persist
    /// 全程 → oplog 数 = 盘面 rev = 2N,零丢更新(修复前:锁外读+后写整文件覆盖)。
    #[test]
    fn concurrent_writes_no_loss() {
        use std::sync::Barrier;
        let root = tests_fixture("ws-concurrent").unwrap();
        let root_s = root.to_string_lossy().to_string();
        let n = 5;
        let barrier = std::sync::Arc::new(Barrier::new(2));
        let handles: Vec<_> = ["并发-A", "并发-B"].into_iter().map(|who| {
            let barrier = barrier.clone();
            let root_s = root_s.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for i in 0..n {
                    let mut ws = Workspace::open_exclusive(Path::new(&root_s)).unwrap();
                    ws.apply(
                        Command::ClipUpdate {
                            clip_id: "V1-001".into(),
                            patch: cutforge_core::command::ClipPatch {
                                duration_ms: Some(7000 - (who.len() * 10 + i) as u64),
                                ..Default::default()
                            },
                        },
                        Actor::agent(who),
                        ApplyOpts::default(),
                    ).unwrap();
                }
            })
        }).collect::<Vec<_>>();
        for h in handles {
            h.join().unwrap();
        }
        let ws = Workspace::open(&root).unwrap();
        assert_eq!(ws.rev(), 2 * n as u64, "rev 必须 = 2N(零丢更新)");
        assert_eq!(ws.engine().oplog().len(), 2 * n, "OpLog 必须 = 2N");
        let disk_rev: u64 = std::fs::read_to_string(root.join(".cutforge/rev")).unwrap().trim().parse().unwrap();
        assert_eq!(disk_rev, 2 * n as u64, "盘面 rev 与 OpLog 不得分叉");
        fsutil::cleanup(&root);
    }

    /// M8-2 门禁:CutFlow 真实 IR(顶层 _meta)可打开、可迁移、roundtrip 不丢 _meta。
    /// 夹具由 tools/gen_real_ir_fixture.py 调 CutFlow rs_ir.py 生成(计划书 D2)。
    #[test]
    fn open_real_cutflow_ir_with_meta_roundtrip() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/real_ir/project.json");
        let text = std::fs::read_to_string(&fixture)
            .expect("缺 tests/fixtures/real_ir/project.json:先跑 tools/gen_real_ir_fixture.py");
        let original_meta: serde_json::Value = {
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            v.get("_meta").cloned().expect("夹具必须含顶层 _meta(CutFlow 真实 IR 的判别特征)")
        };
        let root = fsutil::temp_dir("ws-real-ir");
        fsutil::ensure(&root.join("05_ir")).unwrap();
        std::fs::write(root.join(PROJECT_REL), &text).unwrap();
        {
            let mut ws = Workspace::open_exclusive(&root).unwrap();
            ws.apply(
                Command::ClipUpdate {
                    clip_id: "V1-001".into(),
                    patch: cutforge_core::command::ClipPatch { duration_ms: Some(3000), ..Default::default() },
                },
                Actor::agent("m8-2"),
                ApplyOpts::default(),
            )
            .unwrap();
        }
        // 重开:roundtrip 后 _meta 原样保留
        let mut ws2 = Workspace::open(&root).unwrap();
        assert_eq!(ws2.rev(), 1);
        let _ = ws2; // 打开即证明 roundtrip 后文件仍合法
        let disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(PROJECT_REL)).unwrap()).unwrap();
        assert_eq!(disk["_meta"], original_meta, "cutforge 写回不得丢/改 _meta");
        fsutil::cleanup(&root);
    }
}

#[cfg(test)]
mod m9_tests {
    use super::*;
    use crate::fsutil;

    fn read_project(root: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(root.join(PROJECT_REL)).unwrap()).unwrap()
    }

    fn write_project(root: &Path, v: &serde_json::Value) {
        std::fs::write(root.join(PROJECT_REL), serde_json::to_string_pretty(v).unwrap()).unwrap();
    }

    /// M9-1 门禁:外部改 A 字段 + 本地改 A 字段 → 必须 CF-001 + 停写
    /// (计划书:当前实现下该测试必红,V2 转绿)。
    #[test]
    fn conflict_real_repro_and_stop_writes() {
        let root = tests_fixture("ws-conflict").unwrap();
        // 上一会话:rev1(改 V1-002 时长),留下 bases/1
        {
            let mut w = Workspace::open_exclusive(&root).unwrap();
            w.apply(
                Command::ClipUpdate {
                    clip_id: "V1-002".into(),
                    patch: cutforge_core::command::ClipPatch { duration_ms: Some(6000), ..Default::default() },
                },
                Actor::user("u"),
                ApplyOpts::default(),
            ).unwrap();
        }
        assert!(root.join(BASES_REL).join("1.json").exists(), "persist 必须留 baseRev 快照");

        // 本会话:长活工作区,本地待写 V1-002 startMs→9000(尚未落盘)
        let mut ws = Workspace::open_exclusive(&root).unwrap();
        ws.pre_write_sync().unwrap();
        let receipt = ws.engine.apply(
            Command::ClipUpdate {
                clip_id: "V1-002".into(),
                patch: cutforge_core::command::ClipPatch { start_ms: Some(9000), ..Default::default() },
            },
            Actor::agent("m9"),
            ApplyOpts::default(),
        ).unwrap();
        assert_eq!(receipt.rev, 2);

        // 写入窗口内:外部写者改同一字段 → startMs=9500
        let mut v = read_project(&root);
        v["tracks"][0]["clips"][1]["startMs"] = serde_json::json!(9500);
        write_project(&root, &v);

        // persist 必须检出漂移 → CF-001 → 本地待写弃用
        let err = ws.persist().unwrap_err();
        assert!(err.to_string().starts_with("CONFLICT"), "必须报 CONFLICT: {err}");
        assert_eq!(read_project(&root)["tracks"][0]["clips"][1]["startMs"], serde_json::json!(9500),
            "外部改动获胜,本地不得静默覆盖");
        assert_eq!(ws.rev(), 1, "弃用待写后 rev 回到磁盘真相");
        let conflicts = ws.conflict_list().unwrap();
        assert!(!conflicts.is_empty() && conflicts.iter().all(|(_, c)| c.code.code() == "CF-001"),
            "必须落 CF-001: {conflicts:?}");

        // 停写:冲突未裁决前一切写拒绝
        let blocked = ws.apply(
            Command::ClipUpdate {
                clip_id: "V1-001".into(),
                patch: cutforge_core::command::ClipPatch { duration_ms: Some(7000), ..Default::default() },
            },
            Actor::agent("m9"),
            ApplyOpts::default(),
        );
        assert!(blocked.is_err() && blocked.err().unwrap().to_string().contains("CONFLICT"));
        fsutil::cleanup(&root);
    }

    /// M9-1 正例:窗口内外部改**不同**字段 → 自动合并,双方改动都存活。
    #[test]
    fn window_drift_different_fields_auto_merge() {
        let root = tests_fixture("ws-automerge").unwrap();
        let mut ws = Workspace::open_exclusive(&root).unwrap();
        ws.pre_write_sync().unwrap();
        let _ = ws.engine.apply(
            Command::ClipUpdate {
                clip_id: "V1-002".into(),
                patch: cutforge_core::command::ClipPatch { start_ms: Some(9000), ..Default::default() },
            },
            Actor::agent("m9"),
            ApplyOpts::default(),
        ).unwrap();
        let mut v = read_project(&root);
        v["slug"] = serde_json::json!("renamed-外部");
        write_project(&root, &v);
        ws.persist().unwrap();
        let disk = read_project(&root);
        assert_eq!(disk["slug"], serde_json::json!("renamed-外部"), "外部改动必须存活");
        assert_eq!(disk["tracks"][0]["clips"][1]["startMs"], serde_json::json!(9000), "本地改动必须存活");
        fsutil::cleanup(&root);
    }

    /// M9-1:快照链 LRU——40 次写后 bases 目录 ≤ 32 份,最新快照在。
    #[test]
    fn bases_snapshot_lru() {
        let root = tests_fixture("ws-lru").unwrap();
        let mut ws = Workspace::open_exclusive(&root).unwrap();
        for i in 0..40u64 {
            ws.apply(
                Command::ClipUpdate {
                    clip_id: "V1-001".into(),
                    patch: cutforge_core::command::ClipPatch {
                        duration_ms: Some(7000 - i.min(500)),
                        ..Default::default()
                    },
                },
                Actor::agent("m9"),
                ApplyOpts::default(),
            ).unwrap();
        }
        let count = std::fs::read_dir(root.join(BASES_REL)).unwrap().count();
        assert!(count <= 32, "LRU 上限 32,实际 {count}");
        assert!(root.join(BASES_REL).join("40.json").exists(), "最新快照必须在");
        fsutil::cleanup(&root);
    }

    /// M9-2 门禁:外部手改 project.json 后,守护 ≤1s 产出事件(北极星:外部改动可见)。
    #[test]
    fn external_edit_visible_within_1s() {
        let root = tests_fixture("ws-daemon").unwrap();
        let hub = crate::watcher::ensure_sync_daemon(&root);
        std::thread::sleep(std::time::Duration::from_millis(500)); // 让守护完成初扫
        let since = hub.current();
        let mut v = read_project(&root);
        v["slug"] = serde_json::json!("daemon-visible");
        write_project(&root, &v);
        let t0 = std::time::Instant::now();
        let seq = hub.wait_since(since, std::time::Duration::from_millis(1500))
            .expect("外部改动必须 ≤1s 可见(1.5s 容差含 CI 抖动)");
        assert!(seq > since);
        println!("外部改动可见耗时: {:?}", t0.elapsed());
        fsutil::cleanup(&root);
    }
}
