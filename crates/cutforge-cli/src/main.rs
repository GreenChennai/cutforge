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
        c if c == "OK" => EXIT_OK,
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

fn run(argv: Vec<String>) -> i32 {
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
        "check-write-paths" => check_write_paths(args.json),
        "check-deps" => check_deps(args.json),
        other => emit(false, args.json, "USAGE", &format!("未知子命令: {other}"), serde_json::json!({})),
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
        ("cutforge-mcp", vec!["cutforge-core"]),
        ("cutforge-script", vec!["cutforge-core"]),
        ("cutforge-wasm", vec!["cutforge-core"]),
        ("cutforge-plugin-host", vec!["cutforge-core"]),
        ("cutforge-render", vec!["cutforge-core", "cutforge-io"]),
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
            if in_deps && t.starts_with("cutforge-") {
                if let Some((dep, _)) = t.split_once('=') {
                    edges.push((name.clone(), dep.trim().to_string()));
                }
            }
        }
    }
    let mut violations: Vec<serde_json::Value> = Vec::new();
    for (from, to) in &edges {
        let ok = allowed
            .get(from.as_str())
            .is_some_and(|list| list.iter().any(|d| d == &to));
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

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(argv));
}
