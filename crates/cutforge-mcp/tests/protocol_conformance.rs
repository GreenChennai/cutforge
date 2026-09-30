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
        // 阶段三新工具同样受缺参门禁约束
        ("transition_set", json!({"root": root_s})),
        ("transition_set", json!({"root": root_s, "clipId": "V1-002"})),
        ("motion_set", json!({"root": root_s})),
        ("motion_set", json!({"root": root_s, "clipId": "V1-002"})),
        ("bgm_set", json!({"root": root_s})),
        // T2.4:render_frame 缺 atMs 同样 PRECONDITION_FAILED(缺 root 由统一探针覆盖)
        ("render_frame", json!({"root": root_s})),
        // 册四 A4 T4.2 时间线编辑全工具缺参/非法参数面
        ("clip_trim", json!({"root": root_s})),
        ("clip_trim", json!({"root": root_s, "clipId": "V1-001", "mode": "prune", "deltaMs": 100})),
        ("clip_trim", json!({"root": root_s, "clipId": "V1-001", "mode": "trim", "deltaMs": 100})),
        ("clip_split_all", json!({"root": root_s})),
        ("track_update", json!({"root": root_s})),
        ("track_update", json!({"root": root_s, "trackId": "V1", "patch": {}})),
        ("clip_gap_delete", json!({"root": root_s})),
        ("clip_copy", json!({"root": root_s})),
        ("clip_paste_at", json!({"root": root_s})),
        ("clip_paste_at", json!({"root": root_s, "trackId": "V1"})),
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
    // (数量与 json 对拍;阶段二 34→38;阶段三新增 transition_set/motion_set/bgm_set → 41;
    //  册二 A2 新增 render_frame → 42;册四 A4 新增 clip_trim/clip_split_all/
    //  track_update/clip_gap_delete/clip_copy/clip_paste_at → 48;册四 A4-BE3b 增
    //  text_add/subtitle_import/subtitle_replace/subtitle_export/media_peaks/
    //  media_thumbnail/media_proxy/audio_beats → 56;册五 A5 增 lut_import/
    //  scope_data/audio_loudness/encode_probe/render_queue → 61)
    let names = cutforge_mcp::tool_names();
    assert_eq!(names.len(), 61, "B7 口径:工具数以 schemas/mcp-tools.json 为准(册五 A5 增 lut_import/scope_data/audio_loudness/encode_probe/render_queue)");
    for t in cutforge_mcp::registry() {
        assert!(t["name"].is_string() && t["description"].is_string());
        assert!(t["inputSchema"].is_object(), "{} 缺 inputSchema", t["name"]);
        assert!(t["outputSchema"].is_object(), "{} 缺 outputSchema", t["name"]);
    }
    // kind 口径:15 查询 + 30 写 + 16 编排(与 _doc 同句;册五 A5 56→61)
    let mut kinds = std::collections::BTreeMap::new();
    for t in cutforge_mcp::registry() {
        *kinds.entry(t["kind"].as_str().unwrap().to_string()).or_insert(0usize) += 1;
    }
    assert_eq!(kinds.get("query"), Some(&15), "查询 15:{kinds:?}");
    assert_eq!(kinds.get("write"), Some(&30), "写 30:{kinds:?}");
    assert_eq!(kinds.get("orchestrate"), Some(&16), "编排 16:{kinds:?}");

    // M4-1 单注册表双通道:注册表与 dispatch **逐一相等**——每个注册工具都必须有
    // 实现分支,不得出现"已注册但未实现"。统一以缺 root 空参探针:所有工具(capability_matrix
    // 除外)在 {} 下必须返回协议完整的 PRECONDITION_FAILED;"未实现"分支返回 INTERNAL,在此红。
    for t in cutforge_mcp::registry() {
        let name = t["name"].as_str().unwrap();
        let resp = cutforge_mcp::dispatch(name, &json!({}));
        assert_envelope(&resp, name);
        let msg = resp["message"].as_str().unwrap_or_default();
        assert!(!msg.contains("未实现"), "{name} 已注册但 dispatch 无实现分支: {resp}");
        let expected = if name == "capability_matrix" { "OK" } else { "PRECONDITION_FAILED" };
        assert_eq!(resp["code"], json!(expected), "{name} 空参探针口径漂移: {resp}");
    }
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

/// 册四 A4 T4.2:时间线编辑全工具的 dispatch 级闭环——roll/trim/slip/slide、
/// 全轨分割、轨道属性、间隙删除、复制粘贴,外加 undo 逐级还原与守护拒绝。
/// 夹具工程迁移后:V1-001[0,8400) V1-002[8400,14600) A1-001[8400,8800)。
#[test]
fn edit_ops_tools_full_chain() {
    let root = cutforge_io::tests_fixture("mcp-edit-ops").unwrap();
    let root_s = root.to_string_lossy().to_string();
    let call = |name: &str, args: Value| cutforge_mcp::dispatch(name, &args);

    // 剪贴板空板:粘贴必须 PRECONDITION_FAILED(先于任何 copy)
    let r = call("clip_paste_at", json!({"root": root_s, "trackId": "V1", "startMs": 15000}));
    assert_eq!(r["code"], json!("PRECONDITION_FAILED"), "{r}");

    // 1) roll out +200:V1-001[0,8600) / V1-002[8600,14600)(srcIn 21800)
    let r = call("clip_trim", json!({"root": root_s, "clipId": "V1-001", "mode": "roll", "edge": "out", "deltaMs": 200}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    // 2) trim out -600:V1-001[0,8000),与 V1-002 间出现 600ms 间隙
    let r = call("clip_trim", json!({"root": root_s, "clipId": "V1-001", "mode": "trim", "edge": "out", "deltaMs": -600}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    // 3) slip -300:内容平移 srcIn 21800→21500,时间线占位不变
    //    (夹具无媒体文件,正向 slip 的素材末尾约束需 ffprobe 会 DEP_MISSING,故取负向)
    let r = call("clip_trim", json!({"root": root_s, "clipId": "V1-002", "mode": "slip", "deltaMs": -300}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    // 4) slide -600:V1-002[8000,14000)(左侧非贴合,间隙吸收;内容窗不变)
    let r = call("clip_trim", json!({"root": root_s, "clipId": "V1-002", "mode": "slide", "deltaMs": -600}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    // roll 无贴合邻居 → GUARD_FAILED(内核不变量经既有 Reject 码)
    let r = call("clip_trim", json!({"root": root_s, "clipId": "V1-002", "mode": "roll", "edge": "out", "deltaMs": 100}));
    assert_eq!(r["code"], json!("GUARD_FAILED"), "{r}");
    // 5) clip_split_all @4000:V1-001[0,4000)+V1-003[4000,8000),单 Op
    let r = call("clip_split_all", json!({"root": root_s, "tMs": 4000}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert_eq!(r["data"]["opIds"].as_array().unwrap().len(), 1, "全轨分割单 Op");
    // 6) clip_gap_delete(A2 头部 8400ms 空档;迁移器轨道 id=字母+下标,A1 是视频轨):A1-001 → [0,400)
    let r = call("clip_gap_delete", json!({"root": root_s, "trackId": "A2", "tMs": 100}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    // t 位于片段内部 → GUARD_FAILED
    let r = call("clip_gap_delete", json!({"root": root_s, "trackId": "V1", "tMs": 1000}));
    assert_eq!(r["code"], json!("GUARD_FAILED"), "{r}");
    // 7) track_update:轨道属性进 IR 并回读可见(TrackPatch 按字段合并)
    let r = call("track_update", json!({"root": root_s, "trackId": "V1",
                 "patch": {"name": "主画面A4", "mute": true, "heightPx": 260, "color": "#3366CC"}}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    let r = call("track_update", json!({"root": root_s, "trackId": "V1", "patch": {"heightPx": 260}}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert_eq!(r["data"]["idempotent"], json!(true), "同值 patch 幂等回执");
    // 8) clip_copy + clip_paste_at:带属性粘贴,新 id,rev 推进
    let rev_before = call("project_get", json!({"root": root_s}))["data"]["rev"].clone();
    let r = call("clip_copy", json!({"root": root_s, "clipId": "V1-003"}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert_eq!(r["data"]["clipboard"]["kind"], json!("video"));
    let rev_after_copy = call("project_get", json!({"root": root_s}))["data"]["rev"].clone();
    assert_eq!(rev_before, rev_after_copy, "clip_copy 不得升 rev");
    // 跨 kind 粘贴拒绝(剪贴板来源 video → 音频轨)
    let r = call("clip_paste_at", json!({"root": root_s, "trackId": "A2", "startMs": 15000}));
    assert_eq!(r["code"], json!("GUARD_FAILED"), "{r}");
    let r = call("clip_paste_at", json!({"root": root_s, "trackId": "V1", "startMs": 15000,
                 "requestId": "conf-paste-1"}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    // 9) 投影面:全链结果逐项可见(契约链端到端)
    let r = call("timeline_get", json!({"root": root_s}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    let rows: Vec<&Value> = r["data"]["clips"].as_array().unwrap()
        .iter().filter(|c| c["track"] == json!("V1")).collect();
    assert!(rows.iter().any(|c| c["id"] == json!("V1-003") && c["startMs"] == json!(4000)));
    let pasted = rows.iter().find(|c| c["id"] == json!("V1-004")).expect("粘贴片段必须在投影");
    assert_eq!(pasted["startMs"], json!(15000));
    assert_eq!(pasted["durationMs"], json!(4000), "粘贴保留时长(带属性)");
    let pv = call("project_get", json!({"root": root_s}));
    let v1 = pv["data"]["project"]["tracks"].as_array().unwrap().iter()
        .find(|t| t["id"] == json!("V1")).unwrap();
    assert_eq!(v1["name"], json!("主画面A4"));
    assert_eq!(v1["mute"], json!(true));
    assert_eq!(v1["heightPx"], json!(260));
    assert_eq!(v1["color"], json!("#3366CC"));
    // 10) undo 八步(roll/trim/slip/slide/split_all/gap_delete/track_update/paste)回到夹具原状
    for _ in 0..8 {
        let r = call("undo", json!({"root": root_s}));
        assert_eq!(r["code"], json!("OK"), "{r}");
    }
    let r = call("timeline_get", json!({"root": root_s}));
    let rows = r["data"]["clips"].as_array().unwrap();
    assert_eq!(rows.len(), 3, "undo 后应回到夹具三片段: {rows:?}");
    let v1 = call("project_get", json!({"root": root_s}))["data"]["project"]["tracks"].as_array().unwrap()
        .iter().find(|t| t["id"] == json!("V1")).unwrap().clone();
    assert!(v1.get("mute").is_none() && v1.get("heightPx").is_none(), "undo 后轨道属性字段消失");
    assert_eq!(v1["clips"].as_array().unwrap()[0]["durationMs"], json!(8400));
    cutforge_io::fsutil::cleanup(&root);
}
