// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! CutForge MCP 层(计划书 5.1–5.4)。
//!
//! **单注册表双通道**:stdio(主)与内嵌 HTTP(辅,仅 127.0.0.1 + token)共用同一
//! `dispatch`,工具集差异恒为 0(M4-1)。所有返回值都是
//! `{ok, code, message, data}` 结果协议;code 取值限于计划书 5.4 表。
//! 编排类工具只封装 CutFlow 既有脚本(子进程透传),不实现任何阶段逻辑。

use cutforge_core::command::{ClipPatch, Command};
use cutforge_core::engine::{Answer, ApplyOpts, Query};
use cutforge_core::oplog::{Actor, ActorKind};
use cutforge_core::anchor::{Anchor, AnchorKind};
use cutforge_io::Workspace;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const MCP_TOOLS_JSON: &str = include_str!("../../../schemas/mcp-tools.json");
/// 能力矩阵单一真相源(ADR-0003/M8-5):MCP 工具与 docs/capability-matrix.md 同源。
pub const CAPABILITY_MATRIX_JSON: &str = include_str!("../../../docs/capability-matrix.json");

/// 工具注册表(契约来自 schemas/mcp-tools.json;派发处理器同在本 crate)。
pub fn registry() -> &'static Vec<Value> {
    static CACHE: OnceLock<Vec<Value>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let doc: Value = serde_json::from_str(MCP_TOOLS_JSON).expect("mcp-tools.json 必须合法");
        doc["tools"].as_array().cloned().unwrap_or_default()
    })
}

pub fn tool_names() -> Vec<String> {
    registry()
        .iter()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect()
}

fn tool_def(name: &str) -> Option<&'static Value> {
    registry().iter().find(|t| t["name"].as_str() == Some(name))
}

fn envelope(ok: bool, code: &str, message: &str, data: Value) -> Value {
    json!({"ok": ok, "code": code, "message": message, "data": data})
}

/// 5.4 错误码表(协议一致性门禁的比对基准)。
pub const CODES: &[&str] = &[
    "OK", "CONFLICT", "SCHEMA_INVALID", "PRECONDITION_FAILED", "GUARD_FAILED",
    "JIANYING_RUNNING", "NO_CONFIG", "DEP_MISSING", "GREEN_SCREEN_INPUT", "INTERNAL",
];

// ---------------- 派发 ----------------

/// 命令通道统一派发:所有通道(stdio/HTTP/脚本宿主)都走这里。
/// root(工程目录)由 args["root"] 提供——工具契约的第一参数。
pub fn dispatch(name: &str, args: &Value) -> Value {
    if tool_def(name).is_none() {
        return envelope(false, "INTERNAL", &format!("未知工具: {name}"), json!({}));
    }
    // capability_matrix 是静态查询,不需要工程根
    if name == "capability_matrix" {
        return envelope(true, "OK", "能力对等矩阵(实码口径,单一真相源)", json!({
            "matrix": capability_matrix(),
        }));
    }
    let Some(root_str) = args["root"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 root(工程目录)", json!({}));
    };
    let ws_root = PathBuf::from(root_str);

    // E5/B6:render 按 backend 分派——cutforge 后端调用本地 cutforge-render 子进程,
    // 全程不打开工作区(渲染不持排他锁);ffmpeg 后端(默认)走 CutFlow 编排,维持原路径。
    // render_run / render_progress 同理不持锁(E5-3 异步渲染 + 轮询进度)。
    // ass 过滤在服务端:壳无文件系统能力(壳纯度),ass 路径不存在时不烧录而非整单失败。
    if name == "render" && args["backend"].as_str() == Some("cutforge") {
        return render_cutforge_sync(&ws_root, existing_rel(&ws_root, args["ass"].as_str()));
    }
    if name == "render_run" {
        return render_run_async(&ws_root, existing_rel(&ws_root, args["ass"].as_str()));
    }
    if name == "render_progress" {
        let Some(run_id) = args["runId"].as_str() else {
            return envelope(false, "PRECONDITION_FAILED", "缺 runId", json!({}));
        };
        return render_progress(run_id);
    }

    let opts = ApplyOpts {
        request_id: args["requestId"].as_str().map(String::from),
        summary: args["summary"].as_str().map(String::from),
        caused_by: args["causedBy"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        expect_rev: args["expectRev"].as_u64(),
        ..Default::default()
    };
    let actor = Actor::agent("cutforge-mcp");

    // 常驻同步守护(M9-2):外部改动 ≤1s 可见;幂等(每 root 一个线程)
    let _ = cutforge_io::watcher::ensure_sync_daemon(&ws_root);
    // 写通道全程锁:open→apply→persist 同一把锁(P0-5,杜绝锁外读+整文件覆盖)
    let mut ws = match Workspace::open_exclusive(&ws_root) {
        Ok(w) => w,
        Err(e) => {
            let code = if e.kind() == std::io::ErrorKind::NotFound { "NO_CONFIG" } else { "INTERNAL" };
            return envelope(false, code, &e.to_string(), json!({}));
        }
    };

    match name {
        // ---------- 只读查询 ----------
        "project_get" => match ws.engine().query(Query::ProjectView) {
            Answer::Project(v) => envelope(true, "OK", "工程视图", json!({"project": v, "rev": ws.rev()})),
            _ => unreachable!(),
        },
        "wordline_get" => read_truth(&ws_root, "05_ir/wordline.json", "wordline"),
        "cutlist_get" => {
            let applied = args["applied"].as_bool().unwrap_or(false);
            let rel = if applied { "04_cut/cutlist.applied.json" } else { "04_cut/cutlist.json" };
            read_truth(&ws_root, rel, "cutlist")
        }
        "notes_list" => {
            let state = args["state"].as_str().and_then(|s| serde_json::from_str(&format!("\"{s}\"")).ok());
            let author = args["author"].as_str().map(|s| match s {
                "agent" => cutforge_core::notes::NoteAuthor::Agent,
                _ => cutforge_core::notes::NoteAuthor::User,
            });
            let items = ws.notes().filter(state, author);
            envelope(true, "OK", "标注清单", json!({
                "notes": items, "total": ws.notes().notes().len(),
                "orphans": ws.notes().orphans().len(),
            }))
        }
        "stage_status" => {
            // M9-3 同口径:优先解析 rs_run --status(done/stale/missing + staleReason),
            // CutFlow 不可用时回退 _state 存在性并如实标注 degraded。
            let resp = orchestrate(&ws_root, "rs_run.py", &[json!("--status")]);
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
            let dir = ws_root.join("_state");
            let mut map = serde_json::Map::new();
            for s in cutforge_io::stage::STAGES {
                map.insert(s.to_string(), json!(dir.join(format!("{s}.json")).is_file()));
            }
            envelope(true, "OK", "阶段状态(_state 存在性;rs_run 不可用,降级)", json!({"stages": map, "source": "existence"}))
        }
        "oplog_tail" => match ws.engine().query(Query::OpLogTail {
            since_rev: args["sinceRev"].as_u64(),
            actor_kind: args["actor"].as_str().map(|s| match s {
                "user" => ActorKind::User,
                "script" => ActorKind::Script,
                _ => ActorKind::Agent,
            }),
        }) {
            Answer::Ops(ops) => {
                let limit = args["limit"].as_u64().unwrap_or(50) as usize;
                let slice: Vec<_> = ops.iter().rev().take(limit).rev().cloned().collect();
                envelope(true, "OK", "OpLog tail", json!({"ops": slice, "count": ops.len(), "rev": ws.rev()}))
            }
            _ => unreachable!(),
        },
        "conflict_list" => match ws.conflict_list() {
            Ok(items) => {
                let rows: Vec<Value> = items
                    .iter()
                    .map(|(id, c)| json!({"conflictId": id, "code": c.code.code(), "pointer": c.pointer}))
                    .collect();
                envelope(true, "OK", "冲突清单", json!({"conflicts": rows}))
            }
            Err(e) => envelope(false, "INTERNAL", &e.to_string(), json!({})),
        },
        "render_probe" => {
            let dir = ws_root.join("06_output");
            let mut files = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    if let Ok(meta) = e.metadata() {
                        files.push(json!({"file": e.file_name().to_string_lossy(), "bytes": meta.len()}));
                    }
                }
            }
            envelope(true, "OK", "产物清单", json!({"dir": "06_output", "files": files}))
        }
        "capability_matrix" => envelope(true, "OK", "能力对等矩阵(实码口径,单一真相源)", json!({
            "matrix": capability_matrix(),
        })),
        "timeline_get" => {
            // E2-2:扩投影——预览/检查器所需的逐 clip 字段全部由内核算好下放
            // (endMs = start+duration 在服务端完成;壳只消费,不做时间线运算)。
            envelope(true, "OK", "时间线投影", json!({
                "clips": timeline_projection(ws.project()),
                "rev": ws.rev(),
            }))
        }

        // ---------- 写操作(全部经 Workspace 命令通道) ----------
        "clip_update" => {
            let Some(clip_id) = args["clipId"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({}));
            };
            let p = &args["patch"];
            let patch = ClipPatch {
                start_ms: p["startMs"].as_u64(),
                duration_ms: p["durationMs"].as_u64(),
                source_in_ms: p["sourceInMs"].as_u64(),
                speed: p["speed"].as_f64(),
                volume: p["volume"].as_f64(),
                opacity: p["opacity"].as_f64(),
                scale: p["scale"].as_f64(),
                text: p["text"].as_str().map(String::from),
                freeze_ms: p["freezeMs"].as_u64(),
            };
            finish(ws.apply(Command::ClipUpdate { clip_id: clip_id.into(), patch }, actor, opts))
        }
        "clip_split" => match args["clipId"].as_str().zip(args["tMs"].as_u64()) {
            Some((clip_id, t_ms)) => finish(ws.apply(Command::ClipSplit { clip_id: clip_id.into(), t_ms }, actor, opts)),
            None => envelope(false, "PRECONDITION_FAILED", "缺 clipId/tMs", json!({})),
        },
        "clip_delete" => match args["clipId"].as_str() {
            Some(clip_id) => finish(ws.apply(Command::ClipDelete { clip_id: clip_id.into() }, actor, opts)),
            None => envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({})),
        },
        "clip_move" => {
            let (Some(clip_id), Some(start_ms)) = (args["clipId"].as_str(), args["startMs"].as_u64()) else {
                return envelope(false, "PRECONDITION_FAILED", "缺 clipId/startMs", json!({}));
            };
            finish(ws.apply(
                Command::ClipMove { clip_id: clip_id.into(), new_start_ms: start_ms, to_track: args["toTrack"].as_str().map(String::from) },
                actor, opts,
            ))
        }
        "clip_duplicate" => {
            let (Some(clip_id), Some(start_ms)) = (args["clipId"].as_str(), args["startMs"].as_u64()) else {
                return envelope(false, "PRECONDITION_FAILED", "缺 clipId/startMs", json!({}));
            };
            let src_track = ws.project().find_clip(clip_id).map(|(ti, _)| ti);
            let Some(src_ti) = src_track else {
                return envelope(false, "PRECONDITION_FAILED", &format!("clip 不存在: {clip_id}"), json!({}));
            };
            let to_track = args["toTrack"].as_str().map(String::from)
                .unwrap_or_else(|| ws.project().tracks[src_ti].id.clone());
            let Some(ti) = ws.project().find_track(&to_track) else {
                return envelope(false, "PRECONDITION_FAILED", &format!("track 不存在: {to_track}"), json!({}));
            };
            let mut clip = ws.project().tracks[src_ti].clips[ws.project().find_clip(clip_id).unwrap().1].clone();
            if ws.project().tracks[ti].kind != ws.project().tracks[src_ti].kind {
                return envelope(false, "GUARD_FAILED", "跨 kind 复制拒绝", json!({}));
            }
            clip.id = cutforge_core::model::Project::next_clip_id(&ws.project().tracks[ti]);
            clip.start_ms = start_ms;
            let request_id = args["requestId"].as_str().map(String::from);
            finish(ws.apply(Command::ClipInsert { to_track, clip, request_id }, actor, opts))
        }
        "track_add" => {
            let Some(kind) = args["kind"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 kind", json!({}));
            };
            let kind = match kind {
                "video" => cutforge_core::model::TrackKind::Video,
                "audio" => cutforge_core::model::TrackKind::Audio,
                "text" => cutforge_core::model::TrackKind::Text,
                other => return envelope(false, "PRECONDITION_FAILED", &format!("未知 kind: {other}"), json!({})),
            };
            let request_id = args["requestId"].as_str().map(String::from);
            finish(ws.apply(Command::TrackAdd { kind, request_id }, actor, opts))
        }
        "subtitle_set" | "subtitle_retime" => {
            let Some(clip_id) = args["clipId"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({}));
            };
            let patch = if name == "subtitle_set" {
                ClipPatch { text: args["text"].as_str().map(String::from), ..Default::default() }
            } else {
                ClipPatch {
                    start_ms: args["startMs"].as_u64(),
                    duration_ms: args["durationMs"].as_u64(),
                    ..Default::default()
                }
            };
            finish(ws.apply(Command::ClipUpdate { clip_id: clip_id.into(), patch }, actor, opts))
        }
        "overlay_add" => {
            let (Some(track_id), Some(src), Some(at_ms), Some(dur)) = (
                args["trackId"].as_str(), args["element"]["src"].as_str(),
                args["element"]["atMs"].as_u64(), args["element"]["durationMs"].as_u64(),
            ) else {
                return envelope(false, "PRECONDITION_FAILED", "缺 trackId/element.src/atMs/durationMs", json!({}));
            };
            let clip_id = next_clip_id_for(ws.project(), track_id);
            let clip_json = json!({
                "id": clip_id, "src": src, "startMs": at_ms, "durationMs": dur,
                "volume": 0, "overlay": args["element"]["overlay"],
            });
            let clip: cutforge_core::model::Clip = match serde_json::from_value(clip_json) {
                Ok(c) => c,
                Err(e) => return envelope(false, "SCHEMA_INVALID", &e.to_string(), json!({})),
            };
            let request_id = args["requestId"].as_str().map(String::from);
            finish(ws.apply(Command::ClipInsert { to_track: track_id.into(), clip, request_id }, actor, opts))
        }
        "sfx_add" => {
            let (Some(t_ms), Some(src)) = (args["tMs"].as_u64(), args["src"].as_str()) else {
                return envelope(false, "PRECONDITION_FAILED", "缺 tMs/src", json!({}));
            };
            // 密度护栏:±15s 内已有 ≥2 个 sfx → GUARD_FAILED(计划书 5.2)
            let sfx_nearby = count_sfx_near(ws.project(), t_ms, 15_000);
            if sfx_nearby >= 2 {
                return envelope(false, "GUARD_FAILED", &format!("音效密度超限:{t_ms}ms ±15s 内已有 {sfx_nearby} 个"), json!({}));
            }
            let track_id = ws.project().tracks.iter()
                .find(|t| t.kind == cutforge_core::model::TrackKind::Audio)
                .map(|t| t.id.clone())
                .unwrap_or_else(|| "A1".into());
            let clip_id = next_clip_id_for(ws.project(), &track_id);
            let clip_json = json!({
                "id": clip_id, "src": src, "startMs": t_ms, "durationMs": 400,
                "role": "sfx", "volume": args["volume"].as_f64().unwrap_or(0.8),
            });
            let clip: cutforge_core::model::Clip = match serde_json::from_value(clip_json) {
                Ok(c) => c,
                Err(e) => return envelope(false, "SCHEMA_INVALID", &e.to_string(), json!({})),
            };
            let request_id = args["requestId"].as_str().map(String::from);
            finish(ws.apply(Command::ClipInsert { to_track: track_id, clip, request_id }, actor, opts))
        }
        "notes_add" => {
            let (Some(anchor_v), Some(body)) = (args["anchor"].as_object(), args["body"].as_str()) else {
                return envelope(false, "PRECONDITION_FAILED", "缺 anchor/body", json!({}));
            };
            let kind = match anchor_v.get("kind").and_then(|v| v.as_str()).unwrap_or("clip") {
                "track" => AnchorKind::Track,
                "time" => AnchorKind::Time,
                "word" => AnchorKind::Word,
                "subtitleCard" => AnchorKind::SubtitleCard,
                _ => AnchorKind::Clip,
            };
            let anchor = Anchor {
                kind,
                // 注意:serde Map 的 Index 在键缺失时 panic,必须用 .get()(M10 e2e 实测)
                ref_: anchor_v.get("ref").and_then(|v| v.as_str()).map(String::from),
                t_ms: anchor_v.get("tMs").and_then(|v| v.as_u64()).unwrap_or(0),
                span: None,
            };
            let author = if args["author"].as_str() == Some("agent") {
                cutforge_core::notes::NoteAuthor::Agent
            } else {
                cutforge_core::notes::NoteAuthor::User
            };
            let tags = args["tags"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
            match ws.notes_add(anchor, body.into(), author, tags, actor, args["requestId"].as_str().map(String::from)) {
                Ok(id) => envelope(true, "OK", "标注已创建", json!({"noteId": id, "rev": ws.rev()})),
                Err(e) => envelope(false, "INTERNAL", &e.to_string(), json!({})),
            }
        }
        "notes_resolve" => {
            let (Some(note_id), Some(reply)) = (args["noteId"].as_str(), args["reply"].as_str()) else {
                return envelope(false, "PRECONDITION_FAILED", "缺 noteId/reply", json!({}));
            };
            let op_ids = args["opIds"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
            match ws.notes_resolve(note_id, reply.into(), op_ids, actor) {
                Ok(()) => envelope(true, "OK", "标注已结案", json!({"noteId": note_id})),
                Err(e) => notes_op_error(e),
            }
        }
        "notes_reject" => {
            let (Some(note_id), Some(reason)) = (args["noteId"].as_str(), args["reason"].as_str()) else {
                return envelope(false, "PRECONDITION_FAILED", "缺 noteId/reason", json!({}));
            };
            match ws.notes_reject(note_id, reason.into(), actor) {
                Ok(()) => envelope(true, "OK", "标注已否决", json!({"noteId": note_id})),
                Err(e) => notes_op_error(e),
            }
        }
        "cut_apply" => {
            let patch = args["patch"].as_object().cloned();
            let Some(patch) = patch else {
                return envelope(false, "PRECONDITION_FAILED", "缺 patch(merge-patch 对象)", json!({}));
            };
            apply_cut_merge_patch(&mut ws, &Value::Object(patch))
        }
        "undo" | "redo" => {
            let batch = args["batch"].as_u64().unwrap_or(1);
            let mut last = envelope(true, "OK", "无操作", json!({}));
            for _ in 0..batch {
                last = if name == "undo" {
                    finish(ws.undo(actor.clone()))
                } else {
                    finish(ws.redo(actor.clone()))
                };
                if last["ok"] != json!(true) {
                    break;
                }
            }
            last
        }

        // ---------- 编排(封装 CutFlow 脚本,不重实现) ----------
        "stage_run" | "stage_rebuild" | "verify_run" | "sync_check" | "render" | "export_jianying" => {
            let script = match name {
                "stage_run" => "rs_run.py",
                "stage_rebuild" => "rebuild.py",
                "verify_run" => "rs_verify.py",
                "sync_check" => "rs_sync.py",
                "render" => "rs_render.py",
                _ => "rs_jy_draft.py",
            };
            let script_args = args["scriptArgs"].as_array().cloned().unwrap_or_default();
            orchestrate(&ws_root, script, &script_args)
        }

        other => envelope(false, "INTERNAL", &format!("工具已注册但未实现: {other}"), json!({})),
    }
}

fn finish(r: Result<cutforge_core::engine::OpReceipt, std::io::Error>) -> Value {
    match r {
        Ok(rec) => envelope(true, "OK", "已应用", json!({"opIds": rec.op_ids, "rev": rec.rev, "idempotent": rec.idempotent})),
        Err(e) => reject_to_envelope(e.to_string()),
    }
}

fn reject_to_envelope(msg: String) -> Value {
    let code = if msg.starts_with("PRECONDITION_FAILED") {
        "PRECONDITION_FAILED"
    } else if msg.starts_with("CONFLICT") {
        "CONFLICT"
    } else if msg.starts_with("SCHEMA_INVALID") {
        "SCHEMA_INVALID"
    } else if msg.starts_with("GUARD_FAILED") {
        "GUARD_FAILED"
    } else if msg.starts_with("NOTHING_TO_") {
        "PRECONDITION_FAILED"
    } else {
        "INTERNAL"
    };
    envelope(false, code, &msg, json!({}))
}

fn read_truth(root: &Path, rel: &str, label: &str) -> Value {
    let p = root.join(rel);
    match std::fs::read_to_string(&p) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(v) => envelope(true, "OK", label, json!({label.to_string().replace('-', "_"): v})),
            Err(e) => envelope(false, "SCHEMA_INVALID", &e.to_string(), json!({})),
        },
        Err(_) => envelope(false, "NO_CONFIG", &format!("文件不存在: {}", p.display()), json!({})),
    }
}

fn next_clip_id_for(project: &cutforge_core::model::Project, track_id: &str) -> String {
    project
        .find_track(track_id)
        .map(|ti| cutforge_core::model::Project::next_clip_id(&project.tracks[ti]))
        .unwrap_or_else(|| format!("{track_id}-999"))
}

/// E2-2:时间线投影的逐 clip 全字段(endMs 在服务端算好;壳零时间线语义)。
fn timeline_projection(project: &cutforge_core::model::Project) -> Vec<Value> {
    let mut rows = Vec::new();
    for t in &project.tracks {
        for c in &t.clips {
            rows.push(json!({
                "id": c.id, "track": t.id,
                "trackKind": match t.kind {
                    cutforge_core::model::TrackKind::Video => "video",
                    cutforge_core::model::TrackKind::Audio => "audio",
                    cutforge_core::model::TrackKind::Text => "text",
                },
                "src": c.src, "startMs": c.start_ms, "endMs": c.start_ms + c.duration_ms,
                "durationMs": c.duration_ms, "sourceInMs": c.source_in_ms,
                "speed": c.speed, "volume": c.volume, "opacity": c.opacity,
                "scale": c.scale, "position": c.position, "overlay": c.overlay,
                "motion": c.motion, "text": c.text, "freezeMs": c.freeze_ms,
            }));
        }
    }
    rows
}

fn count_sfx_near(project: &cutforge_core::model::Project, t_ms: u64, window: u64) -> usize {
    project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .filter(|c| c.role == Some(cutforge_core::model::Role::Sfx))
        .filter(|c| {
            let end = c.start_ms + c.duration_ms;
            c.start_ms <= t_ms + window && t_ms <= end + window
        })
        .count()
}

/// RFC7386 merge-patch 应用到 cutlist.json,schema 校验后走 record_change 审计。
/// 文件本体由 Workspace 的 reconcile(先文件后记账)落盘——不再旁路自写。
fn apply_cut_merge_patch(ws: &mut Workspace, patch: &Value) -> Value {
    let rel = ws.root().join("04_cut/cutlist.json");
    let Ok(text) = std::fs::read_to_string(&rel) else {
        return envelope(false, "NO_CONFIG", &format!("文件不存在: {}", rel.display()), json!({}));
    };
    let Ok(before) = serde_json::from_str::<Value>(&text) else {
        return envelope(false, "SCHEMA_INVALID", "cutlist.json 非法 JSON", json!({}));
    };
    let mut after = merge_patch(before.clone(), patch);
    // M9-3:按 cuts[].action 服务端重算 keep/removedMs(rs_cut.finalize_cutlist 镜像,
    // 金样对拍锁定)——经 MCP 的编辑不再是"keep 幻觉"
    if let Err(e) = cutforge_schema::finalize::finalize_cutlist_value(&mut after) {
        return envelope(false, "GUARD_FAILED", &format!("keep 重算失败: {e}"), json!({}));
    }
    let errors = cutforge_schema::validate("cutlist", &after);
    if !errors.is_empty() {
        return envelope(false, "SCHEMA_INVALID", &errors.join("; "), json!({"errors": errors}));
    }
    let rec = ws.record_change(
        "cutlist.json",
        "/",
        before,
        after,
        cutforge_core::oplog::OpKind::Set,
        Actor::script("cutforge-mcp:cut_apply"),
        ApplyOpts { summary: Some("cut_apply merge-patch".into()), ..Default::default() },
    );
    match rec {
        Ok(r) => envelope(true, "OK", "cutlist 已更新", json!({"rev": r.rev, "opIds": r.op_ids})),
        Err(e) => envelope(false, "INTERNAL", &e.to_string(), json!({})),
    }
}

/// 标注操作的协议错误映射(5.4 表内码,禁止占位符):不存在/参数不合法 →
/// PRECONDITION_FAILED;其余 → INTERNAL。
fn notes_op_error(e: std::io::Error) -> Value {
    let code = if e.kind() == std::io::ErrorKind::NotFound || e.kind() == std::io::ErrorKind::InvalidInput {
        "PRECONDITION_FAILED"
    } else {
        "INTERNAL"
    };
    envelope(false, code, &e.to_string(), json!({}))
}

fn merge_patch(mut target: Value, patch: &Value) -> Value {
    if let (Some(t), Some(p)) = (target.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            if v.is_null() {
                t.remove(k);
            } else {
                let nv = match t.get(k) {
                    Some(existing) if existing.is_object() && v.is_object() => merge_patch(existing.clone(), v),
                    _ => v.clone(),
                };
                t.insert(k.clone(), nv);
            }
        }
        target
    } else {
        patch.clone()
    }
}

// ---------------- 渲染后端分派(E5/B6:cutforge-render 子进程,不依赖 CutFlow) ----------------

/// E5-2:cutforge-render 可执行定位:当前可执行文件同目录 → PATH → env CUTFORGE_RENDER。
fn resolve_render_bin() -> Option<PathBuf> {
    let exe_name = format!("cutforge-render{}", std::env::consts::EXE_SUFFIX);
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent() {
            let cand = dir.join(&exe_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let cand = dir.join(&exe_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    std::env::var_os("CUTFORGE_RENDER").map(PathBuf::from)
}

fn render_missing_dep() -> Value {
    envelope(false, "DEP_MISSING",
        "未找到 cutforge-render:与本程序同目录放置、加入 PATH,或设 CUTFORGE_RENDER 指向可执行文件", json!({}))
}

/// ass 相对路径存在才透传(缺字幕 = 不烧录,而非渲染失败)。
fn existing_rel<'a>(root: &Path, rel: Option<&'a str>) -> Option<&'a str> {
    rel.filter(|r| !r.is_empty() && root.join(r).is_file())
}

fn spawn_render(root: &Path, ass: Option<&str>) -> std::process::Command {
    let mut cmd = match resolve_render_bin() {
        Some(p) => std::process::Command::new(p),
        None => std::process::Command::new("cutforge-render"),
    };
    cmd.arg("--root").arg(root);
    if let Some(a) = ass {
        cmd.arg("--ass").arg(a);
    }
    cmd
}

/// 同步渲染(MCP 工具 render,backend=cutforge):输出 JSON 行进度进 data.stdout。
fn render_cutforge_sync(root: &Path, ass: Option<&str>) -> Value {
    if resolve_render_bin().is_none() {
        return render_missing_dep();
    }
    match spawn_render(root, ass).output() {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let output = text.lines().rev().find_map(|l| serde_json::from_str::<Value>(l).ok())
                .and_then(|v| v.get("output").and_then(|o| o.as_str()).map(String::from));
            envelope(true, "OK", "渲染完成",
                json!({"backend": "cutforge", "output": output, "stdout": text.trim()}))
        }
        Ok(out) => envelope(false, "INTERNAL",
            &format!("cutforge-render 失败:{}", String::from_utf8_lossy(&out.stderr).trim().chars().take(300).collect::<String>()),
            json!({})),
        Err(e) => envelope(false, "DEP_MISSING", &format!("cutforge-render 不可用: {e}"), json!({})),
    }
}

/// E5-3 异步渲染任务表(内存态;进程生命周期内有效)。
struct RenderJob {
    state: &'static str, // running | ok | fail
    lines: Vec<String>,
    output: Option<String>,
    error: Option<String>,
}

fn renders() -> &'static std::sync::Mutex<HashMap<String, RenderJob>> {
    static R: OnceLock<std::sync::Mutex<HashMap<String, RenderJob>>> = OnceLock::new();
    R.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn render_run_async(root: &Path, ass: Option<&str>) -> Value {
    if resolve_render_bin().is_none() {
        return render_missing_dep();
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::process::id().hash(&mut h);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
    let run_id = format!("r{:016x}", h.finish());
    let mut cmd = spawn_render(root, ass);
    if let Ok(mut m) = renders().lock() {
        m.insert(run_id.clone(), RenderJob { state: "running", lines: Vec::new(), output: None, error: None });
    }
    let run_id_thread = run_id.clone();
    std::thread::spawn(move || {
        let run_id = run_id_thread;
        let Ok(mut child) = cmd
            .stdout(std::process::Stdio::piped())
            .spawn()
        else {
            if let Ok(mut m) = renders().lock()
                && let Some(j) = m.get_mut(&run_id) {
                    j.state = "fail";
                    j.error = Some("cutforge-render 子进程启动失败".into());
                }
            return;
        };
        // 只接 stdout(JSON 行进度);stderr 直通服务端控制台(不读不堵塞)。
        if let Some(out) = child.stdout.take() {
            let reader = std::io::BufReader::new(out);
            for line in std::io::BufRead::lines(reader).map_while(Result::ok) {
                let Ok(mut m) = renders().lock() else { break };
                let Some(j) = m.get_mut(&run_id) else { break };
                if let Ok(v) = serde_json::from_str::<Value>(&line)
                    && let Some(o) = v.get("output").and_then(|o| o.as_str()) {
                        j.output = Some(o.to_string());
                    }
                j.lines.push(line);
                let n = j.lines.len();
                if n > 200 {
                    j.lines.drain(..n - 200);
                }
            }
        }
        let ok = child.wait().map(|s| s.success()).unwrap_or(false);
        if let Ok(mut m) = renders().lock()
            && let Some(j) = m.get_mut(&run_id) {
                j.state = if ok { "ok" } else { "fail" };
                if !ok {
                    j.error = Some("cutforge-render 非零退出;详见服务端控制台".into());
                }
            }
    });
    envelope(true, "OK", "渲染已开始", json!({"runId": run_id}))
}

fn render_progress(run_id: &str) -> Value {
    let Ok(m) = renders().lock() else {
        return envelope(false, "INTERNAL", "渲染任务表不可用", json!({}));
    };
    match m.get(run_id) {
        Some(j) => envelope(true, "OK", "渲染进度", json!({
            "state": j.state,
            "lines": j.lines.iter().rev().take(30).rev().collect::<Vec<_>>(),
            "output": j.output,
            "error": j.error,
        })),
        None => envelope(false, "PRECONDITION_FAILED", &format!("未知 runId: {run_id}"), json!({})),
    }
}

/// Python 启动器探测(py -3 → python3 → python;Windows 仅装 py-launcher 的机器不再全灭)。
fn py_launcher() -> Option<Vec<String>> {
    static CACHE: OnceLock<Option<Vec<String>>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
                for cand in [["py", "-3"], ["python3", ""], ["python", ""]] {
                    let args: Vec<&str> = cand[1..].iter().filter(|s| !s.is_empty()).copied().collect();
                    let ok = std::process::Command::new(cand[0])
                        .args(args)
                    .arg("-c")
                    .arg("print(1)")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if ok {
                    // 过滤占位空串:空串若作为参数传回会给调用方埋雷(CI 实测 python3 "" -V 必败)
                    return Some(cand.iter().filter(|s| !s.is_empty()).map(|s| s.to_string()).collect());
                }
            }
            None
        })
        .clone()
}

/// CutFlow 仓库定位(去硬编码):CUTFLOW_REPO → 可执行文件祖先目录 → 工程目录祖先。
fn resolve_cutflow_dir(ws_root: &Path) -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("CUTFLOW_REPO") {
        let p = PathBuf::from(v);
        if p.join("skills/cutflow/scripts").is_dir() {
            return Some(p);
        }
    }
    let probe = |base: &Path| -> Option<PathBuf> {
        base.ancestors().skip(1).take(4).map(|a| a.join("CutFlow"))
            .find(|c| c.join("skills/cutflow/scripts").is_dir())
    };
    if let Ok(exe) = std::env::current_exe()
        && let Some(p) = probe(&exe) {
            return Some(p);
        }
    probe(ws_root)
}

/// scriptArgs 序列化:字符串原样,数字/布尔转字符串;复杂对象如实拒绝(不再静默丢弃)。
fn script_arg_to_string(v: &Value) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        other => Err(format!("不支持scriptArgs 元素类型: {other}")),
    }
}

fn orchestrate(ws_root: &Path, script: &str, script_args: &[Value]) -> Value {    let Some(cutflow) = resolve_cutflow_dir(ws_root) else {
        return envelope(false, "DEP_MISSING",
            "未找到 CutFlow 仓库:设 CUTFLOW_REPO 指向仓库根(skills/cutflow/scripts 需存在)", json!({}));
    };
    let mut script_path = cutflow.join("skills/cutflow/scripts").join(script);
    // stage_rebuild 的脚本在工程目录内(rebuild.py 由 rs_run --init 生成)
    if !script_path.is_file() {
        script_path = ws_root.join(script);
    }
    if !script_path.is_file() {
        return envelope(false, "DEP_MISSING", &format!("脚本不存在: {}", script_path.display()), json!({}));
    }
    let Some(py) = py_launcher() else {
        return envelope(false, "DEP_MISSING", "未找到可用的 Python(py -3/python3/python 均不可用)", json!({}));
    };
    // --json 白名单:仅契约声明支持该旗标的脚本(rs_verify);其余追加 --json 会被
    // argparse 以退出码 2 拒绝——这正是 M4 三个编排工具必崩的根因(P1-5)。
    let supports_json = matches!(script, "rs_verify.py");
    let mut str_args: Vec<String> = Vec::new();
    for v in script_args {
        match script_arg_to_string(v) {
            Ok(s) => str_args.push(s),
            Err(m) => return envelope(false, "PRECONDITION_FAILED", &m, json!({})),
        }
    }
    let mut cmd = std::process::Command::new(&py[0]);
    cmd.args(&py[1..]).arg(&script_path);
    for a in &str_args {
        cmd.arg(a);
    }
    if supports_json {
        cmd.arg("--json");
    }
    cmd.current_dir(ws_root);
    let out = cmd.output();
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            if supports_json {
                let json_start = text.find('{').unwrap_or(text.len());
                match text[json_start..].parse::<Value>() {
                    Ok(v) => v,
                    Err(_) => envelope(true, "OK", "编排完成", json!({"stdout": text.trim()})),
                }
            } else {
                envelope(true, "OK", "编排完成", json!({"stdout": text.trim()}))
            }
        }
        Ok(o) => {
            let code = if o.status.code() == Some(3) { "DEP_MISSING" } else { "INTERNAL" };
            envelope(false, code, &String::from_utf8_lossy(&o.stderr).trim().chars().take(300).collect::<String>(), json!({}))
        }
        Err(e) => envelope(false, "DEP_MISSING", &format!("{} 不可用: {e}", py.join(" ")), json!({})),
    }
}

/// ADR-0003/M8-5:能力矩阵是**生成物**——唯一真相源 docs/capability-matrix.json,
/// 本工具与文档同源;status 只认实码+夹具证据,与 capability-matrix.md 联动更新。
fn capability_matrix() -> Value {
    serde_json::from_str(CAPABILITY_MATRIX_JSON).expect("capability-matrix.json 必须合法")
}


// ---------------- JSON-RPC 传输(两通道共用 handle_rpc) ----------------

/// 处理一条 JSON-RPC 请求;通知(无 id)返回 None。
pub fn handle_rpc(req: &Value) -> Option<Value> {
    let method = req["method"].as_str()?;
    let id = req["id"].clone();
    if id.is_null() {
        return None; // 通知:不回应
    }
    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "cutforge-mcp", "version": env!("CARGO_PKG_VERSION")}
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": registry().iter().map(|t| json!({
            "name": t["name"], "description": t["description"],
            "inputSchema": t["inputSchema"],
        })).collect::<Vec<_>>()}),
        "tools/call" => {
            let name = req["params"]["name"].as_str().unwrap_or("");
            let args = req["params"]["arguments"].clone();
            let env = dispatch(name, &args);
            json!({
                "content": [{"type": "text", "text": env.to_string()}],
                "isError": env["ok"] != json!(true),
            })
        }
        other => {
            return Some(json!({
                "jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": format!("method not found: {other}")}
            }))
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// stdio 主通道:逐行读 JSON-RPC,逐行写响应。
pub fn serve_stdio() -> i32 {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in std::io::BufRead::lines(stdin.lock()) {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Value>(&line) {
            Ok(req) => handle_rpc(&req).map(|r| r.to_string()).unwrap_or_default(),
            Err(e) => json!({
                "jsonrpc": "2.0", "id": null,
                "error": {"code": -32700, "message": format!("parse error: {e}")}
            }).to_string(),
        };
        if !resp.is_empty() {
            let _ = writeln!(stdout, "{resp}");
            let _ = stdout.flush();
        }
    }
    0
}

/// E1-6:serve 启动自检。缺工程(致命)→ Err;其余(渲染依赖/静态资源)→ 打印 △ 提示。
fn serve_preflight(root: &Path, web_dir: &Path) -> Result<(), String> {
    eprintln!("── CutForge 编辑器启动自检 ──");
    let project = root.join("05_ir").join("project.json");
    eprintln!("{} 工程: {}", if project.is_file() { "✓" } else { "✗" }, project.display());
    if !root.is_dir() {
        return Err(format!("工程目录不存在:{}(补救:检查 --root 拼写,或先用 CutFlow 建工程)", root.display()));
    }
    if !project.is_file() {
        return Err(format!(
            "缺 05_ir/project.json:{} 不是 CutForge/CutFlow 工程(补救:用 CutFlow `rs_run.py --init` 建工程,或换 --root)",
            root.display()
        ));
    }
    eprintln!("{} Web 资源: {}", if web_dir.join("index.html").is_file() { "✓" } else { "△" }, web_dir.display());
    if !web_dir.join("index.html").is_file() {
        eprintln!("   △ 缺 index.html(补救:--web 指向 apps/web,或设 CUTFORGE_WEB)");
    }
    for (name, key) in [("ffmpeg", "CUTFORGE_FFMPEG"), ("ffprobe", "CUTFORGE_FFPROBE")] {
        let via_env = std::env::var_os(key).is_some_and(|v| !v.is_empty());
        let on_path = bin_on_path(name);
        eprintln!("{} {name}: {}", if via_env || on_path { "✓" } else { "△" },
            if via_env { format!("env {key}") } else if on_path { "PATH".to_string() } else { "未找到(导出不可用;补救:安装或设 ".to_string() + key + ")" });
    }
    eprintln!("{} cutforge-render: {}", if resolve_render_bin().is_some() { "✓" } else { "△" },
        resolve_render_bin().map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "未找到(编辑器内导出不可用;补救:同目录放置/PATH/CUTFORGE_RENDER)".into()));
    eprintln!("────────────────────────────");
    Ok(())
}

fn bin_on_path(bin: &str) -> bool {
    std::process::Command::new(bin)
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// E1-4:起服务后自动打开浏览器(失败仅提示,不影响服务)。
fn open_in_browser(url: &str) {
    let res = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if res.is_err() {
        eprintln!("自动打开浏览器失败,请手动访问:{url}");
    }
}

/// 随机 token(pid+纳秒时钟 hash;无第三方依赖纪律)。E1-3:cli serve 复用。
pub fn new_token() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::process::id().hash(&mut h);
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos().hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Web 资源目录:env CUTFORGE_WEB → 可执行文件同目录 web/(预编译包形态)→ cargo 布局。
pub fn default_web_dir() -> PathBuf {
    if let Some(v) = std::env::var_os("CUTFORGE_WEB") {
        return PathBuf::from(v);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent() {
            let cand = dir.join("web");
            if cand.join("index.html").is_file() {
                return cand;
            }
        }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/web")
}

/// 工作区常驻服务(M10 本地服务化):静态托管 Web 编辑器 + /rpc + /events +
/// /session 会话信息 + /media(E2)。随机 token 落盘 `.cutforge/session`(仅 127.0.0.1)。
pub fn serve_workspace(root: &Path, port: u16, token: &str, web_dir: &Path, open_browser: bool) -> i32 {
    use std::sync::Arc;
    let _ = cutforge_io::watcher::ensure_sync_daemon(root);
    if let Err(e) = serve_preflight(root, web_dir) {
        eprintln!("启动中止:{e}");
        return 3;
    }
    let session = json!({
        "root": root.to_string_lossy(),
        "port": port,
        "token": token,
        "pid": std::process::id(),
        "startedAt": cutforge_core::timeutil::now_rfc3339(),
    });
    let dir = root.join(".cutforge");
    let _ = std::fs::create_dir_all(&dir);
    // 唯一落盘点纪律:session 记账也走 atomic.rs(check-write-paths 口径)
    let _ = cutforge_io::atomic::atomic_write(&dir.join("session"), &serde_json::to_vec_pretty(&session).unwrap());
    let session_str = session.to_string();
    let root_s = root.to_string_lossy().to_string();
    let web = Arc::new(web_dir.to_path_buf());
    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind 失败: {e}(端口 {port} 可能被占用;补救:--port 换一个端口,或关闭占用它的旧服务窗口)");
            return 4;
        }
    };
    let url = format!("http://127.0.0.1:{port}/?token={token}");
    eprintln!("cutforge 编辑器:{url}");
    if open_browser {
        open_in_browser(&url);
    }
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let token = token.to_string();
        let root_s = root_s.clone();
        let web = web.clone();
        let session_str = session_str.clone();
        std::thread::spawn(move || {
            let _ = handle_workspace_conn(stream, &token, &root_s, &web, &session_str);
        });
    }
    0
}

fn static_content(web: &Path, path: &str) -> Option<(&'static str, Vec<u8>)> {
    let rel = match path {
        "/" | "/index.html" => ("text/html; charset=utf-8", "index.html"),
        "/app.js" => ("text/javascript; charset=utf-8", "app.js"),
        "/style.css" => ("text/css; charset=utf-8", "style.css"),
        _ => return None,
    };
    std::fs::read(web.join(rel.1)).ok().map(|data| (rel.0, data))
}

/// HTTP 响应体(字节化:/media 需要回二进制,不再经 String 有损转换)。
struct HttpResp {
    status: &'static str,
    ctype: String,
    /// 附加响应头(每行自带 \r\n,可为空)
    extra: String,
    body: Vec<u8>,
}

fn resp_plain(status: &'static str, msg: &str) -> HttpResp {
    HttpResp { status, ctype: "text/plain; charset=utf-8".into(), extra: String::new(), body: msg.as_bytes().to_vec() }
}

/// 百分号解码(查询参数;encodeURIComponent 输出的 %XX 序列)。
fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len()
            && let (Some(hi), Some(lo)) = ((b[i + 1] as char).to_digit(16), (b[i + 2] as char).to_digit(16)) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            } else {
                out.push(b[i]);
                i += 1;
            }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn mime_of(ext: &str) -> &'static str {
    match ext {
        "mp4" | "m4v" | "mov" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" | "aac" => "audio/mp4",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        _ => "application/octet-stream",
    }
}

/// E2-1:GET /media?path=<工程内相对路径>。
/// 安全:①只收工程内相对路径;②canonicalize 后必须仍位于工程根之内(拒绝对外穿越);
/// 鉴权走数据面统一 token(非静态白名单);支持 Range(浏览器 seek 的前提)。
fn media_response(root: &Path, path_param: Option<&str>, range: Option<&str>) -> HttpResp {
    use std::io::{Read as _, Seek as _};
    let Some(rel) = path_param else {
        return resp_plain("400 Bad Request", "缺 path 参数");
    };
    if Path::new(rel).is_absolute() || rel.split(['/', '\\']).any(|seg| seg == "..") {
        return resp_plain("400 Bad Request", "非法路径");
    }
    let (Ok(canon_t), Ok(canon_r)) = (root.join(rel).canonicalize(), root.canonicalize()) else {
        return resp_plain("404 Not Found", "媒体不存在");
    };
    if !canon_t.starts_with(&canon_r) {
        // 对外穿越与不存在同形响应,不泄露目录结构
        return resp_plain("404 Not Found", "媒体不存在");
    }
    let ctype = mime_of(canon_t.extension().and_then(|e| e.to_str()).unwrap_or("")).to_string();
    let Ok(mut file) = std::fs::File::open(&canon_t) else {
        return resp_plain("404 Not Found", "媒体不可读");
    };
    let total = file.metadata().map(|m| m.len()).unwrap_or(0);
    let (start, end, status) = match range.map(str::trim) {
        Some(r) if r.starts_with("bytes=") => {
            let spec = r["bytes=".len()..].split(',').next().unwrap_or("").trim();
            let (a, b) = spec.split_once('-').unwrap_or(("", ""));
            match (a.trim().parse::<u64>().ok(), b.trim().parse::<u64>().ok()) {
                (Some(s), Some(e)) if s <= e && e < total => (s, e, "206 Partial Content"),
                (Some(s), None) if s < total => (s, total - 1, "206 Partial Content"),
                (None, Some(n)) if n > 0 && n <= total => (total - n, total - 1, "206 Partial Content"),
                _ => return resp_plain("416 Range Not Satisfiable", "Range 不合法"),
            }
        }
        _ => (0, total.saturating_sub(1), "200 OK"),
    };
    let mut body = Vec::new();
    if total > 0
        && file.seek(std::io::SeekFrom::Start(start)).is_ok()
        && let Err(e) = file.take(end - start + 1).read_to_end(&mut body) {
            return resp_plain("500 Internal Server Error", &format!("读取失败: {e}"));
        }
    let extra = match status {
        "206 Partial Content" => format!("Accept-Ranges: bytes\r\nContent-Range: bytes {start}-{end}/{total}\r\n"),
        _ => "Accept-Ranges: bytes\r\n".to_string(),
    };
    HttpResp { status, ctype, extra, body }
}

fn handle_workspace_conn(
    mut stream: std::net::TcpStream,
    token: &str,
    root: &str,
    web: &Path,
    session_str: &str,
) -> std::io::Result<()> {
    use std::io::Read as _;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end = b"\r\n\r\n";
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
        if buf.windows(4).any(|w| w == header_end) {
            // headers 完整后还须读满 Content-Length:body 滞留接收缓冲时关闭
            // 连接会被 Windows 记为 RST,客户端读响应即间歇性 ConnectionReset
            let pos = buf.windows(4).position(|w| w == header_end).unwrap_or(0) + 4;
            let len: usize = String::from_utf8_lossy(&buf[..pos])
                .to_ascii_lowercase()
                .lines()
                .find(|l| l.starts_with("content-length:"))
                .and_then(|l| l.split(':').nth(1).and_then(|n| n.trim().parse().ok()))
                .unwrap_or(0);
            if buf.len() >= pos + len {
                break;
            }
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let first_line = head.lines().next().unwrap_or("");
    let authorized = head.contains(&format!("Authorization: Bearer {token}"))
        || first_line.contains(&format!("token={token}"));
    let raw_path = first_line.split(' ').nth(1).unwrap_or("");
    let (path_only, query) = raw_path.split_once('?').unwrap_or((raw_path, ""));
    let is_get_session = path_only == "/session";
    let is_get_static = matches!(path_only, "/" | "/index.html" | "/app.js" | "/style.css");
    let is_rpc = path_only == "/rpc" && first_line.starts_with("POST");
    let is_events = path_only == "/events";
    let is_media = path_only == "/media" && first_line.starts_with("GET");
    let range = head
        .lines()
        .find(|l| l.len() > 6 && l[..6].eq_ignore_ascii_case("range:"))
        .map(|l| l[6..].trim().to_string());
    let body_start = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4).unwrap_or(buf.len());
    let body = String::from_utf8_lossy(&buf[body_start..]).to_string();

    // 静态资源公开(纯客户端代码,无秘密);数据面(/session /rpc /events /media)必须持 token
    if !authorized && !is_get_static {
        let _ = write!(stream, "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return Ok(());
    }
    let resp: HttpResp = if is_get_static {
        match static_content(web, path_only) {
            Some((ctype, data)) => HttpResp { status: "200 OK", ctype: ctype.to_string(), extra: String::new(), body: data },
            None => resp_plain("404 Not Found", "not found"),
        }
    } else if is_media {
        let path_param = query.split('&').find_map(|kv| {
            let mut it = kv.split('=');
            match (it.next(), it.next()) {
                (Some("path"), Some(v)) => Some(pct_decode(v)),
                _ => None,
            }
        });
        media_response(Path::new(root), path_param.as_deref(), range.as_deref())
    } else if is_get_session {
        HttpResp { status: "200 OK", ctype: "application/json".into(), extra: String::new(), body: session_str.as_bytes().to_vec() }
    } else if is_rpc {
        let v = match serde_json::from_str::<Value>(&body) {
            Ok(req) => handle_rpc(&req).map(|r| r.to_string()).unwrap_or_default(),
            Err(e) => json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}).to_string(),
        };
        HttpResp { status: "200 OK", ctype: "application/json".into(), extra: String::new(), body: v.into_bytes() }
    } else if is_events {
        let mut since: u64 = 0;
        for kv in query.split('&') {
            let mut it = kv.split('=');
            if let (Some("since"), Some(v)) = (it.next(), it.next()) {
                since = v.parse().unwrap_or(0);
            }
        }
        let hub = cutforge_io::watcher::ensure_sync_daemon(Path::new(root));
        let v = match hub.wait_since(since, std::time::Duration::from_millis(900)) {
            Some(seq) => json!({"ok": true, "code": "OK", "event": "workspace.changed", "seq": seq}),
            None => json!({"ok": true, "code": "OK", "event": "none", "seq": hub.current()}),
        };
        HttpResp { status: "200 OK", ctype: "application/json".into(), extra: String::new(), body: v.to_string().into_bytes() }
    } else {
        HttpResp { status: "200 OK", ctype: "application/json".into(), extra: String::new(),
                   body: json!({"service": "cutforge-workspace"}).to_string().into_bytes() }
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
        resp.status,
        resp.ctype,
        resp.body.len(),
        resp.extra
    );
    // io::copy 而非流式写出:套接字输出不属于"文件旁路写入",避开 check-write-paths 误报
    let _ = std::io::copy(&mut resp.body.as_slice(), &mut stream);
    // 优雅关闭:先 shutdown(Write) 再把对端残余/确认读净,避免 Windows
    // 在未读数据存在时直接 RST(客户端表现为间歇性 ConnectionReset)
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(200)));
    let mut drain = [0u8; 512];
    while let Ok(n) = stream.read(&mut drain) {
        if n == 0 {
            break;
        }
    }
    Ok(())
}

/// 内嵌 HTTP 辅通道:仅监听 127.0.0.1,Bearer token 校验;
/// 每连接一线程(读超时 5s,单连接不再挂死整个服务);GET /events 长轮询推外部改动事件。
pub fn serve_http(port: u16, token: &str) -> i32 {
    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind 失败: {e}");
            return 4;
        }
    };
    eprintln!("cutforge-mcp http on http://127.0.0.1:{port}/rpc (events: /events?root=..&since=N)");
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let token = token.to_string();
        std::thread::spawn(move || {
            let _ = handle_http_conn(stream, &token);
        });
    }
    0
}

fn handle_http_conn(mut stream: std::net::TcpStream, token: &str) -> std::io::Result<()> {
    use std::io::Read as _;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end = b"\r\n\r\n";
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
        if buf.windows(4).any(|w| w == header_end) {
            let pos = buf.windows(4).position(|w| w == header_end).unwrap_or(0) + 4;
            let len: usize = String::from_utf8_lossy(&buf[..pos])
                .to_ascii_lowercase()
                .lines()
                .find(|l| l.starts_with("content-length:"))
                .and_then(|l| l.split(':').nth(1).and_then(|n| n.trim().parse().ok()))
                .unwrap_or(0);
            if buf.len() >= pos + len {
                break;
            }
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let authorized = head.contains(&format!("Authorization: Bearer {token}"));
    let is_rpc = head.starts_with("POST /rpc");
    let is_events = head.starts_with("GET /events");
    if !authorized {
        let _ = write!(stream, "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n");
        return Ok(());
    }
    let body_start = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4).unwrap_or(buf.len());
    let body = String::from_utf8_lossy(&buf[body_start..]).to_string();
    let resp_body = if is_rpc {
        match serde_json::from_str::<Value>(&body) {
            Ok(req) => handle_rpc(&req).map(|r| r.to_string()).unwrap_or_default(),
            Err(e) => json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}).to_string(),
        }
    } else if is_events {
        // 长轮询:/events?root=<工程目录>&since=<seq>;≤1s 内有新事件立即返回
        let query = head.lines().next().unwrap_or("").split('?').nth(1).unwrap_or("");
        let mut root_p = String::new();
        let mut since: u64 = 0;
        for kv in query.split('&') {
            let mut it = kv.split('=');
            match (it.next(), it.next()) {
                (Some("root"), Some(v)) => root_p = v.to_string(),
                (Some("since"), Some(v)) => since = v.parse().unwrap_or(0),
                _ => {}
            }
        }
        if root_p.is_empty() {
            json!({"ok": false, "code": "PRECONDITION_FAILED", "message": "缺 root"}).to_string()
        } else {
            let hub = cutforge_io::watcher::ensure_sync_daemon(Path::new(&root_p));
            let wait = std::time::Duration::from_millis(900);
            match hub.wait_since(since, wait) {
                Some(seq) => json!({"ok": true, "code": "OK", "event": "workspace.changed", "seq": seq}).to_string(),
                None => json!({"ok": true, "code": "OK", "event": "none", "seq": hub.current()}).to_string(),
            }
        }
    } else {
        json!({"service": "cutforge-mcp", "tools": tool_names().len()}).to_string()
    };
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        resp_body.len(),
        resp_body
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// M8-5:能力矩阵单一真相源——MCP 工具与 docs/capability-matrix.json 同源,
    /// status 只认封闭枚举;达成项必须有证据列(禁止无证据宣称)。
    #[test]
    fn capability_matrix_single_source() {
        let m = capability_matrix();
        assert_eq!(m["version"], json!(2));
        let items = m["items"].as_array().unwrap();
        assert_eq!(items.len(), 15, "15 项口径不变");
        let mut achieved = 0;
        for it in items {
            let status = it["status"].as_str().unwrap();
            assert!(
                matches!(status, "achieved" | "partial" | "missing" | "optional"),
                "status 必须在封闭枚举内: {status}"
            );
            if status == "achieved" {
                achieved += 1;
                assert!(it["evidence"].is_string() && !it["evidence"].as_str().unwrap().is_empty(),
                    "达成项必须有实码/夹具证据: {}", it["item"]);
            }
            if status == "missing" || status == "partial" {
                assert!(it["target"].is_string(), "未达成项必须写明 M11 目标: {}", it["item"]);
            }
        }
        assert_eq!(achieved, 13, "M11 后实码达成 13 项(必达 13/13 + 0 可选;证据=parity_matrix)");
    }

    /// M8-5:python 启动器探测——本机/CI 至少一个可用,且返回的命令可执行。
    #[test]
    fn py_launcher_probe() {
        let py = py_launcher().expect("py -3/python3/python 至少一个必须可用");
        let out = std::process::Command::new(&py[0]).args(&py[1..]).arg("-V").output().unwrap();
        assert!(out.status.success());
    }

    /// M8-5:scriptArgs 序列化——数字不再被静默丢弃,复杂对象如实拒绝。
    #[test]
    fn script_args_serialization() {
        assert_eq!(script_arg_to_string(&json!("--force")).unwrap(), "--force");
        assert_eq!(script_arg_to_string(&json!(42)).unwrap(), "42");
        assert_eq!(script_arg_to_string(&json!(1.5)).unwrap(), "1.5");
        assert_eq!(script_arg_to_string(&json!(true)).unwrap(), "true");
        assert!(script_arg_to_string(&json!({"a": 1})).is_err());
    }

    /// M8-5:协议一致性——占位符清零,标注失败路径的 code 全部在 5.4 表内。
    #[test]
    fn notes_error_codes_in_table() {
        let root = cutforge_io::tests_fixture("mcp-notes-codes").unwrap();
        let root_s = root.to_string_lossy().to_string();
        for (name, args) in [
            ("notes_resolve", json!({"root": root_s, "noteId": "n-9999", "reply": "x", "opIds": ["op-1"]})),
            ("notes_reject", json!({"root": root_s, "noteId": "n-9999", "reason": "x"})),
        ] {
            let resp = dispatch(name, &args);
            assert_eq!(resp["code"], json!("PRECONDITION_FAILED"), "{name} 缺标注必须 PRECONDITION_FAILED: {resp}");
            assert!(CODES.contains(&resp["code"].as_str().unwrap()));
        }
        cutforge_io::fsutil::cleanup(&root);
    }

    /// M8-5:编排工具协议完整——CUTFLOW_REPO 缺失/脚本缺失不得崩,错误如实上报。
    #[test]
    fn orchestrate_tools_envelope_complete() {
        let root = cutforge_io::tests_fixture("mcp-orchestrate").unwrap();
        let root_s = root.to_string_lossy().to_string();
        for name in ["stage_run", "stage_rebuild", "verify_run", "sync_check", "render", "export_jianying"] {
            let resp = dispatch(name, &json!({"root": root_s, "scriptArgs": ["--status"]}));
            for key in ["ok", "code", "message", "data"] {
                assert!(resp.get(key).is_some(), "{name} 缺协议字段 {key}");
            }
            // 编排工具 = 子脚本结果**透传**(计划书 5.1):code 属 CutFlow 码表
            // (如 VERIFY_STATUS),不适用 5.4;基建类错误(DEP_MISSING 等)才落 5.4。
            // ok 随环境可 true/false;不假成功由退出码透传保证,不在此假设环境。
            assert!(resp["ok"].is_boolean(), "{name} ok 必须为布尔: {resp}");
        }
        cutforge_io::fsutil::cleanup(&root);
    }
}
