// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! TC-CORE-APPLY-099(R-12①):apply 失败回滚后 project 与原态**逐字节相等**。
//!
//! R-12① 把全量 clone 快照换成 Revert 逆向回滚——回滚正确性必须与快照方案
//! 等价:各失败路径(mutate 中途拒绝 / schema 拒绝 / 幂等无变更 / trim 部分
//! 修改后拒绝)逐一验证 canonical JSON 字节级还原,且 rev/OpLog 零污染。

use cutforge_core::command::{ClipPatch, Command, TransitionPatch, TrimEdge, TrimMode};
use cutforge_core::engine::{
    Answer, ApplyOpts, Engine, Query, Reject, canonical_json, sample_project,
};
use cutforge_core::model::Project;
use cutforge_core::oplog::Actor;
use serde_json::json;

fn agent() -> Actor {
    Actor::agent("tc-099")
}

/// canonical JSON 字节(排序键;逐字节相等的判定面)。
fn bytes_of(eng: &Engine) -> String {
    let v = serde_json::to_value(eng.project()).expect("工程必须可序列化");
    canonical_json(&v)
}

/// 同一命令流在各失败路径下的逐字节还原断言(前置:先落一笔真实变更,
/// 保证回滚面非"空工程原态"的平凡情形)。
fn assert_rollback_byte_equal(fail: fn(&mut Engine)) {
    let mut eng = Engine::new(sample_project()).unwrap();
    eng.apply(
        Command::ClipUpdate {
            clip_id: "V1-002".into(),
            patch: ClipPatch {
                volume: Some(0.66),
                ..Default::default()
            },
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let before = bytes_of(&eng);
    let (rev_before, ops_before) = (eng.rev(), eng.oplog().len());
    fail(&mut eng);
    assert_eq!(
        bytes_of(&eng),
        before,
        "失败回滚后 project 必须与原态逐字节相等"
    );
    assert_eq!(eng.rev(), rev_before, "失败不得升 rev");
    assert_eq!(eng.oplog().len(), ops_before, "失败不得产生 Op");
}

/// 路径 1:mutate 中途拒绝(enforce_no_overlap 在修改后裁决)→ 逆向回滚。
fn fail_mutate_midway(eng: &mut Engine) {
    // V1-001[0,8400) 移到 9000 与 V1-002[8400,14600) 重叠:clip 值已改、数组已
    // 重排后才被 overlap 裁决——回滚必须还原 clip 值与数组序
    let r = eng.apply(
        Command::ClipUpdate {
            clip_id: "V1-001".into(),
            patch: ClipPatch {
                start_ms: Some(9000),
                duration_ms: Some(2000),
                ..Default::default()
            },
        },
        agent(),
        ApplyOpts::default(),
    );
    assert!(matches!(r, Err(Reject::InvariantViolation(_))), "{r:?}");
}

/// 路径 2:schema 增量校验拒绝(枚举外转场)→ 逆向回滚。
fn fail_schema(eng: &mut Engine) {
    let r = eng.apply(
        Command::ClipUpdate {
            clip_id: "V1-001".into(),
            patch: ClipPatch {
                transition: Some(TransitionPatch {
                    type_: Some("爆闪".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        },
        agent(),
        ApplyOpts::default(),
    );
    assert!(matches!(r, Err(Reject::SchemaInvalid(_))), "{r:?}");
}

/// 路径 3:幂等无变更(同值 patch)→ 还原 + 幂等回执。
fn fail_idempotent(eng: &mut Engine) {
    let r = eng
        .apply(
            Command::ClipUpdate {
                clip_id: "V1-002".into(),
                patch: ClipPatch {
                    volume: Some(0.66),
                    ..Default::default()
                },
            },
            agent(),
            ApplyOpts::default(),
        )
        .unwrap();
    assert!(r.idempotent, "同值 patch 必须回执幂等");
}

/// TC-CORE-ROLL-001 同款夹具(roll In 使 sourceIn 变负 → 拒绝式守卫)。
fn roll_reject_engine() -> Engine {
    let v = json!({
        "version": 1, "schemaVersion": "2.0.0", "slug": "tc-099-roll", "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "tracks": [
            {"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 1000,
                 "sourceInMs": 5000},
                {"id": "V1-002", "src": "a.mp4", "startMs": 1000, "durationMs": 1000,
                 "sourceInMs": 100}
            ]}
        ]
    });
    Engine::new(Project::from_value(&v).unwrap()).unwrap()
}

#[test]
fn tc_core_apply_099_rollback_byte_equal_mutate_reject() {
    assert_rollback_byte_equal(fail_mutate_midway);
}

#[test]
fn tc_core_apply_099_rollback_byte_equal_schema_reject() {
    assert_rollback_byte_equal(fail_schema);
}

#[test]
fn tc_core_apply_099_rollback_byte_equal_idempotent_nochange() {
    assert_rollback_byte_equal(fail_idempotent);
}

/// 路径 4:slide 部分修改后拒绝(左邻 duration 已改、右邻/越界裁决失败)。
#[test]
fn tc_core_apply_099_rollback_byte_equal_slide_partial() {
    let mut eng = Engine::new(sample_project()).unwrap();
    // slide 需要"贴合"邻居:V1-001 切出 [0,2000)+[2000,4000),2000 边界两侧贴合
    eng.apply(
        Command::ClipSplit {
            clip_id: "V1-001".into(),
            t_ms: 2000,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    // slide -3000:V1-003[2000,4000) 左移越出时间轴起点 → 越界拒绝
    // (部分修改面:左邻 duration 已被改写之后才被裁决)
    let before = bytes_of(&eng);
    let r = eng.apply(
        Command::ClipTrim {
            clip_id: "V1-003".into(),
            mode: TrimMode::Slide,
            edge: TrimEdge::In,
            delta_ms: -3000,
        },
        agent(),
        ApplyOpts::default(),
    );
    assert!(r.is_err(), "slide 越界必须拒绝: {r:?}");
    assert_eq!(bytes_of(&eng), before, "部分修改后的拒绝必须逐字节还原");
    assert_eq!(eng.rev(), 1, "拒绝不得升 rev(切分后为 1)");
}

/// 路径 5:roll 部分修改后拒绝(邻居 duration 已改、sourceIn 拒绝式守卫失败)。
#[test]
fn tc_core_apply_099_rollback_byte_equal_roll_partial() {
    let mut eng = roll_reject_engine();
    let before = bytes_of(&eng);
    let r = eng.apply(
        Command::ClipTrim {
            clip_id: "V1-002".into(),
            mode: TrimMode::Roll,
            edge: TrimEdge::In,
            delta_ms: -600,
        },
        agent(),
        ApplyOpts::default(),
    );
    match r {
        Err(Reject::InvariantViolation(m)) => assert!(m.contains("素材入点"), "{m}"),
        other => panic!("负 sourceIn 必须被拒绝式守卫拦下: {other:?}"),
    }
    assert_eq!(
        bytes_of(&eng),
        before,
        "roll 部分修改后的拒绝必须逐字节还原"
    );
    assert_eq!(eng.rev(), 0, "拒绝不得升 rev");
    assert_eq!(eng.oplog().len(), 0, "拒绝不得产生 Op");
    // 对照:拒绝后片段值未被回绕(BUG-03 语义随 R-12① 回滚保持)
    match eng.query(Query::Clip {
        id: "V1-002".into(),
    }) {
        Answer::Clip(Some(c)) => assert_eq!(c["sourceInMs"], json!(100)),
        other => panic!("意外: {other:?}"),
    }
}
