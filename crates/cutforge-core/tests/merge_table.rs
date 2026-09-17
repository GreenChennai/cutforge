//! M3-4 门禁:三路合并表(计划书 4.6)九行全覆盖 + id 数组语义。

use cutforge_core::merge::{three_way_merge, MergeOutcome};
use serde_json::{json, Value};

fn merged(v: Value) -> Value {
    match three_way_merge(&json!({}), &v, &v) {
        MergeOutcome::Merged(m) => m,
        MergeOutcome::Conflicts(c) => panic!("自合自不得冲突: {c:?}"),
    }
}

fn expect_conflict(base: &Value, disk: &Value, local: &Value, code: &str) {
    match three_way_merge(base, disk, local) {
        MergeOutcome::Conflicts(c) => {
            assert!(
                c.iter().any(|x| x.code.code() == code),
                "期望 {code},实际: {:?}",
                c.iter().map(|x| x.code.code().to_string()).collect::<Vec<_>>()
            );
        }
        MergeOutcome::Merged(_) => panic!("期望 {code} 冲突,却被静默合并(静默覆盖!): base={base} disk={disk} local={local}"),
    }
}

#[test]
fn merge_table_nine_rows_object_leaves() {
    // 行 1:双方未变 → 无操作
    assert_eq!(merged(json!({"a": 1}))["a"], json!(1));
    // 行 2:磁盘变,本地未变 → 磁盘
    match three_way_merge(&json!({"a": 1}), &json!({"a": 2}), &json!({"a": 1})) {
        MergeOutcome::Merged(v) => assert_eq!(v["a"], json!(2), "行 2 必须取磁盘"),
        _ => panic!("行 2 不得冲突"),
    }
    // 行 3:本地变,磁盘未变 → 本地
    match three_way_merge(&json!({"a": 1}), &json!({"a": 1}), &json!({"a": 3})) {
        MergeOutcome::Merged(v) => assert_eq!(v["a"], json!(3), "行 3 必须取本地"),
        _ => panic!("行 3 不得冲突"),
    }
    // 行 4:双方同值 → 幂等合并
    match three_way_merge(&json!({"a": 1}), &json!({"a": 9}), &json!({"a": 9})) {
        MergeOutcome::Merged(v) => assert_eq!(v["a"], json!(9)),
        _ => panic!("行 4 同值必须幂等合并"),
    }
    // 行 5:双方不同值 → CF-001
    expect_conflict(&json!({"a": 1}), &json!({"a": 2}), &json!({"a": 3}), "CF-001");
    // 行 6:一侧删除另一侧未动 → 删除(两个方向)
    match three_way_merge(&json!({"a": 1, "b": 2}), &json!({"a": 1}), &json!({"a": 1, "b": 2})) {
        MergeOutcome::Merged(v) => assert!(v.get("b").is_none(), "磁盘删 → 删除生效"),
        _ => panic!("行 6 不得冲突"),
    }
    match three_way_merge(&json!({"a": 1, "b": 2}), &json!({"a": 1, "b": 2}), &json!({"a": 1})) {
        MergeOutcome::Merged(v) => assert!(v.get("b").is_none(), "本地删 → 删除生效"),
        _ => panic!("行 6 不得冲突"),
    }
    // 行 7:一侧删除另一侧修改 → CF-002(两个方向)
    expect_conflict(&json!({"a": 1, "b": 2}), &json!({"a": 1}), &json!({"a": 1, "b": 3}), "CF-002");
    expect_conflict(&json!({"a": 1, "b": 2}), &json!({"a": 1, "b": 7}), &json!({"a": 1}), "CF-002");
}

#[test]
fn merge_table_row8_id_arrays_coexist() {
    let mk = |ids: &[&str]| -> Value {
        Value::Array(ids.iter().map(|i| json!({"id": i, "n": 1})).collect())
    };
    // 行 8:两侧各自插入新元素(不同 id)→ 并存,不冲突
    match three_way_merge(&mk(&["A"]), &mk(&["A", "B"]), &mk(&["A", "C"])) {
        MergeOutcome::Merged(v) => {
            let ids: Vec<&str> = v.as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
            assert_eq!(ids, vec!["A", "B", "C"], "两侧新增并存");
        }
        MergeOutcome::Conflicts(c) => panic!("行 8 不得冲突: {c:?}"),
    }
}

#[test]
fn merge_table_row9_dup_id_conflict() {
    // 行 9:两侧插入相同 id 但内容不同 → CF-003
    let base = json!([{"id": "A", "n": 1}]);
    let disk = json!([{"id": "A", "n": 1}, {"id": "X", "n": 2}]);
    let local = json!([{"id": "A", "n": 1}, {"id": "X", "n": 3}]);
    expect_conflict(&base, &disk, &local, "CF-003");
}

#[test]
fn merge_table_deep_object_field_level() {
    let base = json!({"clip": {"durationMs": 100, "volume": 1.0, "nested": {"x": 1}}});
    // 不同字段(含嵌套)自动合并
    let disk = json!({"clip": {"durationMs": 80, "volume": 1.0, "nested": {"x": 1}}});
    let local = json!({"clip": {"durationMs": 100, "volume": 0.5, "nested": {"x": 2}}});
    match three_way_merge(&base, &disk, &local) {
        MergeOutcome::Merged(v) => {
            assert_eq!(v["clip"]["durationMs"], json!(80));
            assert_eq!(v["clip"]["volume"], json!(0.5));
            assert_eq!(v["clip"]["nested"]["x"], json!(2));
        }
        MergeOutcome::Conflicts(c) => panic!("不同字段不得冲突: {c:?}"),
    }
    // 同一字段双方不同值:精确到叶路径
    let local2 = json!({"clip": {"durationMs": 120, "volume": 1.0, "nested": {"x": 1}}});
    match three_way_merge(&base, &disk, &local2) {
        MergeOutcome::Conflicts(c) => assert_eq!(c[0].pointer, "$/clip/durationMs"),
        _ => panic!("同字段异值必须冲突"),
    }
}

#[test]
fn merge_table_clip_array_mixed_ops() {
    let mk = |id: &str, start: u64, dur: u64| json!({"id": id, "startMs": start, "durationMs": dur});
    let base = json!([mk("V1-001", 0, 100), mk("V1-002", 100, 100)]);
    // 磁盘(AI):改 V1-001 时长 + 删 V1-002(本地未动) + 新增 V1-003
    let disk = json!([mk("V1-001", 0, 90), mk("V1-003", 90, 50)]);
    // 本地(用户):V1-001 未动 + 本地新增 V1-004
    let local = json!([mk("V1-001", 0, 100), mk("V1-002", 100, 100), mk("V1-004", 200, 30)]);
    match three_way_merge(&base, &disk, &local) {
        MergeOutcome::Merged(v) => {
            let ids: Vec<&str> = v.as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
            assert_eq!(ids, vec!["V1-001", "V1-003", "V1-004"]);
        }
        MergeOutcome::Conflicts(c) => panic!("不得冲突: {c:?}"),
    }
    // 磁盘删除 V1-002 而本地修改它 → CF-002
    let local3 = json!([mk("V1-001", 0, 100), mk("V1-002", 110, 100)]);
    expect_conflict(&base, &disk, &local3, "CF-002");
}
