// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 崩溃恢复(册六 T6.1/AC-6.1):强杀进程 → 重启 → 恢复清单 → 清锁 + OpLog 一致性校验。
//!
//! 恢复判据(启动检测):`.cutforge/lock` 在盘且——
//! ① pid 不存活(强杀),或 ② 锁龄超过 stale 阈值(与 `lock::acquire` 接管阈值同源,
//! 活进程长持锁的场景由 acquire 的重试/接管语义处理,不算恢复对象);
//! 附加证据:`.cutforge/session-summary.json`(RT-1 会话摘要)在盘说明存在未收尾会话,
//! 恢复清单里如实带出 revFrom/revTo。
//!
//! 执行恢复 = 清残留锁 → `Workspace::open_for_write`(OpLog 半行截断恢复 + rev 对账
//! + v1 盘面规范形落盘,全部复用既有装载语义,不另造第二套一致性逻辑)→ 报告 rev/Op 数。

use crate::paths;
use crate::probe::pid_alive;
use serde_json::Value;
use std::io;
use std::path::{Path, PathBuf};

/// 锁龄 stale 阈值(与 `lock::acquire` 的缺省接管阈值一致)。
pub const STALE_LOCK_MS: u128 = 30_000;

/// 一个可恢复(残留锁)工程的证据清单。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StaleLock {
    pub root: PathBuf,
    /// 锁文件记录的 pid。
    pub pid: Option<u32>,
    /// 锁龄(毫秒)。
    pub age_ms: u128,
    /// pid 是否仍存活(false = 强杀残留)。
    pub pid_alive: bool,
    /// 会话摘要证据(RT-1):(revFrom, revTo, userOpCount)。
    pub session: Option<(u64, u64, u64)>,
}

fn lock_ts_ms(text: &str) -> Option<u128> {
    text.split_whitespace()
        .find_map(|t| t.strip_prefix("ts=").and_then(|v| v.parse::<u128>().ok()))
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// 单工程恢复判据:锁在盘 && (pid 已死 || 锁龄超阈值)。
pub fn detect_stale(root: &Path) -> Option<StaleLock> {
    let lock = root.join(".cutforge/lock");
    let text = std::fs::read_to_string(&lock).ok()?;
    let pid = text.split_whitespace().find_map(|t| t.strip_prefix("pid=").and_then(|v| v.parse::<u32>().ok()));
    let age_ms = lock_ts_ms(&text).map(|ts| now_ms().saturating_sub(ts)).unwrap_or(u128::MAX);
    let alive = pid.map(pid_alive).unwrap_or(true);
    if alive && age_ms < STALE_LOCK_MS {
        return None; // 活进程且未超时:正常持有,不是恢复对象
    }
    let session = std::fs::read_to_string(root.join(".cutforge/session-summary.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(|v| {
            (
                v["revFrom"].as_u64().unwrap_or(0),
                v["revTo"].as_u64().unwrap_or(0),
                v["userOpCount"].as_u64().unwrap_or(0),
            )
        });
    Some(StaleLock { root: root.to_path_buf(), pid, age_ms, pid_alive: alive, session })
}

/// 扫描一个库根(顶层一层)下的可恢复工程(库根缺省面见 `library::library_root`)。
pub fn scan_stale(library: &Path) -> Vec<StaleLock> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(library) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && paths::has_project(&p)
            && let Some(stale) = detect_stale(&p) {
                out.push(stale);
            }
    }
    out.sort();
    out
}

/// 恢复报告(OpLog 一致性校验结果)。
#[derive(Debug, Clone, PartialEq)]
pub struct RecoverReport {
    pub root: PathBuf,
    /// 清掉的残留锁(无锁 = 只做一致性校验)。
    pub lock_cleared: bool,
    /// 恢复后 rev(oplog 与 rev 文件对账后的值)。
    pub rev: u64,
    /// OpLog 记账数。
    pub op_count: usize,
    /// 会话摘要证据(若有)。
    pub session: Option<(u64, u64, u64)>,
}

/// 执行恢复:清残留锁 + 打开工程做 OpLog 一致性校验(半行截断/rev 对账/规范形落盘
/// 全部由 `Workspace::open_for_write` 既有语义承接)。
pub fn recover(root: &Path) -> io::Result<RecoverReport> {
    let stale = detect_stale(root);
    let lock = root.join(".cutforge/lock");
    let lock_cleared = if stale.is_some() && lock.is_file() {
        crate::atomic::remove(&lock)?;
        true
    } else {
        false
    };
    let ws = crate::Workspace::open_for_write(root)?;
    let session = stale.and_then(|s| s.session);
    Ok(RecoverReport {
        root: root.to_path_buf(),
        lock_cleared,
        rev: ws.rev(),
        op_count: ws.engine().oplog().ops().len(),
        session,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LayoutKind;
    use crate::{fsutil, Workspace};

    fn make_project(dir: &Path) {
        crate::scaffold::scaffold_project_layout(dir, "恢复体", 30, 1080, 1920,
            &[cutforge_core::model::TrackKind::Video], LayoutKind::V2).unwrap();
    }

    fn forge_stale_lock(root: &Path) {
        fsutil::ensure(&root.join(".cutforge")).unwrap();
        // 假 pid:Windows PID 恒为 4 的倍数,4194303 必不存活
        crate::library::write_atomic(&root.join(".cutforge/lock"), b"pid=4194303 ts=1").unwrap();
    }

    #[test]
    fn stale_detection_and_recover_clears_lock() {
        let lib = fsutil::temp_dir("recover-scan");
        let p = lib.join("crashed");
        make_project(&p);
        forge_stale_lock(&p);
        // 扫描命中且证据齐全
        let stale = scan_stale(&lib);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].pid, Some(4194303));
        assert!(!stale[0].pid_alive, "假 pid 必判死");
        assert!(stale[0].age_ms > STALE_LOCK_MS, "ts=1 必然超龄");
        // 活进程持锁(本进程 pid,锁龄 0)不算恢复对象
        let q = lib.join("alive");
        make_project(&q);
        let _g = crate::lock::acquire(&q, 60_000, 0).unwrap();
        assert!(detect_stale(&q).is_none(), "活进程新锁不是恢复对象");
        drop(_g);
        assert!(detect_stale(&q).is_none(), "锁已释放更不是");
        // 执行恢复:清锁 + OpLog 一致性校验(rev 归 0,零 Op)
        let report = recover(&p).unwrap();
        assert!(report.lock_cleared);
        assert_eq!(report.rev, 0);
        assert_eq!(report.op_count, 0);
        assert!(!p.join(".cutforge/lock").exists(), "残留锁必须被清");
        // 恢复后工程可正常独占打开并写入(零数据丢失的下一步)
        let mut ws = Workspace::open_exclusive(&p).unwrap();
        ws.apply(cutforge_core::command::Command::TrackAdd {
            kind: cutforge_core::model::TrackKind::Text, request_id: None,
        }, cutforge_core::oplog::Actor::user("恢复"), Default::default()).unwrap();
        assert_eq!(ws.rev(), 1);
        fsutil::cleanup(&lib);
    }

    #[test]
    fn recover_after_write_preserves_oplog_integrity() {
        let lib = fsutil::temp_dir("recover-oplog");
        let p = lib.join("worked");
        make_project(&p);
        // 先真实写两笔(Op 记账),再强造崩溃残留锁
        let mut ws = Workspace::open_exclusive(&p).unwrap();
        for _ in 0..2 {
            ws.apply(cutforge_core::command::Command::TrackAdd {
                kind: cutforge_core::model::TrackKind::Audio, request_id: None,
            }, cutforge_core::oplog::Actor::agent("pre-crash"), Default::default()).unwrap();
        }
        drop(ws);
        forge_stale_lock(&p);
        let report = recover(&p).unwrap();
        assert!(report.lock_cleared);
        assert_eq!(report.rev, 2, "oplog 对账后的 rev");
        assert_eq!(report.op_count, 2, "OpLog 完整性:强杀前两笔 Op 全在");
        // 再次打开可继续撤销(undo 栈由 oplog 重建)
        let mut ws2 = Workspace::open_exclusive(&p).unwrap();
        ws2.undo(cutforge_core::oplog::Actor::agent("post-recover")).unwrap();
        // undo 本身也是一笔 Op(rev 前进);断言**状态**回归:最后一轨被撤销
        assert_eq!(ws2.project().tracks.iter().filter(|t| t.kind == cutforge_core::model::TrackKind::Audio).count(), 1);
        fsutil::cleanup(&lib);
    }
}
