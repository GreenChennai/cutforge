// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 原子写入(计划书 4.2 步骤 6)。
//!
//! 本文件是**全仓唯一**允许调用文件系统写入 API 的位置
//! (`cargo run -p cutforge-cli -- check-write-paths` 强制,门禁 M2-4)。
//! 临时文件写全量 → fsync → rename → **父目录 fsync**;追加走 append-only 打开,
//! 每条 `sync_data`(R-01 fsync 纪律:rename 只保证原子性,父目录 fsync 才保证
//! 持久性;`write()` 返回 ≠ 落盘)。批量追加用 [`append_lines`] 组提交:
//! 单次打开、逐行 write_all、**返回前 sync 一次**(等价"apply 返回前 sync 一次",
//! 全部在 io 层 persist 管线内完成,不需要内核改动调用点)。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// 父目录 fsync(R-01:目录项持久化;Windows 需 FILE_FLAG_BACKUP_SEMANTICS
/// 才能打开目录句柄,且 FlushFileBuffers 要求写权限)。
fn fsync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        let f = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(dir)?;
        f.sync_all()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let f = fs::File::open(dir)?;
        f.sync_all()
    }
}

/// 原子写:同目录临时文件(唯一名)→ fsync → rename → 父目录 fsync。
pub fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("路径缺少父目录"))?;
    fs::create_dir_all(dir)?;
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("file");
    let tmp: PathBuf = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let write = || -> io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        Ok(())
    };
    if let Err(e) = write() {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    match fs::rename(&tmp, path) {
        Ok(()) => {}
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }
    // R-01:rename 后父目录 fsync,目录项变更才对掉电持久。
    fsync_dir(dir)
}

/// 追加一行(append-only,OpLog .jsonl 用);调用方保证行内已含换行。
/// R-01:每条 `sync_data`——Op 记账返回前必须落盘(单条持久原语)。
pub fn append_line(path: &Path, line: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(line.as_bytes())?;
    f.sync_data()
}

/// 批量追加(组提交,R-01 性能预案):单次打开、逐行 write_all、
/// **返回前 sync 一次**。持久语义与逐条 [`append_line`] 等价
/// (调用方返回前已落盘),批量场景省 N−1 次 sync 开销。
/// 中途失败可能留下半行(与逐条形态一致),由 R-03 修复模式兜底。
pub fn append_lines(path: &Path, lines: &[String]) -> io::Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    for line in lines {
        f.write_all(line.as_bytes())?;
    }
    f.sync_data()
}

/// 互斥创建(锁文件用:O_EXCL 语义,绕过 rename 会破坏排他性)。
pub fn create_exclusive(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(data)
}

/// 删除单个文件(锁释放/死锁接管/watcher 测试用)。
pub fn remove(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

/// 结构性改名/搬移(册六 T6.1:布局迁移器与工程库的目录级操作原语)。
/// 目录/文件整体 rename 在同卷内原子,不经临时文件(单文件落盘语义不适用整目录)。
pub fn rename(src: &Path, target: &Path) -> io::Result<()> {
    fs::rename(src, target)
}

/// 递归删除目录(腾空目录移除/回收站清理;文件删除走 [`remove`])。
pub fn remove_dir_all(path: &Path) -> io::Result<()> {
    fs::remove_dir_all(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil;
    use std::time::Instant;

    #[test]
    fn atomic_write_survives_immediate_exit_and_readback() {
        let dir = fsutil::temp_dir("cutforge-atomic");
        fsutil::ensure(&dir).unwrap();
        let p = dir.join("a.json");
        atomic_write(&p, b"{\"x\":1}").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"{\"x\":1}");
        atomic_write(&p, b"{\"x\":2}").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"{\"x\":2}", "重写必须原子替换");
        // 无残留临时文件
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "临时文件不得残留: {leftovers:?}");
        fsutil::cleanup(&dir);
    }

    /// R-01 性能数据(审查报告 §5 R-01 性能预案):批量 100 Op 的三种形态——
    /// 无 fsync 基线(修复前)/ 逐条 sync_data / 组提交(返回前 sync 一次)。
    /// 阈值:逐条 sync 相对基线拖慢 >15% → 启用组提交(单 Op persist 两者等价)。
    /// 运行:`cargo test -p cutforge-io -j 2 fsync -- --ignored --nocapture`
    #[test]
    #[ignore = "fsync 性能基线(手动运行;计时断言不进 CI 防抖)"]
    fn fsync_bench_batch_100_ops_per_line_vs_group_commit() {
        use std::io::Write as _;
        let dir = fsutil::temp_dir("cutforge-fsync-bench");
        fsutil::ensure(&dir).unwrap();
        let line = |i: usize| format!("{{\"op_id\":\"op-{i}\",\"n\":{i}}}\n");

        // (0) 无 fsync 基线(修复前形态:write_all 即返回)
        let baseline = dir.join("baseline.jsonl");
        let tb = Instant::now();
        let mut f0 = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&baseline)
            .unwrap();
        for i in 0..100 {
            f0.write_all(line(i).as_bytes()).unwrap();
        }
        drop(f0);
        let baseline_ms = tb.elapsed().as_millis();

        // (a) 逐条 append_line(每条 sync_data)
        let per_line = dir.join("per-line.jsonl");
        let t0 = Instant::now();
        for i in 0..100 {
            append_line(&per_line, &line(i)).unwrap();
        }
        let per_line_ms = t0.elapsed().as_millis();

        // (b) 组提交 append_lines(一次 sync)
        let group = dir.join("group.jsonl");
        let t1 = Instant::now();
        let lines: Vec<String> = (0..100).map(line).collect();
        append_lines(&group, &lines).unwrap();
        let group_ms = t1.elapsed().as_millis();

        // (c) atomic_write ×100(含父目录 fsync 的整文件替换形态)
        let f = dir.join("aw.json");
        let t2 = Instant::now();
        for i in 0..100 {
            atomic_write(&f, line(i).as_bytes()).unwrap();
        }
        let aw_ms = t2.elapsed().as_millis();

        // 内容一致性:三种方式逐字节等价
        assert_eq!(fs::read(&baseline).unwrap(), fs::read(&group).unwrap());
        assert_eq!(fs::read(&per_line).unwrap(), fs::read(&group).unwrap());
        println!("fsync bench(debug 构建,批量 100 Op):");
        println!("  无 fsync 基线: {baseline_ms} ms");
        println!("  逐条 sync_data: {per_line_ms} ms");
        println!("  组提交(1 次 sync): {group_ms} ms");
        println!("  atomic_write ×100(含父目录 fsync): {aw_ms} ms");
        let drag = if baseline_ms == 0 {
            f64::INFINITY
        } else {
            (per_line_ms as f64 / baseline_ms as f64 - 1.0) * 100.0
        };
        println!("  逐条 sync 相对基线拖慢: {drag:.1}%(>15% → 启用组提交)");
        fsutil::cleanup(&dir);
    }
}
