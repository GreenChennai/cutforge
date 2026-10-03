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
        assert!(body.windows(5).any(|w| w == b"token"), "/session 负载异常");
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
