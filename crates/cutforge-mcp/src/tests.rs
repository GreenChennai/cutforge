use super::*;
use serde_json::json;

/// M8-5:能力矩阵单一真相源——MCP 工具与 docs/capability-matrix.json 同源,
/// status 只认封闭枚举;达成项必须有证据列(禁止无证据宣称)。
#[test]
fn capability_matrix_single_source() {
    let m = capability_matrix();
    assert_eq!(m["version"], json!(2));
    let items = m["items"].as_array().unwrap();
    assert_eq!(items.len(), 15, "15 项口径不变");
    let mut achieved = 0;
    for it in items {
        let status = it["status"].as_str().unwrap();
        assert!(
            matches!(status, "achieved" | "partial" | "missing" | "optional"),
            "status 必须在封闭枚举内: {status}"
        );
        if status == "achieved" {
            achieved += 1;
            assert!(it["evidence"].is_string() && !it["evidence"].as_str().unwrap().is_empty(),
                "达成项必须有实码/夹具证据: {}", it["item"]);
        }
        if status == "missing" || status == "partial" {
            assert!(it["target"].is_string(), "未达成项必须写明 M11 目标: {}", it["item"]);
        }
    }
    assert_eq!(achieved, 13, "M11 后实码达成 13 项(必达 13/13 + 0 可选;证据=parity_matrix)");
}

/// M8-5:python 启动器探测——本机/CI 至少一个可用,且返回的命令可执行。
#[test]
fn py_launcher_probe() {
    let py = py_launcher().expect("py -3/python3/python 至少一个必须可用");
    let out = std::process::Command::new(&py[0]).args(&py[1..]).arg("-V").output().unwrap();
    assert!(out.status.success());
}

/// M8-5:scriptArgs 序列化——数字不再被静默丢弃,复杂对象如实拒绝。
#[test]
fn script_args_serialization() {
    assert_eq!(script_arg_to_string(&json!("--force")).unwrap(), "--force");
    assert_eq!(script_arg_to_string(&json!(42)).unwrap(), "42");
    assert_eq!(script_arg_to_string(&json!(1.5)).unwrap(), "1.5");
    assert_eq!(script_arg_to_string(&json!(true)).unwrap(), "true");
    assert!(script_arg_to_string(&json!({"a": 1})).is_err());
}

/// M8-5:协议一致性——占位符清零,标注失败路径的 code 全部在 5.4 表内。
#[test]
fn notes_error_codes_in_table() {
    let root = cutforge_io::tests_fixture("mcp-notes-codes").unwrap();
    let root_s = root.to_string_lossy().to_string();
    for (name, args) in [
        ("notes_resolve", json!({"root": root_s, "noteId": "n-9999", "reply": "x", "opIds": ["op-1"]})),
        ("notes_reject", json!({"root": root_s, "noteId": "n-9999", "reason": "x"})),
    ] {
        let resp = dispatch(name, &args);
        assert_eq!(resp["code"], json!("PRECONDITION_FAILED"), "{name} 缺标注必须 PRECONDITION_FAILED: {resp}");
        assert!(CODES.contains(&resp["code"].as_str().unwrap()));
    }
    cutforge_io::fsutil::cleanup(&root);
}

/// M8-5:编排工具协议完整——CUTFLOW_REPO 缺失/脚本缺失不得崩,错误如实上报。
#[test]
fn orchestrate_tools_envelope_complete() {
    let root = cutforge_io::tests_fixture("mcp-orchestrate").unwrap();
    let root_s = root.to_string_lossy().to_string();
    for name in ["stage_run", "stage_rebuild", "verify_run", "sync_check", "render", "export_jianying"] {
        let resp = dispatch(name, &json!({"root": root_s, "scriptArgs": ["--status"]}));
        for key in ["ok", "code", "message", "data"] {
            assert!(resp.get(key).is_some(), "{name} 缺协议字段 {key}");
        }
        // 编排工具 = 子脚本结果**透传**(计划书 5.1):code 属 CutFlow 码表
        // (如 VERIFY_STATUS),不适用 5.4;基建类错误(DEP_MISSING 等)才落 5.4。
        // ok 随环境可 true/false;不假成功由退出码透传保证,不在此假设环境。
        assert!(resp["ok"].is_boolean(), "{name} ok 必须为布尔: {resp}");
    }
    cutforge_io::fsutil::cleanup(&root);
}

/// E3-1/E3-2 门禁:clip_add 内部走 Command::ClipInsert → rev 上涨 → 盘面出现新
/// clip;路径穿越拒绝;durationMs 缺省在有 ffprobe 时由探测自动填(时长语义断言)。
#[test]
fn clip_add_inserts_and_rejects_traversal() {
    let root = cutforge_io::tests_fixture("mcp-clip-add").unwrap();
    let root_s = root.to_string_lossy().to_string();

    // 穿越路径 → PRECONDITION_FAILED,不触碰盘面
    let rev0 = dispatch("project_get", &json!({"root": root_s}))["data"]["rev"].as_u64().unwrap();
    let bad = dispatch("clip_add", &json!({"root": root_s, "trackId": "V1", "src": "../逃逸.mp4", "startMs": 0}));
    assert_eq!(bad["code"], json!("PRECONDITION_FAILED"), "{bad}");
    // track 不存在 → PRECONDITION_FAILED
    let no_track = dispatch("clip_add", &json!({"root": root_s, "trackId": "V9", "src": "a.mp4", "startMs": 0}));
    assert_eq!(no_track["code"], json!("PRECONDITION_FAILED"), "{no_track}");

    // 显式时长:不依赖 ffprobe(CI 兜底路径);requestId 去重语义由引擎承接
    cutforge_io::fsutil::ensure(&root.join("01_原始素材")).unwrap();
    cutforge_io::atomic::atomic_write(&root.join("01_原始素材/take1.mp4"), b"x").unwrap();
    let add = dispatch("clip_add", &json!({
        "root": root_s, "trackId": "V1", "src": "01_原始素材/take1.mp4",
        "startMs": 999_000, "durationMs": 1500, "volume": 0.5, "requestId": "add-1",
    }));
    assert_eq!(add["code"], json!("OK"), "clip_add 必须成功: {add}");
    assert!(add["data"]["rev"].as_u64().unwrap() > rev0, "rev 必须上涨");
    let after = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    let v1 = after["tracks"].as_array().unwrap().iter().find(|t| t["id"] == "V1").unwrap();
    let clip = v1["clips"].as_array().unwrap().iter().find(|c| c["src"] == "01_原始素材/take1.mp4");
    assert!(clip.is_some(), "盘面必须出现新 clip: {v1}");
    assert_eq!(clip.unwrap()["volume"], json!(0.5));
    cutforge_io::fsutil::cleanup(&root);
}

/// E3-3 门禁:media_browse 列出可导入媒体;非法目录 → PRECONDITION_FAILED。
#[test]
fn media_browse_lists_media_and_rejects_bad_dir() {
    let root = cutforge_io::tests_fixture("mcp-browse").unwrap();
    let root_s = root.to_string_lossy().to_string();
    cutforge_io::fsutil::ensure(&root.join("01_原始素材")).unwrap();
    cutforge_io::atomic::atomic_write(&root.join("01_原始素材/a.mp4"), b"x").unwrap();
    cutforge_io::atomic::atomic_write(&root.join("01_原始素材/notes.txt"), b"x").unwrap();

    let resp = dispatch("media_browse", &json!({"root": root_s, "dir": "01_原始素材"}));
    assert_eq!(resp["code"], json!("OK"), "{resp}");
    let files = resp["data"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "只列媒体扩展名,不列 .txt: {files:?}");
    assert_eq!(files[0]["path"], json!("01_原始素材/a.mp4"));
    assert_eq!(files[0]["kind"], json!("video"));

    let bad = dispatch("media_browse", &json!({"root": root_s, "dir": "../.."}));
    assert_eq!(bad["code"], json!("PRECONDITION_FAILED"), "穿越目录必须拒绝: {bad}");
    cutforge_io::fsutil::cleanup(&root);
}

/// RT-1 前置口径:produces_rev_mutation 的分类面(查询/静态类不采集)。
#[test]
fn mutation_classification() {
    for q in ["project_get", "timeline_get", "oplog_tail", "notes_list", "conflict_list",
              "render_probe", "stage_status", "media_probe", "media_browse", "capability_matrix",
              "render_run", "render_progress", "render_frame", "project_new",
              "clip_copy"] {
        assert!(!produces_rev_mutation(q), "{q} 不应计入会话变更");
    }
    for w in ["clip_update", "clip_add", "clip_delete", "clip_split", "clip_move",
              "track_add", "undo", "redo", "cut_apply", "notes_add",
              "transition_set", "motion_set", "bgm_set",
              "clip_trim", "clip_split_all", "track_update", "clip_gap_delete",
              "clip_paste_at"] {
        assert!(produces_rev_mutation(w), "{w} 应计入会话变更");
    }
}

/// T2.4 门禁:render_frame 参数校验面(缺参/未知格式/无工程;协议完整,
/// 错误码如实——不依赖 cutforge-render 二进制在位)。
#[test]
fn render_frame_param_guards() {
    // 缺 root → PRECONDITION_FAILED(派发表统一 root 探针)
    let r = dispatch("render_frame", &json!({}));
    assert_eq!(r["code"], json!("PRECONDITION_FAILED"), "{r}");
    // 缺 atMs → PRECONDITION_FAILED
    let root = cutforge_io::tests_fixture("mcp-frame-guards").unwrap();
    let root_s = root.to_string_lossy().to_string();
    let r = dispatch("render_frame", &json!({"root": root_s}));
    assert_eq!(r["code"], json!("PRECONDITION_FAILED"), "缺 atMs: {r}");
    // 未知 format → PRECONDITION_FAILED
    let r = dispatch("render_frame", &json!({"root": root_s, "atMs": 500, "format": "webp"}));
    assert_eq!(r["code"], json!("PRECONDITION_FAILED"), "未知 format: {r}");
    // 无工程 → NO_CONFIG
    let nowhere = cutforge_io::fsutil::temp_dir("mcp-frame-noproject");
    let r = dispatch("render_frame", &json!({"root": nowhere.to_string_lossy(), "atMs": 500}));
    assert_eq!(r["code"], json!("NO_CONFIG"), "无工程必须 NO_CONFIG: {r}");
    cutforge_io::fsutil::cleanup(&root);
    cutforge_io::fsutil::cleanup(&nowhere);
}

/// 阶段三门禁:transition_set/motion_set 走 ClipPatch,逐字段落盘;bgm_set 走
/// /bgm 项目级 op(src=null 清除);全部经既有 rev/冲突/原子写通道。
#[test]
fn transition_motion_bgm_tools_end_to_end() {
    let root = cutforge_io::tests_fixture("mcp-tmb-tools").unwrap();
    let root_s = root.to_string_lossy().to_string();
    let rev0 = dispatch("project_get", &json!({"root": root_s}))["data"]["rev"].as_u64().unwrap();

    // 缺参 → PRECONDITION_FAILED(协议完整)
    for (name, args) in [
        ("transition_set", json!({"root": root_s})),
        ("transition_set", json!({"root": root_s, "clipId": "V1-002"})),
        ("motion_set", json!({"root": root_s, "clipId": "V1-002"})),
        ("motion_set", json!({"root": root_s})),
        ("bgm_set", json!({"root": root_s})),
    ] {
        let resp = dispatch(name, &args);
        assert_eq!(resp["code"], json!("PRECONDITION_FAILED"), "{name} 缺参: {resp}");
        assert!(CODES.contains(&resp["code"].as_str().unwrap()));
    }

    // transition_set:设置转场(枚举内值)→ rev 上涨 → 盘面可读回
    let r = dispatch("transition_set", &json!({
        "root": root_s, "clipId": "V1-002", "type": "slideleft", "durMs": 320, "reason": "topic",
    }));
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert!(r["data"]["rev"].as_u64().unwrap() > rev0, "rev 必须上涨");
    // 部分合并:再给 fx,既有 type/durMs/reason 保持
    let r = dispatch("transition_set", &json!({
        "root": root_s, "clipId": "V1-002", "type": "slideleft", "fx": "tr.demo",
    }));
    assert_eq!(r["code"], json!("OK"), "{r}");
    // 枚举外 type → schema 层拒(SCHEMA_INVALID)
    let bad = dispatch("transition_set", &json!({"root": root_s, "clipId": "V1-002", "type": "爆闪"}));
    assert_eq!(bad["code"], json!("SCHEMA_INVALID"), "{bad}");

    // motion_set:in+out 设置;缺 clipId 已在上面覆盖
    let r = dispatch("motion_set", &json!({
        "root": root_s, "clipId": "V1-002", "in": "zoomIn", "inMs": 280, "out": "fadeOut",
    }));
    assert_eq!(r["code"], json!("OK"), "{r}");

    // 盘面核对:transition 按字段合并(fx 加入,其余保持),motion 全量在位
    let proj = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    let v1 = proj["tracks"].as_array().unwrap().iter().find(|t| t["id"] == "V1").unwrap();
    let clip = v1["clips"].as_array().unwrap().iter().find(|c| c["id"] == "V1-002").unwrap();
    assert_eq!(clip["transition"]["type"], json!("slideleft"));
    assert_eq!(clip["transition"]["durMs"], json!(320.0), "先设的 durMs 不得被部分合并清掉");
    assert_eq!(clip["transition"]["reason"], json!("topic"));
    assert_eq!(clip["transition"]["fx"], json!("tr.demo"));
    assert_eq!(clip["motion"]["in"], json!("zoomIn"));
    assert_eq!(clip["motion"]["inMs"], json!(280.0));
    assert_eq!(clip["motion"]["out"], json!("fadeOut"));

    // bgm_set:夹具工程自带 bgm → 合并 gainDb(src/ducking 保持);
    // "无 bgm 须先给 src"的前置拒绝由 engine 单测覆盖(MissingBgm)。
    let r = dispatch("bgm_set", &json!({"root": root_s, "gainDb": -12}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    let proj = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    assert_eq!(proj["bgm"]["gainDb"], json!(-12.0));
    assert_eq!(proj["bgm"]["src"], json!("03_assets/bgm/loop1.mp3"), "未给出的 src 不得被清掉");
    assert_eq!(proj["bgm"]["ducking"], json!(true));
    let bad_src = dispatch("bgm_set", &json!({"root": root_s, "src": "../逃逸.mp3"}));
    assert_eq!(bad_src["code"], json!("PRECONDITION_FAILED"), "bgm 路径穿越必须拒绝: {bad_src}");
    let rev_before_clear = dispatch("project_get", &json!({"root": root_s}))["data"]["rev"].as_u64().unwrap();
    let r = dispatch("bgm_set", &json!({"root": root_s, "src": null}));
    assert_eq!(r["code"], json!("OK"), "src:null 必须清除 bgm: {r}");
    assert!(r["data"]["rev"].as_u64().unwrap() > rev_before_clear);
    let proj = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    assert!(proj.get("bgm").is_none(), "清除后 bgm 必须消失: {proj}");

    // 撤销清除 → bgm 恢复(撤销语义对齐既有)
    let r = dispatch("undo", &json!({"root": root_s}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    let proj = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    assert_eq!(proj["bgm"]["src"], json!("03_assets/bgm/loop1.mp3"), "撤销清除必须恢复 bgm");
    assert_eq!(proj["bgm"]["gainDb"], json!(-12.0), "撤销只回退清除,不得连带回退合并");

    // clip_update 嵌套 patch 同通道:patch.transition/patch.motion 对象与专用工具同一承接
    let r = dispatch("clip_update", &json!({
        "root": root_s, "clipId": "V1-001",
        "patch": {"transition": {"type": "circleopen", "durMs": 450}, "motion": {"out": "slideOutRight", "outMs": 260}},
    }));
    assert_eq!(r["code"], json!("OK"), "{r}");
    let proj = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    let v1 = proj["tracks"].as_array().unwrap().iter().find(|t| t["id"] == "V1").unwrap();
    let first = v1["clips"].as_array().unwrap().iter().find(|c| c["id"] == "V1-001").unwrap();
    assert_eq!(first["transition"]["type"], json!("circleopen"));
    assert_eq!(first["transition"]["durMs"], json!(450.0));
    assert_eq!(first["motion"]["out"], json!("slideOutRight"));
    assert_eq!(first["motion"]["outMs"], json!(260.0));

    cutforge_io::fsutil::cleanup(&root);
}
