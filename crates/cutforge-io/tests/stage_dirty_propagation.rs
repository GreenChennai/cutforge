//! M3-7 门禁:与阶段缓存的衔接(计划书 4.10)。
//! 改 project.json 只标 S3 及下游脏(上游不脏);改 subtitles.ass 只标 S8(S8 护栏)。

use cutforge_io::stage::impact_for;

#[test]
fn stage_dirty_propagation() {
    // 改 IR:S3 起全部脏,S0–S2 不脏
    let imp = impact_for("05_ir/project.json").unwrap();
    assert_eq!(imp.first_dirty, "S3");
    assert_eq!(imp.dirty.len(), 9);
    assert!(!imp.dirty.iter().any(|s| ["S0", "S1", "S2"].contains(&s.as_str())), "上游不得标脏");
    assert!(imp.b8_guardrail, "手注单 clip 音频/转场场景须提示 B8 护栏(改跑 06_output/rebuild)");

    // 改字幕 ASS:仅 S8,不碰 IR 相关阶段
    let sub = impact_for("06_output/subtitles.ass").unwrap();
    assert_eq!(sub.dirty, vec!["S8".to_string()]);
    assert!(!sub.b8_guardrail);

    // 改 cutlist:S2 起级联
    let cut = impact_for("04_cut/cutlist.json").unwrap();
    assert_eq!(cut.first_dirty, "S2");
    assert_eq!(cut.dirty.len(), 10);

    // artboard 卡片:S4 起
    let card = impact_for(r"03_assets\artboard\cards\kc01.mp4").unwrap(); // Windows 路径分隔符
    assert_eq!(card.first_dirty, "S4");

    // 无关文件不标脏
    assert!(impact_for("01_materials/a.mp4").is_none());
    assert!(impact_for("_state/verify.json").is_none());
}
