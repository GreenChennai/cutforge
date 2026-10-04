//! 音频输出流(I1 M1,工单 docs/tickets/I1-S1):
//! `ffmpeg -f f32le -ar 48000 -ac 2 pipe:1` → 环形缓冲 → cpal 默认输出设备
//! (f32 立体声)。
//!
//! 纪律:
//! - 欠载写静音继续;预缓冲 100ms 起播(docs/upstream/05 §I1 风险表口径);
//! - 环满**背压**(读线程等待声卡回调腾位),不丢样本——音频头部永不丢,
//!   生产速度被拉平到 1x 实时,音画零漂移;看门狗不得把背压等待计为停流;
//! - 设备打开失败 → [`PlaybackError::AudioUnavailable`](引擎据此回落墙钟);
//! - cpal 相关类型全部封装在本模块内,对外只暴露控制面;
//! - 子进程 Drop kill + wait 并唤醒背压等待线程,不留僵尸/挂起线程;
//!   静音 = 丢弃式消费(解除静音不回跳)。

use super::PlaybackError;
use super::ring::Ring;
use super::subprocess::{self, lock_or_recover};
use callback::{ensure_pair_space, fill_output, push_pair};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::io::{BufReader, Read};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

mod callback;

/// 固定音频面:48kHz 立体声 f32(工单口径;设备实际协商失败即不可用)
pub(crate) const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: usize = 2;
/// 预缓冲 100ms 起播(帧数)
const PRIME_FRAMES: u32 = SAMPLE_RATE / 10;
/// 环形缓冲 400ms(帧数)—— 环满时生产端背压等待,不丢样本
const RING_CAP_FRAMES: usize = SAMPLE_RATE as usize * 2 / 5;

/// 跨线程共享状态(读线程写,cpal 回调消费)
struct AudioShared {
    ring: Ring<f32>, // 交错样本;push/pop 均按帧成对,长度恒为偶数
    /// ffmpeg stdout 干净 EOF
    finished: bool,
    /// 读线程失败 / cpal 错误回调
    failed: Option<String>,
    /// 看门狗判定停流(已杀进程)
    stalled: bool,
    /// 环满背压等待中(读线程阻塞等回调腾位;看门狗不得计为停流)
    backpressured: bool,
    /// 欠载次数(诊断)
    underruns: u64,
    /// 最近一次字节进展(None = 从未出字节)
    last_byte: Option<Instant>,
}

impl AudioShared {
    fn new() -> Self {
        Self {
            ring: Ring::new(RING_CAP_FRAMES * CHANNELS),
            finished: false,
            failed: None,
            stalled: false,
            backpressured: false,
            underruns: 0,
            last_byte: None,
        }
    }
}

/// 一次 ffmpeg f32le 解码 + cpal 输出会话。
/// Drop:停 cpal 流、杀 ffmpeg 并 wait,不留僵尸。
pub(crate) struct AudioStream {
    shared: Arc<Mutex<AudioShared>>,
    /// 环满背压唤醒(读线程在此等待声卡回调腾位)
    cond: Arc<Condvar>,
    /// 已消费的每声道样本帧数(48000 = 1s;主时钟数据源)
    consumed: Arc<AtomicU64>,
    muted: Arc<AtomicBool>,
    /// 预缓冲达标前只写静音不消费(Arc 本体交给音频回调闭包)
    #[allow(dead_code)] // 诊断面,当前未被门面消费(Arc 已移交回调闭包)
    primed: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    /// cpal 流句柄;Drop 先 take(析构即停回调、还设备)
    stream: Mutex<Option<cpal::Stream>>,
}

impl AudioStream {
    /// 起「ffmpeg f32le → 环形缓冲 → cpal 默认设备」会话。
    /// 设备/流打开失败即返回 [`PlaybackError::AudioUnavailable`]。
    /// 构建顺序:先开设备再起子进程/线程 —— 任一步失败已建资源均可安全 Drop。
    pub(crate) fn start(
        ffmpeg: &Path,
        src: &Path,
        start_s: f64,
        dur_s: f64,
    ) -> Result<Self, PlaybackError> {
        let start_s = start_s.max(0.0);
        let dur_s = dur_s.max(0.001);
        let args: Vec<String> = [
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
            "-vn",
            "-f",
            "f32le",
            "-ar",
            "48000",
            "-ac",
            "2",
            "pipe:1",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        // 共享面先行(回调闭包需要 Arc)
        let shared = Arc::new(Mutex::new(AudioShared::new()));
        let cond = Arc::new(Condvar::new());
        let consumed = Arc::new(AtomicU64::new(0));
        let muted = Arc::new(AtomicBool::new(false));
        let primed = Arc::new(AtomicBool::new(false));

        // 1) cpal 输出流(f32、2ch、默认设备;打开失败 → AudioUnavailable,
        //    此时尚无子进程/线程,天然无泄漏)
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| PlaybackError::AudioUnavailable("无默认音频输出设备".to_string()))?;
        let config = cpal::StreamConfig {
            channels: CHANNELS as u16,
            sample_rate: SAMPLE_RATE, // cpal 0.18:SampleRate 是 u32 别名
            buffer_size: cpal::BufferSize::Default,
        };
        let err_shared = Arc::clone(&shared);
        let stream = device
            .build_output_stream::<f32, _, _>(
                config,
                {
                    let shared = Arc::clone(&shared);
                    let cond = Arc::clone(&cond);
                    let consumed = Arc::clone(&consumed);
                    let muted = Arc::clone(&muted);
                    let primed = Arc::clone(&primed);
                    move |data, _| {
                        fill_output(data, &shared, &cond, &consumed, &muted, &primed);
                    }
                },
                move |err| {
                    let mut st = lock_or_recover(&err_shared);
                    if st.failed.is_none() {
                        st.failed = Some(format!("cpal 输出流错误:{err}"));
                    }
                },
                None,
            )
            .map_err(|e| PlaybackError::AudioUnavailable(format!("打开输出流失败:{e}")))?;
        stream
            .play()
            .map_err(|e| PlaybackError::AudioUnavailable(format!("启动输出流失败:{e}")))?;

        // 2) ffmpeg 子进程(spawn 失败 → stream 局部变量 Drop 停回调)
        let mut child = subprocess::command(ffmpeg)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                PlaybackError::Spawn(format!("ffmpeg 启动失败({}): {e}", ffmpeg.display()))
            })?;
        let stdout = child.stdout.take().expect("stdout 已设 piped");
        let stderr = child.stderr.take().expect("stderr 已设 piped");
        let stopped = Arc::new(AtomicBool::new(false));
        let child_cell = Arc::new(Mutex::new(Some(child)));

        // 读样本线程:字节流按 8 字节(f32 立体声帧)切帧,余量留待下轮拼接。
        // 环满时背压等待声卡回调腾位(不再丢样本),生产速度被拉平到 1x 实时
        {
            let shared = Arc::clone(&shared);
            let cond = Arc::clone(&cond);
            let stopped = Arc::clone(&stopped);
            std::thread::Builder::new()
                .name("cf-audio-rd".into())
                .spawn(move || {
                    let mut reader = BufReader::with_capacity(1 << 16, stdout);
                    let mut buf = [0u8; 8192];
                    let mut pending: Vec<u8> = Vec::with_capacity(8);
                    loop {
                        if stopped.load(Ordering::Relaxed) {
                            break;
                        }
                        match reader.read(&mut buf) {
                            Ok(0) => {
                                lock_or_recover(&shared).finished = true;
                                break;
                            }
                            Ok(n) => {
                                {
                                    let mut st = lock_or_recover(&shared);
                                    st.last_byte = Some(Instant::now());
                                }
                                let mut data = &buf[..n];
                                // 补齐上一轮残帧(读管道可能按任意字节切)
                                if !pending.is_empty() {
                                    let need = 8 - pending.len();
                                    let take = need.min(data.len());
                                    pending.extend_from_slice(&data[..take]);
                                    data = &data[take..];
                                    if pending.len() == 8 {
                                        if !ensure_pair_space(&shared, &cond, &stopped) {
                                            break;
                                        }
                                        let mut st = lock_or_recover(&shared);
                                        push_pair(&pending, &mut st);
                                        pending.clear();
                                    }
                                }
                                let (chunks, remainder) = data.as_chunks::<8>();
                                for c in chunks {
                                    if !ensure_pair_space(&shared, &cond, &stopped) {
                                        break; // 停机;外层 loop 顶部随 stopped 退出
                                    }
                                    let mut st = lock_or_recover(&shared);
                                    push_pair(c, &mut st);
                                }
                                pending.extend_from_slice(remainder);
                            }
                            Err(e) => {
                                let mut st = lock_or_recover(&shared);
                                if st.failed.is_none() {
                                    st.failed = Some(format!("读音频失败:{e}"));
                                }
                                break;
                            }
                        }
                    }
                })
                .ok();
        }

        // stderr 排流:防管道缓冲打满卡死 ffmpeg(内容不用于归因,音频失败
        // 一律降级墙钟,不致命)
        {
            std::thread::Builder::new()
                .name("cf-audio-err".into())
                .spawn(move || {
                    let mut sink = [0u8; 4096];
                    let mut r = BufReader::new(stderr);
                    while let Ok(n) = r.read(&mut sink) {
                        if n == 0 {
                            break;
                        }
                    }
                })
                .ok();
        }

        // 停流看门狗:与视频同款 3s 纪律(音频停流同样会冻住主时钟)。
        // 环满背压等待不算停流(字节停止是消费端 1x 节流的正常后果)
        subprocess::spawn_watchdog(
            Arc::clone(&stopped),
            Arc::clone(&child_cell),
            Arc::clone(&shared),
            |st, started| {
                if st.finished || st.failed.is_some() || st.stalled || st.backpressured {
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
            consumed,
            muted,
            primed,
            stopped,
            child: child_cell,
            stream: Mutex::new(Some(stream)),
        })
    }

    pub(crate) fn play(&self) {
        // 尽力而为;失败经 err 回调进 failed → 引擎降级
        if let Ok(guard) = self.stream.lock()
            && let Some(s) = guard.as_ref()
        {
            let _ = s.play();
        }
    }

    pub(crate) fn pause(&self) {
        if let Ok(guard) = self.stream.lock()
            && let Some(s) = guard.as_ref()
        {
            let _ = s.pause();
        }
    }

    pub(crate) fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    /// 已消费的每声道样本帧数(48000 = 1s;音频主时钟数据源;静音照常推进)
    pub(crate) fn consumed_samples(&self) -> u64 {
        self.consumed.load(Ordering::Relaxed)
    }

    /// 流故障(读失败/停流/cpal 错误);引擎据此弃音频回落墙钟
    pub(crate) fn fault(&self) -> Option<String> {
        let st = lock_or_recover(&self.shared);
        if let Some(m) = &st.failed {
            return Some(m.clone());
        }
        if st.stalled {
            return Some("音频流停流(3s 无字节)".to_string());
        }
        None
    }

    /// ffmpeg 已 EOF 且缓冲排空(自然播完;主时钟切换归引擎)
    pub(crate) fn finished(&self) -> bool {
        let st = lock_or_recover(&self.shared);
        st.finished && st.ring.is_empty()
    }

    /// 欠载计数(诊断面,当前未被门面消费)
    #[allow(dead_code)]
    pub(crate) fn underruns(&self) -> u64 {
        lock_or_recover(&self.shared).underruns
    }
}

impl Drop for AudioStream {
    fn drop(&mut self) {
        // 先唤醒可能阻塞在环满背压上的读线程,再停流杀进程,不留等待线程
        self.stopped.store(true, Ordering::Relaxed);
        self.cond.notify_all();
        if let Ok(mut guard) = self.stream.lock() {
            guard.take(); // 析构即停回调并释放设备
        }
        subprocess::kill_and_wait(&self.child);
    }
}

#[cfg(test)]
mod tests;
