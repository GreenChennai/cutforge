//! M4-5 门禁:结果协议一致性。
//! 查询工具对夹具工程的返回值全部符合 {ok,code,message,data};
//! code 取值限于计划书 5.4 表;参数缺失 → PRECONDITION_FAILED;未知工具 → INTERNAL。

use serde_json::{json, Value};

const CODES: &[&str] = &[
    "OK", "CONFLICT", "SCHEMA_INVALID", "PRECONDITION_FAILED", "GUARD_FAILED",
    "JIANYING_RUNNING", "NO_CONFIG", "DEP_MISSING", "GREEN_SCREEN_INPUT", "INTERNAL",
];

fn assert_envelope(resp: &Value, what: &str) {
    for key in ["ok", "code", "message", "data"] {
        assert!(resp.get(key).is_some(), "{what}: 缺协议字段 {key}: {resp}");
    }
    assert!(resp["ok"].is_boolean(), "{what}: ok 必须是布尔");
    let code = resp["code"].as_str().expect("{what}: code 必须是字符串");
    assert!(CODES.contains(&code), "{what}: code '{code}' 不在 5.4 表内");
    assert!(resp["data"].is_object(), "{what}: data 必须是对象");
}

#[test]
fn protocol_conformance() {
    let root = cutforge_io::tests_fixture("mcp-protocol").unwrap();
    let root_s = root.to_string_lossy().to_string();

    // 全部 10 个查询工具:合法入参 → OK 且协议完整
    let queries: Vec<(&str, Value)> = vec![
        ("project_get", json!({"root": root_s})),
        ("wordline_get", json!({"root": root_s})),
        ("cutlist_get", json!({"root": root_s})),
        ("notes_list", json!({"root": root_s})),
        ("stage_status", json!({"root": root_s})),
        ("oplog_tail", json!({"root": root_s})),
        ("conflict_list", json!({"root": root_s})),
        ("render_probe", json!({"root": root_s})),
        ("capability_matrix", json!({})),
        ("timeline_get", json!({"root": root_s})),
    ];
    for (name, args) in queries {
        let resp = cutforge_mcp::dispatch(name, &args);
        assert_envelope(&resp, name);
        assert_eq!(resp["code"], json!("OK"), "{name} 对合法工程必须 OK: {resp}");
    }

    // 写工具:参数缺失 → PRECONDITION_FAILED(协议仍完整)
    for (name, args) in [
        ("clip_update", json!({})),
        ("clip_split", json!({})),
        ("notes_add", json!({"root": root_s})),
        ("notes_resolve", json!({"root": root_s})),
        ("sfx_add", json!({})),
    ] {
        let resp = cutforge_mcp::dispatch(name, &args);
        assert_envelope(&resp, name);
        assert_eq!(resp["code"], json!("PRECONDITION_FAILED"), "{name} 缺参: {resp}");
    }

    // 未知工具 → INTERNAL 且协议完整
    let resp = cutforge_mcp::dispatch("不存在", &json!({}));
    assert_envelope(&resp, "unknown");
    assert_eq!(resp["code"], json!("INTERNAL"));

    // 工程不存在 → NO_CONFIG(环境缺失,不得报成 OK)
    let resp = cutforge_mcp::dispatch("project_get", &json!({"root": "Z:/不存在/工程"}));
    assert_envelope(&resp, "missing-project");
    assert_eq!(resp["code"], json!("NO_CONFIG"));

    // 注册表与 mcp-tools.json 契约:31 行工具全部有名/有描述/有双 schema
    let names = cutforge_mcp::tool_names();
    assert_eq!(names.len(), 32);
    for t in cutforge_mcp::registry() {
        assert!(t["name"].is_string() && t["description"].is_string());
        assert!(t["inputSchema"].is_object(), "{} 缺 inputSchema", t["name"]);
        assert!(t["outputSchema"].is_object(), "{} 缺 outputSchema", t["name"]);
    }
    cutforge_io::fsutil::cleanup(&root);
}
