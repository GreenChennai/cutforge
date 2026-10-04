// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 崩溃恢复(册六 T6.1/AC-6.1):强杀进程 → 重启 → 恢复清单 → 清锁 + OpLog 一致性校验。
//!
//! 恢复判据(启动检测,R-02 同源):`.cutforge/lock` 在盘且满足与
//! `lock::acquire` 完全相同的接管前置链——锁龄 ≥ stale 阈值 && 原持锁 pid
//! 不存活(Windows pid+启动时间联合判定)&& 心跳过期(持锁方 5s 更新 mtime)。
//! 活进程长持锁(慢盘/杀毒扫描)不是恢复对象,acquire 也不会误接管——两处策略单源。
//! 附加证据:`.cutforge/session-summary.json`(RT-1 会话摘要)在盘说明存在未收尾会话,
//! 恢复清单里如实带出 revFrom/revTo。
//!
//! 执行恢复 = 清残留锁 → `Workspace::open_for_write`(OpLog 半行截断恢复 + rev 对账
//! + v1 盘面规范形落盘,全部复用既有装载语义,不另造第二套一致性逻辑)→ 报告 rev/Op 数。

use crate::lock::{self, LockMeta};
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

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// 单工程恢复判据(R-02 同源化):锁在盘 && 与 `lock::can_takeover` **同一
/// 前置链**(锁龄 ≥ stale && 原持锁进程已不在(pid+启动时间联合判定)&&
/// 心跳过期)。不再出现"acquire 不接管而 recover 清活进程锁"的策略分叉。
pub fn detect_stale(root: &Path) -> Option<StaleLock> {
    let lock_path = root.join(".cutforge/lock");
    let meta = LockMeta::read(&lock_path).ok()?;
    let takeover = lock::can_takeover(
        &meta,
        now_ms() as u64,
        STALE_LOCK_MS as u64,
        lock::HEARTBEAT_WINDOW_MS,
        &|pid| (pid_alive(pid), crate::probe::pid_start_time(pid)),
    );
    // 证据归集:即便不满足接管链(活进程长持锁),锁龄仍如实带出供诊断
    let age_ms = meta
        .ts_ms
        .map(|ts| now_ms().saturating_sub(ts as u128))
        .unwrap_or(u128::MAX);
    if !takeover {
        return None; // 活进程持有(或心跳未过期):正常持有,不是恢复对象
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
    Some(StaleLock {
        root: root.to_path_buf(),
        pid: meta.pid,
        age_ms,
        pid_alive: meta.pid.map(pid_alive).unwrap_or(true),
        session,
    })
}

/// 扫描一个库根(顶层一层)下的可恢复工程(库根缺省面见 `library::library_root`)。
pub fn scan_stale(library: &Path) -> Vec<StaleLock> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(library) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir()
            && paths::has_project(&p)
            && let Some(stale) = detect_stale(&p)
        {
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
    use crate::{Workspace, fsutil};

    fn make_project(dir: &Path) {
        crate::scaffold::scaffold_project_layout(
            dir,
            "恢复体",
            30,
            1080,
            1920,
            &[cutforge_core::model::TrackKind::Video],
            LayoutKind::V2,
        )
        .unwrap();
    }

    fn forge_stale_lock(root: &Path) -> u32 {
        fsutil::ensure(&root.join(".cutforge")).unwrap();
        // 假 pid:用跨平台"必死"助手(Linux 的 pid=1 是 init 恒活,高段 pid 也
        // 可能被占,须探测;Windows 取 pid 空间外的奇数 4194303);
        // mtime 拨旧(心跳过期,与锁龄超阈值同证)。
        let dead = crate::probe::definitely_dead_pid();
        let lock = root.join(".cutforge/lock");
        crate::library::write_atomic(&lock, format!("pid={dead} boot= ts=1").as_bytes()).unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&lock).unwrap();
        f.set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2))
            .unwrap();
        dead
    }

    #[test]
    fn stale_detection_and_recover_clears_lock() {
        let lib = fsutil::temp_dir("recover-scan");
        let p = lib.join("crashed");
        make_project(&p);
        let dead = forge_stale_lock(&p);
        // 扫描命中且证据齐全
        let stale = scan_stale(&lib);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].pid, Some(dead));
        assert!(!stale[0].pid_alive, "假 pid 必判死");
        assert!(stale[0].age_ms > STALE_LOCK_MS, "ts=1 必然超龄");
        // 活进程持锁(本进程 pid,锁龄 0)不算恢复对象
        let q = lib.join("alive");
        make_project(&q);
        let _g = crate::lock::acquire(&q, 60_000, 0).unwrap();
        assert!(detect_stale(&q).is_none(), "活进程新锁不是恢复对象");
        drop(_g);
        assert!(detect_stale(&q).is_none(), "锁已释放更不是");
        // 陈旧但持锁进程活着(锁龄超 + 心跳过期 + pid 活)→ 不是恢复对象。
        // 两平台等价口径:不再依赖"某硬编码 pid 恰好死"(Linux pid=1 恒活,
        // 曾致 conformance 用例在 CI 翻红)。伪造"陈旧现场"必须配必死 pid。
        let r2 = lib.join("aged-alive");
        make_project(&r2);
        let alive_lock = r2.join(".cutforge/lock");
        crate::library::write_atomic(
            &alive_lock,
            format!(
                "pid={} boot= ts={}",
                std::process::id(),
                now_ms().saturating_sub(120_000) as u64
            )
            .as_bytes(),
        )
        .unwrap();
        {
            let f = std::fs::OpenOptions::new()
                .write(true)
                .open(&alive_lock)
                .unwrap();
            f.set_modified(
                std::time::SystemTime::now()
                    .checked_sub(std::time::Duration::from_millis(120_000))
                    .unwrap(),
            )
            .unwrap();
        }
        assert!(detect_stale(&r2).is_none(), "陈旧但 pid 活 → 不是恢复对象");
        // 执行恢复:清锁 + OpLog 一致性校验(rev 归 0,零 Op)
        let report = recover(&p).unwrap();
        assert!(report.lock_cleared);
        assert_eq!(report.rev, 0);
        assert_eq!(report.op_count, 0);
        assert!(!p.join(".cutforge/lock").exists(), "残留锁必须被清");
        // 恢复后工程可正常独占打开并写入(零数据丢失的下一步)
        let mut ws = Workspace::open_exclusive(&p).unwrap();
        ws.apply(
            cutforge_core::command::Command::TrackAdd {
                kind: cutforge_core::model::TrackKind::Text,
                request_id: None,
            },
            cutforge_core::oplog::Actor::user("恢复"),
            Default::default(),
        )
        .unwrap();
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
            ws.apply(
                cutforge_core::command::Command::TrackAdd {
                    kind: cutforge_core::model::TrackKind::Audio,
                    request_id: None,
                },
                cutforge_core::oplog::Actor::agent("pre-crash"),
                Default::default(),
            )
            .unwrap();
        }
        drop(ws);
        forge_stale_lock(&p);
        let report = recover(&p).unwrap();
        assert!(report.lock_cleared);
        assert_eq!(report.rev, 2, "oplog 对账后的 rev");
        assert_eq!(report.op_count, 2, "OpLog 完整性:强杀前两笔 Op 全在");
        // 再次打开可继续撤销(undo 栈由 oplog 重建)
        let mut ws2 = Workspace::open_exclusive(&p).unwrap();
        ws2.undo(cutforge_core::oplog::Actor::agent("post-recover"))
            .unwrap();
        // undo 本身也是一笔 Op(rev 前进);断言**状态**回归:最后一轨被撤销
        assert_eq!(
            ws2.project()
                .tracks
                .iter()
                .filter(|t| t.kind == cutforge_core::model::TrackKind::Audio)
                .count(),
            1
        );
        fsutil::cleanup(&lib);
    }
}
