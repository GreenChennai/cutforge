//! CutForge CLI(计划书 2.1 接入层):人肉操作、CI、门禁脚本入口。
//! 全部输出 {ok, code, message, data} 结果协议;退出码 0/2/3/4。
//! 手写参数解析(不引第三方 CLI 框架,依赖纪律见 ADR-0034)。

use cutforge_core::command::{ClipPatch, Command};
use cutforge_core::engine::{ApplyOpts, Query};
use cutforge_core::oplog::{Actor, ActorKind};
use cutforge_io::Workspace;
use std::path::{Path, PathBuf};

const EXIT_OK: i32 = 0;
const EXIT_FAIL: i32 = 2;
const EXIT_ENV: i32 = 3;

fn emit(json: bool, ok: bool, code: &str, message: &str, data: serde_json::Value) -> i32 {
    let envelope = serde_json::json!({"ok": ok, "code": code, "message": message, "data": data});
    if json {
        println!("{envelope}");
    } else {
        println!("[{}] {code}: {message}", if ok { "PASS" } else { "FAIL" });
    }
    match code {
        "NO_ENV" => EXIT_ENV,
        "OK" => EXIT_OK,
        _ => EXIT_FAIL,
    }
}

struct Args {
    positional: Vec<String>,
    flags: std::collections::BTreeMap<String, String>,
    json: bool,
}

fn parse_args(args: &[String]) -> Args {
    let mut positional = Vec::new();
    let mut flags = std::collections::BTreeMap::new();
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--json" {
            json = true;
        } else if let Some(rest) = a.strip_prefix("--") {
            if let Some(eq) = rest.find('=') {
                flags.insert(rest[..eq].to_string(), rest[eq + 1..].to_string());
            } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                flags.insert(rest.to_string(), args[i + 1].clone());
                i += 1;
            } else {
                flags.insert(rest.to_string(), String::new());
            }
        } else {
            positional.push(a.clone());
        }
        i += 1;
    }
    Args { positional, flags, json }
}

fn actor_from(flag: Option<&String>) -> Actor {
    match flag.and_then(|s| s.split_once(':')) {
        Some(("user", id)) => Actor::user(id),
        Some(("script", id)) => Actor::script(id),
        Some(("agent", id)) => Actor::agent(id),
        _ => Actor::user("cli"),
    }
}

fn open_ws(root: &Path) -> Result<Workspace, String> {
    Workspace::open(root).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("工程不存在: {}({e})", root.display())
        } else {
            format!("打开失败: {e}")
        }
    })
}

fn repo_root() -> PathBuf {
    if let Some(r) = std::env::var_os("CUTFORGE_REPO") {
        return PathBuf::from(r);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn run(argv: Vec<String>) -> i32 {
    let Some(cmd) = argv.first().cloned() else {
        return emit(false, false, "USAGE", "用法: cutforge-cli <子命令> […]", serde_json::json!({
            "subcommands": ["project", "timeline", "clip", "clip-update", "split", "undo", "redo", "oplog", "check-write-paths", "check-deps"]
        }));
    };
    let args = parse_args(&argv[1..]);
    match cmd.as_str() {
        "project" => {
            let Some(root) = args.positional.first() else { return emit(false, args.json, "USAGE", "用法: project <工程目录>", serde_json::json!({})) };
            match open_ws(Path::new(root)) {
                Ok(ws) => match ws.engine().query(Query::ProjectView) {
                    cutforge_core::engine::Answer::Project(v) => emit(
                        args.json, true, "OK", "工程视图",
                        serde_json::json!({"rev": ws.rev(), "project": v}),
                    ),
                    _ => unreachable!(),
                },
                Err(e) => emit(args.json, false, "NO_ENV", &e, serde_json::json!({})),
            }
        }
        "timeline" => {
            let Some(root) = args.positional.first() else { return emit(false, args.json, "USAGE", "用法: timeline <工程目录>", serde_json::json!({})) };
            match open_ws(Path::new(root)) {
                Ok(ws) => match ws.engine().query(Query::Timeline) {
                    cutforge_core::engine::Answer::Timeline(tl) => {
                        let rows: Vec<_> = tl.into_iter().map(|(id, s, e, t)| serde_json::json!({"id": id, "startMs": s, "endMs": e, "track": t})).collect();
                        emit(args.json, true, "OK", "时间线", serde_json::json!({"clips": rows, "rev": ws.rev()}))
                    }
                    _ => unreachable!(),
                },
                Err(e) => emit(args.json, false, "NO_ENV", &e, serde_json::json!({})),
            }
        }
        "clip" => {
            if args.positional.len() < 2 {
                return emit(false, args.json, "USAGE", "用法: clip <工程目录> <clipId>", serde_json::json!({}));
            }
            match open_ws(Path::new(&args.positional[0])) {
                Ok(ws) => match ws.engine().query(Query::Clip { id: args.positional[1].clone() }) {
                    cutforge_core::engine::Answer::Clip(v) => emit(args.json, v.is_some(), if v.is_some() { "OK" } else { "NOT_FOUND" }, "片段", serde_json::json!({"clip": v})),
                    _ => unreachable!(),
                },
                Err(e) => emit(args.json, false, "NO_ENV", &e, serde_json::json!({})),
            }
        }
        "clip-update" | "split" | "undo" | "redo" => {
            let root = args.positional.first().cloned().unwrap_or_default();
            let actor = actor_from(args.flags.get("actor"));
            let apply_result = (|| -> Result<cutforge_core::engine::OpReceipt, String> {
                let mut ws = open_ws(Path::new(&root))?;
                let opts = ApplyOpts {
                    request_id: args.flags.get("request-id").cloned(),
                    summary: args.flags.get("summary").cloned(),
                    caused_by: args.flags.get("caused-by").map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default(),
                    ..Default::default()
                };
                let receipt = match cmd.as_str() {
                    "clip-update" => {
                        if args.positional.len() < 2 {
                            return Err("用法: clip-update <工程目录> <clipId> --duration-ms N …".into());
                        }
                        let mut patch = ClipPatch::default();
                        if let Some(v) = args.flags.get("duration-ms") {
                            patch.duration_ms = Some(v.parse().map_err(|_| "duration-ms 非数字")?);
                        }
                        if let Some(v) = args.flags.get("start-ms") {
                            patch.start_ms = Some(v.parse().map_err(|_| "start-ms 非数字")?);
                        }
                        if let Some(v) = args.flags.get("volume") {
                            patch.volume = Some(v.parse().map_err(|_| "volume 非数字")?);
                        }
                        ws.apply(Command::ClipUpdate { clip_id: args.positional[1].clone(), patch }, actor, opts)
                    }
                    "split" => {
                        if args.positional.len() < 3 {
                            return Err("用法: split <工程目录> <clipId> <tMs>".into());
                        }
                        let t: u64 = args.positional[2].parse().map_err(|_| "tMs 非数字")?;
                        ws.apply(Command::ClipSplit { clip_id: args.positional[1].clone(), t_ms: t }, actor, opts)
                    }
                    "undo" => ws.undo(actor),
                    "redo" => ws.redo(actor),
                    _ => unreachable!(),
                };
                receipt.map_err(|e| e.to_string())
            })();
            match apply_result {
                Ok(r) => emit(args.json, true, "OK", "已应用", serde_json::json!({"opIds": r.op_ids, "rev": r.rev, "idempotent": r.idempotent})),
                Err(e) => {
                    let code = if e.contains("工程不存在") { "NO_ENV" } else { "REJECTED" };
                    emit(args.json, false, code, &e, serde_json::json!({}))
                }
            }
        }
        "oplog" => {
            let Some(root) = args.positional.first() else { return emit(false, args.json, "USAGE", "用法: oplog <工程目录> [--since N] [--actor agent]", serde_json::json!({})) };
            match open_ws(Path::new(root)) {
                Ok(ws) => {
                    let since = args.flags.get("since").and_then(|s| s.parse().ok());
                    let kind = args.flags.get("actor").and_then(|s| match s.as_str() {
                        "agent" => Some(ActorKind::Agent),
                        "user" => Some(ActorKind::User),
                        "script" => Some(ActorKind::Script),
                        _ => None,
                    });
                    match ws.engine().query(Query::OpLogTail { since_rev: since, actor_kind: kind }) {
                        cutforge_core::engine::Answer::Ops(ops) => emit(
                            args.json, true, "OK", "操作日志",
                            serde_json::json!({"count": ops.len(), "ops": ops}),
                        ),
                        _ => unreachable!(),
                    }
                }
                Err(e) => emit(args.json, false, "NO_ENV", &e, serde_json::json!({})),
            }
        }
        "notes" => notes_list(&args),
        "notes-add" => notes_add(&args),
        "notes-resolve" => notes_resolve(&args),
        "notes-reject" => notes_reject(&args),
        "conflicts" => conflicts_list(&args),
        "check-shell-purity" => check_shell_purity(args.json),
        "check-write-paths" => check_write_paths(args.json),
        "check-deps" => check_deps(args.json),
        other => emit(false, args.json, "USAGE", &format!("未知子命令: {other}"), serde_json::json!({})),
    }
}

// ---------- 标注与冲突(计划书 4.9 / 4.7) ----------

fn notes_list(a: &Args) -> i32 {
    let Some(root) = a.positional.first() else { return emit(a.json, false, "USAGE", "用法: notes <工程目录> [--state open] [--author user]", serde_json::json!({})) };
    match Workspace::open(Path::new(root)) {
        Ok(ws) => {
            let state = a.flags.get("state").and_then(|s| serde_json::from_value::<cutforge_core::notes::NoteState>(serde_json::Value::String(s.clone())).ok());
            let author = a.flags.get("author").and_then(|s| serde_json::from_value::<cutforge_core::notes::NoteAuthor>(serde_json::Value::String(s.clone())).ok());
            let items = ws.notes().filter(state, author);
            let data = serde_json::json!({"count": items.len(), "notes": items, "orphans": ws.notes().orphans().len()});
            emit(a.json, true, "OK", "标注清单", data)
        }
        Err(e) => emit(a.json, false, "NO_ENV", &e.to_string(), serde_json::json!({})),
    }
}

fn notes_add(a: &Args) -> i32 {
    let usage = "用法: notes-add <工程目录> --kind clip --ref V1-001 --t-ms 4000 --body \"...\" [--author user] [--tag 节奏]";
    let Some(root) = a.positional.first() else { return emit(a.json, false, "USAGE", usage, serde_json::json!({})) };
    let kind = a.flags.get("kind").cloned().unwrap_or_else(|| "clip".into());
    let ref_id = a.flags.get("ref").filter(|s| !s.is_empty()).cloned();
    let t_ms: u64 = a.flags.get("t-ms").and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = a.flags.get("body").cloned().unwrap_or_default();
    if body.is_empty() {
        return emit(a.json, false, "USAGE", "body 必填", serde_json::json!({}));
    }
    let anchor_kind: cutforge_core::anchor::AnchorKind = match serde_json::from_value::<String>(serde_json::json!(kind)) {
        Ok(s) => match s.as_str() {
            "clip" => cutforge_core::anchor::AnchorKind::Clip,
            "track" => cutforge_core::anchor::AnchorKind::Track,
            "time" => cutforge_core::anchor::AnchorKind::Time,
            "word" => cutforge_core::anchor::AnchorKind::Word,
            "subtitleCard" | "subtitlecard" => cutforge_core::anchor::AnchorKind::SubtitleCard,
            _ => return emit(a.json, false, "USAGE", &format!("未知锚点类型: {kind}"), serde_json::json!({})),
        },
        Err(_) => return emit(a.json, false, "USAGE", "kind 解析失败", serde_json::json!({})),
    };
    let anchor = cutforge_core::anchor::Anchor { kind: anchor_kind, ref_: ref_id, t_ms, span: None };
    let author = match a.flags.get("author").map(|s| s.as_str()) {
        Some("agent") => cutforge_core::notes::NoteAuthor::Agent,
        _ => cutforge_core::notes::NoteAuthor::User,
    };
    let tags = a.flags.get("tag").map(|s| vec![s.clone()]).unwrap_or_default();
    match Workspace::open(Path::new(root)) {
        Ok(mut ws) => match ws.notes_add(anchor, body, author, tags, actor_from(a.flags.get("actor")), None) {
            Ok(id) => emit(a.json, true, "OK", "标注已创建", serde_json::json!({"noteId": id, "rev": ws.rev()})),
            Err(e) => emit(a.json, false, "REJECTED", &e.to_string(), serde_json::json!({})),
        },
        Err(e) => emit(a.json, false, "NO_ENV", &e.to_string(), serde_json::json!({})),
    }
}

fn notes_resolve(a: &Args) -> i32 {
    if a.positional.len() < 2 {
        return emit(a.json, false, "USAGE", "用法: notes-resolve <工程目录> <noteId> --reply \"...\" --op-ids op-1,op-2", serde_json::json!({}));
    }
    let reply = a.flags.get("reply").cloned().unwrap_or_default();
    let op_ids: Vec<String> = a.flags.get("op-ids").map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    match Workspace::open(Path::new(&a.positional[0])) {
        Ok(mut ws) => match ws.notes_resolve(&a.positional[1], reply, op_ids, actor_from(a.flags.get("actor"))) {
            Ok(()) => emit(a.json, true, "OK", "标注已结案", serde_json::json!({"noteId": a.positional[1]})),
            Err(e) => emit(a.json, false, "REJECTED", &e.to_string(), serde_json::json!({})),
        },
        Err(e) => emit(a.json, false, "NO_ENV", &e.to_string(), serde_json::json!({})),
    }
}

fn notes_reject(a: &Args) -> i32 {
    if a.positional.len() < 2 {
        return emit(a.json, false, "USAGE", "用法: notes-reject <工程目录> <noteId> --reason \"...\"", serde_json::json!({}));
    }
    let reason = a.flags.get("reason").cloned().unwrap_or_default();
    match Workspace::open(Path::new(&a.positional[0])) {
        Ok(mut ws) => match ws.notes_reject(&a.positional[1], reason, actor_from(a.flags.get("actor"))) {
            Ok(()) => emit(a.json, true, "OK", "标注已否决", serde_json::json!({})),
            Err(e) => emit(a.json, false, "REJECTED", &e.to_string(), serde_json::json!({})),
        },
        Err(e) => emit(a.json, false, "NO_ENV", &e.to_string(), serde_json::json!({})),
    }
}

fn conflicts_list(a: &Args) -> i32 {
    let Some(root) = a.positional.first() else { return emit(a.json, false, "USAGE", "用法: conflicts <工程目录>", serde_json::json!({})) };
    match Workspace::open(Path::new(root)) {
        Ok(ws) => match ws.conflict_list() {
            Ok(items) => {
                let data = serde_json::json!({"count": items.len(), "conflicts": items.iter().map(|(id, c)| serde_json::json!({"conflictId": id, "code": c.code.code(), "pointer": c.pointer})).collect::<Vec<_>>()});
                emit(a.json, true, "OK", "冲突清单", data)
            }
            Err(e) => emit(a.json, false, "INTERNAL", &e.to_string(), serde_json::json!({})),
        },
        Err(e) => emit(a.json, false, "NO_ENV", &e.to_string(), serde_json::json!({})),
    }
}

/// M5-5 判定器:壳不持有真相——apps/ 与 cutforge-wasm 禁文件系统/子进程/时间线语义运算。
fn check_shell_purity(json: bool) -> i32 {
    let root = repo_root();
    // (模式串拼接构造,避免本文件自匹配;时间线语义模式针对 JS 侧手算)
    let js_pats: Vec<String> = vec![
        ["node", "fs"].join(":"), ["requ", "ire(\"fs\")"].join(""), ["child_process"].join(""),
        ["startMs", "+"].join(" "), ["startMs", "+"].join(""), ["endMs", " ="].join(" "),
        ["durationMs", " +"].join(" "),
    ];
    let rs_pats: Vec<String> = vec![["std", "fs"].join("::"), ["fs", "read_to_string"].join("::")];
    let mut violations: Vec<serde_json::Value> = Vec::new();
    let mut scan_dir = |dir: &Path, pats: &[String], is_rs: bool| {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let ext_ok = if is_rs {
                    p.extension().is_some_and(|x| x == "rs")
                } else {
                    p.extension().is_some_and(|x| x == "js" || x == "html")
                };
                if !ext_ok || p.file_name().is_some_and(|n| n.to_string_lossy().contains("min.")) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&p) else { continue };
                for line in text.lines() {
                    if pats.iter().any(|pat| line.contains(pat.as_str())) {
                        violations.push(serde_json::json!({
                            "file": p.strip_prefix(&root).map(|r| r.to_string_lossy()).unwrap_or_default(),
                            "line": line.trim().chars().take(80).collect::<String>(),
                        }));
                        break;
                    }
                }
            }
        }
    };
    // JS 壳:禁 node 能力面与时间线手算
    scan_dir(&root.join("apps/web"), &js_pats, false);
    // wasm 绑定:禁文件系统(RS 侧)
    scan_dir(&root.join("crates/cutforge-wasm/src"), &rs_pats, true);
    let data = serde_json::json!({"violations": violations,
        "rule": "壳禁文件系统/子进程/时间线语义运算;一切投影来自内核(计划书 2.6/7.6)"});
    if violations.is_empty() {
        emit(json, true, "OK", "壳纯度合规:违规点 = 0", data)
    } else {
        emit(json, false, "SHELL_PURITY_VIOLATION", &format!("违规 {} 处", violations.len()), data)
    }
}

/// M2-4 判定器:文件写入 API 只允许出现在 cutforge-io 的 atomic.rs(唯一落盘点)。
/// 注意:模式串在此处拼接构造,避免本文件自匹配。
fn check_write_paths(json: bool) -> i32 {
    let root = repo_root();
    let pats: Vec<String> = vec![
        ["fs", "write"].join("::"),
        ["File", "create"].join("::"),
        ["Open", "Options"].join(""),
        ["write", "_all"].join(""),
        ["fs", "rename"].join("::"),
        ["remove", "_file"].join("::"),
        ["fs", "copy"].join("::"),
    ];
    let mut violations: Vec<serde_json::Value> = Vec::new();
    let mut sanctioned = 0usize;
    let crates_dir = root.join("crates");
    let mut stack = vec![crates_dir.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let is_atomic = p.ends_with("atomic.rs");
            let mut hits = 0usize;
            for line in text.lines() {
                if pats.iter().any(|pat| line.contains(pat.as_str())) {
                    hits += 1;
                }
            }
            if hits == 0 {
                continue;
            }
            if is_atomic {
                sanctioned += hits;
            } else {
                violations.push(serde_json::json!({
                    "file": p.strip_prefix(&root).map(|r| r.to_string_lossy()).unwrap_or_default(),
                    "hits": hits,
                }));
            }
        }
    }
    let data = serde_json::json!({
        "sanctioned_atomic_hits": sanctioned,
        "violations": violations,
        "rule": "文件写入 API 仅允许 crates/cutforge-io/src/atomic.rs(计划书 4.2 唯一写入路径)"
    });
    if sanctioned > 0 && violations.is_empty() {
        emit(json, true, "OK", "写入路径唯一:仅 atomic.rs 落盘,旁路写入 = 0", data)
    } else {
        emit(json, false, "WRITE_PATH_VIOLATION", &format!("旁路写入 {} 处", violations.len()), data)
    }
}

/// M2-6 判定器:依赖方向合规(计划书 2.3 禁止清单)。
fn check_deps(json: bool) -> i32 {
    let root = repo_root();
    // 允许的 cutforge 内部依赖边
    let allowed: std::collections::BTreeMap<&str, Vec<&str>> = [
        ("cutforge-schema", vec![]),
        ("cutforge-core", vec!["cutforge-schema"]),
        ("cutforge-io", vec!["cutforge-core", "cutforge-schema"]),
        ("cutforge-cli", vec!["cutforge-core", "cutforge-io"]),
        ("cutforge-mcp", vec!["cutforge-core", "cutforge-io", "cutforge-script", "cutforge-schema"]),
        ("cutforge-script", vec!["cutforge-core"]),
        ("cutforge-wasm", vec!["cutforge-core", "cutforge-script"]),
        ("cutforge-plugin-host", vec!["cutforge-core"]),
        ("cutforge-render", vec!["cutforge-core", "cutforge-io"]), // M6:已实现,合法边
    ]
    .into_iter()
    .collect();

    let mut edges: Vec<(String, String)> = Vec::new();
    let crates_dir = root.join("crates");
    let Ok(rd) = std::fs::read_dir(&crates_dir) else {
        return emit(json, false, "NO_ENV", "crates/ 目录不存在", serde_json::json!({}));
    };
    for entry in rd.flatten() {
        let manifest = entry.path().join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(text) = std::fs::read_to_string(&manifest) else { continue };
        let mut in_deps = false;
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with('[') {
                in_deps = t == "[dependencies]";
                continue;
            }
            if in_deps && t.starts_with("cutforge-")
                && let Some((dep, _)) = t.split_once('=') {
                    edges.push((name.clone(), dep.trim().to_string()));
                }
        }
    }
    let mut violations: Vec<serde_json::Value> = Vec::new();
    for (from, to) in &edges {
        let ok = allowed
            .get(from.as_str())
            .is_some_and(|list| list.iter().any(|d| d == to));
        if !ok {
            violations.push(serde_json::json!({"from": from, "to": to}));
        }
    }
    let data = serde_json::json!({"edges": edges.iter().map(|(a,b)| format!("{a} -> {b}")).collect::<Vec<_>>(), "violations": violations});
    if violations.is_empty() {
        emit(json, true, "OK", "依赖方向合规(2.3 禁止清单零违反)", data)
    } else {
        emit(json, false, "DEP_VIOLATION", &format!("违规依赖边 {} 条", violations.len()), data)
    }
}

