// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 备份(计划书 4.3/全局约定 B.8):覆写工程文件前先备份到 `_内部状态/backup/<ts>/`
//! (目录名唯一来源 `paths::STATE`;0.4.x 旧布局为 `_state/backup/`,见 LEGACY_STATE)。
//! 内容写盘统一走 `atomic::atomic_write`(唯一落盘点纪律);时间戳走 core::timeutil。

use crate::atomic::atomic_write;
use crate::paths;
use cutforge_core::timeutil;
use std::io;
use std::path::{Path, PathBuf};

fn stamp() -> String {
    timeutil::now_datetime_compact()
}

/// 备份工程内相对路径 `rel` 的内容;返回备份文件位置。
/// 路径分隔符折叠为 `__`,避免在备份目录再造子树。
pub fn backup_file(root: &Path, rel: &str, content: &[u8]) -> io::Result<PathBuf> {
    let dir = root.join(paths::STATE).join("backup").join(stamp());
    let flat = rel.replace(['/', '\\'], "__");
    let dest = dir.join(flat);
    atomic_write(&dest, content)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil;

    #[test]
    fn backup_writes_flattened_copy() {
        let root = fsutil::temp_dir("cutforge-backup");
        fsutil::ensure(&root).unwrap();
        let dest = backup_file(&root, paths::PROJECT_REL, b"{}").unwrap();
        let name = dest.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("05_时间线工程__project.json"), "实际: {name}");
        assert!(dest.parent().unwrap().starts_with(root.join(paths::STATE).join("backup")));
        assert_eq!(std::fs::read(&dest).unwrap(), b"{}");
        fsutil::cleanup(&root);
    }
}
