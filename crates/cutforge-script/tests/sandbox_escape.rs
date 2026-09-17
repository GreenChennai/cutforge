//! M4-4 门禁:沙箱无逃逸(独立集成测试;逃逸尝试零到达派发器)。

use cutforge_script::{run_script, Policy, ToolDispatch};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Mock {
    allowed: Vec<String>,
    calls: AtomicUsize,
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

#[test]
fn sandbox_escape_all_rejected() {
    let root = std::env::temp_dir().join("cutforge-sandbox-gate");
    let policy = Policy::new(&root);
    let mut d = Mock {
        allowed: ["project_get", "oplog_tail", "clip_update", "undo", "stage_run", "render"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        calls: AtomicUsize::new(0),
    };
    // 宿主语义:遇首次逃逸即停写(单步违规终止);六类逃逸逐脚本验证
    let escapes: Vec<Value> = vec![
        json!([{"tool": "fs_read",   "args": {"path": r"C:\Windows\win.ini"}}]),
        json!([{"tool": "exec",      "args": {"cmd": "cmd /c dir"}}]),
        json!([{"tool": "socket",    "args": {"host": "127.0.0.1", "port": 80}}]),
        json!([{"tool": "stage_run", "args": {"stage": "S3"}}]),
        json!([{"tool": "render",    "args": {}}]),
        json!([{"tool": "project_get", "args": {"root": r"C:\Windows"}}]),
    ];
    let mut total_rejected = 0usize;
    for steps in escapes {
        let script = json!({"format": "cutforge-script-v1", "steps": steps});
        let report = run_script(&script, &mut d, &policy).unwrap();
        assert_eq!(report.rejected.len(), 1, "逃逸必须被拒: {report:?}");
        assert_eq!(report.dispatched, 0);
        total_rejected += report.rejected.len();
    }
    assert_eq!(total_rejected, 6, "六类逃逸全部被拒");
    assert_eq!(d.calls.load(Ordering::SeqCst), 0, "成功逃逸数必须为 0(零到达派发器)");
}
