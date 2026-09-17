//! CutForge 脚本宿主与沙箱(计划书 5.6,ADR-0034 依赖纪律:纯标准库 + serde)。
//!
//! 脚本 = `cutforge-script-v1` JSON 批式步骤序列,**只能**通过 [`ToolDispatch`]
//! 调用命令通道工具;沙箱策略在派发**之前**拦截:
//! - 工具不在允许清单 → 拒(未知的 fs/exec/socket 类工具天然被拒);
//! - 编排类工具默认禁用(stage_/render/export_… 归 CutFlow 侧,须显式开启);
//! - 参数中出现工程目录之外的绝对路径 → 拒(路径白名单);
//! - 步数超限 / 总超时 → 拒。
//!
//! 宿主自身不派生子进程、不建 socket——越界能力**结构上不存在**,而非运行时拦截。

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 工具派发面(由 cutforge-mcp 注册表或测试桩实现)。
pub trait ToolDispatch {
    /// 本会话允许调用的工具名(策略层的白名单来源)。
    fn allowed_tools(&self) -> &[String];
    /// 执行并返回结果协议 envelope(JSON)。
    fn call(&mut self, tool: &str, args: &Value) -> Result<Value, String>;
}

/// 沙箱策略。
#[derive(Debug, Clone)]
pub struct Policy {
    pub project_root: PathBuf,
    pub max_steps: usize,
    pub timeout: Duration,
    /// 编排类工具(stage_run/render/…)默认禁止——它们会派生子进程。
    pub allow_orchestrate: bool,
}

impl Policy {
    pub fn new(project_root: &Path) -> Self {
        Self {
            project_root: project_root.to_path_buf(),
            max_steps: 200,
            timeout: Duration::from_secs(60),
            allow_orchestrate: false,
        }
    }
}

/// 计划书 5.2 的编排类工具(派生子进程封装 CutFlow 脚本)。
pub const ORCHESTRATE_TOOLS: &[&str] = &[
    "stage_run", "stage_rebuild", "verify_run", "sync_check", "render", "export_jianying",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscapeKind {
    UnknownTool(String),
    OrchestrateDisabled(String),
    PathEscape(String),
    StepOverflow,
    Timeout,
    BadFormat(String),
}

impl std::fmt::Display for EscapeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EscapeKind::UnknownTool(t) => write!(f, "GUARD_FAILED: 工具不在允许清单: {t}"),
            EscapeKind::OrchestrateDisabled(t) => write!(f, "GUARD_FAILED: 编排类工具默认禁用: {t}"),
            EscapeKind::PathEscape(p) => write!(f, "GUARD_FAILED: 路径逃逸(工程目录之外): {p}"),
            EscapeKind::StepOverflow => write!(f, "GUARD_FAILED: 步数超限"),
            EscapeKind::Timeout => write!(f, "GUARD_FAILED: 脚本超时"),
            EscapeKind::BadFormat(m) => write!(f, "SCHEMA_INVALID: {m}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct StepReport {
    pub tool: String,
    pub ok: bool,
    pub code: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ScriptReport {
    pub executed: Vec<StepReport>,
    pub rejected: Vec<(usize, String)>,
    /// 是否有步骤真正到达了派发器(逃逸审计的核心计数)。
    pub dispatched: usize,
}

impl ScriptReport {
    /// M4-4 判定语义:被拒步骤不得到达派发器。
    pub fn escapes(&self) -> usize {
        self.rejected.len()
    }
}

/// 执行批式脚本。格式非法整体拒绝;单步违规记录进 `rejected` 并终止(停写原则)。
pub fn run_script(
    script: &Value,
    dispatcher: &mut dyn ToolDispatch,
    policy: &Policy,
) -> Result<ScriptReport, EscapeKind> {
    if script.get("format").and_then(Value::as_str) != Some("cutforge-script-v1") {
        return Err(EscapeKind::BadFormat("缺 format: cutforge-script-v1".into()));
    }
    let Some(steps) = script.get("steps").and_then(Value::as_array) else {
        return Err(EscapeKind::BadFormat("缺 steps 数组".into()));
    };
    if steps.len() > policy.max_steps {
        return Err(EscapeKind::StepOverflow);
    }
    let started = Instant::now();
    let mut report = ScriptReport::default();
    for (i, step) in steps.iter().enumerate() {
        if started.elapsed() > policy.timeout {
            report.rejected.push((i, EscapeKind::Timeout.to_string()));
            return Ok(report);
        }
        let Some(tool) = step.get("tool").and_then(Value::as_str) else {
            report.rejected.push((i, EscapeKind::BadFormat("步骤缺 tool".into()).to_string()));
            break;
        };
        // 1) 白名单
        if !dispatcher.allowed_tools().iter().any(|t| t == tool) {
            report.rejected.push((i, EscapeKind::UnknownTool(tool.into()).to_string()));
            break;
        }
        // 2) 编排类默认禁用
        if !policy.allow_orchestrate && ORCHESTRATE_TOOLS.contains(&tool) {
            report.rejected.push((i, EscapeKind::OrchestrateDisabled(tool.into()).to_string()));
            break;
        }
        // 3) 路径白名单:任何字符串参数若是绝对路径必须在工程目录内
        let args = step.get("args").cloned().unwrap_or(Value::Object(Default::default()));
        if let Some(escape) = find_path_escape(&args, &policy.project_root) {
            report.rejected.push((i, EscapeKind::PathEscape(escape).to_string()));
            break;
        }
        // 4) 派发
        match dispatcher.call(tool, &args) {
            Ok(envelope) => {
                report.dispatched += 1;
                report.executed.push(StepReport {
                    tool: tool.into(),
                    ok: envelope.get("ok").and_then(Value::as_bool).unwrap_or(false),
                    code: envelope.get("code").and_then(Value::as_str).map(String::from),
                });
            }
            Err(e) => {
                report.executed.push(StepReport { tool: tool.into(), ok: false, code: Some(e) });
                break;
            }
        }
    }
    Ok(report)
}

/// 在参数树中寻找工程目录之外的绝对路径字符串;返回第一个逃逸路径。
fn find_path_escape(args: &Value, root: &Path) -> Option<String> {
    match args {
        Value::String(s) => {
            if looks_absolute(s) && !is_inside(s, root) {
                Some(s.clone())
            } else {
                None
            }
        }
        Value::Array(a) => a.iter().find_map(|v| find_path_escape(v, root)),
        Value::Object(m) => m.values().find_map(|v| find_path_escape(v, root)),
        _ => None,
    }
}

fn looks_absolute(s: &str) -> bool {
    let bytes = s.as_bytes();
    (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
        || s.starts_with('/')
        || s.starts_with('\\')
}

/// 大小写不敏感的前缀包含判定(Windows 盘符路径语义)。
fn is_inside(candidate: &str, root: &Path) -> bool {
    let norm = candidate.replace('\\', "/").to_ascii_lowercase();
    let root_norm = root.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    let root_norm = root_norm.trim_end_matches('/');
    norm == root_norm || norm.starts_with(&format!("{root_norm}/"))
}

/// 供测试与宿主复用:从工具清单构造允许集。
pub fn allow_list(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 记录型桩:allowed 之外的调用根本不该到达这里。
    struct Mock {
        allowed: Vec<String>,
        calls: AtomicUsize,
    }
    impl Mock {
        fn new(allowed: &[&str]) -> Self {
            Self { allowed: allow_list(allowed), calls: AtomicUsize::new(0) }
        }
    }
    impl ToolDispatch for Mock {
        fn allowed_tools(&self) -> &[String] {
            &self.allowed
        }
        fn call(&mut self, _tool: &str, _args: &Value) -> Result<Value, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"ok": true, "code": "OK", "message": "stub", "data": {}}))
        }
    }

    fn root_dir() -> PathBuf {
        std::env::temp_dir().join("cutforge-script-sandbox")
    }

    fn script(steps: Value) -> Value {
        json!({"format": "cutforge-script-v1", "steps": steps})
    }

    const QUERY: &[&str] = &["project_get", "oplog_tail"];
    const ALL: &[&str] = &[
        "project_get", "oplog_tail", "clip_update", "clip_split", "clip_delete", "undo", "redo",
        "stage_run", "render", "export_jianying",
    ];

    #[test]
    fn sandbox_escape() {
        let root = root_dir();
        let policy = Policy::new(&root);

        // 逃逸 1:越界读文件类工具(不在允许清单)
        let mut d = Mock::new(QUERY);
        let r = run_script(&script(json!([{"tool": "fs_read", "args": {"path": r"C:\Windows\win.ini"}}])), &mut d, &policy).unwrap();
        assert_eq!(r.rejected.len(), 1);
        assert!(r.rejected[0].1.contains("fs_read"));

        // 逃逸 2:起子进程类工具
        let r = run_script(&script(json!([{"tool": "exec", "args": {"cmd": "cmd /c dir"}}])), &mut d, &policy).unwrap();
        assert_eq!(r.rejected.len(), 1);
        assert!(r.rejected[0].1.contains("exec"));

        // 逃逸 3:建 socket 类工具
        let r = run_script(&script(json!([{"tool": "socket_connect", "args": {"host": "127.0.0.1"}}])), &mut d, &policy).unwrap();
        assert_eq!(r.rejected.len(), 1);
        assert!(r.rejected[0].1.contains("socket_connect"));

        // 逃逸 4:编排工具默认禁用(即使注册表里有)
        let mut d_all = Mock::new(ALL);
        let r = run_script(&script(json!([{"tool": "stage_run", "args": {"stage": "S3"}}])), &mut d_all, &policy).unwrap();
        assert!(r.rejected[0].1.contains("stage_run"));

        // 逃逸 5:路径白名单——参数携带工程外绝对路径
        let r = run_script(&script(json!([{"tool": "project_get", "args": {"root": r"C:\Windows"}}])), &mut d_all, &policy).unwrap();
        assert!(r.rejected[0].1.contains(r"C:\Windows"));
        let r = run_script(&script(json!([{"tool": "project_get", "args": {"root": root.to_string_lossy(), "out": "/etc/passwd"}}])), &mut d_all, &policy).unwrap();
        assert!(r.rejected[0].1.contains("/etc/passwd"));

        // 逃逸 6:步数超限
        let steps = (0..policy.max_steps + 1).map(|i| json!({"tool": "project_get", "args": {"root": root.to_string_lossy(), "i": i}})).collect::<Vec<_>>();
        assert!(run_script(&script(Value::Array(steps)), &mut d_all, &policy)
            .unwrap_err()
            .to_string()
            .contains("步数超限"));

        // 逃逸 7:格式不对
        assert!(matches!(run_script(&json!({"steps": []}), &mut d_all, &policy), Err(EscapeKind::BadFormat(_))));

        // 核心断言:以上所有逃逸尝试**零到达**派发器
        assert_eq!(d.calls.load(Ordering::SeqCst), 0);
        assert_eq!(d_all.calls.load(Ordering::SeqCst), 0, "成功逃逸数必须为 0");
    }

    #[test]
    fn legitimate_script_runs_and_reports() {
        let root = root_dir();
        let policy = Policy::new(&root);
        let mut d = Mock::new(ALL);
        let s = script(json!([
            {"tool": "project_get", "args": {"root": root.to_string_lossy()}},
            {"tool": "clip_update", "args": {"root": root.to_string_lossy(), "clipId": "V1-001"}}
        ]));
        let r = run_script(&s, &mut d, &policy).unwrap();
        assert!(r.rejected.is_empty());
        assert_eq!(r.dispatched, 2);
        assert_eq!(r.executed.len(), 2);
        assert!(r.executed.iter().all(|s| s.ok));
    }
}
