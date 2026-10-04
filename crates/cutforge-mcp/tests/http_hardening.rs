//! AC-1.7 门禁:HTTP 连接纪律(真实 socket,起真实 serve_workspace)。
//!
//! 断言链:
//!   1. 隔离性:一条挂死连接(请求头发一半、永不补完)存在时,其余并发请求
//!      (/session 数据面 + /rpc 写路径)全部正常应答,总耗时有上界;
//!   2. 回收:挂死连接在读超时(10s)后被服务端断开(对端读到 EOF/RST),
//!      回收后服务仍健康。超时值给足余量,避免 CI 抖动。
//!   3. 体积上限:Content-Length 超过 8 MiB 的请求立即 413,且服务端不读体。
//!
//! Windows 注意:std 读超时走每读一查的实现,挂死连接约在 READ_TIMEOUT(10s)
//! 被回收;断言截止给到 10s + 15s 余量。

use serde_json::json;
use std::io::{Read as _, Write as _};
use std::net::{TcpStream, UdpSocket};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const TOKEN: &str = "hardening-token";
/// 回收判定的截止:读超时 10s + 15s 余量(注释兑现:CI 抖动不误报)。
const RECLAIM_DEADLINE: Duration = Duration::from_secs(25);

/// 随机空闲端口(bind :0 探测;极小概率竞态,服务起不来会被就绪等待兜住报错)。
fn free_port() -> u16 {
    let s = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    let port = s.local_addr().unwrap().port();
    drop(s);
    // UDP 探测端口同时大概率可用于 TCP;再让 TCP bind 试一下,失败则换号
    match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => {
            let p = l.local_addr().unwrap().port();
            drop(l);
            p
        }
        Err(_) => free_port(),
    }
}

/// 起一个真实 serve(临时工程 + 仓库内 apps/web),阻塞到 /session 可答。
fn spawn_serve(tag: &str) -> (u16, PathBuf) {
    let root = std::env::temp_dir().join(format!("cf-harden-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("05_时间线工程")).unwrap();
    std::fs::write(
        root.join("05_时间线工程").join("project.json"),
        br#"{"rev":1}"#,
    )
    .unwrap();
    let web = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/web");
    let port = free_port();
    {
        let root = root.clone();
        std::thread::spawn(move || {
            // serve 不返回(accept 循环);测试进程退出即随之回收
            let _ = cutforge_mcp::serve_workspace(&root, port, TOKEN, &web, false);
        });
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if Instant::now() > deadline {
            panic!("serve 未就绪(15s):{tag}");
        }
        if let Ok((code, _, _)) = http_roundtrip(
            port,
            "GET",
            "/session",
            &[("Authorization", &format!("Bearer {TOKEN}"))],
            None,
        ) && code == 200
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    (port, root)
}

/// 一次完整 HTTP 往返(原始 socket;返回 状态码/头/体)。
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
    let head_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(buf.len());
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let code: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    Ok((code, head, buf[head_end..].to_vec()))
}

/// 挂死连接:请求头发一半、永不补完、也不断开。
fn hang_connection(port: u16) -> TcpStream {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    s.write_all(b"GET /session?token=hardening-token HTTP/1.1\r\nHost: x\r\n")
        .unwrap();
    s.flush().unwrap();
    s
}

/// AC-1.7 主断言:挂死连接存在 → 并发请求照常;超时后连接被回收;服务仍健康。
#[test]
fn hung_connection_does_not_block_others_and_is_reclaimed() {
    let (port, ws) = spawn_serve("iso");
    let mut hung = hang_connection(port);

    // 1) 隔离性:挂死连接压在服务上,其余请求必须照常应答(数据面 + RPC 写路径探测)
    let t0 = Instant::now();
    for i in 0..10 {
        let (code, _, body) = http_roundtrip(
            port,
            "GET",
            "/session",
            &[("Authorization", &format!("Bearer {TOKEN}"))],
            None,
        )
        .unwrap_or_else(|e| panic!("第 {i} 个并发请求失败: {e}"));
        assert_eq!(code, 200, "并发 /session 应 200,实得 {code}");
        // S-01 起响应体不含主 token,但会话元数据(root)必须在
        assert!(body.windows(4).any(|w| w == b"root"), "/session 负载异常");
    }
    let rpc_body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "capability_matrix", "arguments": {}}
    })
    .to_string();
    let (code, _, body) = http_roundtrip(
        port,
        "POST",
        "/rpc",
        &[
            ("Authorization", &format!("Bearer {TOKEN}")),
            ("Content-Type", "application/json"),
        ],
        Some(&rpc_body),
    )
    .unwrap();
    assert_eq!(code, 200, "挂死连接存在时 /rpc 必须照常应答,实得 {code}");
    // JSON-RPC 信封在,text 内嵌 JSON 会被转义,断言信封字段而非内层 ok
    assert!(
        String::from_utf8_lossy(&body).contains("\"result\""),
        "/rpc 负载异常"
    );
    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "挂死连接存在时其余请求被拖慢:11 个请求共 {elapsed:?}(应秒级内)"
    );
    println!("隔离性:11 个并发请求共 {elapsed:?}(挂死连接在场)");

    // 2) 回收:读超时(10s)后挂死连接被服务端断开(EOF 或 RST 都算回收)
    let deadline = Instant::now() + RECLAIM_DEADLINE;
    let mut reclaimed = false;
    while Instant::now() < deadline {
        match hung.read(&mut [0u8; 64]) {
            Ok(0) => {
                reclaimed = true; // EOF:服务端优雅关闭
                break;
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::ConnectionReset
                    || e.kind() == std::io::ErrorKind::ConnectionAborted =>
            {
                reclaimed = true; // RST:服务端直接丢弃
                break;
            }
            Err(_) => {} // 本端读超时(1s):继续等服务端动手
            Ok(_) => panic!("挂死连接不应收到任何数据"),
        }
    }
    assert!(reclaimed, "挂死连接未在 {RECLAIM_DEADLINE:?} 内被回收");

    // 3) 回收后服务仍健康
    let (code, _, _) = http_roundtrip(
        port,
        "GET",
        "/session",
        &[("Authorization", &format!("Bearer {TOKEN}"))],
        None,
    )
    .unwrap();
    assert_eq!(code, 200, "回收后服务应仍健康");
    let _ = std::fs::remove_dir_all(&ws);
}

/// 体积上限:Content-Length 超限 → 立即 413,服务端不读体(体根本不发)。
#[test]
fn oversized_body_rejected_with_413() {
    let (port, ws) = spawn_serve("cap");
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    // 只发头,声明 1 GiB 的体(>8 MiB 上限),一个字节体都不发
    let head = format!(
        "POST /rpc HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: 1073741824\r\n\r\n"
    );
    s.write_all(head.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let t0 = Instant::now();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    assert!(
        text.starts_with("HTTP/1.1 413"),
        "超限请求应 413,实得:{:.80}",
        text.replace('\r', " ")
    );
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "413 应在头读齐后立即返回,实耗 {:?}",
        t0.elapsed()
    );
    // 上限内的正常小请求不受影响
    let (code, _, _) = http_roundtrip(
        port,
        "GET",
        "/session",
        &[("Authorization", &format!("Bearer {TOKEN}"))],
        None,
    )
    .unwrap();
    assert_eq!(code, 200);
    let _ = std::fs::remove_dir_all(&ws);
}

/// 慢连接隔离(变体):半途静默的连接(timeout 后被回收)不影响后续新连接。
#[test]
fn silent_connection_reclaimed_and_service_stays_healthy() {
    let (port, ws) = spawn_serve("slow");
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(b"GET /session HTTP/1.1\r\n").unwrap(); // 只发一行就静默
    let deadline = Instant::now() + RECLAIM_DEADLINE;
    s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let mut reclaimed = false;
    while Instant::now() < deadline {
        match s.read(&mut [0u8; 16]) {
            Ok(0) => {
                reclaimed = true;
                break;
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::ConnectionReset
                    || e.kind() == std::io::ErrorKind::ConnectionAborted =>
            {
                reclaimed = true;
                break;
            }
            _ => {}
        }
    }
    assert!(reclaimed, "静默连接未在 {RECLAIM_DEADLINE:?} 内被回收");
    // 新连接立即可用
    let (code, _, _) = http_roundtrip(
        port,
        "GET",
        "/session",
        &[("Authorization", &format!("Bearer {TOKEN}"))],
        None,
    )
    .unwrap();
    assert_eq!(code, 200);
    let _ = std::fs::remove_dir_all(&ws);
}

/// 未鉴权数据面仍 401(加固不放松鉴权口径)。
#[test]
fn data_plane_without_token_is_401() {
    let (port, ws) = spawn_serve("auth");
    let (code, _, _) = http_roundtrip(port, "GET", "/session", &[], None).unwrap();
    assert_eq!(code, 401);
    let (code, _, _) = http_roundtrip(
        port,
        "POST",
        "/rpc",
        &[("Content-Type", "application/json")],
        Some("{}"),
    )
    .unwrap();
    assert_eq!(code, 401);
    let _ = std::fs::remove_dir_all(&ws);
}

// ==================== V2-W1 安全轮(BUG-10 / S-01 / S-04 / BUG-11 / S-02 / R-08) ====================

/// 原始字节级请求(头区完全由调用方控制:重复头/续行/大小写)。
fn raw_roundtrip(port: u16, raw: &str) -> std::io::Result<(u16, String, Vec<u8>)> {
    let mut s = TcpStream::connect(("127.0.0.1", port))?;
    s.set_read_timeout(Some(Duration::from_secs(20)))?;
    s.write_all(raw.as_bytes())?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    while let Ok(n) = s.read(&mut tmp) {
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let head_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(buf.len());
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let code: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    Ok((code, head, buf[head_end..].to_vec()))
}

/// 测试用百分号编码(encodeURIComponent 口径:非保留字符外全部 %XX)。
fn pct_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// TC-SEC-007(BUG-10):Bearer 值必须精确相等——`Bearer {token}garbage` 子串
/// 命中必须拒绝(旧实现 head.contains 整段子串匹配,垃圾后缀直接放行)。
#[test]
fn bearer_suffix_garbage_rejected() {
    let (port, ws) = spawn_serve("sec007");
    let (code, _, _) = http_roundtrip(
        port,
        "GET",
        "/session",
        &[("Authorization", &format!("Bearer {TOKEN}garbage"))],
        None,
    )
    .unwrap();
    assert_eq!(code, 401, "Bearer 值带垃圾后缀必须 401(子串匹配漏洞)");
    // 前缀垃圾同理
    let (code, _, _) = http_roundtrip(
        port,
        "GET",
        "/session",
        &[("Authorization", &format!("xBearer {TOKEN}"))],
        None,
    )
    .unwrap();
    assert_eq!(code, 401, "scheme 非法必须 401");
    let _ = std::fs::remove_dir_all(&ws);
}

/// TC-SEC-008(BUG-10):HTTP/1.1 头名大小写不敏感——小写 `authorization:` 必须
/// 与规范拼写同权(旧实现整段包含匹配,头名小写即 401)。
#[test]
fn lowercase_auth_header_accepted() {
    let (port, ws) = spawn_serve("sec008");
    let (code, _, _) = raw_roundtrip(
        port,
        &format!(
            "GET /session HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\
             authorization: Bearer {TOKEN}\r\n\r\n"
        ),
    )
    .unwrap();
    assert_eq!(code, 200, "小写头名是合法 HTTP/1.1,必须放行");
    let (code, _, _) = http_roundtrip(
        port,
        "GET",
        "/session",
        &[("AUTHORIZATION", &format!("Bearer {TOKEN}"))],
        None,
    )
    .unwrap();
    assert_eq!(code, 200, "大写头名同样必须放行");
    let _ = std::fs::remove_dir_all(&ws);
}

/// TC-SEC-009(BUG-10):鉴权失败响应不得差异化——错 token / 垃圾后缀 / 缺头
/// 三种失败必须逐字节同一份 401(不给时序/内容侧信道)。
#[test]
fn auth_failures_are_indistinguishable() {
    let (port, ws) = spawn_serve("sec009");
    let cases = [
        // 错误 token
        raw_roundtrip(
            port,
            "GET /session HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\
             Authorization: Bearer wrong-token\r\n\r\n",
        )
        .unwrap(),
        // 正确 token + 垃圾后缀
        raw_roundtrip(
            port,
            &format!(
                "GET /session HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\
                 Authorization: Bearer {TOKEN}x\r\n\r\n"
            ),
        )
        .unwrap(),
        // 缺头
        http_roundtrip(port, "GET", "/session", &[], None).unwrap(),
    ];
    for (i, (code, head, body)) in cases.iter().enumerate() {
        assert_eq!(*code, 401, "case{i} 必须 401");
        let (_, head0, body0) = &cases[0];
        assert_eq!(head, head0, "case{i} 响应头与基准不一致(差异化泄露)");
        assert_eq!(body, body0, "case{i} 响应体与基准不一致(差异化泄露)");
    }
    let _ = std::fs::remove_dir_all(&ws);
}

/// TC-SEC-010(S-04):query 按 key 精确切分——`mytoken=`/`xtoken=` 子串
/// 不得再误命中 `token=`(旧实现 first_line.contains("token=..."))。
#[test]
fn query_token_exact_key_match() {
    let (port, ws) = spawn_serve("sec010");
    for evil in ["mytoken", "xtoken", "tokenx"] {
        let (code, _, _) =
            http_roundtrip(port, "GET", &format!("/session?{evil}={TOKEN}"), &[], None).unwrap();
        assert_eq!(code, 401, "{evil}={TOKEN} 不是 token=,必须 401");
    }
    let _ = std::fs::remove_dir_all(&ws);
}

/// TC-SEC-011(S-04):重复 Authorization 头 = 请求走私面 → 拒绝(只认首个或
/// 拒绝重复两案取严:拒绝重复)。
#[test]
fn duplicate_authorization_rejected() {
    let (port, ws) = spawn_serve("sec011");
    let (code, _, _) = raw_roundtrip(
        port,
        &format!(
            "GET /session HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\
             Authorization: Bearer {TOKEN}\r\nAuthorization: Bearer {TOKEN}\r\n\r\n"
        ),
    )
    .unwrap();
    assert_eq!(code, 401, "重复 Authorization 头必须拒绝(防走私)");
    let _ = std::fs::remove_dir_all(&ws);
}

/// TC-SEC-012(S-04):obs-fold 续行头 → 拒绝(不折叠;旧实现整段 contains
/// 会把跨行值当合法命中)。
#[test]
fn obs_fold_authorization_rejected() {
    let (port, ws) = spawn_serve("sec012");
    let (code, _, _) = raw_roundtrip(
        port,
        &format!(
            "GET /session HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\
             Authorization: Bearer {TOKEN}\r\n extra\r\n\r\n"
        ),
    )
    .unwrap();
    assert_eq!(code, 401, "带续行的 Authorization 头必须拒绝");
    let _ = std::fs::remove_dir_all(&ws);
}

/// TC-SEC-030(S-01):URL ?token= 兼容一版——放行但必须带 Deprecation 响应头。
#[test]
fn url_token_still_works_with_deprecation_header() {
    let (port, ws) = spawn_serve("sec030");
    let (code, head, _) =
        http_roundtrip(port, "GET", &format!("/session?token={TOKEN}"), &[], None).unwrap();
    assert_eq!(code, 200, "兼容版内 URL token 仍应放行");
    assert!(
        head.contains("Deprecation:"),
        "URL token 鉴权的响应必须带 Deprecation 头: {head}"
    );
    // Bearer 头鉴权不带 Deprecation(只有 URL token 面被判弃用)
    let (_, head2, _) = http_roundtrip(
        port,
        "GET",
        "/session",
        &[("Authorization", &format!("Bearer {TOKEN}"))],
        None,
    )
    .unwrap();
    assert!(
        !head2.contains("Deprecation:"),
        "Bearer 鉴权不应带 Deprecation"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

/// TC-SEC-031(S-01):/session 响应体不再携带主 token——改发一次性短期凭据
/// (绑定 sessionId、单次使用、5 分钟过期);凭据可恰好兑换一次会话 token。
#[test]
fn session_response_hides_master_token() {
    let (port, ws) = spawn_serve("sec031");
    let (code, _, body) = http_roundtrip(
        port,
        "GET",
        "/session",
        &[("Authorization", &format!("Bearer {TOKEN}"))],
        None,
    )
    .unwrap();
    assert_eq!(code, 200);
    let text = String::from_utf8_lossy(&body);
    assert!(
        !text.contains(TOKEN),
        "/session 响应体不得携带主 token(黄金断言): {text}"
    );
    let doc: serde_json::Value = serde_json::from_str(text.trim()).expect("/session 必须是 JSON");
    let sid = doc["sessionId"].as_str().expect("必须携带 sessionId");
    let cred = doc["credential"].as_str().expect("必须携带一次性凭据");
    assert!(
        !cred.is_empty() && cred != TOKEN,
        "凭据必须存在且不等于主 token"
    );
    assert_eq!(doc["expiresInSec"], serde_json::json!(300), "5 分钟有效期");
    // 一次性兑换:第一次 200 出会话 token,第二次 401(单次使用)
    let req_body = serde_json::json!({"sessionId": sid}).to_string();
    let (code1, _, body1) = http_roundtrip(
        port,
        "POST",
        "/session/exchange",
        &[
            ("Authorization", &format!("Bearer {cred}")),
            ("Content-Type", "application/json"),
        ],
        Some(&req_body),
    )
    .unwrap();
    assert_eq!(
        code1,
        200,
        "首次兑换必须成功: {}",
        String::from_utf8_lossy(&body1)
    );
    let ex: serde_json::Value = serde_json::from_slice(&body1).unwrap();
    assert!(
        ex["sessionToken"]
            .as_str()
            .is_some_and(|t| !t.is_empty() && t != TOKEN),
        "兑换出的会话 token 不得是主 token: {ex}"
    );
    let (code2, _, _) = http_roundtrip(
        port,
        "POST",
        "/session/exchange",
        &[
            ("Authorization", &format!("Bearer {cred}")),
            ("Content-Type", "application/json"),
        ],
        Some(&req_body),
    )
    .unwrap();
    assert_eq!(code2, 401, "同一凭据第二次兑换必须 401(单次使用)");
    let _ = std::fs::remove_dir_all(&ws);
}

/// TC-SEC-032(S-01):会话记账文件收紧——内容只含短期凭据(不含主 token);
/// Unix 上权限 0600(Windows 无纯 std ACL 面,以内容断言为准)。
#[test]
fn session_file_contains_no_master_token() {
    let (_, ws) = spawn_serve("sec032");
    let sess_path = ws.join(".cutforge").join("session");
    let text = std::fs::read_to_string(&sess_path).expect("会话文件必须存在");
    assert!(!text.contains(TOKEN), "会话文件不得携带主 token: {text}");
    let doc: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
    assert!(
        doc["credential"].as_str().is_some(),
        "会话文件必须含一次性凭据"
    );
    assert!(
        doc["sessionId"].as_str().is_some(),
        "会话文件必须含 sessionId"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&sess_path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "会话文件权限必须 0600,实得 {:o}",
            mode & 0o777
        );
    }
    let _ = std::fs::remove_dir_all(&ws);
}

/// TC-MCP-SSE-001/002(BUG-11):SSE root 查询参数必须 pct_decode——中文/空格
/// 路径订阅后,改动工程内文件必须能在流上收到 workspace.changed
/// (旧实现拿字面 `%E4..` 串当路径,事件永不到达)。
#[test]
fn sse_root_pct_decoded_cjk_and_space() {
    let port = free_port();
    std::thread::spawn(move || {
        let _ = cutforge_mcp::serve_http(port, TOKEN);
    });
    // 就绪等待:辅通道起 accept 后任意请求应有响应(401/200 都算就绪)
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if Instant::now() > deadline {
            panic!("辅通道 serve 未就绪");
        }
        if let Ok((code, _, _)) = http_roundtrip(port, "GET", "/session", &[], None)
            && code == 401
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    for tag in ["中文 目录", "has space"] {
        let root = std::env::temp_dir().join(format!("cf-sse-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("05_时间线工程")).unwrap();
        std::fs::write(
            root.join("05_时间线工程").join("project.json"),
            br#"{"rev":1}"#,
        )
        .unwrap();
        let url = format!(
            "/events?root={}&since=0",
            pct_encode(&root.to_string_lossy())
        );
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
        let req = format!(
            "GET {url} HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/event-stream\r\n\
             Authorization: Bearer {TOKEN}\r\n\r\n"
        );
        s.write_all(req.as_bytes()).unwrap();
        // 1) 流建立(: connected)
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        let t_conn = Instant::now();
        while Instant::now() - t_conn < Duration::from_secs(5) {
            match s.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    if let Ok(t) = std::str::from_utf8(&buf)
                        && t.contains(": connected")
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        assert!(
            std::str::from_utf8(&buf)
                .unwrap_or("")
                .contains(": connected"),
            "[{tag}] SSE 流必须建立: {}",
            String::from_utf8_lossy(&buf)
        );
        // 2) 外部改动 → 流上收到 workspace.changed(根路径解码正确的唯一可观察证据)
        std::thread::sleep(Duration::from_millis(400)); // 让 watcher 先扫一轮基线
        let pj = root.join("05_时间线工程").join("project.json");
        std::fs::write(&pj, br#"{"rev":2}"#).unwrap();
        let mut seen = false;
        let t_evt = Instant::now();
        buf.clear();
        while Instant::now() - t_evt < Duration::from_secs(8) {
            match s.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
                Err(_) => {} // 读超时继续等
            }
            if let Ok(t) = std::str::from_utf8(&buf)
                && t.contains("workspace.changed")
            {
                seen = true;
                break;
            }
        }
        assert!(
            seen,
            "[{tag}] pct 编码 root 订阅必须能收到 workspace.changed: {}",
            String::from_utf8_lossy(&buf)
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// TC-MCP-MEDIA-001(R-08):/media 流式转发行为面——整文件 200 与区间 206
/// 字节全对(>64KB 跨缓冲界;内存恒定由 io::copy 64KB 缓冲保证,500MB RSS
/// 实测见工单报告)。
#[test]
fn media_range_and_full_serving_byte_exact() {
    let (port, ws) = spawn_serve("media1");
    // 256KB 伪随机图案文件(LCG;确定性)
    let mut data = vec![0u8; 262_144];
    let mut x: u32 = 0x1234_5678;
    for b in data.iter_mut() {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *b = (x >> 24) as u8;
    }
    std::fs::write(ws.join("pattern.bin"), &data).unwrap();

    // 整文件 200:Content-Length == 大小,体逐字节相等
    let (code, head, body) = http_roundtrip(
        port,
        "GET",
        "/media?path=pattern.bin",
        &[("Authorization", &format!("Bearer {TOKEN}"))],
        None,
    )
    .unwrap();
    assert_eq!(code, 200);
    assert!(
        head.contains(&format!("Content-Length: {}\r\n", data.len())),
        "整文件 Content-Length 必须等于文件大小: {head}"
    );
    assert_eq!(body.len(), data.len(), "整文件体必须完整(流式不截断)");
    assert_eq!(body, data, "整文件体必须逐字节相等");

    // 区间 206:bytes=100-70000(>64KB 跨缓冲界)
    let (code, head, body) = http_roundtrip(
        port,
        "GET",
        "/media?path=pattern.bin",
        &[
            ("Authorization", &format!("Bearer {TOKEN}")),
            ("Range", "bytes=100-70000"),
        ],
        None,
    )
    .unwrap();
    assert_eq!(code, 206);
    assert!(
        head.contains("Content-Range: bytes 100-70000/262144\r\n"),
        "Content-Range 必须如实: {head}"
    );
    assert_eq!(body.len(), 70_000 - 100 + 1);
    assert_eq!(body, data[100..=70000], "区间体必须逐字节相等");

    // 后缀区间:suffix=1000
    let (code, _, body) = http_roundtrip(
        port,
        "GET",
        "/media?path=pattern.bin",
        &[
            ("Authorization", &format!("Bearer {TOKEN}")),
            ("Range", "bytes=-1000"),
        ],
        None,
    )
    .unwrap();
    assert_eq!(code, 206);
    assert_eq!(body, data[data.len() - 1000..], "后缀区间必须对");
    let _ = std::fs::remove_dir_all(&ws);
}

/// TC-MCP-MEDIA-002(R-08):单 Range 上限 clamp 256MB——显式区间超上限 → 416,
/// 不再整段读入内存。夹具用 NTFS/ext4 稀疏文件(set_len,零拷贝 300MB)。
#[test]
fn media_range_over_cap_rejected_with_416() {
    let (port, ws) = spawn_serve("media2");
    let big = ws.join("big.bin");
    {
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(300 * 1024 * 1024).unwrap(); // 300MB 稀疏文件(> 256MB 上限)
    }
    let (code, _, _) = http_roundtrip(
        port,
        "GET",
        "/media?path=big.bin",
        &[
            ("Authorization", &format!("Bearer {TOKEN}")),
            ("Range", &format!("bytes=0-{}", 300 * 1024 * 1024 - 1)),
        ],
        None,
    )
    .unwrap();
    assert_eq!(code, 416, "显式区间超 256MB 上限必须 416");
    // 上限内的区间照常 206(文件真实存在的头部可读)
    let (code, _, body) = http_roundtrip(
        port,
        "GET",
        "/media?path=big.bin",
        &[
            ("Authorization", &format!("Bearer {TOKEN}")),
            ("Range", "bytes=0-1023"),
        ],
        None,
    )
    .unwrap();
    assert_eq!(code, 206, "上限内的区间必须照常服务");
    assert_eq!(body.len(), 1024);
    let _ = std::fs::remove_dir_all(&ws);
}

/// TC-SEC-040(S-02):连接计数上限——占满名额后,新连接立即 503+Retry-After,
/// 不进读循环;名额释放后服务恢复。测试钩子以小上限起服务(生产缺省 64)。
#[test]
fn connection_limit_returns_503_with_retry_after() {
    let port = free_port();
    std::thread::spawn(move || {
        let _ = cutforge_mcp::_serve_http_with_limit(port, TOKEN, 2);
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if Instant::now() > deadline {
            panic!("限流 serve 未就绪");
        }
        if let Ok((code, _, _)) = http_roundtrip(port, "GET", "/session", &[], None)
            && code == 401
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // 占满 2 个名额(静默连接:不发送任何数据,压在读超时上)
    let mut held = Vec::new();
    for _ in 0..2 {
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        held.push(s);
        std::thread::sleep(Duration::from_millis(150)); // 让服务端 accept 完成
    }
    // 第 3 个连接 → 503 + Retry-After
    let (code, head, _) = raw_roundtrip(
        port,
        &format!(
            "GET /session HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\
             Authorization: Bearer {TOKEN}\r\n\r\n"
        ),
    )
    .unwrap();
    assert_eq!(code, 503, "超限连接必须 503");
    assert!(
        head.contains("Retry-After:"),
        "503 必须带 Retry-After: {head}"
    );
    // 释放名额后服务恢复
    drop(held);
    std::thread::sleep(Duration::from_millis(300));
    let (code, _, _) = http_roundtrip(
        port,
        "GET",
        "/session",
        &[("Authorization", &format!("Bearer {TOKEN}"))],
        None,
    )
    .unwrap();
    assert_eq!(code, 200, "名额释放后必须恢复 200");
}
