//! Workspace 打开/装载:盘面 → 内存。
//! 布局判定(0.5 中文目录/0.4.x 英文目录)、OpLog 恢复(含 R-03 半行截断
//! 显式修复)、rev 对账与 R-04 差异自愈(ReconciledOp)、notes 装载、文件态初始化。

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use cutforge_core::engine::Engine;
use cutforge_core::model::Project;
use cutforge_core::notes::NotesStore;

use super::{Layout, Workspace};
use crate::paths::NOTES_REL;
use crate::repair::{self, RepairReport};

impl Workspace {
    /// 打开工程目录(只读语义安全:检测到损坏面只**报告**不落盘;
    /// 修复动作只发生在写通道打开 `open_exclusive`/`open_for_write` 的锁内)。
    pub fn open(root: &Path) -> io::Result<Self> {
        let (engine, persisted, notes, meta_bypass, files, synced_disk, layout, repair) =
            Self::load(root, false)?;
        Ok(Self {
            root: root.to_path_buf(),
            engine,
            persisted,
            notes,
            notes_dirty: false,
            meta_bypass,
            files,
            layout,
            lock: None,
            synced_disk,
            repair,
        })
    }

    /// 独占打开:锁覆盖 open→apply→persist 全程(P0-5:跨进程并发写不再丢更新)。
    /// MCP 写通道与 CLI 变更子命令一律走本入口。
    /// 装载期发现 oplog 半行截断 / 记账 rev 缺口 → 锁内显式修复模式(R-03/R-04)。
    pub fn open_exclusive(root: &Path) -> io::Result<Self> {
        let guard = crate::lock::acquire(root, 30_000, 20)?;
        let (engine, persisted, notes, meta_bypass, files, synced_disk, layout, repair) =
            Self::load(root, true)?;
        let mut ws = Self {
            root: root.to_path_buf(),
            engine,
            persisted,
            notes,
            notes_dirty: false,
            meta_bypass,
            files,
            layout,
            lock: Some(guard),
            synced_disk,
            repair,
        };
        // 迁移升级:盘面为旧形态(v1/缺 id)时,独占打开即落规范形,
        // 使后续外部改动检测与守护合并都以 v2 规范形为基准。
        let view = ws.engine.query(cutforge_core::engine::Query::ProjectView);
        let cutforge_core::engine::Answer::Project(ref v) = view else {
            unreachable!()
        };
        if Some(v) != ws.synced_disk.as_ref() {
            ws.persist()?;
        }
        Ok(ws)
    }

    /// 打开工程供写入(常驻复用形态,AC-1.8 性能专项):不持有全程锁,
    /// 写入时由 `Workspace::apply`/`undo` 等方法内部临时补锁(与 notes 复合写的
    /// 既有纪律同一口径)。锁外装载的安全性由写前同步兜底:`check_window_drift`
    /// /`sync_with_disk` 在锁内以装载视图为基准检测外部改动(漂移即三路合并/停写)。
    /// 与 `open_exclusive` 等价的迁移升级语义:盘面为旧形态(v1/缺 id)时,
    /// 锁内立即落规范形。装载期修复(R-03/R-04)同样在锁内完成后才交还锁。
    pub fn open_for_write(root: &Path) -> io::Result<Self> {
        let guard = crate::lock::acquire(root, 30_000, 20)?;
        let (engine, persisted, notes, meta_bypass, files, synced_disk, layout, repair) =
            Self::load(root, true)?;
        let mut ws = Self {
            root: root.to_path_buf(),
            engine,
            persisted,
            notes,
            notes_dirty: false,
            meta_bypass,
            files,
            layout,
            lock: Some(guard),
            synced_disk,
            repair,
        };
        let view = ws.engine.query(cutforge_core::engine::Query::ProjectView);
        let cutforge_core::engine::Answer::Project(ref v) = view else {
            unreachable!()
        };
        if Some(v) != ws.synced_disk.as_ref() {
            ws.persist()?;
        }
        // 回归常驻形态:迁移/修复完成即交还全程锁,写入走方法内临时补锁
        ws.lock = None;
        Ok(ws)
    }

    #[allow(clippy::type_complexity)]
    pub(super) fn load(
        root: &Path,
        repair: bool,
    ) -> io::Result<(
        Engine,
        usize,
        NotesStore,
        Option<serde_json::Value>,
        BTreeMap<String, serde_json::Value>,
        Option<serde_json::Value>,
        Layout,
        Option<RepairReport>,
    )> {
        // 盘面布局判定(0.5 中文目录为准;0.4.x 英文目录工程兼容读写、原地保留)
        let layout = Layout::detect(root);
        let project_path = root.join(layout.project_rel);
        let text = std::fs::read_to_string(&project_path).map_err(|e| {
            io::Error::new(e.kind(), format!("打开工程失败({project_path:?}): {e}"))
        })?;
        let mut value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| io::Error::other(format!("project.json 非法 JSON: {e}")))?;
        // _meta 旁路(ADR-0002):进内存旁路,不进契约校验
        let meta_bypass = value.as_object_mut().and_then(|o| o.remove("_meta"));
        let project = Project::from_value(&value)
            .or_else(|_| cutforge_core::model::migrate_from_value(&value))
            .map_err(|errs| {
                io::Error::other(format!("project.json 未通过 v2 契约: {}", errs.join("; ")))
            })?;

        // OpLog 恢复(R-03):扫描全部分片;半行截断只中断当前分片(§2.3 措辞
        // 修正:后续分片照常装载),截断点与分片名结构化记录,绝不静默。
        let scanned = repair::scan_oplog(root);
        let mut log = scanned.log;

        // 记账 rev(先读;差异自愈与丢失数都以它为基准)
        let rev_path = root.join(".cutforge/rev");
        let disk_rev = std::fs::read_to_string(&rev_path)
            .ok()
            .and_then(|t| t.trim().parse::<u64>().ok());

        // R-03 丢失操作数:记账已到 disk_rev,而最后完整 Op 只到 last_complete_rev
        let last_complete_rev = log.last_rev().unwrap_or(0);
        let lost_ops = disk_rev.unwrap_or(0).saturating_sub(last_complete_rev);

        // R-03 显式修复模式(写通道 + 锁内才落盘;只读 open 只报告不落盘):
        // 备份 .cutforge/ → recovery-{ts}/ → 物理截断到最后完整 Op。
        let mut report = RepairReport {
            root: root.to_path_buf(),
            truncated_files: scanned.truncated_files,
            lost_ops,
            reconciled_revs: Vec::new(),
            backup_path: None,
            ts: cutforge_core::timeutil::now_rfc3339(),
        };
        if repair && !report.truncated_files.is_empty() {
            match repair::backup_state_dir(root) {
                Ok(p) => report.backup_path = Some(p),
                Err(e) => eprintln!("[cutforge-io][warn] 修复前备份失败(修复继续,备份缺位): {e}"),
            }
            for (file, good_end) in &scanned.truncation_points {
                let data = std::fs::read(file).unwrap_or_default();
                // 截到最后完整 Op(字节终点;原子写 = 唯一落盘点)
                crate::atomic::atomic_write(file, &data[..(*good_end as usize)])?;
            }
        }

        // R-04 差异自愈:记账 rev > oplog max rev(先文件后记账的崩溃窗口)→
        // 补记 ReconciledOp 屏障(undo 到此处明确拒绝),用户可见(报告 + warn)。
        if let Some(d) = disk_rev
            && d > last_complete_rev
            && repair
        {
            let oplog_dir = root.join(".cutforge/oplog");
            let oplog_file = oplog_dir.join(format!(
                "{}.jsonl",
                cutforge_core::timeutil::now_date_compact()
            ));
            let op = repair::reconciled_op(d, last_complete_rev, log.next_op_id());
            let line = serde_json::to_string(&op)?;
            crate::atomic::append_lines(&oplog_file, &[format!("{line}\n")])?;
            log.push_loaded(op);
            report.reconciled_revs.push(d);
        }

        // rev 对账:以 oplog(含补记)与记账文件的最大值为准
        let mut rev = log.last_rev().unwrap_or(0);
        if let Some(d) = disk_rev {
            rev = rev.max(d);
        }
        let (undo_stack, redo_stack) = cutforge_core::engine::rebuild_stacks(log.ops());
        let persisted = log.len();

        // notes.json(标注):缺失 = 空存储;存在则必须过 notes.schema
        let (notes, notes_value) = match std::fs::read_to_string(root.join(NOTES_REL)) {
            Ok(text) => {
                let v: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| io::Error::other(format!("notes.json 非法 JSON: {e}")))?;
                let store = NotesStore::from_value(&v).map_err(|errs| {
                    io::Error::other(format!(
                        "CF-005 SCHEMA_DRIFT(notes.json): {}",
                        errs.join("; ")
                    ))
                })?;
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
        for (name, rel) in layout.truths {
            if let Ok(text) = std::fs::read_to_string(root.join(rel))
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
            {
                file_states.insert(name.to_string(), v);
            }
        }
        file_states.insert("notes.json".to_string(), notes_value);

        let engine =
            Engine::restore_with_stacks(project, log, rev, undo_stack, redo_stack, file_states)
                .map_err(|errs| io::Error::other(errs.join("; ")))?;

        // 最近落盘值快照(落盘 diff 用)
        let mut files = BTreeMap::new();
        for (name, rel) in layout.truths {
            if let Ok(text) = std::fs::read_to_string(root.join(rel))
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
            {
                files.insert(name.to_string(), v);
            }
        }

        // R-03/R-04:修复报告落盘 + stderr warn(绝不静默的第二/第三出口)
        let report = if report.has_actions() {
            if repair {
                repair::persist_report(root, &report);
                eprintln!("[cutforge-io][warn] {}", report.summary());
            }
            Some(report)
        } else {
            None
        };
        Ok((
            engine,
            persisted,
            notes,
            meta_bypass,
            files,
            value.clone().into(),
            layout,
            report,
        ))
    }
}

/// 测试夹具:从仓库回归样本搭建完整工程目录(0.5 新布局/中文目录;旧布局夹具见
/// tests/layout_compat.rs 的兼容用例)。
#[doc(hidden)]
pub fn tests_fixture(tag: &str) -> io::Result<PathBuf> {
    let sample =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/regression/talking-head");
    let root = crate::fsutil::temp_dir(tag);
    crate::fsutil::ensure(&root.join(crate::paths::TIMELINE))?;
    crate::fsutil::ensure(&root.join(crate::paths::CUT))?;
    crate::fsutil::copy_file(
        &sample.join("project.json"),
        &root.join(crate::paths::PROJECT_REL),
    )?;
    crate::fsutil::copy_file(
        &sample.join("wordline.json"),
        &root.join(crate::paths::WORDLINE_REL),
    )?;
    crate::fsutil::copy_file(
        &sample.join("cutlist.json"),
        &root.join(crate::paths::CUTLIST_REL),
    )?;
    crate::fsutil::copy_file(
        &sample.join("notes.json"),
        &root.join(crate::paths::NOTES_REL),
    )?;
    Ok(root)
}
