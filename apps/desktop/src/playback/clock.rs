//! 播放主时钟(I1,工单 docs/tickets/I1-S1):音频样本主时钟 + 墙钟 fallback。
//!
//! 纯结构体、无线程、无 I/O,由引擎在 UI 泵里驱动:
//! - **AudioMaster**(音频流活着且 speed == 1.0):position = 重锚位 + 已消费
//!   样本数 ÷ 采样率(样本计数由引擎从 audio.rs 取,喂给 [`PlaybackClock::tick`]);
//! - **WallClock**(无音频/变速/音频故障回落):`Instant × speed`。
//!
//! 模式切换在 `tick()` 内自动重锚(位置折算,不跳变);`position_ms()` 单调
//! 非降,唯一可回退的路径是 `seek()` 重锚。

use std::time::Instant;

/// 速度档边界(工单:0.25~4.0,越界夹取)
pub(crate) const SPEED_MIN: f32 = 0.25;
pub(crate) const SPEED_MAX: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Wall,
    Audio,
}

pub(crate) struct PlaybackClock {
    speed: f32,
    playing: bool,
    mode: Mode,
    /// 最近一次重锚时的播放位置(ms)
    anchor_pos_ms: f64,
    /// Wall 模式重锚时刻
    anchor_wall: Instant,
    /// Audio 模式重锚时的已消费样本计数
    anchor_samples: u64,
    /// 最近一次 tick 喂入的样本计数与采样率
    latest_samples: Option<u64>,
    sample_rate: u32,
    /// 挂载音频时,下一个音频流起点的媒体位置(seek 时由引擎写入;
    /// None = 挂载属流中途重挂,以当前墙钟位置为基准)
    pending_audio_base_ms: Option<f64>,
    /// 单调下限:position_ms() 永不返回比这更小的值(seek 时重置)
    floor_ms: f64,
}

impl Default for PlaybackClock {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaybackClock {
    pub(crate) fn new() -> Self {
        Self {
            speed: 1.0,
            playing: false,
            mode: Mode::Wall,
            anchor_pos_ms: 0.0,
            anchor_wall: Instant::now(),
            anchor_samples: 0,
            latest_samples: None,
            sample_rate: 48_000,
            pending_audio_base_ms: None,
            floor_ms: 0.0,
        }
    }

    pub(crate) fn play(&mut self) {
        if !self.playing {
            self.anchor_wall = Instant::now();
            self.playing = true;
        }
    }

    /// 暂停:折算已走时长进锚点,回落墙钟(音频流由引擎暂停,样本冻结;
    /// 恢复播放后下个 tick 以新样本计数重挂音频,位置无跳变)
    pub(crate) fn pause(&mut self) {
        if self.playing {
            self.anchor_pos_ms = self.raw_position_ms();
            self.mode = Mode::Wall;
            self.playing = false;
        }
    }

    pub(crate) fn is_playing(&self) -> bool {
        self.playing
    }

    /// 变速:折算当前位置后按新速度重锚(墙钟);速度回到 1.0 且音频可用时
    /// 由引擎下个 tick 重挂音频主时钟
    pub(crate) fn set_speed(&mut self, speed: f32) {
        self.anchor_pos_ms = self.raw_position_ms();
        self.speed = speed.clamp(SPEED_MIN, SPEED_MAX);
        self.mode = Mode::Wall;
        self.anchor_wall = Instant::now();
    }

    pub(crate) fn speed(&self) -> f32 {
        self.speed
    }

    /// 引擎每次泵调用:音频流健康时喂已消费样本计数(48000 样本 = 1s@48k),
    /// 否则喂 None(回落墙钟)。内部按需重锚换模式,位置不跳变。
    pub(crate) fn tick(&mut self, audio_consumed: Option<u64>, sample_rate: u32) {
        match (audio_consumed, self.mode) {
            (Some(c), Mode::Audio) => {
                // 主时钟续走:锚点不动
                let _ = c;
            }
            (Some(c), Mode::Wall) => {
                // 挂载音频主时钟:seek 后首挂以流起点媒体位为基准(计数自
                // 流起点);流中途重挂(如变速回 1.0)以当前墙钟位折算
                if let Some(base) = self.pending_audio_base_ms.take() {
                    self.anchor_pos_ms = base;
                    self.anchor_samples = 0;
                } else {
                    self.anchor_pos_ms = self.raw_position_ms();
                    self.anchor_samples = c;
                }
                self.sample_rate = sample_rate;
                self.mode = Mode::Audio;
            }
            (None, Mode::Audio) => {
                // 卸载音频主时钟(流结束/变速/故障回落):当前位置折算回墙钟
                self.anchor_pos_ms = self.raw_position_ms();
                self.anchor_wall = Instant::now();
                self.mode = Mode::Wall;
                self.pending_audio_base_ms = None;
            }
            (None, Mode::Wall) => {
                self.pending_audio_base_ms = None;
            }
        }
        self.latest_samples = audio_consumed;
    }

    /// seek 重锚:位置跳到 t 并以墙钟起步;同时登记 t 为下一个音频流的
    /// 起点(引擎 seek 必然重启音视频流,挂载时按此对齐样本计数)
    pub(crate) fn seek(&mut self, pos_ms: f64) {
        self.anchor_pos_ms = pos_ms;
        self.anchor_wall = Instant::now();
        self.latest_samples = None;
        self.mode = Mode::Wall;
        self.pending_audio_base_ms = Some(pos_ms);
        self.floor_ms = pos_ms;
    }

    /// 当前位置(ms);除 seek 外单调非降
    pub(crate) fn position_ms(&mut self) -> f64 {
        let raw = self.raw_position_ms();
        if raw < self.floor_ms {
            self.floor_ms
        } else {
            self.floor_ms = raw;
            raw
        }
    }

    /// 不带单调夹取的原始位置(重锚计算内部用)
    fn raw_position_ms(&self) -> f64 {
        match self.mode {
            Mode::Wall => {
                if !self.playing {
                    return self.anchor_pos_ms;
                }
                let elapsed_ms = self.anchor_wall.elapsed().as_secs_f64() * 1000.0;
                self.anchor_pos_ms + elapsed_ms * f64::from(self.speed)
            }
            Mode::Audio => {
                let Some(consumed) = self.latest_samples else {
                    return self.anchor_pos_ms;
                };
                let delta = consumed.saturating_sub(self.anchor_samples);
                self.anchor_pos_ms + delta as f64 / f64::from(self.sample_rate) * 1000.0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PlaybackClock, SPEED_MAX, SPEED_MIN};
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    #[test]
    fn 暂停冻结_播放前进() {
        let mut c = PlaybackClock::new();
        assert!(!c.is_playing());
        assert!((c.position_ms() - 0.0).abs() < 1e-9, "初始位 0");
        sleep(Duration::from_millis(10));
        assert!((c.position_ms() - 0.0).abs() < 1e-9, "暂停不走");

        c.play();
        sleep(Duration::from_millis(30));
        let p = c.position_ms();
        assert!(p >= 25.0, "1x 下 30ms 至少走 ~30ms,实际 {p}");
        c.pause();
        let a = c.position_ms();
        sleep(Duration::from_millis(20));
        let b = c.position_ms();
        assert!((a - b).abs() < 1e-9, "暂停后冻结 {a} vs {b}");
    }

    #[test]
    fn 位置单调非降() {
        let mut c = PlaybackClock::new();
        c.play();
        let mut last = c.position_ms();
        for _ in 0..20 {
            sleep(Duration::from_millis(2));
            let p = c.position_ms();
            assert!(p >= last, "位置回退 {last} → {p}");
            last = p;
        }
    }

    #[test]
    fn seek_重锚并可继续前进() {
        let mut c = PlaybackClock::new();
        c.play();
        sleep(Duration::from_millis(20));
        c.seek(5_000.0);
        let p = c.position_ms();
        assert!((p - 5_000.0).abs() < 5.0, "seek 后应≈5000,实际 {p}");
        sleep(Duration::from_millis(20));
        let q = c.position_ms();
        assert!(q >= 5_015.0, "seek 后继续走 {q}");
        // seek 允许回退(唯一的回退路径)
        c.seek(100.0);
        assert!((c.position_ms() - 100.0).abs() < 5.0);
    }

    #[test]
    fn 速度换算与夹取() {
        let mut c = PlaybackClock::new();
        c.set_speed(0.1);
        assert!((c.speed() - SPEED_MIN).abs() < 1e-6, "低于下限夹到 0.25");
        c.set_speed(99.0);
        assert!((c.speed() - SPEED_MAX).abs() < 1e-6, "高于上限夹到 4.0");
        c.set_speed(1.5);
        assert!((c.speed() - 1.5).abs() < 1e-6);

        // 2x 墙钟:位移 ≈ 真实流逝 × 2(宽容界,防调度抖动)
        let mut c = PlaybackClock::new();
        c.set_speed(2.0);
        c.play();
        let t0 = Instant::now();
        sleep(Duration::from_millis(40));
        let elapsed = t0.elapsed().as_secs_f64() * 1000.0;
        let p = c.position_ms();
        assert!(
            p >= elapsed * 2.0 - 5.0 && p <= elapsed * 2.0 + 5.0 + elapsed * 0.3,
            "2x 下 {elapsed:.1}ms 应走 ≈{:.1}ms,实际 {p}",
            elapsed * 2.0
        );
    }

    #[test]
    fn 音频主时钟_样本推进与模式切换无跳变() {
        let mut c = PlaybackClock::new();
        c.play();
        c.seek(0.0); // 引擎流程:load/seek 登记流起点后,音频流才开始喂样本
        // 48000 样本 = 1s@48k(自流起点计数)
        c.tick(Some(48_000), 48_000);
        let p = c.position_ms();
        assert!((p - 1_000.0).abs() < 1.0, "1s 音频样本应≈1000ms,实际 {p}");
        c.tick(Some(96_000), 48_000);
        let p = c.position_ms();
        assert!((p - 2_000.0).abs() < 1.0);

        // 音频消失 → 回落墙钟,位置不跳变
        c.tick(None, 48_000);
        let before = c.position_ms();
        assert!((before - 2_000.0).abs() < 5.0, "切墙钟不跳变,实际 {before}");
        // 墙钟继续走(播放中)
        sleep(Duration::from_millis(10));
        let anchor_pos = c.position_ms();
        assert!(anchor_pos > before);

        // 流中途重挂(pending 基准已被清):以当前位置为锚,增量继续换算
        c.tick(Some(9_600), 48_000);
        let p = c.position_ms();
        assert!(
            (p - anchor_pos).abs() < 5.0,
            "重挂不跳变,实际 {p} 锚 {anchor_pos}"
        );
        c.tick(Some(14_400), 48_000); // +4800 样本 = +100ms
        let q = c.position_ms();
        assert!(
            (q - (anchor_pos + 100.0)).abs() < 2.0,
            "重挂后 +4800 样本应 = +100ms,实际 {q} 锚 {anchor_pos}"
        );
    }

    #[test]
    fn 音频主时钟_样本冻结即暂停_乱序不回退() {
        let mut c = PlaybackClock::new();
        c.play();
        c.seek(0.0);
        c.tick(Some(48_000), 48_000);
        let a = c.position_ms();
        // 音频流被暂停 → 消费计数冻结 → 位置冻结
        sleep(Duration::from_millis(20));
        let b = c.position_ms();
        assert!((a - b).abs() < 1.0, "样本冻结位置应冻结 {a} vs {b}");
        // 喂入更小的计数(异常/重启竞态)→ 单调夹取不回退
        c.tick(Some(0), 48_000);
        let d = c.position_ms();
        assert!(d >= b, "乱序样本不得回退 {b} → {d}");
    }

    #[test]
    fn seek_后音频重挂以新位为锚() {
        let mut c = PlaybackClock::new();
        c.play();
        c.tick(Some(480_000), 48_000); // 走到 10s
        c.seek(1_000.0); // 回跳 1s:先落墙钟
        let p = c.position_ms();
        assert!((p - 1_000.0).abs() < 5.0);
        // 新流从 0 计数:1s 音频 = 锚位 +1000ms
        c.tick(Some(48_000), 48_000);
        let q = c.position_ms();
        assert!((q - 2_000.0).abs() < 5.0, "seek 后重挂应≈2000ms,实际 {q}");
    }

    #[test]
    fn 变速瞬间不跳变_音频模式变速回落墙钟() {
        let mut c = PlaybackClock::new();
        c.play();
        c.seek(0.0);
        c.tick(Some(96_000), 48_000); // 走到 2s
        c.set_speed(2.0); // 折算 2s 后按 2x 重锚
        let p = c.position_ms();
        assert!((p - 2_000.0).abs() < 5.0, "变速不跳变,实际 {p}");
        // 音频在 2x 下不挂载(engine 契约:仅 speed==1 喂样本),喂 None 维持墙钟
        sleep(Duration::from_millis(20));
        let q = c.position_ms();
        assert!(q > p + 30.0, "2x 墙钟应明显前进 {p} → {q}");
    }
}
