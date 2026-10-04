// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 工程锁(计划书 4.2 步骤 1 / 4.3):`.cutforge/lock`,含 pid + 创建时间 + 启动指纹;
//! 接管前置链(R-02,按序全过才接管):① 锁龄 ≥ stale 阈值 → ② 持锁方 pid
//! 不存活(Windows 用 pid+启动时间联合判定防 pid 复用误判)→ ③ 心跳不新鲜
//! (持锁方每 5s 更新锁文件 mtime;心跳窗口 10s)。活进程慢盘/杀毒扫描/负载
//! 尖峰下长持锁**不再被误接管**(双写者风险的根治)。Drop 自动释放 + 停心跳。

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

/// pid 探测注入面:返回 (pid 是否存活, 该进程启动时间指纹)。
/// 生产路径 = [`crate::probe::pid_alive`] + [`crate::probe::pid_start_time`];
/// 测试注入假探针(pid 复用等真实环境造不出的场景)。
pub type PidProbe<'a> = &'a dyn Fn(u32) -> (bool, Option<String>);

/// 接管前置链(R-02,按序全过才返回 true):
/// ① 锁龄 ≥ stale_after_ms;② 持锁方**原进程**已不在
/// (pid 死,或 pid 活但启动时间指纹与记录不一致 = pid 被复用,原持锁方已亡);
/// ③ 心跳过期(mtime 距今 > heartbeat_window_ms)。
/// ts 不可解析(外来/损坏锁文件)→ 一律不接管(宁等勿夺)。
pub fn can_takeover(
    meta: &LockMeta,
    now_ms: u64,
    stale_after_ms: u64,
    heartbeat_window_ms: u64,
    probe: PidProbe,
) -> bool {
    let Some(ts) = meta.ts_ms else { return false };
    if now_ms.saturating_sub(ts) < stale_after_ms {
        return false;
    }
    let Some(pid) = meta.pid else { return false };
    let (alive, start) = probe(pid);
    // pid 活但启动时间对不上 → 是复用 pid 的无关进程,原持锁方已死
    let boot_recorded = meta.boot.as_deref().unwrap_or("");
    let holder_alive =
        alive && (boot_recorded.is_empty() || start.as_deref().is_none_or(|s| s == boot_recorded));
    if holder_alive {
        return false;
    }
    match meta.mtime_ms {
        Some(m) if now_ms.saturating_sub(m) <= heartbeat_window_ms => false, // 心跳新鲜
        _ => true,
    }
}

fn real_probe(pid: u32) -> (bool, Option<String>) {
    (
        crate::probe::pid_alive(pid),
        crate::probe::pid_start_time(pid),
    )
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
/// 持有者满足 [`can_takeover`] 前置链(锁龄+pid 活性+心跳)才接管,
/// 接管不计入重试次数。
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
                // R-02 接管前置链:锁龄 + pid 活性(pid+启动时间联合)+ 心跳,全过才夺
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

    fn probe_of(alive: bool, boot: &str) -> impl Fn(u32) -> (bool, Option<String>) + '_ {
        move |_| (alive, Some(boot.to_string()))
    }

    #[test]
    fn takeover_chain_requires_all_three_signals() {
        const S: u64 = 30_000;
        const HB: u64 = 10_000;
        let now = now_ms();
        // ① 锁龄不足 → 不接管(哪怕 pid 已死、心跳过期)
        assert!(!can_takeover(
            &meta(4_194_303, 1_000, 9_999, "T1"),
            now,
            S,
            HB,
            &probe_of(false, "")
        ));
        // ② pid 活且启动指纹一致 → 不接管(哪怕锁龄超、心跳过期——TC-IO-LOCK-001 机制)
        assert!(!can_takeover(
            &meta(100, 60_000, 60_000, "T1"),
            now,
            S,
            HB,
            &probe_of(true, "T1")
        ));
        // ③ 心跳新鲜 → 不接管(哪怕锁龄超、pid 存活证据一致——TC-IO-LOCK-003 机制)
        assert!(!can_takeover(
            &meta(100, 60_000, 1_000, "T1"),
            now,
            S,
            HB,
            &probe_of(true, "T1")
        ));
        // 全过:锁龄超 + pid 死 + 心跳过期 → 接管(TC-IO-LOCK-002 机制)
        assert!(can_takeover(
            &meta(4_194_303, 60_000, 60_000, "T1"),
            now,
            S,
            HB,
            &probe_of(false, "")
        ));
        // pid 复用:pid 活但启动指纹与记录不一致 → 原持锁方已亡,可接管
        assert!(can_takeover(
            &meta(100, 60_000, 60_000, "T1"),
            now,
            S,
            HB,
            &probe_of(true, "T2")
        ));
        // 旧版锁文件(无 boot)退化为仅 pid 判定
        let mut old = meta(4_194_303, 60_000, 60_000, "");
        old.boot = None;
        assert!(can_takeover(&old, now, S, HB, &probe_of(false, "")));
        // ts 不可解析 → 不接管
        let junk = LockMeta {
            pid: None,
            ts_ms: None,
            boot: None,
            mtime_ms: None,
        };
        assert!(!can_takeover(&junk, now, S, HB, &probe_of(false, "")));
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
        // 残留死锁(pid 死 + 锁龄造旧 + mtime 拨旧 = 心跳过期)→ 接管
        let stale = now_ms() - 120_000;
        crate::atomic::atomic_write(&lock_path, format!("pid=1 boot= ts={stale}").as_bytes())
            .unwrap();
        set_mtime_old(&lock_path, 120_000);
        let _g3 = acquire(&dir, 60_000, 0).expect("过期锁应被接管");
        drop(_g3);
        fsutil::cleanup(&dir);
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

    /// R-02 心跳活性:持锁 ≥6s(超过一个心跳周期)后,锁文件 mtime 必被刷新
    /// (接近当下,而非创建时刻)。
    #[test]
    fn heartbeat_refreshes_lock_mtime() {
        let dir = fsutil::temp_dir("cutforge-lock-hb");
        let g = acquire(&dir, 60_000, 0).expect("获取");
        let lock_path = dir.join(".cutforge/lock");
        let created = LockMeta::read(&lock_path).unwrap().mtime_ms.unwrap();
        std::thread::sleep(Duration::from_millis(HEARTBEAT_INTERVAL_MS + 1_500));
        let fresh = LockMeta::read(&lock_path).unwrap().mtime_ms.unwrap();
        assert!(
            fresh > created && now_ms().saturating_sub(fresh) < HEARTBEAT_WINDOW_MS,
            "心跳必须刷新 mtime: created={created} fresh={fresh} now={}",
            now_ms()
        );
        drop(g);
        fsutil::cleanup(&dir);
    }
}
