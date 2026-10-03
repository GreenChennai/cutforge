// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 媒体池/音频工具后端(册四 A4 T4.1/T4.8,免开工作区:派生物缓存与纯计算
//! 不改工程 IR,不持排他锁):
//! - `media_peaks`:波形 peaks(ffmpeg 解单声道 PCM → 多级分辨率 min/max 桶,
//!   JSON 落 `.cutforge/peaks-cache/`,内容寻址 = 路径+mtime+size+桶数档);
//! - `media_thumbnail`:抽帧缩略图(ffmpeg 单帧 → PNG 落 `.cutforge/thumb-cache/`,
//!   内容寻址含 atMs+宽度;时间线波形绘制是 FE 活,本工具只供数据);
//! - `media_proxy`:1/2 分辨率代理(`.cutforge/proxy/`,内容寻址;渲染端
//!   useProxy 经 plan::swap_to_proxies 消费同一目录契约——显式 opt-in 不悄悄降质);
//! - `audio_beats`:节拍检测(吸收旧版 rs_beat 的 onset-energy 降级档算法:
//!   RMS 能量包络 → 半波整流差分 → 自适应阈值局部极大 → 自相关 BPM + 网格相位;
//!   启发式诚实标注,置信度随产出;落盘与吸附是 FE 活)。
//!
//! 纯计算核心(rms 包络/onset/BPM/网格/peaks 桶分)在本模块内为纯函数,
//! 可不装 ffmpeg 单测;ffmpeg/ffprobe 缺失 → DEP_MISSING(协议码如实)。

use crate::dispatch::resolve_within_root;
use crate::registry::envelope;
use serde_json::{Value, json};
use std::path::Path;

/// ffmpeg 定位(E5-2 同口径:env CUTFORGE_FFMPEG 优先,缺省按 PATH 名;
/// 册五 T5.4 起 multicam/scene 工具共用,crate 内 pub(crate))。
pub(crate) fn ff_bin() -> String {
    if let Some(v) = std::env::var_os("CUTFORGE_FFMPEG")
        && !v.is_empty()
    {
        return v.to_string_lossy().into_owned();
    }
    "ffmpeg".into()
}

pub(crate) fn ffmpeg_available() -> bool {
    std::process::Command::new(ff_bin())
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// peaks 桶数档位(coarse/standard/fine)。
fn peaks_buckets(level: &str) -> Option<u32> {
    match level {
        "coarse" => Some(1000),
        "standard" => Some(2000),
        "fine" => Some(4000),
        _ => None,
    }
}

// ---------------- 波形 peaks(T4.8-7) ----------------

/// PCM s16le 单声道 → N 桶 min/max(峰值归一到 [-1,1] 的 f64;确定性:桶边界
/// 固定切分,尾样本并入末桶)。
pub fn pcm_to_peaks(samples: &[i16], buckets: u32) -> Vec<(f32, f32)> {
    let n = buckets.max(1) as usize;
    if samples.is_empty() {
        return vec![(0.0, 0.0); n];
    }
    let per = samples.len().div_ceil(n);
    let mut out = Vec::with_capacity(n);
    for b in 0..n {
        // 界守卫:短音频(len < buckets)时 per=1,b>=len 的轮次起点越界,
        // 钳到样本末尾即空桶(与既有空桶分支同语义:恒零峰)。
        let start = (b * per).min(samples.len());
        let end = ((b + 1) * per).min(samples.len());
        if start >= end {
            out.push((0.0, 0.0));
            continue;
        }
        let slice = &samples[start..end];
        if slice.is_empty() {
            out.push((0.0, 0.0));
            continue;
        }
        let mut mn = i16::MAX;
        let mut mx = i16::MIN;
        for s in slice {
            mn = mn.min(*s);
            mx = mx.max(*s);
        }
        out.push((mn as f32 / 32768.0, mx as f32 / 32768.0));
    }
    out
}

/// media_peaks 工具面:素材路径 + 分辨率档 → peaks 文件(缓存命中零 ffmpeg)。
pub fn media_peaks_tool(root: &Path, args: &Value) -> Value {
    let Some(src) = args["src"].as_str() else {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            "缺 src(工程内相对路径)",
            json!({}),
        );
    };
    let level = args["level"].as_str().unwrap_or("standard");
    let Some(buckets) = peaks_buckets(level) else {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            &format!("未知 level: {level}(允许 coarse|standard|fine)"),
            json!({}),
        );
    };
    let abs = match resolve_within_root(root, src) {
        Ok(p) => p,
        Err(msg) => {
            return envelope(
                false,
                "PRECONDITION_FAILED",
                &format!("路径不合法({src}): {msg}"),
                json!({}),
            );
        }
    };
    let Some((mtime, size)) = cutforge_io::mediacache::source_stamp(&abs) else {
        return envelope(false, "NO_CONFIG", &format!("素材不可读: {src}"), json!({}));
    };
    let rel = cutforge_io::mediacache::peaks_rel(src, mtime, size, buckets);
    let out = root.join(&rel);
    if out.is_file() {
        return peaks_hit(root, src, level, buckets, &rel, true);
    }
    if !ffmpeg_available() {
        return envelope(
            false,
            "DEP_MISSING",
            "ffmpeg 不可用(安装 ffmpeg 或设 CUTFORGE_FFMPEG)",
            json!({}),
        );
    }
    // 解单声道 8kHz s16le PCM 到内存(短素材口径;peaks 是预览数据,非归档)
    let dec = std::process::Command::new(ff_bin())
        .args([
            "-v",
            "error",
            "-i",
            &abs.to_string_lossy(),
            "-vn",
            "-ac",
            "1",
            "-ar",
            "8000",
            "-f",
            "s16le",
            "-",
        ])
        .output();
    let Ok(dec) = dec else {
        return envelope(false, "DEP_MISSING", "ffmpeg 启动失败", json!({}));
    };
    if !dec.status.success() {
        return envelope(
            false,
            "DEP_MISSING",
            &format!(
                "PCM 解码失败: {}",
                String::from_utf8_lossy(&dec.stderr)
                    .chars()
                    .take(200)
                    .collect::<String>()
            ),
            json!({}),
        );
    }
    let mut samples: Vec<i16> = Vec::with_capacity(dec.stdout.len() / 2);
    for chunk in dec.stdout.as_chunks::<2>().0 {
        samples.push(i16::from_le_bytes(*chunk));
    }
    if samples.is_empty() {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            &format!("素材无音频流: {src}"),
            json!({}),
        );
    }
    let peaks = pcm_to_peaks(&samples, buckets);
    let doc = json!({
        "version": 1,
        "src": src,
        "level": level,
        "buckets": buckets,
        "sampleRate": 8000,
        "durationMs": (samples.len() as u64) * 1000 / 8000,
        "min": peaks.iter().map(|p| p.0).collect::<Vec<_>>(),
        "max": peaks.iter().map(|p| p.1).collect::<Vec<_>>(),
    });
    if let Err(e) = cutforge_io::atomic::atomic_write(&out, doc.to_string().as_bytes()) {
        return envelope(
            false,
            "INTERNAL",
            &format!("peaks 落盘失败: {e}"),
            json!({}),
        );
    }
    peaks_hit(root, src, level, buckets, &rel, false)
}

fn peaks_hit(_root: &Path, src: &str, level: &str, buckets: u32, rel: &str, hit: bool) -> Value {
    envelope(
        true,
        "OK",
        if hit {
            "peaks 缓存命中"
        } else {
            "peaks 已生成"
        },
        json!({
            "src": src, "level": level, "buckets": buckets,
            "cached": hit, "file": rel,
            "media": rel,
            "hint": "波形绘制消费:GET /media?path=<file>;FE 拿 min/max 数组画多级分辨率波形",
        }),
    )
}

// ---------------- 缩略图(T4.1-12) ----------------

/// media_thumbnail 工具面:素材路径(+atMs/width)→ PNG 缩略图(缓存命中零 ffmpeg)。
pub fn media_thumbnail_tool(root: &Path, args: &Value) -> Value {
    let Some(src) = args["src"].as_str() else {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            "缺 src(工程内相对路径)",
            json!({}),
        );
    };
    let width = args["width"].as_u64().unwrap_or(320).clamp(64, 1280) as u32;
    let abs = match resolve_within_root(root, src) {
        Ok(p) => p,
        Err(msg) => {
            return envelope(
                false,
                "PRECONDITION_FAILED",
                &format!("路径不合法({src}): {msg}"),
                json!({}),
            );
        }
    };
    let Some((mtime, size)) = cutforge_io::mediacache::source_stamp(&abs) else {
        return envelope(false, "NO_CONFIG", &format!("素材不可读: {src}"), json!({}));
    };
    // atMs 缺省 = 时长 10%(ffprobe 可用时);显式 0 合法(首帧)
    let at_ms = match args["atMs"].as_u64() {
        Some(v) => v,
        None => {
            let dur = cutforge_io::probe::ffprobe_available()
                .then(|| cutforge_io::probe::probe(&abs).ok())
                .flatten()
                .map(|i| i.duration_ms())
                .unwrap_or(0);
            dur / 10
        }
    };
    let rel = cutforge_io::mediacache::thumb_rel(src, mtime, size, at_ms, width);
    let out = root.join(&rel);
    if out.is_file() {
        return thumb_hit(src, at_ms, width, &rel, true);
    }
    if !ffmpeg_available() {
        return envelope(
            false,
            "DEP_MISSING",
            "ffmpeg 不可用(安装 ffmpeg 或设 CUTFORGE_FFMPEG)",
            json!({}),
        );
    }
    if let Some(dir) = out.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let r = std::process::Command::new(ff_bin())
        .args([
            "-y",
            "-v",
            "error",
            "-ss",
            &format!("{:.3}", at_ms as f64 / 1000.0),
            "-i",
            &abs.to_string_lossy(),
            "-frames:v",
            "1",
            "-vf",
            &format!("scale={width}:-2"),
            "-f",
            "image2",
            &out.to_string_lossy(),
        ])
        .output();
    match r {
        Ok(o) if o.status.success() && out.is_file() => thumb_hit(src, at_ms, width, &rel, false),
        Ok(o) => envelope(
            false,
            "DEP_MISSING",
            &format!(
                "抽帧失败(素材无视频流或 atMs 越界): {}",
                String::from_utf8_lossy(&o.stderr)
                    .chars()
                    .take(200)
                    .collect::<String>()
            ),
            json!({}),
        ),
        Err(e) => envelope(
            false,
            "DEP_MISSING",
            &format!("ffmpeg 启动失败: {e}"),
            json!({}),
        ),
    }
}

fn thumb_hit(src: &str, at_ms: u64, width: u32, rel: &str, hit: bool) -> Value {
    envelope(
        true,
        "OK",
        if hit {
            "缩略图缓存命中"
        } else {
            "缩略图已生成"
        },
        json!({
            "src": src, "atMs": at_ms, "width": width,
            "cached": hit, "file": rel, "media": rel, "format": "png",
        }),
    )
}

// ---------------- 代理工作流(T4.1-13) ----------------

/// media_proxy 工具面:查询/生成 1/2 分辨率代理(generate 缺省 true;
/// 内容寻址 = 路径+mtime+size,改素材即 miss;渲染端 useProxy 消费同一目录)。
pub fn media_proxy_tool(root: &Path, args: &Value) -> Value {
    let Some(src) = args["src"].as_str() else {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            "缺 src(工程内相对路径)",
            json!({}),
        );
    };
    let generate = args["generate"].as_bool().unwrap_or(true);
    let abs = match resolve_within_root(root, src) {
        Ok(p) => p,
        Err(msg) => {
            return envelope(
                false,
                "PRECONDITION_FAILED",
                &format!("路径不合法({src}): {msg}"),
                json!({}),
            );
        }
    };
    let Some((mtime, size)) = cutforge_io::mediacache::source_stamp(&abs) else {
        return envelope(false, "NO_CONFIG", &format!("素材不可读: {src}"), json!({}));
    };
    let rel = cutforge_io::mediacache::proxy_rel(src, mtime, size);
    let out = root.join(&rel);
    if out.is_file() {
        return proxy_hit(src, &rel, "ready", true);
    }
    if !generate {
        return envelope(
            true,
            "OK",
            "代理未生成",
            json!({
                "src": src, "state": "missing", "file": rel, "cached": false,
                "hint": "generate=true 生成;.cutforge/proxy/ 为渲染 useProxy 的消费目录",
            }),
        );
    }
    if !ffmpeg_available() {
        return envelope(
            false,
            "DEP_MISSING",
            "ffmpeg 不可用(安装 ffmpeg 或设 CUTFORGE_FFMPEG)",
            json!({}),
        );
    }
    // 半分辨率代理(偶数宽高;veryfast + crf 28 预览档;音轨重编 aac 保证可寻址时长)
    let probe = cutforge_io::probe::ffprobe_available()
        .then(|| cutforge_io::probe::probe(&abs).ok())
        .flatten();
    let (w, h) = match probe.as_ref().and_then(|i| i.video_size()) {
        Some((w, h)) => ((w / 2).max(16) & !1, (h / 2).max(16) & !1),
        None => (0, 0), // 无视频流(纯音频)→ 不缩放,仅转码压缩
    };
    if let Some(dir) = out.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let scale = if w > 0 {
        format!("-vf,scale={w}:{h}")
    } else {
        String::new()
    };
    let mut cmd = std::process::Command::new(ff_bin());
    cmd.args(["-y", "-v", "error", "-i", &abs.to_string_lossy()]);
    if w > 0 {
        cmd.args(["-vf", &format!("scale={w}:{h}")]);
    }
    let _ = scale;
    cmd.args([
        "-c:v", "libx264", "-preset", "veryfast", "-crf", "28", "-c:a", "aac", "-b:a", "96k",
        "-pix_fmt", "yuv420p",
    ]);
    cmd.arg(&out);
    match cmd.output() {
        Ok(o) if o.status.success() && out.is_file() => proxy_hit(src, &rel, "ready", false),
        Ok(o) => envelope(
            false,
            "DEP_MISSING",
            &format!(
                "代理生成失败: {}",
                String::from_utf8_lossy(&o.stderr)
                    .chars()
                    .take(200)
                    .collect::<String>()
            ),
            json!({}),
        ),
        Err(e) => envelope(
            false,
            "DEP_MISSING",
            &format!("ffmpeg 启动失败: {e}"),
            json!({}),
        ),
    }
}

fn proxy_hit(src: &str, rel: &str, state: &str, hit: bool) -> Value {
    envelope(
        true,
        "OK",
        if hit {
            "代理已存在(缓存命中)"
        } else {
            "代理已生成"
        },
        json!({
            "src": src, "state": state, "file": rel, "media": rel,
            "cached": hit, "scale": "1/2",
        }),
    )
}

// ---------------- 卡点检测(T4.8-10;启发式诚实标注) ----------------

/// onset-energy 检测参数(吸收 rs_beat 常量面;20ms 包络窗,60–180 BPM 域)。
pub const PCM_RATE: u32 = 22050;
pub const WINDOW_MS: u64 = 20;
pub const BPM_MIN: f64 = 60.0;
pub const BPM_MAX: f64 = 180.0;
pub const ONSET_MIN_GAP_MS: u64 = 200;

// rms 包络 / onset 强度纯函数(册五 T5.4 起单一实现移入 cutforge-render::analyze
// —— multicam_sync 与 parity 夹具同源;此处 1 参薄封装保持既有路径与签名逐字不变):
pub fn rms_envelope(samples: &[i16]) -> Vec<f32> {
    cutforge_render::analyze::rms_envelope(samples, PCM_RATE, WINDOW_MS)
}

pub use cutforge_render::analyze::onset_strength;

/// 起音候选:超过自适应阈值的局部极大值(阈值 = min(中位数×K, 峰值×0.5);
/// K 由灵敏度 0..1 派生:K = 4.0 − 3.0×sens,sens 越高越密)。
pub fn pick_onsets(strength: &[f32], sensitivity: f64) -> Vec<usize> {
    let sens = sensitivity.clamp(0.0, 1.0);
    let k = 4.0 - 3.0 * sens;
    let pos: Vec<f32> = strength.iter().copied().filter(|s| *s > 0.0).collect();
    if pos.is_empty() {
        return Vec::new();
    }
    let mut sorted = pos.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted[sorted.len() / 2];
    let peak = sorted[sorted.len() - 1];
    let thr = (median * k as f32).min(peak * 0.5);
    let gap_frames = (ONSET_MIN_GAP_MS / WINDOW_MS).max(1) as usize;
    let mut picked: Vec<usize> = Vec::new();
    for (i, s) in strength.iter().enumerate() {
        if *s < thr {
            continue;
        }
        let prev = if i > 0 { strength[i - 1] } else { 0.0 };
        let next = strength.get(i + 1).copied().unwrap_or(0.0);
        if *s < prev || *s < next {
            continue; // 非局部极大(等式并肩取先者)
        }
        if let Some(last) = picked.last()
            && i - last < gap_frames
        {
            if *s > strength[*last] {
                *picked.last_mut().unwrap() = i; // 间隔内更强者替换
            }
            continue;
        }
        picked.push(i);
    }
    picked
}

/// BPM 估计:起音强度自相关(60–180 BPM 对应滞后逐个打分取峰;倍频矫正)。
pub fn estimate_bpm(strength: &[f32]) -> f64 {
    let env_rate = 1000.0 / WINDOW_MS as f64;
    let lag_lo = (env_rate * 60.0 / BPM_MAX).max(2.0) as usize;
    let lag_hi = (strength.len().saturating_sub(1)).min((env_rate * 60.0 / BPM_MIN) as usize);
    if lag_hi <= lag_lo || strength.len() < lag_hi * 2 {
        return 0.0;
    }
    let mean = strength.iter().sum::<f32>() / strength.len() as f32;
    let dev: Vec<f32> = strength.iter().map(|s| s - mean).collect();
    let mut best_lag = 0usize;
    let mut best_score = 0f64;
    for lag in lag_lo..=lag_hi {
        let score: f64 = dev[..dev.len() - lag]
            .iter()
            .zip(&dev[lag..])
            .map(|(a, b)| (a * b) as f64)
            .sum();
        if score > best_score {
            best_lag = lag;
            best_score = score;
        }
    }
    if best_lag == 0 || best_score <= 0.0 {
        return 0.0;
    }
    let mut bpm = 60.0 * env_rate / best_lag as f64;
    while bpm < BPM_MIN {
        bpm *= 2.0;
    }
    while bpm > BPM_MAX {
        bpm /= 2.0;
    }
    (bpm * 10.0).round() / 10.0
}

/// 节拍网格:由 BPM 周期在起音点上做相位扫描(命中最多者胜;命中比 = 置信度)。
pub fn beat_grid(onsets_ms: &[u64], bpm: f64, total_ms: u64) -> (Vec<u64>, f64) {
    if bpm <= 0.0 || onsets_ms.is_empty() {
        return (Vec::new(), 0.0);
    }
    let period = 60000.0 / bpm;
    let mut best: Option<(f64, Vec<u64>)> = None;
    for o in onsets_ms.iter().take(16) {
        let phase = *o as f64 % period;
        let mut beats: Vec<u64> = Vec::new();
        let mut t = phase;
        while t < total_ms as f64 {
            beats.push(t.round() as u64);
            t += period;
        }
        let hits = onsets_ms
            .iter()
            .filter(|x| beats.iter().any(|b| (**x as i64 - *b as i64).abs() <= 60))
            .count();
        let conf = hits as f64 / onsets_ms.len() as f64;
        if best.as_ref().is_none_or(|(bc, _)| conf > *bc) {
            best = Some((conf, beats));
        }
    }
    match best {
        Some((conf, beats)) => (beats, (conf * 100.0).round() / 100.0),
        None => (Vec::new(), 0.0),
    }
}

/// audio_beats 工具面(纯计算,免开工作区):音频 + 灵敏度 → {bpm, beats, confidence};
/// engine=onset-energy,degraded=true(启发式诚实标注;检测质量不作硬验收)。
pub fn audio_beats_tool(root: &Path, args: &Value) -> Value {
    let Some(src) = args["src"].as_str() else {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            "缺 src(工程内相对路径)",
            json!({}),
        );
    };
    let sensitivity = args["sensitivity"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0);
    let abs = match resolve_within_root(root, src) {
        Ok(p) => p,
        Err(msg) => {
            return envelope(
                false,
                "PRECONDITION_FAILED",
                &format!("路径不合法({src}): {msg}"),
                json!({}),
            );
        }
    };
    if !cutforge_io::probe::ffprobe_available() && !ffmpeg_available() {
        return envelope(
            false,
            "DEP_MISSING",
            "ffmpeg 不可用(安装 ffmpeg 或设 CUTFORGE_FFMPEG)",
            json!({}),
        );
    }
    let dec = std::process::Command::new(ff_bin())
        .args([
            "-v",
            "error",
            "-i",
            &abs.to_string_lossy(),
            "-vn",
            "-ac",
            "1",
            "-ar",
            &PCM_RATE.to_string(),
            "-f",
            "s16le",
            "-",
        ])
        .output();
    let Ok(dec) = dec else {
        return envelope(false, "DEP_MISSING", "ffmpeg 启动失败", json!({}));
    };
    if !dec.status.success() {
        return envelope(
            false,
            "DEP_MISSING",
            &format!(
                "PCM 解码失败: {}",
                String::from_utf8_lossy(&dec.stderr)
                    .chars()
                    .take(200)
                    .collect::<String>()
            ),
            json!({}),
        );
    }
    let mut samples: Vec<i16> = Vec::with_capacity(dec.stdout.len() / 2);
    for chunk in dec.stdout.as_chunks::<2>().0 {
        samples.push(i16::from_le_bytes(*chunk));
    }
    let total_ms = (samples.len() as u64) * 1000 / PCM_RATE as u64;
    let strength = onset_strength(&rms_envelope(&samples));
    let onsets = pick_onsets(&strength, sensitivity);
    let onsets_ms: Vec<u64> = onsets.iter().map(|i| (*i as u64) * WINDOW_MS).collect();
    let bpm = estimate_bpm(&strength);
    let (beats, conf) = beat_grid(&onsets_ms, bpm, total_ms.max(1));
    envelope(
        true,
        "OK",
        "节拍检测完成(启发式;置信度如实)",
        json!({
            "src": src,
            "engine": "onset-energy",
            "degraded": true,
            "sensitivity": sensitivity,
            "bpm": bpm,
            "beatCount": beats.len(),
            "onsetCount": onsets_ms.len(),
            "confidence": conf,
            "durationMs": total_ms,
            "beats": beats,
            "onsets": onsets_ms,
            "hint": "纯计算不落盘不产 Op;标记落盘与素材卡点吸附是 FE 活(检测质量为启发式口径,不作硬验收)",
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, ms: u64) -> Vec<i16> {
        // 22050Hz 单声道正弦(确定性合成;点击轨 = 正弦 × 慢开关窗)
        let n = (PCM_RATE as u64 * ms / 1000) as usize;
        (0..n)
            .map(|i| {
                let v = (2.0 * std::f64::consts::PI * freq * i as f64 / PCM_RATE as f64).sin();
                (v * 12000.0) as i16
            })
            .collect()
    }

    #[test]
    fn pcm_to_peaks_buckets_and_normalizes() {
        let samples = sine(440.0, 1000); // 1s
        let peaks = pcm_to_peaks(&samples, 10);
        assert_eq!(peaks.len(), 10, "10 桶");
        for (mn, mx) in &peaks {
            assert!(mn <= mx);
            assert!(*mn >= -1.0 && *mx <= 1.0, "归一 [-1,1]: {mn}/{mx}");
        }
        // 正弦应有接近满幅的正负峰
        let mx = peaks.iter().map(|p| p.1).fold(0.0f32, f32::max);
        assert!(mx > 0.3, "正弦峰幅应可见: {mx}");
        assert_eq!(pcm_to_peaks(&[], 4), vec![(0.0, 0.0); 4], "空 PCM 恒零峰");
        // 确定性
        assert_eq!(pcm_to_peaks(&samples, 8), pcm_to_peaks(&samples, 8));
    }

    /// 短音频 × 高桶数:len < buckets 时 per=1,越界轮次不得 panic,
    /// 末尾桶恒零峰(CI ubuntu 实证边界缺陷的回归测试)。
    #[test]
    fn pcm_to_peaks_short_audio_more_buckets_than_samples() {
        let samples: Vec<i16> = vec![100, -200, 300, -400, 500]; // 5 样本
        let peaks = pcm_to_peaks(&samples, 1000);
        assert_eq!(peaks.len(), 1000, "桶数恒等于请求值");
        for (mn, mx) in &peaks {
            assert!(mn <= mx);
            assert!(*mn >= -1.0 && *mx <= 1.0);
        }
        // 前 5 桶各含 1 样本(per=1),峰值逐样本对应
        for (b, s) in samples.iter().enumerate() {
            let expect = *s as f32 / 32768.0;
            assert_eq!(peaks[b], (expect, expect), "桶 {b} 应为单样本峰");
        }
        // 末尾桶(样本耗尽)恒零峰
        for (mn, mx) in &peaks[5..] {
            assert_eq!(*mn, 0.0);
            assert_eq!(*mx, 0.0);
        }
        // 恰好 len == buckets 也应整除无越界
        let peaks = pcm_to_peaks(&samples, 5);
        assert_eq!(peaks.len(), 5);
        assert_eq!(peaks[0], (100.0 / 32768.0, 100.0 / 32768.0));
    }

    /// 空 PCM 既有路径保持:恒 n 个零峰(不动语义)。
    #[test]
    fn pcm_to_peaks_empty_samples_stays_flat() {
        assert_eq!(pcm_to_peaks(&[], 1), vec![(0.0, 0.0)]);
        assert_eq!(pcm_to_peaks(&[], 8), vec![(0.0, 0.0); 8]);
        assert_eq!(pcm_to_peaks(&[], 4000), vec![(0.0, 0.0); 4000]);
    }

    /// 正常路径抽查:桶数正确 + 已知波形的峰值正确性(极值桶逐点核对)。
    #[test]
    fn pcm_to_peaks_normal_path_bucket_and_peak_correctness() {
        // 9 样本分 3 桶:每桶 3 样本,极值可手算
        let samples: Vec<i16> = vec![1000, -2000, 500, -3000, 4000, 0, -100, 200, 6000];
        let peaks = pcm_to_peaks(&samples, 3);
        assert_eq!(peaks.len(), 3);
        assert_eq!(peaks[0], (-2000.0 / 32768.0, 1000.0 / 32768.0));
        assert_eq!(peaks[1], (-3000.0 / 32768.0, 4000.0 / 32768.0));
        assert_eq!(peaks[2], (-100.0 / 32768.0, 6000.0 / 32768.0));
        // 整除余数:7 样本分 3 桶(per=3,尾样本并入末桶)
        let samples: Vec<i16> = vec![1, -2, 3, -4, 5, -6, 7];
        let peaks = pcm_to_peaks(&samples, 3);
        assert_eq!(peaks.len(), 3);
        assert_eq!(peaks[0], (-2.0 / 32768.0, 3.0 / 32768.0));
        assert_eq!(peaks[1], (-6.0 / 32768.0, 5.0 / 32768.0));
        assert_eq!(peaks[2], (7.0 / 32768.0, 7.0 / 32768.0), "尾样本并入末桶");
        // 正弦:整体 min/max 应近满幅对极
        let samples = sine(440.0, 1000);
        let peaks = pcm_to_peaks(&samples, 50);
        assert_eq!(peaks.len(), 50);
        let mn = peaks.iter().map(|p| p.0).fold(0.0f32, f32::min);
        let mx = peaks.iter().map(|p| p.1).fold(0.0f32, f32::max);
        assert!(mn < -0.3, "正弦负峰应可见: {mn}");
        assert!(mx > 0.3, "正弦正峰应可见: {mx}");
    }

    /// 周期点击轨(每 500ms 一个 40ms 窄脉冲)→ 120 BPM 网格。
    #[test]
    fn beat_detection_finds_periodic_clicks() {
        // 4s:每 500ms 注入 40ms 强脉冲(背景静音)
        let mut samples = vec![0i16; PCM_RATE as usize * 4];
        for k in 0..8 {
            let start = PCM_RATE as usize / 2 * k;
            for i in 0..PCM_RATE as usize * 40 / 1000 {
                if start + i < samples.len() {
                    samples[start + i] =
                        ((2.0 * std::f64::consts::PI * 1000.0 * i as f64 / PCM_RATE as f64).sin()
                            * 15000.0) as i16;
                }
            }
        }
        let strength = onset_strength(&rms_envelope(&samples));
        let onsets = pick_onsets(&strength, 0.5);
        let onsets_ms: Vec<u64> = onsets.iter().map(|i| (*i as u64) * WINDOW_MS).collect();
        assert!(onsets_ms.len() >= 6, "8 个脉冲应检出多数: {onsets_ms:?}");
        let bpm = estimate_bpm(&strength);
        assert!(
            (bpm - 120.0).abs() <= 1.0,
            "500ms 周期 = 120 BPM,实得 {bpm}"
        );
        let (beats, conf) = beat_grid(&onsets_ms, bpm, 4000);
        assert!(!beats.is_empty(), "网格非空");
        assert!(conf >= 0.5, "周期脉冲置信度应高: {conf}");
        // 静音轨:BPM 0 / 空网格(诚实零)
        let strength = onset_strength(&rms_envelope(&vec![0i16; PCM_RATE as usize]));
        assert_eq!(estimate_bpm(&strength), 0.0);
        let (beats, conf) = beat_grid(&[], 120.0, 1000);
        assert!(beats.is_empty() && conf == 0.0);
    }

    #[test]
    fn onset_threshold_scales_with_sensitivity() {
        let env: Vec<f32> = (0..100)
            .map(|i| if i % 10 == 0 { 1.0 } else { 0.1 })
            .collect();
        let strength = onset_strength(&env);
        let loose = pick_onsets(&strength, 1.0);
        let strict = pick_onsets(&strength, 0.0);
        assert!(!loose.is_empty(), "高灵敏度应出点");
        assert!(loose.len() >= strict.len(), "灵敏度越高点越多");
    }

    /// AC-4.5 实测面:三个媒体工具对真实 ffmpeg 产物落盘(缺失 ffmpeg 即失败,
    /// 与 render_matrix 同口径;产物魔数/结构断言)。

    #[test]
    fn media_tools_real_pipeline() {
        assert!(ffmpeg_available(), "ffmpeg 必须存在(媒体工具实测面)");
        let root = std::env::temp_dir().join(format!("cf-media-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("01_原始素材")).unwrap();
        std::fs::create_dir_all(root.join("05_时间线工程")).unwrap();
        cutforge_io::atomic::atomic_write(
            &root.join("05_时间线工程/project.json"),
            serde_json::to_string_pretty(&json!({
                "version": 1, "schemaVersion": "2.0.0", "slug": "media", "fps": 30,
                "canvas": {"width": 320, "height": 240}, "tracks": []
            }))
            .unwrap()
            .as_bytes(),
        )
        .unwrap();
        let r = std::process::Command::new(ff_bin())
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x240:rate=30:duration=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-c:a",
                "aac",
                "-shortest",
                "01_原始素材/clip.mp4",
            ])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        let src = "01_原始素材/clip.mp4";

        // ① peaks:JSON 落盘 + 二次调用缓存命中
        let r1 = media_peaks_tool(&root, &json!({"src": src, "level": "standard"}));
        assert_eq!(r1["code"], json!("OK"), "{r1}");
        assert_eq!(r1["data"]["cached"], json!(false));
        let file = r1["data"]["file"].as_str().unwrap();
        let doc: Value = serde_json::from_slice(&std::fs::read(root.join(file)).unwrap()).unwrap();
        assert_eq!(doc["buckets"], json!(2000));
        assert_eq!(doc["min"].as_array().unwrap().len(), 2000);
        let mx = doc["max"]
            .as_array()
            .unwrap()
            .iter()
            .fold(0.0f64, |a, v| a.max(v.as_f64().unwrap()));
        assert!(mx > 0.1, "正弦峰应可见(AAC 域 8 桶采样,幅值有折损): {mx}");
        let r2 = media_peaks_tool(&root, &json!({"src": src, "level": "standard"}));
        assert_eq!(r2["data"]["cached"], json!(true), "二次调用必须缓存命中");

        // ② thumbnail:PNG 魔数 + 命中
        let r3 = media_thumbnail_tool(&root, &json!({"src": src, "atMs": 500}));
        assert_eq!(r3["code"], json!("OK"), "{r3}");
        let bytes = std::fs::read(root.join(r3["data"]["file"].as_str().unwrap())).unwrap();
        assert_eq!(
            &bytes[..8],
            b"\x89PNG\r\n\x1a\n",
            "缩略图必须是合法 PNG(魔数)"
        );
        let r4 = media_thumbnail_tool(&root, &json!({"src": src, "atMs": 500}));
        assert_eq!(r4["data"]["cached"], json!(true));

        // ③ proxy:mp4 落盘(1/2 分辨率)+ 渲染端 proxy_lookup 同目录契约可见
        let r5 = media_proxy_tool(&root, &json!({"src": src}));
        assert_eq!(r5["code"], json!("OK"), "{r5}");
        let proxy_file = r5["data"]["file"].as_str().unwrap().to_string();
        let bytes = std::fs::read(root.join(&proxy_file)).unwrap();
        assert_eq!(&bytes[4..8], b"ftyp", "代理必须是合法 MP4(ftyp 盒)");
        let lookup = cutforge_io::mediacache::proxy_lookup(&root, src).expect("渲染端查找必须命中");
        assert_eq!(lookup, proxy_file, "MCP 产物与渲染端查找同一内容寻址路径");
        let r6 = media_proxy_tool(&root, &json!({"src": src, "generate": false}));
        assert_eq!(r6["data"]["state"], json!("ready"));

        // ④ audio_beats:点击轨(4s × 每 500ms 一击)→ 120 BPM
        let mut pcm = vec![0i16; PCM_RATE as usize * 4];
        for k in 0..8 {
            let start = PCM_RATE as usize / 2 * k;
            for i in 0..PCM_RATE as usize * 40 / 1000 {
                if start + i < pcm.len() {
                    pcm[start + i] =
                        ((2.0 * std::f64::consts::PI * 1000.0 * i as f64 / PCM_RATE as f64).sin()
                            * 15000.0) as i16;
                }
            }
        }
        let mut raw = Vec::with_capacity(pcm.len() * 2);
        for s in &pcm {
            raw.extend_from_slice(&s.to_le_bytes());
        }
        let mut child = std::process::Command::new(ff_bin())
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "s16le",
                "-ar",
                "22050",
                "-ac",
                "1",
                "-i",
                "-",
                "-c:a",
                "aac",
                "01_原始素材/clicks.m4a",
            ])
            .current_dir(&root)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write as _;
        child.stdin.take().unwrap().write_all(&raw).unwrap();
        assert!(child.wait().unwrap().success(), "点击轨生成失败");
        let rb = audio_beats_tool(
            &root,
            &json!({"src": "01_原始素材/clicks.m4a", "sensitivity": 0.6}),
        );
        assert_eq!(rb["code"], json!("OK"), "{rb}");
        assert_eq!(rb["data"]["engine"], json!("onset-energy"));
        assert_eq!(rb["data"]["degraded"], json!(true), "启发式诚实标注");
        let bpm = rb["data"]["bpm"].as_f64().unwrap();
        assert!(
            (bpm - 120.0).abs() <= 1.0,
            "500ms 周期 = 120 BPM,实得 {bpm}"
        );
        assert!(rb["data"]["beatCount"].as_u64().unwrap() >= 6, "{rb}");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn beats_tool_guards_bad_input() {
        let root = std::env::temp_dir().join(format!("cf-beats-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // 缺 src
        let r = audio_beats_tool(&root, &json!({}));
        assert_eq!(r["code"], json!("PRECONDITION_FAILED"));
        // 越出工程根
        let r = audio_beats_tool(&root, &json!({"src": "../外.mp3"}));
        assert_eq!(r["code"], json!("PRECONDITION_FAILED"));
        // peaks/thumbnail/proxy 缺 src 同口径
        for name in ["peaks", "thumb", "proxy"] {
            let r = match name {
                "peaks" => media_peaks_tool(&root, &json!({})),
                "thumb" => media_thumbnail_tool(&root, &json!({})),
                _ => media_proxy_tool(&root, &json!({})),
            };
            assert_eq!(r["code"], json!("PRECONDITION_FAILED"), "{name}");
        }
        // peaks 未知 level
        let r = media_peaks_tool(&root, &json!({"src": "x", "level": "超细"}));
        assert_eq!(r["code"], json!("PRECONDITION_FAILED"));
        std::fs::remove_dir_all(&root).ok();
    }
}
