//! M4-3 门禁:标注全链路闭环(MCP 工具级)。
//! notes_add(user) → notes_list(agent 读到) → 执行改动 → notes_resolve
//! → notes_list 显示 resolved 且可回看 opIds;全链路 ≤ 3 s。

use serde_json::json;
use std::time::Instant;

#[test]
fn e2e_note_loop() {
    let t0 = Instant::now();
    let root = cutforge_io::tests_fixture("mcp-note-loop").unwrap();
    let root_s = root.to_string_lossy().to_string();

    // 1. 用户经 MCP 创建标注
    let resp = cutforge_mcp::dispatch(
        "notes_add",
        &json!({"root": root_s, "anchor": {"kind": "clip", "ref": "V1-001", "tMs": 4000},
                "body": "这里删短一点", "author": "user", "requestId": "loop-1"}),
    );
    assert_eq!(resp["ok"], json!(true), "{resp}");
    let note_id = resp["data"]["noteId"].as_str().unwrap().to_string();

    // 2. AI 读标注列表
    let resp = cutforge_mcp::dispatch("notes_list", &json!({"root": root_s, "state": "open"}));
    assert_eq!(resp["ok"], json!(true));
    assert!(resp["data"]["notes"].to_string().contains(&note_id), "AI 必须读到新标注");

    // 3. AI 执行改动(causedBy 指向标注)
    let resp = cutforge_mcp::dispatch(
        "clip_update",
        &json!({"root": root_s, "clipId": "V1-001", "patch": {"durationMs": 8000},
                "causedBy": [note_id], "summary": "按标注删短"}),
    );
    assert_eq!(resp["ok"], json!(true), "{resp}");
    let op_id = resp["data"]["opIds"][0].as_str().unwrap().to_string();

    // 4. AI 结案回执
    let resp = cutforge_mcp::dispatch(
        "notes_resolve",
        &json!({"root": root_s, "noteId": note_id, "reply": "已删短到 8s", "opIds": [op_id]}),
    );
    assert_eq!(resp["ok"], json!(true), "{resp}");

    // 5. 编辑器/列表视角:resolved 且 opIds 可回看
    let resp = cutforge_mcp::dispatch("notes_list", &json!({"root": root_s, "state": "resolved"}));
    assert!(resp["data"]["notes"].to_string().contains(&op_id), "回执 opIds 必须可回看");

    // 6. 幂等:同 request_id 重复建标 → 不重复
    let before: usize = {
        let resp = cutforge_mcp::dispatch("notes_list", &json!({"root": root_s}));
        resp["data"]["total"].as_u64().unwrap() as usize
    };
    let resp = cutforge_mcp::dispatch(
        "notes_add",
        &json!({"root": root_s, "anchor": {"kind": "clip", "ref": "V1-001", "tMs": 4000},
                "body": "重复请求", "requestId": "loop-dup"}),
    );
    let _ = resp;
    let resp2 = cutforge_mcp::dispatch(
        "notes_add",
        &json!({"root": root_s, "anchor": {"kind": "clip", "ref": "V1-001", "tMs": 4000},
                "body": "重复请求", "requestId": "loop-dup"}),
    );
    assert_eq!(resp2["ok"], json!(true));
    let after: usize = {
        let resp = cutforge_mcp::dispatch("notes_list", &json!({"root": root_s}));
        resp["data"]["total"].as_u64().unwrap() as usize
    };
    assert_eq!(before + 1, after, "同 request_id 只应新增一条");

    let elapsed = t0.elapsed();
    assert!(elapsed.as_secs() <= 3, "全链路须 ≤3s,实际 {elapsed:?}");
}
