// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 渲染步骤纯函数(T1.4):每步 = 纯函数(生成 ffmpeg 参数)+ lib.rs 执行器调用。
//! 本文件**不启动任何进程**:全部函数只做输入 → 命令行字符串的映射,
//! 可在不装 ffmpeg 的环境单测断言;参数串与拆分前的 render() 逐字一致
//! (渲染输出逐字节语义不变的底线,由 parity_matrix 实渲夹具锁定)。
//! 册四 A4-BE2:段提取链(步 2 segment)随曲线/变换扩容,纯移动至
//! [`crate::segment`](段链模块);此处 `pub use` 保持 `steps::segment_*` 接口路径不变。

use crate::plan::{OverlaySeg, RenderPlan};
use cutforge_core::model::{Clip, Transition};
use serde_json::Value;
use std::path::{Path, PathBuf};

// ---- 步 2 segment(实现在 segment.rs;路径兼容再导出,册四 A4-BE2) ----
pub use crate::segment::{
    play_segments, reverse_chain, segment_args, segment_filter, segment_filter_complex,
    segment_pad_ms, segment_read_ms, transform_pre_chain,
};

/// clip i 的出向转场时长(ms;转场字段在 clip i 上表示 i-1→i 的转场,
/// i=0 无意义)。type=cut/none 或 durMs<=0 → 硬切(None)。
pub fn transition_out_ms(clip: &Clip) -> Option<(String, f64)> {
    if clip.start_ms == 0 && clip.transition.is_none() {
        return None;
    }
    let t: &Transition = clip.transition.as_ref()?;
    match t.type_.as_deref() {
        Some("cut") | Some("none") => return None,
        _ => {}
    }
    let dur = t.dur_ms.unwrap_or(0.0);
    if dur <= 0.0 {
        return None;
    }
    let kind = t.type_.clone().unwrap_or_else(|| "fade".into());
    Some((kind, dur))
}

/// 段 i 的尾帧扩展毫秒(ADR-0023):clip i 的转场由 seg_i 与 seg_{i+1} 之间的
/// xfade 消费,段 i 需要延长 D_{i+1} 保证整链零时间漂移。该值进入段缓存键
/// (旧键漏掉它 → 改转场会陈旧复用前一段的 tpad)。D 取**有效转场时长**
/// (册四 BE3a:钳到两侧片段时长,与 compose/acrossfade 同一函数,尾帧只进重叠)。
pub fn segment_tail_ms(video_clips: &[Clip], i: usize) -> f64 {
    if i + 1 < video_clips.len() {
        crate::catalog::effective_transition_ms(video_clips, i + 1)
    } else {
        0.0
    }
}

// ---------------- 步 1 probe ----------------

/// probe 目标:全部存在的素材源(容错:图片等无时长素材合法存在;真坏了在 segment 步炸)。
/// 覆盖三路来源:主时间线视频段 / 音频事件 / overlay 叠加层。
pub fn probe_paths(plan: &RenderPlan) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for c in &plan.video_clips {
        if let Some(src) = &c.src {
            out.push(plan.project_dir.join(src));
        }
    }
    for s in &plan.audio_segs {
        out.push(s.src.clone());
    }
    for o in &plan.overlay_segs {
        out.push(o.src.clone());
    }
    out.retain(|p| p.is_file());
    out
}

// ---------------- 步 3 compose(xfade 链 / concat 退化) ----------------

/// 有出向转场 → xfade 链;否则退化 concat(-c copy,零重编码)。
pub fn is_xfade_chain(video_clips: &[Clip]) -> bool {
    video_clips.iter().enumerate().any(|(i, c)| i > 0 && transition_out_ms(c).is_some())
}

/// xfade 链命令行:offset_k = **前序名义时长累计**(ADR-0023 口径:不含尾帧扩展
/// ——尾帧只进转场重叠不前移 offset;册四 BE3a 实测修复:旧实现把尾帧计入累计,
/// offset 越过输入末端导致 xfade 坍缩截断,后段整段丢失)。返回 (参数, WARN 列表)
/// ——转场名经目录直通解析,未注册降级 fade 留痕(册四 T4.5)。
pub fn compose_xfade_args(
    video_clips: &[Clip],
    seg_files: &[PathBuf],
    composed_out: &Path,
) -> (Vec<String>, Vec<String>) {
    let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];
    for f in seg_files {
        args.extend(["-i".into(), f.to_string_lossy().into()]);
    }
    let mut filters: Vec<String> = Vec::new();
    let mut warns: Vec<String> = Vec::new();
    let mut acc_nominal_ms = 0f64;
    let mut cur_label = "[0:v]".to_string();
    for i in 1..video_clips.len() {
        let dur = crate::catalog::effective_transition_ms(video_clips, i);
        acc_nominal_ms += video_clips[i - 1].duration_ms as f64;
        let offset = acc_nominal_ms / 1000.0;
        let (kind, warn) = crate::catalog::resolve_transition(&video_clips[i]);
        if let Some(w) = warn {
            warns.push(format!("clip {} 转场: {w}", video_clips[i].id));
        }
        let out_label = format!("[x{i}]");
        filters.push(format!(
            "{cur_label}[{i}:v]xfade=transition={kind}:duration={:.6}:offset={:.6}{out_label}",
            dur / 1000.0,
            offset
        ));
        cur_label = out_label;
    }
    let last_label = format!("[x{}]", video_clips.len() - 1);
    args.extend(["-filter_complex".into(), filters.join(";"), "-map".into(), last_label]);
    args.extend([
        "-c:v".into(), "libx264".into(), "-preset".into(), "veryfast".into(),
        composed_out.to_string_lossy().into(),
    ]);
    (args, warns)
}

/// concat 清单内容(路径统一正斜杠;concat demuxer 对引号内反斜杠敏感)。
pub fn concat_list_content(seg_files: &[PathBuf]) -> String {
    let mut list = String::new();
    for f in seg_files {
        list.push_str(&format!("file '{}'\n", f.to_string_lossy().replace('\\', "/")));
    }
    list
}

/// concat 合成命令行(-c copy 不重编码)。
pub fn compose_concat_args(concat_list: &Path, composed_out: &Path) -> Vec<String> {
    [
        "-y", "-v", "error", "-f", "concat", "-safe", "0",
        "-i", &concat_list.to_string_lossy(),
        "-c", "copy", &composed_out.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

// ---------------- 步 4 overlay ----------------

/// overlay 合成命令行:绝对像素 + opacity(colorchannelmixer)+ 时间窗 enable。
pub fn overlay_args(overlay_segs: &[OverlaySeg], composed_in: &Path, overlaid_out: &Path) -> Vec<String> {
    let mut args: Vec<String> =
        vec!["-y".into(), "-v".into(), "error".into(), "-i".into(), composed_in.to_string_lossy().into()];
    for ov in overlay_segs {
        args.extend(["-i".into(), ov.src.to_string_lossy().into()]);
    }
    let mut filters: Vec<String> = Vec::new();
    let mut cur = "[0:v]".to_string();
    for (i, ov) in overlay_segs.iter().enumerate() {
        let scale_chain = format!("scale={}:{}", ov.spec.w, ov.spec.h);
        let chain = if (ov.spec.opacity - 1.0).abs() < f64::EPSILON {
            scale_chain
        } else {
            format!("{scale_chain},format=rgba,colorchannelmixer=aa={:.4}", ov.spec.opacity)
        };
        filters.push(format!(
            "[{i}:v]{chain}[l{i}];{cur}[l{i}]overlay=x={}:y={}:enable='between(t,{:.3},{:.3})'[o{}]",
            ov.spec.x, ov.spec.y,
            ov.start_ms as f64 / 1000.0,
            (ov.start_ms + ov.duration_ms) as f64 / 1000.0,
            i
        ));
        cur = format!("[o{i}]");
    }
    args.extend([
        "-filter_complex".into(), filters.join(";"), "-map".into(), cur,
        "-c:v".into(), "libx264".into(), "-preset".into(), "veryfast".into(),
        overlaid_out.to_string_lossy().into(),
    ]);
    args
}

// ---------------- 步 5 mix(画幅无关,共享缓存) ----------------

/// mix pass A 命令行:主时间线有视频转场时走 acrossfade 链(册四 T4.5,M11-R1;
/// 见 [`crate::across`] 模块注释),否则既有逐段落点路径(参数逐字一致,parity 红线)。
pub fn mix_pass_a_args(plan: &RenderPlan, mixed_raw_out: &Path) -> Vec<String> {
    if crate::across::chain_active(plan) {
        return crate::across::mix_pass_a_chain_args(plan, mixed_raw_out);
    }
    let audio_segs = &plan.audio_segs;
    let total_ms = plan.total_ms;
    let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];
    let mut filters: Vec<String> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    if audio_segs.is_empty() && plan.bgm.is_none() {
        args.extend(["-f".into(), "lavfi".into(), "-i".into(), "anullsrc=r=48000:cl=stereo".into()]);
    }
    let mut input_idx = 0usize;
    for seg in audio_segs {
        let read_ms = (seg.duration_ms as f64 * seg.speed).ceil();
        args.extend([
            "-ss".into(), format!("{}", seg.source_in_ms as f64 / 1000.0),
            "-t".into(), format!("{}", read_ms / 1000.0),
            "-i".into(), seg.src.to_string_lossy().into(),
        ]);
        let chain_body = crate::across::event_body(seg);
        filters.push(format!("[{input_idx}:a]{chain_body},adelay={}:all=1[a{input_idx}]", seg.start_ms));
        labels.push(format!("[a{input_idx}]"));
        input_idx += 1;
    }
    // 总线([bus] 恒存在:bgm-only 工程用 anullsrc 占位)
    if audio_segs.is_empty() && plan.bgm.is_some() {
        args.extend(["-f".into(), "lavfi".into(), "-i".into(), "anullsrc=r=48000:cl=stereo".into()]);
        filters.push(format!("[{input_idx}:a]anull[bus]"));
        input_idx += 1;
    } else if !audio_segs.is_empty() {
        filters.push(format!(
            "{}amix=inputs={}:duration=longest:normalize=0[bus]",
            labels.join(""),
            audio_segs.len()
        ));
    }
    // BGM(循环铺满 + gain + ducking 侧链)
    if let Some(bgm) = &plan.bgm {
        let bgm_path = plan.project_dir.join(&bgm.src);
        args.extend([
            "-stream_loop".into(), "-1".into(),
            "-t".into(), format!("{}", total_ms as f64 / 1000.0),
            "-i".into(), bgm_path.to_string_lossy().into(),
        ]);
        let bgm_idx = input_idx;
        filters.push(format!(
            "[{bgm_idx}:a]aformat=sample_rates=48000:channel_layouts=stereo,volume={:.1}dB[bgmg]",
            bgm.gain_db
        ));
        if bgm.ducking && !audio_segs.is_empty() {
            // [bus] 需被 sidechain(key)与 amix 各消费一次 → asplit 分流
            // (本地 ffmpeg 容忍重复 label,CI 严格报 Invalid stream specifier)
            filters.push("[bus]asplit=2[busA][busB]".into());
            filters.push(
                "[bgmg][busA]sidechaincompress=threshold=0.03:ratio=8:attack=80:release=500[bgmc]".into(),
            );
            filters.push("[busB][bgmc]amix=inputs=2:duration=first:normalize=0[mixout]".into());
        } else {
            filters.push("[bus][bgmg]amix=inputs=2:duration=first:normalize=0[mixout]".into());
        }
        args.extend(["-filter_complex".into(), filters.join(";"), "-map".into(), "[mixout]".into()]);
    } else if !audio_segs.is_empty() {
        args.extend(["-filter_complex".into(), filters.join(";"), "-map".into(), "[bus]".into()]);
    }
    if plan.bgm.is_none() && audio_segs.is_empty() {
        // anullsrc 路径:输入即静音源,无 filter_complex
    }
    args.extend([
        "-t".into(), format!("{}", total_ms as f64 / 1000.0),
        "-c:a".into(), "aac".into(),
        mixed_raw_out.to_string_lossy().into(),
    ]);
    args
}

/// loudnorm 测量 pass 命令行(测量输出在 stderr 的 JSON)。
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

/// 由测量结果构造 linear=true 的 loudnorm 滤镜串(先测后编)。
pub fn mix_linear_filter(measured: &Value) -> String {
    format!(
        "loudnorm=I=-14:TP=-1.0:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:linear=true",
        measured["input_i"].as_str().unwrap_or("-14"),
        measured["input_tp"].as_str().unwrap_or("-1"),
        measured["input_lra"].as_str().unwrap_or("0"),
        measured["input_thresh"].as_str().unwrap_or("-30"),
    )
}

/// mix pass B(有响度测量值)命令行。
pub fn mix_pass_b_args(measured: &Value, mixed_raw: &Path, mix_out: &Path) -> Vec<String> {
    [
        "-y", "-v", "error", "-i", &mixed_raw.to_string_lossy(),
        "-af", &mix_linear_filter(measured),
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

// ---------------- 步 6 subtitle(最后叠) ----------------

/// 字幕烧录命令行(cwd = 缓存根;ASS 以相对路径喂给 subtitles 滤镜,规避
/// Windows 盘符冒号在滤镜参数里的转义问题)。ass_rels = 烧录序列(册四 T4.7:
/// 外部字幕 + 文本轨生成 ASS 串联——多条 subtitles 滤镜链式应用,免解析合并
/// 外部文件;单条时参数与拆分前逐字一致,parity 红线)。
pub fn subtitle_burn_args(video_in: &Path, mixed_in: &Path, ass_rels: &[String], subbed_out: &Path) -> Vec<String> {
    let vf = ass_rels.iter().map(|r| format!("subtitles={r}")).collect::<Vec<_>>().join(",");
    [
        "-y", "-v", "error",
        "-i", &video_in.to_string_lossy(),
        "-i", &mixed_in.to_string_lossy(),
        "-vf", &vf,
        "-map", "0:v", "-map", "1:a",
        "-c:v", "libx264", "-preset", "veryfast", "-c:a", "copy",
        &subbed_out.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// 无字幕合流命令行(零重编码转封装)。
pub fn subtitle_mux_args(video_in: &Path, mixed_in: &Path, subbed_out: &Path) -> Vec<String> {
    [
        "-y", "-v", "error",
        "-i", &video_in.to_string_lossy(),
        "-i", &mixed_in.to_string_lossy(),
        "-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", "copy",
        &subbed_out.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

// ---------------- 步 7 encode ----------------

/// 最终成片输出路径:06_成片输出/final_cutforge_{slug}_{W}x{H}.mp4(文件名消毒;
/// 路径与格式与拆分前一致,不缓存)。
pub fn encode_output_path(plan: &RenderPlan) -> PathBuf {
    let canvas = format!("{}x{}", plan.canvas_w, plan.canvas_h);
    plan.out_dir.join(format!("final_cutforge_{}_{}.mp4", sanitize_slug(&plan.slug), canvas))
}

// ---------------- 共享小工具 ----------------

/// 输出文件名消毒:仅替换文件系统敌对字符,保留 CJK/字母数字/-_
pub fn sanitize_slug(slug: &str) -> String {
    slug.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect()
}

/// atempo 链(单实例只支持 0.5–2.0;speed ∈ [0.25,4] 用链式分解)
pub fn atempo_chain(speed: f64) -> Vec<String> {
    let mut out = Vec::new();
    let mut s = speed;
    while s > 2.0 {
        out.push("atempo=2.0".into());
        s /= 2.0;
    }
    while s < 0.5 {
        out.push("atempo=0.5".into());
        s /= 0.5;
    }
    out.push(format!("atempo={s:.6}"));
    out
}

/// f64 → 紧凑字符串(去尾零;空串回落 "0")。
pub fn fmt_f64(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s.is_empty() { "0".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cutforge_core::model::Overlay;
    use serde_json::json;
    use std::path::PathBuf;

    /// 测试计划:fake 路径(纯函数不触 IO)。
    fn test_plan(clips: Value) -> (RenderPlan, Vec<Clip>) {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "demo", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": clips}]
        }))
        .unwrap();
        let plan = RenderPlan::build(&project, Path::new("/w"), None);
        (plan, project.tracks[0].clips.clone())
    }

    fn base_clips() -> Value {
        json!([
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000,
             "sourceInMs": 500, "role": "voice", "volume": 1.0},
            {"id": "V1-002", "src": "a.mp4", "startMs": 2000, "durationMs": 2000, "role": "voice"}
        ])
    }

    fn strv(v: &[String]) -> Vec<&str> {
        v.iter().map(|s| s.as_str()).collect()
    }

    /// 平台一致的路径期望值(join 的分隔符随平台变化)。
    fn pj(base: &str, rel: &str) -> String {
        Path::new(base).join(rel).to_string_lossy().into_owned()
    }

    /// 全参数拼成单串做子串断言(filter_complex 的滤镜以 ';' 连在同一参数内)。
    fn joined(args: &[String]) -> String {
        args.join("\u{1}")
    }

    #[test]
    fn tail_ms_comes_from_next_clip_transition() {
        let mut v = base_clips();
        v[1]["transition"] = json!({"type": "fade", "durMs": 500, "reason": "topic"});
        let (_, clips) = test_plan(v);
        assert_eq!(segment_tail_ms(&clips, 0), 500.0, "段 0 的尾帧由 clip 1 的入向转场决定");
        assert_eq!(segment_tail_ms(&clips, 1), 0.0, "末段无尾帧");
    }

    #[test]
    fn transition_cut_and_zero_dur_are_hard_cut() {
        let mut v = base_clips();
        v[1]["transition"] = json!({"type": "cut", "durMs": 500});
        let (_, clips) = test_plan(v.clone());
        assert_eq!(transition_out_ms(&clips[1]), None);
        v[1]["transition"] = json!({"type": "fade", "durMs": 0});
        let (_, clips) = test_plan(v);
        assert_eq!(transition_out_ms(&clips[1]), None);
    }

    // ---- 步 3:compose ----

    #[test]
    fn compose_concat_args_when_no_transitions() {
        let (_, clips) = test_plan(base_clips());
        assert!(!is_xfade_chain(&clips));
        let segs = vec![PathBuf::from("/c/seg/1.mp4"), PathBuf::from("/c/seg/2.mp4")];
        let list = concat_list_content(&segs);
        assert_eq!(list, "file '/c/seg/1.mp4'\nfile '/c/seg/2.mp4'\n");
        let args = compose_concat_args(Path::new("/c/tmp/c.txt"), Path::new("/c/compose/o.mp4"));
        assert_eq!(
            strv(&args),
            ["-y", "-v", "error", "-f", "concat", "-safe", "0", "-i", "/c/tmp/c.txt",
             "-c", "copy", "/c/compose/o.mp4"]
        );
    }

    #[test]
    fn compose_xfade_offsets_accumulate_segment_durations() {
        let mut v = base_clips();
        v[1]["transition"] = json!({"type": "fade", "durMs": 500, "reason": "topic"});
        let (_, clips) = test_plan(v);
        assert!(is_xfade_chain(&clips));
        let segs = vec![PathBuf::from("/c/seg/1.mp4"), PathBuf::from("/c/seg/2.mp4")];
        let (args, warns) = compose_xfade_args(&clips, &segs, Path::new("/c/compose/o.mp4"));
        assert!(warns.is_empty());
        let s = strv(&args);
        assert_eq!(
            s[8],
            "[0:v][1:v]xfade=transition=fade:duration=0.500000:offset=2.000000[x1]",
            "offset = 前序**名义**时长累计(ADR-0023;尾帧只进重叠不前移 offset)"
        );
        assert_eq!(&s[..7], ["-y", "-v", "error", "-i", "/c/seg/1.mp4", "-i", "/c/seg/2.mp4"]);
        assert_eq!(
            &s[9..],
            ["-map", "[x1]", "-c:v", "libx264", "-preset", "veryfast", "/c/compose/o.mp4"]
        );
    }

    /// 册四 BE3a 零漂移红线:两段各 2s + 500ms 转场,xfade 链总输出 = Σdur = 4.0s
    /// (offset=名义时长口径;旧实现 offset 含尾帧会把后段截没——夹具以**视频流**
    /// 帧数锁定,容器时长会被音频 mux 撑大而说谎)。
    #[test]
    fn compose_xfade_two_boundary_chain_offsets_use_nominal_durations() {
        let mut v = base_clips();
        v[1]["durationMs"] = json!(1500);
        v[1]["startMs"] = json!(2000);
        v[1]["transition"] = json!({"type": "fade", "durMs": 500});
        v.as_array_mut().unwrap().push(json!({"id": "V1-003", "src": "a.mp4", "startMs": 3500, "durationMs": 1000,
             "role": "voice", "transition": {"type": "fade", "durMs": 400}}));
        let (_, clips) = test_plan(v);
        let segs: Vec<PathBuf> = (1..=3).map(|i| PathBuf::from(format!("/c/seg/{i}.mp4"))).collect();
        let (args, warns) = compose_xfade_args(&clips, &segs, Path::new("/c/o.mp4"));
        assert!(warns.is_empty());
        let s = args.join("\u{1}");
        assert!(s.contains("xfade=transition=fade:duration=0.500000:offset=2.000000[x1]"), "{s}");
        assert!(s.contains("[x1][2:v]xfade=transition=fade:duration=0.400000:offset=3.500000[x2]"), "{s}");
    }

    /// 目录直通:fx=tr.circleclose 覆写 type;未注册 id 进 WARN 列表(降级 fade)。
    #[test]
    fn compose_xfade_resolves_catalog_ids_with_warnings() {
        let mut v = base_clips();
        v[1]["transition"] = json!({"type": "fade", "fx": "tr.circleclose", "durMs": 500});
        let (_, clips) = test_plan(v.clone());
        let segs = vec![PathBuf::from("/c/seg/1.mp4"), PathBuf::from("/c/seg/2.mp4")];
        let (args, warns) = compose_xfade_args(&clips, &segs, Path::new("/c/o.mp4"));
        assert!(args.join("\u{1}").contains("transition=circleclose"));
        assert!(warns.iter().any(|w| w.contains("以 fx 为准")), "{warns:?}");
        v[1]["transition"] = json!({"type": "fade", "fx": "tr.幽灵", "durMs": 500});
        let (_, clips) = test_plan(v);
        let (args, warns) = compose_xfade_args(&clips, &segs, Path::new("/c/o.mp4"));
        assert!(args.join("\u{1}").contains("transition=fade"), "未注册降级 fade");
        assert!(warns.iter().any(|w| w.contains("未注册")), "{warns:?}");
    }

    // ---- 步 4:overlay ----

    #[test]
    fn overlay_args_chain_scale_window_and_opacity() {
        let (plan, _) = test_plan(base_clips());
        let ovs = vec![OverlaySeg {
            src: PathBuf::from("/w/logo.png"),
            start_ms: 0,
            duration_ms: 2000,
            spec: Overlay { x: 40, y: 40, w: 60, h: 60, opacity: 0.5 },
        }];
        let args = overlay_args(&ovs, &plan.cache_dir.join("c.mp4"), Path::new("/c/overlay/o.mp4"));
        let s = strv(&args);
        assert_eq!(
            &s[..7],
            ["-y", "-v", "error", "-i", &pj(&plan.cache_dir.to_string_lossy(), "c.mp4"), "-i", "/w/logo.png"]
        );
        assert_eq!(
            s[8],
            "[0:v]scale=60:60,format=rgba,colorchannelmixer=aa=0.5000[l0];\
[0:v][l0]overlay=x=40:y=40:enable='between(t,0.000,2.000)'[o0]"
        );
        assert_eq!(&s[9..], ["-map", "[o0]", "-c:v", "libx264", "-preset", "veryfast", "/c/overlay/o.mp4"]);
    }

    #[test]
    fn overlay_opacity_one_skips_alpha_chain() {
        let ovs = vec![OverlaySeg {
            src: PathBuf::from("/w/logo.png"),
            start_ms: 0,
            duration_ms: 1000,
            spec: Overlay { x: 1, y: 2, w: 3, h: 4, opacity: 1.0 },
        }];
        let args = overlay_args(&ovs, Path::new("/c.mp4"), Path::new("/o.mp4"));
        assert!(args[8].starts_with("[0:v]scale=3:4[l0];"), "opacity=1 不引入 rgba 链");
    }

    // ---- 步 5:mix ----

    #[test]
    fn mix_pass_a_voice_segment_chain_has_crop_fades_and_delay() {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "m", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [
                {"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000,
                     "sourceInMs": 0, "role": "voice", "volume": 1.0, "speed": 1.5,
                     "fade": {"inMs": 800, "outMs": 400}}
                ]},
                {"id": "A1", "kind": "audio", "clips": []}
            ]
        }))
        .unwrap();
        let plan = RenderPlan::build(&project, Path::new("/w"), None);
        let args = mix_pass_a_args(&plan, Path::new("/c/mix/r.m4a"));
        // read_ms = 2000×1.5 = 3000 → 输入侧 -ss 0 / -t 3
        assert_eq!(&strv(&args)[3..8], ["-ss", "0", "-t", "3", "-i"]);
        assert_eq!(args[8], pj("/w", "a.mp4"));
        let j = joined(&args);
        assert!(
            j.contains(
                "[0:a]aformat=sample_rates=48000:channel_layouts=stereo,atempo=1.500000,volume=1.0000,\
afade=t=in:st=0:d=0.800,afade=t=out:st=1.600:d=0.400,adelay=0:all=1[a0]"
            ),
            "单段链(变速/音量/淡入淡出/落点)不符: {j}"
        );
        assert!(j.contains("amix=inputs=1:duration=longest:normalize=0[bus]"));
        assert!(j.contains("-map"));
        assert!(j.contains("[bus]"));
        assert_eq!(args.last().unwrap(), "/c/mix/r.m4a");
    }

    #[test]
    fn mix_pass_a_bgm_only_uses_anullsrc_bus_and_ducking_splits_bus() {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "m", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "bgm": {"src": "bgm.mp3", "gainDb": -6, "ducking": true, "loop": true},
            "tracks": [
                {"id": "V1", "kind": "video", "clips": []},
                {"id": "A1", "kind": "audio", "clips": []}
            ]
        }))
        .unwrap();
        let plan = RenderPlan::build(&project, Path::new("/w"), None);
        let args = mix_pass_a_args(&plan, Path::new("/c/mix/r.m4a"));
        let j = joined(&args);
        assert!(j.contains("anullsrc=r=48000:cl=stereo"));
        assert!(j.contains("[0:a]anull[bus]"));
        assert!(j.contains("volume=-6.0dB[bgmg]"));
        // bgm-only:ducking 分支需要人声,缺省走直混
        assert!(j.contains("[bus][bgmg]amix=inputs=2:duration=first:normalize=0[mixout]"));
        assert!(j.contains("-stream_loop"));
        assert!(!j.contains("sidechaincompress"), "无人声时无侧链");
    }

    #[test]
    fn mix_pass_a_ducking_with_voice_splits_bus() {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "m", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "bgm": {"src": "bgm.mp3", "gainDb": -6, "ducking": true, "loop": true},
            "tracks": [
                {"id": "V1", "kind": "video", "clips": []},
                {"id": "A1", "kind": "audio", "clips": [
                    {"id": "A1-001", "src": "v.mp3", "startMs": 0, "durationMs": 2000, "volume": 1.0}
                ]}
            ]
        }))
        .unwrap();
        let plan = RenderPlan::build(&project, Path::new("/w"), None);
        let args = mix_pass_a_args(&plan, Path::new("/c/mix/r.m4a"));
        let j = joined(&args);
        assert!(j.contains("[bus]asplit=2[busA][busB]"));
        assert!(j.contains("[bgmg][busA]sidechaincompress=threshold=0.03:ratio=8:attack=80:release=500[bgmc]"));
        assert!(j.contains("[busB][bgmc]amix=inputs=2:duration=first:normalize=0[mixout]"));
    }

    #[test]
    fn mix_silent_project_is_anullsrc_passthrough() {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "m", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": []}]
        }))
        .unwrap();
        let plan = RenderPlan::build(&project, Path::new("/w"), None);
        let args = mix_pass_a_args(&plan, Path::new("/c/mix/r.m4a"));
        assert!(!args.iter().any(|a| a.contains("filter_complex") || a.contains("amix")), "无 filter_complex");
        assert!(args.iter().any(|a| a.contains("anullsrc")));
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
            strv(&mix_pass_b_args(&measured, raw, Path::new("/c/mix/o.m4a"))),
            ["-y", "-v", "error", "-i", "/c/mix/r.m4a", "-af",
             "loudnorm=I=-14:TP=-1.0:measured_I=-14.5:measured_TP=-1.2:measured_LRA=3.1:measured_thresh=-24.5:linear=true",
             "-c:a", "aac", "/c/mix/o.m4a"]
        );
        // 静音:input_i = -inf → 不走 linear,转封装
        let silent = json!({"input_i": "-inf", "input_tp": "-inf", "input_lra": "0", "input_thresh": "-70"});
        assert!(!mix_measured_is_loud(&silent));
        assert_eq!(
            strv(&mix_pass_b_silent_args(raw, Path::new("/c/mix/o.m4a"))),
            ["-y", "-v", "error", "-i", "/c/mix/r.m4a", "-c:a", "copy", "/c/mix/o.m4a"]
        );
    }

    #[test]
    fn atempo_chain_decomposes_extreme_speeds() {
        assert_eq!(atempo_chain(1.0), vec!["atempo=1.000000"]);
        assert_eq!(atempo_chain(4.0), vec!["atempo=2.0", "atempo=2.000000"]);
        assert_eq!(atempo_chain(0.25), vec!["atempo=0.5", "atempo=0.500000"]);
        assert_eq!(atempo_chain(3.0), vec!["atempo=2.0", "atempo=1.500000"]);
    }

    // ---- 步 6/7:subtitle + encode ----

    #[test]
    fn subtitle_burn_and_mux_args_are_backward_compatible() {
        let burn = subtitle_burn_args(
            Path::new("/c/sub/v.mp4"), Path::new("/c/mix/m.m4a"), &["tmp/ass-k.ass".into()], Path::new("/c/sub/o.mp4"),
        );
        assert_eq!(
            strv(&burn),
            ["-y", "-v", "error", "-i", "/c/sub/v.mp4", "-i", "/c/mix/m.m4a",
             "-vf", "subtitles=tmp/ass-k.ass", "-map", "0:v", "-map", "1:a",
             "-c:v", "libx264", "-preset", "veryfast", "-c:a", "copy", "/c/sub/o.mp4"]
        );
        // 册四 T4.7:外部字幕 + 文本轨生成 ASS 串联(链式 subtitles 滤镜)
        let both = subtitle_burn_args(
            Path::new("/c/sub/v.mp4"), Path::new("/c/mix/m.m4a"),
            &["tmp/ass-user.ass".into(), "tmp/ass-text.ass".into()], Path::new("/c/sub/o.mp4"),
        );
        assert_eq!(both[8], "subtitles=tmp/ass-user.ass,subtitles=tmp/ass-text.ass", "{:?}", both[8]);
        let mux = subtitle_mux_args(
            Path::new("/c/sub/v.mp4"), Path::new("/c/mix/m.m4a"), Path::new("/c/sub/o.mp4"),
        );
        assert_eq!(
            strv(&mux),
            ["-y", "-v", "error", "-i", "/c/sub/v.mp4", "-i", "/c/mix/m.m4a",
             "-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", "copy", "/c/sub/o.mp4"]
        );
    }

    #[test]
    fn encode_output_path_sanitizes_hostile_slug() {
        let (plan, _) = test_plan(base_clips());
        assert_eq!(
            encode_output_path(&plan),
            Path::new("/w").join("06_成片输出").join("final_cutforge_demo_1080x1920.mp4")
        );
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "bad*slug:<>", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": []}]
        }))
        .unwrap();
        let plan = RenderPlan::build(&project, Path::new("/w"), None);
        assert_eq!(
            encode_output_path(&plan).file_name().unwrap().to_string_lossy(),
            "final_cutforge_bad_slug____1080x1920.mp4"
        );
    }

    #[test]
    fn fmt_f64_trims_trailing_zeros() {
        assert_eq!(fmt_f64(2.0), "2");
        assert_eq!(fmt_f64(1.5), "1.5");
        assert_eq!(fmt_f64(0.0), "0");
    }
}
