//! CutForge wasm 绑定(计划书 7.6;依赖纪律:wasm-bindgen 由计划书 2.4 选型预论证)。
//!
//! Web 壳只做展示:所有投影(时间线行/标注汇总/OpLog 摘要)由内核在此算好,
//! 以 JSON 字符串交给 JS——**壳不持有真相、不算时间线语义**(M5-5 纯度门禁)。
//! 同一批纯函数同时编译到 native 与 wasm32:`cross_shell_equivalence` 测试
//! 在 native 侧验证"原生壳投影"与"wasm ABI 投影"逐语义相等。

use cutforge_core::engine::Answer;
use cutforge_core::notes::NotesStore;
use cutforge_core::Project;
use serde_json::{json, Value};
use std::path::Path;
use wasm_bindgen::prelude::*;

fn parse(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| format!("JSON 解析失败: {e}"))
}

/// 工程视图:解析 → v1 迁移 → v2 校验 → 全量投影(壳渲染的唯一数据源)。
#[wasm_bindgen]
pub fn forge_open(project_json: &str) -> Result<String, String> {
    let v = parse(project_json)?;
    let project = Project::from_value(&v).or_else(|_| cutforge_core::model::migrate_from_value(&v)).map_err(|e| e.join("; "))?;
    let view = serde_json::to_value(&project).map_err(|e| e.to_string())?;
    Ok(json!({ "project": view, "rev": 0 }).to_string())
}

/// 时间线投影:行数据(id/start/end/track)由内核算好,壳只做排版。
#[wasm_bindgen]
pub fn forge_timeline(project_json: &str) -> Result<String, String> {
    let v = parse(project_json)?;
    let project = Project::from_value(&v).or_else(|_| cutforge_core::model::migrate_from_value(&v)).map_err(|e| e.join("; "))?;
    let rows: Vec<Value> = project
        .tracks
        .iter()
        .flat_map(|t| {
            t.clips.iter().map(move |c| {
                json!({"id": c.id, "track": t.id, "startMs": c.start_ms,
                       "endMs": c.start_ms + c.duration_ms})
            })
        })
        .collect();
    Ok(json!({ "clips": rows }).to_string())
}

/// 标注投影:校验 + 各状态计数 + 孤儿清单(orphan 必须在面板可见)。
#[wasm_bindgen]
pub fn forge_notes(notes_json: &str) -> Result<String, String> {
    let v = parse(notes_json)?;
    let store = NotesStore::from_value(&v).map_err(|e| e.join("; "))?;
    let counts = json!({
        "total": store.notes().len(),
        "open": store.filter(Some(cutforge_core::notes::NoteState::Open), None).len(),
        "resolved": store.filter(Some(cutforge_core::notes::NoteState::Resolved), None).len(),
        "rejected": store.filter(Some(cutforge_core::notes::NoteState::Rejected), None).len(),
        "orphan": store.filter(Some(cutforge_core::notes::NoteState::Orphan), None).len(),
    });
    Ok(json!({ "counts": counts, "notes": store.notes(),
               "orphans": store.orphans().iter().map(|n| n.id.clone()).collect::<Vec<_>>() }).to_string())
}

/// OpLog 摘要:解析 .jsonl 文本,按 actor 汇总并列出带 before/after 的改动(差异面板)。
#[wasm_bindgen]
pub fn forge_oplog(oplog_lines: &str) -> Result<String, String> {
    let mut ops: Vec<Value> = Vec::new();
    for line in oplog_lines.lines().filter(|l| !l.trim().is_empty()) {
        let op: Value = parse(line)?;
        ops.push(op);
    }
    ops.sort_by_key(|o| o["rev"].as_u64().unwrap_or(0));
    let by_actor: std::collections::BTreeMap<String, usize> = {
        let mut m = std::collections::BTreeMap::new();
        for o in &ops {
            let k = o["actor"]["kind"].as_str().unwrap_or("?").to_string();
            *m.entry(k).or_insert(0) += 1;
        }
        m
    };
    Ok(json!({ "total": ops.len(), "byActor": by_actor, "ops": ops }).to_string())
}

/// 冲突投影:透传校验后的三方快照列表。
#[wasm_bindgen]
pub fn forge_conflicts(conflicts_json: &str) -> Result<String, String> {
    let v = parse(conflicts_json)?;
    Ok(json!({ "conflicts": v }).to_string())
}

/// 契约校验门(v1 自动迁移后验证)。
#[wasm_bindgen]
pub fn forge_validate(project_json: &str) -> Result<String, String> {
    let v = parse(project_json)?;
    match Project::from_value(&v).or_else(|_| cutforge_core::model::migrate_from_value(&v)) {
        Ok(_) => Ok(json!({"ok": true, "code": "OK", "message": "通过 v2 契约", "data": {}}).to_string()),
        Err(errors) => Ok(json!({"ok": false, "code": "SCHEMA_INVALID",
                                 "message": errors.join("; "), "data": {"errors": errors}}).to_string()),
    }
}

/// 脚本页执行器:批式查询脚本经策略沙箱跑在内存工程上(查询类工具,无副作用)。
#[wasm_bindgen]
pub fn forge_script_run(project_json: &str, steps_json: &str) -> Result<String, String> {
    use cutforge_script::{run_script, Policy, ToolDispatch};
    struct InMemory {
        doc: Value,
        allowed: Vec<String>,
    }
    impl ToolDispatch for InMemory {
        fn allowed_tools(&self) -> &[String] {
            &self.allowed
        }
        fn call(&mut self, tool: &str, _args: &Value) -> Result<Value, String> {
            match tool {
                "project_get" => Ok(json!({"ok": true, "code": "OK", "message": "工程视图",
                                           "data": {"project": self.doc}})),
                other => Ok(json!({"ok": false, "code": "INTERNAL",
                                   "message": format!("wasm 查询宿主未实现: {other}"), "data": {}})),
            }
        }
    }
    let doc = parse(project_json)?;
    let script = parse(steps_json)?;
    let mut host = InMemory {
        doc,
        allowed: ["project_get"].iter().map(|s| s.to_string()).collect(),
    };
    let policy = Policy::new(Path::new("."));
    let report = run_script(&script, &mut host, &policy).map_err(|e| e.to_string())?;
    Ok(serde_json::to_string(&report).map_err(|e| e.to_string())?)
}

/// native/wasm 共用:Answer 投影规范化(跨壳等价测试用)。
pub fn answer_to_value(a: &Answer) -> Value {
    match a {
        Answer::Project(v) => json!({"kind": "project", "value": v}),
        Answer::Timeline(rows) => json!({"kind": "timeline", "rows": rows.iter().map(|(id, s, e, t)|
            json!({"id": id, "startMs": s, "endMs": e, "track": t})).collect::<Vec<_>>()}),
        Answer::Clip(v) => json!({"kind": "clip", "value": v}),
        Answer::Track(v) => json!({"kind": "track", "value": v}),
        Answer::Ops(ops) => json!({"kind": "ops", "value": ops}),
        Answer::Rev(r) => json!({"kind": "rev", "value": r}),
    }
}
