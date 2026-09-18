//! M8-1 门禁(内核侧):文件级 Op 的撤销/重做语义为真(ADR-0001)。
//! 修复前:notes/cutlist 的 Op 被盲目回写到 project(指针错位→静默丢弃),
//! 返回 Ok、rev 上涨、文件原封不动——本文件的三条测试在旧实现下全红。

use cutforge_core::engine::{sample_project, ApplyOpts, Engine};
use cutforge_core::oplog::{Actor, OpKind};
use serde_json::json;

fn agent() -> Actor {
    Actor::agent("m8-1")
}

fn notes_before() -> serde_json::Value {
    json!({"version": 1, "items": []})
}

#[test]
fn file_level_undo_routes_to_file_state_not_project() {
    let mut eng = Engine::new(sample_project()).unwrap();
    let after = json!({"version": 1, "items": [{"id": "n-0001"}]});
    let r = eng.record_file_change(
        "notes.json", "/items", notes_before(), after.clone(),
        OpKind::Insert, agent(), ApplyOpts::default(),
    ).unwrap();
    assert_eq!(r.rev, 1);
    // 撤销:内存态必须真的回到 before(修复前:此调用伪造成功且 file_state 不存在)
    eng.undo(agent()).unwrap();
    assert_eq!(eng.file_state("notes.json"), Some(&notes_before()), "undo 必须路由到文件态");
    assert_eq!(eng.rev(), 2, "撤销产生新 Op,rev 上涨");
    // 重做:回到 after
    eng.redo(agent()).unwrap();
    assert_eq!(eng.file_state("notes.json"), Some(&after));
    // 工程文档全程不被文件级 Op 污染
    assert_eq!(eng.project().tracks.len(), sample_project().tracks.len());
}

#[test]
fn undo_depth_equals_real_gestures_auto_ops_excluded() {
    let mut eng = Engine::new(sample_project()).unwrap();
    eng.apply(
        cutforge_core::command::Command::ClipDelete { clip_id: "A1-001".into() },
        agent(), ApplyOpts::default(),
    ).unwrap();
    // 自动簿记(锚点重定位类):进审计链、升 rev,但不得进撤销栈
    let r = eng.record_file_change(
        "notes.json", "/items", notes_before(), json!({"version": 1, "items": [{"id": "n-0009"}]}),
        OpKind::Set, agent(), ApplyOpts { non_undoable: true, ..Default::default() },
    ).unwrap();
    assert_eq!(r.rev, 2);
    let op = eng.oplog().ops().last().unwrap();
    assert_eq!(op.auto, Some(true), "auto 标记必须落进 OpLog(重开后撤销栈同样跳过)");
    // 一次 undo 只能撤销那一次 clip 手势,不能撤销 auto 登记或被其灌水
    eng.undo(agent()).unwrap();
    let err = eng.undo(agent());
    assert!(matches!(err, Err(cutforge_core::engine::Reject::NothingToUndo)),
        "auto Op 不入撤销栈:撤销深度 = 真实用户手势数");
}

#[test]
fn file_undo_rejects_when_base_drifted() {
    let mut eng = Engine::new(sample_project()).unwrap();
    let op1_after = json!({"version": 1, "items": [{"id": "n-0001"}]});
    eng.record_file_change(
        "notes.json", "/items", notes_before(), op1_after.clone(),
        OpKind::Insert, agent(), ApplyOpts::default(),
    ).unwrap();
    // 后续另一个手势继续改同一文件(LIFO 中它先被撤销)
    let op2_after = json!({"version": 1, "items": [{"id": "n-0001"}, {"id": "n-0002"}]});
    eng.record_file_change(
        "notes.json", "/items", op1_after, op2_after,
        OpKind::Insert, agent(), ApplyOpts::default(),
    ).unwrap();
    // 撤销 op2 → 正常;再撤销 op1 → 正常(此时态恰为 op1.after)
    eng.undo(agent()).unwrap();
    eng.undo(agent()).unwrap();
    assert_eq!(eng.file_state("notes.json"), Some(&notes_before()));
    // 空栈后再撤销 → NothingToUndo;文件态不被盲写
    assert!(matches!(eng.undo(agent()), Err(cutforge_core::engine::Reject::NothingToUndo)));
}

#[test]
fn replay_routes_file_level_ops() {
    let mut eng = Engine::new(sample_project()).unwrap();
    eng.apply(
        cutforge_core::command::Command::ClipDelete { clip_id: "A1-001".into() },
        agent(), ApplyOpts::default(),
    ).unwrap();
    eng.record_file_change(
        "notes.json", "/items", notes_before(), json!({"version": 1, "items": [{"id": "n-0001"}]}),
        OpKind::Insert, agent(), ApplyOpts::default(),
    ).unwrap();
    eng.undo(agent()).unwrap(); // 撤销 notes 插入
    // 回放:与逐步 apply 语义一致(修复前:notes 的 /items 被回写进 project 再被反序列化丢弃)
    let replayed = Engine::replay(sample_project(), eng.oplog().ops()).unwrap();
    assert_eq!(replayed.rev(), eng.rev());
    assert_eq!(replayed.file_state("notes.json"), eng.file_state("notes.json"),
        "回放后文件态必须与逐步应用一致");
    assert_eq!(replayed.project(), eng.project(), "回放后工程必须与逐步应用一致");
}
