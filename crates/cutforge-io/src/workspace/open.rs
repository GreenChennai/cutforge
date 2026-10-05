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

        // R-13② 快照优先装载:最新 `.cutforge/snapshots/r<S>/` 存在且可信 →
        // `replay_from_snapshot(base, S, rev>S 后缀)` 重建头态(rev 链从 S+1 起
        // 强校验,残留前缀幂等由 core::retained_ops 保证);撤销/重做双栈 =
        // 快照前缀重建 + 后缀折叠(`rebuild_stacks_incremental`,与全量重建
        // 逐语义相等)。引擎内存 OpLog 保持**全量虚拟历史**(前缀取自未压实的
        // 盘面日志或快照 oplog 副本)——护城河 6(OpLog 即历史)在压实后依然
        // 成立:跨压实边界的 undo/redo 所需 Op 全在内存。任一环节不可信
        //(旧日志缺 rev / rev 链断裂 / 快照残缺 / rev 对账不齐)→ stderr 警告
        //(绝不静默)+ 回退既有全量路径(该路径零变化)。
        let mut snapshot_state = None;
        if let Some(s_rev) = crate::snapshot::latest_snapshot_rev(root) {
            match load_from_snapshot(root, s_rev, &log, rev) {
                Ok(state) => {
                    // V2-HOTFIX(外部追加丢失定案):磁盘 project.json 是"先文件后
                    // 记账"的**权威落盘**;快照+重放重建头态若与磁盘不一致,说明
                    // 存在快照面之外的外部直写/崩溃窗口差异——此时快照优化必须
                    // 放弃(回退全量装载,以磁盘为真),绝不能静默采用快照态把
                    // 外部改动覆写丢失(web-e2e A2 第 13 步回归的丢点行)。
                    if state.project != project {
                        eprintln!(
                            "[cutforge-io][warn] 快照优先装载放弃(r{s_rev} 头态与磁盘 project.json 不一致),回退全量装载(磁盘为权威,保留外部改动)"
                        );
                    } else {
                        snapshot_state = Some(state);
                    }
                }
                Err(reason) => eprintln!(
                    "[cutforge-io][warn] 快照优先装载放弃(r{s_rev} 不可信),回退全量装载: {reason}"
                ),
            }
        }

        let (engine, persisted) = match snapshot_state {
            Some(state) => {
                // 全量虚拟日志:前缀(快照侧权威)+ 后缀(rev > S,盘面实况)
                let mut log = cutforge_core::oplog::OpLog::new();
                for op in &state.prefix_ops {
                    log.push_loaded(op.clone());
                }
                for op in &state.suffix_ops {
                    log.push_loaded(op.clone());
                }
                let (undo_stack, redo_stack) = cutforge_core::engine::rebuild_stacks_incremental(
                    cutforge_core::engine::rebuild_stacks(&state.prefix_ops),
                    &state.suffix_ops,
                );
                let engine = Engine::restore_with_stacks(
                    state.project,
                    log,
                    rev,
                    undo_stack,
                    redo_stack,
                    file_states,
                )
                .map_err(|errs| io::Error::other(errs.join("; ")))?;
                (engine, state.persisted)
            }
            None => {
                let (undo_stack, redo_stack) = cutforge_core::engine::rebuild_stacks(log.ops());
                let persisted = log.len();
                let engine = Engine::restore_with_stacks(
                    project,
                    log,
                    rev,
                    undo_stack,
                    redo_stack,
                    file_states,
                )
                .map_err(|errs| io::Error::other(errs.join("; ")))?;
                (engine, persisted)
            }
        };

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

/// R-13② 快照优先装载的中间态(纯装载,无磁盘副作用)。
struct SnapshotState {
    /// 快照时刻工程(replay_from_snapshot 的 base)经后缀回放后的**头态**。
    project: Project,
    /// rev ≤ S 的前缀 Op(撤销栈前缀重建 + 虚拟全量日志的前半)。
    prefix_ops: Vec<cutforge_core::oplog::Op>,
    /// rev > S 的后缀 Op(retained_ops;虚拟全量日志的后半)。
    suffix_ops: Vec<cutforge_core::oplog::Op>,
    /// 已落盘 Op 数口径:盘面仍持完整前缀 = 盘面 Op 数;盘面已截 = 后缀数。
    persisted: usize,
}

/// 快照优先装载(R-13②):以快照 r<S> 为基线做增量回放,返回引擎构造所需
/// 中间态。返回 Err = 快照面不可信/与盘面不衔接,调用方回退全量装载
/// (报告原因,绝不静默)。装载本身只读,无任何磁盘副作用。
fn load_from_snapshot(
    root: &Path,
    s_rev: u64,
    log: &cutforge_core::oplog::OpLog,
    rev: u64,
) -> Result<SnapshotState, String> {
    let ops = log.ops();
    // 旧日志缺 rev:无法证明「被截前缀 ⊆ 快照覆盖面」→ 放弃(compact::plan 同口径)
    if ops.iter().any(|o| o.rev.is_none()) {
        return Err("oplog 存在缺 rev 的旧格式 Op".to_string());
    }
    let base = crate::snapshot::read_snapshot_base(root, s_rev)
        .ok_or_else(|| "快照 project.json 缺失或未过契约".to_string())?;
    let split = ops.partition_point(|o| o.rev.unwrap_or(0) <= s_rev);
    let suffix_ops: Vec<cutforge_core::oplog::Op> = ops[split..].to_vec();
    // 前缀来源二选一:
    // - 盘面日志仍完整覆盖 1..=S(未压实/截断中断态)→ 直接取盘面
    //  (追加序 = rev 序,首条 rev=1 且 rev≤S 恰有 S 条 ⇔ 连续无缺);
    // - 盘面已截 → 取快照 oplog 副本,并整链校验恰为 1..=S(残缺即放弃)。
    let disk_prefix_complete =
        ops.first().and_then(|o| o.rev) == Some(1) && split == s_rev as usize;
    let (prefix_ops, persisted) = if disk_prefix_complete {
        (ops[..split].to_vec(), ops.len())
    } else {
        let snap_ops = crate::snapshot::read_snapshot_ops(root, s_rev)
            .ok_or_else(|| "快照 oplog 副本缺失或含非法行".to_string())?;
        let contiguous = snap_ops.len() == s_rev as usize
            && snap_ops
                .iter()
                .enumerate()
                .all(|(i, o)| o.rev == Some(i as u64 + 1));
        if !contiguous {
            return Err("快照 oplog 副本非完整 1..=S 前缀".to_string());
        }
        (snap_ops, suffix_ops.len())
    };
    // 增量回放(rev 链从 S+1 起强校验;断链/乱序 = 快照与日志不衔接 → 放弃)
    let replayed = Engine::replay_from_snapshot(base, s_rev, &suffix_ops)
        .map_err(|e| format!("增量回放失败: {e:?}"))?;
    if replayed.rev() != rev {
        return Err(format!(
            "rev 对账不一致:回放到 {},盘面对账为 {rev}(存在缺口,走全量装载)",
            replayed.rev()
        ));
    }
    Ok(SnapshotState {
        project: replayed.project().clone(),
        prefix_ops,
        suffix_ops,
        persisted,
    })
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
