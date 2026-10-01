// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 预演副本工程(册七 T7.5 preview_plan):把工程复制到同卷邻位临时目录,
//! 供 AI 改动预演在**副本**上全链 dry-run——不落盘真工程、不产真 Op,
//! 防"部分应用"污染真相源。
//!
//! 复制策略(逐文件三级回退):
//!   1. 硬链接(同卷零拷贝,GB 级媒体瞬时完成;副本侧任何写入都是
//!      "临时文件+改名"的原子替换,只换目录项不碰原 inode——原工程逐字节安全);
//!   2. 整拷贝(跨卷回退;预演目录与工程同父目录,正常路径走不到);
//!   3. 空占位(超大文件跨卷兜底:路径校验可过、媒体探测诚实报错,
//!      预演错误面可见而非静默幻觉)。
//!
//! 跳过面(不进副本):成片输出目录(新/旧布局)、exports、渲染缓存、
//! baseRev 快照备份目录、`.cutforge/lock`(避免把真工程的锁带进副本)、
//! `.cutforge/session-summary.json`(会话簿记属真工程,不属预演)。

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::paths;

/// 成片/产物目录(新/旧布局 + v3 exports):纯渲染产物,预演不需要。
const SKIP_DIRS: [&str; 5] = [
    paths::OUTPUT,
    paths::LEGACY_OUTPUT,
    paths::V3_EXPORTS,
    "_内部状态/backup",
    "_state/backup",
];

/// 副本内跳过文件:真工程的锁与会话簿记绝不进副本。
fn skipped_file(rel: &Path) -> bool {
    let s = rel.to_string_lossy().replace('\\', "/");
    s == ".cutforge/lock" || s == ".cutforge/session-summary.json"
}

fn skipped_dir(rel: &Path) -> bool {
    let s = rel.to_string_lossy().replace('\\', "/");
    SKIP_DIRS.iter().any(|d| s == *d) || s.ends_with("/render-cache")
}

/// 建副本工程根(调用方负责 [`remove_scratch`] 清理;唯一出口见 [`ai_scratch_root`])。
/// 目录名:`.cf-scratch-<pid>-<纳秒>`(工程父目录下,同卷 → 硬链接必成)。
pub fn make_scratch_copy(root: &Path) -> io::Result<PathBuf> {
    if !crate::paths::has_project(root) {
        return Err(io::Error::new(io::ErrorKind::NotFound, "工程不存在(缺 project.json)"));
    }
    let dir = ai_scratch_root(root)?;
    // 防撞车:同名残留(上次进程崩溃)→ 先清再建
    let _ = std::fs::remove_dir_all(&dir);
    copy_tree(root, &dir, Path::new(""))?;
    Ok(dir)
}

/// 副本根定位:优先工程父目录(同卷硬链接),无父目录(盘根)回退系统临时目录。
pub fn ai_scratch_root(root: &Path) -> io::Result<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::process::id().hash(&mut h);
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos().hash(&mut h);
    let name = format!(".cf-scratch-{:016x}", h.finish());
    let base = match root.parent() {
        Some(p) if p.is_dir() => p.to_path_buf(),
        _ => crate::fsutil::temp_dir("cf-scratch"),
    };
    Ok(base.join(name))
}

/// 删除副本(尽力而为;失败不上抛——临时目录残留可被下次同名清理吸收)。
pub fn remove_scratch(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn copy_tree(src_root: &Path, dst_root: &Path, rel: &Path) -> io::Result<()> {
    let src = src_root.join(rel);
    let dst = dst_root.join(rel);
    // 跳过目录:不建不递归(工程根 rel 为空除外)
    if !rel.as_os_str().is_empty() && skipped_dir(rel) {
        return Ok(());
    }
    let meta = std::fs::metadata(&src)?;
    if meta.is_dir() {
        std::fs::create_dir_all(&dst)?;
        for entry in std::fs::read_dir(&src)? {
            let entry = entry?;
            let child_rel = rel.join(entry.file_name());
            copy_tree(src_root, dst_root, &child_rel)?;
        }
        return Ok(());
    }
    if skipped_file(rel) {
        return Ok(());
    }
    std::fs::create_dir_all(dst.parent().unwrap_or(&dst))?;
    // 三级回退:硬链接 → 整拷贝 → 空占位
    if std::fs::hard_link(&src, &dst).is_ok() {
        return Ok(());
    }
    if std::fs::copy(&src, &dst).is_ok() {
        return Ok(());
    }
    std::fs::File::create(&dst)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 副本与真工程隔离:副本可写、真工程字节不变;跳过面不进副本。
    #[test]
    fn scratch_copy_is_writable_and_isolated() {
        let root = crate::tests_fixture("scratch-basic").unwrap();
        let root_s = root.to_string_lossy().to_string();
        let project = std::fs::read(root.join(crate::paths::PROJECT_REL)).unwrap();
        let dir = make_scratch_copy(&root).unwrap();
        // 真相源进了副本(project.json + notes.json + oplog/rev 面任意之一)
        assert!(dir.join(crate::paths::PROJECT_REL).is_file());
        // 副本可独立打开并写入(硬链接不传回真工程)
        let mut ws = crate::Workspace::open_exclusive(&dir).unwrap();
        let rec = ws
            .apply(
                cutforge_core::command::Command::ClipUpdate {
                    clip_id: "V1-001".into(),
                    patch: cutforge_core::command::ClipPatch {
                        volume: Some(0.5),
                        ..Default::default()
                    },
                },
                cutforge_core::oplog::Actor::agent("scratch-test"),
                cutforge_core::engine::ApplyOpts::default(),
            )
            .unwrap();
        assert_eq!(rec.rev, 1);
        drop(ws);
        // 真工程逐字节不变 + rev 不变
        assert_eq!(std::fs::read(root.join(crate::paths::PROJECT_REL)).unwrap(), project);
        remove_scratch(&dir);
        assert!(!dir.exists(), "副本清理后必须消失");
        // 真工程仍可打开(rev 未动)
        let ws2 = crate::Workspace::open(Path::new(&root_s)).unwrap();
        assert_eq!(ws2.rev(), 0);
        crate::fsutil::cleanup(&root);
    }
}
