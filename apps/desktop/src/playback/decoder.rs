//! 视频解码流(I1 M1,工单 docs/tickets/I1-S1):
//! `ffmpeg -ss <in> -i <src> -t <dur> -f rawvideo -pix_fmt rgba pipe:1`
//! 管道 → 读线程整帧校验 → 24 帧环形缓冲(满则**背压**,不丢帧)。
//! 大档源经 `-vf scale` 缩到预览档(短边 ≤540,与 zone 半分辨率口径一致),
//! 帧内存 8.3MB(1080p)→ 2.0MB,消除长跑堆分配压力;精确画面走 render_frame。
//!
//! 纪律:
//! - ffmpeg/ffprobe 路径由调用方注入,模块**不读环境变量**(可测性);
//! - 3s 无字节 → 停流看门狗:杀进程、置故障态(见 [`VideoStream::fault`]);
//!   环满背压等待属消费端节流,看门狗不得误判为停流;
//! - Drop 必须 kill + wait,并唤醒背压等待线程,不留僵尸/挂起线程;
//! - 帧宽高/行字节数严格校验(按 `w*h*4` 整帧填充读取),坏帧(截断)丢弃计数。

use super::PlaybackError;
use super::ring::Ring;
use super::subprocess::{self, lock_or_recover};
use std::io::{BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// 环形缓冲容量:24 帧(工单纪律)
const RING_FRAMES: usize = 24;

/// 流故障分类(引擎映射为 [`PlaybackError`])
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StreamFault {
    /// 3s 无字节,看门狗已杀进程
    Stalled,
    /// 读线程/进程异常
    Failed(String),
    /// 进程正常退出但一帧未出(坏文件/坏编码),附 stderr 尾部
    NoOutput(String),
}

/// 跨线程共享流状态(读线程写,UI 泵/看门狗读)
struct StreamState {
    frames: Ring<Arc<Vec<u8>>>,
    /// stdout 干净 EOF(帧边界对齐)
    eof: bool,
    /// 读线程失败
    failed: Option<String>,
    /// 看门狗判定停流(已杀进程)
    stalled: bool,
    /// 环满背压等待中(读线程阻塞等消费者腾位;看门狗不得计为停流)
    backpressured: bool,
    /// 最近一次字节进展时刻(None = 从未出字节)
    last_byte: Option<Instant>,
    /// 累计产出帧数
    produced: u64,
    /// 坏帧(截断)丢弃计数
    bad_frames: u64,
    /// stderr 尾部(进程退出后供 NoOutput 归因)
    stderr_tail: Option<String>,
}

impl StreamState {
    fn new() -> Self {
        Self {
            frames: Ring::new(RING_FRAMES),
            eof: false,
            failed: None,
            stalled: false,
            backpressured: false,
            last_byte: None,
            produced: 0,
            bad_frames: 0,
            stderr_tail: None,
        }
    }
}

/// 一次 ffmpeg rawvideo 解码会话。持有子进程与三个后台线程
/// (读帧/stderr 排流/看门狗);Drop 杀进程收尸,线程随管道关闭自行退出。
pub(crate) struct VideoStream {
    shared: Arc<Mutex<StreamState>>,
    /// 环满背压唤醒(读线程在此等待消费者腾位)
    cond: Arc<Condvar>,
    stopped: Arc<AtomicBool>,
    /// Option 便于 take 语义:看门狗与 Drop 竞争时只杀一次
    child: Arc<Mutex<Option<Child>>>,
    /// rawvideo 管道输入时持有 stdin 保活(停流测试用)
    stdin: Option<ChildStdin>,
    /// 实际输出帧宽(源尺寸或 preview 缩放后尺寸;引擎据此建纹理)
    width: u32,
    /// 实际输出帧高
    height: u32,
}

/// 预览解码档:输出短边上限。16:9 1080p → 960×540(帧内存 8.3MB → 2.0MB,
/// 与 zone 半分辨率口径一致;预览画质,精确画面走 render_frame)。
const PREVIEW_SHORT_EDGE: u32 = 540;

/// 预览输出尺寸:源短边等比缩到 [`PREVIEW_SHORT_EDGE`](已在档内的源不缩),
/// 宽高偶数对齐(scale 滤镜对 yuv 源的硬性要求)。
fn preview_dims(width: u32, height: u32) -> (u32, u32) {
    let short = width.min(height).max(1);
    let s = f64::from(PREVIEW_SHORT_EDGE) / f64::from(short);
    if s >= 1.0 {
        return (width, height); // 小档源原样输出(不缩,尺寸不加滤镜)
    }
    // 偶数化:round 后清末位;竖屏源等比(1080×1920 → 540×960)
    let even = |v: f64| ((v.round().max(2.0) as u64) & !1) as u32;
    (even(f64::from(width) * s), even(f64::from(height) * s))
}

impl VideoStream {
    /// 起标准视频解码流(`-ss` 输入 seeking,精确到帧;大档源自动缩到
    /// preview 档)。`width`/`height` 为**源**尺寸,实际输出以
    /// [`Self::width`]/[`Self::height`] 为准。
    pub(crate) fn start(
        ffmpeg: &Path,
        src: &Path,
        start_s: f64,
        dur_s: f64,
        width: u32,
        height: u32,
    ) -> Result<Self, PlaybackError> {
        let start_s = start_s.max(0.0);
        let dur_s = dur_s.max(0.001);
        let (out_w, out_h) = preview_dims(width, height);
        let mut args: Vec<String> = [
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-ss",
            &format!("{start_s:.3}"),
            "-i",
            &src.display().to_string(),
            "-t",
            &format!("{dur_s:.3}"),
            "-an",
            "-sn",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if (out_w, out_h) != (width, height) {
            // 预览缩放:帧内存 4 倍削减,消除长跑堆分配压力
            args.push("-vf".to_string());
            args.push(format!("scale={out_w}:{out_h}"));
        }
        args.extend(
            ["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"]
                .iter()
                .map(|s| s.to_string()),
        );
        Self::spawn_raw(ffmpeg, &args, out_w, out_h, Stdio::null())
    }

    /// 实际输出帧宽(源尺寸或 preview 缩放后尺寸)
    pub(crate) fn width(&self) -> u32 {
        self.width
    }

    /// 实际输出帧高
    pub(crate) fn height(&self) -> u32 {
        self.height
    }

    /// 低层 spawn(停流看门狗测试用:可自定义参数并保住 stdin);
    /// `width`/`height` 为**实际输出**尺寸(帧字节校验与镜像都按它来)
    pub(crate) fn spawn_raw(
        ffmpeg: &Path,
        args: &[String],
        width: u32,
        height: u32,
        stdin: Stdio,
    ) -> Result<Self, PlaybackError> {
        let frame_bytes = checked_frame_bytes(width, height)?;
        let mut child = subprocess::command(ffmpeg)
            .args(args)
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                PlaybackError::Spawn(format!("ffmpeg 启动失败({}): {e}", ffmpeg.display()))
            })?;
        let stdout = child.stdout.take().expect("stdout 已设 piped");
        let stderr = child.stderr.take().expect("stderr 已设 piped");
        let child_stdin = child.stdin.take(); // Stdio::null() 时为 None

        let shared = Arc::new(Mutex::new(StreamState::new()));
        let cond = Arc::new(Condvar::new());
        let stopped = Arc::new(AtomicBool::new(false));
        let child_cell = Arc::new(Mutex::new(Some(child)));

        // 读帧线程:按 frame_bytes 整帧填充;截断=坏帧计数;EOF/错误置态。
        // 环满时背压等待消费者腾位(不再丢最旧),解码速度被拉平到消费速度
        {
            let shared = Arc::clone(&shared);
            let cond = Arc::clone(&cond);
            let stopped = Arc::clone(&stopped);
            std::thread::Builder::new()
                .name("cf-decoder".into())
                .spawn(move || {
                    let mut reader = BufReader::with_capacity(1 << 16, stdout);
                    let mut buf: Vec<u8> = vec![0; frame_bytes];
                    let mut filled = 0usize;
                    loop {
                        if stopped.load(Ordering::Relaxed) {
                            break;
                        }
                        match reader.read(&mut buf[filled..]) {
                            Ok(0) => {
                                let mut st = lock_or_recover(&shared);
                                if filled > 0 {
                                    st.bad_frames += 1; // 帧尾截断 → 坏帧丢弃
                                }
                                st.eof = true;
                                break;
                            }
                            Ok(n) => {
                                filled += n;
                                {
                                    let mut st = lock_or_recover(&shared);
                                    st.last_byte = Some(Instant::now());
                                }
                                if filled == frame_bytes {
                                    // 背压:环满则等待消费者 pop 腾位;停机则退出
                                    if !wait_ring_space(&shared, &cond, &stopped) {
                                        break;
                                    }
                                    let mut st = lock_or_recover(&shared);
                                    st.frames
                                        .push(Arc::new(std::mem::take(&mut buf)))
                                        .expect("背压等待后必有空位");
                                    st.produced += 1;
                                    buf = vec![0; frame_bytes];
                                    filled = 0;
                                }
                            }
                            Err(e) => {
                                let mut st = lock_or_recover(&shared);
                                if st.failed.is_none() {
                                    st.failed = Some(format!("读帧失败:{e}"));
                                }
                                break;
                            }
                        }
                    }
                })
                .ok();
        }

        // stderr 排流线程:防管道缓冲打满导致 ffmpeg 卡死;留尾部归因
        {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("cf-dec-stderr".into())
                .spawn(move || {
                    let mut s = String::new();
                    let _ = BufReader::new(stderr).read_to_string(&mut s);
                    if !s.is_empty() {
                        let tail: String = s.chars().rev().take(400).collect::<String>();
                        let mut st = lock_or_recover(&shared);
                        st.stderr_tail = Some(tail.chars().rev().collect());
                    }
                })
                .ok();
        }

        // 停流看门狗:3s 无字节 → 置 stalled 并杀进程。
        // 环满背压等待不算停流(字节停止是消费端节流的正常后果)
        subprocess::spawn_watchdog(
            Arc::clone(&stopped),
            Arc::clone(&child_cell),
            Arc::clone(&shared),
            |st, started| {
                if st.eof || st.failed.is_some() || st.stalled || st.backpressured {
                    return None; // 已收敛,或背压等待中,看门狗收工
                }
                Some(match st.last_byte {
                    Some(t) => t.elapsed(),
                    None => started.elapsed(),
                })
            },
            |st| st.stalled = true,
        );

        Ok(Self {
            shared,
            cond,
            stopped,
            child: child_cell,
            stdin: child_stdin,
            width,
            height,
        })
    }

    /// 测试用:取出 stdin 句柄保活(保持管道不关闭)
    #[cfg(test)]
    pub(crate) fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.stdin.take()
    }

    /// UI 泵取帧(FIFO);腾出空位后唤醒背压等待的生产者
    pub(crate) fn pop_frame(&self) -> Option<Arc<Vec<u8>>> {
        let frame = lock_or_recover(&self.shared).frames.pop();
        if frame.is_some() {
            self.cond.notify_one();
        }
        frame
    }

    /// 流故障(非阻塞检查);None = 健康
    pub(crate) fn fault(&self) -> Option<StreamFault> {
        let st = lock_or_recover(&self.shared);
        if st.stalled {
            return Some(StreamFault::Stalled);
        }
        if let Some(m) = &st.failed {
            return Some(StreamFault::Failed(m.clone()));
        }
        if st.eof && st.produced == 0 {
            let why = st
                .stderr_tail
                .clone()
                .unwrap_or_else(|| "ffmpeg 无输出帧".to_string());
            return Some(StreamFault::NoOutput(why));
        }
        None
    }

    /// ffmpeg 已 EOF 且缓冲排空(自然播完)
    #[allow(dead_code)] // 诊断面,当前未被门面消费(引擎经 fault/poll 感知播完)
    pub(crate) fn finished(&self) -> bool {
        let st = lock_or_recover(&self.shared);
        st.eof && st.frames.is_empty()
    }

    /// 看门狗/正常退出后子进程确已收尸(测试断言用)
    #[cfg(test)]
    pub(crate) fn child_exited(&self) -> bool {
        let mut guard = lock_or_recover(&self.child);
        match guard.as_mut() {
            Some(c) => c.try_wait().map(|o| o.is_some()).unwrap_or(true),
            None => true,
        }
    }
}

/// 背压等待:环满则阻塞在 cond 上,直到消费者 pop 腾位或停机。
/// 返回 false = 停机(shutdown/seek/load 换流),调用方应立即退出读线程。
/// 背压等待期间置 `backpressured`,看门狗不得将"字节停"误判为停流。
fn wait_ring_space(shared: &Mutex<StreamState>, cond: &Condvar, stopped: &AtomicBool) -> bool {
    let mut st = lock_or_recover(shared);
    while st.frames.is_full() {
        if stopped.load(Ordering::Relaxed) {
            return false;
        }
        st.backpressured = true;
        st = cond
            .wait(st)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    st.backpressured = false;
    true
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        // 先唤醒可能阻塞在环满背压上的读线程,再杀进程收尸,不留等待线程
        self.stopped.store(true, Ordering::Relaxed);
        self.cond.notify_all();
        self.stdin.take(); // 关 stdin → ffmpeg 自然退出(双保险)
        subprocess::kill_and_wait(&self.child);
    }
}

/// 帧尺寸严格校验:行字节 = w*4,整帧 = w*h*4(溢出即拒绝)
fn checked_frame_bytes(width: u32, height: u32) -> Result<usize, PlaybackError> {
    if width == 0 || height == 0 {
        return Err(PlaybackError::Spawn(format!("帧尺寸非法:{width}x{height}")));
    }
    (width as usize)
        .checked_mul(4)
        .and_then(|stride| stride.checked_mul(height as usize))
        .ok_or_else(|| PlaybackError::Spawn(format!("帧字节数溢出:{width}x{height} rgba")))
}

#[cfg(test)]
mod tests;
