// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 工具注册表:编译期嵌入 mcp-tools.json / capability-matrix.json / ui-fields.json,
//! 结果协议 envelope 与 5.4 错误码表(T1.1 拆分自 lib.rs,纯移动)。

use serde_json::{json, Value};
use std::sync::OnceLock;

pub const MCP_TOOLS_JSON: &str = include_str!("../../../schemas/mcp-tools.json");
/// 能力矩阵单一真相源(ADR-0003/M8-5):MCP 工具与 docs/capability-matrix.md 同源。
pub const CAPABILITY_MATRIX_JSON: &str = include_str!("../../../docs/capability-matrix.json");
/// E4-2 单一真相源:壳允许编辑的字段集(机械校验 = cutforge-cli check-ui-fields;
/// 壳经由 GET /ui-fields 取本文件渲染检查器分组,壳不读文件系统)。
pub const UI_FIELDS_JSON: &str = include_str!("../../../schemas/ui-fields.json");

/// 工具注册表(契约来自 schemas/mcp-tools.json;派发处理器同在本 crate)。
pub fn registry() -> &'static Vec<Value> {
    static CACHE: OnceLock<Vec<Value>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let doc: Value = serde_json::from_str(MCP_TOOLS_JSON).expect("mcp-tools.json 必须合法");
        doc["tools"].as_array().cloned().unwrap_or_default()
    })
}

pub fn tool_names() -> Vec<String> {
    registry()
        .iter()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect()
}

pub(crate) fn tool_def(name: &str) -> Option<&'static Value> {
    registry().iter().find(|t| t["name"].as_str() == Some(name))
}

pub(crate) fn envelope(ok: bool, code: &str, message: &str, data: Value) -> Value {
    json!({"ok": ok, "code": code, "message": message, "data": data})
}

/// 5.4 错误码表(协议一致性门禁的比对基准)。
pub const CODES: &[&str] = &[
    "OK", "CONFLICT", "SCHEMA_INVALID", "PRECONDITION_FAILED", "GUARD_FAILED",
    "JIANYING_RUNNING", "NO_CONFIG", "DEP_MISSING", "GREEN_SCREEN_INPUT", "INTERNAL",
];

/// ADR-0003/M8-5:能力矩阵是**生成物**——唯一真相源 docs/capability-matrix.json,
/// 本工具与文档同源;status 只认实码+夹具证据,与 capability-matrix.md 联动更新。
pub(crate) fn capability_matrix() -> Value {
    serde_json::from_str(CAPABILITY_MATRIX_JSON).expect("capability-matrix.json 必须合法")
}
