// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 工程锁(计划书 4.2 步骤 1 / 4.3):`.cutforge/lock`,含 pid + 创建时间 + 启动指纹;
//! 接管判定(R-02 语义精化,按 pid 确定性分级):
//! - **pid 确证不在**(进程不存在,或存在但启动指纹 ≠ 锁内指纹 = pid 被复用)
//!   → 持有者必已亡,**无视锁龄/心跳立即接管**——锁龄门的本意(防活进程被
//!   误判死)已由启动时间指纹承担,陈旧锁龄不再承载信息量(强杀/退出进程的
//!   新鲜锁必须立即可接管,否则 serve 类常驻形态降级只读);
//! - **pid 活**(含指纹缺失无法证复用)→ 永不接管——活进程的兜底保护
//!   (心跳 mtime 每 5s 刷新)维持不变;
//! - **探测失败**(工具不可用)→ 活性无法判定,回退保守门:锁龄 ≥ 阈值
//!   且心跳过期(原 R-02 三重链,只服务这一退化情形)。
//!
//! Drop 自动释放 + 停心跳(写门闩防幽灵锁复活)。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 心跳窗口(R-02):锁文件 mtime 在窗口内 = 持锁方心跳正常,不可接管。
pub const HEARTBEAT_WINDOW_MS: u64 = 10_000;
/// 心跳更新间隔:持锁方每 5s 更新一次锁文件 mtime。
pub const HEARTBEAT_INTERVAL_MS: u64 = 5_000;

pub struct LockGuard {
    path: PathBuf,
    /// 心跳线程控制面(Drop 同步停写,见 [`Heartbeat`])。
    heartbeat: Heartbeat,
}

/// 心跳线程控制面:stop 旗标 + 写门闩。
/// Drop 次序 = 置 stop → 抢门闩(等在飞的最后一次写完成)→ 删锁文件。
/// 门闩保证 Drop 返回后心跳线程**绝不可能再写**(否则会复活刚删除的锁文件,
/// 形成幽灵锁,让同进程/他进程的后续 acquire 误判"被锁定")。
struct Heartbeat {
    stop: Arc<AtomicBool>,
    gate: Arc<std::sync::Mutex<()>>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 锁文件元数据(R-02 接管判定的输入;解析失败的字段为 None)。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LockMeta {
    pub pid: Option<u32>,
    /// 创建时间(锁文件内容 `ts=`,毫秒;接管判定的"锁龄"基准)。
    pub ts_ms: Option<u64>,
    /// 创建进程的启动时间指纹(Windows pid 复用联合判定;空 = 旧版锁文件)。
    pub boot: Option<String>,
    /// 锁文件 mtime(心跳基准;持锁方每 5s 更新)。
    pub mtime_ms: Option<u64>,
}

impl LockMeta {
    /// 读取并解析锁文件(不存在/不可读 → Err)。
    pub fn read(path: &Path) -> io::Result<LockMeta> {
        let text = String::from_utf8_lossy(&fs::read(path)?).into_owned();
        let mut meta = LockMeta {
            mtime_ms: None,
            ..Default::default()
        };
        for tok in text.split_whitespace() {
            if let Some(v) = tok.strip_prefix("pid=") {
                meta.pid = v.parse::<u32>().ok();
            } else if let Some(v) = tok.strip_prefix("ts=") {
                meta.ts_ms = v.parse::<u64>().ok();
            } else if let Some(v) = tok.strip_prefix("boot=") {
                meta.boot = Some(v.to_string());
            }
        }
        meta.mtime_ms = fs::metadata(path).ok().and_then(|m| {
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
        });
        Ok(meta)
    }
}

/// pid 探测注入面(三态,见 [`crate::probe::PidState`])。
/// 生产路径 = [`crate::probe::probe_pid`];测试注入假探针
/// (pid 复用/探测失败等真实环境难造的场景)。
pub type PidProbe<'a> = &'a dyn Fn(u32) -> crate::probe::PidState;

/// 接管判定(R-02 语义精化,按 pid 确定性分级;详见模块头):
/// - 探测 = [`PidState::Dead`](进程不存在)→ 立即接管;
/// - 探测 = 活但启动指纹 ≠ 锁内指纹(pid 复用,原持有者必已亡)→ 立即接管;
/// - 探测 = 活(含指纹缺失无法证复用)→ 永不接管;
/// - 探测 = [`PidState::Unknown`](探测失败)→ 回退保守门:锁龄 ≥ 阈值
///   且心跳过期。
///
/// 无 pid 可探测(锁文件无 pid 字段)→ 宁等勿夺,不接管。
pub fn can_takeover(
    meta: &LockMeta,
    now_ms: u64,
    stale_after_ms: u64,
    heartbeat_window_ms: u64,
    probe: PidProbe,
) -> bool {
    let Some(pid) = meta.pid else {
        return false;
    };
    let boot_recorded = meta.boot.as_deref().unwrap_or("");
    match probe(pid) {
        crate::probe::PidState::Dead => true,
        crate::probe::PidState::Alive(Some(start)) => {
            // pid 复用:活进程的启动指纹与锁创建者不符 → 原持有者必已亡
            !boot_recorded.is_empty() && start != boot_recorded
        }
        crate::probe::PidState::Alive(None) => false,
        crate::probe::PidState::Unknown => {
            let aged = meta
                .ts_ms
                .is_some_and(|ts| now_ms.saturating_sub(ts) >= stale_after_ms);
            let hb_stale = meta
                .mtime_ms
                .is_none_or(|m| now_ms.saturating_sub(m) > heartbeat_window_ms);
            aged && hb_stale
        }
    }
}

fn real_probe(pid: u32) -> crate::probe::PidState {
    crate::probe::probe_pid(pid)
}

/// 锁文件内容(pid + 创建时间 + 本进程启动时间指纹)。
fn lock_content() -> String {
    format!(
        "pid={} boot={} ts={}",
        std::process::id(),
        crate::probe::pid_start_time(std::process::id()).unwrap_or_default(),
        now_ms()
    )
}

/// 心跳线程:每 5s 原子重写锁文件内容(内容不变,mtime 前移)。
/// 写操作全程持门闩,且进门前/门内双重检查 stop——与 [`Drop for LockGuard`]
/// 的"置 stop → 抢门闩 → 删文件"构成同步:Drop 返回后不可能再有心跳写。
fn spawn_heartbeat(
    path: PathBuf,
    content: String,
    gate: Arc<std::sync::Mutex<()>>,
) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(HEARTBEAT_INTERVAL_MS));
            if stop2.load(Ordering::SeqCst) {
                break;
            }
            let _permit = gate.lock().unwrap();
            if stop2.load(Ordering::SeqCst) {
                break; // 门内复查:Drop 可能正等着删文件
            }
            let _ = crate::atomic::atomic_write(&path, content.as_bytes());
        }
    });
    stop
}

/// 获取工程写锁;被占用时最多重试 `retries` 次(每次间隔 50ms);
/// 持有者满足 [`can_takeover`] 分级判定(pid 确证不在 → 立即接管,活 → 不夺,
/// 探测失败 → 锁龄+心跳保守门)才接管,接管不计入重试次数。
pub fn acquire(root: &Path, stale_after_ms: u64, retries: u32) -> io::Result<LockGuard> {
    let dir = root.join(".cutforge");
    let path = dir.join("lock");
    let mut attempt = 0u32;
    loop {
        fs::create_dir_all(&dir)?;
        let content = lock_content();
        match crate::atomic::create_exclusive(&path, content.as_bytes()) {
            Ok(()) => {
                let gate = Arc::new(std::sync::Mutex::new(()));
                let stop = spawn_heartbeat(path.clone(), content, gate.clone());
                return Ok(LockGuard {
                    path,
                    heartbeat: Heartbeat { stop, gate },
                });
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                // R-02 分级接管判定(见 can_takeover):pid 确证不在 → 立即夺
                if let Ok(meta) = LockMeta::read(&path)
                    && can_takeover(
                        &meta,
                        now_ms(),
                        stale_after_ms,
                        HEARTBEAT_WINDOW_MS,
                        &real_probe,
                    )
                {
                    let _ = crate::atomic::remove(&path);
                    continue;
                }
                if attempt == retries {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "工程被其他进程锁定",
                    ));
                }
                attempt += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // 次序纪律:置 stop → 抢门闩(同步在飞的最后一次心跳写)→ 删锁文件。
        self.heartbeat.stop.store(true, Ordering::SeqCst);
        let _permit = self.heartbeat.gate.lock().unwrap();
        let _ = crate::atomic::remove(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil;

    fn meta(pid: u32, age_ms: u64, mtime_age_ms: u64, boot: &str) -> LockMeta {
        LockMeta {
            pid: Some(pid),
            ts_ms: Some(now_ms() - age_ms),
            boot: Some(boot.to_string()),
            mtime_ms: Some(now_ms() - mtime_age_ms),
        }
    }

    /// 注入探针:恒返回给定三态(指纹场景用 Alive(Some(指纹)))。
    fn probe_const(s: crate::probe::PidState) -> impl Fn(u32) -> crate::probe::PidState {
        move |_| s.clone()
    }

    use crate::probe::PidState;

    #[test]
    fn takeover_semantics_by_pid_certainty() {
        const S: u64 = 30_000;
        const HB: u64 = 10_000;
        let now = now_ms();
        // ---- pid 确证不在 → 立即接管,锁龄/心跳不再承载信息量 ----
        // 新鲜锁 + pid 死(强杀/退出进程的现场,e2e M10 形态)→ 立即接管
        assert!(can_takeover(
            &meta(4_194_303, 0, 0, "T1"),
            now,
            S,
            HB,
            &probe_const(PidState::Dead)
        ));
        // 陈旧锁 + pid 死 + 心跳过期 → 照常接管
        assert!(can_takeover(
            &meta(4_194_303, 60_000, 60_000, "T1"),
            now,
            S,
            HB,
            &probe_const(PidState::Dead)
        ));
        // pid 复用:活进程启动指纹 ≠ 锁内指纹 → 原持有者必已亡,立即接管
        assert!(can_takeover(
            &meta(100, 0, 0, "T1"),
            now,
            S,
            HB,
            &probe_const(PidState::Alive(Some("T2".into())))
        ));
        // ---- pid 活 → 永不接管(心跳是其兜底保护,维持) ----
        // 活进程持锁 60s(TC-IO-LOCK-001 机制:慢盘/负载下长持锁不误夺)
        assert!(!can_takeover(
            &meta(100, 60_000, 60_000, "T1"),
            now,
            S,
            HB,
            &probe_const(PidState::Alive(Some("T1".into())))
        ));
        // 心跳新鲜(TC-IO-LOCK-003 机制)
        assert!(!can_takeover(
            &meta(100, 60_000, 1_000, "T1"),
            now,
            S,
            HB,
            &probe_const(PidState::Alive(Some("T1".into())))
        ));
        // 锁龄不足
        assert!(!can_takeover(
            &meta(100, 1_000, 1_000, "T1"),
            now,
            S,
            HB,
            &probe_const(PidState::Alive(Some("T1".into())))
        ));
        // 旧版锁文件(无 boot 指纹):活进程无法证复用 → 保守不接管
        let mut old = meta(100, 60_000, 60_000, "");
        old.boot = None;
        assert!(!can_takeover(
            &old,
            now,
            S,
            HB,
            &probe_const(PidState::Alive(Some("T1".into())))
        ));
        // 探测取不到指纹 → 同样无法证复用 → 不接管
        assert!(!can_takeover(
            &meta(100, 60_000, 60_000, "T1"),
            now,
            S,
            HB,
            &probe_const(PidState::Alive(None))
        ));
        // ---- 探测失败 → 回退保守门(锁龄 + 心跳,原三重链只服务此退化情形) ----
        assert!(!can_takeover(
            &meta(4_194_303, 1_000, 9_999, "T1"),
            now,
            S,
            HB,
            &probe_const(PidState::Unknown)
        ));
        assert!(can_takeover(
            &meta(4_194_303, 60_000, 60_000, "T1"),
            now,
            S,
            HB,
            &probe_const(PidState::Unknown)
        ));
        assert!(!can_takeover(
            &meta(4_194_303, 60_000, 1_000, "T1"),
            now,
            S,
            HB,
            &probe_const(PidState::Unknown)
        ));
        // ---- 无 pid 可探测(锁文件缺 pid 字段)→ 宁等勿夺 ----
        let junk = LockMeta {
            pid: None,
            ts_ms: None,
            boot: None,
            mtime_ms: None,
        };
        assert!(!can_takeover(
            &junk,
            now,
            S,
            HB,
            &probe_const(PidState::Dead)
        ));
    }

    #[test]
    fn lock_exclusive_and_stale_takeover() {
        let dir = fsutil::temp_dir("cutforge-lock");
        let _g = acquire(&dir, 60_000, 1).expect("首次获取");
        assert!(acquire(&dir, 60_000, 0).is_err(), "二次获取必须失败");
        // 心跳在动:持锁期间锁文件 mtime 被 5s 心跳线程维护(acquire 后立即写即已新鲜)
        let lock_path = dir.join(".cutforge/lock");
        let m = LockMeta::read(&lock_path).expect("锁文件可解析");
        assert_eq!(m.pid, Some(std::process::id()));
        assert!(m.boot.is_some(), "锁文件必须带启动时间指纹: {m:?}");
        drop(_g);
        let _g2 = acquire(&dir, 60_000, 1).expect("释放后可再获取");
        drop(_g2);
        // 残留死锁(pid 死 + 锁龄造旧 + mtime 拨旧 = 心跳过期)→ 接管。
        // 死 pid 必须两平台都"不存在":pid=1 在 Linux 是 init(恒活),不得用。
        let dead = crate::probe::definitely_dead_pid();
        let stale = now_ms() - 120_000;
        crate::atomic::atomic_write(
            &lock_path,
            format!("pid={dead} boot= ts={stale}").as_bytes(),
        )
        .unwrap();
        set_mtime_old(&lock_path, 120_000);
        assert!(
            can_takeover_of_file(&lock_path),
            "测试前提:伪造现场必须满足接管链(pid={dead} 探测为死 + 锁龄超 + 心跳过期)"
        );
        let _g3 = acquire(&dir, 60_000, 0).expect("过期锁应被接管");
        drop(_g3);
        fsutil::cleanup(&dir);
    }

    /// 测试观察面:对盘上锁文件跑一遍真实接管判定(真实 pid 三态探测)。
    fn can_takeover_of_file(lock_path: &Path) -> bool {
        LockMeta::read(lock_path)
            .map(|meta| {
                can_takeover(&meta, now_ms(), 30_000, HEARTBEAT_WINDOW_MS, &|pid: u32| {
                    crate::probe::probe_pid(pid)
                })
            })
            .unwrap_or(false)
    }

    fn set_mtime_old(p: &Path, ms_ago: u64) {
        let f = fs::OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(
            SystemTime::now()
                .checked_sub(Duration::from_millis(ms_ago))
                .unwrap(),
        )
        .unwrap();
    }

    /// R-02 心跳活性:持锁超过一个心跳周期后,锁文件 mtime 必被刷新
    /// (接近当下,而非创建时刻)。轮询等待(慢 CI 上心跳线程可能被饿死数秒,
    /// 固定 sleep 断言会 flake):最多等 2 个心跳周期。
    #[test]
    fn heartbeat_refreshes_lock_mtime() {
        let dir = fsutil::temp_dir("cutforge-lock-hb");
        let g = acquire(&dir, 60_000, 0).expect("获取");
        let lock_path = dir.join(".cutforge/lock");
        let created = LockMeta::read(&lock_path).unwrap().mtime_ms.unwrap();
        let deadline =
            std::time::Instant::now() + Duration::from_millis(HEARTBEAT_INTERVAL_MS * 2 + 1_000);
        let mut fresh = created;
        while std::time::Instant::now() < deadline {
            fresh = LockMeta::read(&lock_path).unwrap().mtime_ms.unwrap();
            if fresh > created {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(
            fresh > created && now_ms().saturating_sub(fresh) < HEARTBEAT_WINDOW_MS,
            "心跳必须刷新 mtime: created={created} fresh={fresh} now={}",
            now_ms()
        );
        drop(g);
        fsutil::cleanup(&dir);
    }
}
