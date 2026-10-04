// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
// apply 命令单测(册五 T5.4 起部分拆分自 engine::apply——行数红线 A1-3,
// 纯移动;`#[path]` 内联回 apply::tests,断言路径与语义零变化)。

use super::*;
use serde_json::json;

// ---- 册五 T5.4:compound_create / compound_unbind / track_split_at ----

/// 打包:两片段 → 复合壳(局部时间域,原片段移除,单 Op);解包 = 逆操作
/// (id 重分配,时间域平移回主时间线);undo 逐级还原。守卫:单片段/跨 kind/
/// 复合嵌套/选区间隙 各自拒绝。
#[test]
fn compound_create_unbind_roundtrip() {
    let mut eng = Engine::new(sample_project()).unwrap();
    // 守卫:单片段拒绝
    let r = eng.apply(
        Command::CompoundCreate {
            clip_ids: vec!["V1-001".into()],
            to_track: "V1".into(),
            start_ms: 0,
            request_id: None,
        },
        agent(),
        ApplyOpts::default(),
    );
    assert!(
        matches!(r, Err(Reject::InvariantViolation(_))),
        "单片段必须拒绝: {r:?}"
    );
    // 打包 V1-001[0,8400)+V1-002[8400,14600) → 壳 @0,总时长 14600
    let r = eng
        .apply(
            Command::CompoundCreate {
                clip_ids: vec!["V1-001".into(), "V1-002".into()],
                to_track: "V1".into(),
                start_ms: 0,
                request_id: None,
            },
            agent(),
            ApplyOpts::default(),
        )
        .unwrap();
    assert_eq!(r.op_ids.len(), 1, "打包单 Op");
    let shell_id = match eng.query(Query::Timeline) {
        Answer::Timeline(tl) => {
            assert_eq!(tl.len(), 2, "V1 一个复合壳 + A1 一个音效: {tl:?}");
            tl.iter()
                .find(|(_, s, e, _)| *s == 0 && *e == 14600)
                .map(|(id, _, _, _)| id.clone())
                .expect("复合壳 [0,14600) 必须在投影")
        }
        other => panic!("意外: {other:?}"),
    };
    match eng.query(Query::Clip {
        id: shell_id.clone(),
    }) {
        Answer::Clip(Some(c)) => {
            assert!(c.get("src").is_none_or(|v| v.is_null()), "壳无 src");
            let inner = c["compound"]["clips"].as_array().unwrap();
            assert_eq!(inner.len(), 2);
            assert_eq!(inner[0]["startMs"], json!(0), "局部时间域: base=0");
            assert_eq!(inner[1]["startMs"], json!(8400));
            assert_eq!(
                inner[0]["sourceInMs"],
                json!(12000),
                "子片段属性(sourceInMs)随片保留"
            );
        }
        other => panic!("意外: {other:?}"),
    }
    // 守卫:复合嵌套打包(深度上限两级)
    let r = eng.apply(
        Command::CompoundCreate {
            clip_ids: vec![shell_id.clone()],
            to_track: "V1".into(),
            start_ms: 0,
            request_id: None,
        },
        agent(),
        ApplyOpts::default(),
    );
    assert!(
        matches!(r, Err(Reject::InvariantViolation(_))),
        "复合再打包必须拒绝: {r:?}"
    );
    // 解包:还原两片段,id 重分配,时间域回主时间线
    let r = eng
        .apply(
            Command::CompoundUnbind {
                clip_id: shell_id.clone(),
            },
            agent(),
            ApplyOpts::default(),
        )
        .unwrap();
    assert_eq!(r.op_ids.len(), 1, "解包单 Op");
    match eng.query(Query::Timeline) {
        Answer::Timeline(tl) => {
            assert!(
                tl.iter().any(|(_, s, e, _)| *s == 0 && *e == 8400),
                "解包第一段回主时间线"
            );
            assert!(
                tl.iter().any(|(_, s, e, _)| *s == 8400 && *e == 14600),
                "时间域平移回主时间线"
            );
        }
        other => panic!("意外: {other:?}"),
    }
    // undo 两级(解包→打包)回到夹具原状;redo 再走一遍
    eng.undo(agent()).unwrap();
    let shell_again = match eng.query(Query::Timeline) {
        Answer::Timeline(tl) => tl
            .iter()
            .find(|(_, s, e, _)| *e - *s == 14600)
            .map(|(id, _, _, _)| id.clone())
            .unwrap(),
        other => panic!("意外: {other:?}"),
    };
    assert_eq!(shell_again, shell_id, "undo 解包 = 复合壳回归");
    eng.undo(agent()).unwrap();
    match eng.query(Query::Timeline) {
        Answer::Timeline(tl) => {
            assert!(
                tl.iter().any(|(id, _, _, _)| id == "V1-001"),
                "undo 打包 = 原两片段回归"
            );
            assert!(tl.iter().any(|(id, _, _, _)| id == "V1-002"));
        }
        other => panic!("意外: {other:?}"),
    }
}

/// 打包守卫:选区含间隙 / 跨 kind / 目标轨非视频 各自拒绝(状态零变化)。
#[test]
fn compound_create_guards() {
    let mut eng = Engine::new(sample_project()).unwrap();
    // 间隙:V1-001[0,8400) 与 A1-001[8400,8800) 不重叠但 A1 是音频轨 → 跨 kind 先拒
    let r = eng.apply(
        Command::CompoundCreate {
            clip_ids: vec!["V1-001".into(), "A1-001".into()],
            to_track: "V1".into(),
            start_ms: 0,
            request_id: None,
        },
        agent(),
        ApplyOpts::default(),
    );
    assert!(
        matches!(r, Err(Reject::InvariantViolation(_))),
        "跨 kind 打包必须拒绝: {r:?}"
    );
    // 目标轨非视频
    let r = eng.apply(
        Command::CompoundCreate {
            clip_ids: vec!["V1-001".into(), "V1-002".into()],
            to_track: "A1".into(),
            start_ms: 0,
            request_id: None,
        },
        agent(),
        ApplyOpts::default(),
    );
    assert!(
        matches!(r, Err(Reject::InvariantViolation(_))),
        "落音频轨必须拒绝: {r:?}"
    );
    // 间隙:切出 V1-003[4000,8400) 后 trim out -1400 → [4000,7000),与 V1-002[8400,…) 有隙
    eng.apply(
        Command::ClipSplit {
            clip_id: "V1-001".into(),
            t_ms: 4000,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    eng.apply(
        Command::ClipTrim {
            clip_id: "V1-003".into(),
            mode: TrimMode::Trim,
            edge: TrimEdge::Out,
            delta_ms: -1400,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let r = eng.apply(
        Command::CompoundCreate {
            clip_ids: vec!["V1-003".into(), "V1-002".into()],
            to_track: "V1".into(),
            start_ms: 0,
            request_id: None,
        },
        agent(),
        ApplyOpts::default(),
    );
    assert!(
        matches!(r, Err(Reject::InvariantViolation(_))),
        "选区间隙必须拒绝: {r:?}"
    );
    assert_eq!(eng.oplog().len(), 2, "守卫拒绝不得产 Op");
}

/// track_split_at:单轨多点一次全切(单 Op);切点不在片段内部不切;
/// 全部不命中 → InvariantViolation;sourceInMs/text 语义与 clip_split 同口径。
#[test]
fn track_split_at_multi_points_single_op() {
    let mut eng = Engine::new(sample_project()).unwrap();
    let r = eng
        .apply(
            Command::TrackSplitAt {
                track_id: "V1".into(),
                t_points: vec![4000, 2000, 4000],
            },
            agent(),
            ApplyOpts::default(),
        )
        .unwrap();
    assert_eq!(r.op_ids.len(), 1, "多点分割单 Op");
    match eng.query(Query::Timeline) {
        Answer::Timeline(tl) => {
            let v1: Vec<_> = tl.iter().filter(|(_, s, _, _)| *s < 8400).collect();
            assert_eq!(v1.len(), 3, "两点(去重后 2000/4000)切两刀 → 3 段: {v1:?}");
            assert!(v1.iter().any(|(_, s, e, _)| *s == 0 && *e == 2000));
            assert!(v1.iter().any(|(_, s, e, _)| *s == 2000 && *e == 4000));
            assert!(v1.iter().any(|(_, s, e, _)| *s == 4000 && *e == 8400));
        }
        other => panic!("意外: {other:?}"),
    }
    // 切点在片段边界/轨外 → 无命中拒绝
    let r = eng.apply(
        Command::TrackSplitAt {
            track_id: "V1".into(),
            t_points: vec![4000],
        },
        agent(),
        ApplyOpts::default(),
    );
    assert!(
        matches!(r, Err(Reject::InvariantViolation(_))),
        "边界点必须拒绝: {r:?}"
    );
    // sourceInMs 平移语义(与 clip_split 同):V1-001 srcIn 12000,再切 1000 → 右段 13000
    eng.apply(
        Command::TrackSplitAt {
            track_id: "V1".into(),
            t_points: vec![1000],
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let rid = match eng.query(Query::Timeline) {
        Answer::Timeline(tl) => tl
            .iter()
            .find(|(_, s, e, _)| *s == 1000 && *e == 2000)
            .map(|(id, _, _, _)| id.clone())
            .unwrap(),
        other => panic!("意外: {other:?}"),
    };
    match eng.query(Query::Clip { id: rid }) {
        Answer::Clip(Some(c)) => assert_eq!(c["sourceInMs"], json!(13000), "右段 sourceIn 平移"),
        other => panic!("意外: {other:?}"),
    }
}

// ---- 既有内联单测(纯移动自 apply.rs——行数红线 A1-3,include 同模块零语义变化) ----

#[test]
fn empty_patch_rejected() {
    let mut eng = Engine::new(sample_project()).unwrap();
    let r = eng.apply(
        Command::ClipUpdate {
            clip_id: "V1-001".into(),
            patch: ClipPatch::default(),
        },
        agent(),
        ApplyOpts::default(),
    );
    assert!(matches!(r, Err(Reject::EmptyPatch(_))));
}

#[test]
fn duplicate_clip_id_rejected() {
    let mut eng = Engine::new(sample_project()).unwrap();
    let clip = sample_project().tracks[0].clips[0].clone();
    let r = eng.apply(
        Command::ClipInsert {
            to_track: "V1".into(),
            clip,
            request_id: None,
        },
        agent(),
        ApplyOpts::default(),
    );
    assert!(matches!(r, Err(Reject::DuplicateClipId(_))));
}

#[test]
fn caused_by_and_summary_recorded() {
    let mut eng = Engine::new(sample_project()).unwrap();
    eng.apply(
        Command::ClipUpdate {
            clip_id: "V1-001".into(),
            patch: patch(8000),
        },
        agent(),
        ApplyOpts {
            caused_by: vec!["n-0001".into()],
            summary: Some("按标注缩短".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let op = &eng.oplog().ops()[0];
    assert_eq!(op.caused_by.as_deref(), Some(&["n-0001".to_string()][..]));
    assert_eq!(op.summary, "按标注缩短");
}

// ---- V2-W1 内核数据正确性(审查报告 v2 §4;红-绿循环先行测试) ----

/// 构造最小工程的引擎(clip id 必须匹配 schema 模式 ^[VATX][0-9]+-[0-9]{3}$)。
fn proj_engine(tracks: serde_json::Value) -> Engine {
    let v = json!({
        "version": 1, "schemaVersion": "2.0.0", "slug": "tc-w1", "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "tracks": tracks
    });
    Engine::new(crate::model::Project::from_value(&v).unwrap()).unwrap()
}

fn clip_of(eng: &Engine, id: &str) -> serde_json::Value {
    match eng.query(Query::Clip { id: id.into() }) {
        Answer::Clip(Some(c)) => c,
        other => panic!("clip {id} 不存在: {other:?}"),
    }
}

/// TC-CORE-SPLIT-001(BUG-01):speed=2.0 片段中点切分 → 右段 sourceIn
/// = 原入点 + 2×(切点-片段起点)。1:1 平移(现状)会得 12000,正确值 14000。
#[test]
fn tc_core_split_001_speed_split_right_source_in_uses_integral() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 4000,
             "sourceInMs": 10000, "speed": 2.0}
        ]}
    ]));
    eng.apply(
        Command::ClipSplit {
            clip_id: "V1-001".into(),
            t_ms: 2000,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let right = clip_of(&eng, "V1-002");
    assert_eq!(
        right["sourceInMs"],
        json!(14000),
        "右段 sourceIn 必须走 speed 积分换算(2×2000),不得 1:1 平移: {right}"
    );
    assert_eq!(right["startMs"], json!(2000));
    assert_eq!(right["durationMs"], json!(2000));
    // 左段入点不变、右段源窗总长守恒:14000 + 2×2000 = 原窗终点 18000
    assert_eq!(clip_of(&eng, "V1-001")["sourceInMs"], json!(10000));
}

/// TC-CORE-SPLIT-002(BUG-01):reverse=true 切分 → 右段持源窗口头部(入点
/// 不变),左段持窗口尾部(入点 = si + D - R(o));拼接播放 = 原倒放窗口。
#[test]
fn tc_core_split_002_reverse_split_window_direction() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 4000,
             "sourceInMs": 10000, "reverse": true}
        ]}
    ]));
    eng.apply(
        Command::ClipSplit {
            clip_id: "V1-001".into(),
            t_ms: 1000,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let right = clip_of(&eng, "V1-002");
    let left = clip_of(&eng, "V1-001");
    assert_eq!(
        right["sourceInMs"],
        json!(10000),
        "倒放右段持窗口头部,入点不变: {right}"
    );
    assert_eq!(
        left["sourceInMs"],
        json!(13000),
        "倒放左段持窗口尾部: 10000+(4000-1000): {left}"
    );
    assert_eq!(right["reverse"], json!(true));
    assert_eq!(left["reverse"], json!(true));
}

/// TC-CORE-SPLIT-003(BUG-01):带 speedCurve 片段切分 → 右段 sourceIn 与
/// speed_segments 分段积分解析解对拍:R(1500) = 1000×1.25 + 500×2.0 = 2250。
#[test]
fn tc_core_split_003_speed_curve_analytic_agreement() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000,
             "sourceInMs": 5000,
             "speedCurve": [{"atMs": 0, "speed": 0.5}, {"atMs": 1000, "speed": 2.0}]}
        ]}
    ]));
    eng.apply(
        Command::ClipSplit {
            clip_id: "V1-001".into(),
            t_ms: 1500,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let right = clip_of(&eng, "V1-002");
    assert_eq!(
        right["sourceInMs"],
        json!(7250),
        "解析解 5000+2250: {right}"
    );
    // 右段自身积分 = 500×2.0 = 1000 → 右段源窗 [7250, 8250) = 原窗尾部(守恒)
    let c: crate::model::Clip = serde_json::from_value(right.clone()).unwrap();
    let read = crate::model::source_read_ms(&c);
    assert!(
        (read - 1000.0).abs() < 1e-9,
        "右段源读时长必须与原曲线一致: {read}"
    );
    assert_eq!(right["startMs"], json!(1500));
    assert_eq!(right["durationMs"], json!(500));
}

/// TC-CORE-SPLIT-010(BUG-02):跨切点关键帧 → 左段留 t<切点,右段全部
/// rebase 为右段局部坐标(切点 2000:左留 @0/@1500,右 @2500/@3500→@500/@1500)。
#[test]
fn tc_core_split_010_keyframes_rebase_to_local_domain() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 4000,
             "keyframes": [
                {"property": "opacity", "timeMs": 0, "value": 0.0},
                {"property": "opacity", "timeMs": 1500, "value": 0.5},
                {"property": "opacity", "timeMs": 2500, "value": 0.8},
                {"property": "opacity", "timeMs": 3500, "value": 1.0}
             ]}
        ]}
    ]));
    eng.apply(
        Command::ClipSplit {
            clip_id: "V1-001".into(),
            t_ms: 2000,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let left = clip_of(&eng, "V1-001");
    let right = clip_of(&eng, "V1-002");
    let lt: Vec<u64> = left["keyframes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["timeMs"].as_u64().unwrap())
        .collect();
    let rt: Vec<u64> = right["keyframes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["timeMs"].as_u64().unwrap())
        .collect();
    assert_eq!(lt, vec![0, 1500], "左段只留切点前关键帧: {left}");
    assert_eq!(
        rt,
        vec![500, 1500],
        "右段关键帧必须 rebase 为局部坐标: {right}"
    );
}

/// TC-CORE-SPLIT-011(BUG-02):fade_in+fade_out 片段切分 → fade_in 仅左、
/// fade_out 仅右(防切点处左段出淡 + 右段入淡双重生效)。
#[test]
fn tc_core_split_011_fade_ownership_left_in_right_out() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 4000,
             "fade": {"inMs": 300.0, "outMs": 500.0}}
        ]}
    ]));
    eng.apply(
        Command::ClipSplit {
            clip_id: "V1-001".into(),
            t_ms: 2000,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let left = clip_of(&eng, "V1-001");
    let right = clip_of(&eng, "V1-002");
    assert_eq!(
        left["fade"]["inMs"].as_f64(),
        Some(300.0),
        "左段保入场淡: {left}"
    );
    assert_eq!(
        left["fade"]["outMs"].as_f64(),
        Some(0.0),
        "左段不得携带原出淡(原出淡在原片尾=右段尾): {left}"
    );
    assert_eq!(
        right["fade"]["inMs"].as_f64(),
        Some(0.0),
        "右段不得重复入场淡: {right}"
    );
    assert_eq!(
        right["fade"]["outMs"].as_f64(),
        Some(500.0),
        "出淡归属右段: {right}"
    );
}

/// TC-CORE-SPLIT-012(BUG-02):右段不携带 transition_in(防切点双重转场)。
#[test]
fn tc_core_split_012_right_segment_drops_transition() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 4000,
             "transition": {"type": "fade", "durMs": 300}}
        ]}
    ]));
    eng.apply(
        Command::ClipSplit {
            clip_id: "V1-001".into(),
            t_ms: 2000,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let left = clip_of(&eng, "V1-001");
    let right = clip_of(&eng, "V1-002");
    assert!(
        left.get("transition").is_some_and(|v| !v.is_null()),
        "转场只留左段: {left}"
    );
    assert!(
        right.get("transition").is_none_or(|v| v.is_null()),
        "右段不得携带转场(双重转场): {right}"
    );
}

/// TC-CORE-ROLL-001(BUG-03):roll 使 sourceIn 变负 → 拒绝式守卫报错、
/// 工程零变更(快照回滚)、不产 Op 不升 rev。现状:负 i128 as u64 回绕成
/// 天文数字并成功落 Op。
#[test]
fn tc_core_roll_001_negative_source_in_rejected_no_mutation() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 1000,
             "sourceInMs": 5000},
            {"id": "V1-002", "src": "a.mp4", "startMs": 1000, "durationMs": 1000,
             "sourceInMs": 100}
        ]}
    ]));
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
        clip_of(&eng, "V1-002")["sourceInMs"],
        json!(100),
        "拒绝后工程零变更"
    );
    assert_eq!(eng.oplog().len(), 0, "拒绝不得产生 Op");
    assert_eq!(eng.rev(), 0, "拒绝不得升 rev");
}

/// TC-CORE-SLIDE-001(BUG-03):slide 联动右邻 sourceIn 变负 → 同 ROLL-001 拒绝。
#[test]
fn tc_core_slide_001_negative_neighbor_source_in_rejected() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 1000,
             "sourceInMs": 5000},
            {"id": "V1-002", "src": "a.mp4", "startMs": 1000, "durationMs": 2000,
             "sourceInMs": 100},
            {"id": "V1-003", "src": "a.mp4", "startMs": 3000, "durationMs": 1000,
             "sourceInMs": 100}
        ]}
    ]));
    let r = eng.apply(
        Command::ClipTrim {
            clip_id: "V1-002".into(),
            mode: TrimMode::Slide,
            edge: TrimEdge::In,
            delta_ms: -950,
        },
        agent(),
        ApplyOpts::default(),
    );
    match r {
        Err(Reject::InvariantViolation(m)) => assert!(m.contains("素材入点"), "{m}"),
        other => panic!("slide 右邻负 sourceIn 必须拒绝: {other:?}"),
    }
    assert_eq!(
        clip_of(&eng, "V1-003")["sourceInMs"],
        json!(100),
        "拒绝后工程零变更"
    );
    assert_eq!(eng.oplog().len(), 0);
}

/// TC-TCHEMA-001(BUG-03 schema 层):sourceInMs 天文数字(u64 回绕产物)
/// 必须被 schema maximum(86400000=24h)拒绝,不得落盘。
#[test]
fn tc_tchema_001_source_in_schema_upper_bound() {
    let v = json!({
        "version": 1, "schemaVersion": "2.0.0", "slug": "tchema", "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "tracks": [
            {"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 1000,
                 "sourceInMs": 18446744073709550616u64}
            ]}
        ]
    });
    let errs = crate::model::Project::from_value(&v).unwrap_err();
    assert!(
        errs.iter().any(|e| e.contains("sourceInMs")),
        "schema 必须拒绝天文数字 sourceInMs: {errs:?}"
    );
    // 合法值回归:86400000 恰在上界内必须通过
    let ok = json!({
        "version": 1, "schemaVersion": "2.0.0", "slug": "tchema", "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "tracks": [
            {"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 1000,
                 "sourceInMs": 86400000u64}
            ]}
        ]
    });
    assert!(
        crate::model::Project::from_value(&ok).is_ok(),
        "上界内合法值不得误拒"
    );
}

/// TC-CORE-MERGE-001(BUG-04):异 src 片段即便边界贴合也不得焊成一段。
#[test]
fn tc_core_merge_001_different_source_rejected() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 1000,
             "sourceInMs": 0},
            {"id": "V1-002", "src": "b.mp4", "startMs": 1000, "durationMs": 1000,
             "sourceInMs": 0}
        ]}
    ]));
    let r = eng.apply(
        Command::ClipMerge {
            left_id: "V1-001".into(),
            right_id: "V1-002".into(),
        },
        agent(),
        ApplyOpts::default(),
    );
    match r {
        Err(Reject::InvariantViolation(m)) => assert!(m.contains("MergeDifferentSource"), "{m}"),
        other => panic!("异源合并必须拒绝: {other:?}"),
    }
    assert_eq!(eng.oplog().len(), 0);
}

/// TC-CORE-MERGE-002(BUG-04):同 src 但 sourceIn 不连续(跳变)→ 拒绝。
#[test]
fn tc_core_merge_002_source_gap_rejected() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 1000,
             "sourceInMs": 0},
            {"id": "V1-002", "src": "a.mp4", "startMs": 1000, "durationMs": 1000,
             "sourceInMs": 5000}
        ]}
    ]));
    let r = eng.apply(
        Command::ClipMerge {
            left_id: "V1-001".into(),
            right_id: "V1-002".into(),
        },
        agent(),
        ApplyOpts::default(),
    );
    match r {
        Err(Reject::InvariantViolation(m)) => assert!(m.contains("MergeNotContiguous"), "{m}"),
        other => panic!("sourceIn 不连续合并必须拒绝: {other:?}"),
    }
    assert_eq!(eng.oplog().len(), 0);
}

/// TC-CORE-MERGE-003(BUG-04):split 后立即 merge 应还原为与原始**逐字节
/// 相等**的 Clip(往返不变量;配 BUG-02 的归属规则与 merge 并回实现)。
#[test]
fn tc_core_merge_003_split_merge_roundtrip_byte_equal() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 4000,
             "sourceInMs": 1000, "text": "标题",
             "fade": {"inMs": 300.0, "outMs": 500.0},
             "transition": {"type": "fade", "durMs": 300},
             "keyframes": [
                {"property": "opacity", "timeMs": 0, "value": 0.0},
                {"property": "opacity", "timeMs": 4000, "value": 1.0}
             ]}
        ]}
    ]));
    let orig = clip_of(&eng, "V1-001");
    eng.apply(
        Command::ClipSplit {
            clip_id: "V1-001".into(),
            t_ms: 2000,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    eng.apply(
        Command::ClipMerge {
            left_id: "V1-001".into(),
            right_id: "V1-002".into(),
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let merged = clip_of(&eng, "V1-001");
    assert_eq!(
        merged, orig,
        "split→merge 必须逐字节还原原 Clip: {merged} vs {orig}"
    );
}

/// TC-CORE-MOVE-001(BUG-05):跨轨移动保 id——移动是位置变更不是身份变更,
/// 旧 target(clip id)仍须解析到该片段。
#[test]
fn tc_core_move_001_cross_track_keeps_clip_id() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 1000}
        ]},
        {"id": "V2", "kind": "video", "clips": []}
    ]));
    eng.apply(
        Command::ClipMove {
            clip_id: "V1-001".into(),
            new_start_ms: 5000,
            to_track: Some("V2".into()),
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let c = clip_of(&eng, "V1-001");
    assert_eq!(c["startMs"], json!(5000));
    match eng.query(Query::Timeline) {
        Answer::Timeline(tl) => assert!(
            tl.iter()
                .any(|(id, s, e, t)| id == "V1-001" && *s == 5000 && *e == 6000 && t == "V2"),
            "移动后 id 不变且落 V2 轨: {tl:?}"
        ),
        other => panic!("意外: {other:?}"),
    }
    // 旧 target 仍解析:按原 id 继续 update 必须成功
    eng.apply(
        Command::ClipUpdate {
            clip_id: "V1-001".into(),
            patch: crate::command::ClipPatch {
                volume: Some(0.7),
                ..Default::default()
            },
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    assert_eq!(clip_of(&eng, "V1-001")["volume"], json!(0.7));
}

/// TC-CORE-MOVE-002(BUG-05):移动后目标轨 clips 按 startMs 严格升序
/// (轨道有序不变量;现状同轨移动原位改 start 破坏数组有序)。
#[test]
fn tc_core_move_002_track_stays_sorted_by_start() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 1000},
            {"id": "V1-002", "src": "a.mp4", "startMs": 2000, "durationMs": 1000}
        ]}
    ]));
    // 同轨后移:V1-001 → 3000(无重叠;数组序必须重排为 start 升序)
    eng.apply(
        Command::ClipMove {
            clip_id: "V1-001".into(),
            new_start_ms: 3000,
            to_track: None,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let starts: Vec<u64> = eng.project().tracks[0]
        .clips
        .iter()
        .map(|c| c.start_ms)
        .collect();
    assert_eq!(
        starts,
        vec![2000, 3000],
        "同轨移动后数组必须按 startMs 升序: {starts:?}"
    );
    // 跨轨移动:目标轨二分插入保有序
    eng.apply(
        Command::TrackAdd {
            kind: crate::model::TrackKind::Video,
            request_id: None,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    eng.apply(
        Command::ClipMove {
            clip_id: "V1-001".into(),
            new_start_ms: 500,
            to_track: Some("V2".into()),
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let v2: Vec<u64> = eng.project().tracks[1]
        .clips
        .iter()
        .map(|c| c.start_ms)
        .collect();
    assert!(
        v2.windows(2).all(|w| w[0] < w[1]),
        "跨轨插入后目标轨必须有序: {v2:?}"
    );
}

/// TC-CORE-INSERT-001(BUG-05 补口):向已排序轨道**早位置**插入(即 text_add
/// 的真实场景:V1 已有 @5000 片段,text_add 落 @0)→ 数组必须保持 startMs 升序,
/// 不得 push 尾部破坏有序不变量(该路径曾触发 debug_assert panic,tool_parity 金样 DRIFT)。
#[test]
fn tc_core_insert_001_early_insert_keeps_track_sorted() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 5000, "durationMs": 1000}
        ]}
    ]));
    let mut clip = clip_of(&eng, "V1-001").clone();
    clip["id"] = json!("V1-002");
    clip["startMs"] = json!(0);
    let clip: crate::model::Clip = serde_json::from_value(clip).unwrap();
    eng.apply(
        Command::ClipInsert {
            to_track: "V1".into(),
            clip,
            request_id: None,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let starts: Vec<u64> = eng.project().tracks[0]
        .clips
        .iter()
        .map(|c| c.start_ms)
        .collect();
    assert_eq!(
        starts,
        vec![0, 5000],
        "早位置插入后轨道必须升序: {starts:?}"
    );
    assert_eq!(clip_of(&eng, "V1-002")["startMs"], json!(0));
    // undo 逐字节还原数组
    eng.undo(agent()).unwrap();
    let starts: Vec<u64> = eng.project().tracks[0]
        .clips
        .iter()
        .map(|c| c.start_ms)
        .collect();
    assert_eq!(starts, vec![5000], "undo 后数组还原: {starts:?}");
}

/// TC-CORE-INSERT-002(BUG-05 补口):批量插入**乱序批次** → 落轨数组仍升序
/// (extend 尾部追加会破坏有序不变量)。
#[test]
fn tc_core_insert_002_batch_insert_unsorted_batch_sorted_array() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 6000, "durationMs": 1000}
        ]}
    ]));
    let mk = |id: &str, start: u64| -> crate::model::Clip {
        serde_json::from_value(json!({
            "id": id, "src": "a.mp4", "startMs": start, "durationMs": 1000
        }))
        .unwrap()
    };
    eng.apply(
        Command::ClipsInsert {
            to_track: "V1".into(),
            clips: vec![mk("V1-002", 4000), mk("V1-003", 1000), mk("V1-004", 2500)],
            request_id: None,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let seq: Vec<u64> = eng.project().tracks[0]
        .clips
        .iter()
        .map(|c| c.start_ms)
        .collect();
    assert_eq!(
        seq,
        vec![1000, 2500, 4000, 6000],
        "乱序批次插入后必须按 startMs 升序: {seq:?}"
    );
}

/// TC-CORE-COMPOUND-001(BUG-05 补口):compound_create 的壳落在早位置、
/// unbind 解包回主时间线,两步后轨道数组都必须保持升序。
#[test]
fn tc_core_compound_001_shell_early_insert_and_unbind_sorted() {
    let mut eng = proj_engine(json!([
        {"id": "V1", "kind": "video", "clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 6000, "durationMs": 1000},
            {"id": "V1-002", "src": "a.mp4", "startMs": 7000, "durationMs": 1000}
        ]}
    ]));
    eng.apply(
        Command::CompoundCreate {
            clip_ids: vec!["V1-001".into(), "V1-002".into()],
            to_track: "V1".into(),
            start_ms: 1000,
            request_id: None,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let starts: Vec<u64> = eng.project().tracks[0]
        .clips
        .iter()
        .map(|c| c.start_ms)
        .collect();
    assert_eq!(starts, vec![1000], "壳早位置打包后轨道必须升序: {starts:?}");
    let shell = eng.project().tracks[0].clips[0].id.clone();
    eng.apply(
        Command::CompoundUnbind { clip_id: shell },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let starts: Vec<u64> = eng.project().tracks[0]
        .clips
        .iter()
        .map(|c| c.start_ms)
        .collect();
    // unbind 语义:局部时间域平移回 shell.start_ms 起排(1000+0/1000)→ [1000,2000]
    assert_eq!(
        starts,
        vec![1000, 2000],
        "解包后轨道必须保持升序: {starts:?}"
    );
}

// ---- CI 回归(gate 37225964239 / web-e2e M10):clip_update 跨位 startMs ----

/// TC-CORE-UPDATE-001:clip_update 把 startMs 改到**跨兄弟位置**(V1-003:
/// 2000→8400,越过 V1-002@4000)→ 命令必须成功且数组重排保持升序。
/// 现状(修前):原地更新破坏有序不变量 → debug_assert panic → serve 请求
/// 线程崩 → web 端 rev 永久卡死(M10 全红根因)。并锁定 rev 分配完整性:
/// receipt.rev == engine.rev == oplog 末条 Op.rev == 前值+1。
#[test]
fn tc_core_update_001_start_patch_cross_position_keeps_sorted() {
    let mut eng = Engine::new(sample_cross_project()).unwrap();
    eng.apply(
        Command::ClipSplit {
            clip_id: "V1-001".into(),
            t_ms: 2000,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let rev1 = eng.rev();
    let r = eng
        .apply(
            Command::ClipUpdate {
                clip_id: "V1-003".into(),
                patch: crate::command::ClipPatch {
                    start_ms: Some(8400),
                    ..Default::default()
                },
            },
            agent(),
            ApplyOpts::default(),
        )
        .unwrap();
    assert_eq!(r.rev, rev1 + 1, "receipt rev 必须 +1");
    assert_eq!(eng.rev(), rev1 + 1);
    let last = eng.oplog().ops().last().unwrap();
    assert_eq!(last.rev, Some(rev1 + 1), "oplog 末条 Op 必带分配的 rev");
    let seq: Vec<u64> = eng.project().tracks[0]
        .clips
        .iter()
        .map(|c| c.start_ms)
        .collect();
    assert_eq!(
        seq,
        vec![0, 4000, 8400],
        "跨位 start 更新后数组必须重排为升序: {seq:?}"
    );
    eng.undo(agent()).unwrap();
    let seq: Vec<u64> = eng.project().tracks[0]
        .clips
        .iter()
        .map(|c| c.start_ms)
        .collect();
    assert_eq!(seq, vec![0, 2000, 4000], "undo 必须还原跨位更新: {seq:?}");
    eng.redo(agent()).unwrap();
    let seq: Vec<u64> = eng.project().tracks[0]
        .clips
        .iter()
        .map(|c| c.start_ms)
        .collect();
    assert_eq!(seq, vec![0, 4000, 8400], "redo 必须复现跨位更新: {seq:?}");
}

/// TC-CORE-UPDATE-002:跨位 clip_update 的 Op(数组级 before/after)回放等价。
#[test]
fn tc_core_update_002_cross_position_update_replay_eq() {
    let base_project = sample_cross_project();
    let mut eng = Engine::new(base_project.clone()).unwrap();
    eng.apply(
        Command::ClipSplit {
            clip_id: "V1-001".into(),
            t_ms: 2000,
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    eng.apply(
        Command::ClipUpdate {
            clip_id: "V1-003".into(),
            patch: crate::command::ClipPatch {
                start_ms: Some(8400),
                ..Default::default()
            },
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let expected = eng.state_hash();
    let replayed = Engine::replay(base_project, eng.oplog().ops()).unwrap();
    assert_eq!(
        replayed.state_hash(),
        expected,
        "跨位 clip_update 的 Op 必须可回放且等价"
    );
}

/// TC-CORE-UPDATE-003(批量族同口):ClipsPatch 携带跨位 startMs → 数组重排升序。
#[test]
fn tc_core_update_003_clips_patch_start_reorders_sorted() {
    let mut eng = Engine::new(sample_cross_project()).unwrap();
    eng.apply(
        Command::ClipsPatch {
            updates: vec![(
                "V1-001".into(),
                crate::command::ClipPatch {
                    start_ms: Some(8400),
                    ..Default::default()
                },
            )],
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
    let seq: Vec<(&str, u64)> = eng.project().tracks[0]
        .clips
        .iter()
        .map(|c| (c.id.as_str(), c.start_ms))
        .collect();
    assert_eq!(
        seq,
        vec![("V1-002", 4000), ("V1-001", 8400)],
        "批量 patch 跨位 start 后必须重排: {seq:?}"
    );
}

#[test]
fn delete_insert_move_and_request_id_dedup() {
    let mut eng = Engine::new(sample_project()).unwrap();
    let r = eng
        .apply(
            Command::ClipDelete {
                clip_id: "A1-001".into(),
            },
            agent(),
            ApplyOpts::default(),
        )
        .unwrap();
    assert_eq!(r.rev, 1);
    let mut clip = sample_project().tracks[1].clips[0].clone();
    clip.id = "A1-002".into();
    let opts = ApplyOpts {
        request_id: Some("req-1".into()),
        ..Default::default()
    };
    let r2 = eng
        .apply(
            Command::ClipInsert {
                to_track: "A1".into(),
                clip,
                request_id: Some("req-1".into()),
            },
            agent(),
            opts,
        )
        .unwrap();
    assert!(!r2.idempotent);
    assert_eq!(eng.rev(), 2);
    // 同 request_id 再来 → 幂等回执,rev 不动
    let clip2 = {
        let mut c = sample_project().tracks[1].clips[0].clone();
        c.id = "A1-003".into();
        c
    };
    let r3 = eng
        .apply(
            Command::ClipInsert {
                to_track: "A1".into(),
                clip: clip2,
                request_id: Some("req-1".into()),
            },
            agent(),
            ApplyOpts {
                request_id: Some("req-1".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(r3.idempotent);
    assert_eq!(eng.rev(), 2);
    // 跨 kind 移动拒绝
    let r4 = eng.apply(
        Command::ClipMove {
            clip_id: "A1-002".into(),
            new_start_ms: 0,
            to_track: Some("V1".into()),
        },
        agent(),
        ApplyOpts::default(),
    );
    assert!(matches!(r4, Err(Reject::InvariantViolation(_))));
}

/// e2e M10 夹具同款(两段视频,中间留 2000ms 空隙供跨位更新回归)。
fn sample_cross_project() -> crate::model::Project {
    let v = serde_json::json!({
        "version": 1, "schemaVersion": "2.0.0", "slug": "tc-update", "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "tracks": [
            {"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 4000},
                {"id": "V1-002", "src": "a.mp4", "startMs": 4000, "durationMs": 4400}
            ]}
        ]
    });
    crate::model::Project::from_value(&v).unwrap()
}
