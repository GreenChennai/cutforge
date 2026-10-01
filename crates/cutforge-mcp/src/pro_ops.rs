// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 册五 T5.4/T5.5 专业编辑与互操作工具后端:
//! - `compound_create` / `compound_unbind`:复合片段打包/解包(单 Op 原子,
//!   语义裁决全在内核 Command,经 Workspace::apply 唯一写入口);
//! - `multicam_sync`:同源多角度素材音频包络互相关对齐(纯计算,免开工作区,
//!   启发式诚实标注 degraded/confidence);
//! - `multicam_cut`:同步偏移 + 切换点 → 展开为普通片段序列(单 Op,ADR-0019
//!   "多机位 = 序列不是实体");
//! - `scene_detect`:抽帧差分剪切点检测(启发式诚实标注)+ 可选自动切段
//!   (Command::TrackSplitAt 单 Op);
//! - `otio_export`:工程 → OTIO 最小子集 JSON / EDL(CMX3600)(派生物产物,
//!   不产 Op 不改 IR;cutforge_core::interop 单一实现);
//! - `otio_import`:OTIO JSON → 新工程(从零起步,project_new 同类免锁创建;
//!   子集外元素 WARN 留痕不静默丢)。
//!
//! 本模块只做参数抽取、路径校验与产物落盘,禁止旁路写工程文件。

use crate::dispatch::resolve_within_root;
use crate::media_tools::{ff_bin, ffmpeg_available, PCM_RATE};
use crate::registry::envelope;
use cutforge_core::command::Command;
use cutforge_core::engine::ApplyOpts;
use cutforge_core::model::{Clip, TrackKind};
use cutforge_render::analyze::{frame_diffs, pcm_lag, pick_cuts};
use cutforge_core::oplog::Actor;
use cutforge_io::Workspace;
use serde_json::{json, Value};
use std::path::Path;

/// 成功回执追加 data 字段(失败原样透传)。
fn with_data(env: Value, f: impl FnOnce(&mut Value)) -> Value {
    let mut env = env;
    if env["ok"] == json!(true) {
        f(&mut env);
    }
    env
}

// ---------------- 复合片段(T5.4;语义在 Command,此处只接线) ----------------

/// compound_create:选中多片段打包为复合片段(单 Op;守卫在内核:
/// ≥2 片段/全视频轨/不重叠/首尾相接/不含复合)。
pub fn compound_create_tool(ws: &mut Workspace, args: &Value, actor: &Actor, opts: ApplyOpts) -> Value {
    let Some(clip_ids) = args["clipIds"].as_array().map(|a| {
        a.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>()
    }) else {
        return envelope(false, "PRECONDITION_FAILED", "缺 clipIds(选中片段 id 数组)", json!({}));
    };
    let Some(track_id) = args["toTrack"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 toTrack(目标轨)", json!({}));
    };
    let Some(start_ms) = args["startMs"].as_u64() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 startMs(复合片段落点毫秒)", json!({}));
    };
    let env = crate::dispatch::finish_apply(ws.apply(
        Command::CompoundCreate { clip_ids: clip_ids.clone(), to_track: track_id.into(), start_ms, request_id: opts.request_id.clone() },
        actor.clone(),
        opts,
    ));
    with_data(env, |d| {
        // 打包成功时回带新壳 id(FE 直接选中;壳 = 目标轨 next id,由投影核对)
        if let Some(ti) = ws.project().find_track(track_id)
            && let Some(c) = ws.project().tracks[ti].clips.iter().find(|c| c.compound.is_some() && c.start_ms == start_ms) {
                d["data"]["clipId"] = json!(c.id);
                d["data"]["durationMs"] = json!(c.duration_ms);
                d["data"]["innerClips"] = json!(c.compound.as_ref().map(|s| s.clips.len()).unwrap_or(0));
            }
    })
}

/// compound_unbind:复合片段解包还原(单 Op;id 重分配,时间域平移回主时间线)。
pub fn compound_unbind_tool(ws: &mut Workspace, args: &Value, actor: &Actor, opts: ApplyOpts) -> Value {
    let Some(clip_id) = args["clipId"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({}));
    };
    let restored = ws.project().find_clip(clip_id).and_then(|(ti, ci)| {
        ws.project().tracks[ti].clips[ci].compound.as_ref().map(|c| c.clips.len())
    });
    let env = crate::dispatch::finish_apply(ws.apply(
        Command::CompoundUnbind { clip_id: clip_id.into() },
        actor.clone(),
        opts,
    ));
    with_data(env, |d| {
        d["data"]["unbound"] = json!(restored.unwrap_or(0));
    })
}

// ---------------- 多机位(T5.4;ADR-0019 展开方案) ----------------

/// 解码单声道 22050Hz s16le PCM(与 media_tools 同一口径;失败 → Err)。
fn decode_mono_pcm(root: &Path, src: &str) -> Result<Vec<i16>, String> {
    let abs = resolve_within_root(root, src).map_err(|m| format!("路径不合法({src}): {m}"))?;
    let out = std::process::Command::new(ff_bin())
        .args(["-v", "error", "-i", &abs.to_string_lossy(), "-vn", "-ac", "1", "-ar", &PCM_RATE.to_string(), "-f", "s16le", "-"])
        .output()
        .map_err(|e| format!("ffmpeg 启动失败: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "PCM 解码失败({src}): {}",
            String::from_utf8_lossy(&out.stderr).chars().take(160).collect::<String>()
        ));
    }
    Ok(out.stdout.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect())
}

/// multicam_sync:同源多角度素材数组 → 音频包络互相关对齐 → 各素材偏移 ms
/// (angles[0] = 基准,偏移恒 0;offset[i] = 角度 i 的源时间轴相对基准的滞后——
/// 在时间线 T 处读角度 i 内容的 sourceInMs = T + offset)。启发式诚实标注。
pub fn multicam_sync_tool(root: &Path, args: &Value) -> Value {
    let Some(angles) = args["angles"].as_array().map(|a| {
        a.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>()
    }) else {
        return envelope(false, "PRECONDITION_FAILED", "缺 angles(同源多角度素材相对路径数组)", json!({}));
    };
    if angles.len() < 2 {
        return envelope(false, "PRECONDITION_FAILED",
            &format!("至少 2 个角度(收到 {});angles[0] 为基准", angles.len()), json!({}));
    }
    let window_ms = args["windowMs"].as_u64().unwrap_or(5000).clamp(200, 30_000);
    if !ffmpeg_available() {
        return envelope(false, "DEP_MISSING", "ffmpeg 不可用(安装 ffmpeg 或设 CUTFORGE_FFMPEG)", json!({}));
    }
    let mut pcms: Vec<Vec<i16>> = Vec::with_capacity(angles.len());
    let mut durations: Vec<u64> = Vec::with_capacity(angles.len());
    for src in &angles {
        match decode_mono_pcm(root, src) {
            Ok(pcm) => {
                durations.push((pcm.len() as u64) * 1000 / PCM_RATE as u64);
                pcms.push(pcm);
            }
            Err(m) => return envelope(false, "DEP_MISSING", &m, json!({})),
        }
    }
    let mut offsets: Vec<Value> = vec![json!({"index": 0, "src": angles[0], "offsetMs": 0, "confidence": 1.0})];
    let mut min_conf = 1.0f64;
    for i in 1..angles.len() {
        let (offset, score) = pcm_lag(&pcms[0], &pcms[i], PCM_RATE, window_ms);
        min_conf = min_conf.min(score);
        offsets.push(json!({
            "index": i, "src": angles[i],
            "offsetMs": offset,
            "confidence": (score * 100.0).round() / 100.0,
        }));
    }
    envelope(true, "OK", "多机位同步分析完成(启发式;置信度如实)", json!({
        "engine": "pcm-xcorr",
        "degraded": true,
        "reference": angles[0],
        "windowMs": window_ms,
        "angles": offsets,
        "confidence": (min_conf * 100.0).round() / 100.0,
        "durationsMs": durations,
        "hint": "纯计算不落盘不产 Op;offset 语义 = 角度源时间轴相对基准的滞后毫秒(读角度 i 内容于时间线 T:sourceInMs = T + offsetMs);落序列用 multicam_cut",
    }))
}

/// multicam_cut:切换点列表 → 展开为普通片段序列(单 Op 原子;ADR-0019:
/// 多机位 = 序列不是实体)。angles[i].offsetMs 接 multicam_sync 输出;
/// switches[i] = {tMs(相对 startMs), angle(角度下标)};末段补足到 durationMs。
pub fn multicam_cut_tool(ws: &mut Workspace, args: &Value, actor: &Actor, opts: ApplyOpts) -> Value {
    let (Some(track_id), Some(start_ms), Some(duration_ms)) =
        (args["trackId"].as_str(), args["startMs"].as_u64(), args["durationMs"].as_u64())
    else {
        return envelope(false, "PRECONDITION_FAILED", "缺 trackId/startMs/durationMs", json!({}));
    };
    let Some(ti) = ws.project().find_track(track_id) else {
        return envelope(false, "PRECONDITION_FAILED", &format!("track 不存在: {track_id}"), json!({}));
    };
    if ws.project().tracks[ti].kind != TrackKind::Video {
        return envelope(false, "GUARD_FAILED", "多机位序列只能落视频轨", json!({}));
    }
    let Some(angles) = args["angles"].as_array() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 angles([{src, offsetMs}])", json!({}));
    };
    if angles.is_empty() {
        return envelope(false, "PRECONDITION_FAILED", "angles 不得为空", json!({}));
    }
    let mut angle_srcs: Vec<String> = Vec::with_capacity(angles.len());
    let mut angle_offsets: Vec<i64> = Vec::with_capacity(angles.len());
    for (i, a) in angles.iter().enumerate() {
        let Some(src) = a["src"].as_str() else {
            return envelope(false, "PRECONDITION_FAILED", &format!("angles[{i}].src 缺失"), json!({}));
        };
        if let Err(m) = resolve_within_root(ws.root(), src) {
            return envelope(false, "PRECONDITION_FAILED", &format!("素材路径不合法({src}): {m}"), json!({}));
        }
        let off = a["offsetMs"].as_i64().unwrap_or(0);
        if off < 0 {
            return envelope(false, "PRECONDITION_FAILED",
                &format!("angles[{i}].offsetMs 须 ≥0(同步偏移为滞后语义;负值请以基准重排)"), json!({}));
        }
        angle_srcs.push(src.to_string());
        angle_offsets.push(off);
    }
    let Some(switches) = args["switches"].as_array() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 switches([{tMs, angle}])", json!({}));
    };
    if switches.is_empty() {
        return envelope(false, "PRECONDITION_FAILED", "switches 不得为空(至少一个起点切点 tMs=0)", json!({}));
    }
    // 切点解析:升序、严格递增、落在 [0, durationMs);angle 下标合法
    let mut pts: Vec<(u64, usize)> = Vec::with_capacity(switches.len());
    let mut prev = None;
    for (i, s) in switches.iter().enumerate() {
        let (Some(t), Some(a)) = (s["tMs"].as_u64(), s["angle"].as_u64()) else {
            return envelope(false, "PRECONDITION_FAILED", &format!("switches[{i}] 缺 tMs/angle"), json!({}));
        };
        if t >= duration_ms {
            return envelope(false, "PRECONDITION_FAILED",
                &format!("switches[{i}].tMs={t} 越出序列长 {duration_ms}ms"), json!({}));
        }
        let a = a as usize;
        if a >= angle_srcs.len() {
            return envelope(false, "PRECONDITION_FAILED",
                &format!("switches[{i}].angle={a} 越出角度数 {}", angle_srcs.len()), json!({}));
        }
        if let Some(p) = prev && t <= p {
            return envelope(false, "PRECONDITION_FAILED",
                &format!("switches 必须按 tMs 严格递增({p} → {t})"), json!({}));
        }
        prev = Some(t);
        pts.push((t, a));
    }
    if pts[0].0 != 0 {
        return envelope(false, "PRECONDITION_FAILED",
            "switches[0].tMs 必须为 0(序列起点即第一个切点)", json!({}));
    }
    // 展开为普通片段:相邻切点成段;sourceInMs = offset[angle] + 段内偏移
    let mut probe = ws.project().tracks[ti].clone();
    let mut clips: Vec<Clip> = Vec::with_capacity(pts.len());
    for (k, (t, a)) in pts.iter().enumerate() {
        let seg_end = pts.get(k + 1).map(|(nt, _)| *nt).unwrap_or(duration_ms);
        let clip_id = cutforge_core::model::Project::next_clip_id(&probe);
        let c: Clip = match serde_json::from_value(json!({
            "id": clip_id, "src": angle_srcs[*a],
            "startMs": start_ms + t, "durationMs": seg_end - t,
            "sourceInMs": (angle_offsets[*a].max(0) as u64) + t,
        })) {
            Ok(c) => c,
            Err(e) => return envelope(false, "SCHEMA_INVALID", &e.to_string(), json!({})),
        };
        probe.clips.push(c.clone());
        clips.push(c);
    }
    let inserted = clips.len();
    let first_id = clips[0].id.clone();
    let env = crate::dispatch::finish_apply(ws.apply(
        Command::ClipsInsert { to_track: track_id.into(), clips, request_id: opts.request_id.clone() },
        actor.clone(),
        opts,
    ));
    with_data(env, |d| {
        d["data"]["segments"] = json!(inserted);
        d["data"]["firstClipId"] = json!(first_id);
        d["data"]["angles"] = json!(angle_srcs.len());
    })
}

// ---------------- 场景剪切检测(T5.4;启发式诚实标注) ----------------

/// scene_detect:抽帧差分剪切点检测(可选 autoSplit.trackId 自动切段,单 Op)。
pub fn scene_detect_tool(ws: &mut Workspace, root: &Path, args: &Value, actor: &Actor, opts: ApplyOpts) -> Value {
    let Some(src) = args["src"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 src(工程内相对路径)", json!({}));
    };
    let abs = match resolve_within_root(root, src) {
        Ok(p) => p,
        Err(m) => return envelope(false, "PRECONDITION_FAILED", &format!("路径不合法({src}): {m}"), json!({})),
    };
    let sample_fps = args["sampleFps"].as_f64().unwrap_or(5.0).clamp(1.0, 15.0);
    let sensitivity = args["sensitivity"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0);
    if !ffmpeg_available() {
        return envelope(false, "DEP_MISSING", "ffmpeg 不可用(安装 ffmpeg 或设 CUTFORGE_FFMPEG)", json!({}));
    }
    let vf = format!("fps={sample_fps},scale=64:36,format=gray");
    let out = match std::process::Command::new(ff_bin())
        .args(["-v", "error", "-i", &abs.to_string_lossy(), "-vf", &vf, "-f", "rawvideo", "-"])
        .output()
    {
        Ok(o) => o,
        Err(e) => return envelope(false, "DEP_MISSING", &format!("ffmpeg 启动失败: {e}"), json!({})),
    };
    if !out.status.success() {
        return envelope(false, "DEP_MISSING",
            &format!("抽帧失败: {}", String::from_utf8_lossy(&out.stderr).chars().take(160).collect::<String>()), json!({}));
    }
    const FRAME_LEN: usize = 64 * 36;
    let diffs = frame_diffs(&out.stdout, FRAME_LEN);
    let cuts = pick_cuts(&diffs, sample_fps, sensitivity);
    if cuts.is_empty() {
        return envelope(true, "OK", "未检出剪切点(硬切阈值未触发;可调高 sensitivity)", json!({
            "src": src, "engine": "frame-diff", "degraded": true,
            "sampleFps": sample_fps, "sensitivity": sensitivity,
            "frames": diffs.len() + 1, "cutCount": 0, "cuts": [],
        }));
    }
    let cut_list: Vec<Value> = cuts.iter().map(|(ms, c)| json!({"tMs": ms, "confidence": c})).collect();
    // 可选自动切段(单 Op;切点不在片段内部时内核拒绝)
    if let Some(split) = args.get("autoSplit").filter(|v| !v.is_null()) {
        let Some(track_id) = split["trackId"].as_str() else {
            return envelope(false, "PRECONDITION_FAILED", "autoSplit 缺 trackId", json!({}));
        };
        let points: Vec<u64> = cuts.iter().map(|(ms, _)| *ms).collect();
        let env = crate::dispatch::finish_apply(ws.apply(
            Command::TrackSplitAt { track_id: track_id.into(), t_points: points.clone() },
            actor.clone(),
            opts,
        ));
        return with_data(env, |d| {
            d["data"]["src"] = json!(src);
            d["data"]["engine"] = json!("frame-diff");
            d["data"]["degraded"] = json!(true);
            d["data"]["sampleFps"] = json!(sample_fps);
            d["data"]["sensitivity"] = json!(sensitivity);
            d["data"]["cutCount"] = json!(cuts.len());
            d["data"]["cuts"] = json!(cut_list);
            d["data"]["autoSplit"] = json!({"trackId": track_id, "points": points});
        });
    }
    envelope(true, "OK", "剪切点检测完成(启发式;置信度如实)", json!({
        "src": src, "engine": "frame-diff", "degraded": true,
        "sampleFps": sample_fps, "sensitivity": sensitivity,
        "frames": diffs.len() + 1, "cutCount": cuts.len(), "cuts": cut_list,
        "hint": "纯计算不落盘不产 Op;自动切段传 autoSplit.trackId(单 Op)",
    }))
}

// ---------------- OTIO / EDL 互操作(T5.5;实现在 cutforge_core::interop) ----------------

/// otio_export:工程 → OTIO 最小子集 JSON(format=otio,缺省)或 EDL CMX3600
/// (format=edl)。派生物产物落盘(缺省 06_成片输出/<slug>.<ext>),不产 Op 不改 IR。
pub fn otio_export_tool(ws: &Workspace, root: &Path, args: &Value) -> Value {
    let format = args["format"].as_str().unwrap_or("otio");
    if !matches!(format, "otio" | "edl") {
        return envelope(false, "PRECONDITION_FAILED", &format!("未知 format: {format}(允许 otio|edl)"), json!({}));
    }
    let (payload, ext, warns): (String, &str, Vec<String>) = match format {
        "edl" => (cutforge_core::interop::edl_export(ws.project()), "edl", Vec::new()),
        _ => {
            let (doc, w) = cutforge_core::interop::otio_export(ws.project());
            (serde_json::to_string_pretty(&doc).unwrap_or_default(), "otio", w)
        }
    };
    let out = match args["out"].as_str() {
        Some(o) => {
            if o.split(['/', '\\']).any(|seg| seg == "..") || Path::new(o).is_absolute() {
                return envelope(false, "PRECONDITION_FAILED", &format!("out 必须是工程内相对路径: {o}"), json!({}));
            }
            o.to_string()
        }
        None => format!("{}/{}.{}", cutforge_io::paths::output_dir_name(root), ws.project().slug, ext),
    };
    let abs_out = root.join(&out);
    if let Some(dir) = abs_out.parent()
        && let Err(e) = std::fs::create_dir_all(dir) {
            return envelope(false, "INTERNAL", &format!("导出目录创建失败: {e}"), json!({}));
        }
    if let Err(e) = cutforge_io::atomic::atomic_write(&abs_out, payload.as_bytes()) {
        return envelope(false, "INTERNAL", &format!("导出落盘失败: {e}"), json!({}));
    }
    envelope(true, "OK", &format!("互操作导出完成({format})"), json!({
        "format": format, "out": out, "bytes": payload.len(), "media": out,
        "warnings": warns,
    }))
}

/// otio_import:OTIO JSON → 新工程(从零起步;目标 root 须不存在,project_new
/// 同类免锁创建;子集外元素 WARN 留痕不静默丢)。src = OTIO 文件路径(任意可读)。
pub fn otio_import_tool(root: &Path, args: &Value) -> Value {
    let Some(src) = args["src"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 src(OTIO 文件路径)", json!({}));
    };
    let text = match std::fs::read_to_string(src) {
        Ok(t) => t,
        Err(e) => return envelope(false, "NO_CONFIG", &format!("OTIO 文件不可读({src}): {e}"), json!({})),
    };
    let v: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return envelope(false, "SCHEMA_INVALID", &format!("OTIO JSON 非法: {e}"), json!({})),
    };
    let (project, warns) = match cutforge_core::interop::otio_import(&v) {
        Ok(x) => x,
        Err(errs) => return envelope(false, "SCHEMA_INVALID", &errs.join("; "), json!({"errors": errs})),
    };
    let after = match project.to_validated_value() {
        Ok(v) => v,
        Err(errs) => return envelope(false, "SCHEMA_INVALID", &errs.join("; "), json!({"errors": errs})),
    };
    let project_rel = cutforge_io::paths::PROJECT_REL;
    let abs = root.join(project_rel);
    if abs.exists() {
        return envelope(false, "PRECONDITION_FAILED",
            &format!("工程已存在,拒绝覆盖: {}", abs.display()), json!({}));
    }
    if let Some(dir) = abs.parent()
        && let Err(e) = std::fs::create_dir_all(dir) {
            return envelope(false, "INTERNAL", &format!("工程目录创建失败: {e}"), json!({}));
        }
    let mut buf = serde_json::to_vec_pretty(&after).unwrap_or_default();
    buf.push(b'\n');
    if let Err(e) = cutforge_io::atomic::atomic_write(&abs, &buf) {
        return envelope(false, "INTERNAL", &format!("工程落盘失败: {e}"), json!({}));
    }
    let clips: usize = project.tracks.iter().map(|t| t.clips.len()).sum();
    envelope(true, "OK", "OTIO 导入完成(新工程;子集外 WARN 随回执)", json!({
        "project": abs.to_string_lossy(), "slug": project.slug,
        "tracks": project.tracks.len(), "clips": clips,
        "warnings": warns,
        "hint": "从零起步导入(project_new 同类);打开后可继续常规编辑",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// multicam_cut 切换点解析与展开语义在 dispatch 链测试覆盖
    /// (crates/cutforge-mcp/tests/protocol_conformance.rs::pro_ops_tools_full_chain);
    /// 纯计算核心(lag/差分/切点)单测在 cutforge-render::analyze(单一实现同源)。
    #[test]
    fn pure_functions_are_single_sourced() {
        // re-export 可用性锁(mcp 工具面经 cutforge_render::analyze 消费)
        let _ = pcm_lag;
        let _ = frame_diffs;
        let _ = pick_cuts;
    }
}
