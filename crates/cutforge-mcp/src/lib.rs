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
            let dir = ws_root.join("_state");
            let mut map = serde_json::Map::new();
            for s in cutforge_io::stage::STAGES {
                map.insert(s.to_string(), json!(dir.join(format!("{s}.json")).is_file()));
            }
            envelope(true, "OK", "阶段状态(_state 存在性)", Value::Object(map))
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
        "capability_matrix" => envelope(true, "OK", "能力对等矩阵(ADR-0038)", json!({
            "matrix": capability_matrix(),
            "rule": "必达项全达成且整体达成率 ≥90% 才通过(M6-5)",
        })),

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
            let kind = match anchor_v["kind"].as_str().unwrap_or("clip") {
                "track" => AnchorKind::Track,
                "time" => AnchorKind::Time,
                "word" => AnchorKind::Word,
                "subtitleCard" => AnchorKind::SubtitleCard,
                _ => AnchorKind::Clip,
            };
            let anchor = Anchor {
                kind,
                ref_: anchor_v["ref"].as_str().map(String::from),
                t_ms: anchor_v["tMs"].as_u64().unwrap_or(0),
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
    let after = merge_patch(before.clone(), patch);
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
                    return Some(cand.iter().map(|s| s.to_string()).collect());
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

fn orchestrate(ws_root: &Path, script: &str, script_args: &[Value]) -> Value {
    let Some(cutflow) = resolve_cutflow_dir(ws_root) else {
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

/// 内嵌 HTTP 辅通道:仅监听 127.0.0.1,Bearer token 校验,POST /rpc。
pub fn serve_http(port: u16, token: &str) -> i32 {
    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind 失败: {e}");
            return 4;
        }
    };
    eprintln!("cutforge-mcp http on http://127.0.0.1:{port}/rpc");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        // 读取到请求头结束,再按 Content-Length 读 body
        let header_end = b"\r\n\r\n";
        while let Ok(n) = std::io::Read::read(&mut stream, &mut tmp) {
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
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
        if !authorized {
            let _ = write!(stream, "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n");
            continue;
        }
        let pos = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4).unwrap_or(buf.len());
        let body = String::from_utf8_lossy(&buf[pos..]).to_string();
        let resp_body = if !is_rpc {
            json!({"service": "cutforge-mcp", "tools": tool_names().len()}).to_string()
        } else {
            match serde_json::from_str::<Value>(&body) {
                Ok(req) => handle_rpc(&req).map(|r| r.to_string()).unwrap_or_default(),
                Err(e) => json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}).to_string(),
            }
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            resp_body.len(),
            resp_body
        );
    }
    0
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
        assert_eq!(achieved, 7, "M8 后实码达成 7 项(降级重写后的诚实口径)");
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
            assert!(CODES.contains(&resp["code"].as_str().unwrap()), "{name} code 不在 5.4 表: {resp}");
            assert_eq!(resp["ok"], json!(false), "夹具工程无真实脚本,必须失败而非假成功");
        }
        cutforge_io::fsutil::cleanup(&root);
    }
}
