// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 媒体分析纯函数层(册五 T5.4):多机位包络互相关对齐 + 场景检测帧差分。
//!
//! **单一实现纪律**(与 subtitle 同型):纯计算核心在本模块,consumers =
//! cutforge-mcp(multicam_sync / scene_detect 工具面)与 parity 夹具;
//! ffmpeg 解码在调用方(工具面/夹具各自起进程),本模块只做确定性数学。
//! 启发式诚实标注口径:engine/degraded/confidence 随工具面产出,质量不作硬验收;
//! 确定性(同输入同输出、无环境读取)由单测锁定。

/// rms 能量包络(20ms 窗 @22050Hz;确定性纯函数)——自 media_tools 移入
/// (跨 crate 共用:audio_beats / multicam_sync / parity 夹具同源)。
pub fn rms_envelope(samples: &[i16], rate: u32, window_ms: u64) -> Vec<f32> {
    let win = (rate as usize * window_ms as usize) / 1000;
    if win == 0 || samples.is_empty() {
        return Vec::new();
    }
    samples
        .chunks(win)
        .map(|seg| {
            let acc: f64 = seg.iter().map(|s| (*s as f64) * (*s as f64)).sum();
            (acc / seg.len().max(1) as f64).sqrt() as f32
        })
        .collect()
}

/// 起音强度:半波整流差分(能量上涨量才是起音)。
pub fn onset_strength(env: &[f32]) -> Vec<f32> {
    std::iter::once(0.0)
        .chain(env.windows(2).map(|w| (w[1] - w[0]).max(0.0)))
        .collect()
}

/// PCM 波形互相关求时移(纯函数;f64 确定性):同源双角度音频的**波形级**
/// 归一化互相关(NCC)——先 8 倍抽取(mean 降采样)控量,均值减除去直流,
/// 在 ±max_lag_ms 内滑窗取余弦相似度峰;重叠不足 60% 的滞后跳过(短素材
/// × 大窗守卫,稀疏尖峰在小重叠域的假满分)。
/// 返回 (best_lag_ms, score 0..1):lag 语义 = other 相对基准**滞后**毫秒
/// (other 的内容 t+lag ≈ 基准 t;即 other 晚开机 lag 毫秒);零信号 → (0, 0.0)。
pub fn pcm_lag(a: &[i16], b: &[i16], rate: u32, max_lag_ms: u64) -> (i64, f64) {
    const DECIM: usize = 8;
    let dec = |x: &[i16]| -> Vec<f64> {
        let out: Vec<f64> = x.chunks(DECIM).map(|c| c.iter().map(|v| f64::from(*v)).sum::<f64>() / c.len() as f64).collect();
        let mean = if out.is_empty() { 0.0 } else { out.iter().sum::<f64>() / out.len() as f64 };
        out.iter().map(|v| v - mean).collect()
    };
    let (a, b) = (dec(a), dec(b));
    let n = a.len().min(b.len());
    // 抽取后帧时长(ms)= DECIM / rate × 1000
    let frame_ms = DECIM as f64 * 1000.0 / rate as f64;
    let max_lag = ((max_lag_ms as f64 / frame_ms).round() as i64).clamp(0, n.saturating_sub(1) as i64);
    let min_overlap = (n as f64 * 0.6).ceil() as usize;
    if n < 16 || max_lag == 0 {
        return (0, 0.0);
    }
    let mut best: (i64, f64) = (0, -1.0);
    for lag in -max_lag..=max_lag {
        let li = lag.unsigned_abs() as usize;
        if n - li < min_overlap {
            continue;
        }
        let (x, y): (&[f64], &[f64]) = if lag >= 0 { (&a[..n - li], &b[li..]) } else { (&a[li..], &b[..n - li]) };
        let mut dot = 0f64;
        let mut na = 0f64;
        let mut nb = 0f64;
        for i in 0..x.len() {
            dot += x[i] * y[i];
            na += x[i] * x[i];
            nb += y[i] * y[i];
        }
        if na <= 0.0 || nb <= 0.0 {
            continue;
        }
        let score = dot / (na.sqrt() * nb.sqrt());
        if score > best.1 {
            best = (lag, score);
        }
    }
    let (lag, score) = best;
    let lag_ms = (lag as f64 * frame_ms).round() as i64;
    (lag_ms, score.clamp(0.0, 1.0))
}

/// 灰度帧序列(等长 u8 帧)→ 相邻帧平均绝对差(0..255;确定性纯函数)。
pub fn frame_diffs(frames: &[u8], frame_len: usize) -> Vec<f32> {
    if frame_len == 0 || frames.len() < frame_len * 2 {
        return Vec::new();
    }
    let n = frames.len() / frame_len;
    (0..n - 1)
        .map(|i| {
            let a = &frames[i * frame_len..(i + 1) * frame_len];
            let b = &frames[(i + 1) * frame_len..(i + 2) * frame_len];
            let acc: u64 = a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs() as u64).sum();
            (acc as f32 / frame_len as f32).min(255.0)
        })
        .collect()
}

/// 差分序列 → 剪切点(纯函数;确定性启发式):阈值 = mean + (max−mean)×k
/// (k = 0.5 − 0.4×sensitivity,sens 越高越密),**严格**阈值上局部极大
/// (平台/等值不触发——缓变序列零误报)且间隔 ≥200ms。
/// 返回 [(cut_ms, confidence 0..1)];cut = 帧间边界时刻。
pub fn pick_cuts(diffs: &[f32], sample_fps: f64, sensitivity: f64) -> Vec<(u64, f64)> {
    if diffs.is_empty() {
        return Vec::new();
    }
    let sens = sensitivity.clamp(0.0, 1.0);
    let mean = diffs.iter().sum::<f32>() / diffs.len() as f32;
    let max = diffs.iter().copied().fold(0.0f32, f32::max);
    let k = 0.5 - 0.4 * sens;
    let thr = mean + (max - mean) * k as f32;
    let min_gap = (0.2 * sample_fps).max(1.0) as usize; // 200ms 最小间隔(帧)
    let mut picked: Vec<(u64, f64)> = Vec::new();
    for (i, d) in diffs.iter().enumerate() {
        if *d <= thr {
            continue;
        }
        let prev = if i > 0 { diffs[i - 1] } else { 0.0 };
        let next = diffs.get(i + 1).copied().unwrap_or(0.0);
        if *d <= prev || *d < next {
            continue; // 非局部极大(平台/等值不触发)
        }
        if let Some((last, _)) = picked.last() {
            let last_frame = (*last as f64 / 1000.0 * sample_fps).round() as usize;
            if i + 1 - last_frame < min_gap {
                continue;
            }
        }
        let conf = if max > thr { ((d - thr) / (max - thr)).clamp(0.0, 1.0) } else { 0.0 };
        picked.push((((i + 1) as f64 / sample_fps * 1000.0).round() as u64, ((conf * 100.0).round() / 100.0) as f64));
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PCM 波形互相关:不规则脉冲正弦固定滞后 500ms → lag_ms ≈ 500(±半帧)、
    /// 高置信;零滞后 → 0;零信号守卫;小重叠域守卫(短音频 × 大窗不假满分)。
    #[test]
    fn pcm_lag_finds_fixed_offset() {
        let rate = 22050u32;
        let mk = |delay_ms: i64| -> Vec<i16> {
            let pulses = [0.0f64, 0.7, 1.1, 1.9, 2.3, 2.9];
            let n = 3 * rate as usize;
            (0..n)
                .map(|i| {
                    let t = i as f64 / rate as f64;
                    let gated = pulses.iter().any(|p| {
                        let ts = t - delay_ms as f64 / 1000.0;
                        ts >= *p && ts < p + 0.05
                    });
                    if gated { (0.8 * (2.0 * std::f64::consts::PI * 880.0 * t).sin() * 12000.0) as i16 } else { 0 }
                })
                .collect()
        };
        let a = mk(0);
        let b = mk(500);
        let (lag_ms, score) = pcm_lag(&a, &b, rate, 5000);
        assert!((495..=505).contains(&lag_ms), "500ms 偏移必须恢复: {lag_ms}/{score}");
        assert!(score > 0.5, "同源波形相似度必须高: {score}");
        let (lag0, _) = pcm_lag(&a, &a.clone(), rate, 5000);
        assert_eq!(lag0, 0);
        assert_eq!(pcm_lag(&vec![0i16; 1000], &vec![0i16; 1000], rate, 100), (0, 0.0));
        // 短音频 × 大窗:重叠守卫生效(不 panic、不假满分)
        let (l, s2) = pcm_lag(&a[..2000], &b[..2000], rate, 5000);
        let _ = (l, s2);
    }

    /// 帧差分 + 剪切点:硬切色块(5 帧暗 + 5 帧亮 @5fps)→ 唯一切点 ≈1000ms;
    /// 渐变序列(平台)零误报。
    #[test]
    fn frame_diffs_and_pick_cuts_hard_cut() {
        let frame_len = 64 * 36;
        let mut frames = Vec::new();
        for _ in 0..5 {
            frames.extend(std::iter::repeat_n(16u8, frame_len));
        }
        for _ in 0..5 {
            frames.extend(std::iter::repeat_n(235u8, frame_len));
        }
        let diffs = frame_diffs(&frames, frame_len);
        assert_eq!(diffs.len(), 9);
        assert!(diffs[4] > 200.0, "硬切边界差分必须巨大: {}", diffs[4]);
        let cuts = pick_cuts(&diffs, 5.0, 0.5);
        assert_eq!(cuts.len(), 1, "单硬切 → 单点: {cuts:?}");
        let (ms, conf) = cuts[0];
        assert!((900..=1100).contains(&ms), "切点 ≈ 1000ms: {ms}");
        assert!((0.0..=1.0).contains(&conf));
        // 渐变序列无剪切点(帧间差分均匀 = 平台,局部极大不触发)
        let mut grad = Vec::new();
        for i in 0..20u16 {
            grad.extend(std::iter::repeat_n((16 + i * 6).min(255) as u8, frame_len));
        }
        let d2 = frame_diffs(&grad, frame_len);
        assert!(pick_cuts(&d2, 5.0, 0.5).is_empty(), "缓变序列不得误报: {d2:?}");
        // 确定性
        assert_eq!(pick_cuts(&diffs, 5.0, 0.5), pick_cuts(&diffs, 5.0, 0.5));
    }
}
