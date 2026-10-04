//! 播放用户操作面(I1 M3;A-02 拆分自 app.rs「M3」段):
//! 播放/暂停 / seek / JKL 变速 / 静音 / 循环 / 画质 / 沉浸 / 截图 / 拖拽 scrub,
//! 以及面板消费的查询面(engine_active / transport_params 等)。
//! 状态机核心(播放泵/降级链/重同步)在 `playback_facade.rs`。

use std::time::Instant;

use sable::gpui::{Context, Window};
use sable::gpui_component::WindowExt as _;
use sable::gpui_component::notification::Notification;

use crate::rpc::Rpc;

use super::super::DesktopApp;
use super::has_audio_device;
use super::source_map::{
    EngineSource, SPEED_LADDER_ALL, SPEED_LADDER_DOWN, SPEED_LADDER_UP, direct_params_at,
    fmt_speed, ladder_next,
};

impl DesktopApp {
    /// 引擎路径在场(帧由播放泵供给;预览面板据此跳过 render_frame)。
    pub fn engine_active(&self) -> bool {
        self.engine_source.is_some() || self.zone_loading.is_some()
    }

    /// 拖进度条进行中(预览面板 mouse-move/up 收尾判定用)。
    pub fn scrubbing(&self) -> bool {
        self.scrubbing.is_some()
    }

    /// 引擎有效速度(片段速度 × 变速挡)与有效静音(≠1x 自动静音:音频不跟速)。
    pub fn transport_params(&self) -> (f32, bool) {
        let spd = self.engine_speed_effective();
        (spd, self.user_muted || (spd - 1.0).abs() > 1e-3)
    }

    /// 引擎有效速度(检查器/设置页展示)。
    pub fn engine_speed_effective(&self) -> f32 {
        let clip_speed = match &self.engine_source {
            Some(EngineSource::Direct { clip_speed, .. }) => *clip_speed,
            _ => 1.0,
        };
        (clip_speed * self.transport_speed as f64) as f32
    }

    /// 播放/暂停(到尾自动停;从 0 重播)。
    /// 流畅画质(默认)走引擎(M2 zone 优先,M1 直解码),精确画质走幻灯片。
    pub fn toggle_play(&mut self, cx: &mut Context<Self>) {
        if self.playing {
            self.pause_playback(cx);
        } else {
            self.start_play(cx);
        }
        cx.notify();
    }

    pub fn set_playhead(&mut self, ms: u64, cx: &mut Context<Self>) {
        let mut ms = ms;
        if self.duration_ms > 0 {
            ms = ms.min(self.duration_ms);
        }
        self.playhead_ms = ms;
        // 引擎在场:源内 seek(重启流,约百 ms;离散按键可接受),越界弃源。
        // 播放态保持——引擎 seek 不改播放态,恢复由调用方(如 end_scrub)处理
        if let Some(src) = self.engine_source.clone() {
            match &src {
                EngineSource::Zone {
                    start_ms, end_ms, ..
                } => {
                    let (zs, ze) = (*start_ms, *end_ms);
                    if ms >= zs && ms < ze {
                        if let Some(e) = self.engine.as_mut() {
                            e.seek_ms((ms - zs) as f64);
                            self.frame_gate = f64::NEG_INFINITY;
                        }
                    } else {
                        self.drop_engine_source(cx);
                    }
                }
                EngineSource::Direct { .. } => {
                    let ds = direct_params_at(&src, ms);
                    match ds {
                        Some(media_t) => {
                            if let Some(e) = self.engine.as_mut() {
                                e.seek_ms(media_t);
                                self.frame_gate = f64::NEG_INFINITY;
                            }
                        }
                        None => self.drop_engine_source(cx),
                    }
                }
            }
        }
        if !self.engine_active() {
            let at = self.playhead_ms.min(self.duration_ms.saturating_sub(100));
            *self.shared.preview_request.lock().unwrap() = Some(at);
        }
        cx.notify();
    }

    /// L:正向倍速循环 1→2→4→1(≠1x 自动静音,UI 显示静音图标)。
    pub fn transport_faster(&mut self, cx: &mut Context<Self>) {
        let next = ladder_next(&SPEED_LADDER_UP, self.transport_speed, true, 1.0);
        self.set_transport_speed(next, cx);
    }

    /// Shift+L:反向倍速循环 1→0.5→0.25。
    pub fn transport_reverse(&mut self, cx: &mut Context<Self>) {
        let next = ladder_next(&SPEED_LADDER_DOWN, self.transport_speed, true, 1.0);
        self.set_transport_speed(next, cx);
    }

    /// J:减速方向(4→2→1→0.5→0.25,到 0.25 停)。
    pub fn transport_step_down(&mut self, cx: &mut Context<Self>) {
        let next = ladder_next(&SPEED_LADDER_ALL, self.transport_speed, false, 0.25);
        self.set_transport_speed(next, cx);
    }

    /// K:暂停(空格才是播放/暂停切换)。
    pub fn pause_transport(&mut self, cx: &mut Context<Self>) {
        if self.playing {
            self.pause_playback(cx);
        } else {
            self.status = "已暂停".into();
            cx.notify();
        }
    }

    fn set_transport_speed(&mut self, speed: f32, cx: &mut Context<Self>) {
        self.transport_speed = speed;
        let (spd, muted) = self.transport_params();
        if let Some(e) = self.engine.as_mut() {
            e.set_speed(spd);
            e.set_muted(muted);
        }
        let note = if muted && (spd - 1.0).abs() > 1e-3 {
            "(静音)"
        } else {
            ""
        };
        self.status = format!("倍速 {}{}", fmt_speed(speed), note);
        cx.notify();
    }

    /// 静音开关(无声卡时 toast 提示;引擎对静音是丢弃式消费,不回跳)。
    pub fn toggle_mute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.user_muted = !self.user_muted;
        let muted = self.transport_params().1;
        if let Some(e) = self.engine.as_mut() {
            e.set_muted(muted);
        }
        if !has_audio_device() {
            window.push_notification(Notification::info("本机无音频设备,已按静音处理"), cx);
        }
        self.status = format!("静音 {}", if self.user_muted { "开" } else { "关" });
        cx.notify();
    }

    /// 当前片段 A→B 循环开关(引擎播到片段尾自动 seek+play)。
    pub fn toggle_loop(&mut self, cx: &mut Context<Self>) {
        self.loop_clip = !self.loop_clip;
        self.status = if self.loop_clip {
            "循环开:当前片段 A→B".into()
        } else {
            "循环关".into()
        };
        cx.notify();
    }

    /// 画质切换:精确 = 既有 render_frame 路径;流畅 = 引擎(默认)。
    pub fn toggle_quality(&mut self, cx: &mut Context<Self>) {
        self.quality_precise = !self.quality_precise;
        if self.quality_precise {
            if let Some(e) = self.engine.as_mut() {
                e.shutdown();
            }
            self.engine_source = None;
            self.zone_loading = None;
            self.status = "画质:精确(逐帧 render_frame)".into();
            if self.playing {
                self.last_frame = Instant::now();
                let at = self
                    .playhead_ms
                    .min(self.duration_ms.saturating_sub(100).max(self.playhead_ms));
                *self.shared.preview_request.lock().unwrap() = Some(at);
            }
        } else {
            self.status = "画质:流畅(引擎)".into();
            if self.playing {
                self.start_play(cx);
            }
        }
        cx.notify();
    }

    /// 沉浸预览(收起其他面板只留预览;Esc 退出;非系统级全屏)。
    pub fn toggle_immersive(&mut self, cx: &mut Context<Self>) {
        self.immersive = !self.immersive;
        cx.notify();
    }

    /// 截图:render_frame 当前播放头 → 复制到 <工程>/screenshots/<时间戳>.png。
    /// (watcher 会触发一次无害重拉,接受;toast 显示相对路径)
    pub fn screenshot(&mut self, cx: &mut Context<Self>) {
        // 钳在最后一帧之前(=duration 抽帧会落空,内核无帧可出,与预览同口径)
        let t = self.playhead_ms.min(self.duration_ms.saturating_sub(100));
        let rpc = self.rpc.clone();
        let shared = self.shared.clone();
        let root = self.project_dir.clone();
        self.status = "截图渲染中…".into();
        cx.background_executor()
            .spawn(async move {
                let res = screenshot_once(&rpc, &root, t);
                *shared.screenshot_result.lock().unwrap() = Some(res);
            })
            .detach();
        cx.notify();
    }

    /// 拖进度条:按下开始(引擎暂停记忆播放态;拖拽零动画直接映射)。
    pub fn begin_scrub(&mut self, cx: &mut Context<Self>) {
        if self.scrubbing.is_some() {
            return;
        }
        let was = self.playing;
        if was && let Some(e) = self.engine.as_mut() {
            e.pause();
        }
        self.playing = false;
        self.scrubbing = Some(was);
        cx.notify();
    }

    /// 拖动中:只更新播放头显示(不 seek 不出帧请求;松手才 seek)。
    pub fn scrub_to(&mut self, ms: u64, cx: &mut Context<Self>) {
        if self.scrubbing.is_none() {
            return;
        }
        self.playhead_ms = if self.duration_ms > 0 {
            ms.min(self.duration_ms)
        } else {
            ms
        };
        cx.notify();
    }

    /// 松手:提交 seek(引擎重启流),原播放态恢复。
    pub fn end_scrub(&mut self, ms: u64, cx: &mut Context<Self>) {
        let Some(was) = self.scrubbing.take() else {
            return;
        };
        self.set_playhead(ms, cx);
        if was {
            if self.engine_source.is_some() {
                let params = self.transport_params();
                if let Some(e) = self.engine.as_mut() {
                    let (spd, muted) = params;
                    e.set_speed(spd);
                    e.set_muted(muted);
                    e.play();
                }
                self.playing = true;
            } else {
                self.start_play(cx);
            }
        }
        cx.notify();
    }
}

/// 截图单发:render_frame → 复制到 <root>/screenshots/(后台线程)。
fn screenshot_once(rpc: &Rpc, root: &str, t_ms: u64) -> Result<String, String> {
    let data = rpc.call(
        "render_frame",
        serde_json::json!({ "atMs": t_ms }),
        crate::rpc::render_timeout("render_frame"),
    )?;
    let png = crate::panels::preview::find_png_path(&data).ok_or("响应中未找到 PNG 路径")?;
    let src = rpc.absolutize(&png);
    let dir = std::path::PathBuf::from(root).join("screenshots");
    std::fs::create_dir_all(&dir).map_err(|e| format!("建目录失败:{e}"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let name = format!("cutforge-shot-{stamp}.png");
    let dest = dir.join(&name);
    std::fs::copy(&src, &dest).map_err(|e| format!("复制失败:{e}"))?;
    Ok(format!("screenshots/{name}(播放头 {t_ms}ms)"))
}
