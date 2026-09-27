// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 内嵌 HTTP 辅通道(仅 127.0.0.1 + Bearer token)与 HTTP 响应件、百分号解码、
//! MIME 表、静态资源映射共用件(T1.1 拆分自 lib.rs,纯移动)。

use crate::dispatch::handle_rpc;
use crate::registry::tool_names;
use serde_json::{json, Value};
use std::io::Write as _;
use std::path::Path;

pub(crate) fn static_content(web: &Path, path: &str) -> Option<(&'static str, Vec<u8>)> {
    let rel = match path {
        "/" | "/index.html" => ("text/html; charset=utf-8", "index.html"),
        "/app.js" => ("text/javascript; charset=utf-8", "app.js"),
        "/style.css" => ("text/css; charset=utf-8", "style.css"),
        _ => return None,
    };
    std::fs::read(web.join(rel.1)).ok().map(|data| (rel.0, data))
}

/// HTTP 响应体(字节化:/media 需要回二进制,不再经 String 有损转换)。
pub(crate) struct HttpResp {
    pub(crate) status: &'static str,
    pub(crate) ctype: String,
    /// 附加响应头(每行自带 \r\n,可为空)
    pub(crate) extra: String,
    pub(crate) body: Vec<u8>,
}

pub(crate) fn resp_plain(status: &'static str, msg: &str) -> HttpResp {
    HttpResp { status, ctype: "text/plain; charset=utf-8".into(), extra: String::new(), body: msg.as_bytes().to_vec() }
}

/// 百分号解码(查询参数;encodeURIComponent 输出的 %XX 序列)。
pub(crate) fn pct_decode(s: &str) -> String {
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

pub(crate) fn mime_of(ext: &str) -> &'static str {
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
