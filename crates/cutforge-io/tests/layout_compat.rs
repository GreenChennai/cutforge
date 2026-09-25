//! 多风格迭代 M1:目录契约中文化(0.5.0)——新旧布局兼容门禁。
//!
//! - 新布局(中文目录)是新建工程的唯一形态(`tests_fixture` 即按新常量搭台);
//! - 旧布局(0.4.x 英文目录)工程**原地可开可写、不自动迁移**(避免在
//!   CutFlow 并行升级期间挪动正在使用的工程)。
//! 回归样本 JSON 内容不变(talking-head),只换目录树形态。

use cutforge_core::command::{ClipPatch, Command};
use cutforge_core::oplog::Actor;
use cutforge_io::{atomic, fsutil, paths, Workspace};

fn sample() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/regression/talking-head")
}

fn copy_atomic(src: &std::path::Path, dst: &std::path::Path) {
    // 唯一落盘点纪律(M2-4):测试写盘同样走 atomic.rs
    atomic::atomic_write(dst, &std::fs::read(src).unwrap()).unwrap();
}

/// 中文目录(新契约)夹具用例:新布局可开、可写、落新目录。
#[test]
fn chinese_layout_workspace_roundtrip() {
    let root = cutforge_io::tests_fixture("layout-cn").unwrap();
    assert!(root.join(paths::PROJECT_REL).is_file(), "新工程必须落 05_时间线工程/project.json");
    assert!(!root.join(paths::LEGACY_TIMELINE).exists(), "新工程不得再造旧英文目录");

    let mut ws = Workspace::open_exclusive(&root).unwrap();
    ws.apply(
        Command::ClipUpdate {
            clip_id: "V1-001".into(),
            patch: ClipPatch { duration_ms: Some(8000), ..Default::default() },
        },
        Actor::agent("layout-cn"),
        Default::default(),
    )
    .unwrap();
    assert_eq!(ws.rev(), 1);
    let disk: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join(paths::PROJECT_REL)).unwrap(),
    )
    .unwrap();
    assert_eq!(disk["tracks"][0]["clips"][0]["durationMs"], serde_json::json!(8000));
    fsutil::cleanup(&root);
}

/// 旧布局兼容(0.4.x 工程不动盘):原地可开、可写,且不被迁移/复制出新目录。
#[test]
fn legacy_english_layout_still_opens_and_writes_in_place() {
    let root = fsutil::temp_dir("layout-legacy");
    let sample = sample();
    fsutil::ensure(&root.join(paths::LEGACY_TIMELINE)).unwrap();
    fsutil::ensure(&root.join(paths::LEGACY_CUT)).unwrap();
    copy_atomic(&sample.join("project.json"), &root.join(paths::LEGACY_PROJECT_REL));
    copy_atomic(&sample.join("wordline.json"), &root.join(paths::LEGACY_WORDLINE_REL));
    copy_atomic(&sample.join("cutlist.json"), &root.join(paths::LEGACY_CUTLIST_REL));

    let mut ws = Workspace::open_exclusive(&root).unwrap();
    ws.apply(
        Command::ClipUpdate {
            clip_id: "V1-001".into(),
            patch: ClipPatch { duration_ms: Some(7000), ..Default::default() },
        },
        Actor::user("人"),
        Default::default(),
    )
    .unwrap();

    // 写回必须原地:project.json 仍在旧目录;新目录不得被凭空造出
    assert!(root.join(paths::LEGACY_PROJECT_REL).is_file(), "旧工程写回必须原地保留");
    assert!(!root.join(paths::TIMELINE).exists(), "旧工程不得被自动迁移");
    let disk: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join(paths::LEGACY_PROJECT_REL)).unwrap(),
    )
    .unwrap();
    assert_eq!(disk["tracks"][0]["clips"][0]["durationMs"], serde_json::json!(7000));

    // 旧布局下文件级真相源(cutlist)经 record_change 也要落旧目录
    let before: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join(paths::LEGACY_CUTLIST_REL)).unwrap(),
    )
    .unwrap();
    let mut after = before.clone();
    after["cuts"][0]["action"] = serde_json::json!("review");
    ws.record_change(
        "cutlist.json",
        "/",
        before,
        after,
        cutforge_core::oplog::OpKind::Set,
        Actor::agent("layout-legacy"),
        Default::default(),
    )
    .unwrap();
    let cut: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join(paths::LEGACY_CUTLIST_REL)).unwrap(),
    )
    .unwrap();
    assert_eq!(cut["cuts"][0]["action"], serde_json::json!("review"), "cutlist 必须按旧布局原地落盘");
    fsutil::cleanup(&root);
}
