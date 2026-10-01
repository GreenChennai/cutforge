// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 导出矩阵工具后端(册六 T6.3;免开工作区,与 render/render_run 同口径):
//! - `export_preflight`(查询):导出前轻量检查清单——**纯计算/轻探测,不渲全片**:
//!   缺失素材(计划素材源在位性)、黑帧风险(首/末帧亮度,源域抽 32×18 灰度帧)、
//!   静音段(声轨覆盖间隙 + 逐段源 RMS 启发式)、时长(窗口裁剪后)、响度预估
//!   (audio_loudness 复用:最新成片在位则实测,否则 null 不虚标);
//!   启发式面如实打 `heuristic: true` 标注,质量不作硬验收;
//! - `export_all_variants`(编排):多画幅批量 = 一次排队 outputs 全变体任务
//!   (底座 render_variants 已有,本面补编排:逐变体 runId 走既有渲染队列,
//!   父任务聚合状态拉取式查询;action=run|status)。

use crate::media_tools::{ff_bin, ffmpeg_available};
use crate::progress::{build_export_extra, render_progress, render_run_async};
use crate::registry::envelope;
use cutforge_core::model::Project;
use cutforge_render::plan::RenderPlan;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 从盘面加载工程(与 cutforge-render main.rs 同口径:05_时间线工程/project.json
/// 优先、旧布局兼容、_meta 剥出后 migrate)。
fn load_project(root: &Path) -> Result<Project, String> {
    let text = std::fs::read_to_string(cutforge_io::paths::project_path(root))
        .map_err(|e| format!("NO_CONFIG: {e}"))?;
    let mut v: Value = serde_json::from_str(&text).map_err(|e| format!("SCHEMA_INVALID: {e}"))?;
    if let Some(obj) = v.as_object_mut() {
        obj.remove("_meta");
    }
    cutforge_core::model::migrate_from_value(&v).map_err(|errors| format!("SCHEMA_INVALID: {}", errors.join("; ")))
}

fn snap100(v: u64) -> u64 {
    v - v % 100
}

// ---------------- export_preflight(查询;轻探测) ----------------

/// 源域单点亮度均值(0..255;32×18 灰度抽帧,毫秒级)。ffmpeg 缺失/解码失败 → Err。
fn sample_luma(abs: &Path, at_ms: u64) -> Result<f64, String> {
    let out = std::process::Command::new(ff_bin())
        .args([
            "-v", "error",
            "-ss", &format!("{:.3}", at_ms as f64 / 1000.0),
            "-i", &abs.to_string_lossy(),
            "-frames:v", "1",
            "-vf", "scale=32:18",
            "-f", "rawvideo", "-pix_fmt", "gray", "-",
        ])
        .output()
        .map_err(|e| format!("ffmpeg 启动失败: {e}"))?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err(format!(
            "帧解码失败(atMs={at_ms}): {}",
            String::from_utf8_lossy(&out.stderr).chars().take(120).collect::<String>()
        ));
    }
    let sum: u64 = out.stdout.iter().map(|&b| b as u64).sum();
    Ok(sum as f64 / out.stdout.len() as f64)
}

/// 亮度 → 16 级量化(0/16/32/…;抽帧亮度跨 ffmpeg build 有 ±1 级漂移,粗档
/// 量化吸收跨机 golden 漂移;黑帧风险阈值 16 以原始均值判定,量化只进报告)。
fn quant_luma(mean: f64) -> i64 {
    ((mean / 16.0).round() * 16.0) as i64
}

/// 单段源 RMS(0..1;8kHz 单声道 s16le 解码;duration=0 或解码失败 → Err)。
fn seg_rms(abs: &Path, source_in_ms: u64, duration_ms: u64, speed: f64) -> Result<f64, String> {
    if duration_ms == 0 {
        return Err("空段".into());
    }
    let read_s = duration_ms as f64 * speed / 1000.0;
    let out = std::process::Command::new(ff_bin())
        .args([
            "-v", "error",
            "-ss", &format!("{:.3}", source_in_ms as f64 / 1000.0),
            "-t", &format!("{read_s:.3}"),
            "-i", &abs.to_string_lossy(),
            "-vn", "-ac", "1", "-ar", "8000",
            "-f", "s16le", "-",
        ])
        .output()
        .map_err(|e| format!("ffmpeg 启动失败: {e}"))?;
    if !out.status.success() || out.stdout.len() < 2 {
        return Err(format!(
            "PCM 解码失败: {}",
            String::from_utf8_lossy(&out.stderr).chars().take(120).collect::<String>()
        ));
    }
    let acc: f64 = out
        .stdout
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| {
            let s = i16::from_le_bytes(*c) as f64;
            s * s
        })
        .sum();
    Ok((acc / out.stdout.len() as f64 / 2.0).sqrt() / 32768.0)
}

/// 静音段合并(100ms 网格吸附 + 重叠合并;上限 20 段)。
fn merge_ranges(mut rs: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    rs.sort();
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (s, e) in rs {
        let (s, e) = (snap100(s), snap100(e));
        if e <= s {
            continue;
        }
        match out.last_mut() {
            Some((_, pe)) if s <= *pe => *pe = (*pe).max(e),
            _ => out.push((s, e)),
        }
    }
    out.truncate(20);
    out
}

/// export_preflight 工具面:root(+可选 inMs/outMs 窗口、target 响度目标)。
/// ffmpeg 缺失 → DEP_MISSING(黑帧/静音面不可用,时长/缺失素材仍如实报)。
pub fn export_preflight_tool(root: &Path, args: &Value) -> Value {
    let project = match load_project(root) {
        Ok(p) => p,
        Err(e) => {
            let code = if e.starts_with("SCHEMA_INVALID") { "SCHEMA_INVALID" } else { "NO_CONFIG" };
            return envelope(false, code, &e, json!({}));
        }
    };
    let in_ms = args["inMs"].as_u64().unwrap_or(0);
    let out_ms = args["outMs"].as_u64();
    let windowed = cutforge_render::export::window_project(&project, in_ms, out_ms);
    let plan = RenderPlan::build_full(&windowed, root, None, false, Default::default());
    let mut warnings: Vec<String> = Vec::new();

    // ---- 缺失素材(计划素材源在位性;渲染会炸的前置事实) ----
    let mut missing: Vec<String> = Vec::new();
    let mut srcs: Vec<PathBuf> = Vec::new();
    for c in &plan.video_clips {
        if let Some(s) = &c.src {
            srcs.push(root.join(s));
        }
    }
    for s in &plan.audio_segs {
        srcs.push(s.src.clone());
    }
    for o in &plan.overlay_segs {
        srcs.push(o.src.clone());
    }
    for p in srcs {
        if !p.is_file() {
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            missing.push(rel);
        }
    }
    missing.sort();
    missing.dedup();

    // ---- 黑帧风险(首/末视频片段的源域首末帧亮度;<16 视为风险) ----
    let ff_ok = ffmpeg_available();
    let mut black = json!({"firstLuma": Value::Null, "lastLuma": Value::Null, "risk": false});
    if plan.video_clips.is_empty() {
        warnings.push("时间线无视频片段,黑帧面不可判;".into());
    } else if !ff_ok {
        warnings.push("ffmpeg 不可用,黑帧/静音面跳过(安装 ffmpeg 或设 CUTFORGE_FFMPEG);".into());
    } else {
        let first = plan.video_clips.first().expect("非空已判");
        let last = plan.video_clips.last().expect("非空已判");
        let head_at = first.source_in_ms.unwrap_or(0);
        let tail_used = cutforge_render::export::source_advance_ms(last, cutforge_render::plan::clip_play_ms(last));
        let tail_at = last.source_in_ms.unwrap_or(0).saturating_add(tail_used).saturating_sub(50);
        let luma = |clip_src: &Option<String>, at: u64| -> Result<i64, String> {
            let Some(s) = clip_src else { return Err("片段无 src".into()) };
            let abs = root.join(s);
            if !abs.is_file() {
                return Err(format!("素材缺失: {s}"));
            }
            sample_luma(&abs, at).map(quant_luma)
        };
        let first_l = luma(&first.src, head_at);
        let last_l = luma(&last.src, tail_at);
        let mut risk = false;
        match &first_l {
            Ok(v) => black["firstLuma"] = json!(v),
            Err(e) => warnings.push(format!("首帧亮度不可测: {e};")),
        }
        match &last_l {
            Ok(v) => black["lastLuma"] = json!(v),
            Err(e) => warnings.push(format!("末帧亮度不可测: {e};")),
        }
        if let Ok(v) = first_l {
            risk |= v < 16;
        }
        if let Ok(v) = last_l {
            risk |= v < 16;
        }
        black["risk"] = json!(risk);
    }

    // ---- 静音段(声轨覆盖间隙 >200ms + 逐段源 RMS <0.005 的启发式预估;
    //      BGM 在位时静音段被铺底遮蔽,本面只检声轨层,如实标注) ----
    let mut silent: Vec<(u64, u64)> = Vec::new();
    if plan.audio_segs.is_empty() && plan.bgm.is_none() {
        if plan.total_ms > 0 {
            silent.push((0, plan.total_ms));
        }
        warnings.push("无声轨事件且无 BGM:整片静音;".into());
    } else {
        let mut cov: Vec<(u64, u64)> = plan
            .audio_segs
            .iter()
            .map(|s| (s.start_ms, s.start_ms + s.duration_ms))
            .collect();
        cov.sort();
        let mut cursor = 0u64;
        for (s, e) in cov {
            if s > cursor + 200 {
                silent.push((cursor, s));
            }
            cursor = cursor.max(e);
        }
        if cursor + 200 < plan.total_ms {
            silent.push((cursor, plan.total_ms));
        }
        // 逐段源 RMS(段数上限 30:轻探测口径,超出部分不逐段解码)
        const RMS_CAP: usize = 30;
        let mut quiet: Vec<(u64, u64)> = Vec::new();
        for (i, seg) in plan.audio_segs.iter().enumerate() {
            if i >= RMS_CAP {
                warnings.push(format!("声轨事件超 {} 段,其余段不做逐段 RMS(轻探测口径);", RMS_CAP));
                break;
            }
            if !seg.src.is_file() {
                continue;
            }
            if let Ok(r) = seg_rms(&seg.src, seg.source_in_ms, seg.duration_ms, seg.speed)
                && r < 0.005
            {
                quiet.push((seg.start_ms, seg.start_ms + seg.duration_ms));
            }
        }
        silent.extend(quiet);
    }
    let silent_ranges: Vec<Value> = merge_ranges(silent)
        .into_iter()
        .map(|(s, e)| json!({"startMs": s, "endMs": e}))
        .collect();

    // ---- 响度预估(audio_loudness 复用:最新成片在位则实测,否则 null 不虚标) ----
    let mut loudness = Value::Null;
    let finals: Vec<PathBuf> = std::fs::read_dir(&plan.out_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.extension().is_some_and(|x| x == "mp4")
                        && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("final_cutforge_"))
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(newest) = finals.iter().max_by_key(|p| {
        std::fs::metadata(p).and_then(|m| m.modified()).ok().map(|t| {
            t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
        }).unwrap_or(0)
    }) {
        if let Ok(rel) = newest.strip_prefix(root) {
            let mut largs = json!({"root": root.to_string_lossy(), "src": rel.to_string_lossy().replace('\\', "/")});
            if let Some(t) = args["target"].as_f64() {
                largs["target"] = json!(t);
            }
            loudness = crate::grade_tools::audio_loudness_tool(root, &largs);
        }
    } else {
        warnings.push("尚无成片产物,响度预估缺位(导出后用 audio_loudness 实测);".into());
    }

    let mut data = json!({
        "totalMs": plan.total_ms,
        "clips": plan.video_clips.len(),
        "audioEvents": plan.audio_segs.len(),
        "missingAssets": missing,
        "blackFrame": black,
        "silentRanges": silent_ranges,
        "loudness": loudness,
        "heuristic": true,
        "warnings": warnings,
    });
    if in_ms > 0 || out_ms.is_some() {
        data["window"] = json!({"inMs": in_ms, "outMs": out_ms});
    }
    envelope(true, "OK", "导出前检查(轻探测预估面)", data)
}

// ---------------- export_all_variants(编排;父任务聚合) ----------------

/// 父任务(内存态;children = (runId, ratio),状态拉取式聚合)。
struct ParentJob {
    root: PathBuf,
    children: Vec<(String, String)>,
}

fn parents() -> &'static std::sync::Mutex<BTreeMap<String, ParentJob>> {
    static P: OnceLock<std::sync::Mutex<BTreeMap<String, ParentJob>>> = OnceLock::new();
    P.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
}

fn new_parent_id() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::process::id().hash(&mut h);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
    format!("p{:016x}", h.finish())
}

/// export_all_variants 工具面:action=run(缺省)一次排队全部变体(逐变体
/// 走既有渲染队列,runId 独立;进度/取消/重试复用 render_queue 面);
/// action=status 按 parentRunId 聚合子任务状态(all ok → ok;有 fail/canceled →
/// fail;否则 running)。
pub fn export_all_variants_tool(root: &Path, args: &Value) -> Value {
    match args["action"].as_str().unwrap_or("run") {
        "run" => {
            let ratios: Vec<String> = match args["ratios"].as_array() {
                Some(a) if !a.is_empty() => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                _ => match load_project(root).ok().and_then(|p| p.outputs) {
                    Some(o) if !o.is_empty() => o
                        .iter()
                        .map(|r| match r {
                            cutforge_core::model::Ratio::NineBySixteen => "9x16",
                            cutforge_core::model::Ratio::ThreeByFour => "3x4",
                            cutforge_core::model::Ratio::SixteenByNine => "16x9",
                        })
                        .map(String::from)
                        .collect(),
                    _ => vec!["9x16".into(), "16x9".into(), "1x1".into()],
                },
            };
            for r in &ratios {
                if cutforge_render::export::preset_canvas(r).is_none() {
                    return envelope(false, "PRECONDITION_FAILED",
                        &format!("未知画幅预设: {r}(允许 vertical|9x16|horizontal|16x9|square|1x1|3x4|4x5)"), json!({}));
                }
            }
            let ass = crate::progress::existing_rel(root, args["ass"].as_str()).map(String::from);
            let use_proxy = args["useProxy"].as_bool().unwrap_or(false);
            let mut runs: Vec<Value> = Vec::new();
            let mut children: Vec<(String, String)> = Vec::new();
            for r in &ratios {
                // 逐变体入队:preset 参数按变体覆写,其余导出参数共用
                let mut vargs = args.clone();
                vargs["preset"] = json!(r);
                let mut extra: Vec<String> = Vec::new();
                build_export_extra(&vargs, &mut extra);
                let resp = render_run_async(root, ass.as_deref(), use_proxy, extra);
                if resp["ok"] != json!(true) {
                    return resp;
                }
                let run_id = resp["data"]["runId"].as_str().unwrap_or_default().to_string();
                children.push((run_id.clone(), r.clone()));
                runs.push(json!({"ratio": r, "runId": run_id}));
            }
            let parent_id = new_parent_id();
            if let Ok(mut p) = parents().lock() {
                p.insert(parent_id.clone(), ParentJob { root: root.to_path_buf(), children });
            }
            envelope(true, "OK", "多画幅批量导出已入队", json!({
                "parentRunId": parent_id,
                "ratios": ratios,
                "runs": runs,
                "note": "逐变体走渲染队列(runId 独立);聚合状态用 action=status",
            }))
        }
        "status" => {
            let Some(parent_id) = args["parentRunId"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 parentRunId", json!({}));
            };
            let Some(job) = parents().lock().ok().and_then(|p| {
                p.get(parent_id).map(|j| (j.root.clone(), j.children.clone()))
            }) else {
                return envelope(false, "PRECONDITION_FAILED", &format!("未知 parentRunId: {parent_id}"), json!({}));
            };
            let (_root, children) = job;
            let mut runs: Vec<Value> = Vec::new();
            let mut counts: BTreeMap<String, u64> = BTreeMap::new();
            for (run_id, ratio) in &children {
                let state = render_progress(run_id)["data"]["state"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                *counts.entry(state.clone()).or_insert(0) += 1;
                runs.push(json!({"ratio": ratio, "runId": run_id, "state": state}));
            }
            let get = |k: &str| counts.get(k).copied().unwrap_or(0);
            let total = children.len() as u64;
            let state = if get("ok") == total {
                "ok"
            } else if get("fail") + get("canceled") > 0 {
                "fail"
            } else {
                "running"
            };
            envelope(true, "OK", "多画幅批量导出状态", json!({
                "parentRunId": parent_id,
                "runs": runs,
                "aggregate": {
                    "total": total, "ok": get("ok"), "fail": get("fail"),
                    "running": get("running"), "queued": get("queued"),
                    "paused": get("paused"), "canceled": get("canceled"),
                },
                "state": state,
            }))
        }
        other => envelope(false, "PRECONDITION_FAILED",
            &format!("未知 action: {other}(允许 run/status)"), json!({})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 静音段合并:排序 + 100ms 网格吸附 + 重叠合并;零长剔除;上限 20。
    #[test]
    fn merge_ranges_snaps_and_merges() {
        assert_eq!(merge_ranges(vec![(0, 100), (150, 300)]), vec![(0, 300)]);
        assert_eq!(merge_ranges(vec![(1050, 2000), (0, 999)]), vec![(0, 900), (1000, 2000)], "100ms 向下吸附");
        assert!(merge_ranges(vec![(500, 500)]).is_empty(), "零长剔除");
        let wide: Vec<(u64, u64)> = (0..40).map(|i| (i * 1000, i * 1000 + 500)).collect();
        assert_eq!(merge_ranges(wide).len(), 20, "上限 20 段");
    }

    /// 亮度量化:16 级档;风险阈值由原始均值判定(此处锁量化口径)。
    #[test]
    fn luma_quantization_grid() {
        assert_eq!(quant_luma(0.0), 0);
        assert_eq!(quant_luma(14.0), 16, "14 → 16(近黑档)");
        assert_eq!(quant_luma(255.0), 256);
        assert_eq!(quant_luma(100.4), 96);
    }

    /// preflight 缺工程 → NO_CONFIG;坏 schema → SCHEMA_INVALID(协议码如实)。
    #[test]
    fn preflight_rejects_missing_project() {
        let dir = std::env::temp_dir().join(format!("cf-pf-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let resp = export_preflight_tool(&dir, &json!({"root": dir.to_string_lossy()}));
        assert_eq!(resp["code"], json!("NO_CONFIG"), "{resp}");
        std::fs::create_dir_all(dir.join("05_时间线工程")).unwrap();
        std::fs::write(dir.join("05_时间线工程/project.json"), b"{bad").unwrap();
        let resp2 = export_preflight_tool(&dir, &json!({"root": dir.to_string_lossy()}));
        assert_eq!(resp2["code"], json!("SCHEMA_INVALID"), "{resp2}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// preflight 对纯文本工程:黑帧/静音面给出警告而非失败(协议完整,OK)。
    #[test]
    fn preflight_text_only_project_reports_warnings() {
        let dir = std::env::temp_dir().join(format!("cf-pf-text-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("05_时间线工程")).unwrap();
        std::fs::write(
            dir.join("05_时间线工程/project.json"),
            json!({
                "version": 1, "schemaVersion": "2.0.0", "slug": "pf", "fps": 30,
                "canvas": {"width": 1080, "height": 1920},
                "tracks": [{"id": "T1", "kind": "text", "clips": [
                    {"id": "T1-001", "startMs": 0, "durationMs": 1000, "text": "x"}
                ]}]
            })
            .to_string(),
        )
        .unwrap();
        let resp = export_preflight_tool(&dir, &json!({"root": dir.to_string_lossy()}));
        assert_eq!(resp["code"], json!("OK"), "{resp}");
        assert_eq!(resp["data"]["totalMs"], json!(0));
        assert_eq!(resp["data"]["heuristic"], json!(true));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// export_all_variants:未知预设拒绝;status 缺 parentRunId 拒绝(协议码如实)。
    #[test]
    fn variants_tool_rejects_unknown_preset_and_missing_parent() {
        let dir = std::env::temp_dir().join(format!("cf-pf-var-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let resp = export_all_variants_tool(&dir, &json!({"root": dir.to_string_lossy(), "ratios": ["21x9"]}));
        assert_eq!(resp["code"], json!("PRECONDITION_FAILED"), "{resp}");
        let resp2 = export_all_variants_tool(&dir, &json!({"root": dir.to_string_lossy(), "action": "status"}));
        assert_eq!(resp2["code"], json!("PRECONDITION_FAILED"), "{resp2}");
        let resp3 = export_all_variants_tool(&dir, &json!({"root": dir.to_string_lossy(), "action": "status", "parentRunId": "p无"}));
        assert_eq!(resp3["code"], json!("PRECONDITION_FAILED"), "{resp3}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
