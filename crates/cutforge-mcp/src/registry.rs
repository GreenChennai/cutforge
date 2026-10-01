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
/// 转场目录(册四 T4.5;ffmpeg xfade 全集 58 项实测)——渲染端 catalog 模块与
/// 壳 GET /catalogs 同源;不经 MCP 工具(工具数口径不变,四则 48)。
pub const TRANSITION_CATALOG_JSON: &str = include_str!("../../../schemas/transition-catalog.json");
/// 特效 + 动效目录(册四 T4.6;fx 首批 11 项 + motion 真实渲染目录)。
pub const FX_CATALOG_JSON: &str = include_str!("../../../schemas/fx-catalog.json");
/// 花字目录(册四 A4 T4.7;12 模板,渲染端 textass 同源;不经 MCP 工具——
/// 工具数四则口径不变,经 GET /catalogs 下发)。
pub const HUAZI_CATALOG_JSON: &str = include_str!("../../../schemas/huazi-catalog.json");

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

/// 工具分类(query/write/orchestrate);未知工具 None。
/// 插件权限裁决(T7.2)与 plan 准入面(T7.5)共用的分类口径。
pub(crate) fn tool_kind(name: &str) -> Option<&'static str> {
    tool_def(name).and_then(|t| t["kind"].as_str())
}

pub(crate) fn envelope(ok: bool, code: &str, message: &str, data: Value) -> Value {
    // T1.7 三面同码:MCP/HTTP 工具面在此统一派生 ns;CLI 面经 cutforge_mcp::code_namespace
    // 同源取值。`ns` 是加法字段,ok/code/message/data 老字段逐字不变。
    json!({"ok": ok, "code": code, "ns": code_namespace(code), "message": message, "data": data})
}

/// 5.4 错误码表(协议一致性门禁的比对基准)。
/// 兼容红线(T1.7):既有 code 取值逐字不变;命名空间化是**加法维度**,
/// 见 [`CODE_NS`](错误码 → 命名空间表)与 [`code_namespace`]。
pub const CODES: &[&str] = &[
    "OK", "CONFLICT", "SCHEMA_INVALID", "PRECONDITION_FAILED", "GUARD_FAILED",
    "JIANYING_RUNNING", "NO_CONFIG", "DEP_MISSING", "GREEN_SCREEN_INPUT", "INTERNAL",
];

/// T1.7 错误码命名空间表(加法维度;单一真相源 = 本表,CLI/MCP/HTTP 三面同源派生)。
/// 每行 = `(码, 命名空间, 中文语义)`;首列与 [`CODES`] 逐字一致(单测锁定)。
/// 命名空间语义:`io.*` 文件系统/环境资源;`core.*` 内核编辑语义(命令/契约/守护/合并);
/// `mcp.*` 协议与编排面;`render.*` 渲染链路;`ok` 成功码(非错误,单列)。
/// 5.4 表外码(CLI 门禁判定器专用码等)派生为 `unknown`,不冒充表内命名空间。
pub const CODE_NS: &[(&str, &str, &str)] = &[
    ("OK", "ok", "成功"),
    ("CONFLICT", "core", "编辑冲突:合并冲突/rev 前置不满足(core 面)"),
    ("SCHEMA_INVALID", "core", "schema 契约校验失败(core 契约面)"),
    ("PRECONDITION_FAILED", "core", "命令前置校验失败:缺参/对象不存在(core 面)"),
    ("GUARD_FAILED", "core", "内核守护拒绝:密度/跨 kind/keep 守卫(core 面)"),
    ("JIANYING_RUNNING", "mcp", "编排面:剪映占用工程/草稿(mcp 编排)"),
    ("NO_CONFIG", "io", "工程/文件/环境资源不存在(io 面)"),
    ("DEP_MISSING", "io", "外部依赖缺失:ffmpeg/ffprobe/CutFlow 环境(io 面)"),
    ("GREEN_SCREEN_INPUT", "render", "绿幕输入校验失败(render 面)"),
    ("INTERNAL", "mcp", "协议面内部错误:未知工具/未实现分支/意外失败(mcp 面)"),
];

/// 由单一真相源 [`CODE_NS`] 派生命名空间;表外码返回 `"unknown"`(诚实降级,不冒认)。
pub fn code_namespace(code: &str) -> &'static str {
    CODE_NS
        .iter()
        .find(|(c, _, _)| *c == code)
        .map(|(_, ns, _)| *ns)
        .unwrap_or("unknown")
}

/// ADR-0003/M8-5:能力矩阵是**生成物**——唯一真相源 docs/capability-matrix.json,
/// 本工具与文档同源;status 只认实码+夹具证据,与 capability-matrix.md 联动更新。
pub(crate) fn capability_matrix() -> Value {
    serde_json::from_str(CAPABILITY_MATRIX_JSON).expect("capability-matrix.json 必须合法")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单一真相源证明:CODE_NS 首列与既有 CODES 逐字一致(兼容红线的机械锁)。
    #[test]
    fn code_ns_table_covers_codes_verbatim() {
        let ns_codes: Vec<&str> = CODE_NS.iter().map(|(c, _, _)| *c).collect();
        assert_eq!(ns_codes, CODES.to_vec(), "CODE_NS 与 CODES 必须同序同值(命名空间化是加法)");
    }

    /// 命名空间派生:表内码各归其位;表外码诚实降级 unknown,不冒充表内命名空间。
    #[test]
    fn code_namespace_maps_and_degrades() {
        assert_eq!(code_namespace("OK"), "ok");
        assert_eq!(code_namespace("NO_CONFIG"), "io");
        assert_eq!(code_namespace("DEP_MISSING"), "io");
        assert_eq!(code_namespace("CONFLICT"), "core");
        assert_eq!(code_namespace("PRECONDITION_FAILED"), "core");
        assert_eq!(code_namespace("GUARD_FAILED"), "core");
        assert_eq!(code_namespace("SCHEMA_INVALID"), "core");
        assert_eq!(code_namespace("JIANYING_RUNNING"), "mcp");
        assert_eq!(code_namespace("INTERNAL"), "mcp");
        assert_eq!(code_namespace("GREEN_SCREEN_INPUT"), "render");
        assert_eq!(code_namespace("SHELL_PURITY_VIOLATION"), "unknown", "5.4 表外码不得冒认");
    }

    /// envelope 加法字段:ns 派生注入,老四字段原样保留。
    #[test]
    fn envelope_carries_namespace_additively() {
        let e = envelope(false, "NO_CONFIG", "工程不存在", json!({}));
        assert_eq!(e["ok"], json!(false));
        assert_eq!(e["code"], json!("NO_CONFIG"));
        assert_eq!(e["ns"], json!("io"));
        assert_eq!(e["message"], json!("工程不存在"));
        assert!(e["data"].is_object());
    }
}
