//! 册四 A4 T4.7 集成测试:文本样式 IR 的 patch 合并语义 + 批量命令
//! (ClipsInsert/ClipsPatch)的单 Op 原子性。
//! command.rs/engine 行数红线(≤800),本册新增语义的测试集中于此。

use cutforge_core::command::{ClipPatch, Command, TrackPatch};
use cutforge_core::engine::{ApplyOpts, Engine};
use cutforge_core::text_style::{Huazi, TextStyle};
use cutforge_core::{Actor, ApplyOpts as Opts};
use serde_json::{json, Value};

fn agent() -> Actor {
    Actor::agent("test")
}

fn project(v: Value) -> Engine {
    Engine::new(serde_json::from_value(v).unwrap()).unwrap()
}

fn base() -> Value {
    json!({
        "version": 1, "schemaVersion": "2.0.0", "slug": "t", "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "tracks": [
            {"id": "T1", "kind": "text", "clips": [
                {"id": "T1-001", "startMs": 0, "durationMs": 2000, "text": "第一句"},
                {"id": "T1-002", "startMs": 2000, "durationMs": 2000, "text": "第二句"}
            ]}
        ]
    })
}

#[test]
fn text_style_patch_replaces_whole_object() {
    let mut eng = project(base());
    let style = TextStyle {
        font_family: Some("思源黑体".into()),
        font_size: Some(72.0),
        color: Some("#FFCC00".into()),
        align: Some("topCenter".into()),
        ..Default::default()
    };
    let r = eng.apply(Command::ClipUpdate {
        clip_id: "T1-001".into(),
        patch: ClipPatch { text_style: Some(style.clone()), ..Default::default() },
    }, agent(), Opts::default()).unwrap();
    assert_eq!(r.rev, 1);
    match eng.query(cutforge_core::Query::Clip { id: "T1-001".into() }) {
        cutforge_core::Answer::Clip(Some(c)) => {
            assert_eq!(c["textStyle"]["fontFamily"], json!("思源黑体"));
            assert_eq!(c["textStyle"]["fontSize"], json!(72.0));
            assert!(c["textStyle"].get("karaoke").is_none(), "未给出的字段不得臆造");
        }
        other => panic!("意外: {other:?}"),
    }
    // 整对象替换:第二次只给 color → fontFamily 不得残留
    let r = eng.apply(Command::ClipUpdate {
        clip_id: "T1-001".into(),
        patch: ClipPatch {
            text_style: Some(TextStyle { color: Some("#FFFFFF".into()), ..Default::default() }),
            ..Default::default()
        },
    }, agent(), Opts::default()).unwrap();
    assert_eq!(r.rev, 2);
    match eng.query(cutforge_core::Query::Clip { id: "T1-001".into() }) {
        cutforge_core::Answer::Clip(Some(c)) => {
            assert!(c["textStyle"].get("fontFamily").is_none(), "整对象替换:旧 fontFamily 不得残留");
            assert_eq!(c["textStyle"]["color"], json!("#FFFFFF"));
        }
        other => panic!("意外: {other:?}"),
    }
    // 同值 patch:幂等零变更
    let r = eng.apply(Command::ClipUpdate {
        clip_id: "T1-001".into(),
        patch: ClipPatch {
            text_style: Some(TextStyle { color: Some("#FFFFFF".into()), ..Default::default() }),
            ..Default::default()
        },
    }, agent(), Opts::default()).unwrap();
    assert!(r.idempotent, "同值 textStyle 不得计入变更");
}

#[test]
fn huazi_patch_replaces_and_undo_restores() {
    let mut eng = project(base());
    let hz = Huazi { template: "hz.pop".into(), params: None };
    eng.apply(Command::ClipUpdate {
        clip_id: "T1-001".into(),
        patch: ClipPatch { huazi: Some(hz.clone()), ..Default::default() },
    }, agent(), Opts::default()).unwrap();
    match eng.query(cutforge_core::Query::Clip { id: "T1-001".into() }) {
        cutforge_core::Answer::Clip(Some(c)) => assert_eq!(c["huazi"]["template"], json!("hz.pop")),
        other => panic!("意外: {other:?}"),
    }
    // 覆盖:换模板
    eng.apply(Command::ClipUpdate {
        clip_id: "T1-001".into(),
        patch: ClipPatch {
            huazi: Some(Huazi { template: "hz.box".into(), params: None }),
            ..Default::default()
        },
    }, agent(), Opts::default()).unwrap();
    match eng.query(cutforge_core::Query::Clip { id: "T1-001".into() }) {
        cutforge_core::Answer::Clip(Some(c)) => assert_eq!(c["huazi"]["template"], json!("hz.box")),
        other => panic!("意外: {other:?}"),
    }
    // undo 逐级还原
    eng.undo(agent()).unwrap();
    eng.undo(agent()).unwrap();
    match eng.query(cutforge_core::Query::Clip { id: "T1-001".into() }) {
        cutforge_core::Answer::Clip(Some(c)) => assert!(c.get("huazi").is_none(), "undo 应还原到无花字"),
        other => panic!("意外: {other:?}"),
    }
}

#[test]
fn clips_insert_is_single_atomic_op() {
    let mut eng = project(base());
    let mk = |id: &str, s: u64| {
        serde_json::from_value(json!({
            "id": id, "startMs": s, "durationMs": 1000, "text": format!("句{id}")
        })).unwrap()
    };
    let clips = vec![mk("T1-003", 4000), mk("T1-004", 5000), mk("T1-005", 6000)];
    let r = eng.apply(Command::ClipsInsert {
        to_track: "T1".into(), clips, request_id: Some("imp-1".into()),
    }, agent(), ApplyOpts { request_id: Some("imp-1".into()), ..Default::default() }).unwrap();
    assert_eq!(r.op_ids.len(), 1, "批量插入必须单 Op");
    assert_eq!(r.rev, 1);
    // 幂等:同 request_id 重复 → 幂等回执
    let clips2 = vec![mk("T1-006", 7000)];
    let r2 = eng.apply(Command::ClipsInsert {
        to_track: "T1".into(), clips: clips2, request_id: Some("imp-1".into()),
    }, agent(), ApplyOpts { request_id: Some("imp-1".into()), ..Default::default() }).unwrap();
    assert!(r2.idempotent, "同 request_id 幂等");
    assert_eq!(eng.rev(), 1);
    // 原子性:id 撞车 → 整批拒绝,部分不得落盘
    let bad = vec![mk("T1-007", 7000), mk("T1-001", 8000)];
    let r3 = eng.apply(Command::ClipsInsert {
        to_track: "T1".into(), clips: bad, request_id: None,
    }, agent(), ApplyOpts::default());
    assert!(matches!(r3, Err(cutforge_core::Reject::DuplicateClipId(_))), "id 重复整批拒: {r3:?}");
    match eng.query(cutforge_core::Query::Timeline) {
        cutforge_core::Answer::Timeline(tl) => {
            assert_eq!(tl.len(), 5, "拒绝批零落盘(3 插入 + 2 原有)");
        }
        other => panic!("意外: {other:?}"),
    }
    // 重叠:与既有片段重叠 → 整批拒绝
    let ov = vec![mk("T1-008", 1500)];
    let r4 = eng.apply(Command::ClipsInsert {
        to_track: "T1".into(), clips: ov, request_id: None,
    }, agent(), ApplyOpts::default());
    assert!(matches!(r4, Err(cutforge_core::Reject::InvariantViolation(_))), "重叠整批拒");
    // 未知轨拒绝
    let r5 = eng.apply(Command::ClipsInsert {
        to_track: "T9".into(), clips: vec![mk("T9-001", 0)], request_id: None,
    }, agent(), ApplyOpts::default());
    assert!(matches!(r5, Err(cutforge_core::Reject::UnknownTrack(_))));
}

#[test]
fn clips_patch_is_single_atomic_op_for_replace() {
    let mut eng = project(base());
    let updates = vec![
        ("T1-001".to_string(), ClipPatch { text: Some("第一句改".into()), ..Default::default() }),
        ("T1-002".to_string(), ClipPatch { text: Some("第二句改".into()), ..Default::default() }),
    ];
    let r = eng.apply(Command::ClipsPatch { updates }, agent(), ApplyOpts::default()).unwrap();
    assert_eq!(r.op_ids.len(), 1, "批量替换必须单 Op");
    for id in ["T1-001", "T1-002"] {
        match eng.query(cutforge_core::Query::Clip { id: id.into() }) {
            cutforge_core::Answer::Clip(Some(c)) => assert!(c["text"].as_str().unwrap().ends_with("改"), "{id}"),
            other => panic!("意外: {other:?}"),
        }
    }
    // 原子:第二个 clipId 不存在 → 整批拒绝,第一个不得落盘
    let bad = vec![
        ("T1-001".to_string(), ClipPatch { text: Some("不该生效".into()), ..Default::default() }),
        ("T1-999".to_string(), ClipPatch { text: Some("x".into()), ..Default::default() }),
    ];
    let r2 = eng.apply(Command::ClipsPatch { updates: bad }, agent(), ApplyOpts::default());
    assert!(matches!(r2, Err(cutforge_core::Reject::UnknownClip(_))), "未知 clip 整批拒: {r2:?}");
    match eng.query(cutforge_core::Query::Clip { id: "T1-001".into() }) {
        cutforge_core::Answer::Clip(Some(c)) => assert_eq!(c["text"], json!("第一句改"), "拒绝批零落盘"),
        other => panic!("意外: {other:?}"),
    }
    // 空 patch 拒绝
    let r3 = eng.apply(Command::ClipsPatch {
        updates: vec![("T1-001".to_string(), ClipPatch::default())],
    }, agent(), ApplyOpts::default());
    assert!(matches!(r3, Err(cutforge_core::Reject::EmptyPatch(_))));
    // undo 一次还原整批
    eng.undo(agent()).unwrap();
    match eng.query(cutforge_core::Query::Clip { id: "T1-001".into() }) {
        cutforge_core::Answer::Clip(Some(c)) => assert_eq!(c["text"], json!("第一句"), "单 Op undo 整批还原"),
        other => panic!("意外: {other:?}"),
    }
}


#[test]
fn track_patch_merges_by_field() {
    let mut t: cutforge_core::model::Track = serde_json::from_value(serde_json::json!({
        "id": "V1", "kind": "video", "clips": []
    }))
    .unwrap();
    // 全字段 patch:7 项变更,指针逐一对应
    let changes = TrackPatch {
        name: Some("主画面".into()), locked: Some(true), mute: Some(false),
        solo: Some(true), hidden: Some(false), height_px: Some(240),
        color: Some("#3D7EAF".into()),
    }
    .apply_to(&mut t);
    assert_eq!(changes.len(), 7);
    let m: std::collections::BTreeMap<String, (Value, Value)> =
        changes.into_iter().map(|(p, o, n)| (p, (o, n))).collect();
    assert_eq!(m["/name"].1, serde_json::json!("主画面"));
    assert_eq!(m["/heightPx"].1, serde_json::json!(240));
    assert_eq!(m["/color"].1, serde_json::json!("#3D7EAF"));
    assert_eq!(t.height_px, Some(240));
    // 部分合并:只改 mute,name/heightPx 保持
    let changes = TrackPatch { mute: Some(true), ..Default::default() }.apply_to(&mut t);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].0, "/mute");
    assert_eq!(t.name.as_deref(), Some("主画面"), "未给出的字段不得被清掉");
    assert_eq!(t.mute, Some(true));
    // 同值 patch 不产变更
    let changes = TrackPatch { mute: Some(true), ..Default::default() }.apply_to(&mut t);
    assert!(changes.is_empty(), "同值 track 字段不得计入变更");
    assert!(TrackPatch::default().is_empty());
    assert!(!TrackPatch { color: Some("#111111".into()), ..Default::default() }.is_empty());
}
