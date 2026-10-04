// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 备份(计划书 4.3/全局约定 B.8):覆写工程文件前先备份到
//! `_内部状态/backup/{时间戳}-{rev}-{4位随机}/`(目录名唯一来源 `paths::STATE`;
//! 0.4.x 旧布局为 `_state/backup/`,见 LEGACY_STATE)。
//! BUG-12 修复:秒级时间戳同秒互覆 → 加 `{rev}` 与 4 位随机后缀,同秒多次备份
//! 各自成目录;增加保留策略(最多 [`BACKUP_KEEP`] 代,写后备顺带清理),
//! 清理失败仅 warn 不阻塞备份本体。内容写盘统一走 `atomic::atomic_write`
//! (唯一落盘点纪律);时间戳走 core::timeutil。

use crate::atomic::atomic_write;
use crate::paths;
use cutforge_core::timeutil;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// 备份保留代数(超出淘汰最旧;BUG-12:全仓此前无任何清理上限)。
pub const BACKUP_KEEP: usize = 50;

/// 4 位随机后缀熵源:进程内单调计数 × 时间低位 × pid 混合
/// (零依赖;同秒内多次调用计数器递增,目录名必不重复)。
static SUFFIX_SEQ: AtomicU32 = AtomicU32::new(0);

fn rand4() -> String {
    let n = SUFFIX_SEQ.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_millis();
    let mix = n.wrapping_mul(0x9E37_79B9)
        ^ t.wrapping_mul(0x85EB_CA6B)
        ^ (std::process::id().wrapping_mul(0xC2B2_AE35));
    format!("{:04x}", (mix >> 16) & 0xffff)
}

/// 备份目录所属 rev(`.cutforge/rev`;不可读 = 0)。
fn rev_of(root: &Path) -> u64 {
    std::fs::read_to_string(root.join(".cutforge/rev"))
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

fn backup_root(root: &Path) -> PathBuf {
    root.join(paths::STATE).join("backup")
}

/// 备份工程内相对路径 `rel` 的内容;返回备份文件位置。
/// 目录名 `{时间戳}-{rev}-{4位随机}`,同秒多次备份各自成目录;
/// 写后备顺带清理最旧代(保留 [`BACKUP_KEEP`] 代,失败仅 warn)。
/// 路径分隔符折叠为 `__`,避免在备份目录再造子树。
pub fn backup_file(root: &Path, rel: &str, content: &[u8]) -> io::Result<PathBuf> {
    let dir = backup_root(root).join(format!(
        "{}-{}-{}",
        timeutil::now_datetime_compact(),
        rev_of(root),
        rand4()
    ));
    let flat = rel.replace(['/', '\\'], "__");
    let dest = dir.join(flat);
    atomic_write(&dest, content)?;
    prune_backups(&backup_root(root), BACKUP_KEEP);
    Ok(dest)
}

/// 保留最近 `keep` 代(目录名字典序 = 时间序;清理失败仅 warn 不阻塞)。
fn prune_backups(dir: &Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    if names.len() <= keep {
        return;
    }
    names.sort();
    for old in names.iter().take(names.len() - keep) {
        if let Err(e) = crate::atomic::remove_dir_all(&dir.join(old)) {
            eprintln!("[cutforge-io][warn] 备份清理失败({old}): {e}(不阻塞备份与写路径)");
        }
    }
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
        assert!(
            name.starts_with("05_时间线工程__project.json"),
            "实际: {name}"
        );
        assert!(
            dest.parent()
                .unwrap()
                .starts_with(root.join(paths::STATE).join("backup"))
        );
        // BUG-12:目录名 = {时间戳}-{rev}-{4位随机}
        let dir_name = dest
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let parts: Vec<&str> = dir_name.split('-').collect();
        assert_eq!(parts.len(), 4, "目录名三段式: {dir_name}");
        assert_eq!(parts[0].len(), 8, "日期段 YYYYMMDD: {dir_name}");
        assert_eq!(parts[1].len(), 6, "时间段 HHMMSS: {dir_name}");
        assert!(
            parts[2].chars().all(|c| c.is_ascii_digit()),
            "rev 段: {dir_name}"
        );
        assert_eq!(parts[3].len(), 4, "4 位随机后缀: {dir_name}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"{}");
        fsutil::cleanup(&root);
    }

    /// BUG-12:同一秒内多次备份 → 各自成目录(旧实现秒级时间戳互覆)。
    #[test]
    fn same_second_backups_get_distinct_dirs() {
        let root = fsutil::temp_dir("cutforge-backup-same-sec");
        fsutil::ensure(&root).unwrap();
        let a = backup_file(&root, paths::PROJECT_REL, b"{\"v\":1}").unwrap();
        let b = backup_file(&root, paths::PROJECT_REL, b"{\"v\":2}").unwrap();
        assert_ne!(a.parent(), b.parent(), "同秒两次备份不得共用目录");
        assert_eq!(std::fs::read(a).unwrap(), b"{\"v\":1}");
        assert_eq!(std::fs::read(b).unwrap(), b"{\"v\":2}");
        fsutil::cleanup(&root);
    }

    /// BUG-12:超 [`BACKUP_KEEP`] 代 → 最旧代被清理,失败仅 warn。
    #[test]
    fn retention_prunes_oldest_beyond_keep() {
        let root = fsutil::temp_dir("cutforge-backup-prune");
        fsutil::ensure(&root.join(".cutforge")).unwrap();
        crate::atomic::atomic_write(&root.join(".cutforge/rev"), b"7\n").unwrap();
        let broot = backup_root(&root);
        for i in 0..(BACKUP_KEEP + 10) {
            backup_file(
                &root,
                paths::PROJECT_REL,
                format!("{{\"i\":{i}}}").as_bytes(),
            )
            .unwrap();
        }
        let mut dirs: Vec<String> = std::fs::read_dir(&broot)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        dirs.sort();
        assert_eq!(dirs.len(), BACKUP_KEEP, "必须保留 {BACKUP_KEEP} 代");
        assert_eq!(rev_of(&root), 7, "rev 进目录名的依据");
        fsutil::cleanup(&root);
    }
}
