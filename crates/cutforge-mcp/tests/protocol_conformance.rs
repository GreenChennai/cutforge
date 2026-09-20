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

    // 全部查询工具:合法入参 → OK 且协议完整
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
        // E3-3:素材浏览(夹具无媒体 → 空清单,仍必须 OK 且协议完整)
        ("media_browse", json!({"root": root_s})),
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
        ("clip_add", json!({})),
        ("notes_add", json!({"root": root_s})),
        ("notes_resolve", json!({"root": root_s})),
        ("sfx_add", json!({})),
    ] {
        let resp = cutforge_mcp::dispatch(name, &args);
        assert_envelope(&resp, name);
        assert_eq!(resp["code"], json!("PRECONDITION_FAILED"), "{name} 缺参: {resp}");
    }

    // E3-2:media_probe 对不存在的路径 → PRECONDITION_FAILED(协议完整;有 ffprobe
    // 的机器同样在此路径返回,不依赖环境)
    let resp = cutforge_mcp::dispatch("media_probe", &json!({"root": root_s, "src": "无此文件.mp4"}));
    assert_envelope(&resp, "media_probe");
    assert_eq!(resp["code"], json!("PRECONDITION_FAILED"), "media_probe 缺文件: {resp}");

    // 未知工具 → INTERNAL 且协议完整
    let resp = cutforge_mcp::dispatch("不存在", &json!({}));
    assert_envelope(&resp, "unknown");
    assert_eq!(resp["code"], json!("INTERNAL"));

    // 工程不存在 → NO_CONFIG(环境缺失,不得报成 OK)
    let resp = cutforge_mcp::dispatch("project_get", &json!({"root": "Z:/不存在/工程"}));
    assert_envelope(&resp, "missing-project");
    assert_eq!(resp["code"], json!("NO_CONFIG"));

    // 注册表与 mcp-tools.json 契约:工具全部有名/有描述/有双 schema
    // (数量与 json 对拍;阶段二新增 clip_add/media_probe/media_browse/project_new → 38)
    let names = cutforge_mcp::tool_names();
    assert_eq!(names.len(), 38, "B7 口径:工具数以 schemas/mcp-tools.json 为准");
    for t in cutforge_mcp::registry() {
        assert!(t["name"].is_string() && t["description"].is_string());
        assert!(t["inputSchema"].is_object(), "{} 缺 inputSchema", t["name"]);
        assert!(t["outputSchema"].is_object(), "{} 缺 outputSchema", t["name"]);
    }
    // kind 口径:13 查询 + 18 写 + 7 编排(与 _doc 同句)
    let mut kinds = std::collections::BTreeMap::new();
    for t in cutforge_mcp::registry() {
        *kinds.entry(t["kind"].as_str().unwrap().to_string()).or_insert(0usize) += 1;
    }
    assert_eq!(kinds.get("query"), Some(&13), "查询 13:{kinds:?}");
    assert_eq!(kinds.get("write"), Some(&18), "写 18:{kinds:?}");
    assert_eq!(kinds.get("orchestrate"), Some(&7), "编排 7:{kinds:?}");
    cutforge_io::fsutil::cleanup(&root);
}

/// E6-1/B11:project_new 走 dispatch 即可从零建工程;随后 project_get/timeline_get
/// 必须可用(从零剪闭环的第一跳)。已存在 → PRECONDITION_FAILED(拒绝覆盖)。
#[test]
fn project_new_from_zero_and_query() {
    let root = cutforge_io::fsutil::temp_dir("mcp-project-new");
    let root_s = root.to_string_lossy().to_string();
    let resp = cutforge_mcp::dispatch("project_new", &serde_json::json!({
        "root": root_s, "slug": "从零", "fps": 30, "canvasW": 1080, "canvasH": 1920,
    }));
    assert_eq!(resp["code"], json!("OK"), "{resp}");
    for name in ["project_get", "timeline_get"] {
        let r = cutforge_mcp::dispatch(name, &json!({"root": root_s}));
        assert_eq!(r["code"], json!("OK"), "{name} 对新工程必须 OK: {r}");
    }
    let dup = cutforge_mcp::dispatch("project_new", &json!({"root": root_s}));
    assert_eq!(dup["code"], json!("PRECONDITION_FAILED"), "重复创建必须拒绝: {dup}");
    cutforge_io::fsutil::cleanup(&root);
}

/// E6-3/B14:只读查询不申请排他锁——project_get 后 `.cutforge/lock` 不存在
/// (写操作才经 open_exclusive 创建锁)。
#[test]
fn readonly_query_holds_no_lock() {
    let root = cutforge_io::tests_fixture("mcp-readonly-lock").unwrap();
    let root_s = root.to_string_lossy().to_string();
    let resp = cutforge_mcp::dispatch("project_get", &json!({"root": root_s}));
    assert_eq!(resp["code"], json!("OK"), "{resp}");
    assert!(!root.join(".cutforge/lock").exists(), "只读查询不得留下工程锁");
    cutforge_io::fsutil::cleanup(&root);
}
