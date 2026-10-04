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
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
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
/// 503 响应(S-02 连接上限超限;Retry-After 提示客户端退避重试)。
pub(crate) const RESP_UNAVAILABLE: &str = "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

// ---------------- 鉴权与解析共用件(BUG-10 / S-04 / BUG-11;两通道单一实现) ----------------

/// 恒定时间等价(手写 XOR,纯 std):比较耗时与内容无关,不给时序侧信道。
/// 长度不等直接 false——报文长度本身是公开信息,不构成泄露。
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 最小 HTTP 头解析(S-04):请求行之后的头区 → (小写头名, 去空白值) 序列。
/// 头名大小写规范化(HTTP/1.1 头名大小写不敏感);obs-fold 续行(值前导
/// SP/HTAB 的行)不折叠,以 `obs_fold` 如实上报——鉴权面见到续行即拒绝(防走私)。
pub(crate) struct HeadHeaders {
    pub headers: Vec<(String, String)>,
    pub obs_fold: bool,
}

pub(crate) fn parse_headers(head: &str) -> HeadHeaders {
    let mut headers = Vec::new();
    let mut obs_fold = false;
    for line in head.lines().skip(1) {
        if line.is_empty() {
            break; // 头区结束(\r\n\r\n 的空行)
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            obs_fold = true;
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    HeadHeaders { headers, obs_fold }
}

/// 取请求里呈现的 Bearer 值(BUG-10/S-04 校验面前置):
/// 头名大小写不敏感、重复 Authorization 头拒绝(防走私)、obs-fold 续行拒绝。
pub(crate) fn bearer_value(head: &str) -> Option<String> {
    let p = parse_headers(head);
    if p.obs_fold {
        return None;
    }
    let mut it = p.headers.iter().filter(|(n, _)| n == "authorization");
    match (it.next(), it.next()) {
        (Some((_, v)), None) => {
            let v = v.trim();
            // scheme 大小写不敏感(RFC 7235):Bearer/bEARER 同权
            if v.len() >= 7 && v[..7].eq_ignore_ascii_case("bearer ") {
                Some(v[7..].trim().to_string())
            } else {
                None
            }
        }
        _ => None, // 缺头或重复头 → 一律不通过
    }
}

/// Bearer token 校验(BUG-10):值精确相等(恒定时间比较),不再整段子串匹配。
pub(crate) fn check_auth(head: &str, token: &str) -> bool {
    bearer_value(head).is_some_and(|got| constant_time_eq(got.as_bytes(), token.as_bytes()))
}

/// query 按 key 精确切分(S-04/BUG-11):split('&') → split_once('='),
/// key 精确相等(`mytoken=` 不再误命中 `token=`);值统一走 pct_decode
/// (与 /media 同一解码器,SSE root 的 CJK/空格路径由此修复)。
pub(crate) fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| pct_decode(v))
    })
}

// ---------------- S-02 连接计数上限 ----------------

/// 连接上限缺省值(可经 env CUTFORGE_HTTP_MAX_CONNS 配置;两通道同读)。
pub(crate) const DEFAULT_MAX_CONNS: usize = 64;

pub(crate) fn max_conns_from_env() -> usize {
    std::env::var("CUTFORGE_HTTP_MAX_CONNS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(DEFAULT_MAX_CONNS)
}

/// 连接闸门(S-02):acquire 失败 = 已达上限,调用方回 503+Retry-After;
/// 成功返回 RAII 守卫(持有计数器 Arc),连接结束自动释放名额——守卫可安全
/// move 进连接线程,不与 accept 循环的生命周期绑定。
pub(crate) struct ConnGate {
    cur: Arc<AtomicUsize>,
    max: usize,
}

pub(crate) struct ConnGuard(Arc<AtomicUsize>);

impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl ConnGate {
    pub(crate) fn new(max: usize) -> Self {
        Self {
            cur: Arc::new(AtomicUsize::new(0)),
            max,
        }
    }

    pub(crate) fn acquire(&self) -> Option<ConnGuard> {
        let prev = self.cur.fetch_add(1, Ordering::AcqRel);
        if prev >= self.max {
            self.cur.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(ConnGuard(Arc::clone(&self.cur)))
    }
}

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
/// R-08:body 分两态——内存 JSON 体与文件流式体(恒定 64KB 缓冲 io::copy,
/// 整文件/大区间不再 read_to_end 入内存)。
pub(crate) enum RespBody {
    Bytes(Vec<u8>),
    Stream {
        file: std::fs::File,
        start: u64,
        len: u64,
    },
}

pub(crate) struct HttpResp {
    pub(crate) status: &'static str,
    pub(crate) ctype: String,
    /// 附加响应头(每行自带 \r\n,可为空)
    pub(crate) extra: String,
    pub(crate) body: RespBody,
}

impl HttpResp {
    pub(crate) fn bytes(status: &'static str, ctype: &str, body: Vec<u8>) -> Self {
        Self {
            status,
            ctype: ctype.into(),
            extra: String::new(),
            body: RespBody::Bytes(body),
        }
    }

    pub(crate) fn body_len(&self) -> usize {
        match &self.body {
            RespBody::Bytes(b) => b.len(),
            RespBody::Stream { len, .. } => *len as usize,
        }
    }

    /// 内存体视图(Stream 体 = None)。
    pub(crate) fn body_bytes(&self) -> Option<&[u8]> {
        match &self.body {
            RespBody::Bytes(b) => Some(b),
            RespBody::Stream { .. } => None,
        }
    }
}

/// 流式转发读缓冲(R-08:恒定 64KB,与文件大小无关)。
pub(crate) const STREAM_BUF: usize = 64 * 1024;

/// 写响应(两通道单一写出口):head → body(Bytes 一次写;Stream 经
/// BufReader(64KB)+ io::copy 流式转发,不整段入内存)。
pub(crate) fn write_resp(stream: &mut std::net::TcpStream, resp: HttpResp) -> std::io::Result<()> {
    use std::io::{Read as _, Seek as _};
    let clen = resp.body_len();
    write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
        resp.status, resp.ctype, clen, resp.extra
    )?;
    match resp.body {
        RespBody::Bytes(b) => stream.write_all(&b)?,
        RespBody::Stream { file, start, len } => {
            let mut reader = std::io::BufReader::with_capacity(STREAM_BUF, file);
            reader.seek(std::io::SeekFrom::Start(start))?;
            let mut take = reader.take(len);
            std::io::copy(&mut take, &mut *stream)?;
        }
    }
    stream.flush()
}

pub(crate) fn resp_plain(status: &'static str, msg: &str) -> HttpResp {
    HttpResp::bytes(status, "text/plain; charset=utf-8", msg.as_bytes().to_vec())
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
/// S-02:连接计数上限(缺省 64,env CUTFORGE_HTTP_MAX_CONNS 可配),超限 503。
pub fn serve_http(port: u16, token: &str) -> i32 {
    serve_http_with_limit(port, token, max_conns_from_env())
}

/// 测试钩子:以显式上限起辅通道(TC-SEC-040;生产路径走 serve_http 的 env 口径)。
#[doc(hidden)]
pub fn _serve_http_with_limit(port: u16, token: &str, max_conns: usize) -> i32 {
    serve_http_with_limit(port, token, max_conns)
}

fn serve_http_with_limit(port: u16, token: &str, max_conns: usize) -> i32 {
    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind 失败: {e}");
            return 4;
        }
    };
    eprintln!(
        "cutforge-mcp http on http://127.0.0.1:{port}/rpc (events: /events?root=..&since=N;SSE: Accept: text/event-stream;max-conns={max_conns})"
    );
    let gate = Arc::new(ConnGate::new(max_conns));
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        // S-02:先占名额再 spawn;超限立即 503+Retry-After(连接不进读循环)
        let Some(guard) = gate.acquire() else {
            let _ = stream.write_all(RESP_UNAVAILABLE.as_bytes());
            let _ = stream.flush();
            continue;
        };
        let token = token.to_string();
        let gate = gate.clone();
        std::thread::spawn(move || {
            let _ = handle_http_conn(stream, &token);
            drop(guard);
            let _ = gate; // 名额随 guard 释放
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
    // BUG-10/S-04:Bearer 值精确相等(恒定时间),头名大小写不敏感,
    // 重复头/续行拒绝——不再对整段 head 做 contains 子串匹配
    let authorized = check_auth(&req.head, token);
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
        // SSE:root 仍走查询参数(辅通道无绑定工程);起点语义同工作区通道。
        // BUG-11:root/since 统一经 query_param 精确切分 + pct_decode
        // (CJK/空格路径此前拿字面 %XX 串当路径,事件永不到达)。
        return match query_param(api_query, "root") {
            Some(r) => events::serve_sse(
                &mut stream,
                Path::new(&r),
                api_query,
                req.header("last-event-id").as_deref(),
                "",
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
        let body = resp
            .body_bytes()
            .map(String::from_utf8_lossy)
            .unwrap_or_default()
            .into_owned();
        (resp.status, body)
    } else if is_events {
        // 长轮询:/events?root=<工程目录>&since=<seq>;≤1s 内有新事件立即返回。
        // A1-R2:此降级路径兼容旧壳,册二完成后移除(SSE 为新壳唯一事件面)。
        let root_p = query_param(api_query, "root").unwrap_or_default();
        let since: u64 = query_param(api_query, "since")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
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
