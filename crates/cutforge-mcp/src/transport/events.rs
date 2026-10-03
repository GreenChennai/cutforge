// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! SSE 事件面(T1.6/AC-1.5,补 M9-R4 遗留):`/events` 从"仅长轮询"升级为
//! `text/event-stream` 单向流;长轮询降级路径在调用侧原样保留
//! (A1-R2:兼容旧壳,册二完成后移除)。
//!
//! 事件源(只扩发布面,复用 cutforge-io watcher,不建并行监听实现):
//! - project.json(新旧布局)→ `workspace.changed` —— 外部改动可见性(与长轮询同义);
//! - notes.json            → `notes.changed`;
//! - cutlist.json / cutlist.applied.json(新旧布局)→ `cutlist.changed`;
//! - 渲染任务状态           → `render.progress`(progress.rs 发布;hub 未建立即丢弃)。
//!
//! 职责边界:SSE 只做**信号面**——外部 project.json 改动的"锁内 merge_from_disk"
//! 三路合并仍由 cutforge-io 的 250ms 常驻守护独占,本模块绝不碰工作区锁。
//!
//! 节奏:有 SSE 消费者时 80ms 快扫(外部改动 → 推送 ≤200ms P95 的保障);
//! 无人听时 500ms 慢扫(hub 一旦建立即伴生,不搞线程重启/注销的复杂度;
//! 长轮询消费者不计入——那面本来就是 ≤1s 预算,走守护线程即可)。

use cutforge_io::paths;
use cutforge_io::watcher::{EventKind, Watcher};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::io::Write as _;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::http::READ_TIMEOUT;

/// 事件日志容量(重连补发窗口;滚出窗口的旧 seq → 合成 `resync` 事件让客户端全量刷新)。
const LOG_CAP: usize = 512;
/// 有消费者时的扫描间隔:外部改动 → 事件上限 ≈ 80ms + 一轮全树扫描耗时。
const FAST_POLL: Duration = Duration::from_millis(80);
/// 无消费者时的保活扫描间隔。
const IDLE_POLL: Duration = Duration::from_millis(500);
/// SSE 心跳注释行间隔:向客户端与中间层确认连接活性。
const SSE_HEARTBEAT: Duration = Duration::from_secs(15);
/// SSE 流寿命封顶:到点优雅收流,浏览器 EventSource 自动重连(带 Last-Event-ID 续传)。
const SSE_LIFETIME: Duration = Duration::from_secs(30 * 60);

/// SSE 响应头。`Connection: keep-alive` 是本服务唯一的显式长连接形态(连接纪律
/// 见 transport::http:普通响应一律 `Connection: close` 短连接)。
const SSE_HEAD: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";

/// 一条事件:自增 seq + 事件名 + 负载(data 内已注入 ok/code/event/seq——
/// 老字段一个不少,与新老客户端共用同一套解析)。
#[derive(Clone)]
pub(crate) struct Ev {
    seq: u64,
    name: &'static str,
    data: Value,
}

struct EvState {
    seq: u64,
    log: VecDeque<Ev>,
}

/// 每工程根一个的事件 Hub(与 SyncHub 平行的发布面;SyncHub 只盯 project.json
/// 且承担三路合并,这里扩到 notes/cutlist/render 且只发信号)。
pub(crate) struct EvHub {
    st: Mutex<EvState>,
    cv: Condvar,
    /// 活跃 SSE 流数(0 = 慢扫)
    clients: AtomicUsize,
}

impl EvHub {
    fn new() -> Self {
        Self {
            st: Mutex::new(EvState {
                seq: 0,
                log: VecDeque::new(),
            }),
            cv: Condvar::new(),
            clients: AtomicUsize::new(0),
        }
    }

    /// 发布一条事件:seq 自增、入日志窗(封顶弃最旧)、唤醒全部等待者;返回新 seq。
    fn publish(&self, name: &'static str, mut data: Value) -> u64 {
        let mut g = self.st.lock().unwrap();
        g.seq += 1;
        let seq = g.seq;
        data["ok"] = json!(true);
        data["code"] = json!("OK");
        data["event"] = json!(name);
        data["seq"] = json!(seq);
        if g.log.len() >= LOG_CAP {
            g.log.pop_front();
        }
        g.log.push_back(Ev { seq, name, data });
        self.cv.notify_all();
        seq
    }

    /// 等待 seq > since 的新事件;到点无新事件返回 None(调用方发心跳)。
    /// since+1 落在日志窗起点之前 = 有事件已滚出窗口、补发必不完整 → 合成
    /// `resync` 事件让客户端全量刷新(判定看窗口起点,不看过滤结果是否为空)。
    fn wait_after(&self, since: u64, timeout: Duration) -> Option<Vec<Ev>> {
        let deadline = Instant::now() + timeout;
        let mut g = self.st.lock().unwrap();
        loop {
            if g.seq > since {
                let window_start = g
                    .log
                    .front()
                    .map(|e| e.seq)
                    .unwrap_or(g.seq.saturating_add(1));
                if since.saturating_add(1) < window_start {
                    return Some(vec![Ev {
                        seq: g.seq,
                        name: "resync",
                        data: json!({"seq": g.seq}),
                    }]);
                }
                return Some(g.log.iter().filter(|e| e.seq > since).cloned().collect());
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (ng, _) = self.cv.wait_timeout(g, deadline - now).unwrap();
            g = ng;
        }
    }

    fn client_count(&self) -> usize {
        self.clients.load(Ordering::Relaxed)
    }

    /// 当前已发布到的 seq(SSE 首连"只推当下"的起点)。
    fn current(&self) -> u64 {
        self.st.lock().unwrap().seq
    }
}

fn registry() -> &'static Mutex<BTreeMap<PathBuf, Arc<EvHub>>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<PathBuf, Arc<EvHub>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// 每工程根一个 Hub(进程级注册表;首次调用拉起 80/500ms 自适应扫描线程)。
pub(crate) fn ensure_ev_hub(root: &Path) -> Arc<EvHub> {
    let mut map = registry().lock().unwrap();
    if let Some(h) = map.get(root) {
        return h.clone();
    }
    let hub = Arc::new(EvHub::new());
    map.insert(root.to_path_buf(), hub.clone());
    let thread_root = root.to_path_buf();
    let thread_hub = hub.clone();
    std::thread::spawn(move || watch_loop(&thread_root, &thread_hub));
    hub
}

/// 渲染任务状态入事件面(progress.rs 调用)。hub 未建立 = 从无 SSE 消费者,
/// 事件直接丢弃(不为渲染拉起 watcher 线程;错过 running 态的客户端以
/// render_progress RPC 查询为准,事件只是提速信号)。
pub(crate) fn publish_render(root: &Path, run_id: &str, state: &str) {
    let hub = registry().lock().unwrap().get(root).cloned();
    if let Some(h) = hub {
        h.publish("render.progress", json!({"runId": run_id, "state": state}));
    }
}

/// watcher 事件 → SSE 事件名分类(路径一律转成工程内相对、正斜杠形态)。
/// 三态布局全认(册六 V3 扁平布局补齐——册七 SDK 冒烟实测暴露的缺口):
/// V1 `05_ir/project.json` / V2 `05_时间线工程/project.json` / V3 `project.json`。
fn classify(abs: &Path, root: &Path) -> Option<(&'static str, String)> {
    let rel = abs
        .strip_prefix(root)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    let name = if rel == paths::PROJECT_REL
        || rel == paths::LEGACY_PROJECT_REL
        || rel == paths::V3_PROJECT_REL
    {
        "workspace.changed"
    } else if rel == paths::NOTES_REL {
        "notes.changed"
    } else if rel == paths::CUTLIST_REL
        || rel == paths::CUTLIST_APPLIED_REL
        || rel == paths::LEGACY_CUTLIST_REL
        || rel == paths::LEGACY_CUTLIST_APPLIED_REL
        || rel == paths::V3_CUTLIST_REL
        || rel == paths::V3_CUTLIST_APPLIED_REL
    {
        "cutlist.changed"
    } else {
        return None;
    };
    Some((name, rel))
}

fn kind_of(k: EventKind) -> &'static str {
    match k {
        EventKind::Created => "created",
        EventKind::Modified => "modified",
        EventKind::Removed => "removed",
    }
}

fn watch_loop(root: &Path, hub: &EvHub) {
    let mut watcher = Watcher::new(root, 60);
    loop {
        let fast = hub.client_count() > 0;
        std::thread::sleep(if fast { FAST_POLL } else { IDLE_POLL });
        for fe in watcher.poll() {
            if let Some((name, rel)) = classify(&fe.path, root) {
                hub.publish(name, json!({"paths": [rel], "kind": kind_of(fe.kind)}));
            }
        }
    }
}

/// SSE 入口(两通道共用):解析起点 → 建/取 Hub → 跑流。起点语义:
/// 显式 `since=N` 或 `Last-Event-ID: N` → 从 N 补发(滚出窗即 resync);
/// 都没给 → 只推当下(EventSource 首连不该吞历史)。
pub(crate) fn serve_sse(
    stream: &mut TcpStream,
    root: &Path,
    query: &str,
    last_event_id: Option<&str>,
) -> std::io::Result<()> {
    let hub = ensure_ev_hub(root);
    let since_q = query.split('&').find_map(|kv| {
        let mut it = kv.split('=');
        match (it.next(), it.next()) {
            (Some("since"), Some(v)) => v.parse::<u64>().ok(),
            _ => None,
        }
    });
    let leid = last_event_id.and_then(|v| v.trim().parse::<u64>().ok());
    let since = since_q.or(leid).unwrap_or_else(|| hub.current());
    hub.clients.fetch_add(1, Ordering::Relaxed);
    let r = run_sse_stream(stream, &hub, since);
    hub.clients.fetch_sub(1, Ordering::Relaxed);
    r
}

/// 在既有连接上跑 SSE 流:先回响应头 + `: connected` 注释(探针据此确认建立),
/// 随后逐事件写 `id:`/`event:`/`data:` 帧;空闲期以 `: ping` 注释保活。
/// 写失败(客户端离开)或寿命到顶即返回,连接关闭;浏览器端 EventSource 自动重连。
fn run_sse_stream(stream: &mut TcpStream, hub: &EvHub, mut since: u64) -> std::io::Result<()> {
    stream.set_write_timeout(Some(READ_TIMEOUT))?;
    stream.write_all(SSE_HEAD.as_bytes())?;
    stream.write_all(b": connected\n\n")?;
    stream.flush()?;
    let deadline = Instant::now() + SSE_LIFETIME;
    loop {
        if Instant::now() >= deadline {
            let _ = stream.write_all(b"event: bye\ndata: {}\n\n");
            let _ = stream.flush();
            return Ok(());
        }
        if let Some(evs) = hub.wait_after(since, SSE_HEARTBEAT) {
            for ev in evs {
                since = since.max(ev.seq);
                let frame = format!("id: {}\nevent: {}\ndata: {}\n\n", ev.seq, ev.name, ev.data);
                stream.write_all(frame.as_bytes())?;
            }
        } else {
            stream.write_all(b": ping\n\n")?;
        }
        stream.flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 事件面分类:新旧布局 project.json/notes/cutlist 全认;无关文件不进事件面。
    #[test]
    fn classify_covers_dual_layout_and_ignores_noise() {
        let root = Path::new("/ws");
        let mk = |rel: &str| root.join(rel.replace('/', "\\"));
        assert_eq!(
            classify(&mk(paths::PROJECT_REL), root).unwrap().0,
            "workspace.changed"
        );
        assert_eq!(
            classify(&mk(paths::LEGACY_PROJECT_REL), root).unwrap().0,
            "workspace.changed"
        );
        // 册七补口:V3 扁平布局(SDK 冒烟实测暴露——v3 工程此前不发 workspace.changed)
        assert_eq!(
            classify(&mk(paths::V3_PROJECT_REL), root).unwrap().0,
            "workspace.changed"
        );
        assert_eq!(
            classify(&mk(paths::NOTES_REL), root).unwrap().0,
            "notes.changed"
        );
        assert_eq!(
            classify(&mk(paths::CUTLIST_REL), root).unwrap().0,
            "cutlist.changed"
        );
        assert_eq!(
            classify(&mk(paths::LEGACY_CUTLIST_REL), root).unwrap().0,
            "cutlist.changed"
        );
        assert_eq!(
            classify(&mk(paths::V3_CUTLIST_REL), root).unwrap().0,
            "cutlist.changed"
        );
        assert!(classify(&mk("01_原始素材/take1.mp4"), root).is_none());
        assert!(classify(&mk(".cutforge/bases/b1.json"), root).is_none());
    }

    /// Hub 语义:publish 唤醒等待者;wait_after 按 since 增量补发;滚出窗口 → resync。
    #[test]
    fn hub_publish_wait_and_resync() {
        let hub = EvHub::new();
        assert_eq!(hub.current(), 0);
        assert!(
            hub.wait_after(0, Duration::from_millis(10)).is_none(),
            "无事件应超时返回 None"
        );
        let s1 = hub.publish(
            "workspace.changed",
            json!({"paths": ["05_时间线工程/project.json"]}),
        );
        let s2 = hub.publish("notes.changed", json!({"paths": ["notes.json"]}));
        assert_eq!(s2, s1 + 1);
        let evs = hub.wait_after(0, Duration::from_millis(10)).unwrap();
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].name, "workspace.changed");
        assert_eq!(
            evs[0].data["event"],
            json!("workspace.changed"),
            "老字段 event 必须在 data 内"
        );
        assert_eq!(evs[0].data["seq"], json!(s1));
        let evs2 = hub.wait_after(s1, Duration::from_millis(10)).unwrap();
        assert_eq!(evs2.len(), 1, "since 起只补增量");
        // 滚出窗口:since+1 在窗起点之前 → 补发不完整,必须 resync;
        // 窗内 since 仍按增量补发
        for i in 0..(LOG_CAP + 8) {
            hub.publish("workspace.changed", json!({"paths": [format!("f{i}.txt")]}));
        }
        let tail = hub.current();
        let evs3 = hub.wait_after(1, Duration::from_millis(10)).unwrap();
        assert_eq!(evs3.len(), 1);
        assert_eq!(evs3[0].name, "resync");
        assert_eq!(evs3[0].seq, tail);
        let evs5 = hub.wait_after(tail - 2, Duration::from_millis(10)).unwrap();
        assert_eq!(evs5.len(), 2, "窗内 since 按增量补发");
        // render 事件同样携带协议字段
        let s = hub.publish("render.progress", json!({"runId": "r1", "state": "ok"}));
        let evs4 = hub.wait_after(tail, Duration::from_millis(10)).unwrap();
        assert_eq!(evs4[0].data["event"], json!("render.progress"));
        assert_eq!(evs4[0].data["seq"], json!(s));
    }
}
