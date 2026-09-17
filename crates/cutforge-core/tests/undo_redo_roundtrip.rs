//! M2-3 门禁:撤销栈完整性。
//! 任意操作序列执行 N 次后全部 undo,状态 hash 与初始相同;再全部 redo,与最终状态相同。

use cutforge_core::command::{ClipPatch, Command};
use cutforge_core::engine::{sample_project, ApplyOpts, Engine};
use cutforge_core::oplog::Actor;

fn agent() -> Actor {
    Actor::agent("undo-test")
}

#[test]
fn undo_redo_roundtrip() {
    let mut eng = Engine::new(sample_project()).unwrap();
    let initial_hash = eng.state_hash();

    let seq: Vec<Command> = vec![
        Command::ClipUpdate { clip_id: "V1-001".into(), patch: ClipPatch { duration_ms: Some(8000), ..Default::default() } },
        Command::ClipSplit { clip_id: "V1-001".into(), t_ms: 4000 },
        Command::ClipUpdate { clip_id: "V1-003".into(), patch: ClipPatch { volume: Some(0.5), ..Default::default() } },
        Command::ClipMove { clip_id: "A1-001".into(), new_start_ms: 9000, to_track: None },
        Command::ClipDelete { clip_id: "V1-003".into() },
        Command::ClipUpdate { clip_id: "V1-002".into(), patch: ClipPatch { volume: Some(0.9), ..Default::default() } },
        Command::ClipMove { clip_id: "A1-001".into(), new_start_ms: 8400, to_track: None },
        Command::ClipSplit { clip_id: "V1-001".into(), t_ms: 2000 },
    ];

    let mut applied = 0usize;
    for cmd in &seq {
        if eng.apply(cmd.clone(), agent(), ApplyOpts::default()).is_ok() {
            applied += 1;
        }
    }
    assert_eq!(applied, seq.len(), "序列必须全部合法");
    let final_hash = eng.state_hash();
    assert_ne!(initial_hash, final_hash);

    // 全部 undo → 回到初始状态
    for _ in 0..applied {
        eng.undo(agent()).expect("undo 不应耗尽");
    }
    assert_eq!(eng.state_hash(), initial_hash, "全部 undo 后必须回到初始状态");
    assert_eq!(eng.rev(), (applied * 2) as u64, "每次 undo 也产生新 Op");

    // 全部 redo → 回到最终状态
    for _ in 0..applied {
        eng.redo(agent()).expect("redo 不应耗尽");
    }
    assert_eq!(eng.state_hash(), final_hash, "全部 redo 后必须回到最终状态");

    // 交错往返(乱序混合不再合法,但成对 undo/redo 幂等)
    eng.undo(agent()).unwrap();
    eng.redo(agent()).unwrap();
    assert_eq!(eng.state_hash(), final_hash);
}

#[test]
fn replay_is_equivalent_to_step_by_step() {
    let base = sample_project();
    let mut eng = Engine::new(base.clone()).unwrap();
    let ops = [
        Command::ClipUpdate { clip_id: "V1-001".into(), patch: ClipPatch { duration_ms: Some(8000), ..Default::default() } },
        Command::ClipSplit { clip_id: "V1-001".into(), t_ms: 4000 },
        Command::ClipMove { clip_id: "A1-001".into(), new_start_ms: 9000, to_track: None },
    ];
    for cmd in ops {
        eng.apply(cmd, agent(), ApplyOpts::default()).unwrap();
    }
    let expected_hash = eng.state_hash();

    let replayed = Engine::replay(base, eng.oplog().ops()).unwrap();
    assert_eq!(replayed.state_hash(), expected_hash, "回放必须与逐步 apply 语义一致");
    assert_eq!(replayed.rev(), eng.rev());
}

#[test]
fn random_walk_undo_reduces_to_initial() {
    // 伪随机(LCG)操作流:每次成功 apply 后立即 undo,状态必须逐步还原。
    let mut eng = Engine::new(sample_project()).unwrap();
    let initial_hash = eng.state_hash();
    let mut seed: u64 = 20260918;
    let mut lcg = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        seed >> 33
    };
    let targets = ["V1-001", "V1-002", "A1-001"];
    let mut applied = 0usize;
    for _ in 0..50 {
        let which = (lcg() as usize) % targets.len();
        let dur = 100 + lcg() % 2000;
        let cmd = match (lcg() as usize) % 3 {
            0 => Command::ClipUpdate {
                clip_id: targets[which].into(),
                patch: ClipPatch { volume: Some((lcg() % 20) as f64 / 10.0), ..Default::default() },
            },
            1 => Command::ClipSplit { clip_id: targets[which].into(), t_ms: 100 + lcg() % 1000 },
            _ => Command::ClipMove { clip_id: targets[which].into(), new_start_ms: lcg() % 500, to_track: None },
        };
        if eng.apply(cmd, agent(), ApplyOpts::default()).is_ok() {
            applied += 1;
        }
    }
    for _ in 0..applied {
        eng.undo(agent()).unwrap();
    }
    assert_eq!(eng.state_hash(), initial_hash, "{applied} 次操作后全部 undo 必须还原");
    assert_eq!(eng.rev(), (applied * 2) as u64);
    let _ = &mut lcg;
}
