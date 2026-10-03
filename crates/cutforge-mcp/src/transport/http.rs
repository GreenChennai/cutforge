// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 内嵌 HTTP 辅通道(仅 127.0.0.1 + Bearer token)与 HTTP 共用件:请求读取
//! (连接纪律,T1.6/D-A1)、HTTP 响应件、百分号解码、MIME 表(T1.1 拆分自
//! lib.rs;T1.6 加固:读超时/体积上限/显式短连接;静态面在 transport::static_files,
//! SSE 事件面在 transport::events)。

use crate::dispatch::handle_rpc;
use crate::registry::tool_names;
use crate::transport::events;
use cutforge_core::oplog::Actor;
use serde_json::{Value, json};
use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

// ---------------- 连接纪律(T1.6/AC-1.7;决策 D-A1:纯 std 手工加固,ADR-0009) ----------------

/// 单次 socket 读超时:两条数据间隔超过它即视为慢/挂死连接,直接回收(慢连接隔离;
/// 每连接一线程,挂死的只是它自己那根线程,不给它响应机会)。
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// 单个请求总时限(自读循环启动起算):无论客户端多"勤快"地滴水,到点即回收。
pub(crate) const REQUEST_DEADLINE: Duration = Duration::from_secs(30);
/// 请求体大小上限:超限立即 413、不读体(防内存炸弹;/rpc 面是小 JSON,8 MiB 极宽裕)。
pub(crate) const MAX_BODY: usize = 8 * 1024 * 1024;
/// 请求头(含请求行)大小上限:头没读完就超限 → 413 回绝。
pub(crate) const MAX_HEAD: usize = 64 * 1024;
/// 413 响应(两通道共用同一份字面,不各写一份)。
pub(crate) const RESP_TOO_LARGE: &str =
    "HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
/// 401 响应(两通道共用同一份字面)。
pub(crate) const RESP_UNAUTHORIZED: &str =
    "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

// 连接复用策略(明确化):本服务统一**短连接**——每个响应必带 `Connection: close`
// 并在写毕后关闭 socket;唯一例外是 SSE 流(transport::events 的显式
// `Connection: keep-alive` 单向长流,心跳保活、寿命封顶)。不支持 chunked 请求体
// (请求体只认 Content-Length;无 Content-Length 的体一律按 0 处理)。

/// 读出来的一个完整请求(head = 请求行 + 头;body = Content-Length 界定的体)。
pub(crate) struct RawReq {
    /// 从请求行到 `\r\n\r\n`(含)前的整段(lossy 解码)
    pub head: String,
    /// 请求体(lossy 解码;JSON-RPC 面足够)
    pub body: String,
}

impl RawReq {
    pub(crate) fn first_line(&self) -> &str {
        self.head.lines().next().unwrap_or("")
    }

    /// 大小写不敏感取头值(单值语义,取首个命中)。
    pub(crate) fn header(&self, name: &str) -> Option<String> {
        let want = format!("{name}:");
        self.head.lines().find_map(|l| {
            if l.len() > want.len() && l[..want.len()].eq_ignore_ascii_case(&want) {
                Some(l[want.len()..].trim().to_string())
            } else {
                None
            }
        })
    }
}

enum Scan {
    /// 头还没读齐,或体尚未读满
    Incomplete,
    /// 头或体超过上限(调用方回 413)
    TooLarge,
    /// 头读齐且体读满
    Done,
}

/// 对当前缓冲做一次"是否完整请求"的判定(头界 + Content-Length 界)。
fn scan(buf: &[u8]) -> Scan {
    let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
        return if buf.len() > MAX_HEAD {
            Scan::TooLarge
        } else {
            Scan::Incomplete
        };
    };
    let head_end = pos + 4;
    if head_end > MAX_HEAD {
        return Scan::TooLarge;
    }
    let len: usize = String::from_utf8_lossy(&buf[..head_end])
        .to_ascii_lowercase()
        .lines()
        .find(|l| l.starts_with("content-length:"))
        .and_then(|l| l.split(':').nth(1).and_then(|n| n.trim().parse().ok()))
        .unwrap_or(0);
    if len > MAX_BODY {
        return Scan::TooLarge;
    }
    if buf.len() >= head_end + len {
        Scan::Done
    } else {
        Scan::Incomplete
    }
}

/// 读一个完整请求(头 + Content-Length 界定的体),两通道单一实现。
/// 纪律:读超时 `READ_TIMEOUT`(慢连接隔离)、总时限 `REQUEST_DEADLINE`、
/// 头/体上限(超限 413,体不再读)。返回 `Closed` 时连接不完整或对端已走,
/// 调用方直接回收、不回写(挂死连接不值得响应);`TooLarge` 时回 413 再关。
pub(crate) fn read_request(stream: &mut std::net::TcpStream) -> Result<RawReq, super::ReadFail> {
    use std::io::Read as _;
    // 超时设置失败极罕见;不因它放弃请求,后续 read 自会暴露真实错误
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let deadline = Instant::now() + REQUEST_DEADLINE;
    let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024);
    let mut tmp = [0u8; 4096];
    loop {
        // 先判完整再读:请求可能一次到达,客户端也可能发完即 shutdown 写半边
        // (合法模式,须照样应答;Windows 下滞留体未读会记 RST,见旧实现注记)
        match scan(&buf) {
            Scan::Done => {
                let pos = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(0) + 4;
                let head = String::from_utf8_lossy(&buf[..pos]).into_owned();
                let body = String::from_utf8_lossy(&buf[pos..]).into_owned();
                return Ok(RawReq { head, body });
            }
            Scan::TooLarge => return Err(super::ReadFail::TooLarge),
            Scan::Incomplete => {}
        }
        if Instant::now() >= deadline {
            return Err(super::ReadFail::Closed);
        }
        match stream.read(&mut tmp) {
            // 对端收场且请求不完整 → 回收;读超时/任何读错误 → 回收
            Ok(0) | Err(_) => return Err(super::ReadFail::Closed),
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
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
    HttpResp {
        status,
        ctype: "text/plain; charset=utf-8".into(),
        extra: String::new(),
        body: msg.as_bytes().to_vec(),
    }
}

/// 百分号解码(查询参数;encodeURIComponent 输出的 %XX 序列)。
pub(crate) fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(hi), Some(lo)) = (
                (b[i + 1] as char).to_digit(16),
                (b[i + 2] as char).to_digit(16),
            )
        {
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

/// 内嵌 HTTP 辅通道:仅监听 127.0.0.1,Bearer token 校验;每连接一线程
/// (挂死连接只占它自己的线程,读超时后回收);GET /events 长轮询推外部改动事件,
/// 带 `Accept: text/event-stream` 时升级为 SSE(transport::events,单一实现)。
pub fn serve_http(port: u16, token: &str) -> i32 {
    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind 失败: {e}");
            return 4;
        }
    };
    eprintln!(
        "cutforge-mcp http on http://127.0.0.1:{port}/rpc (events: /events?root=..&since=N;SSE: Accept: text/event-stream)"
    );
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
    let req = match read_request(&mut stream) {
        Ok(r) => r,
        Err(super::ReadFail::TooLarge) => {
            let _ = write!(stream, "{RESP_TOO_LARGE}");
            return Ok(());
        }
        // 挂死/半途而废的连接:直接回收,不回写(连接纪律)
        Err(super::ReadFail::Closed) => return Ok(()),
    };
    let first_line = req.first_line().to_string();
    let authorized = req.head.contains(&format!("Authorization: Bearer {token}"));
    let is_rpc = first_line.starts_with("POST /rpc");
    // 册七 T7.1:/api/v1 REST 面(ADR-0025);事件流别名并入既有 /events 分支(SSE 单一实现)
    let raw_path = first_line.split(' ').nth(1).unwrap_or("");
    let (api_path, api_query) = raw_path.split_once('?').unwrap_or((raw_path, ""));
    let is_events = crate::transport::rest::is_events_path(api_path);
    // events 别名不进 rest 面(SSE/长轮询单一实现;rest 管其余 /api/v1/*)
    let is_api_v1 = !is_events && crate::transport::rest::is_api_v1(api_path);
    if !authorized {
        let _ = write!(stream, "{RESP_UNAUTHORIZED}");
        return Ok(());
    }
    if is_events
        && req
            .header("accept")
            .is_some_and(|v| v.to_ascii_lowercase().contains("text/event-stream"))
    {
        // SSE:root 仍走查询参数(辅通道无绑定工程);起点语义同工作区通道
        let query = api_query;
        let root_p = query.split('&').find_map(|kv| {
            let mut it = kv.split('=');
            match (it.next(), it.next()) {
                (Some("root"), Some(v)) => Some(v.to_string()),
                _ => None,
            }
        });
        return match root_p {
            Some(r) => events::serve_sse(
                &mut stream,
                Path::new(&r),
                query,
                req.header("last-event-id").as_deref(),
            ),
            None => {
                // T1.7 三面同码:事件面错误也带 ns(加法字段;code 取值不变)
                let body = json!({"ok": false, "code": "PRECONDITION_FAILED",
                    "ns": crate::code_namespace("PRECONDITION_FAILED"), "message": "缺 root"})
                .to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                Ok(())
            }
        };
    }
    let (resp_status, resp_body): (&str, String) = if is_rpc {
        ("200 OK", match serde_json::from_str::<Value>(&req.body) {
            Ok(req) => handle_rpc(&req).map(|r| r.to_string()).unwrap_or_default(),
            Err(e) => json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}).to_string(),
        })
    } else if is_api_v1 {
        // 册七 T7.1:/api/v1 版本化 REST 面(ADR-0025;辅通道无绑定工程,root 须由
        // 参数/查询串显式给出;actor=agent 与本通道 /rpc 同归因);状态码随 envelope
        let resp = crate::transport::rest::handle_api_v1(
            api_path,
            api_query,
            first_line.starts_with("POST"),
            &req.body,
            None,
            Actor::agent("cutforge-mcp"),
        );
        let body = String::from_utf8_lossy(&resp.body).into_owned();
        (resp.status, body)
    } else if is_events {
        // 长轮询:/events?root=<工程目录>&since=<seq>;≤1s 内有新事件立即返回。
        // A1-R2:此降级路径兼容旧壳,册二完成后移除(SSE 为新壳唯一事件面)。
        let query = first_line
            .split(' ')
            .nth(1)
            .unwrap_or("")
            .split_once('?')
            .map(|(_, q)| q)
            .unwrap_or("");
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
            (
                "200 OK",
                json!({"ok": false, "code": "PRECONDITION_FAILED",
                "ns": crate::code_namespace("PRECONDITION_FAILED"), "message": "缺 root"})
                .to_string(),
            )
        } else {
            let hub = cutforge_io::watcher::ensure_sync_daemon(Path::new(&root_p));
            let wait = Duration::from_millis(900);
            let body = match hub.wait_since(since, wait) {
                Some(seq) => {
                    json!({"ok": true, "code": "OK", "event": "workspace.changed", "seq": seq})
                        .to_string()
                }
                None => json!({"ok": true, "code": "OK", "event": "none", "seq": hub.current()})
                    .to_string(),
            };
            ("200 OK", body)
        }
    } else {
        (
            "200 OK",
            json!({"service": "cutforge-mcp", "tools": tool_names().len()}).to_string(),
        )
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        resp_status,
        resp_body.len(),
        resp_body
    );
    Ok(())
}
