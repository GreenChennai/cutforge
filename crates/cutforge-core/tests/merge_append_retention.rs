//! V2-HOTFIX 回归(web-e2e A2 第 13 步外部追加丢失):TC-CORE-MERGE-012 系列。
//! 定案(serve.log 实证):丢点不在 three_way_merge——core 矩阵对工单全部三方
//! 形态均正确保留 disk 侧追加;真实病灶是 R-13② 快照优先装载无"与磁盘一致
//! 性"校验(修复落 io open.rs:头态 != 磁盘 project.json → 放弃快照优化回退
//! 全量,磁盘为权威)。本文件把 merge 侧语义矩阵永久固化,防同类场景再被误判。

use cutforge_core::merge::{MergeOutcome, three_way_merge};
use serde_json::{Value, json};

/// TC-CORE-MERGE-012 系列(V2-HOTFIX 定案固化):**单侧纯追加必须保留**——
/// BUG-08 序敏感修复与追加保留同源(重排保留 + 追加保留 = 序列 diff 两段式)。
/// 本组为 web-e2e A2 第 13 步外部追加丢失回归的 core 级最小化矩阵:
/// 实测定案(serve.log 无 sync 诊断输出):丢点不在 merge——sync_with_disk
/// 对"新开实例"必然命中 disk==synced_disk 快路径,合并从未发生;真实病灶
/// 在常驻实例不被触发 sync(修复落 io/mcp 交互层,见 V2-HOTFIX 工单报告)。
/// 本矩阵锁死 core 合并语义,防该场景再被误判。matrix:
///   P1 base==local, disk=+X        → 表 2 行整取 disk,必含 X
///   P2 base=[A], disk=[A,B,X], local=[A,B](oplog 重放形态)→ 必含 X
///   P3 e2e 真实三方:disk=A.volume 外改 + 尾追加 X → A 采 disk、X 保留
///   P4 完整 project 嵌套形态(tracks→V1→clips)同 P3
fn ids(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect()
}
fn clip(id: &str, start: u64, volume: f64) -> Value {
    json!({"id": id, "src": "m.mp4", "startMs": start, "durationMs": 1000, "volume": volume})
}

#[test]
fn tc_core_merge_012_p1_base_eq_local_disk_append_kept() {
    let base = json!([clip("A", 0, 1.0), clip("B", 1000, 1.0)]);
    let disk = json!([
        clip("A", 0, 1.0),
        clip("B", 1000, 1.0),
        clip("X", 60000, 1.0)
    ]);
    let local = base.clone();
    match three_way_merge(&base, &disk, &local) {
        MergeOutcome::Merged(v) => assert!(ids(&v).contains(&"X".to_string()), "P1 丢追加: {v}"),
        MergeOutcome::Conflicts(c) => panic!("P1 不得冲突: {c:?}"),
    }
}

#[test]
fn tc_core_merge_012_p2_replay_shape_append_kept() {
    let base = json!([clip("A", 0, 1.0)]);
    let disk = json!([
        clip("A", 0, 1.0),
        clip("B", 1000, 1.0),
        clip("X", 60000, 1.0)
    ]);
    let local = json!([clip("A", 0, 1.0), clip("B", 1000, 1.0)]);
    match three_way_merge(&base, &disk, &local) {
        MergeOutcome::Merged(v) => assert!(ids(&v).contains(&"X".to_string()), "P2 丢追加: {v}"),
        MergeOutcome::Conflicts(c) => panic!("P2 不得冲突: {c:?}"),
    }
}

#[test]
fn tc_core_merge_012_p3_e2e_real_shape_modify_and_append() {
    let base = json!([clip("A", 0, 1.0), clip("B", 1000, 1.0)]);
    let disk = json!([
        clip("A", 0, 0.66),
        clip("B", 1000, 1.0),
        clip("X", 60000, 1.0)
    ]);
    let local = json!([clip("A", 0, 1.0), clip("B", 1000, 1.0)]);
    match three_way_merge(&base, &disk, &local) {
        MergeOutcome::Merged(v) => {
            assert!(ids(&v).contains(&"X".to_string()), "P3 丢追加: {v}");
            let a = v
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == "A")
                .unwrap();
            assert_eq!(a["volume"], json!(0.66), "A.volume 必须采 disk 外改");
        }
        MergeOutcome::Conflicts(c) => panic!("P3 不得冲突: {c:?}"),
    }
}

#[test]
fn tc_core_merge_012_p4_nested_project_shape() {
    let mk = |v1: Value| {
        json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "t", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": v1}]
        })
    };
    let base = mk(json!([clip("A", 0, 1.0), clip("B", 1000, 1.0)]));
    let disk = mk(json!([
        clip("A", 0, 0.66),
        clip("B", 1000, 1.0),
        clip("X", 60000, 1.0)
    ]));
    let local = mk(json!([clip("A", 0, 1.0), clip("B", 1000, 1.0)]));
    match three_way_merge(&base, &disk, &local) {
        MergeOutcome::Merged(v) => {
            let clips = &v["tracks"][0]["clips"];
            assert!(ids(clips).contains(&"X".to_string()), "P4 丢追加: {v}");
        }
        MergeOutcome::Conflicts(c) => panic!("P4 不得冲突: {c:?}"),
    }
}
