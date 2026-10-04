//! 子进程公共设施(I1 播放引擎):统一 spawn 形态、kill+wait 收尸、停流看门狗。
//!
//! decoder(视频)与 audio(音频)读线程共用;纪律:
//! - Windows 下 spawn 带 CREATE_NO_WINDOW,GUI 壳不弹控制台;
//! - Drop/看门狗路径一律 kill 后 wait,不留僵尸;
//! - 看门狗 3s 无进展 → 置故障态并杀进程(工单停流纪律)。

use std::path::Path;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 停流判定阈值:3s 无字节(工单纪律)
pub(crate) const STALL_TIMEOUT: Duration = Duration::from_secs(3);
/// 看门狗轮询间隔
pub(crate) const WATCHDOG_POLL: Duration = Duration::from_millis(250);

/// 统一 spawn 形态:Windows 下加 CREATE_NO_WINDOW
pub(crate) fn command(program: &Path) -> Command {
    // mut 仅 Windows 分支使用(CREATE_NO_WINDOW);非 Windows 下允许 unused_mut
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// 锁不传播毒化:拿数据继续干(收尸路径绝不因 poison 跳过)
pub(crate) fn lock_or_recover<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// kill 并 wait 收尸(take 语义:看门狗与 Drop 竞争时只有一方真正杀)
pub(crate) fn kill_and_wait(child: &Mutex<Option<Child>>) {
    let mut guard = lock_or_recover(child);
    if let Some(mut c) = guard.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// 停流看门狗线程:每 [`WATCHDOG_POLL`] 轮询一次,`idle_and_alive` 返回
/// `Some(idle)` 且 idle ≥ [`STALL_TIMEOUT`] → 调 `mark_stalled` 置故障态并
/// 杀进程收尸。`stopped` 置位或流已收敛(eof/failed)时线程退出。
///
/// `idle_and_alive` 约定:流已收敛(正常结束/已失败/已停流)返回 None;
/// 否则返回"距最近一次字节进展的时长"(从未出字节则用起播时长)。
pub(crate) fn spawn_watchdog<S, Idle, Mark>(
    stopped: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    state: Arc<Mutex<S>>,
    idle_and_alive: Idle,
    mark_stalled: Mark,
) where
    S: Send + 'static,
    Idle: Fn(&mut S, std::time::Instant) -> Option<Duration> + Send + 'static,
    Mark: Fn(&mut S) + Send + 'static,
{
    let spawned = std::thread::Builder::new()
        .name("cf-watchdog".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            while !stopped.load(Ordering::Relaxed) {
                std::thread::sleep(WATCHDOG_POLL);
                if stopped.load(Ordering::Relaxed) {
                    return;
                }
                let stalled = {
                    let mut s = lock_or_recover(&state);
                    idle_and_alive(&mut s, started).is_some_and(|idle| idle >= STALL_TIMEOUT)
                };
                if stalled {
                    {
                        let mut s = lock_or_recover(&state);
                        mark_stalled(&mut s);
                    }
                    kill_and_wait(&child);
                    return;
                }
            }
        });
    // 线程起不来只能放弃看门狗(正常环境不会发生);不 panic 是引擎纪律
    if let Err(e) = spawned {
        eprintln!("cf-playback: 看门狗线程启动失败:{e}");
    }
}
