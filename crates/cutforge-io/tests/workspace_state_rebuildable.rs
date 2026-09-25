//! M3-6 前置(计划书 4.3):`.cutforge/` 可安全删除并重建——
//! 删除后重新打开工程,工程内容哈希不变(仅丢失同步历史)。

use cutforge_core::command::{ClipPatch, Command};
use cutforge_core::engine::{ApplyOpts, Query};
use cutforge_core::oplog::Actor;
use cutforge_io::{fsutil, Workspace};

#[test]
fn workspace_state_rebuildable() {
    let root = cutforge_io::tests_fixture("rebuild").unwrap();
    // 先做一次修改并落盘,得到"内容基准"
    {
        let mut ws = Workspace::open(&root).unwrap();
        ws.apply(
            Command::ClipUpdate {
                clip_id: "V1-001".into(),
                patch: ClipPatch { duration_ms: Some(8000), ..Default::default() },
            },
            Actor::agent("rebuild-test"),
            ApplyOpts::default(),
        )
        .unwrap();
    }
    let disk_after_edit = std::fs::read_to_string(root.join(cutforge_io::PROJECT_REL)).unwrap();

    // 删除 .cutforge/(同步状态,不是交付物)
    std::fs::remove_dir_all(root.join(".cutforge")).unwrap();
    assert!(!root.join(".cutforge").exists());

    // 重新打开:工程内容哈希不变
    let ws = Workspace::open(&root).unwrap();
    let project_value = match ws.engine().query(Query::ProjectView) {
        cutforge_core::engine::Answer::Project(v) => v,
        _ => unreachable!(),
    };
    let disk_now: serde_json::Value = serde_json::from_str(&disk_after_edit).unwrap();
    assert_eq!(
        serde_json::to_value(&project_value).unwrap(),
        disk_now,
        "删 .cutforge 后工程内容必须保持不变(仅丢同步历史)"
    );
    assert_eq!(ws.rev(), 0, "同步历史已清零,rev 归零重建");
    fsutil::cleanup(&root);
}
