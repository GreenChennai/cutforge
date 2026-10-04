// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 导出矩阵(册六 T6.3):预设/清晰度/码率档/格式分派 + 区域导出的**纯函数层**。
//!
//! **设计定档**(诚实口径,逐条可判定):
//! - 预设(preset)与 render_variants 同语义 = 画幅覆盖映射(vertical/horizontal/
//!   square 及比率别名);清晰度档(qualityTier 1080p/720p/480p)= **短边**缩放
//!   (1080p 竖屏 = 1080×1920,与平台惯例一致),倍数取整到偶数;
//! - 码率档(bitrateTier high/medium/low)= 以 1080p 基准(12000/8000/4000 kbps)
//!   按高度线性折算的**估算值**,显式 bitrate 参数覆盖——档位是预估不是承诺;
//! - 区域导出 = **工程级时间窗裁剪**(inMs/outMs):全部轨种的片段按窗口
//!   钳制 + 平移(头部裁剪按速度分段积分折算源域偏移),首段入向转场清除;
//!   字幕(textass)/叠加层/调整层同窗——窗口后整条管线原样复用,零并行实现。
//!   已登记近似:头部裁剪落在 freezeMs 定格区内的源域折算按速度段计算(定格区
//!   实际不耗源),窗口边界恰入定格的极端场景偏差 ≤ 一段定格时长;
//! - 新增渲染出口 = encode 步分派:gif(fps=12 固定,palettegen/paletteuse 两段)、
//!   纯音频(m4a=copy/mp3=libmp3lame;audio-only 走 probe+mix 短路,不渲视频)、
//!   png 序列(image2 %04d)、h265(libx265 + hvc1 标签,软件编码)、mov(容器
//!   随扩展名,与 mp4-h264 共用编码面);单帧 frame-png 在 CLI/MCP 面复用
//!   render_frame(不进本模块的整片管线);
//! - 缺省路径(无任何导出参数)参数逐字不变:RenderOptions::export = None 时
//!   exec_encode 走既有分支,输出名与缓存键零变化。

use crate::plan::{RenderOptions, RenderPlan, StepReport};
use cutforge_core::model::{Canvas, Clip, Project, TrackKind};
use serde_json::json;
use std::path::{Path, PathBuf};

/// GIF 导出帧率(固定 12fps;30fps GIF 体积失控,平台惯例 12–15,取下限并如实声明)。
pub const GIF_FPS: u32 = 12;

/// 码率档 1080p 基准 kbps(high/medium/low);按输出高度线性折算。
pub const BITRATE_TIER_BASE_1080: [(&str, u64); 3] =
    [("high", 12000), ("medium", 8000), ("low", 4000)];

/// 导出格式(encode 步分派;mp4-h264 为缺省容器面,与既有渲染同出口)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Mp4H264,
    Mp4H265,
    Mov,
    Gif,
    M4a,
    Mp3,
    PngSeq,
    FramePng,
}

impl ExportFormat {
    /// 契约字符串解析(mp4-h264|mp4-h265|mov|gif|m4a|mp3|png-seq|frame-png)。
    pub fn parse(s: &str) -> Option<ExportFormat> {
        match s.to_ascii_lowercase().as_str() {
            "mp4-h264" | "mp4" | "h264" => Some(ExportFormat::Mp4H264),
            "mp4-h265" | "h265" | "hevc" => Some(ExportFormat::Mp4H265),
            "mov" => Some(ExportFormat::Mov),
            "gif" => Some(ExportFormat::Gif),
            "m4a" => Some(ExportFormat::M4a),
            "mp3" => Some(ExportFormat::Mp3),
            "png-seq" | "png_seq" | "sequence" => Some(ExportFormat::PngSeq),
            "frame-png" | "frame_png" => Some(ExportFormat::FramePng),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ExportFormat::Mp4H264 => "mp4-h264",
            ExportFormat::Mp4H265 => "mp4-h265",
            ExportFormat::Mov => "mov",
            ExportFormat::Gif => "gif",
            ExportFormat::M4a => "m4a",
            ExportFormat::Mp3 => "mp3",
            ExportFormat::PngSeq => "png-seq",
            ExportFormat::FramePng => "frame-png",
        }
    }

    /// 输出扩展名(png-seq 为目录,返回空串)。
    pub fn ext(self) -> &'static str {
        match self {
            ExportFormat::Mp4H264 | ExportFormat::Mp4H265 => "mp4",
            ExportFormat::Mov => "mov",
            ExportFormat::Gif => "gif",
            ExportFormat::M4a => "m4a",
            ExportFormat::Mp3 => "mp3",
            ExportFormat::PngSeq | ExportFormat::FramePng => "",
        }
    }

    /// 纯音频格式(渲染走 probe+mix 短路,不渲视频链)。
    pub fn is_audio_only(self) -> bool {
        matches!(self, ExportFormat::M4a | ExportFormat::Mp3)
    }
}

/// 导出规格(MCP/CLI 参数面 → 渲染单源结构;进 RenderOptions 随管线透传)。
#[derive(Debug, Clone, Default)]
pub struct ExportSpec {
    pub format: Option<ExportFormat>,
    /// 画幅预设(vertical|9x16|horizontal|16x9|square|1x1|3x4|4x5)。
    pub preset: Option<String>,
    /// 清晰度档(1080/720/480;短边缩放)。
    pub tier: Option<u32>,
    /// 码率档(high|medium|low;估算值,显式 bitrate 覆盖)。
    pub bitrate_tier: Option<String>,
    /// 区域入点(ms;输出时间轴平移到 0)。
    pub in_ms: u64,
    /// 区域出点(ms;None = 到片尾)。
    pub out_ms: Option<u64>,
    /// 仅视频(全部音轨静音 + BGM 摘除,静音轨保留)。
    pub video_only: bool,
}

impl ExportSpec {
    /// 生效格式(缺省 mp4-h264)。
    pub fn effective_format(&self) -> ExportFormat {
        self.format.unwrap_or(ExportFormat::Mp4H264)
    }

    /// 是否声明了任一导出意图(缺省渲染路径判定:全 None = 现行为零变化)。
    pub fn is_present(&self) -> bool {
        self.format.is_some()
            || self.preset.is_some()
            || self.tier.is_some()
            || self.bitrate_tier.is_some()
            || self.in_ms > 0
            || self.out_ms.is_some()
            || self.video_only
    }
}

// ---------------- 预设/清晰度/码率映射(纯函数) ----------------

/// 预设 → 画幅覆盖(vertical/9x16 → 1080×1920;horizontal/16x9 → 1920×1080;
/// square/1x1 → 1080×1080;3x4 → 1080×1440;4x5 → 1080×1350)。与
/// render_variants 的 9x16/16x9/3x4 映射同源(3x4 = 1080×1440 逐字一致)。
pub fn preset_canvas(preset: &str) -> Option<Canvas> {
    match preset.to_ascii_lowercase().as_str() {
        "vertical" | "9x16" => Some(Canvas {
            width: 1080,
            height: 1920,
        }),
        "horizontal" | "16x9" => Some(Canvas {
            width: 1920,
            height: 1080,
        }),
        "square" | "1x1" => Some(Canvas {
            width: 1080,
            height: 1080,
        }),
        "3x4" => Some(Canvas {
            width: 1080,
            height: 1440,
        }),
        "4x5" => Some(Canvas {
            width: 1080,
            height: 1350,
        }),
        _ => None,
    }
}

/// 清晰度档 → 画幅缩放(**短边** = 档位;1080p 竖屏 = 1080×1920 的平台惯例),
/// 倍数取整到偶数、下限 64。允许上采样(确定性映射,不悄悄截断)。
pub fn tier_canvas(w: u32, h: u32, tier: u32) -> (u32, u32) {
    let tier = tier.max(64);
    let short = w.min(h).max(1);
    let factor = tier as f64 / short as f64;
    // 四舍五入到偶数(853.33 → 854:与平台 854×480 惯例一致;非向下取偶)
    let even = |v: f64| -> u32 { ((v / 2.0).round() as u32).max(32) * 2 };
    (even(w as f64 * factor), even(h as f64 * factor))
}

/// 码率档 → 估算 kbps(1080p 基准 × 高度/1080 线性折算;显式 bitrate 覆盖本值)。
pub fn tier_bitrate_kbps(tier: &str, height: u32) -> Option<u64> {
    let base = BITRATE_TIER_BASE_1080
        .iter()
        .find(|(n, _)| *n == tier.to_ascii_lowercase())
        .map(|(_, b)| *b)?;
    Some(((base as f64 * height as f64 / 1080.0).round() as u64).max(100))
}

// ---------------- 区域导出:工程级时间窗裁剪(纯函数) ----------------

/// 头部裁剪 timeline_ms 毫秒的源域消耗(ms;按速度分段积分,左点区间)。
/// freezeMs 定格区近似按速度段计(已登记;见模块注释)。
pub fn source_advance_ms(clip: &Clip, timeline_ms: u64) -> u64 {
    let mut used = 0f64;
    for (a, b, s) in cutforge_core::model::speed_segments(clip) {
        if a >= timeline_ms {
            break;
        }
        used += (b.min(timeline_ms) - a) as f64 * s;
    }
    used.round() as u64
}

/// 单片段钳制到 [in_ms, out_ms) 并平移(输出时间轴 = 输入时间轴 − in_ms);
/// 完全出窗 → None。头部裁剪按源域积分折算 sourceInMs(声画同源同折算)。
fn clamp_clip(c: &Clip, in_ms: u64, out_ms: u64) -> Option<Clip> {
    let end = c.start_ms + c.duration_ms;
    if end <= in_ms || c.start_ms >= out_ms {
        return None;
    }
    let mut n = c.clone();
    let new_start_raw = c.start_ms.max(in_ms);
    let head = new_start_raw - c.start_ms;
    let new_end = end.min(out_ms);
    n.start_ms = new_start_raw - in_ms;
    n.duration_ms = new_end - new_start_raw;
    if head > 0 && c.src.is_some() {
        let adv = source_advance_ms(c, head);
        n.source_in_ms = Some(c.source_in_ms.unwrap_or(0).saturating_add(adv));
    }
    Some(n)
}

/// 工程级时间窗裁剪(in_ms=0 且 out_ms=None 时返回克隆,零变化):
/// - 全轨种片段钳制 + 平移(文本/调整层无源,只裁时间窗);
/// - 各视频轨**新首段**的入向转场清除(转场语义 = 与前段边界,前段已出窗);
/// - BGM 保留(mix 按 windowed 总长循环铺满;相位从源 0 起播,已登记口径);
/// - total/边界转场/混音段等全部由 RenderPlan::build 在窗口化工程上重算。
pub fn window_project(project: &Project, in_ms: u64, out_ms: Option<u64>) -> Project {
    if in_ms == 0 && out_ms.is_none() {
        return project.clone();
    }
    // 出点缺省 = 全轨视音内容末端(与 plan.total_ms 同口径;文本轨不计)
    let total = project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video || t.kind == TrackKind::Audio)
        .flat_map(|t| t.clips.iter())
        .map(|c| c.start_ms + c.duration_ms)
        .max()
        .unwrap_or(0);
    let out = out_ms.unwrap_or(total).max(in_ms);
    let mut p = project.clone();
    let mut first_video_done: Vec<bool> = Vec::new();
    for t in &mut p.tracks {
        let mut kept: Vec<Clip> = Vec::new();
        for c in &t.clips {
            if let Some(n) = clamp_clip(c, in_ms, out) {
                kept.push(n);
            }
        }
        if t.kind == TrackKind::Video {
            first_video_done.push(kept.is_empty());
            // 先记占位,下方统一清首段转场(借用结束后再改)
        }
        t.clips = kept;
    }
    // 清各视频轨新首段的入向转场(前段出窗后转场语义失效)
    let mut vi = 0usize;
    for t in &mut p.tracks {
        if t.kind != TrackKind::Video {
            continue;
        }
        if first_video_done.get(vi) == Some(&false)
            && let Some(first) = t.clips.first_mut()
        {
            first.transition = None;
        }
        vi += 1;
    }
    p
}

/// 导出前的工程准备(画幅/清晰度/仅视频/时间窗,依序应用;纯函数)。
pub fn prepare_project(project: &Project, spec: &ExportSpec) -> Project {
    let mut p = project.clone();
    if let Some(pre) = &spec.preset
        && let Some(c) = preset_canvas(pre)
    {
        p.canvas = c;
    }
    if let Some(tier) = spec.tier {
        let (w, h) = tier_canvas(p.canvas.width, p.canvas.height, tier);
        p.canvas = Canvas {
            width: w,
            height: h,
        };
    }
    if spec.video_only {
        for t in &mut p.tracks {
            if t.kind == TrackKind::Video || t.kind == TrackKind::Audio {
                t.mute = Some(true);
            }
        }
        p.bgm = None;
    }
    p = window_project(&p, spec.in_ms, spec.out_ms);
    p
}

// ---------------- 输出路径(纯函数) ----------------

/// 导出产物路径:mp4-h264 保持既有命名(final_cutforge_{slug}_{W}x{H}.mp4,
/// 与缺省渲染同出口);其余格式加格式后缀避免互相覆盖;png-seq 为目录
/// (内含 frame_%04d.png)。
pub fn export_output_path(
    out_dir: &Path,
    slug: &str,
    w: u32,
    h: u32,
    format: ExportFormat,
) -> PathBuf {
    let slug = crate::steps::sanitize_slug(slug);
    match format {
        ExportFormat::Mp4H264 => out_dir.join(format!("final_cutforge_{slug}_{w}x{h}.mp4")),
        ExportFormat::PngSeq => {
            out_dir.join(format!("final_cutforge_{slug}_{w}x{h}_{}", format.as_str()))
        }
        other => out_dir.join(format!(
            "final_cutforge_{slug}_{w}x{h}_{}.{}",
            other.as_str(),
            other.ext()
        )),
    }
}

// ---------------- 各格式命令行(纯函数;[段] = 依次执行的 ffmpeg 运行) ----------------

/// h265(软件 libx265 + hvc1 标签;标签双写与既有重编码路径同源)。
pub fn h265_args(input: &Path, output: &Path, bitrate_kbps: Option<u64>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-i".into(),
        input.to_string_lossy().into(),
        "-c:v".into(),
        "libx265".into(),
        "-preset".into(),
        "veryfast".into(),
    ];
    match bitrate_kbps {
        Some(k) => args.extend(["-b:v".into(), format!("{k}k")]),
        None => args.extend(["-crf".into(), "28".into()]),
    }
    args.extend([
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-tag:v".into(),
        "hvc1".into(),
    ]);
    args.extend(["-vf".into(), crate::encode::COLOR_TAG_SET_PARAMS.into()]);
    args.extend(
        crate::encode::color_tag_args()
            .iter()
            .map(|s| s.to_string()),
    );
    args.extend(["-c:a".into(), "copy".into()]);
    args.push(output.to_string_lossy().into());
    args
}

/// gif 段 1:调色板生成(palettegen;fps=12 固定 + lanczos 缩放到画布宽)。
pub fn gif_pass1_args(input: &Path, palette_out: &Path, w: u32) -> Vec<String> {
    vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-i".into(),
        input.to_string_lossy().into(),
        "-vf".into(),
        format!("fps={GIF_FPS},scale={w}:-2:flags=lanczos,palettegen"),
        palette_out.to_string_lossy().into(),
    ]
}

/// gif 段 2:paletteuse 应用调色板出 GIF(与段 1 同 fps/scale,逐帧对齐)。
pub fn gif_pass2_args(input: &Path, palette: &Path, output: &Path, w: u32) -> Vec<String> {
    vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-i".into(),
        input.to_string_lossy().into(),
        "-i".into(),
        palette.to_string_lossy().into(),
        "-lavfi".into(),
        format!("[0:v]fps={GIF_FPS},scale={w}:-2:flags=lanczos[x];[x][1:v]paletteuse"),
        output.to_string_lossy().into(),
    ]
}

/// 纯音频(m4a = 零重编码 copy;mp3 = libmp3lame 192k)。
pub fn audio_export_args(format: ExportFormat, input: &Path, output: &Path) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-i".into(),
        input.to_string_lossy().into(),
        "-vn".into(),
    ];
    match format {
        ExportFormat::M4a => args.extend(["-c:a".into(), "copy".into()]),
        ExportFormat::Mp3 => args.extend([
            "-c:a".into(),
            "libmp3lame".into(),
            "-b:a".into(),
            "192k".into(),
        ]),
        _ => {}
    }
    args.push(output.to_string_lossy().into());
    args
}

/// png 序列(image2 %04d;帧率随工程 fps,输出目录须已存在)。
pub fn png_seq_args(input: &Path, pattern: &Path) -> Vec<String> {
    vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-i".into(),
        input.to_string_lossy().into(),
        pattern.to_string_lossy().into(),
    ]
}

/// 导出 encode 的完整运行清单:返回 (依次执行的参数列表, 编码器名)。
/// mp4-h264/mov 复用既有 final_encode_args(缺省 remux+标签/hw/重编码语义零变化)。
pub fn export_encode_runs(
    spec: &ExportSpec,
    input: &Path,
    output: &Path,
    palette_out: &Path,
    canvas: (u32, u32),
    opts: &RenderOptions,
    hw: Option<&'static str>,
) -> (Vec<Vec<String>>, String) {
    let (w, h) = canvas;
    let format = spec.effective_format();
    match format {
        ExportFormat::Mp4H264 | ExportFormat::Mov => {
            let (args, enc) = crate::encode::final_encode_args(input, output, opts, hw);
            (vec![args], enc)
        }
        ExportFormat::Mp4H265 => {
            let kbps = opts.bitrate_kbps.or_else(|| {
                spec.bitrate_tier
                    .as_deref()
                    .and_then(|t| tier_bitrate_kbps(t, h))
            });
            (vec![h265_args(input, output, kbps)], "libx265".into())
        }
        ExportFormat::Gif => (
            vec![
                gif_pass1_args(input, palette_out, w),
                gif_pass2_args(input, palette_out, output, w),
            ],
            "palettegen+paletteuse".into(),
        ),
        ExportFormat::M4a | ExportFormat::Mp3 => (
            vec![audio_export_args(format, input, output)],
            "copy/audio".into(),
        ),
        ExportFormat::PngSeq => (
            vec![png_seq_args(input, &output.join("frame_%04d.png"))],
            "image2".into(),
        ),
        ExportFormat::FramePng => {
            unreachable!("frame-png 在 CLI/MCP 面复用 render_frame,不进整片管线")
        }
    }
}

/// 导出 encode(册六 T6.3):按 ExportFormat 分派命令运行清单(export.rs 单源),
/// gif 的调色板写渲染缓存 tmp(用完即清,导出目录只留成片);mp4-h264/mov 复用
/// 既有 final_encode_args(缺省 remux+标签/hw 试编降级语义零变化);h265 为软件
/// 编码(hw 请求 WARN 留痕不冒充);png-seq 清点帧数;mp4 容器面做色彩复验。
pub(crate) fn exec_export_encode(
    plan: &RenderPlan,
    video_input: &Path,
    spec: &ExportSpec,
) -> Result<(StepReport, PathBuf, Vec<String>), String> {
    let format = spec.effective_format();
    let output = export_output_path(
        &plan.out_dir,
        &plan.slug,
        plan.canvas_w,
        plan.canvas_h,
        format,
    );
    let mut detail = json!({"format": format.as_str()});
    let mut warns: Vec<String> = Vec::new();
    if format == ExportFormat::Mp4H265 && plan.opts.encoder.as_deref() == Some("hw") {
        warns.push("h265 出口为软件编码(libx265),hw 请求不生效(诚实降级);".into());
    }
    let hw = if plan.opts.encoder.as_deref() == Some("hw")
        && matches!(format, ExportFormat::Mp4H264 | ExportFormat::Mov)
    {
        crate::encode::resolve_hw_candidate()
    } else {
        None
    };
    if format == ExportFormat::PngSeq {
        std::fs::create_dir_all(&output).map_err(|e| format!("png 序列目录创建失败: {e}"))?;
    }
    let palette_rel = crate::cache::tmp_rel(&format!(
        "export-palette-{}.png",
        crate::cache::key_hex(
            &json!({"slug": plan.slug, "canvas": [plan.canvas_w, plan.canvas_h], "fmt": format.as_str()})
        )
    ));
    let palette = plan.cache_dir.join(&palette_rel);
    let (runs, enc) = export_encode_runs(
        spec,
        video_input,
        &output,
        &palette,
        (plan.canvas_w, plan.canvas_h),
        &plan.opts,
        hw,
    );
    detail["encoder"] = json!(enc);
    let mut cmds: Vec<String> = Vec::new();
    for args in &runs {
        cmds.push(args.join(" "));
        crate::run_ff("ffmpeg", &crate::strs(args))?;
    }
    if format == ExportFormat::Gif {
        let _ = cutforge_io::atomic::remove(&palette);
        detail["gifFps"] = json!(GIF_FPS);
    }
    if format == ExportFormat::PngSeq {
        let frames = std::fs::read_dir(&output)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
                    .count()
            })
            .unwrap_or(0);
        detail["frames"] = json!(frames);
        detail["output"] = json!(output.to_string_lossy());
        return Ok((StepReport::new("encode", detail), output, cmds));
    }
    match crate::encode::verify_color_tags(&output) {
        Ok(tags) => detail["colorTags"] = tags,
        Err(e) => warns.push(format!("色彩标签复验不可达: {e};")),
    }
    detail["output"] = json!(output.to_string_lossy());
    if !warns.is_empty() {
        detail["warnings"] = json!(warns);
    }
    Ok((StepReport::new("encode", detail), output, cmds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project(v: serde_json::Value) -> Project {
        serde_json::from_value(v).unwrap()
    }

    fn base_project() -> serde_json::Value {
        json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "exp", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [
                {"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000,
                     "sourceInMs": 100, "role": "voice", "volume": 1.0},
                    {"id": "V1-002", "src": "a.mp4", "startMs": 2000, "durationMs": 2000,
                     "role": "voice", "transition": {"type": "fade", "durMs": 300}},
                    {"id": "V1-003", "src": "b.mp4", "startMs": 4000, "durationMs": 2000, "volume": 0}
                ]},
                {"id": "A1", "kind": "audio", "clips": [
                    {"id": "A1-001", "src": "sfx.mp3", "startMs": 500, "durationMs": 5000, "volume": 0.5}
                ]},
                {"id": "T1", "kind": "text", "clips": [
                    {"id": "T1-001", "startMs": 1000, "durationMs": 3000, "text": "字幕"}
                ]}
            ]
        })
    }

    #[test]
    fn format_parse_roundtrip_and_flags() {
        for s in [
            "mp4-h264",
            "mp4-h265",
            "mov",
            "gif",
            "m4a",
            "mp3",
            "png-seq",
            "frame-png",
        ] {
            let f = ExportFormat::parse(s).unwrap_or_else(|| panic!("{s}"));
            assert_eq!(f.as_str(), s, "{s}");
        }
        assert_eq!(ExportFormat::parse("MP4").unwrap(), ExportFormat::Mp4H264);
        assert!(ExportFormat::parse("avi").is_none());
        assert!(ExportFormat::M4a.is_audio_only() && ExportFormat::Mp3.is_audio_only());
        assert!(!ExportFormat::Gif.is_audio_only());
        assert_eq!(ExportFormat::PngSeq.ext(), "");
        assert_eq!(ExportFormat::Mov.ext(), "mov");
    }

    /// 预设映射:三主预设 + 比率别名;3x4 与 render_variants 逐字一致;未知拒绝。
    #[test]
    fn preset_canvas_maps_and_rejects() {
        assert_eq!(
            preset_canvas("vertical"),
            Some(Canvas {
                width: 1080,
                height: 1920
            })
        );
        assert_eq!(preset_canvas("9x16"), preset_canvas("vertical"));
        assert_eq!(
            preset_canvas("horizontal"),
            Some(Canvas {
                width: 1920,
                height: 1080
            })
        );
        assert_eq!(preset_canvas("16x9"), preset_canvas("horizontal"));
        assert_eq!(
            preset_canvas("square"),
            Some(Canvas {
                width: 1080,
                height: 1080
            })
        );
        assert_eq!(preset_canvas("1x1"), preset_canvas("square"));
        assert_eq!(
            preset_canvas("3x4"),
            Some(Canvas {
                width: 1080,
                height: 1440
            })
        );
        assert_eq!(
            preset_canvas("4x5"),
            Some(Canvas {
                width: 1080,
                height: 1350
            })
        );
        assert_eq!(preset_canvas("21x9"), None);
    }

    /// 清晰度档 = 短边缩放:竖屏 1080×1920@1080p 不变;@720p → 720×1280(偶数);
    /// 横屏 1920×1080@480p → 854×480;下限 64;偶数锁。
    #[test]
    fn tier_canvas_scales_short_side_even() {
        assert_eq!(tier_canvas(1080, 1920, 1080), (1080, 1920));
        assert_eq!(tier_canvas(1080, 1920, 720), (720, 1280));
        assert_eq!(tier_canvas(1920, 1080, 480), (854, 480));
        assert_eq!(tier_canvas(1080, 1080, 480), (480, 480));
        assert_eq!(tier_canvas(100, 100, 32), (64, 64), "下限 64");
        let (w, h) = tier_canvas(1080, 1920, 480);
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
    }

    /// 码率档:1080p 基准 + 高度线性折算;未知档 None。
    #[test]
    fn tier_bitrate_estimates() {
        assert_eq!(tier_bitrate_kbps("high", 1080), Some(12000));
        assert_eq!(tier_bitrate_kbps("medium", 1080), Some(8000));
        assert_eq!(tier_bitrate_kbps("low", 1080), Some(4000));
        assert_eq!(tier_bitrate_kbps("low", 480), Some(1778));
        assert_eq!(tier_bitrate_kbps("ultra", 1080), None);
    }

    /// 源域折算:常速 2x 头部裁 1000ms → 源域 2000ms;曲线分段积分逐段累计。
    #[test]
    fn source_advance_integrates_speed() {
        let mk = |v: serde_json::Value| -> Clip { serde_json::from_value(v).unwrap() };
        let c = mk(
            json!({"id": "c", "startMs": 0, "durationMs": 4000, "sourceInMs": 500, "speed": 2.0}),
        );
        assert_eq!(source_advance_ms(&c, 1000), 2000);
        let c2 = mk(
            json!({"id": "c", "startMs": 0, "durationMs": 4000, "sourceInMs": 0,
            "speedCurve": [{"atMs": 0, "speed": 1.0}, {"atMs": 1000, "speed": 1.0}, {"atMs": 4000, "speed": 3.0}]}),
        );
        // [0,1000)@1.0(区间均值 (1+1)/2)+ [1000,+)@2.0(区间均值 (1+3)/2)
        assert_eq!(source_advance_ms(&c2, 1000), 1000);
        assert_eq!(
            source_advance_ms(&c2, 2000),
            1000 + 2000,
            "第二段 [1000,2000) @2.0"
        );
        assert_eq!(source_advance_ms(&c2, 0), 0);
    }

    /// 区域窗口:出窗片段剔除、跨界片段钳制平移、头部源域折算、首段转场清除、
    /// 文本/调整层同窗、BGM 保留;in=0 且无 out 时零变化。
    #[test]
    fn window_project_clamps_shifts_and_clears_first_transition() {
        let p = project(base_project());
        // 零窗口 = 克隆零变化
        let same = window_project(&p, 0, None);
        assert_eq!(
            serde_json::to_string(&same).unwrap(),
            serde_json::to_string(&p).unwrap()
        );
        // 窗口 [1000, 5000) → 平移 -1000
        let w = window_project(&p, 1000, Some(5000));
        let v = &w.tracks[0].clips;
        // V1-001 [0,2000) → 头部截断 [0,1000)(窗口起点 1000 在片段中段)
        assert_eq!((v[0].start_ms, v[0].duration_ms), (0, 1000));
        assert_eq!(
            v[0].source_in_ms,
            Some(1100),
            "头部裁 1000ms → 源域 +1000(常速)"
        );
        assert!(v[0].transition.is_none(), "新首段(原首段)转场清除");
        // V1-002 [2000,4000) → 完整保留,平移到 [1000,3000);其转场保留(前段仍在)
        assert_eq!((v[1].start_ms, v[1].duration_ms), (1000, 2000));
        assert!(v[1].transition.is_some(), "非首段转场保留");
        // V1-003 [4000,6000) → 头部截断 [3000,5000),源域 +0(常速,head=0? head=0)
        assert_eq!((v[2].start_ms, v[2].duration_ms), (3000, 1000));
        // 音频 A1-001 [500,5500) → [0,4000),头部裁 500ms 源域折算
        let a = &w.tracks[1].clips[0];
        assert_eq!((a.start_ms, a.duration_ms), (0, 4000));
        assert_eq!(
            a.source_in_ms,
            Some(500),
            "常速 1.0 头部 500ms → 源域 500ms"
        );
        // 文本 T1-001 [1000,4000) → [0,3000)
        let t = &w.tracks[2].clips[0];
        assert_eq!((t.start_ms, t.duration_ms), (0, 3000));
        assert_eq!(w.slug, "exp", "slug 不变");
        // 计划层重算:总长 = 视音最大末端 4000
        let plan = crate::plan::RenderPlan::build(&w, Path::new("/w"), None);
        assert_eq!(plan.total_ms, 4000);
    }

    /// 头部裁剪的源域折算(变速):窗口起在 2x 片段中部 → sourceIn 前移 2x。
    #[test]
    fn window_head_cut_folds_source_domain() {
        let p = project(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "w2", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 4000,
                 "sourceInMs": 0, "speed": 2.0, "role": "voice"}
            ]}]
        }));
        let w = window_project(&p, 1000, None);
        let c = &w.tracks[0].clips[0];
        assert_eq!((c.start_ms, c.duration_ms), (0, 3000));
        assert_eq!(
            c.source_in_ms,
            Some(2000),
            "2x 头部裁 1000ms → 源域 +2000ms"
        );
        // 全出窗 → 空轨(渲染端守卫 PRECONDITION,由 render_export 把关)
        let w2 = window_project(&p, 9000, None);
        assert!(w2.tracks[0].clips.is_empty());
    }

    /// prepare_project:预设 + 清晰度 + 仅视频 + 窗口依序生效;仅视频 = 音轨静音 +
    /// BGM 摘除(文本轨不受 mute 影响——文本静默是另一语义)。
    #[test]
    fn prepare_project_applies_preset_tier_video_only() {
        let mut p = project(base_project());
        p.bgm = Some(serde_json::from_value(json!({
            "src": "bgm.mp3", "gainDb": -12.0, "ducking": false, "loop": false,
            "duckThreshold": 0.03, "duckRatio": 8.0, "duckAttackMs": 80.0, "duckReleaseMs": 500.0
        })).unwrap());
        let spec = ExportSpec {
            preset: Some("horizontal".into()),
            tier: Some(480),
            video_only: true,
            in_ms: 0,
            out_ms: None,
            ..Default::default()
        };
        let prepared = prepare_project(&p, &spec);
        assert_eq!(prepared.canvas.width, 854);
        assert_eq!(
            prepared.canvas.height, 480,
            "横屏短边 1080 → 480p = 854×480"
        );
        assert!(prepared.bgm.is_none(), "仅视频摘除 BGM");
        for t in &prepared.tracks {
            match t.kind {
                TrackKind::Video | TrackKind::Audio => assert_eq!(t.mute, Some(true)),
                TrackKind::Text => assert_eq!(t.mute, None, "文本轨不吃 mute"),
                _ => {}
            }
        }
    }

    /// 输出路径:mp4-h264 保持既有命名;其余带格式后缀;png-seq 为目录。
    #[test]
    fn export_paths_are_disjoint() {
        let out = Path::new("/w/06_成片输出");
        let mp4 = export_output_path(out, "片", 1080, 1920, ExportFormat::Mp4H264);
        assert!(mp4.ends_with("final_cutforge_片_1080x1920.mp4"));
        let mut paths = vec![mp4];
        for f in [
            ExportFormat::Mp4H265,
            ExportFormat::Mov,
            ExportFormat::Gif,
            ExportFormat::M4a,
            ExportFormat::Mp3,
            ExportFormat::PngSeq,
        ] {
            let p = export_output_path(out, "片", 1080, 1920, f);
            assert!(
                p.to_string_lossy().contains(f.as_str()),
                "{}: {p:?}",
                f.as_str()
            );
            paths.push(p);
        }
        for i in 0..paths.len() {
            for j in i + 1..paths.len() {
                assert_ne!(paths[i], paths[j], "导出产物路径必须互不覆盖");
            }
        }
    }

    /// 命令行面:h265 标签/crf/码率;gif 两段(palettegen → paletteuse,fps=12);
    /// 纯音频 -vn(m4a copy / mp3 libmp3lame);png 序列 %04d;mp4/mov 复用既有面。
    #[test]
    fn export_encode_runs_shape() {
        let input = Path::new("/c/mid.mp4");
        let out = Path::new("/w/o");
        let pal = Path::new("/c/pal.png");
        let spec = ExportSpec::default();
        // mp4-h264:复用 final_encode_args(remux 缺省)
        let (runs, enc) = export_encode_runs(
            &spec,
            input,
            &out.join("o.mp4"),
            pal,
            (1080, 1920),
            &RenderOptions::default(),
            None,
        );
        assert_eq!((runs.len(), enc.as_str()), (1, "copy"));
        let s = runs[0].join("\u{1}");
        assert!(s.contains("-c\u{1}copy") && s.contains("bt709"), "{s}");
        // mov:同面,容器随扩展名
        let (runs, _) = export_encode_runs(
            &spec,
            input,
            &out.join("o.mov"),
            pal,
            (1080, 1920),
            &RenderOptions::default(),
            None,
        );
        assert!(runs[0].last().unwrap().ends_with("o.mov"));
        // h265:libx265 + hvc1 + bt709;crf 缺省 / 码率档覆盖
        let h = ExportSpec {
            format: Some(ExportFormat::Mp4H265),
            ..Default::default()
        };
        let (runs, enc) = export_encode_runs(
            &h,
            input,
            &out.join("o.mp4"),
            pal,
            (1080, 1920),
            &RenderOptions::default(),
            None,
        );
        assert_eq!(enc, "libx265");
        let s = runs[0].join("\u{1}");
        assert!(
            s.contains("-c:v\u{1}libx265")
                && s.contains("-tag:v\u{1}hvc1")
                && s.contains("-crf\u{1}28"),
            "{s}"
        );
        let h2 = ExportSpec {
            format: Some(ExportFormat::Mp4H265),
            bitrate_tier: Some("low".into()),
            ..Default::default()
        };
        let (runs, _) = export_encode_runs(
            &h2,
            input,
            &out.join("o.mp4"),
            pal,
            (1080, 1080),
            &RenderOptions::default(),
            None,
        );
        assert!(
            runs[0].join("\u{1}").contains("-b:v\u{1}4000k"),
            "1080p low = 4000k"
        );
        // gif:两段,fps=12,palettegen → paletteuse
        let g = ExportSpec {
            format: Some(ExportFormat::Gif),
            ..Default::default()
        };
        let (runs, enc) = export_encode_runs(
            &g,
            input,
            &out.join("o.gif"),
            pal,
            (1080, 1920),
            &RenderOptions::default(),
            None,
        );
        assert_eq!((runs.len(), enc.as_str()), (2, "palettegen+paletteuse"));
        assert!(
            runs[0].join("\u{1}").contains("fps=12")
                && runs[0].join("\u{1}").contains("palettegen")
        );
        let s2 = runs[1].join("\u{1}");
        assert!(s2.contains("paletteuse") && s2.ends_with("o.gif"), "{s2}");
        // 纯音频:-vn;m4a copy / mp3 libmp3lame
        let a = ExportSpec {
            format: Some(ExportFormat::M4a),
            ..Default::default()
        };
        let (runs, _) = export_encode_runs(
            &a,
            input,
            &out.join("o.m4a"),
            pal,
            (1080, 1920),
            &RenderOptions::default(),
            None,
        );
        let s = runs[0].join("\u{1}");
        assert!(s.contains("-vn") && s.contains("-c:a\u{1}copy"), "{s}");
        let m = ExportSpec {
            format: Some(ExportFormat::Mp3),
            ..Default::default()
        };
        let (runs, _) = export_encode_runs(
            &m,
            input,
            &out.join("o.mp3"),
            pal,
            (1080, 1920),
            &RenderOptions::default(),
            None,
        );
        let s = runs[0].join("\u{1}");
        assert!(
            s.contains("-vn") && s.contains("libmp3lame") && s.contains("-b:a\u{1}192k"),
            "{s}"
        );
        // png 序列:frame_%04d.png
        let q = ExportSpec {
            format: Some(ExportFormat::PngSeq),
            ..Default::default()
        };
        let (runs, _) = export_encode_runs(
            &q,
            input,
            &out.join("d"),
            pal,
            (1080, 1920),
            &RenderOptions::default(),
            None,
        );
        assert!(runs[0].last().unwrap().ends_with("frame_%04d.png"));
    }

    /// ExportSpec 缺省 = 无导出意图(is_present false);任一参数置位即 true。
    #[test]
    fn spec_presence_gate() {
        assert!(!ExportSpec::default().is_present());
        assert!(
            ExportSpec {
                format: Some(ExportFormat::Gif),
                ..Default::default()
            }
            .is_present()
        );
        assert!(
            ExportSpec {
                in_ms: 1,
                ..Default::default()
            }
            .is_present()
        );
        assert!(
            ExportSpec {
                out_ms: Some(1),
                ..Default::default()
            }
            .is_present()
        );
        assert!(
            ExportSpec {
                video_only: true,
                ..Default::default()
            }
            .is_present()
        );
    }
}
