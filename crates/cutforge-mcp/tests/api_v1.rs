//! AC-7.1 门禁:/api/v1 版本化 Editor API(册七 T7.1,真实 socket,起真实 serve_workspace)。
//!
//! 断言链:
//!   1. 统一工具入口:POST /api/v1/tools/capability_matrix → 200 + envelope(ok=true);
//!      POST /api/v1/tools/project_get(绑定根注入)→ 200 + 工程视图;
//!      写路径转发(POST clip_add)rev 前进——单一 dispatch 单表(非第二套逻辑);
//!   2. GET 别名:/api/v1/project、/api/v1/timeline、/api/v1/notes → 200 + envelope;
//!      /api/v1/capabilities → 200;查询串参数透传(/api/v1/oplog?limit=1);
//!   3. 事件流:GET /api/v1/events 长轮询 → 200 + ok/code/event/seq 老字段齐
//!      (SSE 由 e2e_events 对 /events 的既有断言覆盖,同一 serve_sse 实现);
//!   4. 协议边角:无 token → 401;未知工具 → 404 + INTERNAL;未知端点 → 404;
//!      GET 工具入口 → 405;非法体 → 400 + SCHEMA_INVALID;
//!   5. /rpc 兼容别名(ADR-0025):既有 POST /rpc 行为零变化。

use serde_json::{json, Value};
use std::io::{Read as _, Write as _};
use std::net::{TcpStream, UdpSocket};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const TOKEN: &str = "api-v1-token";

fn free_port() -> u16 {
    let s = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    let port = s.local_addr().unwrap().port();
    drop(s);
    match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => {
            let p = l.local_addr().unwrap().port();
            drop(l);
            p
        }
        Err(_) => free_port(),
    }
}

fn http_roundtrip(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> std::io::Result<(u16, String, Vec<u8>)> {
    let mut s = TcpStream::connect(("127.0.0.1", port))?;
    s.set_read_timeout(Some(Duration::from_secs(20)))?;
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes())?;
    if let Some(b) = body {
        s.write_all(b.as_bytes())?;
    }
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    while let Ok(n) = s.read(&mut tmp) {
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let head_end = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4).unwrap_or(buf.len());
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let code: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    Ok((code, head, buf[head_end..].to_vec()))
}

fn get(port: u16, path: &str) -> (u16, Value) {
    let (code, _, body) = http_roundtrip(
        port, "GET", path,
        &[("Authorization", &format!("Bearer {TOKEN}"))], None,
    ).unwrap();
    (code, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

fn post(port: u16, path: &str, body: &str) -> (u16, Value) {
    let (code, _, body) = http_roundtrip(
        port, "POST", path,
        &[("Authorization", &format!("Bearer {TOKEN}")), ("Content-Type", "application/json")],
        Some(body),
    ).unwrap();
    (code, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

fn assert_envelope(env: &Value, what: &str) {
    for key in ["ok", "code", "message", "data"] {
        assert!(env.get(key).is_some(), "{what}: 缺协议字段 {key}: {env}");
    }
}

#[test]
fn api_v1_rest_surface_end_to_end() {
    // 夹具工程(05_ir 旧布局兼容面,与 http_hardening 同款最小工程)
    let root = std::env::temp_dir().join(format!("cf-api-v1-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("05_时间线工程")).unwrap();
    let project = "{\"version\":1,\"schemaVersion\":\"3.0.0\",\"slug\":\"api面\",\"fps\":30,\
        \"canvas\":{\"width\":320,\"height\":240},\"backends\":[\"ffmpeg\"],\
        \"tracks\":[{\"id\":\"V1\",\"kind\":\"video\",\"clips\":[]}]}";
    std::fs::write(root.join("05_时间线工程").join("project.json"), project.as_bytes()).unwrap();
    let web = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/web");
    let port = free_port();
    {
        let root = root.clone();
        std::thread::spawn(move || {
            let _ = cutforge_mcp::serve_workspace(&root, port, TOKEN, &web, false);
        });
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if Instant::now() > deadline {
            panic!("serve 未就绪(15s)");
        }
        if let Ok((200, _, _)) = http_roundtrip(port, "GET", "/session", &[("Authorization", &format!("Bearer {TOKEN}"))], None) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    // ---- 1) 统一工具入口:capability_matrix 无需 root → 200 + envelope ----
    let (code, env) = post(port, "/api/v1/tools/capability_matrix", "{}");
    assert_eq!(code, 200, "capability_matrix 必须 200: {env}");
    assert_envelope(&env, "capability_matrix");
    assert_eq!(env["code"], json!("OK"));
    assert!(env["data"]["matrix"].is_object(), "{env}");

    // 绑定根注入:project_get 不带 root → 200 + 工程视图
    let (code, env) = post(port, "/api/v1/tools/project_get", "{}");
    assert_eq!(code, 200, "{env}");
    assert_eq!(env["data"]["project"]["slug"], json!("api面"), "{env}");
    let rev0 = env["data"]["rev"].as_u64().unwrap();

    // 写路径转发(单一 dispatch 单表证明):clip_add → rev 前进
    std::fs::write(root.join("media_b.mp4"), b"fake").unwrap();
    let (code, env) = post(port, "/api/v1/tools/clip_add",
        &json!({"trackId": "V1", "src": "media_b.mp4", "startMs": 0, "durationMs": 1500,
                "requestId": "api-v1-add-1"}).to_string());
    assert_eq!(code, 200, "{env}");
    assert_eq!(env["code"], json!("OK"), "{env}");
    let rev1 = env["data"]["rev"].as_u64().unwrap();
    assert_eq!(rev1, rev0 + 1, "REST 写路径 = 同一 apply 通道(rev 前进)");
    let (_, env) = post(port, "/api/v1/tools/timeline_get", "{}");
    assert_eq!(env["data"]["clips"].as_array().map(Vec::len), Some(1), "{env}");

    // ---- 2) GET 别名:同参转发(查询串即参数) ----
    let (code, env) = get(port, "/api/v1/project");
    assert_eq!(code, 200, "{env}");
    assert_eq!(env["data"]["project"]["slug"], json!("api面"));
    let (code, env) = get(port, "/api/v1/timeline");
    assert_eq!(code, 200, "{env}");
    assert_eq!(env["data"]["clips"].as_array().map(Vec::len), Some(1));
    let (code, env) = get(port, "/api/v1/notes");
    assert_eq!(code, 200, "{env}");
    assert_eq!(env["data"]["total"], json!(0));
    let (code, env) = get(port, "/api/v1/capabilities");
    assert_eq!(code, 200, "{env}");
    let (code, env) = get(port, "/api/v1/oplog?limit=1");
    assert_eq!(code, 200, "{env}");
    assert!(env["data"]["ops"].as_array().unwrap().len() <= 1, "查询串参数透传: {env}");
    let (code, env) = get(port, "/api/v1/tools");
    assert_eq!(code, 200, "{env}");
    let n = env["data"]["tools"].as_array().unwrap().len();
    assert_eq!(n, 83, "GET /api/v1/tools = 注册表全集(单一真相源;册七 A7 78→83): {n}");

    // ---- 3) 事件流别名:长轮询降级面老字段齐(SSE 同一实现) ----
    let (code, env) = get(port, "/api/v1/events");
    assert_eq!(code, 200, "{env}");
    for k in ["ok", "code", "event", "seq"] {
        assert!(env.get(k).is_some(), "/api/v1/events 缺老字段 {k}: {env}");
    }

    // ---- 4) 协议边角 ----
    let (code, _, _) = http_roundtrip(port, "GET", "/api/v1/project", &[("Authorization", "Bearer 错token")], None).unwrap();
    assert_eq!(code, 401, "数据面必须持 token");
    let (code, env) = post(port, "/api/v1/tools/无此工具", "{}");
    assert_eq!(code, 404, "{env}");
    assert_eq!(env["code"], json!("INTERNAL"), "未知工具与 /rpc 同语义");
    let (code, env) = get(port, "/api/v1/无此端点");
    assert_eq!(code, 404, "{env}");
    let (code, _, _) = http_roundtrip(port, "GET", "/api/v1/tools/project_get", &[("Authorization", &format!("Bearer {TOKEN}"))], None).unwrap();
    assert_eq!(code, 405, "工具入口必须 POST");
    let (code, env) = post(port, "/api/v1/tools/project_get", "[1,2]");
    assert_eq!(code, 400, "{env}");
    assert_eq!(env["code"], json!("SCHEMA_INVALID"));

    // ---- 5) /rpc 兼容别名(ADR-0025):行为零变化(壳显式带 root 的既有契约) ----
    let rpc_body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "project_get", "arguments": {"root": root.to_string_lossy()}}}).to_string();
    let (code, _, body) = http_roundtrip(port, "POST", "/rpc",
        &[("Authorization", &format!("Bearer {TOKEN}"))], Some(&rpc_body)).unwrap();
    assert_eq!(code, 200);
    let rpc: Value = serde_json::from_slice(&body).unwrap();
    let env: Value = serde_json::from_str(rpc["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(env["data"]["project"]["slug"], json!("api面"), "/rpc 兼容面零漂移");

    let _ = std::fs::remove_dir_all(&root);
}

/// 辅通道(http serve,/rpc 所在)同享 /api/v1 面:root 须显式给出;
/// 工具调用与 GET 别名同参转发(单表双通道纪律的第三通道)。
#[test]
fn api_v1_on_aux_channel_requires_explicit_root() {
    let root = std::env::temp_dir().join(format!("cf-api-v1-aux-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("05_时间线工程")).unwrap();
    let project = "{\"version\":1,\"schemaVersion\":\"3.0.0\",\"slug\":\"辅通道\",\"fps\":30,\
        \"canvas\":{\"width\":320,\"height\":240},\"backends\":[\"ffmpeg\"],\"tracks\":[]}";
    std::fs::write(root.join("05_时间线工程").join("project.json"), project.as_bytes()).unwrap();
    let port = free_port();
    {
        std::thread::spawn(move || {
            let _ = cutforge_mcp::serve_http(port, TOKEN);
        });
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if Instant::now() > deadline {
            panic!("辅通道 serve 未就绪(15s)");
        }
        if matches!(http_roundtrip(port, "POST", "/rpc",
            &[("Authorization", &format!("Bearer {TOKEN}"))],
            Some(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#)), Ok((200, _, _))) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // 缺 root(辅通道无绑定根)→ 400 + PRECONDITION_FAILED
    let (code, env) = post(port, "/api/v1/tools/project_get", "{}");
    assert_eq!(code, 400, "{env}");
    assert_eq!(env["code"], json!("PRECONDITION_FAILED"), "{env}");
    // 显式 root → 200
    let body = json!({"root": root.to_string_lossy()}).to_string();
    let (code, env) = post(port, "/api/v1/tools/project_get", &body);
    assert_eq!(code, 200, "{env}");
    assert_eq!(env["data"]["project"]["slug"], json!("辅通道"));
    // GET 别名经查询串给 root
    let (code, env) = get(port, &format!("/api/v1/timeline?root={}", utf8_query(&root)));
    assert_eq!(code, 200, "{env}");
    assert_eq!(env["code"], json!("OK"));
    // 事件流别名:缺 root → 长轮询面报缺 root(envelope 协议齐)
    let (code, env) = get(port, "/api/v1/events");
    assert_eq!(code, 200, "{env}");
    assert_eq!(env["code"], json!("PRECONDITION_FAILED"));
    let _ = std::fs::remove_dir_all(&root);
}

/// 路径查询串里的 Windows 绝对路径(盘符冒号 + 反斜杠)转正斜杠 + 百分号编码。
fn utf8_query(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/").chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~' | '/' => c.to_string(),
        ':' => "%3A".into(),
        _ => format!("%{:02X}", c as u32),
    }).collect()
}
