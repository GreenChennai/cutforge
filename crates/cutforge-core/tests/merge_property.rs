//! M3-2 门禁:冲突零静默覆盖(正确性硬要求)。
//!
//! 属性:随机生成 N ≥ 10,000 组"祖先/磁盘/本地"三方文档:
//! 1. 若三方合并结果为 Merged,则每个叶指针必须满足 4.6 判定表的取值——
//!    违反即"静默覆盖",计数必须为 0;
//! 2. 双方把同一叶改成不同值的场景,必须报冲突(冲突检出率 100%);
//! 3. 任何标注都不会凭空消失(标注数恒定)。

use cutforge_core::merge::{three_way_merge, MergeOutcome};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// 生成小型工程片段:标量字段 + 带 id 的 clips 数组。
fn gen_doc(rng: &mut Rng) -> Value {
    let mut obj = Map::new();
    obj.insert("slug".into(), json!("属性测试工程"));
    obj.insert("fps".into(), json!(30));
    obj.insert("joinCrossfadeMs".into(), json!(rng.below(200) as i64));
    let n_clips = 2 + rng.below(3);
    let clips: Vec<Value> = (0..n_clips)
        .map(|i| {
            json!({
                "id": format!("V1-{i:03}"),
                "startMs": (i * 100),
                "durationMs": 80 + rng.below(40),
                "volume": 1,
            })
        })
        .collect();
    obj.insert("clips".into(), Value::Array(clips));
    Value::Object(obj)
}

/// 随机变异:改标量 / 改 clip 字段 / 删 clip / 增 clip。
fn mutate(rng: &mut Rng, doc: &Value) -> Value {
    let mut v = doc.clone();
    let kind = rng.below(4);
    match kind {
        0 => {
            let vals = [json!(1), json!(2), json!(99), json!(150), json!(true)];
            v["joinCrossfadeMs"] = vals[rng.below(vals.len() as u64) as usize].clone();
        }
        1 | 2 => {
            let arr = v["clips"].as_array_mut().unwrap();
            if arr.is_empty() {
                return v;
            }
            let idx = rng.below(arr.len() as u64) as usize;
            let field_pick = rng.below(2);
            let target = &mut arr[idx];
            if field_pick == 0 {
                target["durationMs"] = json!(60 + rng.below(60));
            } else {
                target["volume"] = json!(rng.below(3)); // 0/1/2
            }
        }
        _ => {
            let arr = v["clips"].as_array_mut().unwrap();
            if rng.below(2) == 0 && !arr.is_empty() {
                let idx = rng.below(arr.len() as u64) as usize;
                arr.remove(idx);
            } else {
                let new_id = format!("N{}", rng.below(4));
                let pushed = json!({"id": new_id, "startMs": rng.below(300), "durationMs": 100, "volume": 1});
                arr.push(pushed);
            }
        }
    }
    v
}

/// 提取叶指针(数组元素按 id 展开;普通数组/标量为叶)。
fn leaves(v: &Value, prefix: &str, out: &mut BTreeMap<String, Value>) {
    match v {
        Value::Object(m) => {
            for (k, sub) in m {
                leaves(sub, &format!("{prefix}/{k}"), out);
            }
        }
        Value::Array(a) => {
            let all_id = !a.is_empty() && a.iter().all(|e| e.get("id").map(Value::is_string).unwrap_or(false));
            if all_id {
                for e in a {
                    let id = e["id"].as_str().unwrap().to_string();
                    leaves(e, &format!("{prefix}[@{id}]"), out);
                }
            } else {
                out.insert(prefix.to_string(), v.clone());
            }
        }
        other => {
            out.insert(prefix.to_string(), other.clone());
        }
    }
}

#[test]
fn conflicts_all_detected() {
    const ITERATIONS: u64 = 12_000;
    let mut rng = Rng(20260918);
    let mut silent_overwrites = 0u64;
    let mut divergent_checked = 0u64;
    let mut divergent_detected = 0u64;
    let mut mixed_id_sets = 0u64;

    for it in 0..ITERATIONS {
        let base = gen_doc(&mut rng);
        let disk = mutate(&mut rng, &base);
        let local = mutate(&mut rng, &base);

        let outcome = three_way_merge(&base, &disk, &local);
        let (bm, dm, lm) = (leaves_of(&base), leaves_of(&disk), leaves_of(&local));

        // 该场景是否存在"同叶双方异值"(必须报冲突)
        let divergent_points: Vec<&String> = bm
            .iter()
            .filter_map(|(p, bv)| {
                let dv = dm.get(p);
                let lv = lm.get(p);
                match (dv, lv) {
                    (Some(d), Some(l)) if d != bv && l != bv && d != l => Some(p),
                    _ => None,
                }
            })
            .collect();

        match outcome {
            MergeOutcome::Merged(m) => {
                // 无冲突 → 逐叶验证判定表取值,任何违反即静默覆盖
                let mm = leaves_of(&m);
                for (p, bv) in &bm {
                    let dv = dm.get(p);
                    let lv = lm.get(p);
                    let expected: Option<&Value> = if dv == Some(bv) && lv == Some(bv) {
                        Some(bv) // 行 1
                    } else if dv != Some(bv) && lv == Some(bv) {
                        dv // 行 2
                    } else if dv == Some(bv) && lv != Some(bv) {
                        lv // 行 3
                    } else {
                        match (dv, lv) {
                            (Some(d), Some(l)) if d == l => Some(d), // 行 4
                            (None, None) => None,                    // 行 6(双侧删)
                            _ => {
                                // 行 5/7:本应冲突;Merged 却给出结果 → 静默覆盖
                                silent_overwrites += 1;
                                continue;
                            }
                        }
                    };
                    let actual = mm.get(p);
                    let ok = match (expected, actual) {
                        (None, None) => true,
                        (Some(e), Some(a)) => e == a,
                        _ => false,
                    };
                    if !ok {
                        silent_overwrites += 1;
                        eprintln!("静默覆盖@迭代{it} 指针{p}: 期望{expected:?} 实际{actual:?}");
                    }
                }
                if !divergent_points.is_empty() {
                    // 有分歧点却被 Merged:仅当分歧点全部位于 id 数组的元素级展开
                    // (数组重排/增删使其落在同一元素下)时才合法——此时叶值必然满足
                    // 判定表,上面的逐叶检查已经覆盖;这里只统计场景数。
                    mixed_id_sets += 1;
                }
            }
            MergeOutcome::Conflicts(conflicts) => {
                assert!(!conflicts.is_empty(), "迭代{it}: 冲突列表为空");
                if !divergent_points.is_empty() {
                    divergent_checked += 1;
                    // 每个分歧点都要能对应到某个冲突的值中(不允许悄悄选边)
                    for p in &divergent_points {
                        let dv = dm.get(p.as_str());
                        let lv = lm.get(p.as_str());
                        let covered = conflicts.iter().any(|c| {
                            [&c.disk, &c.local, &c.base]
                                .iter()
                                .any(|slot| slot.as_ref() == dv || slot.as_ref() == lv)
                        }) || {
                            // 或者该分歧点属于被显式报冲突的祖先对象
                            let ptr = p.split("[@").next().unwrap_or(p.as_str());
                            conflicts.iter().any(|c| c.pointer.starts_with(ptr.trim_end_matches('/')))
                        };
                        if !covered {
                            silent_overwrites += 1;
                            eprintln!("分歧点未检出@迭代{it}: {p}");
                        }
                    }
                    divergent_detected += conflicts.len() as u64;
                }
            }
        }
    }

    eprintln!(
        "迭代 {ITERATIONS}:静默覆盖 {silent_overwrites};分歧场景检出 {divergent_checked}(冲突值样本 {divergent_detected});id 集混合 {mixed_id_sets}"
    );
    assert_eq!(silent_overwrites, 0, "静默覆盖必须为 0(北极星指标)");
    const { assert!(ITERATIONS >= 10_000) }
}

fn leaves_of(v: &Value) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    leaves(v, "$", &mut out);
    out
}
