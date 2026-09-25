//! M3-7 门禁:与阶段缓存的衔接(计划书 4.10)。
//! 改 project.json 只标 S3 及下游脏(上游不脏);改 subtitles.ass 只标 S8(S8 护栏)。
//! 多风格迭代 M1:目录契约中文化——新(中文)旧(0.4.x 英文)两套相对路径都认。

use cutforge_io::paths;
use cutforge_io::stage::impact_for;

#[test]
fn stage_dirty_propagation() {
    // 改 IR:S3 起全部脏,S0–S2 不脏(新布局)
    let imp = impact_for(paths::PROJECT_REL).unwrap();
    assert_eq!(imp.first_dirty, "S3");
    assert_eq!(imp.dirty.len(), 9);
    assert!(!imp.dirty.iter().any(|s| ["S0", "S1", "S2"].contains(&s.as_str())), "上游不得标脏");
    assert!(imp.b8_guardrail, "手注单 clip 音频/转场场景须提示 B8 护栏(改跑 06_成片输出/rebuild)");

    // 改字幕 ASS:仅 S8,不碰 IR 相关阶段
    let sub = impact_for(&format!("{}/subtitles.ass", paths::OUTPUT)).unwrap();
    assert_eq!(sub.dirty, vec!["S8".to_string()]);
    assert!(!sub.b8_guardrail);

    // 改 cutlist:S2 起级联
    let cut = impact_for(paths::CUTLIST_REL).unwrap();
    assert_eq!(cut.first_dirty, "S2");
    assert_eq!(cut.dirty.len(), 10);

    // artboard 卡片:S4 起(artboard 子目录保留英文;Windows 路径分隔符也认)
    let card = impact_for("03_创作素材\\artboard\\cards\\kc01.mp4").unwrap();
    assert_eq!(card.first_dirty, "S4");

    // 无关文件不标脏
    assert!(impact_for(&format!("{}/a.mp4", paths::MATERIALS)).is_none());
    assert!(impact_for(&format!("{}/verify.json", paths::STATE)).is_none());
}

#[test]
fn stage_dirty_propagation_legacy_layout() {
    // 旧布局(0.4.x 英文目录)同口径——旧工程不被迁移,脏传播照样工作
    let imp = impact_for(paths::LEGACY_PROJECT_REL).unwrap();
    assert_eq!(imp.first_dirty, "S3");
    assert!(imp.hint.contains("05_时间线工程/rebuild.py"), "提示串一律指新目录");
    assert_eq!(impact_for("06_output/subtitles.ass").unwrap().dirty, vec!["S8".to_string()]);
    assert_eq!(impact_for(paths::LEGACY_CUTLIST_REL).unwrap().first_dirty, "S2");
    let legacy_card = impact_for("03_assets\\artboard\\cards\\kc01.mp4").unwrap();
    assert_eq!(legacy_card.first_dirty, "S4");
    assert!(impact_for("_state/verify.json").is_none());
}
