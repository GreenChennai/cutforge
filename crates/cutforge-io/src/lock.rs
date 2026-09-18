// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 工程锁(计划书 4.2 步骤 1 / 4.3):`.cutforge/lock`,含 pid + 时间戳;
//! 超过 stale_after 秒视为死锁残留可接管;Drop 自动释放。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct LockGuard {
    path: PathBuf,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// 获取工程写锁;被占用时最多重试 `retries` 次(每次间隔 50ms),
/// 持有者时间戳超过 `stale_after_ms` 视为崩溃残留,接管。
pub fn acquire(root: &Path, stale_after_ms: u64, retries: u32) -> io::Result<LockGuard> {
    let dir = root.join(".cutforge");
    let path = dir.join("lock");
    let mut attempt = 0u32;
    loop {
        fs::create_dir_all(&dir)?;
        let content = format!("pid={} ts={}", std::process::id(), now_ms());
        match crate::atomic::create_exclusive(&path, content.as_bytes()) {
            Ok(()) => return Ok(LockGuard { path }),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                // 死锁残留(时间戳超过阈值)→ 接管,不计入重试次数
                if let Ok(meta) = fs::read(&path)
                    && let Ok(text) = String::from_utf8(meta)
                        && let Some(ts) = text.split(" ts=").nth(1).and_then(|s| s.trim().parse::<u64>().ok())
                            && now_ms().saturating_sub(ts) > stale_after_ms {
                                let _ = crate::atomic::remove(&path);
                                continue;
                            }
                if attempt == retries {
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists, "工程被其他进程锁定"));
                }
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = crate::atomic::remove(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil;

    #[test]
    fn lock_exclusive_and_stale_takeover() {
        let dir = fsutil::temp_dir("cutforge-lock");
        let _g = acquire(&dir, 60_000, 1).expect("首次获取");
        assert!(acquire(&dir, 60_000, 0).is_err(), "二次获取必须失败");
        drop(_g);
        let _g2 = acquire(&dir, 60_000, 1).expect("释放后可再获取");
        drop(_g2);
        // 残留死锁(时间戳造旧)→ 接管
        let lock = dir.join(".cutforge").join("lock");
        crate::atomic::atomic_write(&lock, format!("pid=1 ts={}", now_ms() - 120_000).as_bytes()).unwrap();
        let _g3 = acquire(&dir, 60_000, 0).expect("过期锁应被接管");
        drop(_g3);
        fsutil::cleanup(&dir);
    }
}
