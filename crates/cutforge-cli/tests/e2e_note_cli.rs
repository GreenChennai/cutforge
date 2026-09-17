//! M3-8 门禁:标注全链路(CLI 级)。
//! 创建标注 → AI 执行改动(causedBy 关联)→ 从 OpLog 读到关联改动 → 结案回执;
//! 全链路可复现且幂等,耗时 ≤ 3 s。

use cutforge_core::engine::{Answer, Query};
use cutforge_core::notes::NoteState;
use cutforge_core::oplog::ActorKind;
use cutforge_io::Workspace;
use std::time::Instant;

#[test]
fn e2e_note_cli() {
    let t0 = Instant::now();
    let root = cutforge_io::tests_fixture("e2e-note").unwrap();
    let root_s = root.to_string_lossy().to_string();

    // 1. 用户在时间轴上打标注:"这里要怎么改"
    let code = cutforge_cli::run(vec![
        "notes-add".into(),
        root_s.clone(),
        "--kind".into(), "clip".into(),
        "--ref".into(), "V1-001".into(),
        "--t-ms".into(), "4000".into(),
        "--body".into(), "这里删短一点".into(),
        "--author".into(), "user".into(),
        "--json".into(),
    ]);
    assert_eq!(code, 0, "notes-add 退出码必须为 0");

    // 2. 落盘验证:夹具自带 2 条 + 新建 1 条;新标注为 open 态
    let note_id = {
        let ws = Workspace::open(&root).unwrap();
        assert_eq!(ws.notes().notes().len(), 3, "夹具 2 条 + 新建 1 条");
        let new_note = ws.notes().find("n-0003").expect("新标注应为 n-0003");
        assert_eq!(new_note.state, NoteState::Open);
        new_note.id.clone()
    };

    // 3. AI 执行改动,causedBy 指向该标注
    let code = cutforge_cli::run(vec![
        "clip-update".into(),
        root_s.clone(),
        "V1-001".into(),
        "--duration-ms".into(), "8000".into(),
        "--caused-by".into(), note_id.clone(),
        "--actor".into(), "agent:claude".into(),
        "--summary".into(), "按标注删短".into(),
        "--json".into(),
    ]);
    assert_eq!(code, 0, "clip-update 退出码必须为 0");

    // 4. 从 OpLog 读到关联改动(AI 感知链)
    let op_id = {
        let ws = Workspace::open(&root).unwrap();
        match ws.engine().query(Query::OpLogTail { since_rev: None, actor_kind: Some(ActorKind::Agent) }) {
            Answer::Ops(ops) => {
                let linked = ops.iter().find(|o| {
                    o.caused_by.as_ref().is_some_and(|c| c.iter().any(|x| x == &note_id))
                });
                let op = linked.expect("OpLog 必须含 causedBy 关联的改动");
                assert_eq!(op.summary, "按标注删短");
                op.op_id.clone()
            }
            _ => panic!("OpLogTail 必须返回 Ops"),
        }
    };

    // 5. AI 回执结案(reply + opIds)
    let code = cutforge_cli::run(vec![
        "notes-resolve".into(),
        root_s.clone(),
        note_id.clone(),
        "--reply".into(), "已把 V1-001 删短到 8s".into(),
        "--op-ids".into(), op_id.clone(),
        "--actor".into(), "agent:claude".into(),
        "--json".into(),
    ]);
    assert_eq!(code, 0, "notes-resolve 退出码必须为 0");

    // 6. 端到端验证:resolved 状态 + opIds 绑定可回看
    let ws = Workspace::open(&root).unwrap();
    let note = ws.notes().find(&note_id).unwrap();
    assert_eq!(note.state, NoteState::Resolved);
    let rb = note.resolved_by.as_ref().expect("结案必须有回执");
    assert_eq!(rb.reply, "已把 V1-001 删短到 8s");
    assert!(rb.op_ids.contains(&op_id), "回执必须绑定改动 Op");
    drop(ws);

    // 7. 幂等:相同回执重复结案 → 仍成功且不产生重复状态
    let code = cutforge_cli::run(vec![
        "notes-resolve".into(),
        root_s.clone(),
        note_id.clone(),
        "--reply".into(), "已把 V1-001 删短到 8s".into(),
        "--op-ids".into(), op_id.clone(),
        "--actor".into(), "agent:claude".into(),
        "--json".into(),
    ]);
    assert_eq!(code, 0, "重复结案(相同回执)必须幂等成功");
    let ws = Workspace::open(&root).unwrap();
    assert_eq!(ws.notes().find(&note_id).unwrap().state, NoteState::Resolved);
    drop(ws);

    let elapsed = t0.elapsed();
    assert!(elapsed.as_secs() <= 3, "全链路须 ≤ 3s,实际 {elapsed:?}");
    cutforge_io::fsutil::cleanup(&root);
}
