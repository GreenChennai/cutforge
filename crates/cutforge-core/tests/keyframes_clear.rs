//! 空数组清空关键帧(册五收口补丁:schema `clip.keyframes` minItems 移除;E-FE1
//! 壳侧秒表全灭所需)。单 Op 记 before=整组/after=[],undo 还原整组,redo 复现清空;
//! `to_validated_value` 对 `keyframes: []` 必须放行(空数组合法,非缺省臆造)。

use cutforge_core::command::{ClipPatch, Command};
use cutforge_core::engine::{sample_project, ApplyOpts, Engine};
use cutforge_core::keyframes::Keyframe;
use cutforge_core::oplog::Actor;

#[test]
fn keyframes_empty_array_clears_all_and_undo_restores() {
    let mut eng = Engine::new(sample_project()).unwrap();
    let kfs = vec![
        Keyframe::new("position.x", 0, 0.5),
        Keyframe::new("opacity", 500, 1.0),
    ];
    let apply_kf = |eng: &mut Engine, kfs: Vec<Keyframe>| {
        eng.apply(
            Command::ClipUpdate {
                clip_id: "V1-001".into(),
                patch: ClipPatch { keyframes: Some(kfs), ..Default::default() },
            },
            Actor::agent("kf-clear"),
            ApplyOpts::default(),
        )
        .unwrap()
    };
    apply_kf(&mut eng, kfs.clone());
    // 空数组清空:单 Op,after = [](非 null;空数组 = 清除全部关键帧的合法形态)
    let rec = apply_kf(&mut eng, Vec::new());
    assert_eq!(rec.op_ids.len(), 1, "清空也是单 Op");
    let op = eng.oplog().ops().last().unwrap();
    assert_eq!(op.after["keyframes"], serde_json::json!([]), "after = 空数组(非 null)");
    assert_eq!(op.before["keyframes"].as_array().map(Vec::len), Some(2), "before 携带原整组");
    assert!(eng.project().to_validated_value().is_ok(), "keyframes:[] 必须过 schema(空=清除)");
    // undo 还原整组;redo 复现清空;空数组无语义违例
    eng.undo(Actor::agent("kf-clear")).unwrap();
    assert_eq!(eng.project().tracks[0].clips[0].keyframes.as_ref(), Some(&kfs), "undo 还原整组");
    eng.redo(Actor::agent("kf-clear")).unwrap();
    let clip = &eng.project().tracks[0].clips[0];
    assert_eq!(clip.keyframes.as_ref().map(Vec::len), Some(0), "redo 复现清空");
    assert!(cutforge_core::keyframes::validate_clip_keyframes(clip).is_empty(), "空数组无语义违例");
}
