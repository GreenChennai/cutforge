// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! acrossfade 音频链(册四 A4 T4.5,消化 M11-R1):主时间线有视频转场时,
//! 视频片段的音频不再逐段绝对落点 adelay 硬拼,而是**镜像视频 xfade 链**——
//!
//! ```text
//! [clip0 音频+尾帧静音 pad]acrossfade(d=D1)[clip1 音频+尾帧 pad]acrossfade(d=D2)…
//!   → adelay(首片段 startMs) → [bus](与音频轨事件/BGM 同一混音面)
//! ```
//!
//! **窗口对齐**(BE3a 实测口径):视频 xfade 窗 = [Σ名义时长, Σ名义时长+D]
//! (offset=名义时长,ADR-0023);acrossfade 窗 = [d1−D, d1] = [Σdur, Σdur+D]
//! (流长 = dur+尾帧静音 pad)——两窗逐点重合,声画同步。
//!
//! **时长零漂移**(构造性保证):链总长 = Σ(dur_i+tail_i) − ΣD_i = Σdur_i
//! (tail_i = D_{i+1},末段尾帧 0);每路流 `apad=whole_dur` 钳到名义长,源不足
//! 补静音,不为时长引入随机性。音频的"尾帧" = 静音 pad(视觉对应冻结帧)。
//! 硬切边界(D=0)走 `concat`(v=0:a=1)等价直拼。
//!
//! 本模块不启动进程;纯函数生成 ffmpeg 参数,可离线单测。

use crate::plan::{AudioSeg, RenderPlan};
use cutforge_core::model::{Bgm, EqBand, TrackDyn};
use serde_json::Value;
use std::path::Path;

/// acrossfade 链模式开关:≥2 个主线视频段且至少一个边界有有效转场。
/// 关闭 = 既有逐段 adelay 总线路径(参数逐字一致,parity 红线)。
pub fn chain_active(plan: &RenderPlan) -> bool {
    plan.video_clips.len() >= 2 && plan.boundary_durs_ms.iter().any(|d| *d > 0.0)
}

/// 视频轨处理链(册五 T5.3,acrossfade 链模式):全部主线片段同属一条**有处理
/// 声明**的轨时返回其链,否则空串(跨轨声明无单点归属,诚实跳过)。
pub fn video_track_chain(plan: &RenderPlan) -> String {
    let Some(first) = plan.video_track_of.first() else { return String::new() };
    if plan.video_track_of.iter().any(|t| t != first) {
        return String::new();
    }
    let Some(p) = plan.track_proc.iter().find(|p| &p.track_id == first) else {
        return String::new();
    };
    if p.is_empty() {
        return String::new();
    }
    track_chain(p.eq.as_deref(), p.dyn_.as_ref())
}

/// ducking 侧链滤镜(册五 T5.3 参数化):BGM 字段缺省 = 既有常量
/// (threshold=0.03/ratio=8/attack=80/release=500),格式化去尾零——缺省时
/// 与既有字面串逐字一致(parity 红线)。
pub fn ducking_filter(bgm: &Bgm) -> String {
    format!(
        "sidechaincompress=threshold={}:ratio={}:attack={}:release={}",
        crate::steps::fmt_f64(bgm.duck_threshold()),
        crate::steps::fmt_f64(bgm.duck_ratio()),
        crate::steps::fmt_f64(bgm.duck_attack_ms()),
        crate::steps::fmt_f64(bgm.duck_release_ms()),
    )
}

/// 降噪档 → afftdn 滤镜(册四 T4.8):nr=降噪量(dB),nf=噪声底(dB),tn=频带跟踪。
/// off/未知值 → None(不降噪;诚实降级,不臆造档位)。
pub fn denoise_filter(d: &str) -> Option<&'static str> {
    match d {
        "low" => Some("afftdn=nr=8:nf=-25:tn=1"),
        "mid" => Some("afftdn=nr=15:nf=-35:tn=1"),
        "high" => Some("afftdn=nr=25:nf=-45:tn=1"),
        _ => None,
    }
}

/// 保速变调系数(册四 T4.8):半音数 → 速率倍率 2^(n/12);非有限/0 → 1.0(不变调)。
pub fn pitch_factor(semitones: f64) -> f64 {
    if !semitones.is_finite() || semitones == 0.0 {
        return 1.0;
    }
    2f64.powf(semitones / 12.0)
}

/// 单条 EQ 频段 → biquad 滤镜(册五 T5.3;本机 ffmpeg biquad 族全量在位,
/// firequalizer 的 FFT 卷积对短段有窗效应且版本差异大——选 biquad 链为稳)。
/// peaking → equalizer(f/t/w/g);shelf 忽略 q(ffmpeg shelf 无 Q 语义,坡宽固定)。
/// 增益 |0| 档跳过(中性段不产滤镜,链零变化)。
pub fn eq_band_filter(b: &EqBand) -> Option<String> {
    if b.gain.abs() < 1e-6 || !b.freq.is_finite() {
        return None;
    }
    let f = crate::steps::fmt_f64(b.freq.clamp(20.0, 20000.0));
    let g = crate::steps::fmt_f64(b.gain.clamp(-24.0, 24.0));
    match b.type_.as_str() {
        "lowshelf" => Some(format!("lowshelf=f={f}:g={g}")),
        "highshelf" => Some(format!("highshelf=f={f}:g={g}")),
        "peaking" => {
            let q = crate::steps::fmt_f64(b.q.unwrap_or(1.0).clamp(0.1, 16.0));
            Some(format!("equalizer=f={f}:width_type=q:w={q}:g={g}"))
        }
        _ => None, // 未知类型诚实跳过(schema 枚举封闭,防御面)
    }
}

/// 轨道 EQ 链(段序 = 数组序;空 = 空串)。
pub fn track_eq_chain(bands: &[EqBand]) -> String {
    bands.iter().filter_map(eq_band_filter).collect::<Vec<_>>().join(",")
}

/// 轨道动态链(册五 T5.3):acompressor(参数子集;threshold/limit IR dB → 线性域
/// 10^(dB/20))+ alimiter(限幅收尾)。未声明压缩/限幅的维度不产滤镜。
pub fn track_dyn_chain(d: &TrackDyn) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(th) = d.threshold_db
        && th.is_finite()
    {
        let lin = 10f64.powf(th.clamp(-60.0, 0.0) / 20.0);
        let ratio = crate::steps::fmt_f64(d.ratio.unwrap_or(4.0).clamp(1.0, 20.0));
        let attack = crate::steps::fmt_f64(d.attack_ms.unwrap_or(25.0).clamp(1.0, 500.0));
        let release = crate::steps::fmt_f64(d.release_ms.unwrap_or(250.0).clamp(5.0, 5000.0));
        parts.push(format!(
            "acompressor=threshold={lin:.6}:ratio={ratio}:attack={attack}:release={release}"
        ));
    }
    if let Some(lim) = d.limit_db
        && lim.is_finite()
    {
        let lin = 10f64.powf(lim.clamp(-24.0, 0.0) / 20.0);
        parts.push(format!("alimiter=limit={lin:.6}"));
    }
    parts.join(",")
}

/// 轨道处理链(册五 T5.3):EQ 全段 → 压缩 → 限幅(链内序固定;空 = 空串)。
pub fn track_chain(eq: Option<&[EqBand]>, dyn_: Option<&TrackDyn>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(bands) = eq
        && !track_eq_chain(bands).is_empty()
    {
        parts.push(track_eq_chain(bands));
    }
    if let Some(d) = dyn_
        && !track_dyn_chain(d).is_empty()
    {
        parts.push(track_dyn_chain(d));
    }
    parts.join(",")
}

/// 轨道组建流标签(册五 T5.3):把 (seg, 事件标签) 平行数组按轨道分组——
/// 有处理声明的轨 amix 建流 → per-track 链 → 单标签 `[tp…]`;无处理轨保持
/// 逐事件标签(既有图零变化)。返回 (追加滤镜片段, 总线标签序列)。
/// 追加次序 = 轨道首次出现序(确定性);track_id 缺失的事件诚实直通。
pub fn grouped_bus_labels(
    plan: &RenderPlan,
    segs: &[&AudioSeg],
    refs: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut parts: Vec<String> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    if plan.track_proc.is_empty() {
        labels.extend(refs.iter().cloned());
        return (parts, labels);
    }
    let mut done: Vec<&str> = Vec::new();
    for (i, s) in segs.iter().enumerate() {
        let Some(tid) = s.track_id.as_deref() else {
            labels.push(refs[i].clone());
            continue;
        };
        if done.contains(&tid) {
            continue;
        }
        done.push(tid);
        let idxs: Vec<usize> =
            (0..segs.len()).filter(|&j| segs[j].track_id.as_deref() == Some(tid)).collect();
        let proc = plan.track_proc.iter().find(|p| p.track_id == tid).filter(|p| !p.is_empty());
        match proc {
            Some(p) => {
                let chain = track_chain(p.eq.as_deref(), p.dyn_.as_ref());
                if chain.is_empty() {
                    // 处理声明存在但滤镜为空(中性段/未知类型全跳过)→ 按无处理直通
                    for j in idxs {
                        labels.push(refs[j].clone());
                    }
                    continue;
                }
                if idxs.len() == 1 {
                    let i0 = idxs[0];
                    // 标签后直接接滤镜名(标签 + 逗号是非法图式:"No such filter: ''")
                    parts.push(format!("{}{chain}[tp{i0}]", refs[i0]));
                    labels.push(format!("[tp{i0}]"));
                } else {
                    let grouped: String =
                        idxs.iter().map(|&j| refs[j].as_str()).collect::<Vec<_>>().join("");
                    parts.push(format!(
                        "{grouped}amix=inputs={}:duration=longest:normalize=0[tg{tid}]",
                        idxs.len()
                    ));
                    parts.push(format!("[tg{tid}]{chain}[tp{tid}]"));
                    labels.push(format!("[tp{tid}]"));
                }
            }
            None => {
                for j in idxs {
                    labels.push(refs[j].clone());
                }
            }
        }
    }
    (parts, labels)
}

/// 单事件音频链体(aformat 归一 → 降噪 → 倒放 → 变调 → 变速 → 音量 → 淡入淡出;
/// 不含 adelay 与标签——落点语义两路径各异:链模式相对片段起点、
/// 音频轨事件绝对时间线)。steps.rs 旧路径同样消费本函数(单源实现)。
///
/// **链序定义**(册四 T4.8):降噪最先(频域去噪应在任何时域变换前,对倒放/
/// 变速不敏感);倒放次之(先于一切变速,PTS 重盖);随后**变调**(asetrate 升降
/// 采样率 + aresample 回 48k,同时改变时长 ×1/k)再**变速**(atempo 补偿链,
/// 因子 = speed/pitch——两者合并为单一 atempo 链,总时长恒 = durationMs);
/// 音量与淡变最后(播放域电平整形)。无新字段时与既有链逐字一致(parity 红线)。
pub fn event_body(seg: &AudioSeg) -> String {
    let mut body = String::from("aformat=sample_rates=48000:channel_layouts=stereo");
    if let Some(d) = &seg.denoise
        && let Some(f) = denoise_filter(d)
    {
        body.push_str(&format!(",{f}"));
    }
    if seg.reverse {
        // 倒放(册四 T4.4):areverse 先于 atempo;PTS 重盖单调时间戳
        body.push_str(",areverse,asetpts=N/SR/TB");
    }
    let k = seg.pitch;
    if k != 1.0 {
        // 变调(保速语义的变调半步):asetrate 整数值在生成端预算(表达式依赖
        // ffmpeg 版本,确定性红线);aresample 拉回 48k 保持链内采样率单一
        let rate = (48_000.0 * k).round() as u64;
        body.push_str(&format!(",asetrate={rate},aresample=48000,asetpts=N/SR/TB"));
    }
    let tempo = seg.speed / k;
    if tempo != 1.0 {
        for t in crate::steps::atempo_chain(tempo) {
            body.push_str(&format!(",{t}"));
        }
    }
    // volume 关键帧(IR v3)优先于静态音量(关键帧覆盖静态值;表达式在 atempo
    // 之后,t 已是播放域子段局部秒);afade 淡变仍在其后叠加(组合顺序文档化:
    // 关键帧包络 × 淡变包络相乘)。
    if let Some(ve) = &seg.volume_expr {
        body.push(',');
        body.push_str(ve);
    } else {
        body.push_str(&format!(",volume={:.4}", seg.volume));
    }
    if seg.fade_in_ms > 0.0 {
        body.push_str(&format!(",afade=t=in:st=0:d={:.3}", seg.fade_in_ms / 1000.0));
    }
    if seg.fade_out_ms > 0.0 {
        let st = (seg.duration_ms as f64 - seg.fade_out_ms).max(0.0) / 1000.0;
        body.push_str(&format!(",afade=t=out:st={st:.3}:d={:.3}", seg.fade_out_ms / 1000.0));
    }
    body
}

/// clip i 的流长秒(dur + 尾帧;尾帧 = 下一边界的有效转场时长,末段 0)。
fn clip_stream_sec(plan: &RenderPlan, i: usize) -> f64 {
    let tail = plan.boundary_durs_ms.get(i).copied().unwrap_or(0.0);
    (plan.video_clips[i].duration_ms as f64 + tail) / 1000.0
}

/// acrossfade 链模式的 mix pass A 命令行。结构:
/// 逐事件输入(-ss/-t,与旧路径同序)→ 逐片段流(子段 amix + apad 钳长;
/// 无事件片段 = anullsrc 静音)→ 边界 acrossfade/concat 链 → adelay 首片段落点
/// → [bus] → (BGM ducking 与旧路径同一形态)→ aac。
pub fn mix_pass_a_chain_args(plan: &RenderPlan, mixed_raw_out: &Path) -> Vec<String> {
    let total_ms = plan.total_ms;
    let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];
    let mut filters: Vec<String> = Vec::new();
    for seg in &plan.audio_segs {
        let read_ms = (seg.duration_ms as f64 * seg.speed).ceil();
        args.extend([
            "-ss".into(), format!("{}", seg.source_in_ms as f64 / 1000.0),
            "-t".into(), format!("{}", read_ms / 1000.0),
            "-i".into(), seg.src.to_string_lossy().into(),
        ]);
    }
    let n = plan.video_clips.len();
    // ---- 逐片段流 ----
    for i in 0..n {
        let clip = &plan.video_clips[i];
        let sec = clip_stream_sec(plan, i);
        let evs: Vec<(usize, &AudioSeg)> = plan
            .audio_segs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.clip_idx == Some(i))
            .collect();
        if evs.is_empty() {
            // 无音频片段(图块/静音):静音流垫位,链结构不变形
            filters.push(format!(
                "anullsrc=r=48000:cl=stereo,atrim=0:{sec:.6},aformat=sample_rates=48000:channel_layouts=stereo[pc{i}]"
            ));
            continue;
        }
        let mut parts: Vec<String> = Vec::new();
        let mut refs = String::new();
        for (k, s) in &evs {
            let rel = s.start_ms.saturating_sub(clip.start_ms);
            parts.push(format!("[{k}:a]{},adelay={rel}:all=1[e{i}_{k}]", event_body(s)));
            refs.push_str(&format!("[e{i}_{k}]"));
        }
        parts.push(format!(
            "{refs}amix=inputs={}:duration=longest:normalize=0,apad=whole_dur={sec:.6}[pc{i}]",
            evs.len()
        ));
        filters.extend(parts);
    }
    // ---- 边界链(acrossfade d=有效转场时长;硬切 concat 直拼) ----
    let mut cur = "pc0".to_string();
    for k in 1..n {
        let d = plan.boundary_durs_ms[k - 1];
        if d > 0.0 {
            filters.push(format!("[{cur}][pc{k}]acrossfade=d={:.6}:c1=tri:c2=tri[x{k}]", d / 1000.0));
        } else {
            filters.push(format!("[{cur}][pc{k}]concat=n=2:v=0:a=1[x{k}]"));
        }
        cur = format!("x{k}");
    }
    // ---- 链落点(首片段 startMs;通常 0)+ 轨道处理 + 音频轨事件(绝对落点)→ [bus] ----
    // 视频轨处理链(册五 T5.3):仅当全部主线片段同属一条**有处理声明**的轨时
    // 应用(acrossfade 链把多视频轨主线熔成单流,跨轨声明无单点归属——诚实跳过);
    // 无处理 = 拼接逐字一致(parity 红线)。
    let vchain = video_track_chain(plan);
    filters.push(format!("[{cur}]adelay={}:all=1{vchain}[acb]", plan.video_clips[0].start_ms));
    let others: Vec<(usize, &AudioSeg)> = plan
        .audio_segs
        .iter()
        .enumerate()
        .filter(|(_, s)| s.clip_idx.is_none())
        .collect();
    if others.is_empty() {
        filters.push("[acb]anull[bus]".into());
    } else {
        let mut parts: Vec<String> = Vec::new();
        let mut other_refs: Vec<String> = Vec::new();
        let other_segs: Vec<&AudioSeg> = others.iter().map(|(_, s)| *s).collect();
        for (k, s) in &others {
            parts.push(format!("[{k}:a]{},adelay={}:all=1[a{k}]", event_body(s), s.start_ms));
            other_refs.push(format!("[a{k}]"));
        }
        // 轨道组建流(册五 T5.3):有处理声明的轨 amix 建流 → per-track 链 → 单标签;
        // 无处理轨保持逐事件标签(零变化面)
        let (group_parts, bus_refs) = grouped_bus_labels(plan, &other_segs, &other_refs);
        parts.extend(group_parts);
        let mut all_refs = vec!["[acb]"];
        all_refs.extend(bus_refs.iter().map(|s| s.as_str()));
        parts.push(format!(
            "{}amix=inputs={}:duration=longest:normalize=0[bus]",
            all_refs.join(""),
            all_refs.len()
        ));
        filters.extend(parts);
    }
    // ---- BGM(与旧路径同一形态:循环铺满 + gain + ducking 侧链) ----
    if let Some(bgm) = &plan.bgm {
        let bgm_idx = plan.audio_segs.len();
        let bgm_path = plan.project_dir.join(&bgm.src);
        args.extend([
            "-stream_loop".into(), "-1".into(),
            "-t".into(), format!("{}", total_ms as f64 / 1000.0),
            "-i".into(), bgm_path.to_string_lossy().into(),
        ]);
        filters.push(format!(
            "[{bgm_idx}:a]aformat=sample_rates=48000:channel_layouts=stereo,volume={:.1}dB[bgmg]",
            bgm.gain_db
        ));
        if bgm.ducking && !plan.audio_segs.is_empty() {
            filters.push("[bus]asplit=2[busA][busB]".into());
            filters.push(format!("[bgmg][busA]{}[bgmc]", ducking_filter(bgm)));
            filters.push("[busB][bgmc]amix=inputs=2:duration=first:normalize=0[mixout]".into());
            args.extend(["-filter_complex".into(), filters.join(";"), "-map".into(), "[mixout]".into()]);
        } else {
            filters.push("[bus][bgmg]amix=inputs=2:duration=first:normalize=0[mixout]".into());
            args.extend(["-filter_complex".into(), filters.join(";"), "-map".into(), "[mixout]".into()]);
        }
    } else {
        args.extend(["-filter_complex".into(), filters.join(";"), "-map".into(), "[bus]".into()]);
    }
    args.extend([
        "-t".into(), format!("{}", plan.total_ms as f64 / 1000.0),
        "-c:a".into(), "aac".into(),
        mixed_raw_out.to_string_lossy().into(),
    ]);
    args
}

/// loudnorm 测量 pass 命令行(测量输出在 stderr 的 JSON)。
/// 字面串与拆分前逐字一致(parity 红线);目标参数化走 [`mix_measure_args_t`]。
pub fn mix_measure_args(mixed_raw: &Path) -> Vec<String> {
    [
        "-hide_banner", "-nostats", "-i", &mixed_raw.to_string_lossy(),
        "-filter_complex", "loudnorm=I=-14:TP=-1.0:print_format=json",
        "-f", "null", "-",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// 同 [`mix_measure_args`],响度目标显式给定(册五 T5.6 loudnormTarget;
/// 响度单/交付域定制,缺省目标用上者的既有字面串)。
pub fn mix_measure_args_t(mixed_raw: &Path, target_i: f64, target_tp: f64) -> Vec<String> {
    [
        "-hide_banner", "-nostats", "-i", &mixed_raw.to_string_lossy(),
        "-filter_complex",
        &format!(
            "loudnorm=I={}:TP={}:print_format=json",
            crate::steps::fmt_f64(target_i), crate::steps::fmt_f64(target_tp)
        ),
        "-f", "null", "-",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// 由测量结果构造 linear=true 的 loudnorm 滤镜串(先测后编;字面串与拆分前
/// 逐字一致,parity 红线)。
pub fn mix_linear_filter(measured: &Value) -> String {
    format!(
        "loudnorm=I=-14:TP=-1.0:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:linear=true",
        measured["input_i"].as_str().unwrap_or("-14"),
        measured["input_tp"].as_str().unwrap_or("-1"),
        measured["input_lra"].as_str().unwrap_or("0"),
        measured["input_thresh"].as_str().unwrap_or("-30"),
    )
}

/// 同 [`mix_linear_filter`],响度目标显式给定(册五 T5.6)。
pub fn mix_linear_filter_t(measured: &Value, target_i: f64, target_tp: f64) -> String {
    format!(
        "loudnorm=I={}:TP={}:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:linear=true",
        crate::steps::fmt_f64(target_i),
        crate::steps::fmt_f64(target_tp),
        measured["input_i"].as_str().unwrap_or("-14"),
        measured["input_tp"].as_str().unwrap_or("-1"),
        measured["input_lra"].as_str().unwrap_or("0"),
        measured["input_thresh"].as_str().unwrap_or("-30"),
    )
}

/// mix pass B(线性滤镜串已构造)命令行。滤镜串由 [`mix_linear_filter`]/
/// [`mix_linear_filter_t`](响度目标参数化,T5.6)单源构造。
pub fn mix_pass_b_args(linear_filter: &str, mixed_raw: &Path, mix_out: &Path) -> Vec<String> {
    [
        "-y", "-v", "error", "-i", &mixed_raw.to_string_lossy(),
        "-af", linear_filter,
        "-c:a", "aac", &mix_out.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// mix pass B(静音总线)命令行:原样转封装,无需归一。
/// (数字静音 -inf 遇 linear=true 的 measured 值,ffmpeg 报 "Result too large" 直接失败。)
pub fn mix_pass_b_silent_args(mixed_raw: &Path, mix_out: &Path) -> Vec<String> {
    [
        "-y", "-v", "error", "-i", &mixed_raw.to_string_lossy(),
        "-c:a", "copy", &mix_out.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// 测量 JSON 中 input_i 是否需要走 linear=true(静音/-inf/缺失 → false)。
pub fn mix_measured_is_loud(measured: &Value) -> bool {
    matches!(
        measured["input_i"].as_str().and_then(|s| s.parse::<f64>().ok()),
        Some(v) if v.is_finite() && v > -70.0
    )
}


#[cfg(test)]
mod tests {
    fn strv(v: &[String]) -> Vec<&str> {
        v.iter().map(|s| s.as_str()).collect()
    }
    #[test]
    fn mix_measure_and_pass_b_args() {
        let measured = json!({
            "input_i": "-14.5", "input_tp": "-1.2", "input_lra": "3.1", "input_thresh": "-24.5"
        });
        assert!(mix_measured_is_loud(&measured));
        assert_eq!(
            mix_linear_filter(&measured),
            "loudnorm=I=-14:TP=-1.0:measured_I=-14.5:measured_TP=-1.2:measured_LRA=3.1:measured_thresh=-24.5:linear=true"
        );
        let raw = Path::new("/c/mix/r.m4a");
        assert_eq!(
            strv(&mix_measure_args(raw)),
            ["-hide_banner", "-nostats", "-i", "/c/mix/r.m4a",
             "-filter_complex", "loudnorm=I=-14:TP=-1.0:print_format=json", "-f", "null", "-"]
        );
        assert_eq!(
            strv(&mix_pass_b_args(&mix_linear_filter(&measured), raw, Path::new("/c/mix/o.m4a"))),
            ["-y", "-v", "error", "-i", "/c/mix/r.m4a", "-af",
             "loudnorm=I=-14:TP=-1.0:measured_I=-14.5:measured_TP=-1.2:measured_LRA=3.1:measured_thresh=-24.5:linear=true",
             "-c:a", "aac", "/c/mix/o.m4a"]
        );
        // 目标参数化(T5.6 loudnormTarget):-16/-1.5 交付域定制
        assert_eq!(
            strv(&mix_measure_args_t(raw, -16.0, -1.5)),
            ["-hide_banner", "-nostats", "-i", "/c/mix/r.m4a",
             "-filter_complex", "loudnorm=I=-16:TP=-1.5:print_format=json", "-f", "null", "-"]
        );
        assert_eq!(
            mix_linear_filter_t(&measured, -16.0, -1.5),
            "loudnorm=I=-16:TP=-1.5:measured_I=-14.5:measured_TP=-1.2:measured_LRA=3.1:measured_thresh=-24.5:linear=true"
        );
        // 静音:input_i = -inf → 不走 linear,转封装
        let silent = json!({"input_i": "-inf", "input_tp": "-inf", "input_lra": "0", "input_thresh": "-70"});
        assert!(!mix_measured_is_loud(&silent));
        assert_eq!(
            strv(&mix_pass_b_silent_args(raw, Path::new("/c/mix/o.m4a"))),
            ["-y", "-v", "error", "-i", "/c/mix/r.m4a", "-c:a", "copy", "/c/mix/o.m4a"]
        );
    }

    use super::*;
    use serde_json::json;
    use std::path::Path;

    fn plan(v: serde_json::Value) -> RenderPlan {
        let project: cutforge_core::model::Project = serde_json::from_value(v).unwrap();
        RenderPlan::build(&project, Path::new("/w"), None)
    }

    fn two_clip(with_audio: bool) -> serde_json::Value {
        let mut c0 = json!({"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000, "role": "voice"});
        let mut c1 = json!({"id": "V1-002", "src": "b.mp4", "startMs": 2000, "durationMs": 2000, "role": "voice",
            "transition": {"type": "fade", "durMs": 500}});
        if with_audio {
            c0["volume"] = json!(1.0);
            c1["volume"] = json!(1.0);
        } else {
            c0["volume"] = json!(0);
            c1["volume"] = json!(0);
        }
        json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "af", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": [c0, c1]}]
        })
    }

    #[test]
    fn chain_mode_active_only_with_transitions() {
        let p = plan(two_clip(true));
        assert!(chain_active(&p), "有转场 → 链模式");
        assert_eq!(p.boundary_durs_ms, vec![500.0]);
        let no_tr = plan(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "nt", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000, "volume": 1.0},
                {"id": "V1-002", "src": "b.mp4", "startMs": 2000, "durationMs": 2000, "volume": 1.0}
            ]}]
        }));
        assert!(!chain_active(&no_tr), "无转场 → 旧路径(参数逐字兼容)");
    }

    /// 链图形状:逐片段流(pc0/pc1)→ acrossfade d=0.5 → adelay → [bus];
    /// 流长 = dur+尾帧(apad whole_dur);相对落点 adelay。
    #[test]
    fn chain_graph_mirrors_xfade_shape() {
        let p = plan(two_clip(true));
        let args = mix_pass_a_chain_args(&p, Path::new("/c/mix/r.m4a"));
        let s = args.join("\u{1}");
        assert!(s.contains("[0:a]aformat=sample_rates=48000:channel_layouts=stereo,volume=1.0000,adelay=0:all=1[e0_0]"), "{s}");
        assert!(s.contains("[e0_0]amix=inputs=1:duration=longest:normalize=0,apad=whole_dur=2.500000[pc0]"), "流长=dur+尾帧 0.5: {s}");
        assert!(s.contains("[e1_1]amix=inputs=1:duration=longest:normalize=0,apad=whole_dur=2.000000[pc1]"), "末段无尾帧: {s}");
        assert!(s.contains("[pc0][pc1]acrossfade=d=0.500000:c1=tri:c2=tri[x1]"), "{s}");
        assert!(s.contains("[x1]adelay=0:all=1[acb];[acb]anull[bus]"), "{s}");
        assert!(s.contains("-map\u{1}[bus]"), "{s}");
        // 链总长 = Σdur(2.5+2−0.5 = 4.0 = Σdur;构造性零漂移)
        assert_eq!(2.5 + 2.0 - 0.5, 4.0);
    }

    /// 静音片段垫位:无事件片段走 anullsrc 静音流,链结构不变形(声画仍零漂移)。
    #[test]
    fn muted_clips_bridge_with_silence_streams() {
        let p = plan(two_clip(false));
        assert!(p.audio_segs.is_empty(), "静音片段无事件");
        let args = mix_pass_a_chain_args(&p, Path::new("/c/mix/r.m4a"));
        let s = args.join("\u{1}");
        assert!(s.contains("anullsrc=r=48000:cl=stereo,atrim=0:2.500000"), "{s}");
        assert!(!s.contains("-i\u{1}"), "无事件不开输入");
        assert!(s.contains("[pc0][pc1]acrossfade=d=0.500000"), "{s}");
    }

    /// 音频轨事件在链模式下保持绝对落点,与链输出 amix 同一 [bus]。
    #[test]
    fn audio_track_events_stay_absolute_on_bus() {
        let mut v = two_clip(true);
        v["tracks"].as_array_mut().unwrap().push(json!(
            {"id": "A1", "kind": "audio", "clips": [
                {"id": "A1-001", "src": "sfx.mp3", "startMs": 2500, "durationMs": 500, "role": "sfx", "volume": 0.9}
            ]}
        ));
        let p = plan(v);
        let args = mix_pass_a_chain_args(&p, Path::new("/c/mix/r.m4a"));
        let s = args.join("\u{1}");
        assert!(s.contains("[2:a]aformat=sample_rates=48000:channel_layouts=stereo,volume=0.9000,adelay=2500:all=1[a2]"), "绝对落点(全局输入下标): {s}");
        assert!(s.contains("[acb][a2]amix=inputs=2:duration=longest:normalize=0[bus]"), "{s}");
    }

    /// 链模式 + BGM ducking:与旧路径同一侧链形态([bus] asplit)。
    #[test]
    fn chain_mode_bgm_ducking_uses_same_sidechain() {
        let mut v = two_clip(true);
        v["bgm"] = json!({"src": "bgm.mp3", "gainDb": -6, "ducking": true, "loop": true});
        let p = plan(v);
        let args = mix_pass_a_chain_args(&p, Path::new("/c/mix/r.m4a"));
        let s = args.join("\u{1}");
        assert!(s.contains("[bus]asplit=2[busA][busB]"), "{s}");
        assert!(s.contains("[bgmg][busA]sidechaincompress=threshold=0.03:ratio=8:attack=80:release=500[bgmc]"), "{s}");
        assert!(s.contains("-map\u{1}[mixout]"), "{s}");
    }

    /// 硬切边界(D=0)与转场边界混链:acrossfade 只出现在转场处。
    #[test]
    fn hard_cut_boundaries_join_via_concat() {
        let v = json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "mix", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000, "volume": 1.0},
                {"id": "V1-002", "src": "b.mp4", "startMs": 2000, "durationMs": 2000, "volume": 1.0,
                 "transition": {"type": "fade", "durMs": 400}},
                {"id": "V1-003", "src": "c.mp4", "startMs": 4000, "durationMs": 2000, "volume": 1.0,
                 "transition": {"type": "cut", "durMs": 400}}
            ]}]
        });
        let p = plan(v);
        assert!(chain_active(&p));
        let args = mix_pass_a_chain_args(&p, Path::new("/c/mix/r.m4a"));
        let s = args.join("\u{1}");
        assert!(s.contains("[pc0][pc1]acrossfade=d=0.400000"), "{s}");
        assert!(s.contains("[x1][pc2]concat=n=2:v=0:a=1[x2]"), "{s}");
    }

    /// 事件链体与旧路径单源:变速/倒放/淡变的拼装顺序逐字一致。
    /// 无 denoise/pitch 字段时链与既有逐字一致(parity 红线)。
    #[test]
    fn event_body_matches_legacy_shape() {
        let seg = AudioSeg {
            src: "a.mp4".into(), start_ms: 0, duration_ms: 1000, source_in_ms: 0,
            volume: 0.8, speed: 2.0, reverse: true, denoise: None, pitch: 1.0,
            fade_in_ms: 100.0, fade_out_ms: 200.0,
            volume_expr: None,
            track_id: None,
            clip_idx: None,
        };
        assert_eq!(
            event_body(&seg),
            "aformat=sample_rates=48000:channel_layouts=stereo,areverse,asetpts=N/SR/TB,atempo=2.000000,volume=0.8000,afade=t=in:st=0:d=0.100,afade=t=out:st=0.800:d=0.200"
        );
    }

    // ---- 册四 T4.8:降噪 + 变调链 ----

    /// 降噪档映射:low/mid/high → afftdn 参数;off/未知 → 不降噪。
    #[test]
    fn denoise_levels_map_to_afftdn() {
        assert_eq!(denoise_filter("low"), Some("afftdn=nr=8:nf=-25:tn=1"));
        assert_eq!(denoise_filter("mid"), Some("afftdn=nr=15:nf=-35:tn=1"));
        assert_eq!(denoise_filter("high"), Some("afftdn=nr=25:nf=-45:tn=1"));
        assert_eq!(denoise_filter("off"), None, "off 不降噪");
        assert_eq!(denoise_filter("爆裂"), None, "未知档诚实降级");
    }

    /// 变调系数:半音 → 2^(n/12)(+12 恰为 2 倍;0/非有限恒 1)。
    #[test]
    fn pitch_factor_semantics() {
        assert!((pitch_factor(12.0) - 2.0).abs() < 1e-9, "+12 半音 = 倍频");
        assert!((pitch_factor(-12.0) - 0.5).abs() < 1e-9, "-12 半音 = 半频");
        assert!((pitch_factor(0.0) - 1.0).abs() < 1e-9);
        assert!((pitch_factor(f64::NAN) - 1.0).abs() < 1e-9, "非有限诚实回落");
    }

    /// 链序红线(册四 T4.8):aformat → **denoise** → areverse → **变调(asetrate
    /// +aresample)** → volume。asetrate 用生成端预算的整数值(96000 = 48000×2),
    /// 不依赖 ffmpeg 表达式(确定性红线)。
    #[test]
    fn denoise_pitch_chain_order_is_fixed() {
        let seg = AudioSeg {
            src: "a.mp4".into(), start_ms: 0, duration_ms: 1000, source_in_ms: 0,
            volume: 1.0, speed: 1.0, reverse: true, denoise: Some("mid".into()), pitch: 2.0,
            fade_in_ms: 0.0, fade_out_ms: 0.0,
            volume_expr: None,
            track_id: None,
            clip_idx: None,
        };
        assert_eq!(
            event_body(&seg),
            "aformat=sample_rates=48000:channel_layouts=stereo,afftdn=nr=15:nf=-35:tn=1,\
             areverse,asetpts=N/SR/TB,asetrate=96000,aresample=48000,asetpts=N/SR/TB,atempo=0.500000,volume=1.0000"
                .replace("             ", "")
        );
    }

    /// 保速组合语义:atempo 因子 = speed/k 合并为单一链(总时长恒 durationMs);
    /// k=2 且 speed=0.5 → 0.25(链分解);k=0.5(-12 半音)且 speed=2 → 4(链分解)。
    #[test]
    fn pitch_tempo_compensation_combines_with_speed() {
        let seg = AudioSeg {
            src: "a.mp4".into(), start_ms: 0, duration_ms: 1000, source_in_ms: 0,
            volume: 1.0, speed: 0.5, reverse: false, denoise: None, pitch: 2.0,
            fade_in_ms: 0.0, fade_out_ms: 0.0,
            volume_expr: None,
            track_id: None,
            clip_idx: None,
        };
        let body = event_body(&seg);
        assert!(body.contains("asetrate=96000"), "{body}");
        assert!(body.contains("atempo=0.5,atempo=0.500000"), "atempo 链 = 0.25 分解: {body}");
        // -12 半音(k=0.5)+ 2x 速度 → atempo = 4(链分解 2×2);降噪先于倒放
        let seg = AudioSeg {
            src: "a.mp4".into(), start_ms: 0, duration_ms: 1000, source_in_ms: 0,
            volume: 1.0, speed: 2.0, reverse: false, denoise: Some("high".into()), pitch: 0.5,
            fade_in_ms: 0.0, fade_out_ms: 0.0,
            volume_expr: None,
            track_id: None,
            clip_idx: None,
        };
        let body = event_body(&seg);
        assert!(body.contains("asetrate=24000"), "{body}");
        assert!(body.contains("afftdn=nr=25:nf=-45:tn=1,asetrate=24000"), "降噪在变调前(此例无倒放): {body}");
        assert!(body.contains("atempo=2.0,atempo=2.000000"), "atempo 链 = 4 分解: {body}");
    }
}
