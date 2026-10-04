//! M4-5 门禁:结果协议一致性。
//! 查询工具对夹具工程的返回值全部符合 {ok,code,message,data};
//! code 取值限于计划书 5.4 表;参数缺失 → PRECONDITION_FAILED;未知工具 → INTERNAL。

use serde_json::{Value, json};

const CODES: &[&str] = &[
    "OK",
    "CONFLICT",
    "SCHEMA_INVALID",
    "PRECONDITION_FAILED",
    "GUARD_FAILED",
    "JIANYING_RUNNING",
    "NO_CONFIG",
    "DEP_MISSING",
    "GREEN_SCREEN_INPUT",
    "INTERNAL",
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
        // 册七 A7 T7.5/T7.2:AI 协作面(预演在副本上 dry-run,对真工程零写入)
        // / 会话报告 / 插件 manifest 校验
        (
            "preview_plan",
            json!({"root": root_s, "plan": [
                {"id": "vol", "tool": "clip_update", "args": {"clipId": "V1-001", "patch": {"volume": 0.5}}},
                {"id": "bad", "tool": "clip_split", "args": {"clipId": "V1-001", "tMs": 999999}}
            ]}),
        ),
        ("session_report", json!({"root": root_s})),
        (
            "plugin_validate",
            json!({"manifest": {"id": "demo-clip", "name": "示例", "version": "1.0.0",
            "form": "process", "entry": "p.py", "permissions": {"read": true}}}),
        ),
    ];
    for (name, args) in queries {
        let resp = cutforge_mcp::dispatch(name, &args);
        assert_envelope(&resp, name);
        assert_eq!(
            resp["code"],
            json!("OK"),
            "{name} 对合法工程必须 OK: {resp}"
        );
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
        (
            "transition_set",
            json!({"root": root_s, "clipId": "V1-002"}),
        ),
        ("motion_set", json!({"root": root_s})),
        ("motion_set", json!({"root": root_s, "clipId": "V1-002"})),
        ("bgm_set", json!({"root": root_s})),
        // T2.4:render_frame 缺 atMs 同样 PRECONDITION_FAILED(缺 root 由统一探针覆盖)
        ("render_frame", json!({"root": root_s})),
        // I1-M2:区间预渲缺参/非法区间同样 PRECONDITION_FAILED
        ("preview_zone_render", json!({"root": root_s})),
        ("preview_zone_render", json!({"root": root_s, "startMs": 0})),
        (
            "preview_zone_render",
            json!({"root": root_s, "startMs": 2000, "endMs": 2000}),
        ),
        (
            "preview_zone_render",
            json!({"root": root_s, "startMs": 3000, "endMs": 1000}),
        ),
        // 册四 A4 T4.2 时间线编辑全工具缺参/非法参数面
        ("clip_trim", json!({"root": root_s})),
        (
            "clip_trim",
            json!({"root": root_s, "clipId": "V1-001", "mode": "prune", "deltaMs": 100}),
        ),
        (
            "clip_trim",
            json!({"root": root_s, "clipId": "V1-001", "mode": "trim", "deltaMs": 100}),
        ),
        ("clip_split_all", json!({"root": root_s})),
        ("track_update", json!({"root": root_s})),
        (
            "track_update",
            json!({"root": root_s, "trackId": "V1", "patch": {}}),
        ),
        ("clip_gap_delete", json!({"root": root_s})),
        ("clip_copy", json!({"root": root_s})),
        ("clip_paste_at", json!({"root": root_s})),
        ("clip_paste_at", json!({"root": root_s, "trackId": "V1"})),
        // 册五 A5-BE3:专业编辑工具缺参面
        ("compound_create", json!({"root": root_s})),
        (
            "compound_create",
            json!({"root": root_s, "clipIds": ["V1-001"]}),
        ),
        ("compound_unbind", json!({"root": root_s})),
        ("multicam_cut", json!({"root": root_s})),
        ("scene_detect", json!({"root": root_s})),
        // 册六 A6 T6.1:布局迁移/工程库缺参面(migrate_layout 缺 to;library_manage 缺 action)
        ("migrate_layout", json!({"root": root_s})),
        ("library_manage", json!({"root": root_s})),
        ("library_manage", json!({"root": root_s, "action": "new"})),
        (
            "library_recover",
            json!({"root": root_s, "action": "recover"}),
        ),
        // 册六 A6 T6.3/T6.2:导出矩阵与素材导入缺参/非法参数面
        (
            "export_all_variants",
            json!({"root": root_s, "action": "status"}),
        ),
        (
            "export_all_variants",
            json!({"root": root_s, "ratios": ["21x9"]}),
        ),
        ("media_import", json!({"root": root_s})),
        (
            "media_import",
            json!({"root": root_s, "src": "无此素材.mp4"}),
        ),
        ("media_library", json!({"root": root_s, "action": "tag"})),
        // 册七 A7 T7.5/T7.2:AI 协作面与插件面缺参
        ("preview_plan", json!({"root": root_s})),
        ("preview_plan", json!({"root": root_s, "plan": []})),
        (
            "preview_plan",
            json!({"root": root_s, "plan": [{"tool": "render"}]}),
        ),
        ("apply_plan", json!({"root": root_s})),
        (
            "apply_plan",
            json!({"root": root_s, "plan": [{"tool": "clip_update"}]}),
        ),
        ("note_reply", json!({"root": root_s})),
        (
            "note_reply",
            json!({"root": root_s, "noteId": "n-9999", "body": "x"}),
        ),
        ("session_report", json!({})),
        ("plugin_validate", json!({"root": root_s})),
    ] {
        let resp = cutforge_mcp::dispatch(name, &args);
        assert_envelope(&resp, name);
        assert_eq!(
            resp["code"],
            json!("PRECONDITION_FAILED"),
            "{name} 缺参: {resp}"
        );
    }

    // E3-2:media_probe 对不存在的路径 → PRECONDITION_FAILED(协议完整;有 ffprobe
    // 的机器同样在此路径返回,不依赖环境)
    let resp = cutforge_mcp::dispatch(
        "media_probe",
        &json!({"root": root_s, "src": "无此文件.mp4"}),
    );
    assert_envelope(&resp, "media_probe");
    assert_eq!(
        resp["code"],
        json!("PRECONDITION_FAILED"),
        "media_probe 缺文件: {resp}"
    );

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
    //  scope_data/audio_loudness/encode_probe/render_queue → 61;册五 A5-BE3 增
    //  compound_create/compound_unbind/multicam_cut/scene_detect(写)+
    //  multicam_sync/otio_export/otio_import(编排)→ 68;册六 A6 T6.1 增
    //  migrate_layout/library_manage/library_recover(写)+ library_list(查询)→ 72;
    //  T6.3/T6.2 增 export_preflight/media_library(查询)+ media_import(写)+
    //  export_all_variants(编排)→ 76;册七 A7 T7.6 增 project_package/
    //  project_unpackage(写,.cfpkg 打包/解包)→ 78;T7.5/T7.2 增
    //  preview_plan/session_report/plugin_validate(查询)+ apply_plan/note_reply
    //  (写,AI 协作面与插件面)→ 83;I1 增 preview_zone_render(区间半分辨率
    //  预渲,编排)→ 84;V2-W1 增 media_thumbs(批量缩略图,编排)→ 85)
    let names = cutforge_mcp::tool_names();
    assert_eq!(
        names.len(),
        85,
        "B7 口径:工具数以 schemas/mcp-tools.json 为准(V2-W1 增 media_thumbs 批量缩略图)"
    );
    for t in cutforge_mcp::registry() {
        assert!(t["name"].is_string() && t["description"].is_string());
        assert!(t["inputSchema"].is_object(), "{} 缺 inputSchema", t["name"]);
        assert!(
            t["outputSchema"].is_object(),
            "{} 缺 outputSchema",
            t["name"]
        );
    }
    // kind 口径:21 查询 + 42 写 + 22 编排(与 _doc 同句;V2-W1 增 media_thumbs 84→85)
    let mut kinds = std::collections::BTreeMap::new();
    for t in cutforge_mcp::registry() {
        *kinds
            .entry(t["kind"].as_str().unwrap().to_string())
            .or_insert(0usize) += 1;
    }
    assert_eq!(kinds.get("query"), Some(&21), "查询 21:{kinds:?}");
    assert_eq!(kinds.get("write"), Some(&42), "写 42:{kinds:?}");
    assert_eq!(kinds.get("orchestrate"), Some(&22), "编排 22:{kinds:?}");

    // M4-1 单注册表双通道:注册表与 dispatch **逐一相等**——每个注册工具都必须有
    // 实现分支,不得出现"已注册但未实现"。统一以缺 root 空参探针:所有工具(capability_matrix
    // 除外)在 {} 下必须返回协议完整的 PRECONDITION_FAILED;"未实现"分支返回 INTERNAL,在此红。
    for t in cutforge_mcp::registry() {
        let name = t["name"].as_str().unwrap();
        let resp = cutforge_mcp::dispatch(name, &json!({}));
        assert_envelope(&resp, name);
        let msg = resp["message"].as_str().unwrap_or_default();
        assert!(
            !msg.contains("未实现"),
            "{name} 已注册但 dispatch 无实现分支: {resp}"
        );
        let expected = if name == "capability_matrix" {
            "OK"
        } else {
            "PRECONDITION_FAILED"
        };
        assert_eq!(
            resp["code"],
            json!(expected),
            "{name} 空参探针口径漂移: {resp}"
        );
    }
    cutforge_io::fsutil::cleanup(&root);
}

/// E6-1/B11:project_new 走 dispatch 即可从零建工程;随后 project_get/timeline_get
/// 必须可用(从零剪闭环的第一跳)。已存在 → PRECONDITION_FAILED(拒绝覆盖)。
#[test]
fn project_new_from_zero_and_query() {
    let root = cutforge_io::fsutil::temp_dir("mcp-project-new");
    let root_s = root.to_string_lossy().to_string();
    let resp = cutforge_mcp::dispatch(
        "project_new",
        &serde_json::json!({
            "root": root_s, "slug": "从零", "fps": 30, "canvasW": 1080, "canvasH": 1920,
        }),
    );
    assert_eq!(resp["code"], json!("OK"), "{resp}");
    for name in ["project_get", "timeline_get"] {
        let r = cutforge_mcp::dispatch(name, &json!({"root": root_s}));
        assert_eq!(r["code"], json!("OK"), "{name} 对新工程必须 OK: {r}");
    }
    let dup = cutforge_mcp::dispatch("project_new", &json!({"root": root_s}));
    assert_eq!(
        dup["code"],
        json!("PRECONDITION_FAILED"),
        "重复创建必须拒绝: {dup}"
    );
    cutforge_io::fsutil::cleanup(&root);
}

/// 册五 A5-BE3(T5.4/T5.5):专业编辑与互操作工具的 dispatch 级闭环——
/// 复合打包/解包(单 Op + undo)、clip_update patch.compound 拒绝 null、
/// multicam_cut 切换序列展开、otio_export/otio_import 往返(工程投影等价)、
/// EDL 头注释。纯计算类(multicam_sync/scene_detect)需 ffmpeg,由渲染端
/// parity 与 pro_ops 单测覆盖,此处只验证缺参协议面(上方写工具缺参探针)。
#[test]
fn pro_ops_tools_full_chain() {
    let root = cutforge_io::fsutil::temp_dir("mcp-pro-ops");
    let root_s = root.to_string_lossy().to_string();
    let call = |name: &str, args: Value| cutforge_mcp::dispatch(name, &args);
    // 从零建工程:V1 两段相邻片段
    let r = call(
        "project_new",
        json!({"root": root_s, "slug": "pro-ops", "fps": 30,
                 "canvasW": 320, "canvasH": 240, "tracks": ["video"]}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    std::fs::write(root.join("a.mp4"), b"fake").unwrap();
    std::fs::write(root.join("b.mp4"), b"fake").unwrap();
    for (i, start) in [0i64, 2000].iter().enumerate() {
        let r = call(
            "clip_add",
            json!({"root": root_s, "trackId": "V1",
                     "src": "a.mp4", "startMs": start, "durationMs": 2000,
                     "requestId": format!("pro-add-{i}")}),
        );
        assert_eq!(r["code"], json!("OK"), "{r}");
    }
    // compound_create:两片段打包(单 Op)
    let r = call(
        "compound_create",
        json!({"root": root_s, "clipIds": ["V1-001", "V1-002"],
                  "toTrack": "V1", "startMs": 0}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert_eq!(r["data"]["opIds"].as_array().unwrap().len(), 1, "打包单 Op");
    assert_eq!(r["data"]["innerClips"], json!(2));
    let shell_id = r["data"]["clipId"].as_str().unwrap().to_string();
    // 投影:compound 概要(clipCount/durationMs)
    let r = call("timeline_get", json!({"root": root_s}));
    let shell = r["data"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == json!(shell_id))
        .unwrap();
    assert_eq!(shell["compound"]["clipCount"], json!(2));
    assert_eq!(shell["compound"]["durationMs"], json!(4000));
    // compound_unbind:还原两片段(单 Op;undo 回复合态)
    let r = call(
        "compound_unbind",
        json!({"root": root_s, "clipId": shell_id.clone()}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert_eq!(r["data"]["opIds"].as_array().unwrap().len(), 1, "解包单 Op");
    assert_eq!(r["data"]["unbound"], json!(2));
    let r = call("undo", json!({"root": root_s}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    // undo 本身也是一笔 Op(rev 前进,审计链);断言**状态**回归复合态
    let rows = call("timeline_get", json!({"root": root_s}))["data"]["clips"].clone();
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .any(|c| c["compound"].is_object() && c["compound"]["clipCount"] == json!(2)),
        "undo 后复合壳回归: {rows}"
    );
    // clip_update patch.compound:null 显式拒绝(摘除走 compound_unbind)
    let r = call(
        "clip_update",
        json!({"root": root_s, "clipId": shell_id, "patch": {"compound": null}}),
    );
    assert_eq!(r["code"], json!("SCHEMA_INVALID"), "{r}");
    // multicam_cut:切换序列展开(单 Op)
    let r = call(
        "multicam_cut",
        json!({"root": root_s, "trackId": "V1",
                  "startMs": 20000, "durationMs": 3000,
                  "angles": [{"src": "a.mp4", "offsetMs": 0}, {"src": "b.mp4", "offsetMs": 500}],
                  "switches": [{"tMs": 0, "angle": 0}, {"tMs": 1500, "angle": 1}],
                  "requestId": "pro-mc-1"}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert_eq!(
        r["data"]["opIds"].as_array().unwrap().len(),
        1,
        "多机位序列单 Op"
    );
    assert_eq!(r["data"]["segments"], json!(2));
    // 守卫:switches 首点非 0 → PRECONDITION_FAILED
    let r = call(
        "multicam_cut",
        json!({"root": root_s, "trackId": "V1",
                  "startMs": 30000, "durationMs": 1000,
                  "angles": [{"src": "a.mp4"}], "switches": [{"tMs": 100, "angle": 0}]}),
    );
    assert_eq!(r["code"], json!("PRECONDITION_FAILED"), "{r}");
    // otio_export(只读派生物)→ otio_import(新工程)→ 工程投影等价
    let r = call("otio_export", json!({"root": root_s, "format": "otio"}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    let otio_out = root.join(r["data"]["out"].as_str().unwrap());
    assert!(otio_out.is_file(), "OTIO 产物必须落盘");
    let dest = cutforge_io::fsutil::temp_dir("mcp-pro-ops-import");
    let r = call(
        "otio_import",
        json!({"root": dest.to_string_lossy(), "src": otio_out.to_string_lossy()}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    let a = call("timeline_get", json!({"root": root_s}))["data"]["clips"].clone();
    let b = call("timeline_get", json!({"root": dest.to_string_lossy()}))["data"]["clips"].clone();
    type Row = (Option<String>, u64, u64, Option<u64>, bool);
    let norm = |rows: &Value| -> Vec<Row> {
        rows.as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["src"].as_str().map(String::from),
                    c["startMs"].as_u64().unwrap_or(0),
                    c["durationMs"].as_u64().unwrap_or(0),
                    // OTIO 侧 source_range.start 恒显式(缺省 0 ≡ 缺席)
                    Some(c["sourceInMs"].as_u64().unwrap_or(0)),
                    c["compound"].is_object(),
                )
            })
            .collect()
    };
    assert_eq!(norm(&a), norm(&b), "OTIO 往返工程投影等价");
    // EDL 导出(头部生成器与版本)
    let r = call("otio_export", json!({"root": root_s, "format": "edl"}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    let edl = std::fs::read_to_string(root.join(r["data"]["out"].as_str().unwrap())).unwrap();
    assert!(
        edl.starts_with(
            "TITLE: pro-ops
"
        ),
        "{edl}"
    );
    assert!(edl.contains("generated by cutforge"));
    cutforge_io::fsutil::cleanup(&root);
    cutforge_io::fsutil::cleanup(&dest);
}

/// 册七 A7 T7.6:.cfpkg 打包/解包 dispatch 级闭环——v3 工程 → 素材+片段 → 打包
/// (manifest 计数/缺素材如实登记)→ 解包到新目录(v3 布局,工程照常打开、片段
/// 可见)→ 再打包语义等价;覆盖目标 CONFLICT、坏容器 SCHEMA_INVALID、缺 src
/// NO_CONFIG、缺参 PRECONDITION_FAILED。
#[test]
fn cfpkg_package_unpackage_full_chain() {
    let tmp = cutforge_io::fsutil::temp_dir("mcp-cfpkg");
    let proj = tmp.join("proj");
    let proj_s = proj.to_string_lossy().to_string();
    let call = |name: &str, args: Value| cutforge_mcp::dispatch(name, &args);
    // 从零建 v3 工程 + 一段引用素材
    let r = call(
        "project_new",
        json!({"root": proj_s, "slug": "打包链", "fps": 30,
                 "canvasW": 320, "canvasH": 240, "tracks": ["video"], "layout": "v3"}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    std::fs::create_dir_all(proj.join("media")).unwrap();
    std::fs::write(proj.join("media/a.mp4"), b"fake-video").unwrap();
    let r = call(
        "clip_add",
        json!({"root": proj_s, "trackId": "V1", "src": "media/a.mp4",
                 "startMs": 0, "durationMs": 2000, "requestId": "cfpkg-add-1"}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    let rev = call("project_get", json!({"root": proj_s}))["data"]["rev"].clone();
    // 打包:缺省 includeMedia;out 显式
    let pkg = tmp.join("链.cfpkg");
    let r = call(
        "project_package",
        json!({"root": proj_s, "out": pkg.to_string_lossy()}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert_eq!(r["data"]["name"], json!("打包链"));
    assert_eq!(r["data"]["rev"], rev, "manifest rev = 工程当前 rev");
    assert_eq!(r["data"]["sourceLayout"], json!("v3"));
    assert_eq!(r["data"]["counts"]["media"], json!(1));
    assert_eq!(r["data"]["missing"], json!([]));
    assert!(pkg.is_file(), "容器落盘");
    // 容器结构自证:manifest + project/project.json 齐备
    let items = cutforge_io::cfpkg::read_pkg(&pkg).unwrap();
    assert!(items.iter().any(|e| e.name == "manifest.json"));
    assert!(items.iter().any(|e| e.name == "project/project.json"));
    assert!(items.iter().any(|e| e.name == "media/media/a.mp4"));
    // 解包到新目录:v3 布局 + 工程照常打开 + 片段可见
    let dest = tmp.join("还原");
    let r = call(
        "project_unpackage",
        json!({"root": dest.to_string_lossy(),
                 "src": pkg.to_string_lossy()}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert_eq!(r["data"]["name"], json!("打包链"));
    assert_eq!(r["data"]["media"], json!(1));
    let r = call("project_get", json!({"root": dest.to_string_lossy()}));
    assert_eq!(r["code"], json!("OK"), "解包工程必须可打开: {r}");
    let clips =
        call("timeline_get", json!({"root": dest.to_string_lossy()}))["data"]["clips"].clone();
    assert_eq!(clips.as_array().unwrap().len(), 1, "片段随包还原:{clips}");
    assert_eq!(clips[0]["src"], json!("media/a.mp4"), "src 相对路径零改写");
    // 再打包:与首包 byte 语义等价(manifest 仅 createdAt 归一)
    let pkg2 = dest.join("again.cfpkg");
    let r = call(
        "project_package",
        json!({"root": dest.to_string_lossy(), "out": pkg2.to_string_lossy()}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    let a = cutforge_io::cfpkg::semantic_entries(&cutforge_io::cfpkg::read_pkg(&pkg).unwrap());
    let b = cutforge_io::cfpkg::semantic_entries(&cutforge_io::cfpkg::read_pkg(&pkg2).unwrap());
    assert_eq!(a, b, "打包→解包→再打包语义等价");
    // 错误码面:覆盖既有非空目标 CONFLICT;坏容器 SCHEMA_INVALID;缺容器 NO_CONFIG
    let r = call(
        "project_unpackage",
        json!({"root": dest.to_string_lossy(), "src": pkg.to_string_lossy()}),
    );
    assert_eq!(r["code"], json!("CONFLICT"), "覆盖既有目录必须拒绝: {r}");
    let bad = tmp.join("bad.cfpkg");
    std::fs::write(&bad, b"not a zip").unwrap();
    let r = call(
        "project_unpackage",
        json!({"root": tmp.join("b1").to_string_lossy(), "src": bad.to_string_lossy()}),
    );
    assert_eq!(r["code"], json!("SCHEMA_INVALID"), "{r}");
    let r = call(
        "project_unpackage",
        json!({"root": tmp.join("b2").to_string_lossy(), "src": tmp.join("无.cfpkg").to_string_lossy()}),
    );
    assert_eq!(r["code"], json!("NO_CONFIG"), "{r}");
    // 打包面:缺 root 探针由统一空参覆盖;此处补:非工程目录 NO_CONFIG
    let r = call(
        "project_package",
        json!({"root": tmp.join("非工程").to_string_lossy()}),
    );
    assert_eq!(r["code"], json!("NO_CONFIG"), "{r}");
    // OpLog 随包:解包工程 rev 与源一致(撤销链保留)
    let rev2 = call("project_get", json!({"root": dest.to_string_lossy()}))["data"]["rev"].clone();
    assert_eq!(rev2, rev, "OpLog 随包,rev 保持");
    cutforge_io::fsutil::cleanup(&tmp);
}

/// E6-3/B14:只读查询不申请排他锁——project_get 后 `.cutforge/lock` 不存在
/// (写操作才经 open_exclusive 创建锁)。
#[test]
fn readonly_query_holds_no_lock() {
    let root = cutforge_io::tests_fixture("mcp-readonly-lock").unwrap();
    let root_s = root.to_string_lossy().to_string();
    let resp = cutforge_mcp::dispatch("project_get", &json!({"root": root_s}));
    assert_eq!(resp["code"], json!("OK"), "{resp}");
    assert!(
        !root.join(".cutforge/lock").exists(),
        "只读查询不得留下工程锁"
    );
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
    let r = call(
        "clip_paste_at",
        json!({"root": root_s, "trackId": "V1", "startMs": 15000}),
    );
    assert_eq!(r["code"], json!("PRECONDITION_FAILED"), "{r}");

    // 1) roll out +200:V1-001[0,8600) / V1-002[8600,14600)(srcIn 21800)
    let r = call(
        "clip_trim",
        json!({"root": root_s, "clipId": "V1-001", "mode": "roll", "edge": "out", "deltaMs": 200}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    // 2) trim out -600:V1-001[0,8000),与 V1-002 间出现 600ms 间隙
    let r = call(
        "clip_trim",
        json!({"root": root_s, "clipId": "V1-001", "mode": "trim", "edge": "out", "deltaMs": -600}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    // 3) slip -300:内容平移 srcIn 21800→21500,时间线占位不变
    //    (夹具无媒体文件,正向 slip 的素材末尾约束需 ffprobe 会 DEP_MISSING,故取负向)
    let r = call(
        "clip_trim",
        json!({"root": root_s, "clipId": "V1-002", "mode": "slip", "deltaMs": -300}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    // 4) slide -600:V1-002[8000,14000)(左侧非贴合,间隙吸收;内容窗不变)
    let r = call(
        "clip_trim",
        json!({"root": root_s, "clipId": "V1-002", "mode": "slide", "deltaMs": -600}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    // roll 无贴合邻居 → GUARD_FAILED(内核不变量经既有 Reject 码)
    let r = call(
        "clip_trim",
        json!({"root": root_s, "clipId": "V1-002", "mode": "roll", "edge": "out", "deltaMs": 100}),
    );
    assert_eq!(r["code"], json!("GUARD_FAILED"), "{r}");
    // 5) clip_split_all @4000:V1-001[0,4000)+V1-003[4000,8000),单 Op
    let r = call("clip_split_all", json!({"root": root_s, "tMs": 4000}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert_eq!(
        r["data"]["opIds"].as_array().unwrap().len(),
        1,
        "全轨分割单 Op"
    );
    // 6) clip_gap_delete(A2 头部 8400ms 空档;迁移器轨道 id=字母+下标,A1 是视频轨):A1-001 → [0,400)
    let r = call(
        "clip_gap_delete",
        json!({"root": root_s, "trackId": "A2", "tMs": 100}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    // t 位于片段内部 → GUARD_FAILED
    let r = call(
        "clip_gap_delete",
        json!({"root": root_s, "trackId": "V1", "tMs": 1000}),
    );
    assert_eq!(r["code"], json!("GUARD_FAILED"), "{r}");
    // 7) track_update:轨道属性进 IR 并回读可见(TrackPatch 按字段合并)
    let r = call(
        "track_update",
        json!({"root": root_s, "trackId": "V1",
                 "patch": {"name": "主画面A4", "mute": true, "heightPx": 260, "color": "#3366CC"}}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    let r = call(
        "track_update",
        json!({"root": root_s, "trackId": "V1", "patch": {"heightPx": 260}}),
    );
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
    let r = call(
        "clip_paste_at",
        json!({"root": root_s, "trackId": "A2", "startMs": 15000}),
    );
    assert_eq!(r["code"], json!("GUARD_FAILED"), "{r}");
    let r = call(
        "clip_paste_at",
        json!({"root": root_s, "trackId": "V1", "startMs": 15000,
                 "requestId": "conf-paste-1"}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    // 9) 投影面:全链结果逐项可见(契约链端到端)
    let r = call("timeline_get", json!({"root": root_s}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    let rows: Vec<&Value> = r["data"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["track"] == json!("V1"))
        .collect();
    assert!(
        rows.iter()
            .any(|c| c["id"] == json!("V1-003") && c["startMs"] == json!(4000))
    );
    let pasted = rows
        .iter()
        .find(|c| c["id"] == json!("V1-004"))
        .expect("粘贴片段必须在投影");
    assert_eq!(pasted["startMs"], json!(15000));
    assert_eq!(pasted["durationMs"], json!(4000), "粘贴保留时长(带属性)");
    let pv = call("project_get", json!({"root": root_s}));
    let v1 = pv["data"]["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == json!("V1"))
        .unwrap();
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
    let v1 = call("project_get", json!({"root": root_s}))["data"]["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == json!("V1"))
        .unwrap()
        .clone();
    assert!(
        v1.get("mute").is_none() && v1.get("heightPx").is_none(),
        "undo 后轨道属性字段消失"
    );
    assert_eq!(
        v1["clips"].as_array().unwrap()[0]["durationMs"],
        json!(8400)
    );
    cutforge_io::fsutil::cleanup(&root);
}

/// 册六 A6(T6.1/AC-6.1/6.2):migrate_layout / library_manage / library_list /
/// library_recover 的 dispatch 级闭环——V2→V3 迁移(幂等/冲突拒绝/工程照常读写)、
/// 工程库七操作、卡片轻量派生、强造残留锁→恢复清单→恢复清零。
#[test]
fn library_migrate_recover_full_chain() {
    let tmp = cutforge_io::fsutil::temp_dir("mcp-library");
    let lib = tmp.join("lib");
    let lib_s = lib.to_string_lossy().to_string();
    let call = |name: &str, args: Value| cutforge_mcp::dispatch(name, &args);

    // ---- 迁移:V2 工程 → v3(一次性),再跑幂等 NOOP,冲突整体拒绝 ----
    let proj = tmp.join("proj");
    let proj_s = proj.to_string_lossy().to_string();
    let r = call(
        "project_new",
        json!({"root": proj_s, "slug": "迁客体", "fps": 30, "tracks": ["video"]}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    std::fs::create_dir_all(proj.join("01_原始素材")).unwrap();
    std::fs::write(proj.join("01_原始素材/a.mp4"), b"m").unwrap();
    let r = call("migrate_layout", json!({"root": proj_s, "to": "v3"}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert_eq!(r["data"]["from"], json!("v2"));
    assert_eq!(r["data"]["idempotent"], json!(false));
    assert!(proj.join("media/a.mp4").is_file(), "素材随映射入 media/");
    assert!(proj.join("project.json").is_file(), "真相源到根");
    assert!(!proj.join("05_时间线工程").exists(), "腾空目录移除");
    // 迁移后工程照常读写(v3 三态打开)
    let r = call("timeline_get", json!({"root": proj_s}));
    assert_eq!(r["code"], json!("OK"), "v3 工程照常打开: {r}");
    // 幂等:再跑 = NOOP
    let r = call("migrate_layout", json!({"root": proj_s, "to": "v3"}));
    assert_eq!(
        r["data"]["idempotent"],
        json!(true),
        "二次迁移必须 NOOP: {r}"
    );
    // 冲突:media/ 已存在的 V2 工程拒绝(盘面不动)
    let proj2 = tmp.join("proj2");
    let proj2_s = proj2.to_string_lossy().to_string();
    let r = call(
        "project_new",
        json!({"root": proj2_s, "slug": "冲突体", "fps": 30, "tracks": ["video"]}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    std::fs::create_dir_all(proj2.join("01_原始素材")).unwrap(); // 迁移源在盘 → 计划含改名
    std::fs::create_dir_all(proj2.join("media")).unwrap(); // 映射目标已存在 → 冲突
    let r = call("migrate_layout", json!({"root": proj2_s, "to": "v3"}));
    assert_eq!(
        r["code"],
        json!("CONFLICT"),
        "映射目标已存在必须 CONFLICT: {r}"
    );
    assert!(
        proj2.join("05_时间线工程/project.json").is_file(),
        "冲突拒绝盘面不动"
    );
    // 非 v3 目标 / 非工程目录
    let r = call("migrate_layout", json!({"root": proj_s, "to": "v2"}));
    assert_eq!(r["code"], json!("PRECONDITION_FAILED"), "{r}");
    let r = call(
        "migrate_layout",
        json!({"root": tmp.join("无此工程").to_string_lossy(), "to": "v3"}),
    );
    assert_eq!(r["code"], json!("NO_CONFIG"), "{r}");

    // ---- 工程库:new/list/search/rename/copy/archive/unarchive/delete ----
    let r = call(
        "library_manage",
        json!({"root": lib_s, "action": "new", "name": "甲",
                 "slug": "工程甲", "fps": 25, "canvasW": 1920, "canvasH": 1080}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    let dup = call(
        "library_manage",
        json!({"root": lib_s, "action": "new", "name": "甲"}),
    );
    assert_eq!(
        dup["code"],
        json!("PRECONDITION_FAILED"),
        "重名必须拒绝: {dup}"
    );
    let bad = call(
        "library_manage",
        json!({"root": lib_s, "action": "new", "name": "../逃逸"}),
    );
    assert_eq!(
        bad["code"],
        json!("PRECONDITION_FAILED"),
        "非法名必须拒绝: {bad}"
    );
    let r = call(
        "library_manage",
        json!({"root": lib_s, "action": "copy", "name": "甲", "to": "乙"}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    // 卡片:轻量派生(slug/fps/画幅/时长/valid)
    let r = call("library_list", json!({"root": lib_s}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    let cards = r["data"]["projects"].as_array().unwrap();
    assert_eq!(cards.len(), 2, "{r}");
    let card = cards.iter().find(|c| c["name"] == json!("甲")).unwrap();
    assert_eq!(card["slug"], json!("工程甲"));
    assert_eq!(card["fps"], json!(25));
    assert_eq!(card["canvas"]["width"], json!(1920));
    assert_eq!(card["valid"], json!(true));
    // search 子串过滤
    let r = call("library_list", json!({"root": lib_s, "query": "乙"}));
    assert_eq!(r["data"]["total"], json!(1));
    // archive → includeArchived → unarchive
    let r = call(
        "library_manage",
        json!({"root": lib_s, "action": "archive", "name": "乙"}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    let r = call("library_list", json!({"root": lib_s}));
    assert_eq!(r["data"]["total"], json!(1), "归档不入缺省清单");
    let r = call(
        "library_list",
        json!({"root": lib_s, "includeArchived": true}),
    );
    let archived = r["data"]["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == json!("乙"))
        .unwrap()
        .clone();
    assert_eq!(archived["archived"], json!(true));
    let r = call(
        "library_manage",
        json!({"root": lib_s, "action": "unarchive", "name": "乙"}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    // rename(目录级;slug 不随改,报告如实标注)
    let r = call(
        "library_manage",
        json!({"root": lib_s, "action": "rename", "name": "乙", "to": "丙"}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    let r = call("library_list", json!({"root": lib_s, "query": "丙"}));
    assert_eq!(
        r["data"]["projects"][0]["slug"],
        json!("工程甲"),
        "slug 不随目录名改写"
    );
    // delete → .trash(可捞回,非物理删除)
    let r = call(
        "library_manage",
        json!({"root": lib_s, "action": "delete", "name": "丙"}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert!(
        r["data"]["at"]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .contains(".trash/"),
        "{r}"
    );

    // ---- 崩溃恢复:强造残留锁(假死 pid)→ 恢复清单 → 恢复清零 ----
    let p = lib.join("崩溃体");
    let r = call(
        "library_manage",
        json!({"root": lib_s, "action": "new", "name": "崩溃体"}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    std::fs::create_dir_all(p.join(".cutforge")).unwrap();
    // R-02 同源判据:锁龄 ≥30s(ts 拨旧)+ pid 死 + 心跳过期(mtime 拨旧)。
    // 死 pid 必须两平台都"不存在":pid=1 在 Linux 是 init/systemd(恒活),
    // /proc/1 存在 → detect_stale 判"持锁进程还在" → 不入恢复清单(Linux CI 红根因)。
    let dead = cutforge_io::probe::definitely_dead_pid();
    let stale_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .saturating_sub(120_000);
    std::fs::write(
        p.join(".cutforge/lock"),
        format!("pid={dead} boot= ts={stale_ts}").as_bytes(),
    )
    .unwrap();
    {
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(p.join(".cutforge/lock"))
            .unwrap();
        f.set_modified(
            std::time::SystemTime::now()
                .checked_sub(std::time::Duration::from_millis(120_000))
                .unwrap(),
        )
        .unwrap();
    }
    let r = call("library_recover", json!({"root": lib_s, "action": "list"}));
    assert_eq!(r["code"], json!("OK"), "{r}");
    let stale = r["data"]["stale"].as_array().unwrap();
    assert_eq!(stale.len(), 1, "{r}");
    assert_eq!(stale[0]["pidAlive"], json!(false), "假 pid 必判死");
    // 执行恢复:清锁 + OpLog 一致性校验
    let r = call(
        "library_recover",
        json!({"root": lib_s, "action": "recover", "name": "崩溃体"}),
    );
    assert_eq!(r["code"], json!("OK"), "{r}");
    assert_eq!(r["data"]["lockCleared"], json!(true));
    assert_eq!(r["data"]["rev"], json!(0));
    assert!(!p.join(".cutforge/lock").exists(), "残留锁必须被清");
    let r = call("library_recover", json!({"root": lib_s, "action": "list"}));
    assert_eq!(r["data"]["total"], json!(0), "恢复后清单归零");

    cutforge_io::fsutil::cleanup(&tmp);
}
