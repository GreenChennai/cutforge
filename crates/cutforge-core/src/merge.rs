// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 三路 diff 合并(计划书 4.6):以 baseRev 为共同祖先,比较 祖先/磁盘/本地 三方。
//!
//! 合并粒度:字段级(叶路径);对象按属性递归,数组整体视为一个值,
//! **唯一例外**是带稳定 `id` 的对象数组(clips/items)——按 id 逐元素套用
//! 同一张判定表,两侧都插入新元素时按各自 id 并存(表第 8 行)。
//! 结构级冲突一律交人/AI 裁决,禁止"最后写入者获胜"。

use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictCode {
    /// 同一字段两侧改成不同值。
    FieldConflict,
    /// 一侧删除、另一侧修改。
    DeleteModify,
    /// 两侧插入相同 id 但内容不同。
    DupId,
}

impl ConflictCode {
    pub fn code(&self) -> &'static str {
        match self {
            ConflictCode::FieldConflict => "CF-001",
            ConflictCode::DeleteModify => "CF-002",
            ConflictCode::DupId => "CF-003",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    pub code: ConflictCode,
    pub pointer: String,
    pub base: Option<Value>,
    pub disk: Option<Value>,
    pub local: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MergeOutcome {
    /// 可自动合并(可能两侧同值幂等合并,表第 4 行)。
    Merged(Value),
    /// 不可自动合并;冲突必须显式报错,写入 .cutforge/conflicts/(M3,IO 层)。
    Conflicts(Vec<Conflict>),
}

/// 三路合并主入口。
pub fn three_way_merge(base: &Value, disk: &Value, local: &Value) -> MergeOutcome {
    let mut conflicts = Vec::new();
    let merged = merge_node(base, disk, local, "$", &mut conflicts);
    if conflicts.is_empty() {
        MergeOutcome::Merged(merged)
    } else {
        MergeOutcome::Conflicts(conflicts)
    }
}

fn merge_node(base: &Value, disk: &Value, local: &Value, ptr: &str, out: &mut Vec<Conflict>) -> Value {
    // 表 4 行:两侧变成同值 → 幂等合并
    if disk == local {
        return disk.clone();
    }
    match (base, disk, local) {
        // 表 2 行:祖先→磁盘变,本地未变 → 采用磁盘
        (_, d, l) if is_unchanged(base, l) => d.clone(),
        // 表 3 行:祖先→本地变,磁盘未变 → 采用本地
        (_, d, l) if is_unchanged(base, d) => l.clone(),
        // 对象:字段级递归
        (Value::Object(b), Value::Object(d), Value::Object(l)) => {
            Value::Object(merge_object(b, d, l, ptr, out))
        }
        // 带 id 的对象数组:按元素 id 逐项合并
        (Value::Array(b), Value::Array(d), Value::Array(l)) if arrays_are_id_keyed(b, d, l) => {
            Value::Array(merge_id_array(b, d, l, ptr, out))
        }
        // 表 5 行:两侧改成不同值(叶/异构)→ 冲突
        _ => {
            out.push(Conflict {
                code: ConflictCode::FieldConflict,
                pointer: ptr.to_string(),
                base: Some(base.clone()),
                disk: Some(disk.clone()),
                local: Some(local.clone()),
            });
            local.clone() // 冲突时的占位;最终以裁决为准,不会被采纳
        }
    }
}

fn is_unchanged(base: &Value, side: &Value) -> bool {
    base == side
}

fn merge_object(
    b: &Map<String, Value>,
    d: &Map<String, Value>,
    l: &Map<String, Value>,
    ptr: &str,
    out: &mut Vec<Conflict>,
) -> Map<String, Value> {
    let mut result = Map::new();
    let mut keys: Vec<&String> = b.keys().chain(d.keys()).chain(l.keys()).collect();
    keys.sort();
    keys.dedup();
    for k in keys {
        let child_ptr = format!("{ptr}/{k}");
        let (bv, dv, lv) = (b.get(k), d.get(k), l.get(k));
        match (bv, dv, lv) {
            // 三方都有:递归(表 1/2/4/5 行由 merge_node 判定)
            (Some(bv), Some(dv), Some(lv)) => {
                result.insert(k.clone(), merge_node(bv, dv, lv, &child_ptr, out));
            }
            // 表 6 行:一侧删除,另一侧未动 → 采用删除(键不进结果)
            (Some(bv), None, Some(lv)) if bv == lv => {}
            (Some(bv), Some(dv), None) if bv == dv => {}
            // 表 7 行:一侧删除,另一侧修改 → 冲突
            (Some(bv), None, Some(lv)) => {
                out.push(Conflict {
                    code: ConflictCode::DeleteModify,
                    pointer: child_ptr,
                    base: Some(bv.clone()),
                    disk: None,
                    local: Some(lv.clone()),
                });
            }
            (Some(bv), Some(dv), None) => {
                out.push(Conflict {
                    code: ConflictCode::DeleteModify,
                    pointer: child_ptr,
                    base: Some(bv.clone()),
                    disk: Some(dv.clone()),
                    local: None,
                });
            }
            // 表 8 行(对象版):两侧各自新增同一键;同值并存,异值 CF-001
            (None, Some(dv), Some(lv)) => {
                if dv == lv {
                    result.insert(k.clone(), dv.clone());
                } else {
                    out.push(Conflict {
                        code: ConflictCode::FieldConflict,
                        pointer: child_ptr,
                        base: None,
                        disk: Some(dv.clone()),
                        local: Some(lv.clone()),
                    });
                }
            }
            // 单侧新增
            (None, Some(dv), None) => {
                result.insert(k.clone(), dv.clone());
            }
            (None, None, Some(lv)) => {
                result.insert(k.clone(), lv.clone());
            }
            // 两侧都删 → 删除
            (Some(_), None, None) => {}
            _ => unreachable!("对象键三元组已穷尽"),
        }
    }
    result
}

/// 数组元素是否都带字符串 `id`(clips/items 的形态)。
fn arrays_are_id_keyed(b: &[Value], d: &[Value], l: &[Value]) -> bool {
    fn ok(a: &[Value]) -> bool {
        a.is_empty() || a.iter().all(|v| v.get("id").map(Value::is_string).unwrap_or(false))
    }
    ok(b) && ok(d) && ok(l)
}

fn id_map(a: &[Value]) -> std::collections::BTreeMap<String, &Value> {
    a.iter()
        .filter_map(|v| v.get("id").and_then(Value::as_str).map(|s| (s.to_string(), v)))
        .collect()
}

fn merge_id_array(
    b: &[Value],
    d: &[Value],
    l: &[Value],
    ptr: &str,
    out: &mut Vec<Conflict>,
) -> Vec<Value> {
    let (bm, dm, lm) = (id_map(b), id_map(d), id_map(l));
    let mut result: Vec<Value> = Vec::new();
    // 先按磁盘顺序吸收:存在/修改/删除
    for el in d {
        let Some(id) = el.get("id").and_then(Value::as_str) else { continue };
        match (bm.get(id), lm.get(id)) {
            (Some(_), Some(_)) => {
                // 三方都在:递归合并该元素
                let bv: &Value = bm.get(id).unwrap();
                let lv: &Value = lm.get(id).unwrap();
                result.push(merge_node(bv, el, lv, &format!("{ptr}[{id}]"), out));
            }
            (Some(_), None) => {
                // 本地删,磁盘在:若磁盘未改 → 删除(表 6);磁盘改了 → CF-002
                let bv: &Value = bm.get(id).unwrap();
                if bv == el {
                    // 采用删除:跳过
                } else {
                    out.push(Conflict {
                        code: ConflictCode::DeleteModify,
                        pointer: format!("{ptr}[{id}]"),
                        base: Some(bv.clone()),
                        disk: Some((*el).clone()),
                        local: None,
                    });
                    result.push((*el).clone());
                }
            }
            _ => result.push((*el).clone()), // 磁盘新增或祖先没有
        }
    }
    // 再吸收本地独有(磁盘没有的新增):保持本地顺序附加
    for (id, lv) in &lm {
        let lv: &Value = lv;
        if !dm.contains_key(id) && !bm.contains_key(id) {
            result.push(lv.clone());
        } else if !dm.contains_key(id) && bm.contains_key(id) {
            // 磁盘删了该元素,本地改了 → CF-002
            let bv: &Value = bm.get(id).unwrap();
            if bv != lv {
                out.push(Conflict {
                    code: ConflictCode::DeleteModify,
                    pointer: format!("{ptr}[{id}]"),
                    base: Some(bv.clone()),
                    disk: None,
                    local: Some(lv.clone()),
                });
            }
        } else if dm.contains_key(id) && !bm.contains_key(id) {
            // 表 9 行:两侧都新增同 id(结果里已有磁盘版;冲突则以裁决为准)
            let dv: &Value = dm.get(id).unwrap();
            if dv != lv {
                out.push(Conflict {
                    code: ConflictCode::DupId,
                    pointer: format!("{ptr}[{id}]"),
                    base: None,
                    disk: Some((*dv).clone()),
                    local: Some((*lv).clone()),
                });
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 计划书 4.6 合并表 9 行,逐行断言(M3-4 的 M2 先行覆盖)。
    #[test]
    fn merge_table_nine_rows() {
        // 行1: 双方未变
        let b = json!({"a": 1});
        match three_way_merge(&b, &json!({"a": 1}), &json!({"a": 1})) {
            MergeOutcome::Merged(v) => assert_eq!(v["a"], json!(1)),
            _ => panic!(),
        }
        // 行2: 磁盘变,本地未变 → 磁盘
        match three_way_merge(&json!({"a": 1}), &json!({"a": 2}), &json!({"a": 1})) {
            MergeOutcome::Merged(v) => assert_eq!(v["a"], json!(2)),
            _ => panic!(),
        }
        // 行3: 本地变,磁盘未变 → 本地
        match three_way_merge(&json!({"a": 1}), &json!({"a": 1}), &json!({"a": 3})) {
            MergeOutcome::Merged(v) => assert_eq!(v["a"], json!(3)),
            _ => panic!(),
        }
        // 行4: 双方改成同值 → 幂等合并
        match three_way_merge(&json!({"a": 1}), &json!({"a": 9}), &json!({"a": 9})) {
            MergeOutcome::Merged(v) => assert_eq!(v["a"], json!(9)),
            _ => panic!(),
        }
        // 行5: 双方改成不同值 → CF-001
        match three_way_merge(&json!({"a": 1}), &json!({"a": 2}), &json!({"a": 3})) {
            MergeOutcome::Conflicts(c) => {
                assert_eq!(c.len(), 1);
                assert_eq!(c[0].code.code(), "CF-001");
                assert_eq!(c[0].pointer, "$/a");
            }
            _ => panic!(),
        }
        // 行6: 一侧删除另一侧未动 → 删除
        match three_way_merge(&json!({"a": 1, "b": 2}), &json!({"a": 1}), &json!({"a": 1, "b": 2})) {
            MergeOutcome::Merged(v) => assert!(v.get("b").is_none()),
            _ => panic!(),
        }
        match three_way_merge(&json!({"a": 1, "b": 2}), &json!({"a": 1, "b": 2}), &json!({"a": 1})) {
            MergeOutcome::Merged(v) => assert!(v.get("b").is_none()),
            _ => panic!(),
        }
        // 行7: 一侧删除另一侧修改 → CF-002
        match three_way_merge(&json!({"a": 1, "b": 2}), &json!({"a": 1}), &json!({"a": 1, "b": 3})) {
            MergeOutcome::Conflicts(c) => assert_eq!(c[0].code.code(), "CF-002"),
            _ => panic!(),
        }
        match three_way_merge(&json!({"a": 1, "b": 2}), &json!({"a": 1, "b": 7}), &json!({"a": 1})) {
            MergeOutcome::Conflicts(c) => assert_eq!(c[0].code.code(), "CF-002"),
            _ => panic!(),
        }
        // 行8: 两侧各自插入新元素(不同 id)→ 并存
        let clips = |ids: &[&str]| -> Value {
            Value::Array(ids.iter().map(|i| json!({"id": i, "n": 1})).collect())
        };
        match three_way_merge(&clips(&["A"]), &clips(&["A", "B"]), &clips(&["A", "C"])) {
            MergeOutcome::Merged(v) => {
                assert_eq!(v.as_array().unwrap().len(), 3);
            }
            _ => panic!(),
        }
        // 行9: 两侧插入同 id 不同内容 → CF-003
        let d = json!([{"id": "A", "n": 1}, {"id": "X", "n": 2}]);
        let l = json!([{"id": "A", "n": 1}, {"id": "X", "n": 3}]);
        match three_way_merge(&clips(&["A"]), &d, &l) {
            MergeOutcome::Conflicts(c) => assert_eq!(c[0].code.code(), "CF-003"),
            _ => panic!(),
        }
    }

    #[test]
    fn nested_object_field_level_merge() {
        let base = json!({"clip": {"durationMs": 100, "volume": 1.0}});
        // AI 改 duration,用户改 volume:不同字段自动合并
        let disk = json!({"clip": {"durationMs": 80, "volume": 1.0}});
        let local = json!({"clip": {"durationMs": 100, "volume": 0.5}});
        match three_way_merge(&base, &disk, &local) {
            MergeOutcome::Merged(v) => {
                assert_eq!(v["clip"]["durationMs"], json!(80));
                assert_eq!(v["clip"]["volume"], json!(0.5));
            }
            _ => panic!("不同字段不得冲突"),
        }
        // 同一字段双方不同值:精确到叶路径
        let local2 = json!({"clip": {"durationMs": 120, "volume": 1.0}});
        match three_way_merge(&base, &disk, &local2) {
            MergeOutcome::Conflicts(c) => assert_eq!(c[0].pointer, "$/clip/durationMs"),
            _ => panic!(),
        }
    }

    #[test]
    fn clip_array_mixed_ops_merge() {
        let mk = |id: &str, start: u64, dur: u64| json!({"id": id, "startMs": start, "durationMs": dur});
        let base = json!([mk("V1-001", 0, 100), mk("V1-002", 100, 100)]);
        // 磁盘(AI):改 V1-001 时长 + 删 V1-002(本地未动) + 新增 V1-003
        let disk = json!([mk("V1-001", 0, 90), mk("V1-003", 90, 50)]);
        // 本地(用户):V1-001 未动 + V1-002 未动(磁盘删除生效,表 6)+ 本地新增 V1-004
        let local = json!([mk("V1-001", 0, 100), mk("V1-002", 100, 100), mk("V1-004", 200, 30)]);
        match three_way_merge(&base, &disk, &local) {
            MergeOutcome::Merged(v) => {
                let a = v.as_array().unwrap();
                let ids: Vec<&str> = a.iter().map(|e| e["id"].as_str().unwrap()).collect();
                assert_eq!(ids, vec!["V1-001", "V1-003", "V1-004"], "V1-001 合并时长、V1-002 删除生效、两个新增并存");
                assert_eq!(a[0]["durationMs"], json!(90));
            }
            MergeOutcome::Conflicts(c) => panic!("不得冲突: {c:?}"),
        }
        // 同一元素双方不同改法 → 冲突
        let disk2 = json!([mk("V1-001", 10, 100), mk("V1-002", 100, 100)]);
        let local2 = json!([mk("V1-001", 20, 100), mk("V1-002", 100, 100)]);
        match three_way_merge(&base, &disk2, &local2) {
            MergeOutcome::Conflicts(c) => assert!(!c.is_empty()),
            _ => panic!(),
        }
        // 磁盘删除 V1-002 而本地修改它 → CF-002(行 7 的数组版)
        let local3 = json!([mk("V1-001", 0, 100), mk("V1-002", 110, 100)]);
        match three_way_merge(&base, &disk, &local3) {
            MergeOutcome::Conflicts(c) => assert_eq!(c[0].code.code(), "CF-002"),
            MergeOutcome::Merged(_) => panic!("删改并存必须冲突"),
        }
    }
}
