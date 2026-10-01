// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 免开工作区工具实现(E6-3/B14:只读/创建类不持排他锁):project_new、
//! media_probe、media_browse、render_probe、stage_status(T1.1 拆分自 lib.rs,纯移动)。

use crate::dispatch::resolve_within_root;
use crate::orchestrate::orchestrate;
use crate::registry::envelope;
use cutforge_io::paths;
use serde_json::{json, Value};
use std::path::Path;

/// 可导入媒体扩展名(media_browse 的口径;与 mime_of 同域)。
const MEDIA_EXTS: &[&str] = &[
    "mp4", "m4v", "mov", "webm", "mkv",
    "mp3", "wav", "m4a", "aac", "ogg", "opus", "flac",
    "png", "jpg", "jpeg", "gif",
];

fn media_kind(ext: &str) -> &'static str {
    match ext {
        "mp4" | "m4v" | "mov" | "webm" | "mkv" => "video",
        "mp3" | "wav" | "m4a" | "aac" | "ogg" | "opus" | "flac" => "audio",
        _ => "image",
    }
}

/// 目录扫描上限(防超大素材目录拖垮服务;如实截断并在响应里标注)。
const BROWSE_CAP: usize = 500;
/// durationMs 元信息探测条数上限(ffprobe 逐文件成本;超出者该字段如实为 null)。
const BROWSE_PROBE_CAP: usize = 64;

/// E3-2/B12:media_probe——probe.rs 的 MCP 接线(此前"有实现无调用"的死代码)。
/// 返回时长/分辨率/是否含音轨;错误如实 DEP_MISSING(ffprobe 语义)。
pub(crate) fn media_probe_tool(root: &Path, args: &Value) -> Value {
    let Some(src) = args["src"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 src(工程内相对路径)", json!({}));
    };
    let abs = match resolve_within_root(root, src) {
        Ok(p) => p,
        Err(msg) => return envelope(false, "PRECONDITION_FAILED", &format!("路径不合法({src}): {msg}"), json!({})),
    };
    if !cutforge_io::probe::ffprobe_available() {
        return envelope(false, "DEP_MISSING", "ffprobe 不可用(安装 ffmpeg 套件或设 CUTFORGE_FFPROBE)", json!({}));
    }
    match cutforge_io::probe::probe(&abs) {
        Ok(info) => {
            let (width, height) = match info.video_size() {
                Some((w, h)) => (json!(w), json!(h)),
                None => (json!(null), json!(null)),
            };
            envelope(true, "OK", "媒体元信息(ffprobe 单一实现)", json!({
                "src": src, "durationMs": info.duration_ms(),
                "width": width, "height": height, "hasAudio": info.has_audio(),
            }))
        }
        Err(e) => envelope(false, "DEP_MISSING", &format!("探测失败: {e}"), json!({})),
    }
}

/// E3-3:media_browse——列工程内可导入媒体 + 元信息(供壳素材面板与 Agent 复用)。
/// 路径校验复用 resolve_within_root;目录递归至上限;durationMs 仅前 BROWSE_PROBE_CAP
/// 个文件逐个 ffprobe(超出如实 null,不阻塞列表)。
pub(crate) fn media_browse_tool(root: &Path, args: &Value) -> Value {
    let dir = args["dir"].as_str().unwrap_or("");
    match media_browse_payload(root, dir) {
        Ok(doc) => envelope(true, "OK", "素材清单", doc),
        Err(m) => envelope(false, "PRECONDITION_FAILED", &m, json!({})),
    }
}

pub(crate) fn media_browse_payload(root: &Path, dir: &str) -> Result<Value, String> {
    // 空 dir = 工程根自身(素材面板默认视图);"." 可通过 resolve_within_root 的校验
    let eff = if dir.is_empty() { "." } else { dir };
    let base = resolve_within_root(root, eff).map_err(|m| format!("目录不合法({dir}): {m}"))?;
    if !base.is_dir() {
        return Err(format!("目录不存在: {dir}"));
    }
    // 相对路径基于工程根的规范形计算(canonicalize 在 Windows 会加 \\?\ 前缀,
    // 必须与规范形根对比,否则 strip_prefix 落空)
    let canon_root = root.canonicalize().map_err(|e| format!("工程根不可达: {e}"))?;
    let mut files: Vec<Value> = Vec::new();
    let mut truncated = false;
    let mut stack = vec![base];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
            if !MEDIA_EXTS.contains(&ext.as_str()) {
                continue;
            }
            if files.len() >= BROWSE_CAP {
                truncated = true;
                break;
            }
            let bytes = e.metadata().map(|m| m.len()).unwrap_or(0);
            let rel = p.strip_prefix(&canon_root)
                .or_else(|_| p.strip_prefix(root))
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            files.push(json!({
                "name": p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
                "path": rel,
                "bytes": bytes,
                "kind": media_kind(&ext),
                "durationMs": json!(null),
            }));
        }
        if truncated {
            break;
        }
    }
    files.sort_by(|a, b| a["path"].as_str().unwrap_or("").cmp(b["path"].as_str().unwrap_or("")));
    if cutforge_io::probe::ffprobe_available() {
        for (i, f) in files.iter_mut().enumerate() {
            if i >= BROWSE_PROBE_CAP {
                break;
            }
            if let Ok(info) = cutforge_io::probe::probe(&root.join(f["path"].as_str().unwrap_or_default())) {
                f["durationMs"] = json!(info.duration_ms());
            }
        }
    }
    Ok(json!({"dir": dir, "total": files.len(), "truncated": truncated, "files": files}))
}

/// render_probe(E6-3 起):仅读成片输出目录存在性,不再为此申请排他锁。
/// 目录名走 paths 契约(三态布局感知:V3 `exports` / V2 `06_成片输出` / V1 `06_output`)。
pub(crate) fn render_probe_tool(root: &Path) -> Value {
    let dir = paths::output_dir(root);
    let mut files = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if let Ok(meta) = e.metadata() {
                files.push(json!({"file": e.file_name().to_string_lossy(), "bytes": meta.len()}));
            }
        }
    }
    let dir_name = dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| paths::output_dir_name(root).to_string());
    envelope(true, "OK", "产物清单", json!({"dir": dir_name, "files": files}))
}

/// stage_status(E6-3 起):编排 + 内部状态目录存在性,均不需要打开工作区(不持排他锁)。
pub(crate) fn stage_status_tool(ws_root: &Path) -> Value {
    // M9-3 同口径:优先解析 rs_run --status(done/stale/missing + staleReason),
    // CutFlow 不可用时回退 _内部状态(旧布局 _state)存在性并如实标注 degraded。
    let resp = orchestrate(ws_root, "rs_run.py", &[json!("--status")]);
    if resp.get("ok") == Some(&json!(true)) {
        if let Some(stages) = resp.get("data").and_then(|d| d.get("stages")).cloned() {
            return envelope(true, "OK", "阶段状态(rs_run 同口径)", json!({"stages": stages, "source": "rs_run"}));
        }
        if let Some(text) = resp.get("stdout").and_then(|v| v.as_str()) {
            let json_start = text.find('{').unwrap_or(text.len());
            if let Ok(v) = text[json_start..].parse::<Value>()
                && let Some(stages) = v.get("data").and_then(|d| d.get("stages")).cloned() {
                    return envelope(true, "OK", "阶段状态(rs_run 同口径)", json!({"stages": stages, "source": "rs_run"}));
                }
        }
    }
    let dir = paths::resolve_dir(ws_root, paths::STATE, paths::LEGACY_STATE);
    let mut map = serde_json::Map::new();
    for s in cutforge_io::stage::STAGES {
        map.insert(s.to_string(), json!(dir.join(format!("{s}.json")).is_file()));
    }
    envelope(true, "OK", "阶段状态(_内部状态 存在性;rs_run 不可用,降级)", json!({"stages": map, "source": "existence"}))
}

/// B11-1:project_new——空工程模板 + 建盘(与 CLI `new` 子命令同走 cutforge_io::scaffold,
/// 单一实现;已存在拒绝覆盖)。册六 ADR-0021:layout 显式开关(v2 缺省/v3 扁平)。
pub(crate) fn project_new_tool(root: &Path, args: &Value) -> Value {
    let slug = args["slug"].as_str().map(String::from)
        .or_else(|| root.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "cutforge-project".into());
    let fps = args["fps"].as_u64().unwrap_or(30) as u32;
    let width = args["canvasW"].as_u64().unwrap_or(1080) as u32;
    let height = args["canvasH"].as_u64().unwrap_or(1920) as u32;
    let layout = match args["layout"].as_str().unwrap_or("v2") {
        "v2" => paths::LayoutKind::V2,
        "v3" => paths::LayoutKind::V3,
        other => return envelope(false, "PRECONDITION_FAILED",
            &format!("未知布局: {other}(允许 v2/v3;过渡期缺省 v2,ADR-0021)"), json!({})),
    };
    let kinds_v = args["tracks"].as_array().cloned()
        .unwrap_or_else(|| vec![json!("video"), json!("audio")]);
    let mut kinds = Vec::new();
    for v in &kinds_v {
        match v.as_str() {
            Some("video") => kinds.push(cutforge_core::model::TrackKind::Video),
            Some("audio") => kinds.push(cutforge_core::model::TrackKind::Audio),
            Some("text") => kinds.push(cutforge_core::model::TrackKind::Text),
            Some("adjust") => kinds.push(cutforge_core::model::TrackKind::Adjust),
            other => return envelope(false, "PRECONDITION_FAILED", &format!("未知轨道类型: {other:?}(允许 video/audio/text/adjust)"), json!({})),
        }
    }
    match cutforge_io::scaffold::scaffold_project_layout(root, &slug, fps, width, height, &kinds, layout) {
        Ok(path) => envelope(true, "OK", "空工程已创建(可独立起步,不依赖 CutFlow)", json!({
            "project": path.to_string_lossy(),
            "layout": if layout == paths::LayoutKind::V3 { "v3" } else { "v2" },
            "hint": format!("打开:cutforge-cli serve {} 或 cutforge-mcp serve --root {}", root.display(), root.display()),
        })),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            envelope(false, "PRECONDITION_FAILED", &e.to_string(), json!({}))
        }
        Err(e) => envelope(false, "SCHEMA_INVALID", &e.to_string(), json!({})),
    }
}
