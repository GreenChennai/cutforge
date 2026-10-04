// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 单帧渲染(T2.4「精确预览」的服务端支撑):对打开的工程渲染指定时间点的
//! 一帧合成画面(PNG/JPEG),供壳预览区显示**最终效果**——含转场/叠加/字幕烧录,
//! 消除"预览与成片落差"(R7)的第一步。
//!
//! 复用路径(不建并行管线):video 链走 RenderPlan 的既有步骤函数
//! segment → compose → overlay(段缓存命中即跳过 ffmpeg),随后对合成基片
//! `-ss` 精确 seek 抽取单帧;ASS 字幕以 subtitles 滤镜直接烧在帧上
//! (cwd=缓存根的相对路径技巧与 exec_subtitle 同源)。
//!
//! 缓存(frame 层,cache.rs):键 = **工作区指纹**(fresh.rs,project/notes/真相源/
//! rev/oplog 全部磁盘输入——改一笔即 miss,绝不给陈旧帧)叠加 atMs(100ms 量化)、
//! 画幅、渲染版本、ASS 字节哈希。命中即零 ffmpeg;未命中先跑 video 链
//! (与整片渲染共享段缓存)再抽帧。

use crate::cache;
use crate::plan::RenderPlan;
use cutforge_core::model::{Clip, Project};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// 帧图格式(png 缺省 / jpeg)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFormat {
    Png,
    Jpeg,
}

impl FrameFormat {
    /// CLI/契约字符串解析(png|jpeg;大小写不敏感)。
    pub fn parse(s: &str) -> Option<FrameFormat> {
        match s.to_ascii_lowercase().as_str() {
            "png" => Some(FrameFormat::Png),
            "jpeg" | "jpg" => Some(FrameFormat::Jpeg),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            FrameFormat::Png => "png",
            FrameFormat::Jpeg => "jpeg",
        }
    }

    /// 落盘扩展名(frame 层缓存文件名用)。
    pub fn ext(self) -> &'static str {
        match self {
            FrameFormat::Png => ".png",
            FrameFormat::Jpeg => ".jpg",
        }
    }
}

/// atMs 量化到 100ms 网格(向下取整):壳播放头毫秒级抖动不至于击穿缓存;
/// 量化后的时间才是实际抽帧点与缓存键输入(两者恒一致,响应如实回传)。
pub fn quantize_ms(at_ms: u64) -> u64 {
    at_ms - at_ms % 100
}

/// 单帧渲染结果。
#[derive(Debug, Clone)]
pub struct FrameOutcome {
    /// 落盘绝对路径(`<工程>/.cutforge/render-cache/frame/<key>.<ext>`)。
    pub output: PathBuf,
    /// 实际抽帧时间点(100ms 量化后;与缓存键一致)。
    pub at_ms: u64,
    /// true = frame 层缓存命中(零 ffmpeg)。
    pub cached: bool,
    /// frame 层缓存键(调试/对账用)。
    pub key: String,
}

/// 抽帧命令行(纯函数,可离线单测):`-ss` 输入侧精确 seek(解码丢弃到目标帧)+
/// `-frames:v 1`。带 ASS 时先 setpts 把输入侧 seek 归零的 PTS 平移回时间线时刻,
/// 再喂链式 subtitles 滤镜(ass_rels 相对缓存根,规避盘符冒号转义;与 exec_subtitle
/// 同技巧;册四 T4.7 起支持外部 + 文本轨生成 ASS 串联,单条时参数与既有逐字一致)。
pub fn frame_extract_args(
    base: &Path,
    at_ms: u64,
    fmt: FrameFormat,
    ass_rels: &[String],
    out: &Path,
) -> Vec<String> {
    let sec = at_ms as f64 / 1000.0;
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-ss".into(),
        format!("{sec:.3}"),
        "-i".into(),
        base.to_string_lossy().into(),
    ];
    let mut vf = String::new();
    if at_ms > 0 {
        vf.push_str(&format!("setpts=PTS+{sec:.3}/TB"));
    }
    for rel in ass_rels {
        if !vf.is_empty() {
            vf.push(',');
        }
        vf.push_str(&format!("subtitles={rel}"));
    }
    if !vf.is_empty() {
        args.extend(["-vf".into(), vf]);
    }
    args.extend(["-frames:v".into(), "1".into(), "-f".into(), "image2".into()]);
    if fmt == FrameFormat::Jpeg {
        // mjpeg 质量档(PNG 无损,不吃 qscale)
        args.extend(["-q:v".into(), "2".into()]);
    }
    args.push(out.to_string_lossy().into());
    args
}

/// 片段在时间线上**真实可产出**的内容长度(ms;不含转场尾帧扩展,单帧越界守卫用)。
/// avail = 源域可用毫秒(源时长 − sourceIn;None = 探测不可用,退回未折算口径)。
/// 口径(I1 起与 segment::source_clamped_clip 的渲染面同源):
/// - 源窗完全越出素材(可用预算 = 0)→ 0:整窗无源可读,守卫拦下给
///   PRECONDITION(渲染面无从定格,任何帧都产不出);
/// - 源部分可用 → **源耗尽的尾段按末帧定格计满**(segment 步已同口径钳制:缺额
///   tpad 末帧补长,与 freezeMs 定格同哲学、与时间线空隙钳前段末帧同哲学),
///   content = durationMs——定格区是真实可见画面,抽帧放行;
/// - 源充足 / 探测不可用 → durationMs(与投影 endMs 一致,行为不变)。
pub fn clip_content_len_ms(clip: &Clip, avail_ms: Option<u64>) -> u64 {
    let Some(avail) = avail_ms else {
        return clip.duration_ms;
    };
    let budget = avail.saturating_sub(clip.source_in_ms.unwrap_or(0));
    if budget == 0 {
        return 0;
    }
    clip.duration_ms
}

/// 时间线 atMs → 合成域毫秒(单帧抽帧专用;与 compose 两路径 offset 口径同源)。
///
/// - `xfade`:与 [`crate::steps::compose_xfade_args`] 同口径——clip i 的合成起点
///   = 前序名义时长累计(ADR-0023),段内偏移直加;
/// - concat:与 concat 顺序拼接一致——clip i 合成起点 = 前序段实际时长累计
///   (`seg_durs` = ffprobe 探测的段文件秒数;None 项回落名义时长);
/// - atMs 不在任何片段区间(时间线空隙)→ `GapClamp::PreviousEnd`(定格前一
///   片段末帧)/ `Start`(首帧);合成域钳末端 - 40ms 防 EOF 空产出。
pub fn timeline_to_compose_ms(
    clips: &[Clip],
    xfade: bool,
    at_ms: u64,
    seg_durs: &[Option<f64>],
) -> u64 {
    const TAIL_GUARD_MS: u64 = 40;
    if clips.is_empty() {
        return 0;
    }
    // 找 at 所在片段(时间线区间 [start, start+dur));空隙取前段(无前段取首段)
    let mut idx = 0usize;
    let mut in_clip = false;
    for (i, c) in clips.iter().enumerate() {
        let s = c.start_ms;
        let e = s + c.duration_ms;
        if at_ms >= s && at_ms < e {
            idx = i;
            in_clip = true;
            break;
        }
        if s > at_ms {
            // 落在本段之前的空隙:钳前段末端
            idx = i.saturating_sub(1);
            break;
        }
        idx = i;
    }
    if !in_clip
        && at_ms
            >= clips
                .last()
                .map(|c| c.start_ms + c.duration_ms)
                .unwrap_or(0)
    {
        idx = clips.len() - 1;
    }
    let seg_dur_ms = |i: usize| -> f64 {
        match seg_durs.get(i).copied().flatten() {
            Some(sec) => sec * 1000.0,
            None => clips[i].duration_ms as f64,
        }
    };
    // 合成起点累计
    let base: f64 = clips[..idx]
        .iter()
        .enumerate()
        .map(|(j, c)| {
            if xfade {
                c.duration_ms as f64
            } else {
                seg_dur_ms(j)
            }
        })
        .sum();
    let clip = &clips[idx];
    let offset_in = if in_clip {
        (at_ms.saturating_sub(clip.start_ms)) as f64
    } else if at_ms < clips[0].start_ms {
        // 头部(早于首段):首帧
        0.0
    } else {
        // 空隙/尾部:定格本段末帧(段实际末端 - 保险)
        (seg_dur_ms(idx) - TAIL_GUARD_MS as f64).max(0.0)
    };
    let t = base + offset_in;
    let total: f64 = (0..clips.len())
        .map(|j| {
            if xfade {
                clips[j].duration_ms as f64
            } else {
                seg_dur_ms(j)
            }
        })
        .sum();
    t.min((total - TAIL_GUARD_MS as f64).max(0.0)).max(0.0) as u64
}

/// 单帧渲染主入口(同步;命中缓存时零 ffmpeg,未命中复用段缓存后一次抽帧)。
/// 错误串带 `NO_CONFIG:` / `PRECONDITION:` 前缀的,调用方(MCP 面)按 5.4 码映射。
pub fn render_frame(
    project: &Project,
    project_dir: &Path,
    ass_path: Option<&Path>,
    at_ms: u64,
    fmt: FrameFormat,
) -> Result<FrameOutcome, String> {
    render_frame_opts(project, project_dir, ass_path, at_ms, fmt, false)
}

/// 同 [`render_frame`],代理预览开关显式给定(册四 T4.1;开关入帧缓存键——
/// 代理帧与原片帧不共享条目)。文本轨文本片段经 textass 生成 ASS 一并烧录。
pub fn render_frame_opts(
    project: &Project,
    project_dir: &Path,
    ass_path: Option<&Path>,
    at_ms: u64,
    fmt: FrameFormat,
    use_proxy: bool,
) -> Result<FrameOutcome, String> {
    let at_q = quantize_ms(at_ms);
    // 工作区指纹:project/notes/wordline/cutlist/rev/oplog 的字节级摘要(fresh.rs 单一实现)
    let fp = cutforge_io::fresh::disk_fingerprint(project_dir)
        .ok_or_else(|| "NO_CONFIG: 工程不可读,无法计算工作区指纹".to_string())?;
    let ass_bytes = match ass_path {
        Some(p) => Some(std::fs::read(p).map_err(|e| format!("NO_CONFIG: ass 不可读: {e}"))?),
        None => None,
    };
    let text_bytes = crate::textass::generate(project).map(|s| s.into_bytes());
    let combined: Option<Vec<u8>> = match (&ass_bytes, &text_bytes) {
        (None, None) => None,
        (a, b) => {
            let mut v = Vec::new();
            v.extend(a.iter().flatten().copied());
            v.extend(b.iter().flatten().copied());
            Some(v)
        }
    };
    let canvas = (project.canvas.width, project.canvas.height);
    let key = cache::frame_key_proxy(
        &fp.cache_key(),
        at_q,
        canvas,
        fmt.as_str(),
        combined.as_deref(),
        use_proxy,
    );
    let cache_root = project_dir.join(cache::CACHE_ROOT);
    cache::ensure_dirs(&cache_root)?;
    let mut idx = cache::CacheIndex::load(&cache_root);
    let now = cache::now_secs();

    // 命中即返回(清单 load 时已 prune,条目在 = 文件在)
    if let Some(rel) = idx.touch("frame", &key, now) {
        return Ok(FrameOutcome {
            output: cache_root.join(rel),
            at_ms: at_q,
            cached: true,
            key,
        });
    }

    // 未命中:video 链(复用 RenderPlan 步骤函数与段缓存)→ 抽帧
    let plan = RenderPlan::build_opts(project, project_dir, None, use_proxy);
    if plan.video_clips.is_empty() {
        return Err("PRECONDITION: 时间线无视频片段,无帧可渲染".into());
    }
    let video_end = plan
        .video_clips
        .iter()
        .map(|c| c.start_ms + c.duration_ms)
        .max()
        .unwrap_or(0);
    if at_q >= video_end {
        return Err(format!(
            "PRECONDITION: atMs({at_q}) 超出视频时间线时长({video_end}ms)"
        ));
    }
    // 越界守卫(I1 口径):content_end = 真实可产出末端——源耗尽的尾段已被
    // segment 步按末帧定格补长(定格区可抽帧),只有「整窗无源可读」的片段
    // content = 0;此处拦 at 落在全部片段都无源的区间,不再给 ffmpeg 空产出。
    // 源可用时长经 ffprobe 探测;探测不可用 → 退回未折算口径(渲染失败如实上报)。
    let content_end = plan
        .video_clips
        .iter()
        .map(|c| {
            let avail = c.src.as_deref().and_then(|src| {
                if !cutforge_io::probe::ffprobe_available() {
                    return None;
                }
                cutforge_io::probe::probe(&plan.project_dir.join(src))
                    .ok()
                    .map(|info| info.duration_ms())
            });
            c.start_ms + clip_content_len_ms(c, avail)
        })
        .max()
        .unwrap_or(0);
    if at_q >= content_end {
        return Err(format!(
            "PRECONDITION: atMs({at_q}) 超出真实内容末端({content_end}ms;时间线标称 {video_end}ms,源窗整体越出素材的片段无帧可产)"
        ));
    }
    let (_rep_seg, seg_files, seg_keys, _hits, _misses) = crate::exec_segment(&plan, &mut idx)?;
    let (_rep_c, composed, compose_key, _cmds_c) =
        crate::exec_compose(&plan, &mut idx, &seg_files, &seg_keys)?;
    let (_rep_o, overlaid, overlay_key, _cmds_o) =
        crate::exec_overlay(&plan, &mut idx, &composed, &compose_key)?;
    // 册五 T5.4 调整层:单帧预览与成片同链(exec_adjust 空透传),预览无落差
    let (_rep_a, base_video, _video_key, _cmds_a) =
        crate::exec_adjust(&plan, &mut idx, &overlaid, &overlay_key)?;

    // 时间域映射:合成片(concat 拼接/xfade 名义累计)与时间线在**空隙/转场**
    // 处不一致,直接 -ss atMs 会越过合成 EOF(实测 rc=0 无输出 → INTERNAL)。
    // 段实际时长经 ffprobe 探测(失败回落名义,映射退化但不出错)。
    let xfade_chain = crate::steps::is_xfade_chain(&plan.video_clips);
    let seg_durs: Vec<Option<f64>> = seg_files
        .iter()
        .map(|f| crate::ffprobe_duration_sec(f).ok())
        .collect();
    let at_compose = timeline_to_compose_ms(&plan.video_clips, xfade_chain, at_q, &seg_durs);

    let rel = idx.record_with_ext(
        "frame",
        &key,
        cache::frame_spec_proxy(
            &fp.cache_key(),
            at_q,
            canvas,
            fmt.as_str(),
            combined.as_deref(),
            use_proxy,
        ),
        now,
        fmt.ext(),
    );
    let out = cache_root.join(&rel);
    // ASS 副本写 tmp(cwd = 缓存根,相对路径喂滤镜);用完即清。
    // 滤镜参数内路径必须正斜杠(Windows 反斜杠会被 filtergraph 转义规则吞掉)
    let write_ass = |tag: &str, payload: &[u8]| -> Result<String, String> {
        let rel = cache::tmp_rel(&format!("ass-{tag}-{key}.ass"));
        cutforge_io::atomic::atomic_write(&cache_root.join(&rel), payload)
            .map_err(|e| e.to_string())?;
        Ok(rel.to_string_lossy().replace('\\', "/"))
    };
    let mut ass_rels: Vec<String> = Vec::new();
    if let Some(user) = &ass_bytes {
        ass_rels.push(write_ass("user-frame", user)?);
    }
    if let Some(text) = &text_bytes {
        ass_rels.push(write_ass("text-frame", text)?);
    }
    let args = frame_extract_args(&base_video, at_compose, fmt, &ass_rels, &out);
    let r = crate::run_ff_in(&cache_root, "ffmpeg", &crate::strs(&args));
    for local in &ass_rels {
        let _ = cutforge_io::atomic::remove(&cache_root.join(local));
    }
    r?;
    if !out.is_file() {
        return Err(format!("INTERNAL: 抽帧未产出文件({})", out.display()));
    }
    idx.set_size("frame", &key, crate::file_size(&out));
    idx.save(&cache_root)?;
    Ok(FrameOutcome {
        output: out,
        at_ms: at_q,
        cached: false,
        key,
    })
}

/// 单帧完成事件(stdout JSON 行;CLI 面唯一进度输出——单帧同步执行,一帧一报)。
pub fn frame_done_event(o: &FrameOutcome) -> Value {
    json!({
        "done": true,
        "mode": "frame",
        "frame": o.output.to_string_lossy(),
        "atMs": o.at_ms,
        "cached": o.cached,
        "key": o.key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- 纯函数面 ----

    #[test]
    fn quantize_floors_to_100ms_grid() {
        assert_eq!(quantize_ms(0), 0);
        assert_eq!(quantize_ms(99), 0);
        assert_eq!(quantize_ms(100), 100);
        assert_eq!(quantize_ms(1234), 1200);
        assert_eq!(quantize_ms(999_999), 999_900);
    }

    #[test]
    fn frame_format_parses_and_names() {
        assert_eq!(FrameFormat::parse("png"), Some(FrameFormat::Png));
        assert_eq!(FrameFormat::parse("JPEG"), Some(FrameFormat::Jpeg));
        assert_eq!(FrameFormat::parse("jpg"), Some(FrameFormat::Jpeg));
        assert_eq!(FrameFormat::parse("webp"), None);
        assert_eq!(FrameFormat::Png.ext(), ".png");
        assert_eq!(FrameFormat::Jpeg.ext(), ".jpg");
    }

    fn mk_clip(start: u64, dur: u64) -> Clip {
        serde_json::from_value(serde_json::json!({
            "id": format!("V1-{start:03}"), "src": "a.mp4",
            "startMs": start, "durationMs": dur
        }))
        .unwrap()
    }

    #[test]
    fn timeline_to_compose_covers_gap_transition_and_tail() {
        // 时间线:A(0-5000) 空隙(5000-12000) B(12000-20000)
        let clips = [mk_clip(0, 5000), mk_clip(12_000, 8000)];
        let segs = [Some(5.0), Some(8.0)];

        // concat(无转场):B 起点 = 前段实际时长累计 5s;段内直加
        assert_eq!(timeline_to_compose_ms(&clips, false, 0, &segs), 0);
        assert_eq!(timeline_to_compose_ms(&clips, false, 4_999, &segs), 4_999);
        // 空隙(6s)→ 定格前段末帧(5000-40)
        assert_eq!(timeline_to_compose_ms(&clips, false, 6_000, &segs), 4_960);
        // B 段:合成 5000 + (13000-12000)
        assert_eq!(timeline_to_compose_ms(&clips, false, 13_000, &segs), 6_000);
        // 越尾(25s)→ 钳合成末 8000+5000-40
        assert_eq!(timeline_to_compose_ms(&clips, false, 25_000, &segs), 12_960);

        // xfade(名义累计):B 合成起点 = A 名义 5000(与 offset 口径同源)
        assert_eq!(timeline_to_compose_ms(&clips, true, 13_000, &segs), 6_000);
        // 段时长探测缺失回落名义(与 segs 一致时不影响)
        assert_eq!(
            timeline_to_compose_ms(&clips, false, 13_000, &[None, None]),
            6_000
        );
    }

    #[test]
    fn timeline_to_compose_single_clip_and_head() {
        let clips = [mk_clip(2_000, 3_000)];
        // 早于首段 → 0
        assert_eq!(timeline_to_compose_ms(&clips, false, 0, &[Some(3.0)]), 0);
        assert_eq!(
            timeline_to_compose_ms(&clips, false, 3_500, &[Some(3.0)]),
            1_500
        );
        // 越尾钳 3000-40
        assert_eq!(
            timeline_to_compose_ms(&clips, false, 9_999, &[Some(3.0)]),
            2_960
        );
    }

    #[test]
    fn extract_args_seek_exact_and_burn_with_pts_restore() {
        let base = Path::new("/c/compose/x.mp4");
        let out = Path::new("/c/frame/k.png");
        // at=0:无 setpts;无 ass:无 -vf;png:无 -q:v
        let none: Vec<String> = Vec::new();
        let a0 = frame_extract_args(base, 0, FrameFormat::Png, &none, out);
        assert_eq!(
            a0,
            [
                "-y",
                "-v",
                "error",
                "-ss",
                "0.000",
                "-i",
                "/c/compose/x.mp4",
                "-frames:v",
                "1",
                "-f",
                "image2",
                "/c/frame/k.png"
            ]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
        );
        // at=1500:输入侧 -ss 1.5 + setpts 平移回时间线时刻
        let a1 = frame_extract_args(base, 1500, FrameFormat::Png, &none, out);
        assert_eq!(&a1[3..5], ["-ss", "1.500"]);
        assert_eq!(a1[8], "setpts=PTS+1.500/TB", "-vf 位于 -i 与输入路径之后");
        // ass:subtitles 追加在 setpts 之后(滤镜按序消费平移后的 PTS);jpeg 附带 -q:v 2
        let a2 = frame_extract_args(
            base,
            1500,
            FrameFormat::Jpeg,
            &["tmp/ass-frame-k.ass".into()],
            out,
        );
        assert_eq!(a2[8], "setpts=PTS+1.500/TB,subtitles=tmp/ass-frame-k.ass");
        assert!(
            a2.windows(2).any(|w| w[0] == "-q:v" && w[1] == "2"),
            "jpeg 必须带质量档"
        );
        // ass 且 at=0:只烧字幕,无 setpts
        let a3 = frame_extract_args(base, 0, FrameFormat::Png, &["tmp/a.ass".into()], out);
        assert_eq!(a3[8], "subtitles=tmp/a.ass");
        assert!(a3.last().unwrap().ends_with("k.png"), "输出路径恒为末参");
    }

    #[test]
    fn frame_done_event_carries_frame_fields() {
        let o = FrameOutcome {
            output: PathBuf::from("/w/.cutforge/render-cache/frame/k.png"),
            at_ms: 1200,
            cached: true,
            key: "abcd".into(),
        };
        let v = frame_done_event(&o);
        assert_eq!(v["done"], json!(true));
        assert_eq!(v["mode"], json!("frame"));
        assert_eq!(v["cached"], json!(true));
        assert_eq!(v["atMs"], json!(1200));
        assert!(v["frame"].as_str().unwrap().ends_with("k.png"));
    }

    // ---- 真实渲染面(ffmpeg 缺失即失败,与 render_matrix 同口径) ----

    fn ffmpeg_ok() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// 最小工程夹具:单轨单段 3s testsrc2 + 合法 project.json(05_时间线工程)。
    fn fixture(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cf-frame-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("05_时间线工程")).unwrap();
        let ff = |args: &[&str]| {
            let out = std::process::Command::new("ffmpeg")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "ffmpeg 失败: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        ff(&[
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=3",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "voice.mp4",
        ]);
        cutforge_io::atomic::atomic_write(
            &dir.join("05_时间线工程/project.json"),
            serde_json::to_string_pretty(&json!({
                "version": 1, "schemaVersion": "2.0.0", "slug": "frame-fixture", "fps": 30,
                "canvas": {"width": 1080, "height": 1920},
                "tracks": [{"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 3000, "role": "voice"},
                ]}],
            })).unwrap().as_bytes(),
        ).unwrap();
        dir
    }

    fn load_project(dir: &Path) -> Project {
        let text = std::fs::read_to_string(cutforge_io::paths::project_path(dir)).unwrap();
        let mut v: Value = serde_json::from_str(&text).unwrap();
        if let Some(obj) = v.as_object_mut() {
            obj.remove("_meta");
        }
        cutforge_core::model::migrate_from_value(&v).unwrap()
    }

    #[test]
    fn render_frame_real_png_then_cache_hit() {
        assert!(
            ffmpeg_ok(),
            "ffmpeg 必须存在(与 render_matrix 同口径:缺失即失败)"
        );
        let dir = fixture("png");
        let project = load_project(&dir);
        let o1 = render_frame(&project, &dir, None, 1555, FrameFormat::Png).expect("首渲必成");
        assert_eq!(o1.at_ms, 1500, "atMs 按 100ms 量化");
        assert!(!o1.cached);
        let bytes = std::fs::read(&o1.output).expect("产物必须存在");
        assert!(bytes.len() > 100, "PNG 不应为空壳");
        assert_eq!(
            &bytes[..8],
            b"\x89PNG\r\n\x1a\n",
            "产物必须是合法 PNG(魔数)"
        );
        let o2 = render_frame(&project, &dir, None, 1555, FrameFormat::Png).expect("二渲必成");
        assert!(o2.cached, "同指纹同时间点必须命中 frame 缓存");
        assert_eq!(o1.output, o2.output);
        // 改一笔工程(project.json 字节变)→ 指纹变 → 缓存必 miss
        let mut v = serde_json::json!(project);
        v["slug"] = json!("frame-fixture-edited");
        cutforge_io::atomic::atomic_write(
            &dir.join("05_时间线工程/project.json"),
            serde_json::to_string_pretty(&v).unwrap().as_bytes(),
        )
        .unwrap();
        let project2 = load_project(&dir);
        let o3 = render_frame(&project2, &dir, None, 1555, FrameFormat::Png).expect("改后重渲必成");
        assert!(!o3.cached, "改一笔即 miss,不得复用陈旧帧");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn render_frame_real_jpeg_magic() {
        assert!(ffmpeg_ok(), "ffmpeg 必须存在");
        let dir = fixture("jpeg");
        let project = load_project(&dir);
        let o = render_frame(&project, &dir, None, 500, FrameFormat::Jpeg).expect("jpeg 渲必成");
        let bytes = std::fs::read(&o.output).unwrap();
        assert!(
            bytes.starts_with(&[0xFF, 0xD8]),
            "产物必须是合法 JPEG(SOI 魔数)"
        );
        assert!(o.output.to_string_lossy().ends_with(".jpg"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn render_frame_real_ass_burn_alters_pixels() {
        assert!(ffmpeg_ok(), "ffmpeg 必须存在");
        let dir = fixture("burn");
        let project = load_project(&dir);
        // 最小合法 ASS:0.5s–2.5s 有字幕
        let ass = dir.join("subs.ass");
        cutforge_io::atomic::atomic_write(&ass, concat!(
            "[Script Info]\nScriptType: v4.00+\nPlayResX: 1080\nPlayResY: 1920\n\n",
            "[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n",
            "Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,60,1\n\n",
            "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
            "Dialogue: 0,0:00:00.50,0:00:02.50,Default,,0,0,0,,帧上字幕\n",
        ).as_bytes()).unwrap();
        let with = render_frame(&project, &dir, Some(&ass), 1500, FrameFormat::Png)
            .expect("带 ass 渲必成");
        let without =
            render_frame(&project, &dir, None, 1500, FrameFormat::Png).expect("无 ass 渲必成");
        assert_ne!(with.key, without.key, "ASS 字节入键:带/不带必须不同键");
        let b_with = std::fs::read(&with.output).unwrap();
        let b_without = std::fs::read(&without.output).unwrap();
        assert_eq!(&b_with[..8], b"\x89PNG\r\n\x1a\n");
        assert_ne!(b_with, b_without, "烧录必须真实改变像素(否则烧录链是幻觉)");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn render_frame_guards_without_ffmpeg_paths() {
        let dir = fixture("guard");
        // 用例 1:无视频片段(纯文本轨)→ PRECONDITION(不触 ffmpeg)
        let mut v: Value = serde_json::from_str(
            &std::fs::read_to_string(cutforge_io::paths::project_path(&dir)).unwrap(),
        )
        .unwrap();
        v["tracks"] = json!([{"id": "T1", "kind": "text", "clips": [
            {"id": "T1-001", "startMs": 0, "durationMs": 1000, "text": "x"}]}]);
        cutforge_io::atomic::atomic_write(
            &dir.join("05_时间线工程/project.json"),
            serde_json::to_string_pretty(&v).unwrap().as_bytes(),
        )
        .unwrap();
        let project = load_project(&dir);
        let err = render_frame(&project, &dir, None, 500, FrameFormat::Png).unwrap_err();
        assert!(err.starts_with("PRECONDITION:"), "{err}");
        // 用例 2:atMs 超出视频时间线 → PRECONDITION(守卫先于 ffmpeg,不触管线)
        let mut v2: Value = serde_json::from_str(
            &std::fs::read_to_string(cutforge_io::paths::project_path(&dir)).unwrap(),
        )
        .unwrap();
        v2["tracks"] = json!([{"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 3000, "role": "voice"}]}]);
        cutforge_io::atomic::atomic_write(
            &dir.join("05_时间线工程/project.json"),
            serde_json::to_string_pretty(&v2).unwrap().as_bytes(),
        )
        .unwrap();
        let project3 = load_project(&dir);
        let err2 = render_frame(&project3, &dir, None, 999_999, FrameFormat::Png).unwrap_err();
        assert!(err2.starts_with("PRECONDITION:"), "{err2}");
        assert!(err2.contains("3000"), "报错须给出时间线时长: {err2}");
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- 内容末端口径(I1:源耗尽定格计满,只拦整窗无源) ----

    /// 纯函数面(I1 口径,与 segment::source_clamped_clip 的渲染面同源):
    /// 源充足/探测不可用 = 未折算(行为不变);源部分耗尽 = 定格计满(段步已按
    /// 末帧定格补长,定格区可抽帧);源窗整体越出素材(预算 0)→ 0,守卫拦下。
    #[test]
    fn content_len_freezes_source_exhausted_tail() {
        let mk = |v: Value| -> Clip { serde_json::from_value(v).unwrap() };
        // 源充足(含曲线):与投影一致
        let c = mk(json!({"id": "V1-001", "startMs": 0, "durationMs": 3000,
                          "speedCurve": [{"atMs": 0, "speed": 2.0}]}));
        assert_eq!(clip_content_len_ms(&c, Some(10_000)), 3000);
        assert_eq!(clip_content_len_ms(&c, None), 3000, "探测不可用退回未折算");
        // 源部分耗尽:常速 2.0 需 6000ms 源,只有 4000ms → 播放域 2000ms 后定格
        // 补满到标称(渲染面段产出满 3000ms,定格区真实可见)
        let c2 = mk(json!({"id": "V1-001", "startMs": 0, "durationMs": 3000, "speed": 2.0}));
        assert_eq!(clip_content_len_ms(&c2, Some(4000)), 3000);
        // 曲线中段耗尽(需 3000ms 源,可用 2500)→ 同样定格计满
        let c3 = mk(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000,
                           "speedCurve": [{"atMs": 0, "speed": 1.0}, {"atMs": 1000, "speed": 1.0},
                                          {"atMs": 2000, "speed": 3.0}]}));
        assert_eq!(clip_content_len_ms(&c3, Some(2500)), 2000);
        // 定格组合:源只剩 900(播放域 600ms)→ 定格计满 3000
        let c4 = mk(
            json!({"id": "V1-001", "startMs": 0, "durationMs": 3000, "freezeMs": 1200,
                           "speedCurve": [{"atMs": 0, "speed": 1.0}, {"atMs": 2000, "speed": 2.0}]}),
        );
        assert_eq!(clip_content_len_ms(&c4, Some(100_000)), 3000);
        assert_eq!(clip_content_len_ms(&c4, Some(900)), 3000);
        // 源窗整体越出素材:sourceIn 5000 > 素材 1000 → 预算 0 → 无帧可产
        let c5 = mk(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000, "sourceInMs": 5000}));
        assert_eq!(clip_content_len_ms(&c5, Some(1000)), 0);
        // sourceIn 计入预算(调用方传素材总时长)
        let c6 = mk(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000, "sourceInMs": 500}));
        assert_eq!(
            clip_content_len_ms(&c6, Some(1000)),
            2000,
            "预算 500ms > 0 → 定格计满"
        );
        assert_eq!(
            clip_content_len_ms(&c6, Some(500)),
            0,
            "预算恰 = srcIn → 无源可读"
        );
    }

    /// 真实渲染面(I1 口径):曲线变速源耗尽(播放域 1.5s 后无源)→ 渲染面末帧
    /// 定格补满到标称 3s,超耗尽点的 atMs **出帧**(修复前报 PRECONDITION/INTERNAL);
    /// 源窗整体越出素材(sourceIn 超素材长)才 PRECONDITION。
    #[test]
    fn render_frame_source_exhaustion_freezes_tail() {
        assert!(ffmpeg_ok(), "ffmpeg 必须存在(与 render_matrix 同口径)");
        assert!(
            cutforge_io::probe::ffprobe_available(),
            "ffprobe 必须存在(内容末端折算依赖)"
        );
        let dir = fixture("exhaust");
        // 夹具源 3s;片段标称 3s + 曲线 [0:1.0, 3000:3.0](区间均值 2.0,需 6s 源)
        // → 源 3s 在播放域 1.5s 处耗尽 → 段步末帧定格补满(成片仍 3s)
        let v: Value = serde_json::from_str(
            &std::fs::read_to_string(cutforge_io::paths::project_path(&dir)).unwrap(),
        )
        .unwrap();
        let mut v2 = v.clone();
        v2["tracks"] = json!([{"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 3000, "role": "voice",
             "speedCurve": [{"atMs": 0, "speed": 1.0}, {"atMs": 3000, "speed": 3.0}]}]}]);
        cutforge_io::atomic::atomic_write(
            &dir.join("05_时间线工程/project.json"),
            serde_json::to_string_pretty(&v2).unwrap().as_bytes(),
        )
        .unwrap();
        let project = load_project(&dir);
        // 源耗尽点前:正常出帧
        let ok = render_frame(&project, &dir, None, 500, FrameFormat::Png).expect("源耗尽点前必成");
        assert_eq!(
            &std::fs::read(&ok.output).unwrap()[..8],
            b"\x89PNG\r\n\x1a\n"
        );
        // 超源耗尽点(定格区)→ 末帧定格出帧,不再 PRECONDITION/INTERNAL
        let frozen =
            render_frame(&project, &dir, None, 2000, FrameFormat::Png).expect("定格区必出帧");
        assert_eq!(
            &std::fs::read(&frozen.output).unwrap()[..8],
            b"\x89PNG\r\n\x1a\n"
        );
        // 定格区画面 = 耗尽点末帧(与末端同画面;有损编码对静止克隆帧有逐帧
        // 量化噪声,断言按解码像素差异比例近似相同,非字节相等)
        let at_end =
            render_frame(&project, &dir, None, 2900, FrameFormat::Png).expect("标称末端内必成");
        let px = |p: &std::path::Path| -> Vec<u8> {
            let out = std::process::Command::new("ffmpeg")
                .args([
                    "-i",
                    p.to_str().unwrap(),
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "rgb24",
                    "-",
                ])
                .output()
                .expect("ffmpeg 必须存在");
            out.stdout
        };
        let fa = px(&frozen.output);
        let fb = px(&at_end.output);
        assert_eq!(fa.len(), fb.len(), "同画幅像素数必须一致");
        let diff = fa.iter().zip(fb.iter()).filter(|(x, y)| x != y).count();
        assert!(
            diff * 100 < fa.len(),
            "定格区两帧应同为末帧画面(允许编码噪声),差异字节 {diff}"
        );
        // 源窗整体越出素材(sourceIn 5000 > 素材 3s)→ PRECONDITION(整窗无源)
        let mut v3 = v.clone();
        v3["tracks"] = json!([{"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 3000,
             "sourceInMs": 5000}]}]);
        cutforge_io::atomic::atomic_write(
            &dir.join("05_时间线工程/project.json"),
            serde_json::to_string_pretty(&v3).unwrap().as_bytes(),
        )
        .unwrap();
        let project3 = load_project(&dir);
        let err = render_frame(&project3, &dir, None, 100, FrameFormat::Png).unwrap_err();
        assert!(
            err.starts_with("PRECONDITION:"),
            "整窗无源必须 PRECONDITION 而非 {err}"
        );
        assert!(err.contains("超出真实内容末端"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
