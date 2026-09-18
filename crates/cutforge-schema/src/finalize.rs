//! cutlist 的 keep/removedMs 重算(apply 路径)——rs_cut.finalize_cutlist 的
//! Rust 逐语义镜像(门禁:tests/fixtures/keep_recompute_golden.json 由
//! CutFlow 真实实现生成,Rust 侧逐例对拍)。M9-3:经 MCP cut_apply 的编辑
//! 由服务端重算,不再是"keep 是幻觉"的旁路路径。

use serde_json::Value;

/// 按当前 action==remove 重算 keep 与 removedMs(就地修改)。
/// 语义与 rs_cut.derive_keep/finalize_cutlist 一致:remove 区间按 inMs 排序,
/// keep = [0,srcTotalMs] 的补集(端点钳制,空段丢弃);相邻 remove 自然并段。
/// 返回 Err = keep 不合法(无序/空段/未覆盖片尾,与 rs_cut 校验同文案语义)。
pub fn finalize_cutlist_value(cl: &mut Value) -> Result<(), String> {
    let total = cl["srcTotalMs"].as_i64().ok_or("缺 srcTotalMs")?;
    let Some(cuts) = cl["cuts"].as_array() else { return Err("缺 cuts".into()) };
    let mut removes: Vec<(i64, i64)> = cuts
        .iter()
        .filter(|c| c["action"].as_str() == Some("remove"))
        .filter_map(|c| Some((c["inMs"].as_i64()?, c["outMs"].as_i64()?)))
        .collect();
    removes.sort_by_key(|(a, _)| *a);

    let mut keep: Vec<Vec<i64>> = Vec::new();
    let mut cursor: i64 = 0;
    for (a, b) in &removes {
        let a = (*a).max(0).min(total);
        let b = (*b).max(0).min(total);
        if a > cursor {
            keep.push(vec![cursor, a]);
        }
        cursor = cursor.max(b);
    }
    if cursor < total {
        keep.push(vec![cursor, total]);
    }
    keep.retain(|k| k[1] - k[0] > 0);

    let removed_ms: i64 = removes.iter().map(|(a, b)| b - a).sum();

    // 与 rs_cut.finalize_cutlist 相同的校验(keep 有序、非空、覆盖到片尾)
    let mut cur: i64 = 0;
    for seg in &keep {
        let (a, b) = (seg[0], seg[1]);
        if a < cur {
            return Err(format!("keep 区间重叠:a={a} < 上一段末尾 {cur}"));
        }
        if b <= a {
            return Err(format!("keep 空区间:{a}-{b}"));
        }
        cur = b;
    }
    if cur != total {
        return Err(format!("keep 未覆盖到片尾:cursor={cur} total={total}"));
    }

    let obj = cl.as_object_mut().ok_or("cutlist 必须是 object")?;
    obj.insert("keep".into(), serde_json::to_value(&keep).unwrap());
    obj.insert("removedMs".into(), serde_json::json!(removed_ms));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 与 CutFlow 真实实现逐例对拍(金样由 rs_cut.finalize_cutlist 生成)。
    #[test]
    fn parity_with_rs_cut_golden() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/keep_recompute_golden.json");
        let text = std::fs::read_to_string(path).expect("缺 keep_recompute_golden.json");
        let doc: Value = serde_json::from_str(&text).unwrap();
        for case in doc["cases"].as_array().unwrap() {
            let mut cl = case["input"].clone();
            finalize_cutlist_value(&mut cl)
                .unwrap_or_else(|e| panic!("case {} 重算失败: {e}", case["name"]));
            assert_eq!(cl["keep"], case["expectedKeep"], "case {} keep 不符", case["name"]);
            assert_eq!(cl["removedMs"], case["expectedRemovedMs"], "case {} removedMs 不符", case["name"]);
        }
    }

    #[test]
    fn tail_remove_is_rejected_like_rs_cut() {
        // CutFlow 现行语义:remove 触及片尾 → keep 不覆盖 → 校验失败(双向一致)
        let mut cl = json!({
            "version": 1, "source": "s", "detector": {"version": "v", "params": {}},
            "cuts": [{"id": "c001", "inMs": 4500, "outMs": 5000, "reason": "silence",
                      "conf": 0.9, "action": "remove", "note": "", "guard": {}, "text": ""}],
            "keep": [[0, 4500]], "removedMs": 500, "srcTotalMs": 5000, "script": []
        });
        assert!(finalize_cutlist_value(&mut cl).is_err());
    }
}
