// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 小工具:测试用临时工程目录(创建/清理)。
//! 本模块的目录创建同样走 `atomic` 的允许面之外——`create_dir_all` 是
//! 目录操作不是文件写,不属于 check-write-paths 的拦截对象,但仍集中在此。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// 生成唯一临时目录(不自动存在,调用方按需创建)。
pub fn temp_dir(tag: &str) -> PathBuf {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("cutforge-{tag}-{}-{ts}-{n}", std::process::id()))
}

/// 创建目录树。
pub fn ensure(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

/// 复制文件(读源 + 原子写目标;写面统一收敛到 atomic)。
pub fn copy_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    let data = std::fs::read(src)?;
    crate::atomic::atomic_write(dst, &data)
}

/// 递归清理(测试收尾;失败静默——临时目录由系统兜底)。
pub fn cleanup(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}
