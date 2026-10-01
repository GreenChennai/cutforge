// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 工作区常驻服务(M10 本地服务化):serve 启动自检、交互选工程、静态资源、
//! /rpc、/media、/media/browse、/events、/ui-fields、/session(T1.1 拆分自 lib.rs,纯移动)。

use crate::dispatch::{handle_rpc_as, produces_rev_mutation, resolve_within_root};
use crate::progress::resolve_render_bin;
use crate::registry::{FX_CATALOG_JSON, HUAZI_CATALOG_JSON, TRANSITION_CATALOG_JSON, UI_FIELDS_JSON};
use crate::session::{session_journal_begin, session_journal_note, session_summary_path};
use crate::tools_nolock::media_browse_payload;
use crate::transport::events;
use crate::transport::http::{self, HttpResp, mime_of, pct_decode, resp_plain};
use crate::transport::static_files;
use cutforge_core::oplog::Actor;
use cutforge_io::paths;
use serde_json::{json, Value};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// E1-6:serve 启动自检。缺工程(致命)→ Err;其余(渲染依赖/静态资源)→ 打印 △ 提示。
fn serve_preflight(root: &Path, web_dir: &Path) -> Result<(), String> {
    eprintln!("── CutForge 编辑器启动自检 ──");
    // 目录契约 0.5:优先 05_时间线工程/project.json,0.4.x 旧布局 05_ir/ 兼容
    let project = paths::project_path(root);
    eprintln!("{} 工程: {}", if project.is_file() { "✓" } else { "✗" }, project.display());
    if !root.is_dir() {
        return Err(format!("工程目录不存在:{}(补救:检查 --root 拼写,或先用 CutFlow 建工程)", root.display()));
    }
    if !project.is_file() {
        return Err(format!(
            "缺 05_时间线工程/project.json(兼容旧 05_ir/):{} 不是 CutForge/CutFlow 工程(补救:用 CutFlow `rs_run.py --init` 建工程,或换 --root)",
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

/// E1-4/E6-1:交互式工程选择(工程存在者,新旧布局皆认;按修改时间倒序,回车 = 最近工程)。
/// 单一实现纪律:cli serve 与 mcp serve 的无 --root 交互列工程都走这里(不再各写一份)。
pub fn pick_project_interactive() -> Option<PathBuf> {
    use std::io::Write as _;
    let base = std::env::var_os("CUTFORGE_PROJECTS")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let mut cands: Vec<PathBuf> = Vec::new();
    let mut stack = vec![base.clone()];
    while let Some(d) = stack.pop() {
        let depth = d.strip_prefix(&base).map(|r| r.components().count()).unwrap_or(0);
        if depth > 2 {
            continue;
        }
        if paths::has_project(&d) {
            cands.push(d.clone());
        }
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if e.path().is_dir() && !name.starts_with('.') && name != "target" && name != "node_modules" {
                    stack.push(e.path());
                }
            }
        }
    }
    cands.sort_by_key(|p| std::cmp::Reverse(paths::project_path(p).metadata().and_then(|m| m.modified()).ok()));
    if cands.is_empty() {
        return None;
    }
    println!("CutForge 编辑器 —— 选择工程(回车 = 最近工程):");
    for (i, p) in cands.iter().take(12).enumerate() {
        println!("  [{}] {}", i + 1, p.display());
    }
    print!("编号: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    let idx: usize = line.trim().parse().unwrap_or(1);
    cands.get(idx.saturating_sub(1)).cloned()
}

/// serve 目标解析(册六 T6.4):`.cfproj` 工程描述文件 → 其 root(安装器文件关联的
/// 「双击打开」落点);其余参数按工程目录原样。单一实现:cli serve 与 mcp serve 共用。
pub fn resolve_root_arg(arg: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(arg);
    let is_cfproj = p.extension().and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cfproj"));
    if is_cfproj {
        return cutforge_io::library::parse_cfproj(&p);
    }
    Ok(p)
}

/// 端口占用时的换端口搜索窗(与 cli doctor 的 PORT_WINDOW 同一口径:+1..+20)。
const PORT_WINDOW: u16 = 20;

/// 工作区常驻服务(M10 本地服务化):静态托管 Web 编辑器 + /rpc + /events +
/// /session 会话信息 + /media(E2)+ /media/browse 与 /ui-fields(E3/E4)。
/// 随机 token 落盘 `.cutforge/session`(仅 127.0.0.1)。
pub fn serve_workspace(root: &Path, port: u16, token: &str, web_dir: &Path, open_browser: bool) -> i32 {
    use std::sync::Arc;
    let _ = cutforge_io::watcher::ensure_sync_daemon(root);
    if let Err(e) = serve_preflight(root, web_dir) {
        eprintln!("启动中止:{e}");
        return 3;
    }
    // E6-2 首次运行体验:.cutforge/ 记账面一次建齐并打印位置。锁文件不预建——
    // 它由首次写操作的 create_exclusive 自动创建并随操作结束释放(预占会挡住 open_exclusive)。
    for sub in [".cutforge", ".cutforge/bases", ".cutforge/oplog"] {
        let _ = std::fs::create_dir_all(root.join(sub));
    }
    // T6.4 端口纪律:先绑定、后写会话记账(session.port 必须是**实际**端口)。
    // 请求端口被占 → 自动换 +1..+20 首个空闲并打横幅(cli serve 与 mcp serve 单一实现;
    // 此前 cli 侧静默换、mcp 侧直接失败,两面不一致且用户不知情);全窗占满才失败。
    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(bind_err) => {
            let alt = (port.saturating_add(1)..=port.saturating_add(PORT_WINDOW))
                .find(|p| std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok())
                .and_then(|p| std::net::TcpListener::bind(("127.0.0.1", p)).ok());
            match alt {
                Some(l) => {
                    eprintln!("⚠ 端口 {port} 已被占用({bind_err}),已自动改用 {}(仅监听 127.0.0.1;排查占用:netstat -ano | findstr :{port})", l.local_addr().map(|a| a.port()).unwrap_or(0));
                    l
                }
                None => {
                    eprintln!("bind 失败:{bind_err}(端口 {port}..+{PORT_WINDOW} 全部被占用;补救:关闭占用它的旧服务窗口,netstat -ano | findstr :{port} 排查)");
                    return 4;
                }
            }
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    let session = json!({
        "root": root.to_string_lossy(),
        "port": port,
        "token": token,
        "pid": std::process::id(),
        "startedAt": cutforge_core::timeutil::now_rfc3339(),
        // 目录契约 0.5:project.json 的工程内相对路径(新布局中文目录;旧布局回退英文),
        // 供壳组装外部脚本参数(如 rs_render 的工程路径)时使用,壳不得硬编码目录名
        "projectRel": paths::project_rel_on_disk(root),
    });
    let dir = root.join(".cutforge");
    let _ = std::fs::create_dir_all(&dir);
    // 唯一落盘点纪律:session 记账也走 atomic.rs(check-write-paths 口径)
    let _ = cutforge_io::atomic::atomic_write(&dir.join("session"), &serde_json::to_vec_pretty(&session).unwrap());
    let session_str = session.to_string();
    let root_s = root.to_string_lossy().to_string();
    let web = Arc::new(web_dir.to_path_buf());
    // RT-1:会话变更摘要从 serve 启动即建档,写操作后增量落盘(Ctrl+C/崩溃也不丢)
    session_journal_begin(root);
    eprintln!("── 首次运行/会话位置 ──");
    eprintln!("  会话: {}", dir.join("session").display());
    eprintln!("  写锁: {}(首次写操作时自动创建/释放)", dir.join("lock").display());
    eprintln!("  基线快照: {}", dir.join("bases").display());
    eprintln!("  本次变更摘要: {}", session_summary_path(root).display());
    eprintln!("────────────────────────");
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

/// E2-1:GET /media?path=<工程内相对路径>。
/// 安全:①只收工程内相对路径;②canonicalize 后必须仍位于工程根之内(拒绝对外穿越);
/// 鉴权走数据面统一 token(非静态白名单);支持 Range(浏览器 seek 的前提)。
/// 路径校验统一走 resolve_within_root(与 /media/browse、clip_add、media_probe 同一实现)。
fn media_response(root: &Path, path_param: Option<&str>, range: Option<&str>) -> HttpResp {
    use std::io::{Read as _, Seek as _};
    let Some(rel) = path_param else {
        return resp_plain("400 Bad Request", "缺 path 参数");
    };
    let canon_t = match resolve_within_root(root, rel) {
        Ok(p) => p,
        Err("非法路径") => return resp_plain("400 Bad Request", "非法路径"),
        Err(_) => return resp_plain("404 Not Found", "媒体不存在"),
    };
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
    // 连接纪律(T1.6/AC-1.7):请求读取统一走 http::read_request(读超时/头体上限/
    // 总时限,慢连接在此被隔离回收);与辅通道单一实现,不再各写一份读循环。
    let req = match http::read_request(&mut stream) {
        Ok(r) => r,
        Err(crate::transport::ReadFail::TooLarge) => {
            let _ = write!(stream, "{}", http::RESP_TOO_LARGE);
            return Ok(());
        }
        // 挂死/半途而废的连接:直接回收,不回写
        Err(crate::transport::ReadFail::Closed) => return Ok(()),
    };
    let first_line = req.first_line().to_string();
    let authorized = req.head.contains(&format!("Authorization: Bearer {token}"))
        || first_line.contains(&format!("token={token}"));
    let raw_path = first_line.split(' ').nth(1).unwrap_or("");
    let (path_only, query) = raw_path.split_once('?').unwrap_or((raw_path, ""));
    // 静态面(T1.6):旧四别名 + /assets/ 目录映射;数据面口径零变化
    let is_get_session = path_only == "/session";
    let is_get_static = static_files::is_static_path(path_only);
    let is_rpc = path_only == "/rpc" && first_line.starts_with("POST");
    let is_events = path_only == "/events";
    let is_media = path_only == "/media" && first_line.starts_with("GET");
    // E3-3 素材浏览 + E4-2 检查器字段真相源:数据面(带 token),不进静态白名单
    let is_media_browse = path_only == "/media/browse" && first_line.starts_with("GET");
    let is_ui_fields = path_only == "/ui-fields" && first_line.starts_with("GET");
    // 册四 T4.5/T4.6:转场/特效/动效目录(壳转场与特效面板的数据面;/ui-fields 同风格,
    // 编译期嵌入,不经 MCP 工具——工具数四则口径不变)
    let is_catalogs = path_only == "/catalogs" && first_line.starts_with("GET");
    let range = req.header("range");
    let body = req.body.as_str();

    // 静态资源公开(纯客户端代码,无秘密);数据面(/session /rpc /events /media)必须持 token
    if !authorized && !is_get_static {
        let _ = write!(stream, "{}", http::RESP_UNAUTHORIZED);
        return Ok(());
    }
    // SSE(T1.6/AC-1.5):Accept 头点名 text/event-stream → 单向流式推送。
    // 旧壳 fetch 不带该头,自然落进下方长轮询分支,行为零变化(兼容红线)。
    if is_events && req.header("accept").is_some_and(|v| v.to_ascii_lowercase().contains("text/event-stream")) {
        return events::serve_sse(&mut stream, Path::new(root), query, req.header("last-event-id").as_deref());
    }
    let resp: HttpResp = if is_get_static {
        static_files::static_resp(web, path_only, req.header("if-none-match").as_deref())
    } else if is_media {
        let path_param = query.split('&').find_map(|kv| {
            let mut it = kv.split('=');
            match (it.next(), it.next()) {
                (Some("path"), Some(v)) => Some(pct_decode(v)),
                _ => None,
            }
        });
        media_response(Path::new(root), path_param.as_deref(), range.as_deref())
    } else if is_media_browse {
        // E3-3:素材浏览(与 media_browse 工具同一 payload 实现,不建并行)
        let dir = query.split('&').find_map(|kv| {
            let mut it = kv.split('=');
            match (it.next(), it.next()) {
                (Some("dir"), Some(v)) => Some(pct_decode(v)),
                _ => None,
            }
        }).unwrap_or_default();
        match media_browse_payload(Path::new(root), &dir) {
            Ok(doc) => HttpResp { status: "200 OK", ctype: "application/json".into(), extra: String::new(), body: doc.to_string().into_bytes() },
            Err(m) => HttpResp {
                status: "400 Bad Request", ctype: "application/json".into(), extra: String::new(),
                // T1.7 三面同码:此面错误也带 ns(加法字段;code 取值不变)
                body: json!({"ok": false, "code": "PRECONDITION_FAILED",
                    "ns": crate::code_namespace("PRECONDITION_FAILED"), "message": m}).to_string().into_bytes(),
            },
        }
    } else if is_ui_fields {
        // E4-2 单一真相源下发:壳检查器分组由此渲染(壳不读文件系统,壳纯度)
        let doc: Value = serde_json::from_str(UI_FIELDS_JSON)
            .expect("schemas/ui-fields.json 必须合法(受 check-ui-fields 机械校验)");
        HttpResp { status: "200 OK", ctype: "application/json".into(), extra: String::new(), body: doc.to_string().into_bytes() }
    } else if is_catalogs {
        // 册四 T4.5/T4.6 目录下发:转场(58 实测)+ 特效/动效(渲染端 catalog 模块同源)
        let doc = json!({
            "transition": serde_json::from_str::<Value>(TRANSITION_CATALOG_JSON)
                .expect("schemas/transition-catalog.json 必须合法"),
            "fx": serde_json::from_str::<Value>(FX_CATALOG_JSON)
                .expect("schemas/fx-catalog.json 必须合法"),
            "huazi": serde_json::from_str::<Value>(HUAZI_CATALOG_JSON)
                .expect("schemas/huazi-catalog.json 必须合法"),
        });
        HttpResp { status: "200 OK", ctype: "application/json".into(), extra: String::new(), body: doc.to_string().into_bytes() }
    } else if is_get_session {
        HttpResp { status: "200 OK", ctype: "application/json".into(), extra: String::new(), body: session_str.as_bytes().to_vec() }
    } else if is_rpc {
        let tool_name = serde_json::from_str::<Value>(body).ok()
            .and_then(|req| req["params"]["name"].as_str().map(String::from));
        let v = match serde_json::from_str::<Value>(body) {
            // 数据面 = 编辑器壳:user 归因(RT-1 摘要的过滤依据)
            Ok(req) => handle_rpc_as(&req, Actor::user("editor")).map(|r| r.to_string()).unwrap_or_default(),
            Err(e) => json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}).to_string(),
        };
        // RT-1:成功的写操作 → 增量更新 .cutforge/session-summary.json
        if tool_name.as_deref().is_some_and(produces_rev_mutation) {
            let parsed: Value = serde_json::from_str(&v).unwrap_or(Value::Null);
            let env_text = parsed["result"]["content"][0]["text"].as_str().unwrap_or("");
            if let Ok(env) = serde_json::from_str::<Value>(env_text)
                && env["ok"] == json!(true)
                && let Some(rev) = env["data"]["rev"].as_u64() {
                    session_journal_note(Path::new(root), rev);
                }
        }
        HttpResp { status: "200 OK", ctype: "application/json".into(), extra: String::new(), body: v.into_bytes() }
    } else if is_events {
        // 长轮询降级路径(A1-R2:兼容旧壳,册二完成后移除;负载老字段一个不少,
        // ok/code/event/seq 原样);新壳走上方 SSE,事件面经 transport::events。
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
    use std::io::Read as _;
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
