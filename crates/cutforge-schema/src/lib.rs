//! CutForge 契约层(ARL-CORE,ADR-0034)。
//!
//! 职责(计划书 2.1):持有 JSON Schema 契约面(draft-07 子集 + x- 断言;
//! 五份工程契约 + 册七 T7.2 起 plugin-manifest);
//! 供 Rust 侧校验与 v1→v2 迁移;与 Python 生成校验器(tools/_generated/cf_validate.py)
//! 双端对拍(M1-3)。不含业务逻辑;不内联第二份常量表(常量经 constants.ratios.json)。
//!
//! schema 文件在编译期经 include_str! 嵌入 `SCHEMA_SOURCES`;build.rs 校验其存在性。

pub mod engine;
pub mod finalize;
pub mod migrate;

/// 契约 schema(唯一手写契约的 Rust 侧嵌入;与 schemas/ 目录一一对应)。
/// 册七 T7.2 起 +plugin-manifest(插件清单;五工程契约语义不动,加法维度)。
pub const SCHEMA_SOURCES: &[(&str, &str)] = &[
    ("project", include_str!("../../../schemas/project.schema.json")),
    ("wordline", include_str!("../../../schemas/wordline.schema.json")),
    ("cutlist", include_str!("../../../schemas/cutlist.schema.json")),
    ("notes", include_str!("../../../schemas/notes.schema.json")),
    ("oplog", include_str!("../../../schemas/oplog.schema.json")),
    ("plugin-manifest", include_str!("../../../schemas/plugin-manifest.schema.json")),
];

/// MCP 工具契约(G5-1 比对基准,M4 填充 schema)。
pub const MCP_TOOLS_SOURCE: &str = include_str!("../../../schemas/mcp-tools.json");

/// 常量单源生成物(3.9;漂移由 tools/gen_constants.py --check 门禁阻断)。
pub const CONSTANTS_SOURCE: &str = include_str!("../../../schemas/constants.ratios.json");

use std::sync::OnceLock;

fn schemas() -> &'static std::collections::BTreeMap<String, serde_json::Value> {
    static CACHE: OnceLock<std::collections::BTreeMap<String, serde_json::Value>> = OnceLock::new();
    CACHE.get_or_init(|| {
        SCHEMA_SOURCES
            .iter()
            .map(|(name, src)| {
                (
                    (*name).to_string(),
                    serde_json::from_str(src).expect("内嵌 schema 必须是合法 JSON"),
                )
            })
            .collect()
    })
}

/// 校验指定 schema;返回错误列表(空 = 通过)。语义与 Python 生成校验器逐一对齐。
pub fn validate(schema_name: &str, data: &serde_json::Value) -> Vec<String> {
    let root = schemas().get(schema_name).unwrap_or_else(|| panic!("未知 schema: {schema_name}"));
    engine::validate_node(root, root, data, "$")
}

/// project v1→v2 迁移(幂等)。语义与 cf_validate.migrate_project_v1_to_v2 一致。
pub fn migrate_project(doc: &serde_json::Value) -> serde_json::Value {
    migrate::migrate_project_v1_to_v2(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_manifest_has_six_entries() {
        assert_eq!(SCHEMA_SOURCES.len(), 6);
        assert_eq!(schemas().len(), 6);
    }

    /// 册七 T7.2:plugin-manifest 契约可校验(正例过 / 缺必填与坏字段拒)。
    #[test]
    fn plugin_manifest_schema_validates() {
        let good = serde_json::json!({
            "id": "demo-clip", "name": "示例插件", "version": "1.0.0",
            "form": "process", "entry": "plugin.py",
            "permissions": {"read": true, "write": true}
        });
        assert!(validate("plugin-manifest", &good).is_empty(), "合法 manifest 必须过");
        let bad = serde_json::json!({
            "id": "Demo", "name": "", "version": "1.0",
            "form": "vm", "permissions": {"read": true}
        });
        let errs = validate("plugin-manifest", &bad);
        assert!(errs.len() >= 4, "id/name/version/form 四处必须点名:{errs:?}");
    }

    #[test]
    fn constants_embed_parse() {
        let v: serde_json::Value = serde_json::from_str(CONSTANTS_SOURCE).unwrap();
        assert!(v["ratios"]["9x16"].is_array());
    }

    #[test]
    fn mcp_tools_embed_parse() {
        let v: serde_json::Value = serde_json::from_str(MCP_TOOLS_SOURCE).unwrap();
        assert!(v["tools"].as_array().unwrap().len() >= 27);
    }
}
