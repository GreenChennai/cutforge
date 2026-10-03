//! 册七 T7.5/T7.2 门禁(dispatch 级闭环):preview_plan 的 dry-run 副本隔离证明
//! (真工程盘面逐字节未动 + 依赖冲突按序暴露)、apply_plan 的无越权写入负例
//! (default-deny,未批准项绝不落地)、note_reply 线程化、session_report 双形态。

use serde_json::json;
use std::path::Path;

/// 工程盘面指纹(递归:rel 路径 + 内容字节 → 排序后拼接)。
/// 隔离证明的判据:预演前后指纹逐字节一致 = 零落盘。
fn dir_fingerprint(root: &Path) -> String {
    let mut rows: Vec<String> = Vec::new();
    fn walk(root: &Path, dir: &Path, rows: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, rows);
            } else if let Ok(bytes) = std::fs::read(&p) {
                // 内容指纹(FNV-1a 64);路径以正斜杠归一
                let mut h: u64 = 0xcbf29ce484222325;
                for b in &bytes {
                    h ^= *b as u64;
                    h = h.wrapping_mul(0x100000001b3);
                }
                let rel = p
                    .strip_prefix(root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                rows.push(format!("{rel}:{:016x}:{}", h, bytes.len()));
            }
        }
    }
    walk(root, root, &mut rows);
    rows.sort();
    rows.join("\n")
}

fn residual_scratch(root: &Path) -> Vec<String> {
    let parent = root.parent().unwrap_or(root);
    std::fs::read_dir(parent)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.starts_with(".cf-scratch-"))
                .collect()
        })
        .unwrap_or_default()
}

/// T7.5-1:预演 = 副本上的真实引擎逐项求值。
/// - 第 2 项在第 1 项之后必然失败(SplitOutside):证明逐项共享同一副本状态(依赖冲突按序暴露);
/// - 第 3 项照常求值:失败项不阻断后续(逐项独立回执);
/// - 真工程盘面指纹不变 + 无 scratch 残留:dry-run 隔离证明。
#[test]
fn preview_plan_dry_run_isolated_and_sequential() {
    let root = cutforge_io::tests_fixture("mcp-plan-preview").unwrap();
    let root_s = root.to_string_lossy().to_string();
    let before = dir_fingerprint(&root);

    let resp = cutforge_mcp::dispatch(
        "preview_plan",
        &json!({
            "root": root_s,
            "plan": [
                {"id": "cut1", "tool": "clip_split", "args": {"clipId": "V1-001", "tMs": 4000}},
                {"id": "cut2", "tool": "clip_split", "args": {"clipId": "V1-001", "tMs": 5000}},
                {"id": "vol", "tool": "clip_update", "args": {"clipId": "V1-001", "patch": {"volume": 0.5}}}
            ]
        }),
    );
    assert_eq!(resp["code"], json!("OK"), "预演本身必须 OK: {resp}");
    let items = resp["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    // 逐项回执:ok / 错误码 / rev 推进
    assert_eq!(items[0]["ok"], json!(true), "{}", items[0]);
    assert_eq!(items[0]["code"], json!("OK"));
    assert!(
        !items[0]["changes"].as_array().unwrap().is_empty(),
        "成功项必须带变更摘要"
    );
    assert_eq!(
        items[0]["changes"][0]["summary"],
        json!("clip_split V1-001@4000")
    );
    // 依赖冲突:V1-001 已被切成 [0..4000)+[4000..8400),5000 不在 V1-001 内部 → 拒
    // (若不是逐项共享副本状态而是各自独立求值,5000 落在 [0..8400) 内会"成功")
    assert_eq!(items[1]["ok"], json!(false), "{}", items[1]);
    assert_eq!(items[1]["id"], json!("cut2"));
    // 失败不阻断:第 3 项照常求值成功(副本状态仍在推进)
    assert_eq!(items[2]["ok"], json!(true), "{}", items[2]);
    // rev 链:0 → 1(split)→ 1(失败不升 rev)→ 2(volume)
    assert_eq!(items[0]["revBefore"], json!(0));
    assert_eq!(items[0]["revAfter"], json!(1));
    assert_eq!(items[1]["revBefore"], json!(1));
    assert_eq!(items[1]["revAfter"], json!(1), "失败项不得推进副本 rev");
    assert_eq!(items[2]["revAfter"], json!(2));
    assert_eq!(
        resp["data"]["summary"],
        json!({"total": 3, "ok": 2, "err": 1})
    );

    // 隔离证明 1:真工程盘面逐字节未动(不落盘不产 Op)
    assert_eq!(
        dir_fingerprint(&root),
        before,
        "preview_plan 不得改真工程任何文件"
    );
    // 隔离证明 2:真工程语义未动(rev=0;V1-001 未被切)
    let tl = cutforge_mcp::dispatch("timeline_get", &json!({"root": root_s}));
    assert_eq!(tl["data"]["rev"], json!(0));
    let clips = tl["data"]["clips"].as_array().unwrap();
    assert!(
        clips
            .iter()
            .any(|c| c["id"] == json!("V1-001") && c["startMs"] == json!(0))
    );
    // 隔离证明 3:副本用后即焚(无 .cf-scratch-* 残留)
    assert!(
        residual_scratch(&root).is_empty(),
        "scratch 残留: {:?}",
        residual_scratch(&root)
    );

    cutforge_io::fsutil::cleanup(&root);
}

/// T7.5-2:apply_plan 无越权写入——
/// 负例 A(缺省全拒):无 approvals → 全部跳过,零 Op 零 rev;
/// 负例 B(显式 reject 恒胜出):reject 与 approve 同指一项 → 拒绝胜出;
/// 正例:逐项批准走原 Op 通道,causedBy 链关联 planId,actor 保持调用方。
#[test]
fn apply_plan_default_deny_and_approval_chain() {
    let root = cutforge_io::tests_fixture("mcp-plan-apply").unwrap();
    let root_s = root.to_string_lossy().to_string();
    let plan = json!([
        {"id": "vol", "tool": "clip_update", "args": {"clipId": "V1-001", "patch": {"volume": 0.5}}},
        {"id": "cut", "tool": "clip_split", "args": {"clipId": "V1-001", "tMs": 4000}},
        {"id": "aud", "tool": "track_add", "args": {"kind": "audio"}}
    ]);

    // 负例 A:空批准面(default-deny)→ 全跳过,工程零变化;整体缺 approvals = 缺参拒
    let miss = cutforge_mcp::dispatch("apply_plan", &json!({"root": root_s, "plan": plan}));
    assert_eq!(
        miss["code"],
        json!("PRECONDITION_FAILED"),
        "缺 approvals 必须缺参拒: {miss}"
    );
    let resp = cutforge_mcp::dispatch(
        "apply_plan",
        &json!({"root": root_s, "plan": plan, "approvals": {}}),
    );
    assert_eq!(resp["code"], json!("OK"), "{resp}");
    assert_eq!(resp["data"]["applied"], json!(0));
    assert_eq!(resp["data"]["skipped"], json!(3));
    assert_eq!(resp["data"]["revTo"], json!(0), "未批准项绝不落地");
    let items = resp["data"]["items"].as_array().unwrap();
    assert!(
        items
            .iter()
            .all(|i| i["skipped"] == json!(true) && i["reason"] == json!("not_approved"))
    );
    let tl = cutforge_mcp::dispatch("timeline_get", &json!({"root": root_s}));
    assert_eq!(tl["data"]["rev"], json!(0));
    let vol = cutforge_mcp::dispatch("project_get", &json!({"root": root_s}));
    assert_eq!(
        vol["data"]["project"]["tracks"][0]["clips"][0]["volume"],
        json!(1.0),
        "volume 不得被未批准项改动"
    );

    // 负例 B:reject 恒胜出(同指一项)→ 该项不落地;只落其余批准项
    let resp = cutforge_mcp::dispatch(
        "apply_plan",
        &json!({
            "root": root_s, "plan": plan, "planId": "plan-test-1",
            "approvals": {"approve": [0, 1, 2], "reject": [1]}
        }),
    );
    assert_eq!(resp["code"], json!("OK"), "{resp}");
    assert_eq!(resp["data"]["planId"], json!("plan-test-1"));
    assert_eq!(resp["data"]["applied"], json!(2));
    assert_eq!(resp["data"]["rejected"], json!(1));
    let items = resp["data"]["items"].as_array().unwrap();
    assert_eq!(items[1]["reason"], json!("rejected_by_caller"));
    assert_eq!(items[1]["skipped"], json!(true));
    // 落地面核查:volume/track_add 生效,split 未发生
    let proj = cutforge_mcp::dispatch("project_get", &json!({"root": root_s}));
    let tracks = proj["data"]["project"]["tracks"].as_array().unwrap();
    assert_eq!(proj["data"]["rev"], json!(2), "恰好两个批准项各产一 Op");
    assert_eq!(tracks[0]["clips"][0]["volume"], json!(0.5));
    assert_eq!(
        tracks[0]["clips"].as_array().unwrap().len(),
        2,
        "V1 未被切(夹具原 2 片段)"
    );
    assert_eq!(tracks.len(), 3, "track_add 落地(夹具原 2 轨 + 新 audio)");
    // causedBy 链:批准项的 Op 必须绑定 planId
    let tail = cutforge_mcp::dispatch("oplog_tail", &json!({"root": root_s, "limit": 10}));
    for op in tail["data"]["ops"].as_array().unwrap() {
        assert_eq!(
            op["caused_by"],
            json!(["plan-test-1"]),
            "causedBy 必须关联 planId: {op}"
        );
    }
    // 按 id 批准同样可用
    let resp = cutforge_mcp::dispatch(
        "apply_plan",
        &json!({
            "root": root_s, "plan": plan,
            "approvals": {"approve": ["cut"]}
        }),
    );
    assert_eq!(resp["data"]["applied"], json!(1), "{resp}");
    let proj = cutforge_mcp::dispatch("project_get", &json!({"root": root_s}));
    assert_eq!(
        proj["data"]["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        3,
        "按 id 批准的 split 落地(2→3)"
    );

    cutforge_io::fsutil::cleanup(&root);
}

/// T7.5-3/T7.5-4:note_reply 线程化(state 不变、多轮追加、未知标注拒)+
/// session_report 双形态(操作分布/改动段/回执率/markdown 人话)。
#[test]
fn note_thread_and_session_report() {
    let root = cutforge_io::tests_fixture("mcp-plan-note").unwrap();
    let root_s = root.to_string_lossy().to_string();
    let added = cutforge_mcp::dispatch(
        "notes_add",
        &json!({
            "root": root_s, "author": "user", "body": "这里语速太快",
            "anchor": {"kind": "clip", "ref": "V1-001", "tMs": 1000}
        }),
    );
    assert_eq!(added["code"], json!("OK"), "{added}");
    let note_id = added["data"]["noteId"].as_str().unwrap().to_string();

    // 两轮追加(author 不同);线程 id = 标注 id
    for (author, body) in [("agent", "建议 1.15x,要我改吗?"), ("user", "好,改吧")] {
        let r = cutforge_mcp::dispatch(
            "note_reply",
            &json!({
                "root": root_s, "noteId": note_id, "author": author, "body": body
            }),
        );
        assert_eq!(r["code"], json!("OK"), "{r}");
    }
    let rep2 = cutforge_mcp::dispatch(
        "note_reply",
        &json!({
            "root": root_s, "noteId": note_id, "body": "  "
        }),
    );
    assert_eq!(rep2["code"], json!("PRECONDITION_FAILED"), "空回复拒绝");
    let rep3 = cutforge_mcp::dispatch(
        "note_reply",
        &json!({
            "root": root_s, "noteId": "n-9999", "body": "x"
        }),
    );
    assert_eq!(rep3["code"], json!("PRECONDITION_FAILED"), "未知标注拒绝");

    let list = cutforge_mcp::dispatch("notes_list", &json!({"root": root_s}));
    let note = list["data"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == json!(note_id))
        .unwrap();
    let thread = note["thread"].as_array().unwrap();
    assert_eq!(thread.len(), 2);
    assert_eq!(thread[0]["author"], json!("agent"));
    assert_eq!(thread[1]["body"], json!("好,改吧"));
    assert_eq!(note["state"], json!("open"), "回复不得改状态");

    // 结案(带 opIds)→ 回执率可统计
    let upd = cutforge_mcp::dispatch(
        "clip_update",
        &json!({
            "root": root_s, "clipId": "V1-001", "patch": {"speed": 1.15},
            "causedBy": [note_id]
        }),
    );
    let op_ids = upd["data"]["opIds"].clone();
    let res = cutforge_mcp::dispatch(
        "notes_resolve",
        &json!({
            "root": root_s, "noteId": note_id, "reply": "已提速 1.15x", "opIds": op_ids
        }),
    );
    assert_eq!(res["code"], json!("OK"), "{res}");

    let rep = cutforge_mcp::dispatch("session_report", &json!({"root": root_s, "sinceRev": 0}));
    assert_eq!(rep["code"], json!("OK"), "{rep}");
    assert!(
        rep["data"]["opCount"].as_u64().unwrap() >= 3,
        "notes_add + 2 回复 + update + 结案"
    );
    assert!(
        rep["data"]["byKind"].get("set").is_some(),
        "操作分布含 set: {}",
        rep["data"]["byKind"]
    );
    assert!(
        rep["data"]["touchedClips"].get("V1-001").is_some(),
        "改动段含 V1-001"
    );
    assert!(
        rep["data"]["notes"]["resolved"].as_u64().unwrap() >= 1,
        "含本会话结案(夹具预含 resolved n-0002)"
    );
    assert_eq!(
        rep["data"]["notes"]["receiptRate"],
        json!(1.0),
        "结案绑定 opIds → 回执率 100%"
    );
    assert!(rep["data"]["notes"]["replies"].as_u64().unwrap() >= 2);
    let md = rep["data"]["markdown"].as_str().unwrap();
    assert!(
        md.contains("会话改动报告") && md.contains("V1-001"),
        "markdown 人话形态: {md}"
    );

    cutforge_io::fsutil::cleanup(&root);
}

/// T7.2-10 服务端裁决面(dispatch 级):插件 manifest 经 authorize 后以
/// actor=plugin 走原单表——写 Op 的 actor 如实落 plugin(与 oplog_tail 查询对账)。
#[test]
fn plugin_channel_end_to_end() {
    let root = cutforge_io::tests_fixture("mcp-plugin-channel").unwrap();
    let root_s = root.to_string_lossy().to_string();
    let manifest = json!({
        "id": "demo-clip", "name": "示例插件", "version": "1.0.0",
        "form": "process", "entry": "plugin.py",
        "permissions": {"read": true, "write": true}
    });
    // 校验 + 裁决 + actor=plugin 写入(与 CLI plugin-call 同一函数面)
    assert!(cutforge_mcp::validate_manifest(&manifest).is_empty());
    cutforge_mcp::authorize(&manifest, "clip_update").expect("已声明写权限必须放行");
    let args = json!({"root": root_s, "clipId": "V1-001", "patch": {"volume": 0.8}});
    let resp = cutforge_mcp::dispatch_with_actor(
        "clip_update",
        &args,
        cutforge_mcp::plugin_actor(&manifest),
    );
    assert_eq!(resp["code"], json!("OK"), "{resp}");
    // OpLog 归因:actor.kind = plugin
    let tail = cutforge_mcp::dispatch("oplog_tail", &json!({"root": root_s, "actor": "plugin"}));
    let ops = tail["data"]["ops"].as_array().unwrap();
    assert_eq!(ops.len(), 1, "{tail}");
    assert_eq!(ops[0]["actor"]["kind"], json!("plugin"));
    assert_eq!(ops[0]["actor"]["id"], json!("demo-clip"));
    // 越权:只读插件调写工具 → GUARD_FAILED(FORBIDDEN 语义),工程零变化
    let ro = json!({"id": "ro-plugin", "permissions": {"read": true}});
    let err = cutforge_mcp::authorize(&ro, "clip_update").unwrap_err();
    assert_eq!(err.0, "GUARD_FAILED");
    assert!(err.1.starts_with("FORBIDDEN:"));
    cutforge_io::fsutil::cleanup(&root);
}
