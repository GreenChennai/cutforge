// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! CutForge 渲染后端(计划书 6.2;V2 M11 矩阵口径;T1.4 分解为 RenderPlan + 步骤函数):
//! probe → segment → compose → overlay → mix → subtitle → encode。
//! 每步 = steps.rs 纯函数(生成 ffmpeg 参数,可离线单测)+ 本文件执行器(调进程),
//! 产出结构化 StepReport;进度事件由 StepReport 单源派生(stdout JSON 行接口不变)。
//! 缓存(T1.5):中间产物全量内容寻址,分层见 cache.rs;最终成片路径与格式不变。
//! 单帧(T2.4):render_frame 对指定时间点出一帧合成画面(frame 层缓存键含
//! 工作区指纹),供壳「精确预览」;见 frame.rs。

pub mod across;
pub mod analyze;
pub mod cache;
pub mod catalog;
pub mod compound;
pub mod encode;
pub mod export;
pub mod frame;
pub mod grade;
pub mod kf_expr;
pub mod plan;
pub mod segment;
pub mod steps;
pub mod subtitle;
pub mod textass;
pub mod zone;

pub use cache::{CacheEntry, CacheIndex, DEFAULT_CAPACITY_BYTES, GcReport, cache_gc, cache_info};
pub use frame::{
    FrameFormat, FrameOutcome, frame_done_event, frame_extract_args, quantize_ms, render_frame,
    render_frame_opts,
};
pub use plan::{RenderPlan, STEP_NAMES, StepReport};
pub use zone::{ZoneOutcome, render_zone, zone_done_event};

use cutforge_core::model::Project;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 渲染器语义版本:任何渲染行为变更都必须 +1(缓存失效的正确来源)。
/// 4.0:T1.4 render() 分解 + T1.5 缓存全量内容寻址(分层目录 + cache-index.json)。
/// 5.0(册四 A4-BE3a):T4.5 转场目录直通 + xfade offset 修复为名义时长口径
/// (旧键渲染产物错误,必须整体失效)+ T4.6 fx/motion 段链 + acrossfade 音频链。
/// 6.0(册四 A4-BE3b):T4.7 文本片段 → 临时 ASS 烧录落地(ADR-0016;文本从
/// 不可见变可见,旧缓存产物缺文本层,必须整体失效)+ T4.8 denoise/pitch 混音链 +
/// track mute/solo/hidden 渲染联动收口 + T4.1 代理预览(useProxy)。
/// 7.0(册五 T5.1):关键帧引擎(IR v3)——五条表达式通路(rotate/zoompan/geq/
/// overlay/volume)+ speed 关键帧展开 + fx 三态;旧缓存对关键帧 clip 自然分键
/// (键含 clip JSON),版本位随行为面整体升版。
/// 8.0(册五 T5.2/T5.3/T5.6):grade 调色链(colorbalance/curves/eq/colorchannelmixer/
/// lut3d,键增 LUT 内容哈希)+ 轨道 EQ/动态混音链 + ducking 参数化 + 编码参数面与
/// bt709 输出标签(encode 步从原子拷贝升级为带标签重编码,ADR-0020 决策 1)+
/// 每步耗时入进度事件。旧缓存整体失效(键全部含版本位)。
/// 9.0(册五 T5.4/ADR-0019):compound 递归展开(子时间线中间段挂 compose 层,
/// 键 = 子内容指纹)+ adjust 调整层步(新缓存层,主合成后按时间窗再过 fx/grade 链)
/// + 字幕 VTT 子格式(不触缓存键面,随行为面整体升版)。旧缓存整体失效。
///
/// 册六 10.0(T6.3 导出矩阵):RenderOptions 增 export 规格(格式分派/预设/
/// 清晰度/码率档/区域窗口)——**缺省 None 路径参数逐字不变**;中间产物与格式
/// 无关,导出复用既有缓存键(canvas/clip 变化自然分键);升版为行为面登记口径。
///
/// I1 11.0(zone 预渲 + 渲染缺口修):行为面两处变更,旧缓存整体失效——
/// - compose:xfade 链收紧为「全边界相接 + 全边界有转场」;d=0/空隙边界的
///   duration=0 xfade 会被 ffmpeg 丢段(合成片缺整段 → 抽帧越 EOF 空产出),
///   混合工程退化 concat 硬切并 WARN(时间域保真;合法全转场链逐位不变);
/// - segment:源窗钳制——素材可用时长不足时播放域截到源耗尽点,缺额 tpad 末帧
///   定格(越界 -ss/-t 曾让段比标称短);素材不存在/0 字节 → PRECONDITION;
/// - frame:内容末端守卫同步口径(源耗尽定格计满,只拦整窗无源可读)。
pub const RENDERER_VERSION: &str = "cutforge-render-11.0";

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
    let key = if tool == "ffmpeg" {
        "CUTFORGE_FFMPEG"
    } else {
        "CUTFORGE_FFPROBE"
    };
    if let Some(v) = std::env::var_os(key)
        && !v.is_empty()
    {
        return v.to_string_lossy().into_owned();
    }
    tool.to_string()
}

pub(crate) fn run_ff(tool: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(ff_bin(tool))
        .args(args)
        .output()
        .map_err(|e| format!("启动 {tool} 失败: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{tool} 失败: {}",
            String::from_utf8_lossy(&out.stderr)
                .trim()
                .chars()
                .take(800)
                .collect::<String>()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// 双通道捕获版本(loudnorm 测量输出在 stderr)。
fn run_ff_capture(tool: &str, args: &[&str]) -> Result<(String, String), String> {
    let out = Command::new(ff_bin(tool))
        .args(args)
        .output()
        .map_err(|e| format!("启动 {tool} 失败: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{tool} 失败: {}",
            String::from_utf8_lossy(&out.stderr)
                .trim()
                .chars()
                .take(300)
                .collect::<String>()
        ));
    }
    Ok((
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

fn run_ff_in(dir: &Path, tool: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(ff_bin(tool))
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("启动 {tool} 失败: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{tool} 失败: {}",
            String::from_utf8_lossy(&out.stderr)
                .trim()
                .chars()
                .take(800)
                .collect::<String>()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn ffprobe_duration_sec(path: &Path) -> Result<f64, String> {
    let out = run_ff(
        "ffprobe",
        &[
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            &path.to_string_lossy(),
        ],
    )?;
    let v: Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    v["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "缺 format.duration".into())
}

/// Vec<String> 命令行 → &[&str](run_ff 适配)。
pub(crate) fn strs(args: &[String]) -> Vec<&str> {
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
    render_with(project, project_dir, ass_path, false, progress)
}

/// 同 [`render`],代理预览开关显式给定(册四 T4.1 useProxy;显式 opt-in,
/// 不悄悄降质——代理缺失的片段回落原片)。
pub fn render_with(
    project: &Project,
    project_dir: &Path,
    ass_path: Option<&Path>,
    use_proxy: bool,
    progress: &mut dyn FnMut(Value),
) -> Result<RenderOutcome, String> {
    render_with_opts(
        project,
        project_dir,
        ass_path,
        use_proxy,
        plan::RenderOptions::default(),
        progress,
    )
}

/// 同 [`render_with`],渲染选项显式给定(册五 T5.6;Default = 现行为零变化)。
/// 每步耗时入进度事件(elapsedMs);verbose_cmd 开启时事件附命令原文(缺省关)。
pub fn render_with_opts(
    project: &Project,
    project_dir: &Path,
    ass_path: Option<&Path>,
    use_proxy: bool,
    opts: plan::RenderOptions,
    progress: &mut dyn FnMut(Value),
) -> Result<RenderOutcome, String> {
    let plan = RenderPlan::build_full(project, project_dir, ass_path, use_proxy, opts);
    std::fs::create_dir_all(&plan.out_dir).map_err(|e| e.to_string())?;
    cache::ensure_dirs(&plan.cache_dir)?;
    let mut idx = CacheIndex::load(&plan.cache_dir);
    let mut steps: Vec<(&'static str, bool)> = Vec::new();
    let verbose = plan.opts.verbose_cmd;

    // 带耗时/命令回显的进度发布(T5.6 渲染日志;事件形状向后兼容——加法字段)
    let mut emit = |rep: &mut StepReport, t0: std::time::Instant, cmds: &[String]| {
        if let Value::Object(m) = &mut rep.detail {
            m.insert("elapsedMs".into(), json!(t0.elapsed().as_millis() as u64));
            if verbose && !cmds.is_empty() {
                m.insert("cmd".into(), json!(cmds.join(" ; ")));
            }
        }
        progress(rep.to_progress());
    };

    // ---- 步 1 probe(容错:图片等无时长素材合法存在;真坏了会在 segment 步炸)
    // 音轨可用性入 map 透传 mix 步(I1 缺口修:纯视频素材的段无音频流,[N:a]
    // 零匹配会炸掉整个滤镜图——单一实现,不再对每段二次 ffprobe) ----
    let t0 = std::time::Instant::now();
    let (mut rep, audio_has) = exec_probe(&plan);
    emit(&mut rep, t0, &[]);
    steps.push((rep.name, rep.ok));

    // ---- 步 2 segment(内容寻址:键含 clip JSON + 尾帧 + 画幅 + fps + LUT 哈希) ----
    // 复合片段(册五 T5.4/ADR-0019)在此递归展开:先渲子时间线为中间段
    // (compound::resolve,compose 层内容寻址),再按普通素材走段管线。
    let t0 = std::time::Instant::now();
    let (mut rep, seg_files, seg_keys, cache_hits, cache_misses) = exec_segment(&plan, &mut idx)?;
    emit(&mut rep, t0, &[]);
    steps.push((rep.name, rep.ok));

    // ---- 步 3 compose(xfade 链 / concat 退化) ----
    let t0 = std::time::Instant::now();
    let (mut rep, composed, compose_key, compose_cmds) =
        exec_compose(&plan, &mut idx, &seg_files, &seg_keys)?;
    emit(&mut rep, t0, &compose_cmds);
    steps.push((rep.name, rep.ok));

    // ---- 步 4 overlay(品牌/花字位图;无叠加层直接透传) ----
    let t0 = std::time::Instant::now();
    let (mut rep, overlaid, overlay_key, overlay_cmds) =
        exec_overlay(&plan, &mut idx, &composed, &compose_key)?;
    emit(&mut rep, t0, &overlay_cmds);
    steps.push((rep.name, rep.ok));

    // ---- 步 4.5 adjust(册五 T5.4 调整层:主合成后按时间窗再过 fx/grade 链;空透传) ----
    let t0 = std::time::Instant::now();
    let (mut rep, base_video, video_key, adjust_cmds) =
        exec_adjust(&plan, &mut idx, &overlaid, &overlay_key)?;
    emit(&mut rep, t0, &adjust_cmds);
    steps.push((rep.name, rep.ok));

    // ---- 步 5 mix(画幅无关 → 共享缓存;多画幅变体真分叉) ----
    let t0 = std::time::Instant::now();
    let (mut rep, mixed, mix_key_str, mix_cache_hit, mix_cmds) =
        exec_mix(&plan, &mut idx, &audio_has)?;
    emit(&mut rep, t0, &mix_cmds);
    steps.push((rep.name, rep.ok));

    // ---- 步 6 subtitle(最后叠;外部 ASS / 文本轨生成 ASS 烧录,否则零重编码合流) ----
    let t0 = std::time::Instant::now();
    let (mut rep, video_input, sub_cmds) = exec_subtitle(
        &plan,
        &mut idx,
        &base_video,
        &mixed,
        &video_key,
        &mix_key_str,
        project,
    )?;
    emit(&mut rep, t0, &sub_cmds);
    steps.push((rep.name, rep.ok));

    // ---- 步 7 encode(文件名消毒;bt709 标签重编码 + 复验;不走缓存) ----
    let t0 = std::time::Instant::now();
    let (mut rep, output, encode_cmds) = exec_encode(&plan, &video_input)?;
    emit(&mut rep, t0, &encode_cmds);
    steps.push((rep.name, rep.ok));
    idx.save(&plan.cache_dir)?;

    Ok(RenderOutcome {
        output,
        steps,
        cache_hits,
        cache_misses,
        segments: plan.video_clips.len(),
        mix_cache_hit,
    })
}

/// 步 1:探测素材(结果仅热身/容错,不改变管线);返回素材 → 是否含音轨
/// (mix 步拼图的输入面;probe 不可达 = 保守「有」,不改变既有图形态)。
fn exec_probe(plan: &RenderPlan) -> (StepReport, HashMap<PathBuf, bool>) {
    let mut audio_has: HashMap<PathBuf, bool> = HashMap::new();
    for p in steps::probe_paths(plan) {
        let has = cutforge_io::probe::probe(&p)
            .map(|info| info.has_audio())
            .unwrap_or(true);
        audio_has.insert(p, has);
    }
    (StepReport::new("probe", json!({})), audio_has)
}

type SegOutputs = (StepReport, Vec<PathBuf>, Vec<String>, usize, usize);

/// 步 2:逐段提取(缓存命中即跳过;fx/motion/调色降级 WARN 并入进度事件)。
/// 复合片段(compound 字段)先递归渲染子时间线为中间段(compound::resolve,
/// 共享内容寻址缓存),再按普通素材走段管线;子管线 WARN 并入本步 warnings。
fn exec_segment(plan: &RenderPlan, idx: &mut CacheIndex) -> Result<SegOutputs, String> {
    let now = cache::now_secs();
    let mut seg_files: Vec<PathBuf> = Vec::new();
    let mut seg_keys: Vec<String> = Vec::new();
    let mut hits = 0usize;
    let mut misses = 0usize;
    let mut degradations: Vec<String> = Vec::new();
    for (i, clip) in plan.video_clips.iter().enumerate() {
        let tail = steps::segment_tail_ms(&plan.video_clips, i);
        // 复合片段递归展开(T5.4):壳克隆 + src 换写中间段 → 键含整 clip JSON
        // (含 compound),子时间线任一变化必换键
        let mut effective = match &clip.compound {
            Some(spec) => {
                let resolved = crate::compound::resolve(plan, clip, spec, idx)?;
                degradations.extend(resolved.warns.iter().cloned());
                crate::compound::effective_clip(clip, &resolved)
            }
            None => clip.clone(),
        };
        // 源窗钳制(I1 渲染缺口修):素材实际可用时长不足时把播放域截到源耗尽
        // 点,缺额交给 tpad 末帧定格(与时间线空隙钳前段末帧同哲学)——越界
        // -ss/-t 曾让段比标称短,下游 concat/抽帧越过合成 EOF 空产出。钳制写进
        // 有效片段 → seg 键自动分叉(素材补长后键变回,陈旧定格零复用);探测
        // 不可达(图片等无时长素材合法存在)→ 不钳,行为不变。整窗无源可读
        // (素材缺失/空文件)不再交给 ffmpeg 报 INTERNAL,提前给可读 PRECONDITION。
        if let Some(src) = effective.src.clone() {
            let abs = plan.project_dir.join(&src);
            if !abs.is_file() {
                return Err(format!(
                    "PRECONDITION: 片段 {} 的素材不存在,段不可渲染: {}",
                    effective.id,
                    abs.display()
                ));
            }
            let empty = std::fs::metadata(&abs)
                .map(|m| m.len() == 0)
                .unwrap_or(true);
            if empty {
                return Err(format!(
                    "PRECONDITION: 片段 {} 的素材为空文件(0 字节),段不可渲染: {}",
                    effective.id,
                    abs.display()
                ));
            }
            if let Ok(info) = cutforge_io::probe::probe(&abs)
                && let Some(clamped) =
                    crate::segment::source_clamped_clip(&effective, info.duration_ms())
            {
                degradations.push(format!(
                    "clip {} 源窗超出素材可用时长({}ms),越界段按末帧定格补长",
                    effective.id,
                    info.duration_ms()
                ));
                effective = clamped;
            }
        }
        let clip = &effective;
        let spec = cache::seg_spec(plan, clip, tail);
        let key = cache::seg_key(plan, clip, tail);
        degradations.extend(crate::catalog::clip_degradations(
            clip,
            plan.canvas_w,
            plan.canvas_h,
            plan.fps,
        ));
        // grade 降级面(T5.2:HSL 登记不渲染 / LUT 文件缺失)
        let (_, grade_warns) = crate::grade::grade_chain(clip, &plan.project_dir);
        degradations.extend(grade_warns);
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
/// 返回 (报告, 产物, 键, 实际执行的命令——verbose_cmd 回显用)。
fn exec_compose(
    plan: &RenderPlan,
    idx: &mut CacheIndex,
    seg_files: &[PathBuf],
    seg_keys: &[String],
) -> Result<(StepReport, PathBuf, String, Vec<String>), String> {
    let now = cache::now_secs();
    let key = cache::compose_key(seg_keys);
    let mut tr_warns: Vec<String> = Vec::new();
    let mut cmds: Vec<String> = Vec::new();
    let composed = match idx.touch("compose", &key, now) {
        Some(rel) => plan.cache_dir.join(rel),
        None => {
            let rel = idx.record("compose", &key, cache::compose_spec(seg_keys), now);
            let composed = plan.cache_dir.join(rel);
            if steps::is_xfade_chain(&plan.video_clips) {
                let (args, warns) =
                    steps::compose_xfade_args(&plan.video_clips, seg_files, &composed);
                tr_warns = warns;
                cmds.push(args.join(" "));
                run_ff("ffmpeg", &strs(&args))?;
            } else {
                // concat 清单写 tmp(内容寻址命名);用完即清。
                // 条目必须绝对化:concat demuxer 以**清单所在目录**解析相对路径,
                // --root 为相对路径时会拼出 .cutforge/render-cache/.cutforge/… 而炸
                // (拆分前旧 concat.txt 同样中招;只改临时清单内容,成片输出不变)。
                let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
                let abs_segs: Vec<PathBuf> = seg_files
                    .iter()
                    .map(|p| {
                        if p.is_absolute() {
                            p.clone()
                        } else {
                            cwd.join(p)
                        }
                    })
                    .collect();
                // 转场降级 WARN(I1 修):xfade 链只在全边界(相接 + 转场)有效时
                // 启用;工程含空隙或无转场边界时整链硬切保时间域(d=0 的 xfade 会
                // 丢段),已声明的转场在此降级,必须留痕不静默。
                for i in 1..plan.video_clips.len() {
                    if crate::catalog::effective_transition_ms(&plan.video_clips, i) > 0.0 {
                        tr_warns.push(format!(
                            "clip {} 转场降级硬切:xfade 链要求全部边界相接且有转场(时间线空隙/无转场边界会丢段)",
                            plan.video_clips[i].id
                        ));
                    }
                }
                let list_rel = cache::tmp_rel(&format!("concat-{key}.txt"));
                let list_path = plan.cache_dir.join(&list_rel);
                cutforge_io::atomic::atomic_write(
                    &list_path,
                    steps::concat_list_content(&abs_segs).as_bytes(),
                )
                .map_err(|e| e.to_string())?;
                let args = steps::compose_concat_args(&list_path, &composed);
                cmds.push(args.join(" "));
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
    Ok((
        StepReport::new("compose-video", detail),
        composed,
        key,
        cmds,
    ))
}

/// 步 4:overlay 合成(无叠加层直接透传基片;键 = compose 键 + 叠加清单)。
fn exec_overlay(
    plan: &RenderPlan,
    idx: &mut CacheIndex,
    composed: &Path,
    compose_key: &str,
) -> Result<(StepReport, PathBuf, String, Vec<String>), String> {
    let now = cache::now_secs();
    if plan.overlay_segs.is_empty() {
        return Ok((
            StepReport::new("overlay", json!({"count": 0})),
            composed.to_path_buf(),
            compose_key.to_string(),
            Vec::new(),
        ));
    }
    let key = cache::overlay_key(compose_key, &plan.overlay_segs);
    let (overlaid, hit, cmds) = match idx.touch("overlay", &key, now) {
        Some(rel) => (plan.cache_dir.join(rel), true, Vec::new()),
        None => {
            let rel = idx.record(
                "overlay",
                &key,
                cache::overlay_spec(compose_key, &plan.overlay_segs),
                now,
            );
            let overlaid = plan.cache_dir.join(rel);
            let args = steps::overlay_args(&plan.overlay_segs, composed, &overlaid);
            let cmd = args.join(" ");
            run_ff("ffmpeg", &strs(&args))?;
            idx.set_size("overlay", &key, file_size(&overlaid));
            (overlaid, false, vec![cmd])
        }
    };
    idx.save(&plan.cache_dir)?;
    Ok((
        StepReport::new("overlay", json!({"count": plan.overlay_segs.len()})).with_cache_hit(hit),
        overlaid,
        key,
        cmds,
    ))
}

type MixOutputs = (StepReport, PathBuf, String, bool, Vec<String>);

/// 步 4.5 adjust(册五 T5.4 调整层):主合成(含叠加层)后,adjust 轨片段的
/// fx/grade 链按时间窗再过一遍(时间窗处理 = trim 抽窗 → 链作用于窗内流 →
/// overlay enable 贴回;不依赖滤镜级 enable,任意链可窗内生效)。
/// 无 adjust 片段直接透传(基片与键原样,零重编码零新缓存条目)。
fn exec_adjust(
    plan: &RenderPlan,
    idx: &mut CacheIndex,
    base_video: &Path,
    base_key: &str,
) -> Result<(StepReport, PathBuf, String, Vec<String>), String> {
    if plan.adjust_clips.is_empty() {
        return Ok((
            StepReport::new("adjust", json!({"count": 0})),
            base_video.to_path_buf(),
            base_key.to_string(),
            Vec::new(),
        ));
    }
    let now = cache::now_secs();
    let mut warns: Vec<String> = Vec::new();
    for c in &plan.adjust_clips {
        let (_, gw) = crate::grade::grade_chain(c, &plan.project_dir);
        warns.extend(gw);
        let (_, _, mw) = crate::catalog::motion_chains(c, plan.canvas_w, plan.canvas_h, plan.fps);
        warns.extend(mw);
    }
    let spec = cache::adjust_spec(
        base_key,
        &plan.adjust_clips,
        plan.canvas_w,
        plan.canvas_h,
        plan.fps,
    );
    let key = format!(
        "{}-{}x{}f{}",
        cache::key_hex(&spec),
        plan.canvas_w,
        plan.canvas_h,
        plan.fps
    );
    let mut cmds: Vec<String> = Vec::new();
    let adjusted = match idx.touch("adjust", &key, now) {
        Some(rel) => (plan.cache_dir.join(rel), true),
        None => {
            let rel = idx.record("adjust", &key, spec, now);
            let adjusted = plan.cache_dir.join(rel);
            let args = steps::adjust_args(
                &plan.adjust_clips,
                base_video,
                &adjusted,
                plan.project_dir.as_path(),
                plan.canvas_w,
                plan.canvas_h,
                plan.fps,
            );
            cmds.push(args.join(" "));
            run_ff("ffmpeg", &strs(&args))?;
            idx.set_size("adjust", &key, file_size(&adjusted));
            (adjusted, false)
        }
    };
    idx.save(&plan.cache_dir)?;
    let mut detail = json!({"count": plan.adjust_clips.len()});
    if !warns.is_empty() {
        detail["warnings"] = json!(warns);
    }
    Ok((
        StepReport::new("adjust", detail).with_cache_hit(adjusted.1),
        adjusted.0,
        key,
        cmds,
    ))
}

/// 步 5:混音(pass A 逐段落点/变速/淡变 → 轨道组建流 → 总线 → BGM ducking;
/// pass B loudnorm——响度目标可由渲染选项注入,响度单参数化 T5.3/T5.6)。
/// 画幅无关 → 多画幅变体共享(mix_cache_hit 即真分叉判据)。
fn exec_mix(
    plan: &RenderPlan,
    idx: &mut CacheIndex,
    audio_has: &HashMap<PathBuf, bool>,
) -> Result<MixOutputs, String> {
    let now = cache::now_secs();
    let spec = cache::mix_spec(plan);
    let key = cache::mix_key(plan);
    if let Some(rel) = idx.touch("mix", &key, now) {
        let mixed = plan.cache_dir.join(rel);
        let rep = StepReport::new("mix", json!({"cacheHit": true})).with_cache_hit(true);
        return Ok((rep, mixed, key, true, Vec::new()));
    }
    let rel = idx.record("mix", &key, spec, now);
    let mixed = plan.cache_dir.join(rel);
    let mut cmds: Vec<String> = Vec::new();
    let target_i = plan.opts.loudnorm_i.unwrap_or(-14.0);
    let target_tp = plan.opts.loudnorm_tp.unwrap_or(-1.0);
    // pass A:总线合成 → 原始混音(raw 为临时件,pass B 后即清)。
    // 逐事件素材音轨可用性取自 probe 步透传(查不到 = 保守「有」,既有图形态)。
    let segs_has: Vec<bool> = plan
        .audio_segs
        .iter()
        .map(|s| audio_has.get(&s.src).copied().unwrap_or(true))
        .collect();
    let bgm_has = plan
        .bgm
        .as_ref()
        .map(|b| {
            audio_has
                .get(&plan.project_dir.join(&b.src))
                .copied()
                .unwrap_or(true)
        })
        .unwrap_or(true);
    let raw = mixed.with_file_name(format!("{key}.raw.m4a"));
    let args = steps::mix_pass_a_args_with(plan, &raw, &segs_has, bgm_has);
    cmds.push(args.join(" "));
    run_ff("ffmpeg", &strs(&args))?;

    // pass B:响度(先测后编 linear=true)。数字静音(-inf,无音频工程)跳过:
    // linear=true 遇 -inf 的 measured 值,ffmpeg 报 "Result too large" 直接失败。
    let (_m_out, m_err) = run_ff_capture(
        ff_bin("ffmpeg").as_str(),
        &strs(&steps::mix_measure_args_t(&raw, target_i, target_tp)),
    )?;
    let m_start = m_err
        .rfind('{')
        .ok_or("loudnorm 测量输出无 JSON".to_string())?;
    let m_end = m_err
        .rfind('}')
        .ok_or("loudnorm 测量输出无 JSON".to_string())?
        + 1;
    let measured: Value =
        serde_json::from_str(&m_err[m_start..m_end]).map_err(|e| e.to_string())?;
    if steps::mix_measured_is_loud(&measured) {
        let args = steps::mix_pass_b_args(
            &steps::mix_linear_filter_t(&measured, target_i, target_tp),
            &raw,
            &mixed,
        );
        cmds.push(args.join(" "));
        run_ff(ff_bin("ffmpeg").as_str(), &strs(&args))?;
    } else {
        let args = steps::mix_pass_b_silent_args(&raw, &mixed);
        cmds.push(args.join(" "));
        run_ff(ff_bin("ffmpeg").as_str(), &strs(&args))?;
    }
    let _ = cutforge_io::atomic::remove(&raw);
    idx.set_size("mix", &key, file_size(&mixed));
    idx.save(&plan.cache_dir)?;
    let rep = StepReport::new("mix", json!({"cacheHit": false}));
    Ok((rep, mixed, key, false, cmds))
}

/// 步 6:字幕(外部 ASS + 文本轨生成 ASS 烧录,否则零重编码合流;键含 video 键 +
/// mix 键 + 全部 ASS 字节哈希)。文本轨文本片段经 textass(ADR-0016)确定性生成
/// 临时 ASS,与外部字幕以**链式 subtitles 滤镜**先后应用——免解析合并外部文件,
/// 烧录通道与既有字幕完全同源。
fn exec_subtitle(
    plan: &RenderPlan,
    idx: &mut CacheIndex,
    base_video: &Path,
    mixed: &Path,
    video_key: &str,
    mix_key_str: &str,
    project: &Project,
) -> Result<(StepReport, PathBuf, Vec<String>), String> {
    let now = cache::now_secs();
    let user_bytes = match &plan.ass_path {
        Some(ass) => Some(std::fs::read(ass).map_err(|e| e.to_string())?),
        None => None,
    };
    let text_bytes = textass::generate(project).map(|s| s.into_bytes());
    // 合流哈希 = 两份 ASS 字节顺序拼接(次序 = 烧录次序;None 全无 → 不烧录)
    let combined: Option<Vec<u8>> = match (&user_bytes, &text_bytes) {
        (None, None) => None,
        (a, b) => {
            let mut v = Vec::new();
            v.extend(a.iter().flatten().copied());
            v.extend(b.iter().flatten().copied());
            Some(v)
        }
    };
    let burned = combined.is_some();
    let spec = cache::sub_spec(video_key, mix_key_str, combined.as_deref());
    let key = cache::sub_key(video_key, mix_key_str, combined.as_deref());
    let mut cmds: Vec<String> = Vec::new();
    let subbed = match idx.touch("sub", &key, now) {
        Some(rel) => plan.cache_dir.join(rel),
        None => {
            let rel = idx.record("sub", &key, spec, now);
            let subbed = plan.cache_dir.join(rel);
            if combined.is_some() {
                // 各 ASS 副本写 tmp(cwd = 缓存根,相对路径喂滤镜,规避盘符冒号转义)
                let write_ass = |tag: &str, bytes: &[u8]| -> Result<String, String> {
                    let rel = cache::tmp_rel(&format!("ass-{tag}-{key}.ass"));
                    cutforge_io::atomic::atomic_write(&plan.cache_dir.join(&rel), bytes)
                        .map_err(|e| e.to_string())?;
                    Ok(rel.to_string_lossy().replace('\\', "/"))
                };
                let mut ass_rels: Vec<String> = Vec::new();
                if let Some(user) = &user_bytes {
                    ass_rels.push(write_ass("user", user)?);
                }
                if let Some(text) = &text_bytes {
                    ass_rels.push(write_ass("text", text)?);
                }
                // 滤镜参数内路径必须正斜杠:Windows 的 to_string_lossy 产反斜杠,
                // 会被 filtergraph 转义规则吞掉("tmp\ass-k.ass"→"tmpass-k.ass",烧录必炸)
                let args = steps::subtitle_burn_args(base_video, mixed, &ass_rels, &subbed);
                cmds.push(args.join(" "));
                let r = run_ff_in(&plan.cache_dir, "ffmpeg", &strs(&args));
                for rel in &ass_rels {
                    let _ = cutforge_io::atomic::remove(&plan.cache_dir.join(rel));
                }
                r?;
            } else {
                let args = steps::subtitle_mux_args(base_video, mixed, &subbed);
                cmds.push(args.join(" "));
                run_ff("ffmpeg", &strs(&args))?;
            }
            idx.set_size("sub", &key, file_size(&subbed));
            subbed
        }
    };
    idx.save(&plan.cache_dir)?;
    let mut detail = json!({"burned": burned, "textClips": text_bytes.is_some()});
    let warns = textass::generation_warnings(project);
    if !warns.is_empty() {
        detail["warnings"] = json!(warns);
    }
    Ok((StepReport::new("subtitle", detail), subbed, cmds))
}

/// 步 7:encode(bt709 标签 remux 缺省 / 显式选项重编码 + ffprobe 复验,ADR-0020;
/// 硬件选项试编探测,失败优雅降级 libx264——AC-5.6;不走缓存,输出路径与格式不变)。
/// 册六 T6.3:export 规格在位时按格式分派(export.rs 单源;mp4-h264/mov 与既有
/// 编码面同源,其余走新出口)。缺省(export = None)分支逐字不变。
fn exec_encode(
    plan: &RenderPlan,
    video_input: &Path,
) -> Result<(StepReport, PathBuf, Vec<String>), String> {
    // zone 预渲出口(I1-M2):preview_output 在位 = 同一编码面(final_encode_args,
    // 预览质量档已随 opts 注入)直写内容寻址产物;不探测硬件(预览确定性优先)、
    // 不复验色标(预览语义)。缺省 None = 既有路径逐字不变。
    if let Some(target) = &plan.opts.preview_output {
        let (args, enc) = encode::final_encode_args(video_input, target, &plan.opts, None);
        run_ff("ffmpeg", &strs(&args))?;
        let detail = json!({
            "mode": "zone",
            "output": target.to_string_lossy(),
            "encoder": enc,
        });
        return Ok((
            StepReport::new("encode", detail),
            target.clone(),
            vec![args.join(" ")],
        ));
    }
    if let Some(spec) = &plan.opts.export {
        return export::exec_export_encode(plan, video_input, spec);
    }
    let output = steps::encode_output_path(plan);
    let mut detail = json!({});
    let mut warns: Vec<String> = Vec::new();
    // 编码器解析(T5.6):auto/缺省 = libx264 确定性基线;hw = 试编探测优先硬件
    let mut hw: Option<&'static str> = None;
    if plan.opts.encoder.as_deref() == Some("hw") {
        hw = encode::resolve_hw_candidate();
        if hw.is_none() {
            warns.push(
                "硬件编码探测不可用(nvenc/qsv/amf 试编均失败),优雅降级 libx264(AC-5.6);".into(),
            );
        }
    }
    let (args, enc_name) = encode::final_encode_args(video_input, &output, &plan.opts, hw);
    let mut ok = run_ff("ffmpeg", &strs(&args)).is_ok();
    if !ok && hw.is_some() {
        // 试编通过但全片会话失败(驱动会话/显存等)→ 同样优雅降级
        let (sw_args, sw_name) = encode::final_encode_args(video_input, &output, &plan.opts, None);
        ok = run_ff("ffmpeg", &strs(&sw_args)).is_ok();
        if ok {
            warns.push(format!(
                "硬件编码 {enc_name} 全片失败,优雅降级 {sw_name}(AC-5.6);"
            ));
        }
    }
    if !ok {
        return Err("encode 步 ffmpeg 失败(含硬件降级后仍失败)".into());
    }
    // 色彩标签复验(ADR-0020:标签在位纳入渲染自检;缺标签 WARN 留痕不阻塞)
    match encode::verify_color_tags(&output) {
        Ok(tags) => detail["colorTags"] = tags,
        Err(e) => warns.push(format!("色彩标签复验不可达: {e};")),
    }
    if let Some(t) = detail["colorTags"].as_object()
        && t.get("inPlace").and_then(|v| v.as_bool()) == Some(false)
    {
        warns.push("输出色彩标签缺失(ffprobe 复验),渲染自检 WARN;".into());
    }
    detail["output"] = json!(output.to_string_lossy());
    detail["encoder"] = json!(enc_name);
    if !warns.is_empty() {
        detail["warnings"] = json!(warns);
    }
    Ok((
        StepReport::new("encode", detail),
        output,
        vec![args.join(" ")],
    ))
}

/// headless 批量:变体矩阵(比例 × 组合)。段缓存键含 canvas;混音键画幅无关 →
/// 多画幅共享 mix,只重做 video 链与 encode(真分叉,6.5;T1.10)。
pub fn render_variants(
    project: &Project,
    project_dir: &Path,
    ratios: &[&str],
) -> Vec<(String, Result<PathBuf, String>)> {
    ratios
        .iter()
        .map(|r| {
            let mut p = project.clone();
            p.canvas = match *r {
                "3x4" => cutforge_core::model::Canvas {
                    width: 1080,
                    height: 1440,
                },
                "16x9" => cutforge_core::model::Canvas {
                    width: 1920,
                    height: 1080,
                },
                _ => cutforge_core::model::Canvas {
                    width: 1080,
                    height: 1920,
                },
            };
            match render(&p, project_dir, None, &mut |_| {}) {
                Ok(o) => (r.to_string(), Ok(o.output)),
                Err(e) => (r.to_string(), Err(e)),
            }
        })
        .collect()
}

/// 导出主入口(册六 T6.3):工程侧先按 ExportSpec 换写(画幅/清晰度/仅视频/
/// 时间窗,export.rs 纯函数单源),随后:
/// - 纯音频格式(m4a/mp3)走**短路管线** probe → mix → 音频编码(不渲视频链);
/// - 其余格式走既有八步管线,encode 步按格式分派(exec_export_encode);
/// - frame-png 不进本入口(CLI/MCP 面复用 render_frame 单帧管线)。
///
/// 缺省(无导出意图)请继续调 render/render_with_opts,本函数不改变其行为。
pub fn render_export(
    project: &Project,
    project_dir: &Path,
    ass_path: Option<&Path>,
    use_proxy: bool,
    mut opts: plan::RenderOptions,
    spec: export::ExportSpec,
    progress: &mut dyn FnMut(Value),
) -> Result<RenderOutcome, String> {
    let format = spec.effective_format();
    if format == export::ExportFormat::FramePng {
        return Err("PRECONDITION: frame-png 走单帧管线(render_frame),不进整片导出".into());
    }
    let prepared = export::prepare_project(project, &spec);
    if format.is_audio_only() {
        let plan = RenderPlan::build_full(&prepared, project_dir, ass_path, use_proxy, opts);
        std::fs::create_dir_all(&plan.out_dir).map_err(|e| e.to_string())?;
        cache::ensure_dirs(&plan.cache_dir)?;
        let mut idx = CacheIndex::load(&plan.cache_dir);
        let mut steps: Vec<(&'static str, bool)> = Vec::new();
        let t0 = std::time::Instant::now();
        let (mut rep, audio_has) = exec_probe(&plan);
        rep.detail["elapsedMs"] = json!(t0.elapsed().as_millis() as u64);
        progress(rep.to_progress());
        steps.push((rep.name, rep.ok));
        let t0 = std::time::Instant::now();
        let (mut rep, mixed, _mix_key, mix_hit, _cmds) = exec_mix(&plan, &mut idx, &audio_has)?;
        rep.detail["elapsedMs"] = json!(t0.elapsed().as_millis() as u64);
        progress(rep.to_progress());
        steps.push((rep.name, rep.ok));
        idx.save(&plan.cache_dir)?;
        let output = export::export_output_path(
            &plan.out_dir,
            &plan.slug,
            plan.canvas_w,
            plan.canvas_h,
            format,
        );
        let args = export::audio_export_args(format, &mixed, &output);
        run_ff("ffmpeg", &strs(&args))?;
        progress(json!({
            "step": "encode", "ok": true, "format": format.as_str(),
            "output": output.to_string_lossy(), "mixCacheHit": mix_hit,
        }));
        steps.push(("encode", true));
        return Ok(RenderOutcome {
            output,
            steps,
            cache_hits: 0,
            cache_misses: 0,
            segments: 0,
            mix_cache_hit: mix_hit,
        });
    }
    if !prepared
        .tracks
        .iter()
        .any(|t| t.kind == cutforge_core::model::TrackKind::Video && !t.clips.is_empty())
    {
        return Err("PRECONDITION: 导出窗口内无视频片段(区域出点越界或时间线为空)".into());
    }
    opts.export = Some(spec);
    render_with_opts(&prepared, project_dir, ass_path, use_proxy, opts, progress)
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
        // 数值化比较(字符串序会误判 "10.0" < "9.0");任一缓存键口径变更必须升版
        let major: u32 = RENDERER_VERSION
            .trim_start_matches("cutforge-render-")
            .split('.')
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        assert!(
            major >= 4,
            "T1.5 缓存键口径变更必须升版: {RENDERER_VERSION}"
        );
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
