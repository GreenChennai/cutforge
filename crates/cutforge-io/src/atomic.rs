//! 原子写入(计划书 4.2 步骤 6)。
//!
//! 本文件是**全仓唯一**允许调用文件系统写入 API 的位置
//! (`cargo run -p cutforge-cli -- check-write-paths` 强制,门禁 M2-4)。
//! 临时文件写全量 → rename 原子替换;追加走 append-only 打开。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// 原子写:同目录临时文件(唯一名)→ fsync → rename。
pub fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| io::Error::other("路径缺少父目录"))?;
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
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// 追加一行(append-only,OpLog .jsonl 用);调用方保证行内已含换行。
pub fn append_line(path: &Path, line: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = fs::OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(line.as_bytes())
}

/// 互斥创建(锁文件用:O_EXCL 语义,绕过 rename 会破坏排他性)。
pub fn create_exclusive(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(data)
}

/// 删除单个文件(锁释放/死锁接管/watcher 测试用)。
pub fn remove(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}
