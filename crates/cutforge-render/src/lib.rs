//! CutForge 渲染后端(计划书 6.2;V2 M11 矩阵口径):
//! 统一帧率 → 逐段提取(变速/冻结/punch-in/转场尾帧扩展)→ xfade 链合成 →
//! overlay 合成 → 混音(逐段落点/BGM ducking/afade/loudnorm 双 pass)→
//! **字幕最后叠** → 编码。
//! 段缓存键 = hash(clip JSON)+版本+canvas+fps;混音缓存键与画幅无关
//! (多画幅变体真·分叉:共享 mix 只重做 video/encode)。

use cutforge_core::model::{Clip, Project, TrackKind};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 渲染器语义版本:任何渲染行为变更都必须 +1(缓存失效的正确来源)。
pub const RENDERER_VERSION: &str = "cutforge-render-3.0";

/// 混音段:一个需要进入总线的音频事件(人声段或音效),带源域裁剪与成片域落点。
struct AudioSeg {
    src: PathBuf,
    /// 成片域落点(ms)——adelay 的唯一来源(P0-4 修复前人声全部 0 秒起播)。
    start_ms: u64,
    duration_ms: u64,
    /// 源域入点(ms)——per-clip 裁剪,-ss 输入侧。
    source_in_ms: u64,
    volume: f64,
    speed: f64,
    fade_in_ms: f64,
    fade_out_ms: f64,
}

struct OverlaySeg {
    src: PathBuf,
    start_ms: u64,
    duration_ms: u64,
    spec: cutforge_core::model::Overlay,
}

pub struct RenderOutcome {
    pub output: PathBuf,
    pub steps: Vec<(&'static str, bool)>,
    pub cache_hits: usize,
    pub segments: usize,
    /// 混音缓存命中(多画幅变体真分叉的判据)
    pub mix_cache_hit: bool,
}

fn run_ff(tool: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(tool).args(args).output().map_err(|e| format!("启动 {tool} 失败: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{tool} 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim().chars().take(800).collect::<String>()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// 双通道捕获版本(loudnorm 测量输出在 stderr)。
fn run_ff_capture(tool: &str, args: &[&str]) -> Result<(String, String), String> {
    let out = Command::new(tool).args(args).output().map_err(|e| format!("启动 {tool} 失败: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{tool} 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim().chars().take(300).collect::<String>()
        ));
    }
    Ok((
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

fn run_ff_in(dir: &Path, tool: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(tool).args(args).current_dir(dir).output().map_err(|e| format!("启动 {tool} 失败: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{tool} 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim().chars().take(800).collect::<String>()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn hash_text(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

fn ffprobe_duration_sec(path: &Path) -> Result<f64, String> {
    let out = run_ff(
        "ffprobe",
        &["-v", "error", "-print_format", "json", "-show_format", &path.to_string_lossy()],
    )?;
    let v: Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    v["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "缺 format.duration".into())
}

/// 输出文件名消毒:仅替换文件系统敌对字符,保留 CJK/字母数字/-_
fn sanitize_slug(slug: &str) -> String {
    slug.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect()
}

/// clip i 的出向转场时长(ms;转场字段在 clip i 上表示 i-1→i 的转场,
/// i=0 无意义)。type=cut/none 或 durMs<=0 → 硬切(None)。
fn transition_out_ms(clip: &Clip) -> Option<(String, f64)> {
    if clip.start_ms == 0 && clip.transition.is_none() {
        return None;
    }
    let t = clip.transition.as_ref()?;
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

/// atempo 链(单实例只支持 0.5–2.0;speed ∈ [0.25,4] 用链式分解)
fn atempo_chain(speed: f64) -> Vec<String> {
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

pub fn render(
    project: &Project,
    project_dir: &Path,
    ass_path: Option<&Path>,
    progress: &mut dyn FnMut(Value),
) -> Result<RenderOutcome, String> {
    let mut steps = Vec::new();
    let out_dir = project_dir.join("06_output");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let cache_dir = project_dir.join(".cutforge/render-cache");
    std::fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
    let canvas_w = project.canvas.width;
    let canvas_h = project.canvas.height;

    // ---- 步 1 probe(容错:图片等无时长素材合法存在;真坏了会在 segment 步炸) ----
    for t in &project.tracks {
        for c in &t.clips {
            if let Some(src) = &c.src {
                let p = project_dir.join(src);
                if p.is_file() {
                    let _ = ffprobe_duration_sec(&p);
                }
            }
        }
    }
    steps.push(("probe", true));
    progress(json!({"step": "probe", "ok": true}));

    // ---- 收集:视频段 / 音频段 / overlay 段 ----
    let mut cache_hits = 0usize;
    let mut segments = 0usize;
    let mut video_clips: Vec<Clip> = Vec::new();
    let mut audio_segs: Vec<AudioSeg> = Vec::new();
    let mut overlay_segs: Vec<OverlaySeg> = Vec::new();
    for t in &project.tracks {
        for c in &t.clips {
            let Some(src) = c.src.clone() else { continue };
            let src_path = project_dir.join(&src);
            match t.kind {
                TrackKind::Video => {
                    segments += 1;
                    // overlay 字段的 clip 是**叠加层**(rs_brand 变体轨口径),
                    // 不占用主时间线 concat 序列,由 overlay 步合成
                    if c.overlay.is_some() {
                        if let (Some(ov), Some(src)) = (c.overlay, c.src.clone()) {
                            overlay_segs.push(OverlaySeg {
                                src: project_dir.join(src),
                                start_ms: c.start_ms,
                                duration_ms: c.duration_ms,
                                spec: ov,
                            });
                        }
                        continue;
                    }
                    video_clips.push(c.clone());
                    if c.volume.unwrap_or(0.0) > 0.0 {
                        audio_segs.push(AudioSeg {
                            src: src_path.clone(),
                            start_ms: c.start_ms,
                            duration_ms: c.duration_ms,
                            source_in_ms: c.source_in_ms.unwrap_or(0),
                            volume: c.volume.unwrap_or(1.0),
                            speed: c.speed.unwrap_or(1.0),
                            fade_in_ms: c.fade.as_ref().map(|f| f.in_ms).unwrap_or(0.0),
                            fade_out_ms: c.fade.as_ref().map(|f| f.out_ms).unwrap_or(0.0),
                        });
                    }
                }
                TrackKind::Audio => {
                    if c.volume.unwrap_or(0.0) > 0.0 {
                        audio_segs.push(AudioSeg {
                            src: src_path,
                            start_ms: c.start_ms,
                            duration_ms: c.duration_ms,
                            source_in_ms: c.source_in_ms.unwrap_or(0),
                            volume: c.volume.unwrap_or(if c.role == Some(cutforge_core::model::Role::Sfx) { 0.8 } else { 1.0 }),
                            speed: c.speed.unwrap_or(1.0),
                            fade_in_ms: c.fade.as_ref().map(|f| f.in_ms).unwrap_or(0.0),
                            fade_out_ms: c.fade.as_ref().map(|f| f.out_ms).unwrap_or(0.0),
                        });
                    }
                }
                TrackKind::Text => {
                    // 文本轨:结构性锚点(字幕经 ASS 烧录链);渲染端不消费,与 CutFlow 同口径
                }
            }
        }
    }

    // ---- 步 2 segment(视频段:P0-3 键含 canvas+fps;M11:变速 setpts + punch-in + 转场尾帧扩展) ----
    let mut segment_files: Vec<PathBuf> = Vec::new();
    let mut seg_durs_ms: Vec<u64> = Vec::new(); // 实际输出时长(含尾帧扩展)
    for (i, clip) in video_clips.iter().enumerate() {
        let src_path = project_dir.join(clip.src.clone().unwrap_or_default());
        // 尾帧扩展(ADR-0023):clip i 的转场由 seg_{i} 与 seg_{i+1} 之间的 xfade 消费,
        // 段 i 需要延长 D_{i+1} 保证整链零时间漂移
        let tail_ms: f64 = if i + 1 < video_clips.len() {
            transition_out_ms(&video_clips[i + 1]).map(|(_, d)| d).unwrap_or(0.0)
        } else {
            0.0
        };
        let speed = clip.speed.unwrap_or(1.0);
        let clip_json = serde_json::to_string(clip).unwrap_or_default();
        let key = format!(
            "{:x}-{}-{}x{}f{}",
            hash_text(&clip_json), RENDERER_VERSION, canvas_w, canvas_h, project.fps
        );
        let seg = cache_dir.join(format!("seg-{key}.mp4"));
        if seg.is_file() {
            cache_hits += 1;
        } else {
            let ss = clip.source_in_ms.unwrap_or(0);
            let read_ms = (clip.duration_ms as f64 * speed).ceil();
            let base_filters = format!(
                "scale={canvas_w}:{canvas_h}:force_original_aspect_ratio=decrease,pad={canvas_w}:{canvas_h}:(ow-iw)/2:(oh-ih)/2,fps={}",
                project.fps
            );
            // punch-in(ADR v0.11 R3):取紧构图——中心裁剪 factor 倍后放大回画布(静态)
            let mut vf = match &clip.punch_in {
                Some(p) => {
                    let f = p.factor.clamp(1.0, 2.0);
                    format!(
                        "{base_filters},crop=w=iw/{f}:h=ih/{f}:x=(iw-ow)/2:y=(ih-oh)/2,scale={canvas_w}:{canvas_h}"
                    )
                }
                None => base_filters,
            };
            if speed != 1.0 {
                vf.push_str(&format!(",setpts=PTS/{}", fmt_f64(speed)));
            }
            if tail_ms > 0.0 {
                // 冻结尾帧扩展(转场零时间漂移的关键,ADR-0023)
                vf.push_str(&format!(",tpad=stop_mode=clone:stop_duration={:.6}", tail_ms / 1000.0));
            }
            let args: Vec<String> = vec![
                "-y".into(), "-v".into(), "error".into(),
                "-ss".into(), format!("{}", ss as f64 / 1000.0), // -ss/-t 均在输入侧
                "-t".into(), format!("{}", read_ms / 1000.0),
                "-i".into(), src_path.to_string_lossy().into(),
                "-vf".into(), vf,
                "-an".into(), "-c:v".into(), "libx264".into(), "-preset".into(), "veryfast".into(),
                seg.to_string_lossy().into(),
            ];
            run_ff("ffmpeg", &args.iter().map(|s| s.as_str()).collect::<Vec<_>>())?;
        }
        segment_files.push(seg);
        seg_durs_ms.push(clip.duration_ms + tail_ms as u64);
    }
    steps.push(("segment", true));
    progress(json!({"step": "segment", "ok": true, "cacheHits": cache_hits, "segments": segments}));

    // ---- 步 3 合成(xfade 链;无转场时退化为 concat) ----
    let composed = cache_dir.join("composed.mp4");
    let has_transitions = video_clips.iter().enumerate().any(|(i, c)| i > 0 && transition_out_ms(c).is_some());
    if has_transitions {
        // xfade 链:offset_k = 前序真实时长累计(尾帧扩展保证零漂移)
        let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];
        for f in &segment_files {
            args.extend(["-i".into(), f.to_string_lossy().into()]);
        }
        let mut filters: Vec<String> = Vec::new();
        let mut acc_ms = 0f64;
        let mut cur_label = "[0:v]".to_string();
        for i in 1..video_clips.len() {
            let (kind, dur) = transition_out_ms(&video_clips[i])
                .unwrap_or_else(|| ("fade".into(), 0.0));
            acc_ms += seg_durs_ms[i - 1] as f64;
            let offset = acc_ms / 1000.0;
            let out_label = format!("[x{i}]");
            filters.push(format!(
                "{cur_label}[{i}:v]xfade=transition={kind}:duration={:.6}:offset={:.6}{out_label}",
                dur / 1000.0,
                offset
            ));
            cur_label = out_label;
        }
        args.extend(["-filter_complex".into(), filters.join(";"), "-map".into(), "[x LAST]".into()]);
        let last_label = format!("[x{}]", video_clips.len() - 1);
        let mut args: Vec<String> = args
            .into_iter()
            .map(|a| if a == "[x LAST]" { last_label.clone() } else { a })
            .collect();
        args.extend(["-c:v".into(), "libx264".into(), "-preset".into(), "veryfast".into(), composed.to_string_lossy().into()]);
        run_ff("ffmpeg", &args.iter().map(|s| s.as_str()).collect::<Vec<_>>())?;
    } else {
        let concat_list = cache_dir.join("concat.txt");
        let mut list = String::new();
        for f in &segment_files {
            list.push_str(&format!("file '{}'\n", f.to_string_lossy().replace('\\', "/")));
        }
        cutforge_io::atomic::atomic_write(&concat_list, list.as_bytes()).map_err(|e| e.to_string())?;
        run_ff("ffmpeg", &[
            "-y", "-v", "error", "-f", "concat", "-safe", "0",
            "-i", concat_list.to_string_lossy().as_ref(),
            "-c", "copy", composed.to_string_lossy().as_ref(),
        ])?;
    }
    steps.push(("compose-video", true));
    progress(json!({"step": "compose-video", "ok": true}));

    // ---- 步 4 overlay 合成(品牌/花字位图;绝对像素 + opacity + 时间窗) ----
    let mut base_video = composed.clone();
    if !overlay_segs.is_empty() {
        let overlaid = cache_dir.join("overlaid.mp4");
        let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into(), "-i".into(), composed.to_string_lossy().into()];
        for ov in &overlay_segs {
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
        args.extend(["-filter_complex".into(), filters.join(";"), "-map".into(), cur, "-c:v".into(), "libx264".into(), "-preset".into(), "veryfast".into(), overlaid.to_string_lossy().into()]);
        run_ff("ffmpeg", &args.iter().map(|s| s.as_str()).collect::<Vec<_>>())?;
        base_video = overlaid;
    }
    steps.push(("overlay", true));
    progress(json!({"step": "overlay", "ok": true, "count": overlay_segs.len()}));

    // ---- 步 5 mix(画幅无关 → 共享缓存;多画幅变体真分叉) ----
    let total_ms: u64 = project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .map(|c| c.start_ms + c.duration_ms)
        .max()
        .unwrap_or(0);
    let mix_spec = json!({
        "segs": audio_segs.iter().map(|s| (s.src.to_string_lossy(), s.start_ms, s.duration_ms, s.source_in_ms, s.volume, s.speed, s.fade_in_ms, s.fade_out_ms)).collect::<Vec<_>>(),
        "bgm": project.bgm,
        "total": total_ms,
        "v": RENDERER_VERSION,
    });
    let mix_key = format!("{:x}", hash_text(&mix_spec.to_string()));
    let mixed = cache_dir.join(format!("mix-{mix_key}.m4a"));
    let mut mix_cache_hit = false;
    if mixed.is_file() {
        mix_cache_hit = true;
        steps.push(("mix", true));
        progress(json!({"step": "mix", "ok": true, "cacheHit": true}));
    } else {
        // pass A:逐段(-ss/-t 裁剪 + atempo 变速 + volume + afade + adelay 落点)→ 总线 → BGM ducking
        let mixed_raw = cache_dir.join(format!("mix-raw-{mix_key}.m4a"));
        let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];
        let mut filters: Vec<String> = Vec::new();
        let mut labels: Vec<String> = Vec::new();
        if audio_segs.is_empty() && project.bgm.is_none() {
            args.extend(["-f".into(), "lavfi".into(), "-i".into(), "anullsrc=r=48000:cl=stereo".into()]);
        }
        let mut input_idx = 0usize;
        for seg in &audio_segs {
            let read_ms = (seg.duration_ms as f64 * seg.speed).ceil();
            args.extend([
                "-ss".into(), format!("{}", seg.source_in_ms as f64 / 1000.0),
                "-t".into(), format!("{}", read_ms / 1000.0),
                "-i".into(), seg.src.to_string_lossy().into(),
            ]);
            let mut chain = format!(
                "[{input_idx}:a]aformat=sample_rates=48000:channel_layouts=stereo"
            );
            if seg.speed != 1.0 {
                for tempo in atempo_chain(seg.speed) {
                    chain.push_str(&format!(",{tempo}"));
                }
            }
            chain.push_str(&format!(",volume={:.4}", seg.volume));
            if seg.fade_in_ms > 0.0 {
                chain.push_str(&format!(",afade=t=in:st=0:d={:.3}", seg.fade_in_ms / 1000.0));
            }
            if seg.fade_out_ms > 0.0 {
                let st = (seg.duration_ms as f64 - seg.fade_out_ms).max(0.0) / 1000.0;
                chain.push_str(&format!(",afade=t=out:st={st:.3}:d={:.3}", seg.fade_out_ms / 1000.0));
            }
            chain.push_str(&format!(",adelay={}:all=1[a{input_idx}]", seg.start_ms));
            filters.push(chain);
            labels.push(format!("[a{input_idx}]"));
            input_idx += 1;
        }
        // 总线([bus] 恒存在:bgm-only 工程用 anullsrc 占位)
        if audio_segs.is_empty() && project.bgm.is_some() {
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
        if let Some(bgm) = &project.bgm {
            let bgm_path = project_dir.join(&bgm.src);
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
                filters.push(format!(
                    "[bgmg][bus]sidechaincompress=threshold=0.03:ratio=8:attack=80:release=500[bgmc]"
                ));
                filters.push(format!("[bus][bgmc]amix=inputs=2:duration=first:normalize=0[mixout]"));
            } else {
                filters.push(format!("[bus][bgmg]amix=inputs=2:duration=first:normalize=0[mixout]"));
            }
            args.extend(["-filter_complex".into(), filters.join(";"), "-map".into(), "[mixout]".into()]);
        } else if !audio_segs.is_empty() {
            args.extend(["-filter_complex".into(), filters.join(";"), "-map".into(), "[bus]".into()]);
        }
        if project.bgm.is_none() && audio_segs.is_empty() {
            // anullsrc 路径:输入即静音源,无 filter_complex
        }
        args.extend([
            "-t".into(), format!("{}", total_ms as f64 / 1000.0),
            "-c:a".into(), "aac".into(), mixed_raw.to_string_lossy().into(),
        ]);
        run_ff("ffmpeg", &args.iter().map(|s| s.as_str()).collect::<Vec<_>>())?;

        // pass B:响度(先测后编 linear=true)
        let (_m_out, m_err) = run_ff_capture("ffmpeg", &[
            "-hide_banner", "-nostats", "-i", mixed_raw.to_string_lossy().as_ref(),
            "-filter_complex", "loudnorm=I=-14:TP=-1.0:print_format=json",
            "-f", "null", "-",
        ])?;
        let m_start = m_err.rfind('{').ok_or("loudnorm 测量输出无 JSON".to_string())?;
        let m_end = m_err.rfind('}').ok_or("loudnorm 测量输出无 JSON".to_string())? + 1;
        let measured: Value = serde_json::from_str(&m_err[m_start..m_end]).map_err(|e| e.to_string())?;
        let ln = format!(
            "loudnorm=I=-14:TP=-1.0:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:linear=true",
            measured["input_i"].as_str().unwrap_or("-14"),
            measured["input_tp"].as_str().unwrap_or("-1"),
            measured["input_lra"].as_str().unwrap_or("0"),
            measured["input_thresh"].as_str().unwrap_or("-30"),
        );
        run_ff("ffmpeg", &[
            "-y", "-v", "error", "-i", mixed_raw.to_string_lossy().as_ref(),
            "-af", &ln,
            "-c:a", "aac", mixed.to_string_lossy().as_ref(),
        ])?;
        steps.push(("mix", true));
        progress(json!({"step": "mix", "ok": true, "cacheHit": false}));
    }

    // ---- 步 6 subtitle(最后叠;有 ass 才烧) ----
    let mut video_input = base_video.clone();
    let subbed = cache_dir.join("subbed.mp4");
    if let Some(ass) = ass_path {
        let ass_local = cache_dir.join("burn.ass");
        let payload = std::fs::read(ass).map_err(|e| e.to_string())?;
        cutforge_io::atomic::atomic_write(&ass_local, &payload).map_err(|e| e.to_string())?;
        run_ff_in(&cache_dir, "ffmpeg", &[
            "-y", "-v", "error", "-i", video_input.to_string_lossy().as_ref(),
            "-i", mixed.to_string_lossy().as_ref(),
            "-vf", "subtitles=burn.ass",
            "-map", "0:v", "-map", "1:a",
            "-c:v", "libx264", "-preset", "veryfast", "-c:a", "copy",
            subbed.to_string_lossy().as_ref(),
        ])?;
        video_input = subbed;
    } else {
        run_ff("ffmpeg", &[
            "-y", "-v", "error", "-i", video_input.to_string_lossy().as_ref(),
            "-i", mixed.to_string_lossy().as_ref(),
            "-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", "copy",
            subbed.to_string_lossy().as_ref(),
        ])?;
        video_input = subbed;
    }
    steps.push(("subtitle", true));
    progress(json!({"step": "subtitle", "ok": true, "burned": ass_path.is_some()}));

    // ---- 步 7 encode(文件名消毒) ----
    let output = out_dir.join(format!(
        "final_cutforge_{}_{}.mp4",
        sanitize_slug(&project.slug),
        format!("{}x{}", canvas_w, canvas_h)
    ));
    let payload = std::fs::read(&video_input).map_err(|e| e.to_string())?;
    cutforge_io::atomic::atomic_write(&output, &payload).map_err(|e| e.to_string())?;
    steps.push(("encode", true));
    progress(json!({"step": "encode", "ok": true, "output": output.to_string_lossy()}));

    Ok(RenderOutcome { output, steps, cache_hits, segments, mix_cache_hit })
}

fn fmt_f64(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s.is_empty() { "0".into() } else { s }
}

/// headless 批量:变体矩阵(比例 × 组合)。段缓存键含 canvas;混音键画幅无关 →
/// 多画幅共享 mix,只重做 video 链与 encode(真分叉,6.5)。
pub fn render_variants(project: &Project, project_dir: &Path, ratios: &[&str]) -> Vec<(String, Result<PathBuf, String>)> {
    ratios
        .iter()
        .map(|r| {
            let mut p = project.clone();
            p.canvas = match *r {
                "3x4" => cutforge_core::model::Canvas { width: 1080, height: 1440 },
                "16x9" => cutforge_core::model::Canvas { width: 1920, height: 1080 },
                _ => cutforge_core::model::Canvas { width: 1080, height: 1920 },
            };
            match render(&p, project_dir, None, &mut |_| {}) {
                Ok(o) => (r.to_string(), Ok(o.output)),
                Err(e) => (r.to_string(), Err(e)),
            }
        })
        .collect()
}

pub fn write_progress(v: Value) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{v}");
}
