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
//!
//! append-only 强制整拷贝面(P0,A7 收口):`.cutforge/oplog/**` 与
//! `.cutforge/rev` 绝不硬链接——OpLog 追加走 append 模式直写 inode
//! (`atomic::append_line`),副本侧追加会经共享 inode **穿透写回真工程**
//! (G2 实证:真 oplog 出现预演 Op、真 rev 被推高);rev 虽走临时文件+改名,
//! 同列排除以保"副本一旦建出即与真工程盘面完全独立"的硬性质。
//!
//! 写路径纪律收口(check-write-paths M2,唯一落盘点):整拷贝级只对
//! [`COPY_INLINE_MAX`] 内的小文件读入后经 `atomic::atomic_write` 落盘
//! (真相源/工程 JSON 均为 KB 级);超过阈值(GB 级媒体)不整拷,直接落
//! 空占位——与第三级"路径校验可过、媒体探测诚实报错,预演错误面可见"
//! 的既有语义一致,且内存有界。空占位同走 `atomic::atomic_write`。

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

/// 整拷贝内联上限:副本回退级只内联复制该大小以下的文件(真相源/工程 JSON 均
/// 为 KB 级;超过即 GB 级媒体,落空占位走"探测诚实报错"语义,内存有界)。
const COPY_INLINE_MAX: u64 = 8 * 1024 * 1024;

/// 副本内跳过文件:真工程的锁与会话簿记绝不进副本。
fn skipped_file(rel: &Path) -> bool {
    let s = rel.to_string_lossy().replace('\\', "/");
    s == ".cutforge/lock" || s == ".cutforge/session-summary.json"
}

/// append-only 强制整拷贝面(P0):oplog 与 rev 不参与硬链接回退。
/// OpLog 追加是 append 模式直写 inode(`atomic::append_line`,全仓唯一
/// append 写面),硬链接下副本侧追加必然穿透真工程;rev 同列排除,
/// 使副本建出后即与真工程盘面完全独立(见模块头)。
fn force_copy_file(rel: &Path) -> bool {
    let s = rel.to_string_lossy().replace('\\', "/");
    s == ".cutforge/rev" || s.starts_with(".cutforge/oplog/")
}

fn skipped_dir(rel: &Path) -> bool {
    let s = rel.to_string_lossy().replace('\\', "/");
    SKIP_DIRS.iter().any(|d| s == *d) || s.ends_with("/render-cache")
}

/// 建副本工程根(调用方负责 [`remove_scratch`] 清理;唯一出口见 [`ai_scratch_root`])。
/// 目录名:`.cf-scratch-<pid>-<纳秒>`(工程父目录下,同卷 → 硬链接必成)。
pub fn make_scratch_copy(root: &Path) -> io::Result<PathBuf> {
    if !crate::paths::has_project(root) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "工程不存在(缺 project.json)",
        ));
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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
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
    // 三级回退:硬链接 → 整拷贝(小文件内联)→ 空占位;append-only 面(oplog/rev)
    // 跳过硬链接;两级落盘均走唯一落盘点 atomic::atomic_write(check-write-paths M2)
    if !force_copy_file(rel) && std::fs::hard_link(&src, &dst).is_ok() {
        return Ok(());
    }
    if meta.len() <= COPY_INLINE_MAX
        && let Ok(bytes) = std::fs::read(&src)
    {
        return crate::atomic::atomic_write(&dst, &bytes);
    }
    // 大文件(跨卷兜底)或读取失败:空占位——媒体探测诚实报错,错误面可见
    crate::atomic::atomic_write(&dst, &[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use cutforge_core::command::{ClipPatch, Command};
    use cutforge_core::engine::ApplyOpts;
    use cutforge_core::oplog::Actor;
    use std::path::PathBuf;

    fn clip_update(volume: f64) -> Command {
        Command::ClipUpdate {
            clip_id: "V1-001".into(),
            patch: ClipPatch {
                volume: Some(volume),
                ..Default::default()
            },
        }
    }

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
                clip_update(0.5),
                Actor::agent("scratch-test"),
                ApplyOpts::default(),
            )
            .unwrap();
        assert_eq!(rec.rev, 1);
        drop(ws);
        // 真工程逐字节不变 + rev 不变
        assert_eq!(
            std::fs::read(root.join(crate::paths::PROJECT_REL)).unwrap(),
            project
        );
        remove_scratch(&dir);
        assert!(!dir.exists(), "副本清理后必须消失");
        // 真工程仍可打开(rev 未动)
        let ws2 = crate::Workspace::open(Path::new(&root_s)).unwrap();
        assert_eq!(ws2.rev(), 0);
        crate::fsutil::cleanup(&root);
    }

    /// P0 回归(文件级,A7 收口):真工程**已有 OpLog 内容**时,副本上的连续写
    /// (oplog 追加 / rev 推进)绝不穿透回真工程——append-only 面强制整拷贝。
    /// G1 的隔离测试夹具零 Op(oplog 文件不存在,副本侧新建 inode 不穿透),
    /// 检不出本缺陷(G2 实证:真 oplog 出现预演 Op、真 rev 被推高);本测试
    /// 补到文件字节级:oplog 逐文件字节一致 + rev 不变 + 副本清理后零残留。
    #[test]
    fn scratch_append_only_files_never_penetrate() {
        // 工程建在测试自有的外层目录内:工程父目录 = 外层(副本落点),
        // 残留断言可精确圈定,不与共享临时目录里其他进程的产物串扰
        let outer = crate::fsutil::temp_dir("scratch-penetre");
        crate::fsutil::ensure(&outer).unwrap();
        let root = outer.join("proj");
        crate::fsutil::ensure(&root.join(crate::paths::TIMELINE)).unwrap();
        let sample =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/regression/talking-head");
        crate::fsutil::copy_file(
            &sample.join("project.json"),
            &root.join(crate::paths::PROJECT_REL),
        )
        .unwrap();
        crate::fsutil::copy_file(
            &sample.join("notes.json"),
            &root.join(crate::paths::NOTES_REL),
        )
        .unwrap();

        // 种子写:真工程先产真实 OpLog(rev=1、oplog/<日>.jsonl 在盘)——
        // 这是穿透场景的前提(真 oplog 文件在盘且会被硬链接)
        let mut seed = crate::Workspace::open_exclusive(&root).unwrap();
        let rec = seed
            .apply(clip_update(0.9), Actor::agent("seed"), ApplyOpts::default())
            .unwrap();
        assert_eq!(rec.rev, 1);
        drop(seed);
        let rev_before = std::fs::read_to_string(root.join(".cutforge/rev")).unwrap();
        let oplog_dir = root.join(".cutforge/oplog");
        let oplog_files: Vec<(String, Vec<u8>)> = std::fs::read_dir(&oplog_dir)
            .unwrap()
            .map(|e| e.unwrap())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".jsonl"))
            .map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                (name, std::fs::read(e.path()).unwrap())
            })
            .collect();
        assert!(
            !oplog_files.is_empty(),
            "种子写后真工程必须有 oplog 文件(本测试前提)"
        );

        // 副本上连续写两笔(模拟预演 plan 的逐项 dry-run:副本 rev 推进、oplog 追加)
        let dir = make_scratch_copy(&root).unwrap();
        let mut ws = crate::Workspace::open_exclusive(&dir).unwrap();
        assert_eq!(ws.rev(), 1, "副本继承真工程 rev");
        ws.apply(
            clip_update(0.5),
            Actor::agent("scratch-preview"),
            ApplyOpts::default(),
        )
        .unwrap();
        ws.apply(
            clip_update(0.6),
            Actor::agent("scratch-preview"),
            ApplyOpts::default(),
        )
        .unwrap();
        assert_eq!(ws.rev(), 3, "副本侧 rev 独立推进");
        drop(ws);
        remove_scratch(&dir);
        assert!(!dir.exists(), "副本清理后必须消失");

        // 文件级断言:真工程 oplog 逐文件字节一致(append 穿透在此现形)
        for (name, bytes) in &oplog_files {
            assert_eq!(
                &std::fs::read(oplog_dir.join(name)).unwrap(),
                bytes,
                "真工程 oplog/{name} 被副本追加穿透(P0 回归)"
            );
        }
        // rev 不变 + 副本名(.cf-scratch-*)零残留(工程父目录级)
        assert_eq!(
            std::fs::read_to_string(root.join(".cutforge/rev")).unwrap(),
            rev_before,
            "真工程 rev 被推高(P0 回归)"
        );
        let residue: Vec<String> = std::fs::read_dir(&outer)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(".cf-scratch"))
            .collect();
        assert!(
            residue.is_empty(),
            "副本清理后工程父目录不得有 .cf-scratch 残留: {residue:?}"
        );
        // 真工程仍可打开且 rev 不变(oplog 对账不吞穿透 Op 的最终证明)
        let ws2 = crate::Workspace::open(Path::new(&root.to_string_lossy().to_string())).unwrap();
        assert_eq!(ws2.rev(), 1);
        crate::fsutil::cleanup(&outer);
    }
}
