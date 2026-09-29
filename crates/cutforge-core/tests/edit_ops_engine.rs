//! 册四 A4 T4.2 门禁:时间线编辑新命令的内核语义。
//! 每命令覆盖:正常路径 / 碰撞与硬约束拒绝 / undo 回滚 / OpLog 回放等价。
//! 样本工程(sample_project):V1-001[0,8400) srcIn 12000、V1-002[8400,14600)、A1-001[8400,8800)。

use cutforge_core::command::{ClipPatch, Command, TrackPatch, TrimEdge, TrimMode};
use cutforge_core::engine::{sample_project, ApplyOpts, Answer, Engine, Query, Reject};
use cutforge_core::oplog::Actor;
use serde_json::json;

fn agent() -> Actor {
    Actor::agent("edit-ops-test")
}

fn apply_ok(eng: &mut Engine, cmd: Command) {
    eng.apply(cmd, agent(), ApplyOpts::default()).unwrap_or_else(|e| panic!("apply 必须成功: {e:?}"));
}

fn clip_json(eng: &Engine, id: &str) -> serde_json::Value {
    match eng.query(Query::Clip { id: id.into() }) {
        Answer::Clip(Some(c)) => c,
        other => panic!("clip {id} 不存在: {other:?}"),
    }
}

fn timeline(eng: &Engine) -> Vec<(String, u64, u64)> {
    match eng.query(Query::Timeline) {
        Answer::Timeline(rows) => rows.into_iter().map(|(id, s, e, _)| (id, s, e)).collect(),
        other => panic!("意外: {other:?}"),
    }
}

// ---------------- clip_trim ----------------

#[test]
fn trim_in_out_shifts_start_duration_source_in() {
    let mut eng = Engine::new(sample_project()).unwrap();
    // trim out -1000:[0,8400) → [0,7400)
    apply_ok(&mut eng, Command::ClipTrim {
        clip_id: "V1-001".into(), mode: TrimMode::Trim, edge: TrimEdge::Out, delta_ms: -1000,
    });
    let c = clip_json(&eng, "V1-001");
    assert_eq!(c["startMs"], json!(0));
    assert_eq!(c["durationMs"], json!(7400));
    // trim in +500:start→500,dur→6900,sourceIn 同步 +500(内容窗口跟随)
    apply_ok(&mut eng, Command::ClipTrim {
        clip_id: "V1-001".into(), mode: TrimMode::Trim, edge: TrimEdge::In, delta_ms: 500,
    });
    let c = clip_json(&eng, "V1-001");
    assert_eq!(c["startMs"], json!(500));
    assert_eq!(c["durationMs"], json!(6900));
    assert_eq!(c["sourceInMs"], json!(12500));
    // undo 两步回到原样(逐字段回滚)
    eng.undo(agent()).unwrap();
    eng.undo(agent()).unwrap();
    let c = clip_json(&eng, "V1-001");
    assert_eq!(c["startMs"], json!(0));
    assert_eq!(c["durationMs"], json!(8400));
    assert_eq!(c["sourceInMs"], json!(12000));
}

#[test]
fn trim_hard_constraint_rejects() {
    let mut eng = Engine::new(sample_project()).unwrap();
    // 越出时间轴起点:in -1 → start < 0
    let r = eng.apply(Command::ClipTrim {
        clip_id: "V1-001".into(), mode: TrimMode::Trim, edge: TrimEdge::In, delta_ms: -1,
    }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::InvariantViolation(_))), "{r:?}");
    // 时长归零:out -8400 → duration 0
    let r = eng.apply(Command::ClipTrim {
        clip_id: "V1-001".into(), mode: TrimMode::Trim, edge: TrimEdge::Out, delta_ms: -8400,
    }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::InvariantViolation(_))), "{r:?}");
    // 素材入点越界:in -12001 → sourceIn < 0
    let r = eng.apply(Command::ClipTrim {
        clip_id: "V1-001".into(), mode: TrimMode::Trim, edge: TrimEdge::In, delta_ms: -12_001,
    }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::InvariantViolation(_))), "{r:?}");
    // 碰撞拒绝:out +1000 → [0,9400) 与 V1-002[8400,14600) 重叠(CF-004)
    let r = eng.apply(Command::ClipTrim {
        clip_id: "V1-001".into(), mode: TrimMode::Trim, edge: TrimEdge::Out, delta_ms: 1000,
    }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::InvariantViolation(_))), "{r:?}");
    assert_eq!(eng.oplog().len(), 0, "拒绝不得产生 Op");
    assert_eq!(eng.rev(), 0);
    // 未知片段
    let r = eng.apply(Command::ClipTrim {
        clip_id: "无此段".into(), mode: TrimMode::Trim, edge: TrimEdge::In, delta_ms: 1,
    }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::UnknownClip(_))), "{r:?}");
}

#[test]
fn roll_moves_boundary_total_duration_unchanged() {
    let mut eng = Engine::new(sample_project()).unwrap();
    apply_ok(&mut eng, Command::ClipSplit { clip_id: "V1-001".into(), t_ms: 4000 });
    // V1: V1-001[0,4000) + V1-003[4000,8400) + V1-002[8400,14600)
    let before_span = timeline(&eng).into_iter().filter(|(id, _, _)| id.starts_with('V'))
        .map(|(_, s, e)| (s, e)).collect::<Vec<_>>();
    // roll out +500:V1-001[0,4500) / V1-003[4500,8400) srcIn 16500(16000+500),总占位不变
    apply_ok(&mut eng, Command::ClipTrim {
        clip_id: "V1-001".into(), mode: TrimMode::Roll, edge: TrimEdge::Out, delta_ms: 500,
    });
    let c = clip_json(&eng, "V1-001");
    assert_eq!(c["durationMs"], json!(4500));
    let r = clip_json(&eng, "V1-003");
    assert_eq!(r["startMs"], json!(4500));
    assert_eq!(r["sourceInMs"], json!(16500));
    // roll in -300:边界 4500→4200,V1-001[0,4200)/V1-003[4200,8400) srcIn 16200
    apply_ok(&mut eng, Command::ClipTrim {
        clip_id: "V1-003".into(), mode: TrimMode::Roll, edge: TrimEdge::In, delta_ms: -300,
    });
    assert_eq!(clip_json(&eng, "V1-001")["durationMs"], json!(4200));
    assert_eq!(clip_json(&eng, "V1-003")["startMs"], json!(4200));
    assert_eq!(clip_json(&eng, "V1-003")["sourceInMs"], json!(16200));
    // 总占位不变:V 轨 min(start)=0、max(end)=14600
    let rows = timeline(&eng).into_iter().filter(|(id, _, _)| id.starts_with('V'))
        .map(|(_, s, e)| (s, e)).collect::<Vec<_>>();
    assert_eq!(rows.iter().map(|(s, _)| *s).min(), Some(0));
    assert_eq!(rows.iter().map(|(_, e)| *e).max(), Some(14_600));
    let _ = before_span;
    // undo:回滚第二次 roll(边界回 4500)
    eng.undo(agent()).unwrap();
    assert_eq!(clip_json(&eng, "V1-001")["durationMs"], json!(4500));
    assert_eq!(clip_json(&eng, "V1-003")["startMs"], json!(4500), "undo 后 startMs 回滚");
    assert_eq!(clip_json(&eng, "V1-003")["sourceInMs"], json!(16500), "undo 后 sourceIn 回滚");
}

#[test]
fn roll_requires_flush_neighbor_and_positive_durations() {
    let mut eng = Engine::new(sample_project()).unwrap();
    // 无贴合右邻(V1-002 是 V1 轨最后一段)→ 拒绝
    let r = eng.apply(Command::ClipTrim {
        clip_id: "V1-002".into(), mode: TrimMode::Roll, edge: TrimEdge::Out, delta_ms: 100,
    }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::InvariantViolation(_))), "{r:?}");
    // V1-001 与 V1-002 之间无片段但边界贴合:V1-001 in 侧无左邻 → 拒绝
    let r = eng.apply(Command::ClipTrim {
        clip_id: "V1-001".into(), mode: TrimMode::Roll, edge: TrimEdge::In, delta_ms: 100,
    }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::InvariantViolation(_))), "{r:?}");
    // 过度压缩:roll out -8400 → 左侧时长 ≤ 0
    let r = eng.apply(Command::ClipTrim {
        clip_id: "V1-001".into(), mode: TrimMode::Roll, edge: TrimEdge::Out, delta_ms: -8400,
    }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::InvariantViolation(_))), "{r:?}");
    assert_eq!(eng.rev(), 0, "全部拒绝不得升 rev");
}

#[test]
fn slip_shifts_source_in_only() {
    let mut eng = Engine::new(sample_project()).unwrap();
    apply_ok(&mut eng, Command::ClipTrim {
        clip_id: "V1-001".into(), mode: TrimMode::Slip, edge: TrimEdge::In, delta_ms: 500,
    });
    let c = clip_json(&eng, "V1-001");
    assert_eq!(c["sourceInMs"], json!(12500));
    assert_eq!(c["startMs"], json!(0), "时间线占位不变");
    assert_eq!(c["durationMs"], json!(8400));
    // 无 sourceIn 的片段从隐含 0 起算:slip +200 落出 sourceIn=200
    apply_ok(&mut eng, Command::ClipTrim {
        clip_id: "V1-002".into(), mode: TrimMode::Slip, edge: TrimEdge::In, delta_ms: 200,
    });
    assert_eq!(clip_json(&eng, "V1-002")["sourceInMs"], json!(200));
    // 越出素材入点:slip -12501 → srcIn(12500)-12501 < 0
    let r = eng.apply(Command::ClipTrim {
        clip_id: "V1-001".into(), mode: TrimMode::Slip, edge: TrimEdge::In, delta_ms: -12_501,
    }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::InvariantViolation(_))), "{r:?}");
    eng.undo(agent()).unwrap();
    assert_eq!(clip_json(&eng, "V1-002").get("sourceInMs"), None, "undo 后不臆造 sourceIn");
}

#[test]
fn slide_moves_clip_neighbors_yield_or_compress() {
    let mut eng = Engine::new(sample_project()).unwrap();
    apply_ok(&mut eng, Command::ClipSplit { clip_id: "V1-001".into(), t_ms: 4000 });
    // slide +200:V1-001[0,4200) 让位 / V1-003[4200,8600) / V1-002[8600,14600) 压缩
    apply_ok(&mut eng, Command::ClipTrim {
        clip_id: "V1-003".into(), mode: TrimMode::Slide, edge: TrimEdge::In, delta_ms: 200,
    });
    assert_eq!(clip_json(&eng, "V1-001")["durationMs"], json!(4200));
    assert_eq!(clip_json(&eng, "V1-003")["startMs"], json!(4200));
    assert_eq!(clip_json(&eng, "V1-002")["startMs"], json!(8600));
    assert_eq!(clip_json(&eng, "V1-002")["durationMs"], json!(6000));
    // 左邻压没:slide -4200 → 新边界 0 ≤ 左邻起点 0 → 拒绝(左邻时长必须 > 0)
    let r = eng.apply(Command::ClipTrim {
        clip_id: "V1-003".into(), mode: TrimMode::Slide, edge: TrimEdge::In, delta_ms: -4200,
    }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::InvariantViolation(_))), "{r:?}");
    // 非贴合滑动进空隙不撞邻居 → 合法;撞上 → CF-004
    apply_ok(&mut eng, Command::ClipTrim {
        clip_id: "V1-001".into(), mode: TrimMode::Trim, edge: TrimEdge::Out, delta_ms: -3200,
    }); // V1-001[0,1000),与 V1-003 间出现空隙
    apply_ok(&mut eng, Command::ClipTrim {
        clip_id: "V1-003".into(), mode: TrimMode::Slide, edge: TrimEdge::In, delta_ms: -3200,
    }); // V1-003[1000,5400),右邻 V1-002 跟随到 5400
    assert_eq!(clip_json(&eng, "V1-003")["startMs"], json!(1000));
    assert_eq!(clip_json(&eng, "V1-002")["startMs"], json!(5400));
    // 滑入**非贴合**邻居 → CF-004 拒绝(贴合邻居会被压缩,不会相撞)
    apply_ok(&mut eng, Command::ClipTrim {
        clip_id: "V1-003".into(), mode: TrimMode::Trim, edge: TrimEdge::Out, delta_ms: -1000,
    }); // V1-003[1000,4400),与 V1-002[5400,…) 间出现 1000ms 空隙
    let r = eng.apply(Command::ClipTrim {
        clip_id: "V1-003".into(), mode: TrimMode::Slide, edge: TrimEdge::In, delta_ms: 1500,
    }, agent(), ApplyOpts::default()); // [2500,5900) 越过 V1-002 起点 5400
    assert!(matches!(r, Err(Reject::InvariantViolation(_))), "{r:?}");
    assert_eq!(clip_json(&eng, "V1-003")["startMs"], json!(1000), "拒绝必须回滚");
    // undo(撤掉最后一笔 trim)后右邻位置不漂移
    eng.undo(agent()).unwrap();
    assert_eq!(clip_json(&eng, "V1-002")["startMs"], json!(5400));
}

// ---------------- clip_split_all ----------------

#[test]
fn split_all_hits_every_track_in_one_op() {
    let mut eng = Engine::new(sample_project()).unwrap();
    apply_ok(&mut eng, Command::ClipSplitAll { t_ms: 8600 });
    assert_eq!(eng.oplog().len(), 1, "全轨分割必须单 Op");
    // V1-002[8400,14600) → [8400,8600)+[8600,14600)(V1-003);A1-001[8400,8800) → [8400,8600)+[8600,8800)(A1-002)
    let rows = timeline(&eng);
    assert!(rows.iter().any(|(id, s, e)| id == "V1-002" && *s == 8400 && *e == 8600));
    assert!(rows.iter().any(|(id, s, e)| id == "V1-003" && *s == 8600 && *e == 14_600));
    assert!(rows.iter().any(|(id, s, e)| id == "A1-001" && *s == 8400 && *e == 8600));
    assert!(rows.iter().any(|(id, s, e)| id == "A1-002" && *s == 8600 && *e == 8800));
    // sourceIn 右段顺移:V1-002 srcIn 缺省(None)→ V1-003 同样缺省,不臆造
    assert_eq!(clip_json(&eng, "V1-003").get("sourceInMs"), None);
    // undo 一刀还原两轨
    eng.undo(agent()).unwrap();
    assert_eq!(timeline(&eng).len(), 3);
    assert!(eng.find_track_bounds_ok());
}

/// 辅助断言(占位以避免未使用告警):V1/A1 轨仍在。
trait FindTrackBounds {
    fn find_track_bounds_ok(&self) -> bool;
}
impl FindTrackBounds for Engine {
    fn find_track_bounds_ok(&self) -> bool {
        self.project().tracks.len() == 2
            && self.project().tracks.iter().all(|t| !t.clips.is_empty())
    }
}

#[test]
fn split_all_outside_every_clip_is_idempotent() {
    let mut eng = Engine::new(sample_project()).unwrap();
    let r = eng.apply(Command::ClipSplitAll { t_ms: 20_000 }, agent(), ApplyOpts::default()).unwrap();
    assert!(r.idempotent, "无命中必须幂等回执");
    assert_eq!(eng.rev(), 0, "无命中不得升 rev");
    assert_eq!(eng.oplog().len(), 0);
}

// ---------------- track_update ----------------

#[test]
fn track_update_field_merge_and_guards() {
    let mut eng = Engine::new(sample_project()).unwrap();
    let r = eng.apply(Command::TrackUpdate { track_id: "V1".into(), patch: TrackPatch {
        name: Some("主画面".into()), locked: Some(true), mute: Some(false),
        solo: Some(true), hidden: Some(false), height_px: Some(240), color: Some("#3D7EAF".into()),
    }}, agent(), ApplyOpts::default()).unwrap();
    assert_eq!(r.rev, 1);
    match eng.query(Query::Track { id: "V1".into() }) {
        Answer::Track(Some(t)) => {
            assert_eq!(t["name"], json!("主画面"));
            assert_eq!(t["locked"], json!(true));
            assert_eq!(t["solo"], json!(true));
            assert_eq!(t["heightPx"], json!(240));
            assert_eq!(t["color"], json!("#3D7EAF"));
            assert!(t.get("mute").is_some() && t["mute"] == json!(false));
            assert!(t.get("hidden").is_some() && t["hidden"] == json!(false));
        }
        other => panic!("意外: {other:?}"),
    }
    // 部分合并:只改 mute,其余保持
    apply_ok(&mut eng, Command::TrackUpdate { track_id: "V1".into(), patch: TrackPatch { mute: Some(true), ..Default::default() } });
    match eng.query(Query::Track { id: "V1".into() }) {
        Answer::Track(Some(t)) => {
            assert_eq!(t["mute"], json!(true));
            assert_eq!(t["name"], json!("主画面"), "未给出的字段不得被清掉");
            assert_eq!(t["heightPx"], json!(240));
        }
        other => panic!("意外: {other:?}"),
    }
    // 同值 patch → 幂等回执,不升 rev
    let r = eng.apply(Command::TrackUpdate { track_id: "V1".into(), patch: TrackPatch { mute: Some(true), ..Default::default() } }, agent(), ApplyOpts::default()).unwrap();
    assert!(r.idempotent);
    assert_eq!(eng.rev(), 2);
    // 空 patch / 未知轨
    let r = eng.apply(Command::TrackUpdate { track_id: "V1".into(), patch: TrackPatch::default() }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::EmptyPatch(_))), "{r:?}");
    let r = eng.apply(Command::TrackUpdate { track_id: "X9".into(), patch: TrackPatch { mute: Some(true), ..Default::default() } }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::UnknownTrack(_))), "{r:?}");
    // schema 层守护:heightPx 低于允许集(≥16)→ SCHEMA_INVALID 且回滚
    let before = eng.query(Query::Track { id: "V1".into() });
    let r = eng.apply(Command::TrackUpdate { track_id: "V1".into(), patch: TrackPatch { height_px: Some(10), ..Default::default() } }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::SchemaInvalid(_))), "{r:?}");
    assert_eq!(eng.query(Query::Track { id: "V1".into() }), before, "拒绝必须回滚");
    // undo:逐字段回滚
    eng.undo(agent()).unwrap();
    eng.undo(agent()).unwrap();
    match eng.query(Query::Track { id: "V1".into() }) {
        Answer::Track(Some(t)) => {
            assert!(t.get("name").is_none() && t.get("locked").is_none() && t.get("heightPx").is_none());
        }
        other => panic!("意外: {other:?}"),
    }
}

// ---------------- clip_gap_delete ----------------

#[test]
fn gap_delete_closes_gap_and_shifts_successors() {
    let mut eng = Engine::new(sample_project()).unwrap();
    apply_ok(&mut eng, Command::ClipSplit { clip_id: "V1-001".into(), t_ms: 4000 });
    apply_ok(&mut eng, Command::ClipMove { clip_id: "V1-003".into(), new_start_ms: 15_000, to_track: None });
    // V1: V1-001[0,4000) | gap 4400 | V1-002[8400,14600) | gap 400 | V1-003[15000,19000)
    apply_ok(&mut eng, Command::ClipGapDelete { track_id: "V1".into(), t_ms: 14_650 });
    assert_eq!(clip_json(&eng, "V1-003")["startMs"], json!(14_600), "尾段前 400ms 间隙闭合");
    assert_eq!(clip_json(&eng, "V1-002")["startMs"], json!(8400), "间隙之前不动");
    // 中段大间隙:tMs 5000 命中 [4000,8400),后继整体左移 4400
    apply_ok(&mut eng, Command::ClipGapDelete { track_id: "V1".into(), t_ms: 5000 });
    assert_eq!(clip_json(&eng, "V1-002")["startMs"], json!(4000));
    assert_eq!(clip_json(&eng, "V1-003")["startMs"], json!(10_200));
    // 首段前空档:slip 成 gap_delete(A1 轨头部 8400ms 空档)
    apply_ok(&mut eng, Command::ClipGapDelete { track_id: "A1".into(), t_ms: 100 });
    assert_eq!(clip_json(&eng, "A1-001")["startMs"], json!(0));
    // undo 三步还原
    for _ in 0..3 {
        eng.undo(agent()).unwrap();
    }
    assert_eq!(clip_json(&eng, "A1-001")["startMs"], json!(8400));
    assert_eq!(clip_json(&eng, "V1-003")["startMs"], json!(15_000));
}

#[test]
fn gap_delete_rejects_when_no_gap_at_t() {
    let mut eng = Engine::new(sample_project()).unwrap();
    // t 在片段内部(V1-001 [0,8400))
    let r = eng.apply(Command::ClipGapDelete { track_id: "V1".into(), t_ms: 3000 }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::InvariantViolation(_))), "{r:?}");
    // t 在全部片段之后
    let r = eng.apply(Command::ClipGapDelete { track_id: "V1".into(), t_ms: 20_000 }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::InvariantViolation(_))), "{r:?}");
    // 未知轨
    let r = eng.apply(Command::ClipGapDelete { track_id: "X9".into(), t_ms: 0 }, agent(), ApplyOpts::default());
    assert!(matches!(r, Err(Reject::UnknownTrack(_))), "{r:?}");
    assert_eq!(eng.rev(), 0);
}

// ---------------- OpLog 回放等价(新命令混入) ----------------

#[test]
fn replay_covers_all_new_commands() {
    let base = sample_project();
    let mut eng = Engine::new(base.clone()).unwrap();
    let seq: Vec<Command> = vec![
        Command::ClipSplit { clip_id: "V1-001".into(), t_ms: 4000 },
        Command::ClipTrim { clip_id: "V1-001".into(), mode: TrimMode::Roll, edge: TrimEdge::Out, delta_ms: 400 },
        Command::ClipTrim { clip_id: "V1-003".into(), mode: TrimMode::Slip, edge: TrimEdge::In, delta_ms: 250 },
        Command::ClipTrim { clip_id: "V1-003".into(), mode: TrimMode::Slide, edge: TrimEdge::In, delta_ms: 100 },
        Command::TrackUpdate { track_id: "V1".into(), patch: TrackPatch {
            name: Some("主画面".into()), height_px: Some(240), color: Some("#3D7EAF".into()),
            mute: Some(false), solo: Some(true), ..Default::default() } },
        Command::ClipSplitAll { t_ms: 8600 },
        Command::ClipGapDelete { track_id: "A1".into(), t_ms: 100 },
        Command::ClipTrim { clip_id: "V1-001".into(), mode: TrimMode::Trim, edge: TrimEdge::Out, delta_ms: -500 },
        Command::ClipUpdate { clip_id: "V1-001".into(), patch: ClipPatch { volume: Some(0.9), ..Default::default() } },
    ];
    for cmd in seq {
        eng.apply(cmd, agent(), ApplyOpts::default()).unwrap();
    }
    eng.undo(agent()).unwrap();
    eng.redo(agent()).unwrap();
    let expected_hash = eng.state_hash();
    let replayed = Engine::replay(base, eng.oplog().ops()).unwrap();
    assert_eq!(replayed.state_hash(), expected_hash, "新命令的 Op 必须可回放且等价");
    assert_eq!(replayed.rev(), eng.rev());
}

#[test]
fn trim_undo_roundtrip_via_projection() {
    // 单命令 undo/redo 往返:投影逐字段一致
    let mut eng = Engine::new(sample_project()).unwrap();
    let before = clip_json(&eng, "V1-001").clone();
    apply_ok(&mut eng, Command::ClipTrim {
        clip_id: "V1-001".into(), mode: TrimMode::Trim, edge: TrimEdge::In, delta_ms: 1200,
    });
    eng.undo(agent()).unwrap();
    let after_undo = clip_json(&eng, "V1-001");
    assert_eq!(after_undo, before, "undo 后投影必须逐字段还原");
    eng.redo(agent()).unwrap();
    let after_redo = clip_json(&eng, "V1-001");
    assert_eq!(after_redo["startMs"], json!(1200));
    assert_eq!(after_redo["sourceInMs"], json!(13200));
}
