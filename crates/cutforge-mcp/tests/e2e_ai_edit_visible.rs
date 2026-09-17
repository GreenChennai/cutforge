//! M4-2 门禁:AI 改一处 → 编辑器可见(≤1 s,且差异可定位到具体字段)。

use cutforge_core::engine::{Answer, Query};
use cutforge_io::Workspace;
use serde_json::json;
use std::time::Instant;

#[test]
fn e2e_ai_edit_visible() {
    let root = cutforge_io::tests_fixture("mcp-e2e-visible").unwrap();
    let root_s = root.to_string_lossy().to_string();

    let t0 = Instant::now();
    // AI 经 MCP 改一处
    let resp = cutforge_mcp::dispatch(
        "clip_update",
        &json!({
            "root": root_s,
            "clipId": "V1-001",
            "patch": {"durationMs": 8000},
            "causedBy": ["n-0001"],
            "summary": "AI 演示改动",
        }),
    );
    assert_eq!(resp["ok"], json!(true), "工具调用失败: {resp}");

    // 模拟编辑器:重开工程拉视图,断言 ≤1 s 内可见
    let ws = Workspace::open(&root).unwrap();
    match ws.engine().query(Query::Timeline) {
        Answer::Timeline(tl) => {
            let v1 = tl.iter().find(|(id, ..)| id == "V1-001").expect("片段仍在");
            assert_eq!(v1.2, 8000, "编辑器视图必须反映改动");
        }
        _ => panic!("Timeline 查询失败"),
    }
    let elapsed = t0.elapsed();
    assert!(elapsed.as_millis() <= 1000, "可见延迟须 ≤1s,实际 {elapsed:?}");

    // 差异面板定位:OpLog 中该改动可定位到具体字段
    match ws.engine().query(Query::OpLogTail { since_rev: None, actor_kind: None }) {
        Answer::Ops(ops) => {
            let op = ops.last().unwrap();
            assert!(op.target.path.starts_with("/tracks/"), "指针路径: {}", op.target.path);
            assert_eq!(op.after["durationMs"], json!(8000));
            assert_eq!(op.summary, "AI 演示改动");
        }
        _ => panic!("OpLog 查询失败"),
    }
    cutforge_io::fsutil::cleanup(&root);
}
