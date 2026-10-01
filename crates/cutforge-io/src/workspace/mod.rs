//! Workspace 实码子模块:按职责切分(原 lib.rs 单文件拆出,纯移动)。
//!
//! - `open`:打开/装载(布局判定、OpLog 恢复、rev 对账)
//! - `query`:只读访问器(root/engine/project/rev)
//! - `apply`:唯一写入路径(4.2 八步)的显式步骤函数管线
//! - `sync`:写前同步(冲突停写 + 磁盘三路合并,M9-1/4.7)
//! - `notes`:标注(4.9)与锚点重定位联动
//! - `persist`:持久化管线(备份 → 原子写 → 快照 → OpLog → rev → 标注)
//! - `bases`:baseRev 快照链(M9-1)与 LRU 淘汰
//! - `conflicts`:冲突三方快照(4.7)
//!
//! `Workspace` 结构体与本模块私有共享类型(`Layout`、`reject_to_io`)定义于此;
//! 私有成员对全部子模块可见(Rust 可见性:定义模块及其后代)。

pub(crate) mod apply;
pub(crate) mod bases;
pub(crate) mod conflicts;
pub(crate) mod notes;
pub(crate) mod open;
pub(crate) mod persist;
pub(crate) mod query;
pub(crate) mod sync;
#[cfg(test)]
mod inline_tests;

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use cutforge_core::engine::Engine;
use cutforge_core::notes::NotesStore;

use crate::lock;

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
    /// 盘面布局(打开时判定;新中文目录或旧英文目录,写回一律原地)。
    layout: Layout,
    /// open_exclusive 持有的全程锁(含 Drop 自动释放)。
    lock: Option<lock::LockGuard>,
    /// 最近一次与磁盘同步时的 project 视图(去 `_meta`;写入窗口漂移检测的基准)。
    synced_disk: Option<serde_json::Value>,
}

/// 盘面布局(打开时判定一次,整个生命周期一致):
/// V2 中文目录 / V1 旧英文目录(兼容读写、原地保留、不自动迁移)/
/// V3 扁平布局(册六 ADR-0021;判定与映射表唯一来源 `paths`)。
#[derive(Clone, Copy)]
struct Layout {
    project_rel: &'static str,
    truths: &'static [(&'static str, &'static str)],
}

impl Layout {
    fn detect(root: &Path) -> Self {
        match crate::paths::detect_layout(root) {
            crate::paths::LayoutKind::Legacy => Self {
                project_rel: crate::paths::LEGACY_PROJECT_REL,
                truths: &crate::paths::FILE_TRUTHS_LEGACY,
            },
            crate::paths::LayoutKind::V3 => Self {
                project_rel: crate::paths::V3_PROJECT_REL,
                truths: &crate::paths::FILE_TRUTHS_V3,
            },
            crate::paths::LayoutKind::V2 => Self {
                project_rel: crate::paths::PROJECT_REL,
                truths: &crate::paths::FILE_TRUTHS_NEW,
            },
        }
    }
}

/// Reject → io::Error 映射(写路径各步骤共用)。
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
