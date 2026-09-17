//! CutForge 契约层(ARL-CORE)。
//!
//! 职责(计划书 2.1):持有五份 JSON Schema;生成 Rust 结构体与 Python 校验器;
//! 漂移检测。**不含业务逻辑;不得内联第二份常量表。**
//!
//! M1 落地内容:build.rs 从 `schemas/*.json` 生成结构体、v1→v2 迁移器(幂等)、
//! 双端校验等价测试。当前为 M0 骨架占位。

/// 占位:返回契约层当前支持的 schema 清单(随 M1 扩充)。
pub const SCHEMAS: &[&str] = &[
    "project.schema.json",
    "wordline.schema.json",
    "cutlist.schema.json",
    "notes.schema.json",
    "oplog.schema.json",
];

#[cfg(test)]
mod tests {
    #[test]
    fn schemas_manifest_has_five_entries() {
        assert_eq!(super::SCHEMAS.len(), 5);
    }
}
