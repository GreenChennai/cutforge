//! M3-3 门禁:OpLog 回放等价。
//! 从工程 + 完整 OpLog 回放得到的状态,与逐步 apply 的当前状态语义 hash 相等;
//! 前缀回放与对应 rev 的状态相等;undo/redo 混入时回放跳过镜像 Op。

use cutforge_core::command::{ClipPatch, Command};
use cutforge_core::engine::{sample_project, ApplyOpts, Engine};
use cutforge_core::oplog::Actor;

fn agent() -> Actor {
    Actor::agent("replay-test")
}

#[test]
fn oplog_replay_hash_eq() {
    let base = sample_project();
    let mut eng = Engine::new(base.clone()).unwrap();

    let seq: Vec<Command> = vec![
        Command::ClipUpdate { clip_id: "V1-001".into(), patch: ClipPatch { duration_ms: Some(8000), ..Default::default() } },
        Command::ClipSplit { clip_id: "V1-001".into(), t_ms: 4000 },
        Command::ClipUpdate { clip_id: "V1-003".into(), patch: ClipPatch { volume: Some(0.5), ..Default::default() } },
        Command::ClipMove { clip_id: "A1-001".into(), new_start_ms: 9000, to_track: None },
        Command::ClipDelete { clip_id: "V1-003".into() },
        Command::ClipMove { clip_id: "V1-002".into(), new_start_ms: 4000, to_track: None },
        Command::ClipMerge { left_id: "V1-001".into(), right_id: "V1-002".into() },
        Command::ClipUpdate { clip_id: "V1-001".into(), patch: ClipPatch { volume: Some(0.9), ..Default::default() } },
    ];
    for cmd in seq {
        eng.apply(cmd, agent(), ApplyOpts::default()).unwrap();
    }
    // 中途撤销一步再重做(日志里混入 undo/redo 镜像 Op)
    eng.undo(agent()).unwrap();
    eng.redo(agent()).unwrap();

    let expected_hash = eng.state_hash();
    let expected_rev = eng.rev();

    let replayed = Engine::replay(base.clone(), eng.oplog().ops()).unwrap();
    assert_eq!(replayed.state_hash(), expected_hash, "全量回放必须与实时状态等价");
    assert_eq!(replayed.rev(), expected_rev, "回放后的 rev 必须一致");

    // 回放输出的 OpLog 再喂一次(往返稳定)
    let again = Engine::replay(base.clone(), replayed.oplog().ops()).unwrap();
    assert_eq!(again.state_hash(), expected_hash);
}

#[test]
fn replay_prefix_equals_stepwise_state() {
    // 逐前缀:回放前 k 条 Op 的状态 == 实时引擎做了 k 条的状态
    let base = sample_project();
    let seq: Vec<Command> = vec![
        Command::ClipUpdate { clip_id: "V1-001".into(), patch: ClipPatch { duration_ms: Some(8100), ..Default::default() } },
        Command::ClipSplit { clip_id: "V1-001".into(), t_ms: 3000 },
        Command::ClipMove { clip_id: "A1-001".into(), new_start_ms: 9500, to_track: None },
        Command::ClipUpdate { clip_id: "V1-002".into(), patch: ClipPatch { volume: Some(0.7), ..Default::default() } },
    ];
    let mut live = Engine::new(base.clone()).unwrap();
    for (i, cmd) in seq.iter().enumerate() {
        live.apply(cmd.clone(), agent(), ApplyOpts::default()).unwrap();
        // 独立引擎逐步复现前 i+1 条
        let mut stepwise = Engine::new(base.clone()).unwrap();
        for c in &seq[..=i] {
            stepwise.apply(c.clone(), agent(), ApplyOpts::default()).unwrap();
        }
        let replayed = Engine::replay(base.clone(), live.oplog().ops()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            stepwise.state_hash(),
            "前缀 {i} 回放不一致"
        );
    }
}

#[test]
fn replay_after_undo_matches_undone_state() {
    let base = sample_project();
    let mut eng = Engine::new(base.clone()).unwrap();
    eng.apply(Command::ClipUpdate { clip_id: "V1-001".into(), patch: ClipPatch { duration_ms: Some(8000), ..Default::default() } }, agent(), ApplyOpts::default()).unwrap();
    eng.apply(Command::ClipUpdate { clip_id: "V1-002".into(), patch: ClipPatch { volume: Some(0.6), ..Default::default() } }, agent(), ApplyOpts::default()).unwrap();
    eng.undo(agent()).unwrap(); // 撤销第二条 → 状态只含第一条
    let replayed = Engine::replay(base.clone(), eng.oplog().ops()).unwrap();
    assert_eq!(replayed.state_hash(), eng.state_hash(), "含 undo 的日志回放必须跳过镜像 Op");
}
