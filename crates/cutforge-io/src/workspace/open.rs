//! Workspace 打开/装载:盘面 → 内存。
//! 布局判定(0.5 中文目录/0.4.x 英文目录)、OpLog 恢复、rev 对账、notes 装载、文件态初始化。

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use cutforge_core::engine::Engine;
use cutforge_core::model::Project;
use cutforge_core::notes::NotesStore;
use cutforge_core::oplog::{Op, OpLog};

use super::{Layout, Workspace};
use crate::paths::NOTES_REL;

impl Workspace {
    /// 打开工程目录(只读语义安全;写操作走 `open_exclusive` 或依赖方法内临时锁)。
    pub fn open(root: &Path) -> io::Result<Self> {
        let (engine, persisted, notes, meta_bypass, files, synced_disk, layout) = Self::load(root)?;
        Ok(Self {
            root: root.to_path_buf(), engine, persisted, notes,
            notes_dirty: false, meta_bypass, files, layout, lock: None, synced_disk,
        })
    }

    /// 独占打开:锁覆盖 open→apply→persist 全程(P0-5:跨进程并发写不再丢更新)。
    /// MCP 写通道与 CLI 变更子命令一律走本入口。
    pub fn open_exclusive(root: &Path) -> io::Result<Self> {
        let guard = crate::lock::acquire(root, 30_000, 20)?;
        let (engine, persisted, notes, meta_bypass, files, synced_disk, layout) = Self::load(root)?;
        let mut ws = Self {
            root: root.to_path_buf(), engine, persisted, notes,
            notes_dirty: false, meta_bypass, files, layout, lock: Some(guard), synced_disk,
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
    pub(super) fn load(
        root: &Path,
    ) -> io::Result<(Engine, usize, NotesStore, Option<serde_json::Value>, BTreeMap<String, serde_json::Value>, Option<serde_json::Value>, Layout)> {
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
        for (name, rel) in layout.truths {
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
        for (name, rel) in layout.truths {
            if let Ok(text) = std::fs::read_to_string(root.join(rel))
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    files.insert(name.to_string(), v);
                }
        }
        Ok((engine, persisted, notes, meta_bypass, files, value.clone().into(), layout))
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
    crate::fsutil::copy_file(&sample.join("project.json"), &root.join(crate::paths::PROJECT_REL))?;
    crate::fsutil::copy_file(&sample.join("wordline.json"), &root.join(crate::paths::WORDLINE_REL))?;
    crate::fsutil::copy_file(&sample.join("cutlist.json"), &root.join(crate::paths::CUTLIST_REL))?;
    crate::fsutil::copy_file(&sample.join("notes.json"), &root.join(crate::paths::NOTES_REL))?;
    Ok(root)
}
