// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! CutForge 渲染后端(计划书 6.2;V2 M11 矩阵口径;T1.4 分解为 RenderPlan + 步骤函数):
//! probe → segment → compose → overlay → mix → subtitle → encode。
//! 每步 = steps.rs 纯函数(生成 ffmpeg 参数,可离线单测)+ 本文件执行器(调进程),
//! 产出结构化 StepReport;进度事件由 StepReport 单源派生(stdout JSON 行接口不变)。
//! 缓存(T1.5):中间产物全量内容寻址,分层见 cache.rs;最终成片路径与格式不变。
//! 单帧(T2.4):render_frame 对指定时间点出一帧合成画面(frame 层缓存键含
//! 工作区指纹),供壳「精确预览」;见 frame.rs。

pub mod across;
pub mod cache;
pub mod catalog;
pub mod frame;
pub mod plan;
pub mod segment;
pub mod steps;

pub use cache::{
    cache_gc, cache_info, CacheEntry, CacheIndex, GcReport, DEFAULT_CAPACITY_BYTES,
};
pub use frame::{frame_done_event, frame_extract_args, quantize_ms, render_frame, FrameFormat, FrameOutcome};
pub use plan::{RenderPlan, StepReport, STEP_NAMES};

use cutforge_core::model::Project;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 渲染器语义版本:任何渲染行为变更都必须 +1(缓存失效的正确来源)。
/// 4.0:T1.4 render() 分解 + T1.5 缓存全量内容寻址(分层目录 + cache-index.json)。
/// 5.0(册四 A4-BE3a):T4.5 转场目录直通 + xfade offset 修复为名义时长口径
/// (旧键渲染产物错误,必须整体失效)+ T4.6 fx/motion 段链 + acrossfade 音频链。
pub const RENDERER_VERSION: &str = "cutforge-render-5.0";

pub struct RenderOutcome {
    pub output: PathBuf,
    pub steps: Vec<(&'static str, bool)>,
    pub cache_hits: usize,
    /// 段缓存未命中数(AC-1.4:改 clip 后重渲同键必 miss 的判据)。
    pub cache_misses: usize,
    pub segments: usize,
    /// 混音缓存命中(多画幅变体真分叉的判据)
    pub mix_cache_hit: bool,
}

/// E5-2/B13:ffmpeg/ffprobe 定位可配置——env CUTFORGE_FFMPEG / CUTFORGE_FFPROBE 优先,
/// 缺省按 PATH 名调用(与 CutFlow 侧 WPI_FFMPEG 口径对齐;Windows 不再把 ffmpeg 塞 PATH 不可导出)。
pub fn ff_bin(tool: &str) -> String {
    let key = if tool == "ffmpeg" { "CUTFORGE_FFMPEG" } else { "CUTFORGE_FFPROBE" };
    if let Some(v) = std::env::var_os(key)
        && !v.is_empty() {
            return v.to_string_lossy().into_owned();
        }
    tool.to_string()
}

fn run_ff(tool: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(ff_bin(tool)).args(args).output().map_err(|e| format!("启动 {tool} 失败: {e}"))?;
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
    let out = Command::new(ff_bin(tool)).args(args).output().map_err(|e| format!("启动 {tool} 失败: {e}"))?;
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
    let out = Command::new(ff_bin(tool)).args(args).current_dir(dir).output().map_err(|e| format!("启动 {tool} 失败: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{tool} 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim().chars().take(800).collect::<String>()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
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

/// Vec<String> 命令行 → &[&str](run_ff 适配)。
fn strs(args: &[String]) -> Vec<&str> {
    args.iter().map(|s| s.as_str()).collect()
}

fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

// ---------------- 主编排:步骤列表按 plan::STEP_NAMES 顺序执行 ----------------

pub fn render(
    project: &Project,
    project_dir: &Path,
    ass_path: Option<&Path>,
    progress: &mut dyn FnMut(Value),
) -> Result<RenderOutcome, String> {
    let plan = RenderPlan::build(project, project_dir, ass_path);
    std::fs::create_dir_all(&plan.out_dir).map_err(|e| e.to_string())?;
    cache::ensure_dirs(&plan.cache_dir)?;
    let mut idx = CacheIndex::load(&plan.cache_dir);
    let mut steps: Vec<(&'static str, bool)> = Vec::new();

    // ---- 步 1 probe(容错:图片等无时长素材合法存在;真坏了会在 segment 步炸) ----
    let rep = exec_probe(&plan);
    progress(rep.to_progress());
    steps.push((rep.name, rep.ok));

    // ---- 步 2 segment(内容寻址:键含 clip JSON + 尾帧 + 画幅 + fps) ----
    let (rep, seg_files, seg_keys, cache_hits, cache_misses) = exec_segment(&plan, &mut idx)?;
    progress(rep.to_progress());
    steps.push((rep.name, rep.ok));

    // ---- 步 3 compose(xfade 链 / concat 退化) ----
    let (rep, composed, compose_key) = exec_compose(&plan, &mut idx, &seg_files, &seg_keys)?;
    progress(rep.to_progress());
    steps.push((rep.name, rep.ok));

    // ---- 步 4 overlay(品牌/花字位图;无叠加层直接透传) ----
    let (rep, base_video, video_key) = exec_overlay(&plan, &mut idx, &composed, &compose_key)?;
    progress(rep.to_progress());
    steps.push((rep.name, rep.ok));

    // ---- 步 5 mix(画幅无关 → 共享缓存;多画幅变体真分叉) ----
    let (rep, mixed, mix_key_str, mix_cache_hit) = exec_mix(&plan, &mut idx)?;
    progress(rep.to_progress());
    steps.push((rep.name, rep.ok));

    // ---- 步 6 subtitle(最后叠;有 ass 才烧,否则零重编码合流) ----
    let (rep, video_input) = exec_subtitle(&plan, &mut idx, &base_video, &mixed, &video_key, &mix_key_str)?;
    progress(rep.to_progress());
    steps.push((rep.name, rep.ok));

    // ---- 步 7 encode(文件名消毒;原子拷贝到 06_成片输出,不走缓存) ----
    let (rep, output) = exec_encode(&plan, &video_input)?;
    progress(rep.to_progress());
    steps.push((rep.name, rep.ok));
    idx.save(&plan.cache_dir)?;

    Ok(RenderOutcome { output, steps, cache_hits, cache_misses, segments: plan.video_clips.len(), mix_cache_hit })
}

/// 步 1:探测素材(结果仅热身/容错,不改变管线)。
fn exec_probe(plan: &RenderPlan) -> StepReport {
    for p in steps::probe_paths(plan) {
        let _ = ffprobe_duration_sec(&p);
    }
    StepReport::new("probe", json!({}))
}

type SegOutputs = (StepReport, Vec<PathBuf>, Vec<String>, usize, usize);

/// 步 2:逐段提取(缓存命中即跳过;fx/motion 降级 WARN 并入进度事件)。
fn exec_segment(plan: &RenderPlan, idx: &mut CacheIndex) -> Result<SegOutputs, String> {
    let now = cache::now_secs();
    let mut seg_files: Vec<PathBuf> = Vec::new();
    let mut seg_keys: Vec<String> = Vec::new();
    let mut hits = 0usize;
    let mut misses = 0usize;
    let mut degradations: Vec<String> = Vec::new();
    for (i, clip) in plan.video_clips.iter().enumerate() {
        let tail = steps::segment_tail_ms(&plan.video_clips, i);
        let spec = cache::seg_spec(plan, clip, tail);
        let key = cache::seg_key(plan, clip, tail);
        degradations.extend(crate::catalog::clip_degradations(clip, plan.canvas_w, plan.canvas_h, plan.fps));
        let seg = match idx.touch("seg", &key, now) {
            Some(rel) => {
                hits += 1;
                plan.cache_dir.join(rel)
            }
            None => {
                misses += 1;
                let rel = idx.record("seg", &key, spec, now);
                let seg = plan.cache_dir.join(rel);
                let args = steps::segment_args(plan, clip, tail, &seg);
                run_ff("ffmpeg", &strs(&args))?;
                idx.set_size("seg", &key, file_size(&seg));
                seg
            }
        };
        seg_files.push(seg);
        seg_keys.push(key);
    }
    idx.save(&plan.cache_dir)?;
    let mut detail = json!({"cacheHits": hits, "segments": seg_files.len()});
    if !degradations.is_empty() {
        detail["warnings"] = json!(degradations);
    }
    let rep = StepReport::new("segment", detail);
    Ok((rep, seg_files, seg_keys, hits, misses))
}

/// 步 3:xfade 链 / concat 合成(缓存命中即跳过;转场降级 WARN 并入进度事件)。
fn exec_compose(
    plan: &RenderPlan,
    idx: &mut CacheIndex,
    seg_files: &[PathBuf],
    seg_keys: &[String],
) -> Result<(StepReport, PathBuf, String), String> {
    let now = cache::now_secs();
    let key = cache::compose_key(seg_keys);
    let mut tr_warns: Vec<String> = Vec::new();
    let composed = match idx.touch("compose", &key, now) {
        Some(rel) => plan.cache_dir.join(rel),
        None => {
            let rel = idx.record("compose", &key, cache::compose_spec(seg_keys), now);
            let composed = plan.cache_dir.join(rel);
            if steps::is_xfade_chain(&plan.video_clips) {
                let (args, warns) =
                    steps::compose_xfade_args(&plan.video_clips, seg_files, &composed);
                tr_warns = warns;
                run_ff("ffmpeg", &strs(&args))?;
            } else {
                // concat 清单写 tmp(内容寻址命名);用完即清。
                // 条目必须绝对化:concat demuxer 以**清单所在目录**解析相对路径,
                // --root 为相对路径时会拼出 .cutforge/render-cache/.cutforge/… 而炸
                // (拆分前旧 concat.txt 同样中招;只改临时清单内容,成片输出不变)。
                let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
                let abs_segs: Vec<PathBuf> = seg_files
                    .iter()
                    .map(|p| if p.is_absolute() { p.clone() } else { cwd.join(p) })
                    .collect();
                let list_rel = cache::tmp_rel(&format!("concat-{key}.txt"));
                let list_path = plan.cache_dir.join(&list_rel);
                cutforge_io::atomic::atomic_write(
                    &list_path,
                    steps::concat_list_content(&abs_segs).as_bytes(),
                )
                .map_err(|e| e.to_string())?;
                let args = steps::compose_concat_args(&list_path, &composed);
                run_ff("ffmpeg", &strs(&args))?;
                let _ = cutforge_io::atomic::remove(&list_path);
            }
            idx.set_size("compose", &key, file_size(&composed));
            composed
        }
    };
    idx.save(&plan.cache_dir)?;
    let mut detail = json!({});
    if !tr_warns.is_empty() {
        detail["warnings"] = json!(tr_warns);
    }
    Ok((StepReport::new("compose-video", detail), composed, key))
}

/// 步 4:overlay 合成(无叠加层直接透传基片;键 = compose 键 + 叠加清单)。
fn exec_overlay(
    plan: &RenderPlan,
    idx: &mut CacheIndex,
    composed: &Path,
    compose_key: &str,
) -> Result<(StepReport, PathBuf, String), String> {
    let now = cache::now_secs();
    if plan.overlay_segs.is_empty() {
        return Ok((StepReport::new("overlay", json!({"count": 0})), composed.to_path_buf(), compose_key.to_string()));
    }
    let key = cache::overlay_key(compose_key, &plan.overlay_segs);
    let (overlaid, hit) = match idx.touch("overlay", &key, now) {
        Some(rel) => (plan.cache_dir.join(rel), true),
        None => {
            let rel = idx.record("overlay", &key, cache::overlay_spec(compose_key, &plan.overlay_segs), now);
            let overlaid = plan.cache_dir.join(rel);
            let args = steps::overlay_args(&plan.overlay_segs, composed, &overlaid);
            run_ff("ffmpeg", &strs(&args))?;
            idx.set_size("overlay", &key, file_size(&overlaid));
            (overlaid, false)
        }
    };
    idx.save(&plan.cache_dir)?;
    Ok((StepReport::new("overlay", json!({"count": plan.overlay_segs.len()})).with_cache_hit(hit), overlaid, key))
}

type MixOutputs = (StepReport, PathBuf, String, bool);

/// 步 5:混音(pass A 逐段落点/变速/淡变 → 总线 → BGM ducking;pass B loudnorm)。
/// 画幅无关 → 多画幅变体共享(mix_cache_hit 即真分叉判据)。
fn exec_mix(plan: &RenderPlan, idx: &mut CacheIndex) -> Result<MixOutputs, String> {
    let now = cache::now_secs();
    let spec = cache::mix_spec(plan);
    let key = cache::mix_key(plan);
    if let Some(rel) = idx.touch("mix", &key, now) {
        let mixed = plan.cache_dir.join(rel);
        let rep = StepReport::new("mix", json!({"cacheHit": true})).with_cache_hit(true);
        return Ok((rep, mixed, key, true));
    }
    let rel = idx.record("mix", &key, spec, now);
    let mixed = plan.cache_dir.join(rel);
    // pass A:总线合成 → 原始混音(raw 为临时件,pass B 后即清)
    let raw = mixed.with_file_name(format!("{key}.raw.m4a"));
    let args = steps::mix_pass_a_args(plan, &raw);
    run_ff("ffmpeg", &strs(&args))?;

    // pass B:响度(先测后编 linear=true)。数字静音(-inf,无音频工程)跳过:
    // linear=true 遇 -inf 的 measured 值,ffmpeg 报 "Result too large" 直接失败。
    let (_m_out, m_err) = run_ff_capture(ff_bin("ffmpeg").as_str(), &strs(&steps::mix_measure_args(&raw)))?;
    let m_start = m_err.rfind('{').ok_or("loudnorm 测量输出无 JSON".to_string())?;
    let m_end = m_err.rfind('}').ok_or("loudnorm 测量输出无 JSON".to_string())? + 1;
    let measured: Value = serde_json::from_str(&m_err[m_start..m_end]).map_err(|e| e.to_string())?;
    if steps::mix_measured_is_loud(&measured) {
        let args = steps::mix_pass_b_args(&measured, &raw, &mixed);
        run_ff(ff_bin("ffmpeg").as_str(), &strs(&args))?;
    } else {
        let args = steps::mix_pass_b_silent_args(&raw, &mixed);
        run_ff(ff_bin("ffmpeg").as_str(), &strs(&args))?;
    }
    let _ = cutforge_io::atomic::remove(&raw);
    idx.set_size("mix", &key, file_size(&mixed));
    idx.save(&plan.cache_dir)?;
    let rep = StepReport::new("mix", json!({"cacheHit": false}));
    Ok((rep, mixed, key, false))
}

/// 步 6:字幕(有 ASS 烧录,否则零重编码合流;键含 video 键 + mix 键 + ASS 字节哈希)。
fn exec_subtitle(
    plan: &RenderPlan,
    idx: &mut CacheIndex,
    base_video: &Path,
    mixed: &Path,
    video_key: &str,
    mix_key_str: &str,
) -> Result<(StepReport, PathBuf), String> {
    let now = cache::now_secs();
    let burned = plan.ass_path.is_some();
    let ass_bytes = match &plan.ass_path {
        Some(ass) => Some(std::fs::read(ass).map_err(|e| e.to_string())?),
        None => None,
    };
    let spec = cache::sub_spec(video_key, mix_key_str, ass_bytes.as_deref());
    let key = cache::sub_key(video_key, mix_key_str, ass_bytes.as_deref());
    let subbed = match idx.touch("sub", &key, now) {
        Some(rel) => plan.cache_dir.join(rel),
        None => {
            let rel = idx.record("sub", &key, spec, now);
            let subbed = plan.cache_dir.join(rel);
            match &ass_bytes {
                Some(payload) => {
                    // ASS 副本写 tmp(cwd = 缓存根,相对路径喂滤镜,规避盘符冒号转义)
                    let ass_rel = cache::tmp_rel(&format!("ass-{key}.ass"));
                    let ass_local = plan.cache_dir.join(&ass_rel);
                    cutforge_io::atomic::atomic_write(&ass_local, payload).map_err(|e| e.to_string())?;
                    // 滤镜参数内路径必须正斜杠:Windows 的 to_string_lossy 产反斜杠,
                    // 会被 filtergraph 转义规则吞掉("tmp\ass-k.ass"→"tmpass-k.ass",烧录必炸)
                    let ass_arg = ass_rel.to_string_lossy().replace('\\', "/");
                    let args = steps::subtitle_burn_args(base_video, mixed, &ass_arg, &subbed);
                    let r = run_ff_in(&plan.cache_dir, "ffmpeg", &strs(&args));
                    let _ = cutforge_io::atomic::remove(&ass_local);
                    r?;
                }
                None => {
                    let args = steps::subtitle_mux_args(base_video, mixed, &subbed);
                    run_ff("ffmpeg", &strs(&args))?;
                }
            }
            idx.set_size("sub", &key, file_size(&subbed));
            subbed
        }
    };
    idx.save(&plan.cache_dir)?;
    Ok((StepReport::new("subtitle", json!({"burned": burned})), subbed))
}

/// 步 7:encode(原子拷贝到成片输出;不缓存,输出路径与格式不变)。
fn exec_encode(plan: &RenderPlan, video_input: &Path) -> Result<(StepReport, PathBuf), String> {
    let output = steps::encode_output_path(plan);
    let payload = std::fs::read(video_input).map_err(|e| e.to_string())?;
    cutforge_io::atomic::atomic_write(&output, &payload).map_err(|e| e.to_string())?;
    Ok((
        StepReport::new("encode", json!({"output": output.to_string_lossy()})),
        output,
    ))
}

/// headless 批量:变体矩阵(比例 × 组合)。段缓存键含 canvas;混音键画幅无关 →
/// 多画幅共享 mix,只重做 video 链与 encode(真分叉,6.5;T1.10)。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_version_is_bumped_for_cache_layout_change() {
        assert!(RENDERER_VERSION >= "cutforge-render-4.0", "T1.5 缓存键口径变更必须升版");
    }

    #[test]
    fn outcome_reports_seg_hits_as_cache_hits() {
        // RenderOutcome 字段口径:cache_hits = 段缓存命中(与 render_matrix 断言对齐)
        let o = RenderOutcome {
            output: PathBuf::from("x"),
            steps: vec![],
            cache_hits: 2,
            cache_misses: 1,
            segments: 3,
            mix_cache_hit: false,
        };
        assert_eq!(o.cache_hits + o.cache_misses, o.segments);
    }
}
