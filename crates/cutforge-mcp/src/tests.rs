use super::*;
use serde_json::json;

/// M8-5:能力矩阵单一真相源——MCP 工具与 docs/capability-matrix.json 同源,
/// status 只认封闭枚举;达成项必须有证据列(禁止无证据宣称)。
#[test]
fn capability_matrix_single_source() {
    let m = capability_matrix();
    assert_eq!(m["version"], json!(3));
    let items = m["items"].as_array().unwrap();
    // M11 时点 15 项;A4-BE2(T4.4/T4.9)增曲线变速/倒放/画布范围 → 18 项;
    // A4-BE3a(T4.5/T4.6)增转场库目录化/特效库/动效库 → 21 项;
    // 册五 A5(T5.2/T5.3/T5.6)增调色/曲线/LUT/示波器/分屏拷贝/轨道 EQ/动态/响度计/
    // ducking 参数化/硬件编码降级/编码参数面/bt709 标签/渲染队列/渲染日志 + HSL/HDR 降级登记 → 37 项;
    // A5-BE3(T5.4/T5.5)增复合/调整层/多机位/场景检测/OTIO-EDL 互操作 → 42 项;
    // 册六 A6(T6.3/T6.2)增导出格式矩阵/导出预设/区域导出/导出预检/多画幅批量/
    // 素材库/剪映随包 + 重构图(诚实降级)/本地转写(明确不做)→ 51 项;
    // 册七 A7(T7.6)增 .cfpkg 打包/解包 → 52 项
    assert_eq!(
        items.len(),
        52,
        "册七 A7 后 52 项口径(51 + T7.6 cfpkg 一项)"
    );
    let mut achieved = 0;
    for it in items {
        let status = it["status"].as_str().unwrap();
        assert!(
            matches!(status, "achieved" | "partial" | "missing" | "optional"),
            "status 必须在封闭枚举内: {status}"
        );
        if status == "achieved" {
            achieved += 1;
            assert!(
                it["evidence"].is_string() && !it["evidence"].as_str().unwrap().is_empty(),
                "达成项必须有实码/夹具证据: {}",
                it["item"]
            );
        }
        if status == "missing" || status == "partial" {
            assert!(
                it["target"].is_string(),
                "未达成项必须写明 M11 目标: {}",
                it["item"]
            );
        }
    }
    assert_eq!(
        achieved, 46,
        "M11+A4 19 项 + 册五 14 项 + T5.4/T5.5 五项 + 册六七项 + 册七 cfpkg 一项(HSL/HDR/重构图/转写登记降级为 missing;证据=parity C1-C4/G/A 夹具、interop 往返单测与 T6.3/T6.2 产物断言、cfpkg 容器往返单测)"
    );
}

/// M8-5:python 启动器探测——本机/CI 至少一个可用,且返回的命令可执行。
#[test]
fn py_launcher_probe() {
    let py = py_launcher().expect("py -3/python3/python 至少一个必须可用");
    let out = std::process::Command::new(&py[0])
        .args(&py[1..])
        .arg("-V")
        .output()
        .unwrap();
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
        (
            "notes_resolve",
            json!({"root": root_s, "noteId": "n-9999", "reply": "x", "opIds": ["op-1"]}),
        ),
        (
            "notes_reject",
            json!({"root": root_s, "noteId": "n-9999", "reason": "x"}),
        ),
    ] {
        let resp = dispatch(name, &args);
        assert_eq!(
            resp["code"],
            json!("PRECONDITION_FAILED"),
            "{name} 缺标注必须 PRECONDITION_FAILED: {resp}"
        );
        assert!(CODES.contains(&resp["code"].as_str().unwrap()));
    }
    cutforge_io::fsutil::cleanup(&root);
}

/// M8-5:编排工具协议完整——CUTFLOW_REPO 缺失/脚本缺失不得崩,错误如实上报。
#[test]
fn orchestrate_tools_envelope_complete() {
    let root = cutforge_io::tests_fixture("mcp-orchestrate").unwrap();
    let root_s = root.to_string_lossy().to_string();
    for name in [
        "stage_run",
        "stage_rebuild",
        "verify_run",
        "sync_check",
        "render",
        "export_jianying",
    ] {
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
    let rev0 = dispatch("project_get", &json!({"root": root_s}))["data"]["rev"]
        .as_u64()
        .unwrap();
    let bad = dispatch(
        "clip_add",
        &json!({"root": root_s, "trackId": "V1", "src": "../逃逸.mp4", "startMs": 0}),
    );
    assert_eq!(bad["code"], json!("PRECONDITION_FAILED"), "{bad}");
    // track 不存在 → PRECONDITION_FAILED
    let no_track = dispatch(
        "clip_add",
        &json!({"root": root_s, "trackId": "V9", "src": "a.mp4", "startMs": 0}),
    );
    assert_eq!(no_track["code"], json!("PRECONDITION_FAILED"), "{no_track}");

    // 显式时长:不依赖 ffprobe(CI 兜底路径);requestId 去重语义由引擎承接
    cutforge_io::fsutil::ensure(&root.join("01_原始素材")).unwrap();
    cutforge_io::atomic::atomic_write(&root.join("01_原始素材/take1.mp4"), b"x").unwrap();
    let add = dispatch(
        "clip_add",
        &json!({
            "root": root_s, "trackId": "V1", "src": "01_原始素材/take1.mp4",
            "startMs": 999_000, "durationMs": 1500, "volume": 0.5, "requestId": "add-1",
        }),
    );
    assert_eq!(add["code"], json!("OK"), "clip_add 必须成功: {add}");
    assert!(add["data"]["rev"].as_u64().unwrap() > rev0, "rev 必须上涨");
    let after = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    let v1 = after["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "V1")
        .unwrap();
    let clip = v1["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["src"] == "01_原始素材/take1.mp4");
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

    let resp = dispatch(
        "media_browse",
        &json!({"root": root_s, "dir": "01_原始素材"}),
    );
    assert_eq!(resp["code"], json!("OK"), "{resp}");
    let files = resp["data"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "只列媒体扩展名,不列 .txt: {files:?}");
    assert_eq!(files[0]["path"], json!("01_原始素材/a.mp4"));
    assert_eq!(files[0]["kind"], json!("video"));

    let bad = dispatch("media_browse", &json!({"root": root_s, "dir": "../.."}));
    assert_eq!(
        bad["code"],
        json!("PRECONDITION_FAILED"),
        "穿越目录必须拒绝: {bad}"
    );
    cutforge_io::fsutil::cleanup(&root);
}

/// RT-1 前置口径:produces_rev_mutation 的分类面(查询/静态类不采集)。
#[test]
fn mutation_classification() {
    for q in [
        "project_get",
        "timeline_get",
        "oplog_tail",
        "notes_list",
        "conflict_list",
        "render_probe",
        "stage_status",
        "media_probe",
        "media_browse",
        "capability_matrix",
        "render_run",
        "render_progress",
        "render_frame",
        "project_new",
        "clip_copy",
    ] {
        assert!(!produces_rev_mutation(q), "{q} 不应计入会话变更");
    }
    for w in [
        "clip_update",
        "clip_add",
        "clip_delete",
        "clip_split",
        "clip_move",
        "track_add",
        "undo",
        "redo",
        "cut_apply",
        "notes_add",
        "transition_set",
        "motion_set",
        "bgm_set",
        "clip_trim",
        "clip_split_all",
        "track_update",
        "clip_gap_delete",
        "clip_paste_at",
    ] {
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
    let r = dispatch(
        "render_frame",
        &json!({"root": root_s, "atMs": 500, "format": "webp"}),
    );
    assert_eq!(r["code"], json!("PRECONDITION_FAILED"), "未知 format: {r}");
    // 无工程 → NO_CONFIG
    let nowhere = cutforge_io::fsutil::temp_dir("mcp-frame-noproject");
    let r = dispatch(
        "render_frame",
        &json!({"root": nowhere.to_string_lossy(), "atMs": 500}),
    );
    assert_eq!(r["code"], json!("NO_CONFIG"), "无工程必须 NO_CONFIG: {r}");
    cutforge_io::fsutil::cleanup(&root);
    cutforge_io::fsutil::cleanup(&nowhere);
}

/// I1-M2 门禁:preview_zone_render 参数校验面(缺参/区间倒置/无工程;协议完整,
/// 错误码如实——不依赖 cutforge-render 二进制在位;实渲证据在渲染端 zone_render.rs)。
#[test]
fn preview_zone_render_param_guards() {
    // 缺 root → PRECONDITION_FAILED(派发表统一 root 探针)
    let r = dispatch("preview_zone_render", &json!({}));
    assert_eq!(r["code"], json!("PRECONDITION_FAILED"), "{r}");
    let root = cutforge_io::tests_fixture("mcp-zone-guards").unwrap();
    let root_s = root.to_string_lossy().to_string();
    // 缺区间参 → PRECONDITION_FAILED
    let r = dispatch("preview_zone_render", &json!({"root": root_s}));
    assert_eq!(
        r["code"],
        json!("PRECONDITION_FAILED"),
        "缺 startMs/endMs: {r}"
    );
    let r = dispatch(
        "preview_zone_render",
        &json!({"root": root_s, "startMs": 0}),
    );
    assert_eq!(r["code"], json!("PRECONDITION_FAILED"), "缺 endMs: {r}");
    // 区间倒置/零长 → PRECONDITION_FAILED
    let r = dispatch(
        "preview_zone_render",
        &json!({"root": root_s, "startMs": 2000, "endMs": 2000}),
    );
    assert_eq!(r["code"], json!("PRECONDITION_FAILED"), "零长区间: {r}");
    let r = dispatch(
        "preview_zone_render",
        &json!({"root": root_s, "startMs": 3000, "endMs": 1000}),
    );
    assert_eq!(r["code"], json!("PRECONDITION_FAILED"), "倒置区间: {r}");
    // 无工程 → NO_CONFIG
    let nowhere = cutforge_io::fsutil::temp_dir("mcp-zone-noproject");
    let r = dispatch(
        "preview_zone_render",
        &json!({"root": nowhere.to_string_lossy(), "startMs": 0, "endMs": 2000}),
    );
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
    let rev0 = dispatch("project_get", &json!({"root": root_s}))["data"]["rev"]
        .as_u64()
        .unwrap();

    // 缺参 → PRECONDITION_FAILED(协议完整)
    for (name, args) in [
        ("transition_set", json!({"root": root_s})),
        (
            "transition_set",
            json!({"root": root_s, "clipId": "V1-002"}),
        ),
        ("motion_set", json!({"root": root_s, "clipId": "V1-002"})),
        ("motion_set", json!({"root": root_s})),
        ("bgm_set", json!({"root": root_s})),
    ] {
        let resp = dispatch(name, &args);
        assert_eq!(
            resp["code"],
            json!("PRECONDITION_FAILED"),
            "{name} 缺参: {resp}"
        );
        assert!(CODES.contains(&resp["code"].as_str().unwrap()));
    }

    // transition_set:设置转场(枚举内值)→ rev 上涨 → 盘面可读回
    let r = dispatch(
        "transition_set",
        &json!({
            "root": root_s, "clipId": "V1-002", "type": "slideleft", "durMs": 320, "reason": "topic",
        }),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert!(r["data"]["rev"].as_u64().unwrap() > rev0, "rev 必须上涨");
    // 部分合并:再给 fx,既有 type/durMs/reason 保持
    let r = dispatch(
        "transition_set",
        &json!({
            "root": root_s, "clipId": "V1-002", "type": "slideleft", "fx": "tr.demo",
        }),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    // 枚举外 type → schema 层拒(SCHEMA_INVALID)
    let bad = dispatch(
        "transition_set",
        &json!({"root": root_s, "clipId": "V1-002", "type": "爆闪"}),
    );
    assert_eq!(bad["code"], json!("SCHEMA_INVALID"), "{bad}");

    // motion_set:in+out 设置;缺 clipId 已在上面覆盖
    let r = dispatch(
        "motion_set",
        &json!({
            "root": root_s, "clipId": "V1-002", "in": "zoomIn", "inMs": 280, "out": "fadeOut",
        }),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");

    // 盘面核对:transition 按字段合并(fx 加入,其余保持),motion 全量在位
    let proj = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    let v1 = proj["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "V1")
        .unwrap();
    let clip = v1["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "V1-002")
        .unwrap();
    assert_eq!(clip["transition"]["type"], json!("slideleft"));
    assert_eq!(
        clip["transition"]["durMs"],
        json!(320.0),
        "先设的 durMs 不得被部分合并清掉"
    );
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
    assert_eq!(
        proj["bgm"]["src"],
        json!("03_assets/bgm/loop1.mp3"),
        "未给出的 src 不得被清掉"
    );
    assert_eq!(proj["bgm"]["ducking"], json!(true));
    let bad_src = dispatch("bgm_set", &json!({"root": root_s, "src": "../逃逸.mp3"}));
    assert_eq!(
        bad_src["code"],
        json!("PRECONDITION_FAILED"),
        "bgm 路径穿越必须拒绝: {bad_src}"
    );
    let rev_before_clear = dispatch("project_get", &json!({"root": root_s}))["data"]["rev"]
        .as_u64()
        .unwrap();
    let r = dispatch("bgm_set", &json!({"root": root_s, "src": null}));
    assert_eq!(r["code"], json!("OK"), "src:null 必须清除 bgm: {r}");
    assert!(r["data"]["rev"].as_u64().unwrap() > rev_before_clear);
    let proj = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    assert!(proj.get("bgm").is_none(), "清除后 bgm 必须消失: {proj}");

    // 撤销清除 → bgm 恢复(撤销语义对齐既有)
    let r = dispatch("undo", &json!({"root": root_s}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    let proj = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    assert_eq!(
        proj["bgm"]["src"],
        json!("03_assets/bgm/loop1.mp3"),
        "撤销清除必须恢复 bgm"
    );
    assert_eq!(
        proj["bgm"]["gainDb"],
        json!(-12.0),
        "撤销只回退清除,不得连带回退合并"
    );

    // clip_update 嵌套 patch 同通道:patch.transition/patch.motion 对象与专用工具同一承接
    let r = dispatch(
        "clip_update",
        &json!({
            "root": root_s, "clipId": "V1-001",
            "patch": {"transition": {"type": "circleopen", "durMs": 450}, "motion": {"out": "slideOutRight", "outMs": 260}},
        }),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    let proj = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    let v1 = proj["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "V1")
        .unwrap();
    let first = v1["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "V1-001")
        .unwrap();
    assert_eq!(first["transition"]["type"], json!("circleopen"));
    assert_eq!(first["transition"]["durMs"], json!(450.0));
    assert_eq!(first["motion"]["out"], json!("slideOutRight"));
    assert_eq!(first["motion"]["outMs"], json!(260.0));

    cutforge_io::fsutil::cleanup(&root);
}

/// 册四收口(候 BE 了断):clip_update patch.huazi 显式清除语义——
/// null 与 {} 双形态均清除(单 Op,undo 可还原);非空对象整替换;非法类型 SCHEMA_INVALID;
/// 字段缺席不改(既有行为回归)。
#[test]
fn clip_update_huazi_clear_semantics() {
    let root = cutforge_io::tests_fixture("mcp-huazi-clear").unwrap();
    let root_s = root.to_string_lossy().to_string();
    let huazi_of = |p: serde_json::Value| {
        p["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == "V1")
            .unwrap()["clips"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "V1-001")
            .unwrap()
            .get("huazi")
            .cloned()
    };

    // 挂载(非空对象整替换)
    let r = dispatch(
        "clip_update",
        &json!({
            "root": root_s, "clipId": "V1-001", "patch": {"huazi": {"template": "hz.pop"}},
        }),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    let proj = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    assert_eq!(
        huazi_of(proj).unwrap()["template"],
        json!("hz.pop"),
        "挂载必须落盘"
    );
    let rev_set = dispatch("project_get", &json!({"root": root_s}))["data"]["rev"]
        .as_u64()
        .unwrap();

    // 显式 null 清除:单 Op、huazi 消失、undo 还原
    let r = dispatch(
        "clip_update",
        &json!({"root": root_s, "clipId": "V1-001", "patch": {"huazi": null}}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert_eq!(
        r["data"]["opIds"].as_array().unwrap().len(),
        1,
        "清除必须单 Op"
    );
    let proj = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    assert_eq!(huazi_of(proj), None, "null 清除后 huazi 必须消失");
    dispatch("undo", &json!({"root": root_s}));
    let proj = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    assert_eq!(
        huazi_of(proj).unwrap()["template"],
        json!("hz.pop"),
        "undo 必须还原花字挂载"
    );

    // 空对象 {} 清除(第二形态);再清除一次幂等(零变更仍 OK 回执)
    dispatch(
        "clip_update",
        &json!({"root": root_s, "clipId": "V1-001", "patch": {"huazi": {}}}),
    );
    let proj = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    assert_eq!(huazi_of(proj), None, "{{}} 清除后 huazi 必须消失");
    let rev_cleared = dispatch("project_get", &json!({"root": root_s}))["data"]["rev"]
        .as_u64()
        .unwrap();
    assert!(rev_cleared > rev_set);
    // 字段缺席 = 不改(既有行为回归;同值幂等回执)
    let r = dispatch(
        "clip_update",
        &json!({"root": root_s, "clipId": "V1-001", "patch": {"volume": 1.0}}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    let proj = dispatch("project_get", &json!({"root": root_s}))["data"]["project"].clone();
    assert_eq!(huazi_of(proj), None);
    // 非法类型拒绝
    let r = dispatch(
        "clip_update",
        &json!({"root": root_s, "clipId": "V1-001", "patch": {"huazi": "hz.pop"}}),
    );
    assert_eq!(r["code"], json!("SCHEMA_INVALID"), "{r}");
    // 非空对象缺 template 拒绝(空对象是清除哨兵,不是合法挂载)
    let r = dispatch(
        "clip_update",
        &json!({"root": root_s, "clipId": "V1-001", "patch": {"huazi": {"params": {"x": 1}}}}),
    );
    assert_eq!(r["code"], json!("SCHEMA_INVALID"), "{r}");
    cutforge_io::fsutil::cleanup(&root);
}

// ==================== V2-W1 MCP 服务轮(BUG-19 media_thumbs / BUG-17 framePath / R-11 / R-14) ====================

/// TC-MCP-THUMB-001(BUG-19):media_thumbs 一次请求多帧——4 个 atMs 一次返回
/// 4 张缩略图(磁盘缓存未命中即生成);同参二调全部命中缓存,不再逐帧单发 IPC。
#[test]
fn media_thumbs_batch_and_cache() {
    assert!(
        crate::media_tools::ffmpeg_available(),
        "ffmpeg 必须存在(媒体工具实测面)"
    );
    let root = std::env::temp_dir().join(format!("cf-thumbs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("01_原始素材")).unwrap();
    std::fs::create_dir_all(root.join("05_时间线工程")).unwrap();
    cutforge_io::atomic::atomic_write(
        &root.join("05_时间线工程/project.json"),
        serde_json::to_string_pretty(&json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "thumbs", "fps": 30,
            "canvas": {"width": 320, "height": 240}, "tracks": []
        }))
        .unwrap()
        .as_bytes(),
    )
    .unwrap();
    let r = std::process::Command::new(crate::media_tools::ff_bin())
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=2",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "01_原始素材/clip.mp4",
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let root_s = root.to_string_lossy().to_string();
    let args = json!({
        "root": root_s, "src": "01_原始素材/clip.mp4",
        "atMs": [100, 500, 900, 1500],
    });
    let r1 = dispatch("media_thumbs", &args);
    assert_eq!(r1["code"], json!("OK"), "{r1}");
    let thumbs = r1["data"]["thumbs"]
        .as_array()
        .expect("必须返回 thumbs 数组");
    assert_eq!(thumbs.len(), 4, "一次请求 4 帧必须返回 4 张: {thumbs:?}");
    for (i, t) in thumbs.iter().enumerate() {
        assert_eq!(t["atMs"], json!([100u64, 500, 900, 1500][i]));
        assert_eq!(t["cached"], json!(false), "首调必须全 miss: {t}");
        let f = t["file"].as_str().expect("必须带 file 相对路径");
        assert!(root.join(f).is_file(), "产物必须真实落盘: {f}");
    }
    // 二调:同参全部磁盘缓存命中
    let r2 = dispatch("media_thumbs", &args);
    assert_eq!(r2["code"], json!("OK"), "{r2}");
    for t in r2["data"]["thumbs"].as_array().unwrap() {
        assert_eq!(t["cached"], json!(true), "二调必须全命中: {t}");
    }
    // 混合:命中 + 未命中同请求合并(一新一旧)
    let r3 = dispatch(
        "media_thumbs",
        &json!({"root": root_s, "src": "01_原始素材/clip.mp4", "atMs": [500, 1200]}),
    );
    let thumbs3 = r3["data"]["thumbs"].as_array().unwrap();
    assert_eq!(thumbs3.len(), 2);
    assert_eq!(thumbs3[0]["cached"], json!(true));
    assert_eq!(thumbs3[1]["cached"], json!(false));
    cutforge_io::fsutil::cleanup(&root);
}

/// TC-MCP-CONTRACT-001(BUG-17):render_frame 成功响应必须显式携带
/// `framePath`(工程内相对路径、正斜杠)——壳不再"任意层级扫 .png";
/// 旧字段 path(绝对)/media(别名)同版本保留。
#[test]
fn render_frame_response_contract_frame_path() {
    let abs =
        std::path::PathBuf::from("D:/ws").join(".cutforge/render-cache/frame/abcdef0123456789.png");
    let media = ".cutforge/render-cache/frame/abcdef0123456789.png";
    let e = crate::progress::frame_success_envelope(
        1200,
        "png",
        false,
        "abcdef0123456789",
        &abs,
        media,
    );
    assert_eq!(e["ok"], json!(true));
    assert_eq!(
        e["data"]["framePath"],
        json!(".cutforge/render-cache/frame/abcdef0123456789.png"),
        "响应必须显式携带 framePath(工程内相对路径): {e}"
    );
    assert_eq!(
        e["data"]["framePath"], e["data"]["media"],
        "framePath 与旧别名 media 同值(兼容)"
    );
    assert!(
        e["data"]["path"].as_str().is_some(),
        "旧字段 path(绝对路径)保留一版: {e}"
    );
    assert!(
        !e["data"]["framePath"].as_str().unwrap().contains('\\'),
        "framePath 必须是正斜杠形态: {e}"
    );
    // schema 契约面:mcp-tools.json 的 render_frame 描述必须声明 framePath
    let def = crate::registry::tool_def("render_frame").expect("render_frame 必须在注册表");
    assert!(
        def["description"].as_str().unwrap().contains("framePath"),
        "契约描述必须声明 framePath 字段"
    );
}

/// TC-MCP-QUEUE-001(R-11):渲染队列持久化——1 running + 2 queued 时重启,
/// 重建后 running → interrupted(可重试),queued 原样;持久层为
/// .cutforge/render-queue.jsonl(append-only)。
#[test]
fn render_queue_survives_restart() {
    let root = cutforge_io::fsutil::temp_dir("mcp-queue-restart");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("05_时间线工程")).unwrap();
    cutforge_io::atomic::atomic_write(&root.join("05_时间线工程/project.json"), br#"{"rev":1}"#)
        .unwrap();
    // 入队 3 个任务(不经 render_run_async 的渲染依赖门——单测只锁队列语义)
    let ids: Vec<String> = (0..3)
        .map(|i| crate::progress::enqueue_test_job(&root, None, false, vec![format!("--arg{i}")]))
        .collect();
    // 第二个转 running(如实记账)
    crate::progress::mark_test_running(&ids[1]);
    // 模拟重启:清空内存态 + 丢弃已加载标记
    crate::progress::reset_queue_for_tests();
    crate::progress::queue_reload(&root);
    let list = crate::progress::render_queue_tool(
        &root,
        &json!({"root": root.to_string_lossy(), "action": "list"}),
    );
    assert_eq!(list["code"], json!("OK"), "{list}");
    let jobs = list["data"]["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 3, "重启后三个任务都必须在: {jobs:?}");
    let by_id = |rid: &str| {
        jobs.iter()
            .find(|j| j["runId"].as_str() == Some(rid))
            .unwrap()
    };
    assert_eq!(by_id(&ids[0])["state"], json!("queued"), "pending 原样恢复");
    assert_eq!(
        by_id(&ids[1])["state"],
        json!("interrupted"),
        "running → interrupted"
    );
    assert_eq!(by_id(&ids[2])["state"], json!("queued"), "pending 原样恢复");
    // interrupted 可一键重试(重新入队)
    let retry = crate::progress::render_queue_tool(
        &root,
        &json!({"root": root.to_string_lossy(), "action": "retry", "runId": ids[1]}),
    );
    assert_eq!(
        retry["code"],
        json!("OK"),
        "interrupted 必须可重试: {retry}"
    );
    assert_eq!(retry["data"]["state"], json!("queued"));
    // 持久层文件在位(append-only jsonl)
    assert!(
        root.join(".cutforge/render-queue.jsonl").is_file(),
        "队列必须持久化到 .cutforge/render-queue.jsonl"
    );
    // 清队列防 worker 拾起夹具任务起真渲染(测试不依赖 worker)
    crate::progress::reset_queue_for_tests();
    let _ = std::fs::remove_dir_all(&root);
}

/// TC-PERF-IDEMP-001(R-14):幂等判定走常驻索引——10 万 request_id 构索后,
/// 单次判定 O(1)(<1ms);索引构建一次 O(n)。
#[test]
fn idemp_index_lookup_is_constant_time() {
    let ops: Vec<cutforge_core::oplog::Op> = (0..100_000u64)
        .map(|i| cutforge_core::oplog::Op {
            op_id: format!("op-{i}"),
            ts: String::new(),
            actor: cutforge_core::oplog::Actor::agent("t"),
            target: cutforge_core::oplog::OpTarget {
                file: "project.json".into(),
                path: "/slug".into(),
            },
            op_kind: cutforge_core::oplog::OpKind::Set,
            before: json!(null),
            after: json!(i),
            base_rev: format!("rev-{i}"),
            rev: Some(i + 1),
            auto: None,
            target_id: None,
            caused_by: None,
            summary: "perf".into(),
            request_id: (i % 2 == 0).then(|| format!("req-{i}")),
        })
        .collect();
    let t0 = std::time::Instant::now();
    let index = crate::resident::idem_index_from_ops(&ops);
    let build = t0.elapsed();
    assert!(
        build.as_millis() < 2_000,
        "10 万条索引构建应在秒级: {build:?}"
    );
    // 单次判定 <1ms(含未命中;旧路径 has_request_id 全扫 10 万条)
    for probe in ["req-0", "req-99998", "req-1", "missing-rid"] {
        let t1 = std::time::Instant::now();
        let hit = index.contains(probe);
        let el = t1.elapsed();
        assert!(
            el.as_millis() < 1,
            "单次幂等判定必须 <1ms: {probe} = {el:?}"
        );
        assert_eq!(
            hit,
            probe != "missing-rid" && probe != "req-1",
            "{probe} 判定错误"
        );
    }
}

/// TC-CORE-IDEMP-001(R-14):索引与 OpLog 全扫判重等价——真实工程写路径
/// 产生的 request_id 全集,索引判定与 `has_request_id` 逐一一致;重启
/// (缓存逐出后重建)后仍等价(compact 重建接口语义)。
#[test]
fn idemp_index_equivalent_to_full_scan() {
    use std::path::Path;
    let root = cutforge_io::tests_fixture("mcp-idemp-index").unwrap();
    let root_s = root.to_string_lossy().to_string();
    cutforge_io::fsutil::ensure(&root.join("01_原始素材")).unwrap();
    cutforge_io::atomic::atomic_write(&root.join("01_原始素材/take1.mp4"), b"x").unwrap();
    let rids = ["idx-req-1", "idx-req-2", "idx-req-3"];
    for (i, rid) in rids.iter().enumerate() {
        let r = dispatch(
            "clip_add",
            &json!({
                "root": root_s, "trackId": "V1", "src": "01_原始素材/take1.mp4",
                "startMs": 1_000_000 + 1000 * (i as u64), "durationMs": 500, "requestId": rid,
            }),
        );
        assert_eq!(r["code"], json!("OK"), "{r}");
    }
    let ws = cutforge_io::Workspace::open(Path::new(&root_s)).unwrap();
    let log = ws.engine().oplog();
    // 同一 request_id 重复提交 → 引擎幂等回执
    let dup = dispatch(
        "clip_add",
        &json!({
            "root": root_s, "trackId": "V1", "src": "01_原始素材/take1.mp4",
            "startMs": 999_000, "durationMs": 500, "requestId": "idx-req-1",
        }),
    );
    assert_eq!(
        dup["data"]["idempotent"],
        json!(true),
        "重复 requestId 必须幂等回执: {dup}"
    );
    // 常驻索引(增量维护)与全扫等价
    crate::resident::_clear_for_tests();
    let probe = |rid: &str| -> bool {
        let seen = crate::resident::request_id_seen(&root_s, Path::new(&root_s), rid);
        let scanned = log.has_request_id(rid);
        assert_eq!(seen, scanned, "{rid}: 索引判定必须与 OpLog 全扫等价");
        seen
    };
    for rid in rids {
        assert!(probe(rid), "{rid} 必须命中");
    }
    assert!(!probe("idx-req-absent"), "未提交过的 id 不得命中");
    // 模拟重启:逐出缓存 → 重建(指纹变化后索引随工作区重开重建)
    crate::resident::_clear_for_tests();
    let ws2 = cutforge_io::Workspace::open(Path::new(&root_s)).unwrap();
    let rebuilt = crate::resident::idem_index_from_ops(ws2.engine().oplog().ops());
    for rid in rids {
        assert!(
            rebuilt.contains(rid),
            "重建后 {rid} 必须命中(compact 重建接口语义)"
        );
    }
    assert!(!rebuilt.contains("idx-req-absent"));
    cutforge_io::fsutil::cleanup(&root);
}
