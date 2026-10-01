// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! CutForge CLI(计划书 2.1 接入层):人肉操作、CI、门禁脚本入口。
//! 全部输出 {ok, code, message, data} 结果协议(T1.7 另派生加法字段 ns,三面同码);
//! 退出码 0/2/3/4。
//! 手写参数解析(不引第三方 CLI 框架,依赖纪律见 ADR-0034)。

use cutforge_core::command::{ClipPatch, Command};
use cutforge_core::engine::{ApplyOpts, Query};
use cutforge_core::oplog::{Actor, ActorKind};
use cutforge_io::Workspace;
use std::path::{Path, PathBuf};

/// 渲染缓存治理(T1.5):cache {info,gc,clear}。
mod cache;
/// 工程环境诊断(T1.7):doctor——每项失败给可复制执行的修复命令。
mod doctor;
/// 门禁判定器(册二 T2.6):check-shell-purity v2 / check-write-paths v2(自本文件迁入并升级)。
mod gates;
/// 库面与管理子命令(册六 T6.1):library / migrate / recover。
mod library;

const EXIT_OK: i32 = 0;
const EXIT_FAIL: i32 = 2;
const EXIT_ENV: i32 = 3;

pub(crate) fn emit(json: bool, ok: bool, code: &str, message: &str, data: serde_json::Value) -> i32 {
    // T1.7 三面同码:CLI 面的 ns 与 MCP/HTTP 同源(cutforge_mcp::code_namespace,
    // 单一真相源 registry::CODE_NS)。ns 是加法字段,ok/code/message/data 老字段逐字不变。
    let envelope = serde_json::json!(
        {"ok": ok, "code": code, "ns": cutforge_mcp::code_namespace(code), "message": message, "data": data});
    if json {
        println!("{envelope}");
    } else {
        println!("[{}] {code}: {message}", if ok { "PASS" } else { "FAIL" });
    }
    match code {
        "OK" => EXIT_OK,
        // 环境类(5.4:NO_CONFIG/DEP_MISSING)→ 退出码 3;其余失败 → 2
        "NO_CONFIG" | "DEP_MISSING" => EXIT_ENV,
        _ => EXIT_FAIL,
    }
}

pub(crate) struct Args {
    pub(crate) positional: Vec<String>,
    pub(crate) flags: std::collections::BTreeMap<String, String>,
    pub(crate) json: bool,
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

/// 变更子命令专用:锁覆盖 open→apply→persist 全程(P0-5)。
fn open_ws_exclusive(root: &Path) -> Result<Workspace, String> {
    Workspace::open_exclusive(root).map_err(|e| {
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
    // 旗标先行归一:`cutforge-cli --json <子命令>` 与 `<子命令> … --json` 等价
    // (B7 口径对拍需要机器可读的自述/结果,无须记忆旗标位置)。
    let json_first = argv.first().map(|a| a == "--json").unwrap_or(false);
    let argv: Vec<String> = if json_first { argv[1..].to_vec() } else { argv };
    let Some(cmd) = argv.first().cloned() else {
        // E7-3/B8:自述与实际实现保持同步(新增子命令时必须更新此清单)
        return emit(json_first, false, "PRECONDITION_FAILED", "用法: cutforge-cli <子命令> […]", serde_json::json!({
            "subcommands": ["new", "project", "timeline", "clip", "clip-update", "split", "undo", "redo",
                "oplog", "notes", "notes-add", "notes-resolve", "notes-reject", "conflicts",
                "cache", "doctor", "serve", "library", "migrate", "recover",
                "check-shell-purity", "check-write-paths", "check-deps", "check-ui-fields"]
        }));
    };
    let mut args = parse_args(&argv[1..]);
    args.json = args.json || json_first;
    match cmd.as_str() {
        "serve" => serve_cmd(&args),
        "new" => new_project_cmd(&args),
        "project" => {
            let Some(root) = args.positional.first() else { return emit(false, args.json, "PRECONDITION_FAILED", "用法: project <工程目录>", serde_json::json!({})) };
            match open_ws(Path::new(root)) {
                Ok(ws) => match ws.engine().query(Query::ProjectView) {
                    cutforge_core::engine::Answer::Project(v) => emit(
                        args.json, true, "OK", "工程视图",
                        serde_json::json!({"rev": ws.rev(), "project": v}),
                    ),
                    _ => unreachable!(),
                },
                Err(e) => emit(args.json, false, "NO_CONFIG", &e, serde_json::json!({})),
            }
        }
        "timeline" => {
            let Some(root) = args.positional.first() else { return emit(false, args.json, "PRECONDITION_FAILED", "用法: timeline <工程目录>", serde_json::json!({})) };
            match open_ws(Path::new(root)) {
                Ok(ws) => match ws.engine().query(Query::Timeline) {
                    cutforge_core::engine::Answer::Timeline(tl) => {
                        let rows: Vec<_> = tl.into_iter().map(|(id, s, e, t)| serde_json::json!({"id": id, "startMs": s, "endMs": e, "track": t})).collect();
                        emit(args.json, true, "OK", "时间线", serde_json::json!({"clips": rows, "rev": ws.rev()}))
                    }
                    _ => unreachable!(),
                },
                Err(e) => emit(args.json, false, "NO_CONFIG", &e, serde_json::json!({})),
            }
        }
        "clip" => {
            if args.positional.len() < 2 {
                return emit(false, args.json, "PRECONDITION_FAILED", "用法: clip <工程目录> <clipId>", serde_json::json!({}));
            }
            match open_ws(Path::new(&args.positional[0])) {
                Ok(ws) => match ws.engine().query(Query::Clip { id: args.positional[1].clone() }) {
                    cutforge_core::engine::Answer::Clip(v) => emit(args.json, v.is_some(), if v.is_some() { "OK" } else { "NO_CONFIG" }, "片段", serde_json::json!({"clip": v})),
                    _ => unreachable!(),
                },
                Err(e) => emit(args.json, false, "NO_CONFIG", &e, serde_json::json!({})),
            }
        }
        "clip-update" | "split" | "undo" | "redo" => {
            let root = args.positional.first().cloned().unwrap_or_default();
            let actor = actor_from(args.flags.get("actor"));
            let apply_result = (|| -> Result<cutforge_core::engine::OpReceipt, String> {
                let mut ws = open_ws_exclusive(Path::new(&root))?;
                let opts = ApplyOpts {
                    request_id: args.flags.get("request-id").cloned(),
                    summary: args.flags.get("summary").cloned(),
                    caused_by: args.flags.get("caused-by").map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default(),
                    ..Default::default()
                };
                let receipt = match cmd.as_str() {
                    "clip-update" => {
                        if args.positional.len() < 2 {
                            return Err("用法: clip-update <工程目录> <clipId> --duration-ms N …(支持字段与 MCP clip_update 对齐)".into());
                        }
                        // B9-1:字段与 MCP clip_update 完全对齐(9 个 ClipPatch 字段)
                        let mut patch = ClipPatch::default();
                        let num = |flag: &str| -> Result<Option<f64>, String> {
                            match args.flags.get(flag) {
                                Some(v) => v.parse::<f64>().map(Some).map_err(|_| format!("{flag} 非数字")),
                                None => Ok(None),
                            }
                        };
                        if let Some(v) = args.flags.get("duration-ms") {
                            patch.duration_ms = Some(v.parse().map_err(|_| "duration-ms 非数字")?);
                        }
                        if let Some(v) = args.flags.get("start-ms") {
                            patch.start_ms = Some(v.parse().map_err(|_| "start-ms 非数字")?);
                        }
                        if let Some(v) = args.flags.get("source-in-ms") {
                            patch.source_in_ms = Some(v.parse().map_err(|_| "source-in-ms 非数字")?);
                        }
                        if let Some(v) = args.flags.get("freeze-ms") {
                            patch.freeze_ms = Some(v.parse().map_err(|_| "freeze-ms 非数字")?);
                        }
                        if let Some(v) = num("volume")? { patch.volume = Some(v); }
                        if let Some(v) = num("opacity")? { patch.opacity = Some(v); }
                        if let Some(v) = num("scale")? { patch.scale = Some(v); }
                        if let Some(v) = num("speed")? { patch.speed = Some(v); }
                        // 册四 A4 T4.4/T4.9:B9-1 字段对齐——五新字段与 MCP clip_update 同面;
                        // speed-curve/crop 收 JSON 字面量(整组替换,与 patch 语义一致)
                        if let Some(v) = args.flags.get("speed-curve") {
                            patch.speed_curve = Some(serde_json::from_str::<Vec<cutforge_core::model::SpeedPoint>>(v)
                                .map_err(|e| format!("speed-curve JSON 非法: {e}"))?);
                        }
                        if let Some(v) = args.flags.get("reverse") {
                            patch.reverse = Some(v != "false");
                        }
                        if let Some(v) = num("rotation")? { patch.rotation = Some(v); }
                        if let Some(v) = args.flags.get("crop") {
                            patch.crop = Some(serde_json::from_str::<cutforge_core::model::Crop>(v)
                                .map_err(|e| format!("crop JSON 非法: {e}"))?);
                        }
                        if let Some(v) = args.flags.get("flip") {
                            patch.flip = Some(v.clone());
                        }
                        if let Some(v) = args.flags.get("text") {
                            patch.text = Some(v.clone());
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
                    let code = if e.contains("工程不存在") { "NO_CONFIG" } else { "PRECONDITION_FAILED" };
                    emit(args.json, false, code, &e, serde_json::json!({}))
                }
            }
        }
        "oplog" => {
            let Some(root) = args.positional.first() else { return emit(false, args.json, "PRECONDITION_FAILED", "用法: oplog <工程目录> [--since N] [--actor agent]", serde_json::json!({})) };
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
                Err(e) => emit(args.json, false, "NO_CONFIG", &e, serde_json::json!({})),
            }
        }
        "notes" => notes_list(&args),
        "notes-add" => notes_add(&args),
        "notes-resolve" => notes_resolve(&args),
        "notes-reject" => notes_reject(&args),
        "conflicts" => conflicts_list(&args),
        "cache" => cache::run(&args),
        "doctor" => doctor::run(&args),
        // 册六 T6.1:工程库 / 布局迁移 / 崩溃恢复(实现在 library.rs,单一实现走 cutforge_io)
        "library" => library::library_cmd(&args),
        "migrate" => library::migrate_cmd(&args),
        "recover" => library::recover_cmd(&args),
        "check-shell-purity" => gates::check_shell_purity(args.json),
        "check-write-paths" => gates::check_write_paths(args.json),
        "check-deps" => check_deps(args.json),
        "check-ui-fields" => check_ui_fields(args.json),
        other => emit(false, args.json, "PRECONDITION_FAILED", &format!("未知子命令: {other}"), serde_json::json!({})),
    }
}

// ---------- serve(E1-3/E1-4:编辑器启动入口;薄转发到 cutforge_mcp::serve_workspace) ----------

fn serve_cmd(a: &Args) -> i32 {
    use std::io::IsTerminal as _;
    let root = a.positional.first().cloned().or_else(|| a.flags.get("root").cloned());
    let root = match root {
        Some(r) => PathBuf::from(r),
        None => {
            // E6-1(提前落):无 --root 时交互列候选工程;非交互环境必须显式给目录
            if !std::io::stdin().is_terminal() {
                return emit(a.json, false, "PRECONDITION_FAILED",
                    "用法: serve <工程目录> [--port N] [--token T] [--web 目录] [--open];非交互环境必须给工程目录",
                    serde_json::json!({}));
            }
            // 单一实现:E1-4 交互选择器迁至 cutforge_mcp(mcp serve 无 --root 同一行为)
            match cutforge_mcp::pick_project_interactive() {
                Some(p) => p,
                None => return emit(a.json, false, "NO_CONFIG",
                    "未找到候选工程(查找:CUTFORGE_PROJECTS 或当前目录下两层内的 05_时间线工程/project.json,兼容旧 05_ir/;或先新建:cutforge-cli new <目录>)",
                    serde_json::json!({})),
            }
        }
    };
    let explicit_port = a.flags.get("port").and_then(|s| s.parse::<u16>().ok());
    let mut port = explicit_port.unwrap_or(8787);
    if explicit_port.is_none() {
        // E1-4:未指定端口时自动挑空闲(+1..+20;探测即放手的竞态由 serve 自检兜底)
        for cand in port..port.saturating_add(20) {
            if std::net::TcpListener::bind(("127.0.0.1", cand)).is_ok() {
                port = cand;
                break;
            }
        }
    }
    let token = a.flags.get("token").cloned().unwrap_or_else(cutforge_mcp::new_token);
    let web = a.flags.get("web").map(PathBuf::from).unwrap_or_else(cutforge_mcp::default_web_dir);
    let open = a.flags.contains_key("open");
    cutforge_mcp::serve_workspace(&root, port, &token, &web, open)
}

/// B11-1:新建空工程(与 MCP `project_new` 工具同走 cutforge_io::scaffold,单一实现)。
/// 用法: new <工程目录> [--slug S] [--fps 30] [--width 1080] [--height 1920] [--track video --track audio]
fn new_project_cmd(a: &Args) -> i32 {
    let Some(root) = a.positional.first() else {
        return emit(a.json, false, "PRECONDITION_FAILED",
            "用法: new <工程目录> [--slug S] [--fps 30] [--width 1080] [--height 1920] [--track video,audio] [--layout v2|v3]",
            serde_json::json!({}));
    };
    let slug = a.flags.get("slug").cloned()
        .or_else(|| std::path::Path::new(root).file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "cutforge-project".into());
    let fps = a.flags.get("fps").and_then(|s| s.parse().ok()).unwrap_or(30);
    let width = a.flags.get("width").and_then(|s| s.parse().ok()).unwrap_or(1080);
    let height = a.flags.get("height").and_then(|s| s.parse().ok()).unwrap_or(1920);
    let mut kinds = Vec::new();
    // 轨道类型支持逗号分隔(--track video,audio);手写参数解析的 flags 表同键覆盖,
    // 故不采用重复旗标形式
    if let Some(v) = a.flags.get("track") {
        for t in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let kind = match t {
                "video" => cutforge_core::model::TrackKind::Video,
                "audio" => cutforge_core::model::TrackKind::Audio,
                "text" => cutforge_core::model::TrackKind::Text,
                "adjust" => cutforge_core::model::TrackKind::Adjust,
                other => return emit(a.json, false, "PRECONDITION_FAILED",
                    &format!("未知轨道类型: {other}(允许 video/audio/text/adjust)"), serde_json::json!({})),
            };
            kinds.push(kind);
        }
    }
    if kinds.is_empty() {
        kinds = vec![cutforge_core::model::TrackKind::Video, cutforge_core::model::TrackKind::Audio];
    }
    // 册六 ADR-0021:布局显式开关(过渡期缺省 v2;v3 = 扁平布局)
    let layout = match a.flags.get("layout").map(|s| s.as_str()).unwrap_or("v2") {
        "v2" => cutforge_io::paths::LayoutKind::V2,
        "v3" => cutforge_io::paths::LayoutKind::V3,
        other => return emit(a.json, false, "PRECONDITION_FAILED",
            &format!("未知布局: {other}(允许 v2/v3;过渡期缺省 v2,ADR-0021)"), serde_json::json!({})),
    };
    match cutforge_io::scaffold::scaffold_project_layout(Path::new(root), &slug, fps, width, height, &kinds, layout) {
        Ok(path) => emit(a.json, true, "OK", "空工程已创建(可独立起步,不依赖 CutFlow)", serde_json::json!({
            "project": path.to_string_lossy(),
            "slug": slug, "fps": fps, "canvas": {"width": width, "height": height},
            "layout": if layout == cutforge_io::paths::LayoutKind::V3 { "v3" } else { "v2" },
            "hint": format!("打开:cutforge-cli serve {root} --open"),
        })),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => emit(a.json, false, "PRECONDITION_FAILED", &e.to_string(), serde_json::json!({})),
        Err(e) => emit(a.json, false, "SCHEMA_INVALID", &e.to_string(), serde_json::json!({})),
    }
}

/// E4-2 机械校验:schemas/ui-fields.json 声明的"壳允许编辑字段集" ⊆ 内核 patch 字段集。
/// editable(片段检查器)⊆ ClipPatch;trackEditable(轨道头,册四 A4 T4.2)⊆ TrackPatch。
/// 与 check-shell-purity 同风格(判定器进 CLI,可入门禁);真相源漂移在此红。
fn check_ui_fields(json: bool) -> i32 {
    let root = repo_root();
    let doc: serde_json::Value = match std::fs::read_to_string(root.join("schemas/ui-fields.json"))
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(e) => return emit(json, false, "NO_CONFIG", &format!("schemas/ui-fields.json 不可读: {e}"), serde_json::json!({})),
    };
    // patch 字段集:从 command.rs 的 struct 块按 `pub <snake>: Option<…>` 抽取,再转 camelCase
    let src = match std::fs::read_to_string(root.join("crates/cutforge-core/src/command.rs")) {
        Ok(s) => s,
        Err(e) => return emit(json, false, "NO_CONFIG", &format!("command.rs 不可读: {e}"), serde_json::json!({})),
    };
    let clip_patch_fields = patch_fields(&src, "ClipPatch");
    let track_patch_fields = patch_fields(&src, "TrackPatch");
    if clip_patch_fields.is_empty() || track_patch_fields.is_empty() {
        return emit(json, false, "INTERNAL", "未能从 command.rs 解析出 ClipPatch/TrackPatch 字段(结构变化需同步本判定器)", serde_json::json!({}));
    }
    let collect = |key: &str| -> Vec<String> {
        doc[key].as_object().map(|groups| {
            groups.values().flat_map(|fields| {
                fields.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>()).unwrap_or_default()
            }).collect()
        }).unwrap_or_default()
    };
    let ui_fields = collect("editable");
    let track_ui_fields = collect("trackEditable");
    let mut violations: Vec<serde_json::Value> = Vec::new();
    for f in &ui_fields {
        if !clip_patch_fields.contains(f) {
            violations.push(serde_json::json!({"field": f, "reason": "ui-fields 声明可编辑,但 ClipPatch 无此字段(内核不支持,编辑会成幻觉)"}));
        }
    }
    for f in &track_ui_fields {
        if !track_patch_fields.contains(f) {
            violations.push(serde_json::json!({"field": f, "reason": "trackEditable 声明可编辑,但 TrackPatch 无此字段(track_update 不认,编辑会成幻觉)"}));
        }
    }
    // 分组声明完整性:分组名单不得为空(壳按分组渲染)
    if doc["editable"].as_object().map(|o| o.len()).unwrap_or(0) == 0 {
        violations.push(serde_json::json!({"field": "editable", "reason": "editable 分组缺失或为空"}));
    }
    if doc["trackEditable"].as_object().map(|o| o.len()).unwrap_or(0) == 0 {
        violations.push(serde_json::json!({"field": "trackEditable", "reason": "trackEditable 分组缺失或为空(册四 A4 轨道头字段必须显式声明)"}));
    }
    let data = serde_json::json!({
        "uiFields": ui_fields,
        "clipPatchFields": clip_patch_fields,
        "trackUiFields": track_ui_fields,
        "trackPatchFields": track_patch_fields,
        "readonlyDisplay": doc["readonly"].clone(),
        "violations": violations,
        "rule": "壳可编辑字段集 ⊆ 内核 patch 字段集(E4-2):editable ⊆ ClipPatch、trackEditable ⊆ TrackPatch;差异必须显式声明,不得静默漂移",
    });
    if violations.is_empty() {
        emit(json, true, "OK", &format!(
            "ui-fields 合规:{} 个片段可编辑字段全部被 ClipPatch 支撑,{} 个轨道可编辑字段全部被 TrackPatch 支撑",
            ui_fields.len(), track_ui_fields.len()), data)
    } else {
        emit(json, false, "UI_FIELDS_VIOLATION", &format!("违规 {} 处", violations.len()), data)
    }
}

/// 从 `struct <name> { … }` 块抽取 `pub <snake>: Option<…>` 字段名并转 camelCase。
fn patch_fields(src: &str, struct_name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(start) = src.find(&format!("pub struct {struct_name}")) else { return out };
    let body = &src[start..];
    let Some(end) = body.find('}') else { return out };
    for line in body[..end].lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("pub ") else { continue };
        let Some((name, _)) = rest.split_once(':') else { continue };
        let name = name.trim();
        // snake_case → camelCase(字段名均无前导下划线)
        let mut camel = String::new();
        let mut upper_next = false;
        for ch in name.chars() {
            if ch == '_' {
                upper_next = true;
            } else if upper_next {
                camel.extend(ch.to_uppercase());
                upper_next = false;
            } else {
                camel.push(ch);
            }
        }
        out.push(camel);
    }
    out
}

// ---------- 标注与冲突(计划书 4.9 / 4.7) ----------

fn notes_list(a: &Args) -> i32 {
    let Some(root) = a.positional.first() else { return emit(a.json, false, "PRECONDITION_FAILED", "用法: notes <工程目录> [--state open] [--author user]", serde_json::json!({})) };
    match Workspace::open(Path::new(root)) {
        Ok(ws) => {
            let state = a.flags.get("state").and_then(|s| serde_json::from_value::<cutforge_core::notes::NoteState>(serde_json::Value::String(s.clone())).ok());
            let author = a.flags.get("author").and_then(|s| serde_json::from_value::<cutforge_core::notes::NoteAuthor>(serde_json::Value::String(s.clone())).ok());
            let items = ws.notes().filter(state, author);
            let data = serde_json::json!({"count": items.len(), "notes": items, "orphans": ws.notes().orphans().len()});
            emit(a.json, true, "OK", "标注清单", data)
        }
        Err(e) => emit(a.json, false, "NO_CONFIG", &e.to_string(), serde_json::json!({})),
    }
}

fn notes_add(a: &Args) -> i32 {
    let usage = "用法: notes-add <工程目录> --kind clip --ref V1-001 --t-ms 4000 --body \"...\" [--author user] [--tag 节奏]";
    let Some(root) = a.positional.first() else { return emit(a.json, false, "PRECONDITION_FAILED", usage, serde_json::json!({})) };
    let kind = a.flags.get("kind").cloned().unwrap_or_else(|| "clip".into());
    let ref_id = a.flags.get("ref").filter(|s| !s.is_empty()).cloned();
    let t_ms: u64 = a.flags.get("t-ms").and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = a.flags.get("body").cloned().unwrap_or_default();
    if body.is_empty() {
        return emit(a.json, false, "PRECONDITION_FAILED", "body 必填", serde_json::json!({}));
    }
    let anchor_kind: cutforge_core::anchor::AnchorKind = match serde_json::from_value::<String>(serde_json::json!(kind)) {
        Ok(s) => match s.as_str() {
            "clip" => cutforge_core::anchor::AnchorKind::Clip,
            "track" => cutforge_core::anchor::AnchorKind::Track,
            "time" => cutforge_core::anchor::AnchorKind::Time,
            "word" => cutforge_core::anchor::AnchorKind::Word,
            "subtitleCard" | "subtitlecard" => cutforge_core::anchor::AnchorKind::SubtitleCard,
            _ => return emit(a.json, false, "PRECONDITION_FAILED", &format!("未知锚点类型: {kind}"), serde_json::json!({})),
        },
        Err(_) => return emit(a.json, false, "PRECONDITION_FAILED", "kind 解析失败", serde_json::json!({})),
    };
    let anchor = cutforge_core::anchor::Anchor { kind: anchor_kind, ref_: ref_id, t_ms, span: None };
    let author = match a.flags.get("author").map(|s| s.as_str()) {
        Some("agent") => cutforge_core::notes::NoteAuthor::Agent,
        _ => cutforge_core::notes::NoteAuthor::User,
    };
    let tags = a.flags.get("tag").map(|s| vec![s.clone()]).unwrap_or_default();
    match Workspace::open_exclusive(Path::new(root)) {
        Ok(mut ws) => match ws.notes_add(anchor, body, author, tags, actor_from(a.flags.get("actor")), None) {
            Ok(id) => emit(a.json, true, "OK", "标注已创建", serde_json::json!({"noteId": id, "rev": ws.rev()})),
            Err(e) => emit(a.json, false, "PRECONDITION_FAILED", &e.to_string(), serde_json::json!({})),
        },
        Err(e) => emit(a.json, false, "NO_CONFIG", &e.to_string(), serde_json::json!({})),
    }
}

fn notes_resolve(a: &Args) -> i32 {
    if a.positional.len() < 2 {
        return emit(a.json, false, "PRECONDITION_FAILED", "用法: notes-resolve <工程目录> <noteId> --reply \"...\" --op-ids op-1,op-2", serde_json::json!({}));
    }
    let reply = a.flags.get("reply").cloned().unwrap_or_default();
    let op_ids: Vec<String> = a.flags.get("op-ids").map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    match Workspace::open_exclusive(Path::new(&a.positional[0])) {
        Ok(mut ws) => match ws.notes_resolve(&a.positional[1], reply, op_ids, actor_from(a.flags.get("actor"))) {
            Ok(()) => emit(a.json, true, "OK", "标注已结案", serde_json::json!({"noteId": a.positional[1]})),
            Err(e) => emit(a.json, false, "PRECONDITION_FAILED", &e.to_string(), serde_json::json!({})),
        },
        Err(e) => emit(a.json, false, "NO_CONFIG", &e.to_string(), serde_json::json!({})),
    }
}

fn notes_reject(a: &Args) -> i32 {
    if a.positional.len() < 2 {
        return emit(a.json, false, "PRECONDITION_FAILED", "用法: notes-reject <工程目录> <noteId> --reason \"...\"", serde_json::json!({}));
    }
    let reason = a.flags.get("reason").cloned().unwrap_or_default();
    match Workspace::open_exclusive(Path::new(&a.positional[0])) {
        Ok(mut ws) => match ws.notes_reject(&a.positional[1], reason, actor_from(a.flags.get("actor"))) {
            Ok(()) => emit(a.json, true, "OK", "标注已否决", serde_json::json!({})),
            Err(e) => emit(a.json, false, "PRECONDITION_FAILED", &e.to_string(), serde_json::json!({})),
        },
        Err(e) => emit(a.json, false, "NO_CONFIG", &e.to_string(), serde_json::json!({})),
    }
}

fn conflicts_list(a: &Args) -> i32 {
    let Some(root) = a.positional.first() else { return emit(a.json, false, "PRECONDITION_FAILED", "用法: conflicts <工程目录>", serde_json::json!({})) };
    match Workspace::open(Path::new(root)) {
        Ok(ws) => match ws.conflict_list() {
            Ok(items) => {
                let data = serde_json::json!({"count": items.len(), "conflicts": items.iter().map(|(id, c)| serde_json::json!({"conflictId": id, "code": c.code.code(), "pointer": c.pointer})).collect::<Vec<_>>()});
                emit(a.json, true, "OK", "冲突清单", data)
            }
            Err(e) => emit(a.json, false, "INTERNAL", &e.to_string(), serde_json::json!({})),
        },
        Err(e) => emit(a.json, false, "NO_CONFIG", &e.to_string(), serde_json::json!({})),
    }
}

/// M2-6 判定器:依赖方向合规(计划书 2.3 禁止清单)。(shell-purity / write-paths 见 gates.rs)
fn check_deps(json: bool) -> i32 {
    let root = repo_root();
    // 允许的 cutforge 内部依赖边
    let allowed: std::collections::BTreeMap<&str, Vec<&str>> = [
        ("cutforge-schema", vec![]),
        ("cutforge-core", vec!["cutforge-schema"]),
        ("cutforge-io", vec!["cutforge-core", "cutforge-schema"]),
        // E1-3:cli 的 serve 子命令薄转发 cutforge_mcp::serve_workspace(同一实现)
        ("cutforge-cli", vec!["cutforge-core", "cutforge-io", "cutforge-mcp"]),
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
        return emit(json, false, "NO_CONFIG", "crates/ 目录不存在", serde_json::json!({}));
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

