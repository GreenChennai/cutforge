// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! cutlist/真相源读取与 merge-patch(册六 T6.1 自 dispatch.rs 纯移动拆分——
//! dispatch 行数红线 A1-3;实现逐字保留,路径解析升级为三态布局感知)。

use crate::registry::envelope;
use cutforge_core::engine::ApplyOpts;
use cutforge_core::oplog::{Actor, OpKind};
use cutforge_io::Workspace;
use cutforge_io::paths;
use serde_json::{Value, json};
use std::path::Path;

/// 真相源文件读取(wordline/cutlist;root 按盘面布局三态解析)。
pub(crate) fn read_truth(p: &Path, label: &str) -> Value {
    match std::fs::read_to_string(p) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(v) => envelope(
                true,
                "OK",
                label,
                json!({label.to_string().replace('-', "_"): v}),
            ),
            Err(e) => envelope(false, "SCHEMA_INVALID", &e.to_string(), json!({})),
        },
        Err(_) => envelope(
            false,
            "NO_CONFIG",
            &format!("文件不存在: {}", p.display()),
            json!({}),
        ),
    }
}

/// RFC7386 merge-patch 应用到 cutlist.json,schema 校验后走 record_change 审计
/// (文件由 Workspace reconcile 先文件后记账落盘——不旁路自写)。
pub(crate) fn apply_cut_merge_patch(ws: &mut Workspace, patch: &Value) -> Value {
    // cutlist 按工程盘面布局解析(V2 04_粗剪决策 / V1 04_cut / V3 工程根)
    let rel = paths::truth_rel_on_disk(ws.root(), "cutlist.json").unwrap_or(paths::CUTLIST_REL);
    let rel_path = ws.root().join(rel);
    let Ok(text) = std::fs::read_to_string(&rel_path) else {
        return envelope(
            false,
            "NO_CONFIG",
            &format!("文件不存在: {}", rel_path.display()),
            json!({}),
        );
    };
    let Ok(before) = serde_json::from_str::<Value>(&text) else {
        return envelope(false, "SCHEMA_INVALID", "cutlist.json 非法 JSON", json!({}));
    };
    let mut after = merge_patch(before.clone(), patch);
    // M9-3:按 cuts[].action 服务端重算 keep/removedMs(rs_cut.finalize_cutlist 镜像,
    // 金样对拍锁定)——经 MCP 的编辑不再是"keep 幻觉"
    if let Err(e) = cutforge_schema::finalize::finalize_cutlist_value(&mut after) {
        return envelope(
            false,
            "GUARD_FAILED",
            &format!("keep 重算失败: {e}"),
            json!({}),
        );
    }
    let errors = cutforge_schema::validate("cutlist", &after);
    if !errors.is_empty() {
        return envelope(
            false,
            "SCHEMA_INVALID",
            &errors.join("; "),
            json!({"errors": errors}),
        );
    }
    let rec = ws.record_change(
        "cutlist.json",
        "/",
        before,
        after,
        OpKind::Set,
        Actor::script("cutforge-mcp:cut_apply"),
        ApplyOpts {
            summary: Some("cut_apply merge-patch".into()),
            ..Default::default()
        },
    );
    match rec {
        Ok(r) => envelope(
            true,
            "OK",
            "cutlist 已更新",
            json!({"rev": r.rev, "opIds": r.op_ids}),
        ),
        Err(e) => envelope(false, "INTERNAL", &e.to_string(), json!({})),
    }
}

/// 标注操作协议错误映射(5.4 表内码):不存在/参数不合法 → PRECONDITION_FAILED,
/// 其余 → INTERNAL。
pub(crate) fn notes_op_error(e: std::io::Error) -> Value {
    let code = if e.kind() == std::io::ErrorKind::NotFound
        || e.kind() == std::io::ErrorKind::InvalidInput
    {
        "PRECONDITION_FAILED"
    } else {
        "INTERNAL"
    };
    envelope(false, code, &e.to_string(), json!({}))
}

pub(crate) fn merge_patch(mut target: Value, patch: &Value) -> Value {
    if let (Some(t), Some(p)) = (target.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            if v.is_null() {
                t.remove(k);
            } else {
                let nv = match t.get(k) {
                    Some(existing) if existing.is_object() && v.is_object() => {
                        merge_patch(existing.clone(), v)
                    }
                    _ => v.clone(),
                };
                t.insert(k.clone(), nv);
            }
        }
        target
    } else {
        patch.clone()
    }
}
