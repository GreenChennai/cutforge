// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 渲染进度与渲染队列:cutforge-render 后端(E5/B6)的同步/异步渲染、
//! 结构化进度轮询(T1.1 拆分自 lib.rs)与**队列语义**(册五 T5.6:
//! 排队/暂停/继续/取消/失败重试,并发上限可配,状态事件接既有 render.progress)。

use crate::registry::envelope;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// ---------------- 渲染后端分派(E5/B6:cutforge-render 子进程,不依赖 CutFlow) ----------------

/// E5-2:cutforge-render 可执行定位:当前可执行文件同目录 → PATH → env CUTFORGE_RENDER。
pub(crate) fn resolve_render_bin() -> Option<PathBuf> {
    let exe_name = format!("cutforge-render{}", std::env::consts::EXE_SUFFIX);
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent() {
            let cand = dir.join(&exe_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let cand = dir.join(&exe_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    std::env::var_os("CUTFORGE_RENDER").map(PathBuf::from)
}

fn render_missing_dep() -> Value {
    envelope(false, "DEP_MISSING",
        "未找到 cutforge-render:与本程序同目录放置、加入 PATH,或设 CUTFORGE_RENDER 指向可执行文件", json!({}))
}

/// ass 相对路径存在才透传(缺字幕 = 不烧录,而非渲染失败)。
pub(crate) fn existing_rel<'a>(root: &Path, rel: Option<&'a str>) -> Option<&'a str> {
    rel.filter(|r| !r.is_empty() && root.join(r).is_file())
}

/// 渲染选项(MCP 参数)→ cutforge-render CLI 附加参数(册五 T5.6;
/// 缺省参数不产生任何 CLI 旗标 = 现行为零变化)。
pub(crate) fn build_render_extra(args: &Value) -> Vec<String> {
    let mut extra: Vec<String> = Vec::new();
    let mut flag = |name: &str, v: Option<String>| {
        if let Some(v) = v.filter(|s| !s.is_empty()) {
            extra.push(name.into());
            extra.push(v);
        }
    };
    flag("--encoder", args["encoder"].as_str().map(String::from));
    flag("--quality", args["quality"].as_str().map(String::from));
    flag("--crf", args["crf"].as_u64().map(|v| v.to_string()));
    flag("--bitrate", args["bitrate"].as_u64().map(|v| v.to_string()));
    flag("--gop", args["gop"].as_u64().map(|v| v.to_string()));
    flag("--pix-fmt", args["pixFmt"].as_str().map(String::from));
    if let Some(t) = args["loudnormTarget"].as_str().filter(|s| !s.is_empty()) {
        extra.push("--loudnorm-target".into());
        extra.push(t.into());
    } else if let (Some(i), Some(tp)) = (args["loudnormI"].as_f64(), args["loudnormTp"].as_f64()) {
        extra.push("--loudnorm-target".into());
        extra.push(format!("{}:{}", fmt_num(i), fmt_num(tp)));
    }
    if args["verboseCmd"].as_bool() == Some(true) {
        extra.push("--verbose-cmd".into());
    }
    extra
}

/// f64 → 紧凑串(去尾零;与 cutforge-render steps::fmt_f64 同风格)。
fn fmt_num(v: f64) -> String {
    let s = format!("{v:.6}").trim_end_matches('0').trim_end_matches('.').to_string();
    if s.is_empty() { "0".into() } else { s }
}

fn spawn_render(root: &Path, ass: Option<&str>, use_proxy: bool, extra: &[String]) -> std::process::Command {
    let mut cmd = match resolve_render_bin() {
        Some(p) => std::process::Command::new(p),
        None => std::process::Command::new("cutforge-render"),
    };
    cmd.arg("--root").arg(root);
    if let Some(a) = ass {
        cmd.arg("--ass").arg(a);
    }
    if use_proxy {
        cmd.arg("--use-proxy");
    }
    cmd.args(extra);
    cmd
}

/// 同步渲染(MCP 工具 render,backend=cutforge):输出 JSON 行进度进 data.stdout。
pub(crate) fn render_cutforge_sync(root: &Path, ass: Option<&str>, use_proxy: bool, extra: &[String]) -> Value {
    if resolve_render_bin().is_none() {
        return render_missing_dep();
    }
    match spawn_render(root, ass, use_proxy, extra).output() {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let output = text.lines().rev().find_map(|l| serde_json::from_str::<Value>(l).ok())
                .and_then(|v| v.get("output").and_then(|o| o.as_str()).map(String::from));
            envelope(true, "OK", "渲染完成",
                json!({"backend": "cutforge", "output": output, "stdout": text.trim()}))
        }
        Ok(out) => envelope(false, "INTERNAL",
            &format!("cutforge-render 失败:{}", String::from_utf8_lossy(&out.stderr).trim().chars().take(300).collect::<String>()),
            json!({})),
        Err(e) => envelope(false, "DEP_MISSING", &format!("cutforge-render 不可用: {e}"), json!({})),
    }
}

// ---------------- T5.6 渲染队列(排队/暂停/继续/取消/重试;并发上限可配) ----------------

/// 队列任务(内存态;进程生命周期内有效)。state 机:
/// `queued → running → ok|fail`;queued/running --pause→ paused;paused --resume→ queued;
/// queued/running/paused --cancel→ canceled;fail/canceled --retry→ queued。
struct QueueJob {
    state: &'static str, // queued | running | paused | ok | fail | canceled
    lines: Vec<String>,
    output: Option<String>,
    error: Option<String>,
    // 重跑/resume 所需的完整 spawn 参数(retry/resume 复用,幂等:渲染缓存吸收重跑成本)
    root: PathBuf,
    ass: Option<String>,
    use_proxy: bool,
    extra: Vec<String>,
    /// 运行中子进程句柄(pause/cancel 取出即杀;None = 未运行)。
    child: Option<std::process::Child>,
}

fn queue() -> &'static std::sync::Mutex<Vec<(String, QueueJob)>> {
    static Q: OnceLock<std::sync::Mutex<Vec<(String, QueueJob)>>> = OnceLock::new();
    Q.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// 并发上限(env CUTFORGE_RENDER_CONCURRENCY,缺省 1;非正数回落 1)。
fn concurrency_limit() -> usize {
    std::env::var("CUTFORGE_RENDER_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(1)
}

fn publish_state(root: &Path, run_id: &str, state: &str) {
    crate::transport::events::publish_render(root, run_id, state);
}

/// 取新 runId(r + 16hex;进程号 + 纳秒哈希,与拆分前同源)。
fn new_run_id() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::process::id().hash(&mut h);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
    format!("r{:016x}", h.finish())
}

/// 异步渲染入口(册五 T5.6 队列化):任务**入队**即返回 runId;
/// 单例 worker 按并发上限派发。队列空且空闲时首任务即时起跑(与拆分前的
/// 立即启动体感一致;进度轮询接口 render_progress 不变)。
pub(crate) fn render_run_async(root: &Path, ass: Option<&str>, use_proxy: bool, extra: Vec<String>) -> Value {
    if resolve_render_bin().is_none() {
        return render_missing_dep();
    }
    let run_id = new_run_id();
    let job = QueueJob {
        state: "queued",
        lines: Vec::new(),
        output: None,
        error: None,
        root: root.to_path_buf(),
        ass: ass.map(String::from),
        use_proxy,
        extra,
        child: None,
    };
    if let Ok(mut q) = queue().lock() {
        q.push((run_id.clone(), job));
    }
    publish_state(root, &run_id, "queued");
    ensure_worker();
    envelope(true, "OK", "渲染任务已入队", json!({"runId": run_id, "concurrency": concurrency_limit()}))
}

fn ensure_worker() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let next = {
                let Ok(mut q) = queue().lock() else { continue };
                let running = q.iter().filter(|(_, j)| j.state == "running").count();
                if running >= concurrency_limit() {
                    continue;
                }
                q.iter_mut().find(|(_, j)| j.state == "queued")
                    .map(|(id, j)| {
                        j.state = "running";
                        id.clone()
                    })
            };
            let Some(run_id) = next else { continue };
            let (root, cmd) = {
                let Ok(mut q) = queue().lock() else { continue };
                let Some((_, j)) = q.iter_mut().find(|(id, _)| *id == run_id) else { continue };
                let ass = j.ass.clone();
                (
                    j.root.clone(),
                    spawn_render(&j.root, ass.as_deref(), j.use_proxy, &j.extra),
                )
            };
            publish_state(&root, &run_id, "running");
            run_job(&root, &run_id, cmd);
        });
    });
}

/// 运行单个任务(独占线程直至子进程退出;stdout JSON 行进度入任务表)。
/// 终态写入前核验 state 仍为 running(pause/cancel 已改写的终态不覆盖)。
fn run_job(root: &Path, run_id: &str, mut cmd: std::process::Command) {
    let mut child = match cmd.stdout(std::process::Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(_) => {
            if let Ok(mut q) = queue().lock()
                && let Some((_, j)) = q.iter_mut().find(|(id, _)| id == run_id)
                && j.state == "running"
            {
                j.state = "fail";
                j.error = Some("cutforge-render 子进程启动失败".into());
            }
            publish_state(root, run_id, "fail");
            return;
        }
    };
    // stdout 句柄先取走;子进程句柄入表(pause/cancel 可杀)
    let mut stdout = child.stdout.take();
    if let Ok(mut q) = queue().lock()
        && let Some((_, j)) = q.iter_mut().find(|(id, _)| id == run_id) {
            j.child = Some(child);
        }
    if let Some(out) = stdout.take() {
        let reader = std::io::BufReader::new(out);
        for line in std::io::BufRead::lines(reader).map_while(Result::ok) {
            let Ok(mut q) = queue().lock() else { break };
            let Some((_, j)) = q.iter_mut().find(|(id, _)| id == run_id) else { break };
            if let Ok(v) = serde_json::from_str::<Value>(&line)
                && let Some(o) = v.get("output").and_then(|o| o.as_str()) {
                    j.output = Some(o.to_string());
                }
            j.lines.push(line);
            let n = j.lines.len();
            if n > 200 {
                j.lines.drain(..n - 200);
            }
        }
    }
    // 收回句柄等待退出(被 pause/cancel 杀过的进程,句柄可能已被取出)
    let mut child = queue().lock().ok().and_then(|mut q| {
        q.iter_mut().find(|(id, _)| id == run_id).and_then(|(_, j)| j.child.take())
    });
    let ok = child.as_mut().map(|c| c.wait().map(|s| s.success()).unwrap_or(false)).unwrap_or(false);
    if let Ok(mut q) = queue().lock()
        && let Some((_, j)) = q.iter_mut().find(|(id, _)| id == run_id) {
            j.child = None;
            if j.state == "running" {
                j.state = if ok { "ok" } else { "fail" };
                if !ok {
                    j.error = Some("cutforge-render 非零退出;详见服务端控制台".into());
                }
            }
        }
    let final_state = queue().lock().ok()
        .and_then(|q| q.iter().find(|(id, _)| id == run_id).map(|(_, j)| j.state))
        .unwrap_or("fail")
        .to_string();
    publish_state(root, run_id, &final_state);
}

/// render_queue 工具面(册五 T5.6):list/pause/resume/cancel/retry + concurrency 设置。
/// pause(运行中)= 终止子进程转 paused(resume 重新入队——渲染缓存吸收重跑成本);
/// 暂停语义诚实声明:进程级冻结跨平台不可达,终止+重入队是可判定语义。
pub(crate) fn render_queue_tool(root: &Path, args: &Value) -> Value {
    let _ = root;
    if args["root"].as_str().is_none() {
        return envelope(false, "PRECONDITION_FAILED", "缺 root(工程目录)", json!({}));
    }
    let action = args["action"].as_str().unwrap_or("list");
    let Ok(mut q) = queue().lock() else {
        return envelope(false, "INTERNAL", "渲染队列不可用", json!({}));
    };
    match action {
        "list" => {
            let jobs: Vec<Value> = q
                .iter()
                .map(|(id, j)| {
                    let mut o = json!({"runId": id, "state": j.state});
                    if let Some(out) = &j.output {
                        o["output"] = json!(out);
                    }
                    if let Some(err) = &j.error {
                        o["error"] = json!(err);
                    }
                    o
                })
                .collect();
            envelope(true, "OK", "渲染队列", json!({
                "jobs": jobs, "concurrency": concurrency_limit(),
            }))
        }
        "pause" | "resume" | "cancel" | "retry" => {
            let Some(run_id) = args["runId"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 runId", json!({}));
            };
            let Some((_, j)) = q.iter_mut().find(|(id, _)| id == run_id) else {
                return envelope(false, "PRECONDITION_FAILED", &format!("未知 runId: {run_id}"), json!({}));
            };
            match (action, j.state) {
                ("pause", "queued" | "running") => {
                    if let Some(mut child) = j.child.take() {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                    j.state = "paused";
                    publish_state(&j.root, run_id, "paused");
                    envelope(true, "OK", "任务已暂停", json!({"runId": run_id, "state": "paused"}))
                }
                ("pause", other) => envelope(
                    false, "PRECONDITION_FAILED",
                    &format!("state={other} 不可暂停(仅 queued/running)"), json!({}),
                ),
                ("resume", "paused") => {
                    j.state = "queued";
                    j.lines.clear();
                    j.error = None;
                    publish_state(&j.root, run_id, "queued");
                    ensure_worker();
                    envelope(true, "OK", "任务已恢复排队", json!({"runId": run_id, "state": "queued"}))
                }
                ("resume", other) => envelope(
                    false, "PRECONDITION_FAILED",
                    &format!("state={other} 不可恢复(仅 paused)"), json!({}),
                ),
                ("cancel", "queued" | "running" | "paused") => {
                    if let Some(mut child) = j.child.take() {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                    j.state = "canceled";
                    publish_state(&j.root, run_id, "canceled");
                    envelope(true, "OK", "任务已取消", json!({"runId": run_id, "state": "canceled"}))
                }
                ("cancel", other) => envelope(
                    false, "PRECONDITION_FAILED",
                    &format!("state={other} 不可取消(终态)"), json!({}),
                ),
                ("retry", "fail" | "canceled" | "ok") => {
                    j.state = "queued";
                    j.lines.clear();
                    j.output = None;
                    j.error = None;
                    publish_state(&j.root, run_id, "queued");
                    ensure_worker();
                    envelope(true, "OK", "任务已重新排队", json!({"runId": run_id, "state": "queued"}))
                }
                ("retry", other) => envelope(
                    false, "PRECONDITION_FAILED",
                    &format!("state={other} 无可重试(仅 fail/canceled/ok)"), json!({}),
                ),
                _ => envelope(false, "PRECONDITION_FAILED", &format!("未知 action: {action}"), json!({})),
            }
        }
        _ => envelope(false, "PRECONDITION_FAILED", &format!("未知 action: {action}(允许 list/pause/resume/cancel/retry)"), json!({})),
    }
}

pub(crate) fn render_progress(run_id: &str) -> Value {
    let Ok(q) = queue().lock() else {
        return envelope(false, "INTERNAL", "渲染任务表不可用", json!({}));
    };
    match q.iter().find(|(id, _)| id == run_id) {
        Some((_, j)) => envelope(true, "OK", "渲染进度", json!({
            "state": j.state,
            "lines": j.lines.iter().rev().take(30).rev().collect::<Vec<_>>(),
            "output": j.output,
            "error": j.error,
        })),
        None => envelope(false, "PRECONDITION_FAILED", &format!("未知 runId: {run_id}"), json!({})),
    }
}

// ---------------- T2.4 单帧精确预览(render_frame) ----------------

/// render_frame(T2.4「精确预览」的服务端支撑):渲染指定时间点的一帧合成画面
/// (PNG/JPEG),含转场/叠加/字幕烧录的真实结果,消除预览与成片落差(R7)。
/// 同步执行(单帧耗时可同步);不持工程锁、免开工作区(与 render/run 同口径,
/// cutforge-render 只读 project.json)。产物落 `.cutforge/render-cache/frame/`
/// (内容寻址:键 = 工作区指纹 fresh.rs + atMs 100ms 量化 + 画幅 + 渲染版本,
/// 改一笔即 miss 不出陈旧帧);壳经 /media 以返回的 `media` 相对路径加载
/// (帧缓存位于工程根内,/media 的 canonicalize 校验天然覆盖,零新增面)。
pub(crate) fn render_frame_tool(root: &Path, args: &Value) -> Value {
    let Some(at_ms) = args["atMs"].as_u64() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 atMs(时间线时间点,毫秒)", json!({}));
    };
    let fmt = match args["format"].as_str() {
        None | Some("png") => "png",
        Some("jpeg") => "jpeg",
        Some(other) => {
            return envelope(false, "PRECONDITION_FAILED", &format!("未知 format: {other}(允许 png/jpeg)"), json!({}))
        }
    };
    if !cutforge_io::paths::has_project(root) {
        return envelope(false, "NO_CONFIG",
            "工程不存在(缺 05_时间线工程/project.json,兼容旧 05_ir/)", json!({}));
    }
    if resolve_render_bin().is_none() {
        return render_missing_dep();
    }
    // ass 过滤在服务端(与 render/render_run 同口径):路径不存在时不烧录而非失败
    let ass = existing_rel(root, args["ass"].as_str());
    let use_proxy = args["useProxy"].as_bool().unwrap_or(false);
    let mut extra: Vec<String> = vec!["--frame".into(), format!("{at_ms}"), "--format".into(), fmt.into()];
    extra.extend(build_render_extra(args));
    let mut cmd = spawn_render(root, ass, use_proxy, &extra);
    match cmd.output() {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            // 末行 JSON 完成事件优先;FRAME_OK 行兜底(接口与整片同风格)
            let ev = text.lines().rev().find_map(|l| serde_json::from_str::<Value>(l).ok())
                .filter(|v| v.get("mode").and_then(|m| m.as_str()) == Some("frame"));
            let frame_path = ev
                .as_ref()
                .and_then(|v| v.get("frame"))
                .and_then(|f| f.as_str())
                .map(PathBuf::from)
                .or_else(|| text.lines().rev().find_map(|l| l.strip_prefix("FRAME_OK ").map(PathBuf::from)));
            let Some(frame_path) = frame_path else {
                return envelope(false, "INTERNAL", "cutforge-render 单帧输出不可解析",
                    json!({"stdout": text.trim()}));
            };
            let at_out = ev.as_ref().and_then(|v| v.get("atMs")).and_then(|a| a.as_u64()).unwrap_or(at_ms);
            let cached = ev.as_ref().and_then(|v| v.get("cached")).and_then(|c| c.as_bool()).unwrap_or(false);
            let key = ev.as_ref().and_then(|v| v.get("key")).and_then(|k| k.as_str()).unwrap_or_default();
            // media 相对路径(工程根内;正斜杠约定)→ 壳直接 /media?path=… 加载
            let media = frame_path
                .strip_prefix(root)
                .unwrap_or(&frame_path)
                .to_string_lossy()
                .replace('\\', "/");
            envelope(true, "OK", if cached { "帧缓存命中" } else { "帧已渲染" }, json!({
                "backend": "cutforge",
                "atMs": at_out,
                "format": fmt,
                "cached": cached,
                "key": key,
                "path": frame_path.to_string_lossy(),
                "media": media,
            }))
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr);
            let msg = err.trim().strip_prefix("FRAME_FAIL: ").unwrap_or(err.trim());
            let code = if msg.starts_with("NO_CONFIG:") {
                "NO_CONFIG"
            } else if msg.starts_with("PRECONDITION:") {
                "PRECONDITION_FAILED"
            } else {
                "INTERNAL"
            };
            envelope(false, code, &msg.chars().take(300).collect::<String>(), json!({}))
        }
        Err(e) => envelope(false, "DEP_MISSING", &format!("cutforge-render 不可用: {e}"), json!({})),
    }
}
