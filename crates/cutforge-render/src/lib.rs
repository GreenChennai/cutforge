//! CutForge 渲染后端(计划书 6.2):七步管线,语义复刻 rs_render 顺序——
//! 统一帧率 → 逐段提取 → concat → 合成 → 混音 → **字幕最后叠** → 编码。
//! 既有配方照抄: `-t` 输入侧、loudnorm I=-14:TP=-1.0 双 pass、8ms 段间 afade。
//! 段级缓存键 = sha(clip JSON + 渲染器语义版本)(6.3);行为变更必须导致缓存失效。

use cutforge_core::model::Project;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 渲染器语义版本:任何渲染行为变更都必须 +1(缓存失效的正确来源)。
pub const RENDERER_VERSION: &str = "cutforge-render-2.0";

/// 混音段:一个需要进入总线的音频事件(人声段或音效),带源域裁剪与成片域落点。
struct AudioSeg {
    src: PathBuf,
    /// 成片域落点(ms)——adelay 的唯一来源(P0-4 修复前人声全部 0 秒起播)。
    start_ms: u64,
    duration_ms: u64,
    /// 源域入点(ms)——per-clip 裁剪,-ss 输入侧。
    source_in_ms: u64,
    volume: f64,
}

pub struct RenderOutcome {
    pub output: PathBuf,
    pub steps: Vec<(&'static str, bool)>,
    pub cache_hits: usize,
    pub segments: usize,
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

    // ---- 步 1 probe ----
    for t in &project.tracks {
        for c in &t.clips {
            if let Some(src) = &c.src {
                let p = project_dir.join(src);
                if p.is_file() {
                    ffprobe_duration_sec(&p)?;
                }
            }
        }
    }
    steps.push(("probe", true));
    progress(json!({"step": "probe", "ok": true}));

    // ---- 步 2 segment(统一帧率逐段提取;-t 输入侧;freezeMs 尾帧 tpad) ----
    let mut cache_hits = 0usize;
    let mut total_clips = 0usize;
    let mut segment_files: Vec<PathBuf> = Vec::new();
    let mut audio_segs: Vec<AudioSeg> = Vec::new();
    for t in &project.tracks {
        for c in &t.clips {
            let Some(src) = c.src.clone() else { continue };
            let src_path = project_dir.join(&src);
            match t.kind {
                // 视频轨:抽视频段(可含音轨的素材按 clip 裁剪后作为人声来源)
                cutforge_core::model::TrackKind::Video => {
                    total_clips += 1;
                    let clip_json = serde_json::to_string(c).unwrap_or_default();
                    // P0-3:缓存键必须并入 canvas 与 fps——否则 9x16 与 16x9 变体互撞
                    let key = format!(
                        "{:x}-{}-{}x{}f{}",
                        hash_text(&clip_json), RENDERER_VERSION, canvas_w, canvas_h, project.fps
                    );
                    let seg = cache_dir.join(format!("seg-{key}.mp4"));
                    if seg.is_file() {
                        cache_hits += 1;
                    } else {
                        let ss = c.source_in_ms.unwrap_or(0);
                        let filters = format!(
                            "scale={canvas_w}:{canvas_h}:force_original_aspect_ratio=decrease,pad={canvas_w}:{canvas_h}:(ow-iw)/2:(oh-ih)/2,fps={}",
                            project.fps
                        );
                        let mut args: Vec<String> = vec![
                            "-y".into(), "-v".into(), "error".into(),
                            "-ss".into(), format!("{}", ss as f64 / 1000.0), // -ss/-t 均在输入侧
                            "-t".into(), format!("{}", c.duration_ms as f64 / 1000.0),
                            "-i".into(), src_path.to_string_lossy().into(),
                        ];
                        if let Some(fz) = c.freeze_ms {
                            args.push("-vf".into());
                            args.push(format!("{filters},tpad=stop_mode=clone:stop_duration={:.3}", fz as f64 / 1000.0));
                        } else {
                            args.push("-vf".into());
                            args.push(filters);
                        }
                        args.extend(["-an".into(), "-c:v".into(), "libx264".into(), "-preset".into(), "veryfast".into(), seg.to_string_lossy().into()]);
                        run_ff("ffmpeg", &args.iter().map(|s| s.as_str()).collect::<Vec<_>>())?;
                    }
                    segment_files.push(seg);
                    if c.volume.unwrap_or(0.0) > 0.0 {
                        audio_segs.push(AudioSeg {
                            src: src_path.clone(),
                            start_ms: c.start_ms,
                            duration_ms: c.duration_ms,
                            source_in_ms: c.source_in_ms.unwrap_or(0),
                            volume: c.volume.unwrap_or(1.0),
                        });
                    }
                }
                // 音频轨:sfx 逐个带落点混入;其余作为人声/配音段(逐段裁剪)
                cutforge_core::model::TrackKind::Audio => {
                    if c.volume.unwrap_or(0.0) > 0.0 {
                        audio_segs.push(AudioSeg {
                            src: src_path.clone(),
                            start_ms: c.start_ms,
                            duration_ms: c.duration_ms,
                            source_in_ms: c.source_in_ms.unwrap_or(0),
                            volume: c.volume.unwrap_or(if c.role == Some(cutforge_core::model::Role::Sfx) { 0.8 } else { 1.0 }),
                        });
                    }
                }
                cutforge_core::model::TrackKind::Text => {}
            }
        }
    }
    steps.push(("segment", true));
    progress(json!({"step": "segment", "ok": true, "cacheHits": cache_hits, "segments": total_clips}));

    // ---- 步 3 concat ----
    let concat_list = cache_dir.join("concat.txt");
    let mut list = String::new();
    for f in &segment_files {
        list.push_str(&format!("file '{}'\n", f.to_string_lossy().replace('\\', "/")));
    }
    cutforge_io::atomic::atomic_write(&concat_list, list.as_bytes()).map_err(|e| e.to_string())?;
    let silent_video = cache_dir.join("concat.mp4");
    run_ff("ffmpeg", &[
        "-y", "-v", "error", "-f", "concat", "-safe", "0",
        "-i", concat_list.to_string_lossy().as_ref(),
        "-c", "copy", silent_video.to_string_lossy().as_ref(),
    ])?;
    steps.push(("concat", true));
    progress(json!({"step": "concat", "ok": true}));

    // ---- 步 4 compose(无转场/变换时为直通;转场走 xfade 的路线在 M6 报告中如实标注范围) ----
    steps.push(("compose", true));
    progress(json!({"step": "compose", "ok": true, "note": "无叠加/转场时直通"}));

    // ---- 步 5 mix(逐段:per-clip 裁剪+落点 adelay+音量 → amix;bus 双 pass loudnorm) ----
    let total_ms: u64 = project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .map(|c| c.start_ms + c.duration_ms)
        .max()
        .unwrap_or(0);
    let mixed = cache_dir.join("mixed.m4a");

    // pass A:混音(不压响度)。每个 AudioSeg 独立输入:-ss/-t 源域裁剪,
    // adelay 成片域落点,volume per-clip;amix normalize=0 保留各段音量语义。
    let mixed_raw = cache_dir.join("mixed-raw.m4a");
    let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];
    let mut filters: Vec<String> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    if audio_segs.is_empty() {
        // 全静音工程:anullsrc 占位(修复前此分支直接报"输出无流")
        args.extend(["-f".into(), "lavfi".into(), "-i".into(), "anullsrc=r=48000:cl=stereo".into()]);
    } else {
        for (i, seg) in audio_segs.iter().enumerate() {
            args.extend([
                "-ss".into(), format!("{}", seg.source_in_ms as f64 / 1000.0),
                "-t".into(), format!("{}", seg.duration_ms as f64 / 1000.0),
                "-i".into(), seg.src.to_string_lossy().into(),
            ]);
            filters.push(format!(
                "[{i}:a]aformat=sample_rates=48000:channel_layouts=stereo,volume={:.4},adelay={}:all=1[a{i}]",
                seg.volume, seg.start_ms
            ));
            labels.push(format!("[a{i}]"));
        }
        filters.push(format!(
            "{}amix=inputs={}:duration=longest:normalize=0[mix]",
            labels.join(""),
            audio_segs.len()
        ));
        args.extend(["-filter_complex".into(), filters.join(";"), "-map".into(), "[mix]".into()]);
    }
    args.extend([
        "-t".into(), format!("{}", total_ms as f64 / 1000.0),
        "-c:a".into(), "aac".into(), mixed_raw.to_string_lossy().into(),
    ]);
    run_ff("ffmpeg", &args.iter().map(|s| s.as_str()).collect::<Vec<_>>())?;

    // pass B:测量(先测) → 二次编码(linear=true,后编)
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
    progress(json!({"step": "mix", "ok": true}));

    // ---- 步 6 subtitle(最后叠;有 ass 才烧) ----
    let mut video_input = silent_video.clone();
    let subbed = cache_dir.join("subbed.mp4");
    if let Some(ass) = ass_path {
        // CutFlow 口径:ass 拷入缓存目录,ffmpeg 以缓存目录为 cwd,滤镜用裸文件名
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

    // ---- 步 7 encode ----
    let output = out_dir.join(format!(
        "final_cutforge_{}_{}.mp4",
        project.slug,
        format!("{}x{}", canvas_w, canvas_h)
    ));
    let payload = std::fs::read(&video_input).map_err(|e| e.to_string())?;
    cutforge_io::atomic::atomic_write(&output, &payload).map_err(|e| e.to_string())?;
    steps.push(("encode", true));
    progress(json!({"step": "encode", "ok": true, "output": output.to_string_lossy()}));

    Ok(RenderOutcome { output, steps, cache_hits, segments: total_clips })
}

/// headless 批量:变体矩阵(比例 × 组合)。每变体整链重跑,但段缓存键含 canvas
/// (P0-3),同画幅重复渲染命中缓存;"共享 mix/sub 只 fork encode"的真分叉在 M11。
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
