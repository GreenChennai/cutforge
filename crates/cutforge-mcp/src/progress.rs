// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 渲染进度:cutforge-render 后端(E5/B6)的同步/异步渲染与结构化进度轮询
//! (T1.1 拆分自 lib.rs,纯移动)。

use crate::registry::envelope;
use serde_json::{json, Value};
use std::collections::HashMap;
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

fn spawn_render(root: &Path, ass: Option<&str>) -> std::process::Command {
    let mut cmd = match resolve_render_bin() {
        Some(p) => std::process::Command::new(p),
        None => std::process::Command::new("cutforge-render"),
    };
    cmd.arg("--root").arg(root);
    if let Some(a) = ass {
        cmd.arg("--ass").arg(a);
    }
    cmd
}

/// 同步渲染(MCP 工具 render,backend=cutforge):输出 JSON 行进度进 data.stdout。
pub(crate) fn render_cutforge_sync(root: &Path, ass: Option<&str>) -> Value {
    if resolve_render_bin().is_none() {
        return render_missing_dep();
    }
    match spawn_render(root, ass).output() {
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

/// E5-3 异步渲染任务表(内存态;进程生命周期内有效)。
struct RenderJob {
    state: &'static str, // running | ok | fail
    lines: Vec<String>,
    output: Option<String>,
    error: Option<String>,
}

fn renders() -> &'static std::sync::Mutex<HashMap<String, RenderJob>> {
    static R: OnceLock<std::sync::Mutex<HashMap<String, RenderJob>>> = OnceLock::new();
    R.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

pub(crate) fn render_run_async(root: &Path, ass: Option<&str>) -> Value {
    if resolve_render_bin().is_none() {
        return render_missing_dep();
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::process::id().hash(&mut h);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
    let run_id = format!("r{:016x}", h.finish());
    let mut cmd = spawn_render(root, ass);
    if let Ok(mut m) = renders().lock() {
        m.insert(run_id.clone(), RenderJob { state: "running", lines: Vec::new(), output: None, error: None });
    }
    // T1.6:任务状态入 SSE 事件面(hub 未建立即丢弃,见 transport::events)
    crate::transport::events::publish_render(root, &run_id, "running");
    let run_id_thread = run_id.clone();
    let root_thread = root.to_path_buf();
    std::thread::spawn(move || {
        let run_id = run_id_thread;
        let root = root_thread;
        let Ok(mut child) = cmd
            .stdout(std::process::Stdio::piped())
            .spawn()
        else {
            if let Ok(mut m) = renders().lock()
                && let Some(j) = m.get_mut(&run_id) {
                    j.state = "fail";
                    j.error = Some("cutforge-render 子进程启动失败".into());
                }
            crate::transport::events::publish_render(&root, &run_id, "fail");
            return;
        };
        // 只接 stdout(JSON 行进度);stderr 直通服务端控制台(不读不堵塞)。
        if let Some(out) = child.stdout.take() {
            let reader = std::io::BufReader::new(out);
            for line in std::io::BufRead::lines(reader).map_while(Result::ok) {
                let Ok(mut m) = renders().lock() else { break };
                let Some(j) = m.get_mut(&run_id) else { break };
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
        let ok = child.wait().map(|s| s.success()).unwrap_or(false);
        if let Ok(mut m) = renders().lock()
            && let Some(j) = m.get_mut(&run_id) {
                j.state = if ok { "ok" } else { "fail" };
                if !ok {
                    j.error = Some("cutforge-render 非零退出;详见服务端控制台".into());
                }
            }
        crate::transport::events::publish_render(&root, &run_id, if ok { "ok" } else { "fail" });
    });
    envelope(true, "OK", "渲染已开始", json!({"runId": run_id}))
}

pub(crate) fn render_progress(run_id: &str) -> Value {
    let Ok(m) = renders().lock() else {
        return envelope(false, "INTERNAL", "渲染任务表不可用", json!({}));
    };
    match m.get(run_id) {
        Some(j) => envelope(true, "OK", "渲染进度", json!({
            "state": j.state,
            "lines": j.lines.iter().rev().take(30).rev().collect::<Vec<_>>(),
            "output": j.output,
            "error": j.error,
        })),
        None => envelope(false, "PRECONDITION_FAILED", &format!("未知 runId: {run_id}"), json!({})),
    }
}
