//! 播放状态机门面(A-02 拆分):`playback/` 引擎的宿主接线,与 UI 解耦——
//! 只经 `Shared` 槽位与 `DesktopApp` 状态字段交互,不认识面板/命令协议。
//!
//! 职责:
//! - 播放泵(16ms):zone 结果搬运 / 引擎推进与帧搬运 / 段尾推进 / 编辑重同步;
//! - 三级降级链:zone 复用 → zone 请求 → 直解码 → 幻灯片(不白屏不 panic);
//! - 播放头 seek 的引擎域换算(纯换算在 [`source_map`],可单测)。
//!
//! 用户操作面(JKL/静音/画质/截图/沉浸/拖拽)在 `playback_facade::transport`。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use sable::gpui::Context;

use crate::playback::{PlaybackEngine, zone as zone_cache};
use crate::state::EngineFrame;

use super::DesktopApp;
use source_map::{
    DirectSource, EngineSource, ZoneReady, parse_zone_data, project_time_of, resolve_direct_source,
    source_end_ms,
};

pub(crate) mod source_map;
mod transport;

/// zone 请求跨度(播放头起向后 8s;钳内容长度,100ms 网格量化在内核)。
const ZONE_SPAN_MS: u64 = 8_000;
/// 播放中编辑 → 重同步防抖。
const RESYNC_DEBOUNCE: Duration = Duration::from_millis(300);

/// 播放路径日志(工单验收:播放路径有日志证据——帧率/回落事件)。
/// GUI 形态下 stderr 常不可见:默认落系统临时目录,CUTFORGE_PLAY_LOG 可覆盖路径。
pub(crate) fn play_log(msg: &str) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    eprintln!("[cutforge-play @{stamp}] {msg}");
    let path = std::env::var("CUTFORGE_PLAY_LOG").unwrap_or_else(|_| {
        std::env::temp_dir()
            .join("cutforge-play.log")
            .display()
            .to_string()
    });
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write as _;
        let _ = writeln!(f, "[cutforge-play @{stamp}] {msg}");
    }
}

/// 本机是否有音频输出设备(无声卡 = WASAPI 0 设备;引擎本就自动回落墙钟)。
fn has_audio_device() -> bool {
    use cpal::traits::HostTrait as _;
    cpal::default_host().default_output_device().is_some()
}

impl DesktopApp {
    /// 播放泵(16ms):zone 结果搬运 / 引擎推进与帧搬运 / 段尾推进 /
    /// 播放中编辑重同步(300ms 防抖)。幻灯片推进仍在 100ms 泵。
    pub(crate) fn engine_pump(&mut self, cx: &mut Context<Self>) {
        let mut notify = false;
        // 1) zone 渲染结果(后台任务写;失败静默回落 M1 直解码)
        let zone_arrival = self
            .shared
            .zone_result
            .lock()
            .ok()
            .and_then(|mut g| g.take());
        if let Some(res) = zone_arrival {
            self.zone_loading = None;
            match res {
                Ok(z) => {
                    let file = self.rpc.absolutize(&z.file);
                    // 指纹 marker:渲染编排方(壳)职责——成功渲染后落
                    // `<key>.fresh`,复用判定走 playback::zone::is_fresh
                    let session = zone_cache::zone_key(&z.key, z.start_ms, z.end_ms);
                    if !z.key.is_empty() {
                        let dir =
                            zone_cache::preview_cache_dir(std::path::Path::new(&self.project_dir));
                        let _ = std::fs::create_dir_all(&dir);
                        let _ = std::fs::write(dir.join(format!("{}.fresh", z.key)), "");
                    }
                    play_log(&format!(
                        "zone 就绪 {session} [{}, {}) file={}",
                        z.start_ms, z.end_ms, z.file
                    ));
                    self.zone_ready = Some(ZoneReady {
                        file,
                        start_ms: z.start_ms,
                        end_ms: z.end_ms,
                        key: z.key,
                        rev: z.rev,
                    });
                    if self.playing && !self.quality_precise {
                        self.play_zone_ready(cx);
                    }
                }
                Err(e) => {
                    play_log(&format!("zone 失败,回落直解码:{e}"));
                    if self.playing && !self.quality_precise && self.engine_source.is_none() {
                        self.start_direct_play(cx);
                    }
                }
            }
            notify = true;
        }
        // 2) 引擎推进 + 帧搬运(故障 → 回落;段尾 → 推进下一源)
        let mut frame_out: Option<EngineFrame> = None;
        if self.playing
            && let Some(src) = self.engine_source.clone()
        {
            let mut fallback_reason: Option<String> = None;
            let mut ended = false;
            if let Some(engine) = self.engine.as_mut() {
                let pos = engine.position_ms();
                if engine.is_faulted() {
                    fallback_reason = Some(
                        engine
                            .last_error()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "未知故障".into()),
                    );
                } else {
                    self.playhead_ms = project_time_of(&src, pos).min(self.duration_ms);
                    if pos + 0.5 >= source_end_ms(&src) && !engine.is_playing() {
                        ended = true; // 引擎到尾自动 pause
                    }
                    // 帧步进按主时钟:背压下解码被消费速度拉平,消费节奏必须
                    // 与帧边界对账——用**累计帧序号**而非 pos 差值:16ms 泵 tick
                    // 走 16/32/48ms,差值门控会在每次弹帧后重置基准,实际
                    // 48ms/帧 = 20.8fps 天花板(实测 17fps);序号对账后每跨一
                    // 条帧边界消费一帧,30fps 内容即 30fps 消费(fps 防御 ≤0)
                    let fps = self.shared.snapshot().fps();
                    let frame_dur = (1000.0 / if fps > 0.0 { fps } else { 30.0 }).max(1.0);
                    let frame_idx = (pos / frame_dur).floor();
                    if frame_idx != self.frame_gate {
                        if let Some(f) = engine.poll_frame() {
                            self.frame_gate = frame_idx;
                            self.pop_miss = false;
                            let t = project_time_of(&src, f.pts_ms).min(self.duration_ms);
                            frame_out = Some(EngineFrame::new(t, f.rgba, f.width, f.height));
                        } else if !self.pop_miss {
                            // 门开而环空:解码供给断(eof/丢帧/停流),每窗记一次
                            self.pop_miss = true;
                            play_log(&format!(
                                "帧供给落空 pos={pos:.0} fault={} err={:?}",
                                engine.is_faulted(),
                                engine.last_error()
                            ));
                        }
                    }
                }
            }
            notify = true;
            if let Some(reason) = fallback_reason {
                self.fallback_to_slideshow(Some(reason), cx);
            } else if ended {
                self.advance_after_source_end(cx);
            }
        }
        // 3) 播放中编辑重同步:rev 变化 → 300ms 防抖后重解析,变了才重启流
        if self.playing && self.engine_source.is_some() {
            let rev = self.shared.snapshot_rev();
            if rev != self.engine_source_rev {
                if self.resync_at.is_none() {
                    self.resync_at = Some(Instant::now() + RESYNC_DEBOUNCE);
                } else if Instant::now() >= self.resync_at.expect("刚置的防抖时刻必有值")
                {
                    self.resync_at = None;
                    self.resync_engine(cx);
                    notify = true;
                }
            } else {
                self.resync_at = None;
            }
        }
        // 4) 帧上屏(shared 槽 → 预览面板泵 → RenderImage)
        // 注:此刻 DesktopApp 正在更新中,面板不可回读宿主(gpui 实体借规,
        // 回读即 panic——冒烟实测),状态由这里算好传入
        if let Some(f) = frame_out {
            self.stat_frames += 1;
            *self.shared.engine_frame.lock().unwrap() = Some(f);
            let zone_loading_now = self.zone_loading.is_some();
            if let Some(p) = self.preview.clone() {
                p.update(cx, |panel, cx| panel.pump_from_host(cx, zone_loading_now));
            }
            notify = true;
        }
        // 帧率日志(2s 窗口;验收要求播放路径有日志证据)
        if self.playing && self.engine_source.is_some() {
            let elapsed = self.stat_at.elapsed();
            if elapsed >= Duration::from_secs(2) {
                let fps = self.stat_frames as f64 / elapsed.as_secs_f64();
                play_log(&format!(
                    "帧率 {:.1} fps(窗口 {} 帧 / {:.1}s)播放头 {}ms",
                    fps,
                    self.stat_frames,
                    elapsed.as_secs_f64(),
                    self.playhead_ms
                ));
                self.stat_at = Instant::now();
                self.stat_frames = 0;
            }
        }
        if notify {
            cx.notify();
        }
    }

    /// 引擎段尾推进:zone 播完续直解码;直解码片段播完按循环/下一段续走;
    /// 到工程尾停机。下一段非视频(图片/文本/空窗)→ 幻灯片续走。
    fn advance_after_source_end(&mut self, cx: &mut Context<Self>) {
        let Some(src) = self.engine_source.clone() else {
            return;
        };
        let t = self.playhead_ms;
        match &src {
            EngineSource::Zone { file, .. } => {
                play_log(&format!(
                    "zone 段播完 t={t} file={} → 续直解码",
                    file.display()
                ));
                self.engine_source = None;
                if t >= self.duration_ms {
                    self.playing = false;
                    self.status = "播放结束".into();
                } else {
                    self.start_direct_play(cx);
                    return;
                }
            }
            EngineSource::Direct {
                clip_start_ms,
                media_in,
                ..
            } => {
                let (cs, mi) = (*clip_start_ms, *media_in);
                if self.loop_clip && t >= cs {
                    // A→B 循环:回片段头续播(引擎 seek = 重启流,约百 ms)
                    let params = self.transport_params();
                    if let Some(e) = self.engine.as_mut() {
                        e.seek_ms(mi);
                        let (spd, muted) = params;
                        e.set_speed(spd);
                        e.set_muted(muted);
                        e.play();
                        self.frame_gate = f64::NEG_INFINITY;
                    }
                    self.playhead_ms = cs;
                    self.status = "循环播放".into();
                    return;
                }
                self.engine_source = None;
                if t >= self.duration_ms {
                    self.playing = false;
                    self.status = "播放结束".into();
                    if let Some(e) = self.engine.as_mut() {
                        e.pause();
                    }
                } else {
                    self.start_direct_play(cx);
                    return;
                }
            }
        }
        cx.notify();
    }

    /// 载入已就绪 zone 并播放(rev/marker/文件失配 → 静默转直解码)。
    fn play_zone_ready(&mut self, cx: &mut Context<Self>) {
        let Some(z) = self.zone_ready.clone() else {
            return;
        };
        let rev = self.shared.snapshot_rev();
        // 新鲜度:rev 对账(提交即作废)+ 壳侧指纹 marker(渲染编排方职责)+
        // 文件存在(防 gc 半途);三者齐备才免重渲复用
        let marker_fresh = !z.key.is_empty()
            && zone_cache::is_fresh(
                &zone_cache::preview_cache_dir(std::path::Path::new(&self.project_dir)),
                &z.key,
            );
        let drifted = self.playhead_ms < z.start_ms || self.playhead_ms >= z.end_ms;
        if z.rev != rev || drifted || !marker_fresh || !z.file.exists() {
            play_log(&format!(
                "zone 作废(rev 失配={}, 播放头漂移={drifted}, marker 失鲜={mf}, 文件缺失={fe}),直解码",
                z.rev != rev,
                mf = !marker_fresh,
                fe = !z.file.exists()
            ));
            self.zone_ready = None;
            self.start_direct_play(cx);
            return;
        }
        let fps = self.shared.snapshot().fps();
        let in_ms = (self.playhead_ms - z.start_ms) as f64;
        let out_ms = (z.end_ms - z.start_ms) as f64;
        let params = self.transport_params();
        let loaded = self
            .engine
            .as_mut()
            .map(|e| e.load_clip(&z.file, in_ms, out_ms, fps));
        match loaded {
            Some(Ok(())) => {
                let (spd, muted) = params;
                let e = self.engine.as_mut().expect("engine 刚加载成功必在场");
                e.set_speed(spd);
                e.set_muted(muted);
                e.play();
                self.engine_source = Some(EngineSource::Zone {
                    file: z.file,
                    start_ms: z.start_ms,
                    end_ms: z.end_ms,
                });
                self.engine_source_rev = rev;
                self.engine_note = None;
                self.playing = true;
                self.frame_gate = f64::NEG_INFINITY;
                play_log(&format!(
                    "zone 起播 [{}, {}) in={in_ms} out={out_ms} fps={fps} 倍速={spd}",
                    z.start_ms, z.end_ms
                ));
                self.status = "播放中(zone 预览)".into();
            }
            Some(Err(e)) => {
                play_log(&format!("zone 加载失败:{e}"));
                self.engine_note = Some(format!("zone 加载失败:{e}"));
                self.start_direct_play(cx);
            }
            None => self.fallback_to_slideshow(Some("引擎未初始化".into()), cx),
        }
        cx.notify();
    }

    /// M1 直解码起播:播放头所在视频片段 → load_clip(绝对 src,媒体内偏移,fps)。
    fn start_direct_play(&mut self, cx: &mut Context<Self>) {
        let t = self.playhead_ms;
        let rev = self.shared.snapshot_rev();
        let fps = self.shared.snapshot().fps();
        let params = self.transport_params();
        let resolved = self.resolve_direct_source(t);
        match resolved {
            Some(ds) => {
                let DirectSource {
                    clip_id,
                    src,
                    clip_start_ms: cs,
                    clip_end_ms: ce,
                    source_in,
                    media_in,
                    media_out,
                    clip_speed,
                } = ds;
                play_log(&format!(
                    "直解码起播 clip={clip_id} src={} media=[{media_in:.0}, {media_out:.0}) 速度={clip_speed} 倍速={}",
                    src.display(),
                    params.0
                ));
                let loaded = self
                    .engine
                    .as_mut()
                    .map(|e| e.load_clip(&src, media_in, media_out, fps));
                match loaded {
                    Some(Ok(())) => {
                        let (spd, muted) = params;
                        let e = self.engine.as_mut().expect("engine 刚加载成功必在场");
                        e.set_speed(spd);
                        e.set_muted(muted);
                        e.play();
                        self.engine_source = Some(EngineSource::Direct {
                            clip_id,
                            src,
                            clip_start_ms: cs,
                            clip_end_ms: ce,
                            source_in,
                            media_in,
                            media_out,
                            clip_speed,
                        });
                        self.engine_source_rev = rev;
                        self.engine_note = None;
                        self.playing = true;
                        self.frame_gate = f64::NEG_INFINITY;
                        self.status = "播放中(直解码)".into();
                    }
                    Some(Err(e)) => self.fallback_to_slideshow(Some(format!("加载失败:{e}")), cx),
                    None => self.fallback_to_slideshow(Some("引擎未初始化".into()), cx),
                }
            }
            // 图片/文本片段/空窗本就无流 → 幻灯片路径,不提示错误
            None => self.fallback_to_slideshow(None, cx),
        }
        cx.notify();
    }

    /// 播放头所在片段 → 直解码参数(纯解析在 source_map,注入工程根)。
    fn resolve_direct_source(&self, t: u64) -> Option<DirectSource> {
        let snap = self.shared.snapshot();
        resolve_direct_source(&snap, std::path::Path::new(&self.project_dir), t)
    }

    /// 降级链终点:回落既有幻灯片模式(引擎不可用不白屏不 panic)。
    /// reason = None 是正常路径切换(图片/文本片段无流),不落错误横幅。
    fn fallback_to_slideshow(&mut self, reason: Option<String>, cx: &mut Context<Self>) {
        self.engine_source = None;
        self.zone_loading = None;
        self.resync_at = None;
        if let Some(r) = &reason {
            play_log(&format!("回落幻灯片:{r}"));
            self.engine_note = Some(r.clone());
            self.status = format!("已回落幻灯片模式:{r}");
        } else {
            play_log("回落幻灯片(非视频段,正常路径)");
            self.status = "播放中(单帧幻灯片模式)".into();
        }
        self.playing = true;
        self.last_frame = Instant::now();
        let at = self
            .playhead_ms
            .min(self.duration_ms.saturating_sub(100).max(self.playhead_ms));
        *self.shared.preview_request.lock().unwrap() = Some(at);
        cx.notify();
    }

    /// 弃引擎源(停流回 Idle;幻灯片静帧接管反映最新工程)。
    pub(crate) fn drop_engine_source(&mut self, cx: &mut Context<Self>) {
        self.engine_source = None;
        self.resync_at = None;
        if let Some(e) = self.engine.as_mut() {
            e.shutdown();
        }
        let at = self
            .playhead_ms
            .min(self.duration_ms.saturating_sub(100).max(self.playhead_ms));
        *self.shared.preview_request.lock().unwrap() = Some(at);
        cx.notify();
    }

    /// 引擎惰性构造(ffmpeg/ffprobe 路径:CUTFORGE_FFMPEG / CUTFORGE_FFPROBE,
    /// 缺省 "ffmpeg" / "ffprobe",与内核 bundle 口径一致)。
    fn ensure_engine(&mut self) -> Result<(), String> {
        if self.engine.is_none() {
            let ffmpeg = std::env::var("CUTFORGE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
            let ffprobe = std::env::var("CUTFORGE_FFPROBE").unwrap_or_else(|_| "ffprobe".into());
            let engine = PlaybackEngine::new(PathBuf::from(ffmpeg), PathBuf::from(ffprobe))
                .map_err(|e| e.to_string())?;
            self.engine = Some(engine);
        }
        Ok(())
    }

    fn pause_playback(&mut self, cx: &mut Context<Self>) {
        self.playing = false;
        if let Some(e) = self.engine.as_mut() {
            e.pause();
        }
        self.status = "已暂停".into();
        cx.notify();
    }

    /// 起播:精确画质 → 幻灯片;流畅 → zone 复用 → zone 请求 → 直解码。
    fn start_play(&mut self, cx: &mut Context<Self>) {
        if self.duration_ms == 0 {
            self.status = "空时间线,没有可播放内容".into();
            return;
        }
        if self.playhead_ms >= self.duration_ms {
            self.playhead_ms = 0;
        }
        if self.quality_precise {
            // 精确:引擎不参与,幻灯片逐帧
            self.engine_source = None;
            self.zone_loading = None;
            if let Some(e) = self.engine.as_mut() {
                e.shutdown();
            }
            self.playing = true;
            self.last_frame = Instant::now();
            self.status = "播放中(单帧精确模式)".into();
            return;
        }
        if let Err(e) = self.ensure_engine() {
            self.fallback_to_slideshow(Some(e), cx);
            return;
        }
        // zone 复用:rev/marker 未变且播放头仍在区间内 → 免重渲直载(原地重播)
        let rev = self.shared.snapshot_rev();
        let reuse = self.zone_ready.as_ref().is_some_and(|z| {
            z.rev == rev
                && self.playhead_ms >= z.start_ms
                && self.playhead_ms < z.end_ms
                && zone_cache::is_fresh(
                    &zone_cache::preview_cache_dir(std::path::Path::new(&self.project_dir)),
                    &z.key,
                )
                && z.file.exists()
        });
        if reuse {
            self.play_zone_ready(cx);
            return;
        }
        // M2:后台请求 zone,发起即角标,返回即切换
        if self.request_zone(cx) {
            self.playing = true; // 等渲染完成;引擎泵收结果后续播
            self.last_frame = Instant::now();
            self.status = "预览渲染中,完成后自动播放".into();
            return;
        }
        // 区间过短/无视频片段 → M1 直解码(再回落幻灯片)
        self.start_direct_play(cx);
    }

    /// 发起 preview_zone_render([播放头, +8s] 钳内容长度;100ms 量化在内核)。
    /// 同步工具无 runId —— 发起即角标,返回即切换。
    fn request_zone(&mut self, cx: &mut Context<Self>) -> bool {
        let snap = self.shared.snapshot();
        let has_video = snap.clips.iter().any(|c| {
            c.get("trackKind").and_then(serde_json::Value::as_str) == Some("video")
                && c.get("src")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|s| !s.is_empty())
        });
        if !has_video {
            return false;
        }
        let rev = self.shared.snapshot_rev();
        let start = (self.playhead_ms / 100 * 100).min(self.duration_ms.saturating_sub(100));
        let end =
            ((self.playhead_ms + ZONE_SPAN_MS).min(self.duration_ms) / 100 * 100).max(start + 100);
        if end <= start {
            return false;
        }
        self.zone_loading = Some((start, end, rev));
        play_log(&format!(
            "zone 请求 [{start}, {end}) rev={rev}(同步工具,无 runId;完成即切换)"
        ));
        let rpc = self.rpc.clone();
        let shared = self.shared.clone();
        cx.background_executor()
            .spawn(async move {
                let res = rpc
                    .call(
                        "preview_zone_render",
                        serde_json::json!({ "startMs": start, "endMs": end }),
                        crate::rpc::render_timeout("preview_zone_render"),
                    )
                    .and_then(|data| parse_zone_data(&data, start, end, rev));
                *shared.zone_result.lock().unwrap() = Some(res);
            })
            .detach();
        true
    }

    /// 播放中编辑重同步(300ms 防抖后调用):重解析当前源,变了才重启流。
    fn resync_engine(&mut self, cx: &mut Context<Self>) {
        let rev = self.shared.snapshot_rev();
        let Some(src) = self.engine_source.clone() else {
            return;
        };
        match src {
            EngineSource::Zone { .. } => {
                // zone 内容寻址随提交失效 → 切直解码续播(下轮播放重新请求 zone)
                self.zone_ready = None;
                self.start_direct_play(cx);
            }
            EngineSource::Direct { .. } => {
                let t = self.playhead_ms;
                let fresh = self.resolve_direct_source(t);
                let changed = match (&fresh, &self.engine_source) {
                    (Some(ds), Some(cur)) => !ds.matches_engine(cur),
                    (None, Some(EngineSource::Direct { .. })) => true,
                    _ => false,
                };
                if !changed {
                    self.engine_source_rev = rev;
                    return;
                }
                play_log(&format!("编辑重同步:片段参数变化,重启流 t={t}"));
                match fresh {
                    Some(_) => self.start_direct_play(cx),
                    None => self.fallback_to_slideshow(None, cx),
                }
            }
        }
    }
}
