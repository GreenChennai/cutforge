//! 备份(计划书 4.3/全局约定 B.8):覆写工程文件前先备份到 `_state/backup/<ts>/`。
//! 内容写盘统一走 `atomic::atomic_write`(唯一落盘点纪律)。

use crate::atomic::atomic_write;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn stamp() -> String {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let secs = ms / 1000;
    let d = secs / 86_400;
    // civil-from-days(与 core::timeutil 同算法,这里只取日期+时分秒)
    let z = d as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mon = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if mon <= 2 { y + 1 } else { y };
    let sod = secs % 86_400;
    let (hour, min, sec) = (sod / 3600, (sod % 3600) / 60, sod % 60);
    format!("{year:04}{mon:02}{day:02}-{hour:02}{min:02}{sec:02}")
}

/// 备份工程内相对路径 `rel` 的内容;返回备份文件位置。
/// 路径分隔符折叠为 `__`,避免在备份目录再造子树。
pub fn backup_file(root: &Path, rel: &str, content: &[u8]) -> io::Result<PathBuf> {
    let dir = root.join("_state").join("backup").join(stamp());
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
        let dest = backup_file(&root, "05_ir/project.json", b"{}").unwrap();
        let name = dest.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("05_ir__project.json"), "实际: {name}");
        assert!(dest.parent().unwrap().starts_with(root.join("_state/backup")));
        assert_eq!(std::fs::read(&dest).unwrap(), b"{}");
        fsutil::cleanup(&root);
    }
}
