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
        let r = eng.apply(Command::CompoundCreate { clip_ids: vec!["V1-001".into()], to_track: "V1".into(), start_ms: 0, request_id: None }, agent(), ApplyOpts::default());
        assert!(matches!(r, Err(Reject::InvariantViolation(_))), "单片段必须拒绝: {r:?}");
        // 打包 V1-001[0,8400)+V1-002[8400,14600) → 壳 @0,总时长 14600
        let r = eng.apply(Command::CompoundCreate {
            clip_ids: vec!["V1-001".into(), "V1-002".into()],
            to_track: "V1".into(), start_ms: 0, request_id: None,
        }, agent(), ApplyOpts::default()).unwrap();
        assert_eq!(r.op_ids.len(), 1, "打包单 Op");
        let shell_id = match eng.query(Query::Timeline) {
            Answer::Timeline(tl) => {
                assert_eq!(tl.len(), 2, "V1 一个复合壳 + A1 一个音效: {tl:?}");
                tl.iter().find(|(_, s, e, _)| *s == 0 && *e == 14600).map(|(id, _, _, _)| id.clone())
                    .expect("复合壳 [0,14600) 必须在投影")
            }
            other => panic!("意外: {other:?}"),
        };
        match eng.query(Query::Clip { id: shell_id.clone() }) {
            Answer::Clip(Some(c)) => {
                assert!(c.get("src").is_none_or(|v| v.is_null()), "壳无 src");
                let inner = c["compound"]["clips"].as_array().unwrap();
                assert_eq!(inner.len(), 2);
                assert_eq!(inner[0]["startMs"], json!(0), "局部时间域: base=0");
                assert_eq!(inner[1]["startMs"], json!(8400));
                assert_eq!(inner[0]["sourceInMs"], json!(12000), "子片段属性(sourceInMs)随片保留");
            }
            other => panic!("意外: {other:?}"),
        }
        // 守卫:复合嵌套打包(深度上限两级)
        let r = eng.apply(Command::CompoundCreate { clip_ids: vec![shell_id.clone()], to_track: "V1".into(), start_ms: 0, request_id: None }, agent(), ApplyOpts::default());
        assert!(matches!(r, Err(Reject::InvariantViolation(_))), "复合再打包必须拒绝: {r:?}");
        // 解包:还原两片段,id 重分配,时间域回主时间线
        let r = eng.apply(Command::CompoundUnbind { clip_id: shell_id.clone() }, agent(), ApplyOpts::default()).unwrap();
        assert_eq!(r.op_ids.len(), 1, "解包单 Op");
        match eng.query(Query::Timeline) {
            Answer::Timeline(tl) => {
                assert!(tl.iter().any(|(_, s, e, _)| *s == 0 && *e == 8400), "解包第一段回主时间线");
                assert!(tl.iter().any(|(_, s, e, _)| *s == 8400 && *e == 14600), "时间域平移回主时间线");
            }
            other => panic!("意外: {other:?}"),
        }
        // undo 两级(解包→打包)回到夹具原状;redo 再走一遍
        eng.undo(agent()).unwrap();
        let shell_again = match eng.query(Query::Timeline) {
            Answer::Timeline(tl) => tl.iter().find(|(_, s, e, _)| *e - *s == 14600).map(|(id, _, _, _)| id.clone()).unwrap(),
            other => panic!("意外: {other:?}"),
        };
        assert_eq!(shell_again, shell_id, "undo 解包 = 复合壳回归");
        eng.undo(agent()).unwrap();
        match eng.query(Query::Timeline) {
            Answer::Timeline(tl) => {
                assert!(tl.iter().any(|(id, _, _, _)| id == "V1-001"), "undo 打包 = 原两片段回归");
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
        let r = eng.apply(Command::CompoundCreate { clip_ids: vec!["V1-001".into(), "A1-001".into()], to_track: "V1".into(), start_ms: 0, request_id: None }, agent(), ApplyOpts::default());
        assert!(matches!(r, Err(Reject::InvariantViolation(_))), "跨 kind 打包必须拒绝: {r:?}");
        // 目标轨非视频
        let r = eng.apply(Command::CompoundCreate { clip_ids: vec!["V1-001".into(), "V1-002".into()], to_track: "A1".into(), start_ms: 0, request_id: None }, agent(), ApplyOpts::default());
        assert!(matches!(r, Err(Reject::InvariantViolation(_))), "落音频轨必须拒绝: {r:?}");
        // 间隙:切出 V1-003[4000,8400) 后 trim out -1400 → [4000,7000),与 V1-002[8400,…) 有隙
        eng.apply(Command::ClipSplit { clip_id: "V1-001".into(), t_ms: 4000 }, agent(), ApplyOpts::default()).unwrap();
        eng.apply(Command::ClipTrim { clip_id: "V1-003".into(), mode: TrimMode::Trim, edge: TrimEdge::Out, delta_ms: -1400 }, agent(), ApplyOpts::default()).unwrap();
        let r = eng.apply(Command::CompoundCreate { clip_ids: vec!["V1-003".into(), "V1-002".into()], to_track: "V1".into(), start_ms: 0, request_id: None }, agent(), ApplyOpts::default());
        assert!(matches!(r, Err(Reject::InvariantViolation(_))), "选区间隙必须拒绝: {r:?}");
        assert_eq!(eng.oplog().len(), 2, "守卫拒绝不得产 Op");
    }

    /// track_split_at:单轨多点一次全切(单 Op);切点不在片段内部不切;
    /// 全部不命中 → InvariantViolation;sourceInMs/text 语义与 clip_split 同口径。
    #[test]
    fn track_split_at_multi_points_single_op() {
        let mut eng = Engine::new(sample_project()).unwrap();
        let r = eng.apply(Command::TrackSplitAt { track_id: "V1".into(), t_points: vec![4000, 2000, 4000] }, agent(), ApplyOpts::default()).unwrap();
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
        let r = eng.apply(Command::TrackSplitAt { track_id: "V1".into(), t_points: vec![4000] }, agent(), ApplyOpts::default());
        assert!(matches!(r, Err(Reject::InvariantViolation(_))), "边界点必须拒绝: {r:?}");
        // sourceInMs 平移语义(与 clip_split 同):V1-001 srcIn 12000,再切 1000 → 右段 13000
        eng.apply(Command::TrackSplitAt { track_id: "V1".into(), t_points: vec![1000] }, agent(), ApplyOpts::default()).unwrap();
        let rid = match eng.query(Query::Timeline) {
            Answer::Timeline(tl) => tl.iter().find(|(_, s, e, _)| *s == 1000 && *e == 2000).map(|(id, _, _, _)| id.clone()).unwrap(),
            other => panic!("意外: {other:?}"),
        };
        match eng.query(Query::Clip { id: rid }) {
            Answer::Clip(Some(c)) => assert_eq!(c["sourceInMs"], json!(13000), "右段 sourceIn 平移"),
            other => panic!("意外: {other:?}"),
        }
    }

    #[test]
    fn delete_insert_move_and_request_id_dedup() {
        let mut eng = Engine::new(sample_project()).unwrap();
        let r = eng.apply(Command::ClipDelete { clip_id: "A1-001".into() }, agent(), ApplyOpts::default()).unwrap();
        assert_eq!(r.rev, 1);
        let mut clip = sample_project().tracks[1].clips[0].clone();
        clip.id = "A1-002".into();
        let opts = ApplyOpts { request_id: Some("req-1".into()), ..Default::default() };
        let r2 = eng.apply(Command::ClipInsert { to_track: "A1".into(), clip, request_id: Some("req-1".into()) }, agent(), opts).unwrap();
        assert!(!r2.idempotent);
        assert_eq!(eng.rev(), 2);
        // 同 request_id 再来 → 幂等回执,rev 不动
        let clip2 = { let mut c = sample_project().tracks[1].clips[0].clone(); c.id = "A1-003".into(); c };
        let r3 = eng.apply(Command::ClipInsert { to_track: "A1".into(), clip: clip2, request_id: Some("req-1".into()) }, agent(), ApplyOpts { request_id: Some("req-1".into()), ..Default::default() }).unwrap();
        assert!(r3.idempotent);
        assert_eq!(eng.rev(), 2);
        // 跨 kind 移动拒绝
        let r4 = eng.apply(Command::ClipMove { clip_id: "A1-002".into(), new_start_ms: 0, to_track: Some("V1".into()) }, agent(), ApplyOpts::default());
        assert!(matches!(r4, Err(Reject::InvariantViolation(_))));
    }
