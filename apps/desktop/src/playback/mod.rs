//! PlaybackEngine 播放门面(I1 M1,工单 docs/tickets/I1-S1)。
//!
//! 线程/子进程全部封装在引擎内,对外 API 全部非阻塞:UI 泵每帧调
//! [`PlaybackEngine::poll_frame`] 消费 RGBA,[`PlaybackEngine::position_ms`]
//! 驱动播放头;音频回调为主时钟,不可用/变速时自动回落墙钟。
//!
//! 状态机:`Idle` →(load_clip 成功)`Running` ⇄ play/pause →(停流/解码失败)
//! `Faulted`。Faulted 后除 `load_clip` 重试与 `shutdown` 外全部 no-op,
//! **引擎绝不 panic**;调用方查询 [`PlaybackEngine::last_error`] 自行降级
//! (回落幻灯片模式是接线方职责)。
//!
//! 接线约定:ffmpeg/ffprobe 路径由调用方注入(从 env `CUTFORGE_FFMPEG` /
//! `CUTFORGE_FFPROBE` 取,缺省 `"ffmpeg"`);src 一律传工程内绝对路径。

pub mod zone;

pub(crate) mod audio;
pub(crate) mod clock;
pub(crate) mod decoder;
pub(crate) mod ring;
pub(crate) mod subprocess;
pub(crate) mod tools;

use decoder::StreamFault;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 播放故障(可展示、可判定降级路径)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackError {
    /// ffmpeg/ffprobe 不可执行(路径无效/缺失)
    ToolMissing(String),
    /// 媒体探测失败(无视频流/超时/坏容器)
    Probe(String),
    /// 子进程启动失败
    Spawn(String),
    /// 3s 无字节停流(看门狗已杀进程)——典型回落幻灯片信号
    Stalled,
    /// 解码失败(进程早退/无输出帧),附 stderr 归因
    Decode(String),
    /// 音频设备不可用(引擎已自动回落墙钟,视频照常)
    AudioUnavailable(String),
}

impl std::fmt::Display for PlaybackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ToolMissing(m) => write!(f, "工具不可用:{m}"),
            Self::Probe(m) => write!(f, "媒体探测失败:{m}"),
            Self::Spawn(m) => write!(f, "子进程启动失败:{m}"),
            Self::Stalled => write!(f, "解码停流(3s 无输出,已杀进程)"),
            Self::Decode(m) => write!(f, "解码失败:{m}"),
            Self::AudioUnavailable(m) => write!(f, "音频设备不可用:{m}"),
        }
    }
}

impl std::error::Error for PlaybackError {}

/// 解码出的一帧(UI 泵消费;rgba 为 RGBA8888,行主序,len == w*h*4)
#[derive(Clone)]
pub struct DecodedFrame {
    pub rgba: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    /// 本流内呈现时间戳(ms,按 fps 均匀推导)
    pub pts_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EngineState {
    Idle,
    Running,
    Faulted(PlaybackError),
}

/// 单 clip 直解码播放引擎(M1)。非线程安全:约定在 gpui UI 线程独占使用。
pub struct PlaybackEngine {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    video: Option<decoder::VideoStream>,
    audio: Option<audio::AudioStream>,
    clock: clock::PlaybackClock,
    state: EngineState,
    // 当前片段参数(seek 重启流与 pts 推导需要)
    src: Option<PathBuf>,
    stream_start_ms: f64,
    end_ms: f64,
    fps: f64,
    width: u32,
    height: u32,
    frames_emitted: u64,
    muted: bool,
}

impl PlaybackEngine {
    /// 构造引擎:验证 ffmpeg/ffprobe 可执行(各 3s 上限)。
    /// 失败返回 [`PlaybackError::ToolMissing`],调用方应保持幻灯片模式。
    pub fn new(ffmpeg: PathBuf, ffprobe: PathBuf) -> Result<Self, PlaybackError> {
        tools::verify_tool(&ffprobe, "ffprobe")?;
        tools::verify_tool(&ffmpeg, "ffmpeg")?;
        Ok(Self {
            ffmpeg,
            ffprobe,
            video: None,
            audio: None,
            clock: clock::PlaybackClock::new(),
            state: EngineState::Idle,
            src: None,
            stream_start_ms: 0.0,
            end_ms: 0.0,
            fps: 30.0,
            width: 0,
            height: 0,
            frames_emitted: 0,
            muted: false,
        })
    }

    /// 加载片段并起新流(自动停旧流)。成功后处于 pause 态,由调用方 play()。
    /// 失败进入 Faulted(last_error 可查),旧流已停。
    pub fn load_clip(
        &mut self,
        src: &Path,
        in_ms: f64,
        out_ms: f64,
        fps: f64,
    ) -> Result<(), PlaybackError> {
        let result = self.try_load(src, in_ms, out_ms, fps);
        match result {
            Ok(()) => {
                self.state = EngineState::Running;
                Ok(())
            }
            Err(e) => {
                self.enter_faulted(e.clone());
                Err(e)
            }
        }
    }

    fn try_load(
        &mut self,
        src: &Path,
        in_ms: f64,
        out_ms: f64,
        fps: f64,
    ) -> Result<(), PlaybackError> {
        if !fps.is_finite() || fps <= 0.0 {
            return Err(PlaybackError::Probe(format!("fps 非法:{fps}")));
        }
        if !in_ms.is_finite() || !out_ms.is_finite() || out_ms <= in_ms {
            return Err(PlaybackError::Probe(format!(
                "片段区间非法:[{in_ms}, {out_ms}]ms"
            )));
        }
        // 预探在停旧流之前:探测失败不打断旧流
        let info = tools::probe_media(&self.ffprobe, src)?;
        let dur_s = (out_ms - in_ms) / 1000.0;
        let start_s = (in_ms / 1000.0).max(0.0);
        self.stop_streams(); // 停旧流起新流
        let vs = decoder::VideoStream::start(
            &self.ffmpeg,
            src,
            start_s,
            dur_s,
            info.width,
            info.height,
        )?;
        // 自持尺寸 = **实际输出尺寸**(大档源已被缩到 preview 档);
        // DecodedFrame.width/height 必须反映真实帧尺寸(接线方按它建纹理)
        let (out_w, out_h) = (vs.width(), vs.height());
        self.video = Some(vs);
        // 音频可选:设备打开失败不致命(回落墙钟),静默降级
        self.audio = if info.has_audio {
            audio::AudioStream::start(&self.ffmpeg, src, start_s, dur_s).ok()
        } else {
            None
        };
        self.apply_muted();
        self.src = Some(src.to_path_buf());
        self.stream_start_ms = in_ms;
        self.end_ms = out_ms;
        self.fps = fps;
        self.width = out_w;
        self.height = out_h;
        self.frames_emitted = 0;
        self.clock.seek(in_ms);
        Ok(())
    }

    pub fn play(&mut self) {
        if matches!(self.state, EngineState::Running) {
            self.clock.play();
            if let Some(a) = &self.audio {
                a.play();
            }
        }
    }

    pub fn pause(&mut self) {
        if matches!(self.state, EngineState::Running) {
            self.clock.pause();
            if let Some(a) = &self.audio {
                a.pause();
            }
        }
    }

    pub fn is_playing(&self) -> bool {
        matches!(self.state, EngineState::Running) && self.clock.is_playing()
    }

    /// 变速(0.25~4.0 越界夹取)。speed != 1.0 时音频不挂载(音频不跟速),
    /// 引擎自动回落墙钟;回到 1.0 且音频健康时下个泵周期重新挂载。
    pub fn set_speed(&mut self, speed: f32) {
        self.clock.set_speed(speed);
    }

    /// 静音。静音为「丢弃式消费」:主时钟照走,解除静音不回跳。
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.apply_muted();
    }

    /// seek:重启音视频流(约百 ms 级开销,非阻塞返回)。Idle/Faulted 下 no-op。
    pub fn seek_ms(&mut self, t_ms: f64) {
        if !matches!(self.state, EngineState::Running) {
            return;
        }
        let Some(src) = self.src.clone() else {
            return;
        };
        if !t_ms.is_finite() {
            return;
        }
        let t = t_ms.clamp(0.0, self.end_ms);
        let start_s = (t / 1000.0).max(0.0);
        let dur_s = ((self.end_ms - t) / 1000.0).max(0.001);
        let result = self.restart_streams(&src, start_s, dur_s);
        match result {
            Ok(()) => {
                self.stream_start_ms = t;
                self.frames_emitted = 0;
                self.clock.seek(t);
            }
            Err(e) => self.enter_faulted(e),
        }
    }

    fn restart_streams(
        &mut self,
        src: &Path,
        start_s: f64,
        dur_s: f64,
    ) -> Result<(), PlaybackError> {
        self.stop_streams();
        let vs = decoder::VideoStream::start(
            &self.ffmpeg,
            src,
            start_s,
            dur_s,
            self.width,
            self.height,
        )?;
        self.video = Some(vs);
        self.audio = audio::AudioStream::start(&self.ffmpeg, src, start_s, dur_s).ok();
        self.apply_muted();
        Ok(())
    }

    /// 当前播放位置(ms)。UI 泵每帧调用;内部完成健康检查与时钟喂入。
    pub fn position_ms(&mut self) -> f64 {
        self.check_health();
        let running = matches!(self.state, EngineState::Running);
        // 音频主时钟仅在:音频流健康、未播完、speed == 1.0 时挂载
        let feed = if running {
            self.audio
                .as_ref()
                .filter(|a| !a.finished())
                .map(audio::AudioStream::consumed_samples)
                .filter(|_| (self.clock.speed() - 1.0).abs() < 1e-6)
        } else {
            None
        };
        self.clock.tick(feed, audio::SAMPLE_RATE);
        let pos = self.clock.position_ms();
        if running && self.end_ms > 0.0 && pos >= self.end_ms {
            // 播到片段尾自动停(循环播放由调用方 seek 后 play)
            self.pause();
        }
        pos
    }

    /// UI 泵取帧(每帧调用一次;FIFO;环满时读线程背压等待本函数腾位,
    /// 泵停帧不丢头,解码速度被自动拉平到消费速度)
    pub fn poll_frame(&mut self) -> Option<DecodedFrame> {
        self.check_health();
        if !matches!(self.state, EngineState::Running) {
            return None;
        }
        let frame = self
            .video
            .as_ref()
            .and_then(decoder::VideoStream::pop_frame)?;
        let pts_ms = self.stream_start_ms + self.frames_emitted as f64 * (1000.0 / self.fps);
        self.frames_emitted += 1;
        Some(DecodedFrame {
            rgba: frame,
            width: self.width,
            height: self.height,
            pts_ms,
        })
    }

    /// 故障态查询(接线方据此切横幅/回落幻灯片)
    pub fn is_faulted(&self) -> bool {
        matches!(self.state, EngineState::Faulted(_))
    }

    /// 最近一次故障(故障态下 Some;load 成功即清除)
    pub fn last_error(&self) -> Option<&PlaybackError> {
        match &self.state {
            EngineState::Faulted(e) => Some(e),
            _ => None,
        }
    }

    /// 停流停进程,回 Idle。Drop 等价于调用本方法。
    pub fn shutdown(&mut self) {
        self.stop_streams();
        self.state = EngineState::Idle;
    }

    /// 非阻塞健康检查:视频停流/解码失败 → Faulted;音频故障 → 弃音频回落墙钟
    fn check_health(&mut self) {
        if !matches!(self.state, EngineState::Running) {
            return;
        }
        let vf = self.video.as_ref().and_then(decoder::VideoStream::fault);
        match vf {
            Some(StreamFault::Stalled) => {
                self.enter_faulted(PlaybackError::Stalled);
                return;
            }
            Some(StreamFault::Failed(m)) | Some(StreamFault::NoOutput(m)) => {
                self.enter_faulted(PlaybackError::Decode(m));
                return;
            }
            None => {}
        }
        if self
            .audio
            .as_ref()
            .and_then(audio::AudioStream::fault)
            .is_some()
        {
            // 音频故障不致命:弃音频,下个泵周期 tick(None) 回落墙钟
            self.audio = None;
        }
    }

    fn enter_faulted(&mut self, e: PlaybackError) {
        self.stop_streams();
        self.clock.pause();
        self.state = EngineState::Faulted(e);
    }

    fn stop_streams(&mut self) {
        self.video = None; // Drop:kill + wait
        self.audio = None;
    }

    fn apply_muted(&mut self) {
        if let Some(a) = &self.audio {
            a.set_muted(self.muted);
        }
    }
}

impl Drop for PlaybackEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;
