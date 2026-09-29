// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 单一派发表:所有通道(stdio/HTTP/脚本宿主)共用的工具分派、JSON-RPC 面、
//! 参数解析与 5.4 错误映射(T1.1 拆分自 lib.rs,纯移动)。

use crate::edit_ops;
use crate::orchestrate::orchestrate;
use crate::progress::{existing_rel, render_cutforge_sync, render_frame_tool, render_progress, render_run_async};
use crate::registry::{capability_matrix, envelope, registry, tool_def};
use crate::tools_nolock::{media_browse_tool, media_probe_tool, project_new_tool, render_probe_tool, stage_status_tool};
use cutforge_core::anchor::{Anchor, AnchorKind};
use cutforge_core::command::{BgmPatch, ClipPatch, Command, MotionPatch, TransitionPatch};
use cutforge_core::engine::{Answer, ApplyOpts, Query};
use cutforge_core::oplog::{Actor, ActorKind};
use cutforge_io::paths;
use cutforge_io::Workspace;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

// ---------------- 派发 ----------------

/// 命令通道统一派发:所有通道(stdio/HTTP/脚本宿主)都走这里。
/// root(工程目录)由 args["root"] 提供——工具契约的第一参数。
/// stdio/内嵌 HTTP/脚本宿主的改动归因 agent;编辑器数据面归因 user(见 dispatch_with_actor)。
pub fn dispatch(name: &str, args: &Value) -> Value {
    dispatch_with_actor(name, args, Actor::agent("cutforge-mcp"))
}

/// 带显式 actor 的派发。工作区数据面(编辑器壳)传 `Actor::user("editor")`,
/// 让"人在编辑器里的手势"在 OpLog 上如实归因——RT-1 会话变更摘要
/// (.cutforge/session-summary.json)按 actor=human 过滤的依据。
/// 其余通道(stdio/内嵌 HTTP/脚本宿主)维持 agent 归因不变。
pub fn dispatch_with_actor(name: &str, args: &Value, actor: Actor) -> Value {
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
    // T2.4 单帧精确预览:同步出帧(帧缓存键含工作区指纹),免开工作区不持锁
    if name == "render_frame" {
        return render_frame_tool(&ws_root, args);
    }

    // ---- 免开工作区的工具(E6-3/B14:只读/创建类不持排他锁) ----
    match name {
        "project_new" => return project_new_tool(&ws_root, args),
        "media_probe" => return media_probe_tool(&ws_root, args),
        "media_browse" => return media_browse_tool(&ws_root, args),
        "render_probe" => return render_probe_tool(&ws_root),
        "stage_status" => return stage_status_tool(&ws_root),
        _ => {}
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

    // 常驻同步守护(M9-2):外部改动 ≤1s 可见;幂等(每 root 一个线程)
    let _ = cutforge_io::watcher::ensure_sync_daemon(&ws_root);
    // 常驻工作区缓存(T1.8/AC-1.8 性能专项):指纹一致 → 复用已打开的 Workspace,
    // 指纹不一致 → 重开(与既有的每笔无状态重开行为一致)。查询类仍只读零工程锁
    // (readonly_query_holds_no_lock 铁律);写类经 open_for_write + apply 内部
    // 临时全程锁,锁内 pre_write_sync 三路合并/冲突停写语义原样保留。
    let readonly = is_readonly_tool(name);
    crate::resident::with_resident(root_str, &ws_root, readonly, |ws| match name {
        // ---------- 只读查询(E6-3:只读打开,不持排他锁) ----------
        "project_get" => match ws.engine().query(Query::ProjectView) {
            Answer::Project(v) => envelope(true, "OK", "工程视图", json!({"project": v, "rev": ws.rev()})),
            _ => unreachable!(),
        },
        "wordline_get" => read_truth(
            &paths::resolve_rel(&ws_root, paths::WORDLINE_REL, paths::LEGACY_WORDLINE_REL),
            "wordline",
        ),
        "cutlist_get" => {
            let applied = args["applied"].as_bool().unwrap_or(false);
            let (rel, legacy) = if applied {
                (paths::CUTLIST_APPLIED_REL, paths::LEGACY_CUTLIST_APPLIED_REL)
            } else {
                (paths::CUTLIST_REL, paths::LEGACY_CUTLIST_REL)
            };
            read_truth(&paths::resolve_rel(&ws_root, rel, legacy), "cutlist")
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
        "timeline_get" => {
            // E2-2:扩投影——预览/检查器所需的逐 clip 字段全部由内核算好下放
            // (endMs = start+duration 在服务端完成;壳只消费,不做时间线运算)。
            envelope(true, "OK", "时间线投影", json!({
                "clips": timeline_projection(ws.project()),
                "rev": ws.rev(),
            }))
        }

        // ---------- 写操作(全部经 Workspace 命令通道) ----------
        "clip_add" => {
            // E3-1:素材导入/新建片段——内部走已存在的 Command::ClipInsert,不新增引擎逻辑;
            // E3-2:durationMs 缺省时由 cutforge_io::probe 探测时长自动填(B12 接线)。
            let (Some(track_id), Some(src), Some(start_ms)) = (
                args["trackId"].as_str(), args["src"].as_str(), args["startMs"].as_u64(),
            ) else {
                return envelope(false, "PRECONDITION_FAILED", "缺 trackId/src/startMs", json!({}));
            };
            if ws.project().find_track(track_id).is_none() {
                return envelope(false, "PRECONDITION_FAILED", &format!("track 不存在: {track_id}"), json!({}));
            }
            // 素材路径复用 /media 的 canonicalize 校验(不建并行实现;E3 风险面对策)
            if let Err(msg) = resolve_within_root(&ws_root, src) {
                return envelope(false, "PRECONDITION_FAILED", &format!("素材路径不合法({src}): {msg}"), json!({}));
            }
            let source_in = args["sourceInMs"].as_u64().unwrap_or(0);
            let duration_ms = match args["durationMs"].as_u64() {
                Some(d) => d,
                None => {
                    if !cutforge_io::probe::ffprobe_available() {
                        return envelope(false, "DEP_MISSING",
                            "durationMs 缺省且 ffprobe 不可用:显式给 durationMs,或安装 ffprobe / 设 CUTFORGE_FFPROBE", json!({}));
                    }
                    match cutforge_io::probe::probe(&ws_root.join(src)) {
                        Ok(info) => (info.duration_ms().saturating_sub(source_in)).max(1),
                        Err(e) => return envelope(false, "DEP_MISSING", &format!("媒体时长探测失败: {e}"), json!({})),
                    }
                }
            };
            let clip_id = next_clip_id_for(ws.project(), track_id);
            let mut clip_json = json!({
                "id": clip_id, "src": src, "startMs": start_ms,
                "durationMs": duration_ms, "sourceInMs": source_in,
            });
            if let Some(v) = args["volume"].as_f64() {
                clip_json["volume"] = json!(v);
            }
            let clip: cutforge_core::model::Clip = match serde_json::from_value(clip_json) {
                Ok(c) => c,
                Err(e) => return envelope(false, "SCHEMA_INVALID", &e.to_string(), json!({})),
            };
            let request_id = args["requestId"].as_str().map(String::from);
            finish_apply(ws.apply(Command::ClipInsert { to_track: track_id.into(), clip, request_id }, actor, opts))
        }
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
                // 嵌套子 patch:对象缺席/显式 null 均为"不改";给出则按字段合并
                transition: p.get("transition").filter(|t| t.is_object()).map(|t| TransitionPatch {
                    type_: t["type"].as_str().map(String::from),
                    dur_ms: t["durMs"].as_f64(),
                    reason: t["reason"].as_str().map(String::from),
                    fx: t["fx"].as_str().map(String::from),
                }),
                motion: p.get("motion").filter(|t| t.is_object()).map(|t| MotionPatch {
                    in_: t["in"].as_str().map(String::from),
                    in_ms: t["inMs"].as_f64(),
                    out: t["out"].as_str().map(String::from),
                    out_ms: t["outMs"].as_f64(),
                }),
            };
            finish_apply(ws.apply(Command::ClipUpdate { clip_id: clip_id.into(), patch }, actor, opts))
        }
        "transition_set" => {
            // 设置片段转场(clip.transition):type 必给(durMs/fx/reason 可选,按字段合并);
            // 显式硬切/关闭用 type="cut"/"none"(schema 语义),枚举外值由 schema 层拒。
            let Some(clip_id) = args["clipId"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({}));
            };
            let Some(t) = args["type"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 type(fade/wipeleft/wipeup/slideleft/circleopen/cut/none)", json!({}));
            };
            let patch = ClipPatch {
                transition: Some(TransitionPatch {
                    type_: Some(t.into()),
                    dur_ms: args["durMs"].as_f64(),
                    reason: args["reason"].as_str().map(String::from),
                    fx: args["fx"].as_str().map(String::from),
                }),
                ..Default::default()
            };
            finish_apply(ws.apply(Command::ClipUpdate { clip_id: clip_id.into(), patch }, actor, opts))
        }
        "motion_set" => {
            // 设置片段入场/出场动效(clip.motion):至少给 in/inMs/out/outMs 之一,按字段合并
            let Some(clip_id) = args["clipId"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({}));
            };
            let motion = MotionPatch {
                in_: args["in"].as_str().map(String::from),
                in_ms: args["inMs"].as_f64(),
                out: args["out"].as_str().map(String::from),
                out_ms: args["outMs"].as_f64(),
            };
            if motion.is_empty() {
                return envelope(false, "PRECONDITION_FAILED", "motion_set 至少给 in/inMs/out/outMs 之一", json!({}));
            }
            let patch = ClipPatch { motion: Some(motion), ..Default::default() };
            finish_apply(ws.apply(Command::ClipUpdate { clip_id: clip_id.into(), patch }, actor, opts))
        }
        "bgm_set" => {
            // 工程级背景乐(doc.bgm;不经 ClipPatch):src 为字符串=设置/合并;
            // src 显式 null=清除;src 缺省=仅调 gainDb/ducking/loop(工程尚无 bgm 时须先给 src)。
            if args.get("src").is_some_and(Value::is_null) {
                return finish_apply(ws.apply(Command::BgmClear, actor, opts));
            }
            let patch = BgmPatch {
                src: args["src"].as_str().map(String::from),
                gain_db: args["gainDb"].as_f64(),
                ducking: args["ducking"].as_bool(),
                loop_: args["loop"].as_bool(),
            };
            if patch.is_empty() {
                return envelope(false, "PRECONDITION_FAILED",
                    "bgm_set 至少给 src/gainDb/ducking/loop 之一(清除背景乐用 src:null)", json!({}));
            }
            // 音源路径与 clip_add 同一校验(不建并行实现)
            if let Some(src) = patch.src.as_deref()
                && let Err(msg) = resolve_within_root(&ws_root, src) {
                    return envelope(false, "PRECONDITION_FAILED", &format!("bgm 路径不合法({src}): {msg}"), json!({}));
                }
            finish_apply(ws.apply(Command::BgmSet { patch }, actor, opts))
        }
        "clip_split" => match args["clipId"].as_str().zip(args["tMs"].as_u64()) {
            Some((clip_id, t_ms)) => finish_apply(ws.apply(Command::ClipSplit { clip_id: clip_id.into(), t_ms }, actor, opts)),
            None => envelope(false, "PRECONDITION_FAILED", "缺 clipId/tMs", json!({})),
        },
        "clip_delete" => match args["clipId"].as_str() {
            Some(clip_id) => finish_apply(ws.apply(Command::ClipDelete { clip_id: clip_id.into() }, actor, opts)),
            None => envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({})),
        },
        "clip_move" => {
            let (Some(clip_id), Some(start_ms)) = (args["clipId"].as_str(), args["startMs"].as_u64()) else {
                return envelope(false, "PRECONDITION_FAILED", "缺 clipId/startMs", json!({}));
            };
            finish_apply(ws.apply(
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
            finish_apply(ws.apply(Command::ClipInsert { to_track, clip, request_id }, actor, opts))
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
            finish_apply(ws.apply(Command::TrackAdd { kind, request_id }, actor, opts))
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
            finish_apply(ws.apply(Command::ClipUpdate { clip_id: clip_id.into(), patch }, actor, opts))
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
            finish_apply(ws.apply(Command::ClipInsert { to_track: track_id.into(), clip, request_id }, actor, opts))
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
            finish_apply(ws.apply(Command::ClipInsert { to_track: track_id, clip, request_id }, actor, opts))
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
            apply_cut_merge_patch(ws, &Value::Object(patch))
        }
        "undo" | "redo" => {
            let batch = args["batch"].as_u64().unwrap_or(1);
            let mut last = envelope(true, "OK", "无操作", json!({}));
            for _ in 0..batch {
                last = if name == "undo" {
                    finish_apply(ws.undo(actor.clone()))
                } else {
                    finish_apply(ws.redo(actor.clone()))
                };
                if last["ok"] != json!(true) {
                    break;
                }
            }
            last
        }

        // ---------- 时间线编辑全工具(册四 A4 T4.2;实现集中在 edit_ops) ----------
        "clip_trim" => edit_ops::clip_trim_tool(ws, args, &actor, opts),
        "clip_split_all" => edit_ops::clip_split_all_tool(ws, args, &actor, opts),
        "track_update" => edit_ops::track_update_tool(ws, args, &actor, opts),
        "clip_gap_delete" => edit_ops::clip_gap_delete_tool(ws, args, &actor, opts),
        "clip_copy" => edit_ops::clip_copy_tool(ws, root_str, args),
        "clip_paste_at" => edit_ops::clip_paste_at_tool(ws, root_str, args, &actor, opts),

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
    })
}

pub(crate) fn finish_apply(r: Result<cutforge_core::engine::OpReceipt, std::io::Error>) -> Value {
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

/// 真相源文件读取(wordline/cutlist;`root` 已按 paths 双布局解析到具体文件)。
fn read_truth(p: &Path, label: &str) -> Value {
    match std::fs::read_to_string(p) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(v) => envelope(true, "OK", label, json!({label.to_string().replace('-', "_"): v})),
            Err(e) => envelope(false, "SCHEMA_INVALID", &e.to_string(), json!({})),
        },
        Err(_) => envelope(false, "NO_CONFIG", &format!("文件不存在: {}", p.display()), json!({})),
    }
}

/// E6-3/B14:查询类工具集合——只读打开(Workspace::open),不申请排他锁。
/// 不在此列也不在免开工作区名单的工具 = 写操作,仍走 open_exclusive 全程锁。
fn is_readonly_tool(name: &str) -> bool {
    matches!(name,
        "project_get" | "wordline_get" | "cutlist_get" | "notes_list"
        | "oplog_tail" | "conflict_list" | "timeline_get")
}

/// RT-1:该工具成功返回 rev 即视为一次会话内变更(会话摘要的采集口径)。
/// 排除:只读查询、免开工作区的静态/编排类、工程创建(不产 rev)、
/// clip_copy(会话态剪贴板写入,不产 Op 不升 rev)。
pub(crate) fn produces_rev_mutation(name: &str) -> bool {
    !(is_readonly_tool(name)
        || matches!(name,
            "capability_matrix" | "project_new" | "render" | "render_run" | "render_progress"
            | "render_frame"
            | "media_probe" | "media_browse" | "render_probe" | "stage_status"
            | "clip_copy"))
}

/// E2-1 的 canonicalize 校验函数化:/media、/media/browse、clip_add、media_probe、
/// media_browse 共用同一份校验(相对路径、拒 `..`、canonicalize 后仍在工程根内),
/// 新端点禁止另造并行实现(阶段一不可回归面 + E3 风险面对策)。
pub(crate) fn resolve_within_root(root: &Path, rel: &str) -> Result<PathBuf, &'static str> {
    if rel.is_empty() {
        return Err("路径为空");
    }
    if Path::new(rel).is_absolute() || rel.split(['/', '\\']).any(|seg| seg == "..") {
        return Err("非法路径");
    }
    let Ok(canon_t) = root.join(rel).canonicalize() else {
        return Err("路径不存在或不可达");
    };
    let Ok(canon_r) = root.canonicalize() else {
        return Err("工程根不可达");
    };
    if !canon_t.starts_with(&canon_r) {
        return Err("路径越出工程根");
    }
    Ok(canon_t)
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
                "transition": c.transition,
                // E4-3 只读展示面:渲染已支持但 ClipPatch 未承接的分散字段,原样下放
                // (transition/motion 已于 ClipPatch 扩展后承接,不再列只读)
                "fade": c.fade,
                "punchIn": c.punch_in, "role": c.role,
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
    // cutlist 按 Workspace 盘面布局解析(新 04_粗剪决策 / 旧 04_cut)
    let rel = paths::resolve_rel(ws.root(), paths::CUTLIST_REL, paths::LEGACY_CUTLIST_REL);
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

// ---------------- JSON-RPC 传输(两通道共用 handle_rpc) ----------------

/// 处理一条 JSON-RPC 请求;通知(无 id)返回 None。
pub fn handle_rpc(req: &Value) -> Option<Value> {
    handle_rpc_as(req, Actor::agent("cutforge-mcp"))
}

/// 带显式 actor 的 JSON-RPC 处理:工作区数据面(编辑器壳)传 user,
/// 使人在编辑器里的手势在 OpLog 上如实归因(RT-1 会话摘要的采集依据)。
pub fn handle_rpc_as(req: &Value, actor: Actor) -> Option<Value> {
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
            let env = dispatch_with_actor(name, &args, actor);
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
