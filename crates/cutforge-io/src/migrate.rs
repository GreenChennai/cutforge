// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 布局迁移器(册六 T6.1/ADR-0021):V1(0.4.x 英文目录)/ V2(中文阶段目录)
//! → V3 扁平布局,一次性到位、幂等、冲突整体拒绝。
//!
//! 纪律:
//! - 目录/文件搬运走 `atomic::rename` 结构性原语(同盘原生改名,同卷内原子;
//!   跨盘回退 copy+delete,文件字节仍走 fsutil::copy_file 的唯一落盘点);
//! - 预检先行:任一映射目标已存在 → 整体拒绝(`MigrateError::Conflict`),
//!   绝不出现"迁一半"的盘面;
//! - 迁移期间持工程锁(`lock::acquire`),活进程持锁 → 拒绝;
//! - OpLog/rev/notes.json/`.cutforge/` 相对工程根不变,零搬运零改写——
//!   OpLog 完整性与撤销链原样保留(AC-6.2);
//! - project.json 内容零字节改动(挪位不改内容;`_meta` 旁路等语义不受影响)。

use crate::lock;
use crate::paths::{self, LayoutKind};
use std::io;
use std::path::{Path, PathBuf};

/// 迁移映射的执行报告(逐项如实列出;kept = 有意不迁的残留)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MigrateReport {
    /// 迁移前布局("v1"/"v2"/"v3")。
    pub from: &'static str,
    /// 目标已为 v3,未做任何搬动(幂等 NOOP)。
    pub idempotent: bool,
    /// 实际搬动(源相对路径, 目标相对路径)。
    pub moved: Vec<(String, String)>,
    /// 搬空后移除的目录(相对路径)。
    pub removed_empty_dirs: Vec<String>,
    /// 原地保留的目录(相对路径;非契约残留如实报告,不强迁)。
    pub kept: Vec<String>,
}

#[derive(Debug)]
pub enum MigrateError {
    /// 目录不是可打开的工程(三态皆无 project.json)。
    NotAProject(PathBuf),
    /// 预检冲突:映射目标已存在(整体拒绝,盘面未动)。
    Conflict(String),
    /// 工程(或目标)被活进程持锁。
    Locked(String),
    Io(io::Error),
}

impl std::fmt::Display for MigrateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrateError::NotAProject(p) => write!(f, "不是可打开的工程(三态布局皆无 project.json): {}", p.display()),
            MigrateError::Conflict(m) => write!(f, "CONFLICT: {m}(整体拒绝,盘面未动)"),
            MigrateError::Locked(m) => write!(f, "工程被其他进程锁定: {m}"),
            MigrateError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for MigrateError {
    fn from(e: io::Error) -> Self {
        MigrateError::Io(e)
    }
}

fn layout_label(k: LayoutKind) -> &'static str {
    match k {
        LayoutKind::Legacy => "v1",
        LayoutKind::V2 => "v2",
        LayoutKind::V3 => "v3",
    }
}

/// 源布局的四个阶段目录(时间线/粗剪/素材/成片输出;V1/V2 各一套)。
fn source_dirs(layout: LayoutKind) -> (&'static str, &'static str, &'static str, &'static str) {
    match layout {
        LayoutKind::Legacy => (
            paths::LEGACY_TIMELINE,
            paths::LEGACY_CUT,
            paths::LEGACY_MATERIALS,
            paths::LEGACY_OUTPUT,
        ),
        _ => (paths::TIMELINE, paths::CUT, paths::MATERIALS, paths::OUTPUT),
    }
}

/// V1/V2 → V3 迁移(幂等:v3 工程直接 NOOP)。
pub fn migrate_to_v3(root: &Path) -> Result<MigrateReport, MigrateError> {
    let layout = paths::detect_layout(root);
    if !paths::has_project(root) {
        return Err(MigrateError::NotAProject(root.to_path_buf()));
    }
    if layout == LayoutKind::V3 {
        return Ok(MigrateReport { from: "v3", idempotent: true, ..Default::default() });
    }
    let (timeline_rel, cut_rel, materials_rel, output_rel) = source_dirs(layout);

    // ---- 预检:算出全部映射,任一目标已存在 → 整体拒绝(盘面未动) ----
    let mut plan: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut moved: Vec<(String, String)> = Vec::new();
    // 1) 时间线真相源文件 → 根(V3 契约位)
    for (name, target) in [
        ("project.json", paths::V3_PROJECT_REL),
        ("wordline.json", paths::V3_WORDLINE_REL),
    ] {
        let src = root.join(timeline_rel).join(name);
        if src.is_file() {
            plan.push((src, root.join(target)));
            moved.push((format!("{timeline_rel}/{name}"), target.to_string()));
        }
    }
    // 2) 粗剪决策文件 → 根
    for (name, target) in [
        ("cutlist.json", paths::V3_CUTLIST_REL),
        ("cutlist.applied.json", paths::V3_CUTLIST_APPLIED_REL),
    ] {
        let src = root.join(cut_rel).join(name);
        if src.is_file() {
            plan.push((src, root.join(target)));
            moved.push((format!("{cut_rel}/{name}"), target.to_string()));
        }
    }
    // 3) 素材目录 → media/、成片输出目录 → exports/(整目录改名)
    for (src_rel, target_rel) in [(materials_rel, paths::V3_MEDIA), (output_rel, paths::V3_EXPORTS)] {
        if root.join(src_rel).is_dir() {
            plan.push((root.join(src_rel), root.join(target_rel)));
            moved.push((src_rel.to_string(), target_rel.to_string()));
        }
    }
    for (_, target) in &plan {
        if target.exists() {
            return Err(MigrateError::Conflict(format!(
                "迁移目标已存在: {}(先处理再迁移;迁移器拒绝合并/覆盖)",
                target.display()
            )));
        }
    }

    // ---- 持锁执行(活进程持锁 → 拒绝;崩溃残留锁由 stale 接管) ----
    let _guard = lock::acquire(root, 30_000, 2)
        .map_err(|e| MigrateError::Locked(e.to_string()))?;
    // 锁内复验(持锁等待期间盘面可能被他人改动)
    for (_, target) in &plan {
        if target.exists() {
            return Err(MigrateError::Conflict(format!("迁移目标已存在: {}", target.display())));
        }
    }
    let mut report =
        MigrateReport { from: layout_label(layout), idempotent: false, moved, ..Default::default() };
    for (src, target) in plan {
        move_path(&src, &target)?;
    }
    // 腾空目录移除(timeline/cut 目录若已无内容;有残留文件则原地保留并如实报告)
    for dir_rel in [timeline_rel, cut_rel] {
        let dir = root.join(dir_rel);
        if dir.is_dir() && is_empty_dir(&dir) {
            crate::atomic::remove_dir_all(&dir)?;
            report.removed_empty_dirs.push(dir_rel.to_string());
        } else if dir.is_dir() {
            report.kept.push(dir_rel.to_string());
        }
    }
    // 上游叙事目录(00/02/03/_内部状态)原地保留,如实报告(ADR-0021:不强迁不发明语义)
    for keep_rel in [paths::BRIEF, paths::SENSED, paths::ASSETS, paths::STATE] {
        if root.join(keep_rel).is_dir() {
            report.kept.push(keep_rel.to_string());
        }
    }
    Ok(report)
}

/// 同卷 rename;跨卷(CrossesDevices)回退 copy+delete。目标必须不存在(预检保证)。
/// 落盘点纪律:结构性搬移走 `atomic::rename`,文件字节走 `fsutil::copy_file`(atomic 唯一写)。
fn move_path(src: &Path, target: &Path) -> io::Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match crate::atomic::rename(src, target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            copy_recursive(src, target)?;
            if src.is_dir() {
                crate::atomic::remove_dir_all(src)
            } else {
                crate::atomic::remove(src)
            }
        }
        Err(e) => Err(e),
    }
}

fn copy_recursive(src: &Path, target: &Path) -> io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(target)?;
        for entry in std::fs::read_dir(src)?.flatten() {
            copy_recursive(&entry.path(), &target.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        crate::fsutil::copy_file(src, target)
    }
}

fn is_empty_dir(dir: &Path) -> bool {
    std::fs::read_dir(dir).map(|mut rd| rd.next().is_none()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{atomic, fsutil, Workspace};

    fn make_v2(root: &Path) {
        fsutil::ensure(&root.join(paths::TIMELINE)).unwrap();
        fsutil::ensure(&root.join(paths::CUT)).unwrap();
        fsutil::ensure(&root.join(paths::MATERIALS)).unwrap();
        fsutil::ensure(&root.join(paths::OUTPUT)).unwrap();
        let minimal = format!("{{\"version\":1,\"schemaVersion\":\"3.0.0\",\"slug\":\"{}\",\"fps\":30,\"canvas\":{{\"width\":1080,\"height\":1920}},\"backends\":[\"ffmpeg\"],\"tracks\":[]}}", "迁客体");
        atomic::atomic_write(&root.join(paths::PROJECT_REL), minimal.as_bytes()).unwrap();
        atomic::atomic_write(&root.join(paths::WORDLINE_REL), b"{}").unwrap();
        atomic::atomic_write(&root.join(paths::CUTLIST_REL), b"{}").unwrap();
        atomic::atomic_write(&root.join(paths::MATERIALS).join("a.mp4"), b"m").unwrap();
        atomic::atomic_write(&root.join(paths::OUTPUT).join("final.mp4"), b"o").unwrap();
    }

    #[test]
    fn migrate_v2_to_v3_and_idempotent() {
        let root = fsutil::temp_dir("migrate-v2");
        make_v2(&root);
        let before = std::fs::read(root.join(paths::PROJECT_REL)).unwrap();
        let r = migrate_to_v3(&root).unwrap();
        assert_eq!(r.from, "v2");
        assert!(!r.idempotent);
        assert_eq!(paths::detect_layout(&root), LayoutKind::V3);
        assert!(root.join(paths::V3_PROJECT_REL).is_file());
        assert!(root.join(paths::V3_WORDLINE_REL).is_file());
        assert!(root.join(paths::V3_CUTLIST_REL).is_file());
        assert!(root.join(paths::V3_MEDIA).join("a.mp4").is_file());
        assert!(root.join(paths::V3_EXPORTS).join("final.mp4").is_file());
        assert!(!root.join(paths::TIMELINE).exists(), "腾空目录必须移除");
        assert!(!root.join(paths::MATERIALS).exists());
        // 幂等:再跑 = NOOP
        let r2 = migrate_to_v3(&root).unwrap();
        assert!(r2.idempotent);
        assert!(r2.moved.is_empty());
        // 内容零丢失:project.json 字节原样(迁移前后逐字节一致)
        assert_eq!(std::fs::read(root.join(paths::V3_PROJECT_REL)).unwrap(), before,
            "迁移前后 project.json 必须逐字节一致");
        fsutil::cleanup(&root);
    }

    #[test]
    fn migrate_conflict_refuses_and_leaves_disk_untouched() {
        let root = fsutil::temp_dir("migrate-conflict");
        make_v2(&root);
        // 预置目标 exports/ 已存在 → 整体拒绝,盘面原样
        fsutil::ensure(&root.join(paths::V3_EXPORTS)).unwrap();
        let err = migrate_to_v3(&root).unwrap_err();
        assert!(matches!(err, MigrateError::Conflict(_)), "{err}");
        assert!(root.join(paths::PROJECT_REL).is_file(), "盘面不得被动过");
        assert_eq!(paths::detect_layout(&root), LayoutKind::V2);
        fsutil::cleanup(&root);
    }

    #[test]
    fn migrate_keeps_residual_and_not_a_project() {
        let root = fsutil::temp_dir("migrate-residual");
        make_v2(&root);
        atomic::atomic_write(&root.join(paths::TIMELINE).join("subs.srt"), b"s").unwrap();
        let r = migrate_to_v3(&root).unwrap();
        // 残留文件目录原地保留并如实报告
        assert!(r.kept.contains(&paths::TIMELINE.to_string()));
        assert!(root.join(paths::TIMELINE).join("subs.srt").is_file());
        // 迁移后工程照常打开(OpLog 完整性不动)
        let ws = Workspace::open(&root).unwrap();
        assert_eq!(ws.rev(), 0);
        // 非工程目录 → NotAProject
        let empty = fsutil::temp_dir("migrate-empty");
        assert!(matches!(migrate_to_v3(&empty), Err(MigrateError::NotAProject(_))));
        fsutil::cleanup(&root);
        fsutil::cleanup(&empty);
    }

    #[test]
    fn migrate_v1_legacy_to_v3() {
        let root = fsutil::temp_dir("migrate-v1");
        fsutil::ensure(&root.join(paths::LEGACY_TIMELINE)).unwrap();
        fsutil::ensure(&root.join(paths::LEGACY_MATERIALS)).unwrap();
        atomic::atomic_write(&root.join(paths::LEGACY_PROJECT_REL), b"{}").unwrap();
        atomic::atomic_write(&root.join(paths::LEGACY_WORDLINE_REL), b"{}").unwrap();
        atomic::atomic_write(&root.join(paths::LEGACY_MATERIALS).join("b.mp4"), b"m").unwrap();
        let r = migrate_to_v3(&root).unwrap();
        assert_eq!(r.from, "v1");
        assert_eq!(paths::detect_layout(&root), LayoutKind::V3);
        assert!(root.join(paths::V3_PROJECT_REL).is_file());
        assert!(root.join(paths::V3_MEDIA).join("b.mp4").is_file());
        fsutil::cleanup(&root);
    }
}
