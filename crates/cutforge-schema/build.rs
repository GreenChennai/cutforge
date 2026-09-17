//! 契约层 build 脚本:五份 schema 与两份生成物在编译期必须存在(ADR-0034)。
//! 内容本身经 lib.rs 的 include_str! 嵌入;此处只做存在性闸与重编译触发。

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let required = [
        "schemas/project.schema.json",
        "schemas/wordline.schema.json",
        "schemas/cutlist.schema.json",
        "schemas/notes.schema.json",
        "schemas/oplog.schema.json",
        "schemas/constants.ratios.json",
        "schemas/mcp-tools.json",
    ];
    for rel in required {
        let p = root.join(rel);
        assert!(p.exists(), "契约文件缺失: {}", p.display());
        println!("cargo:rerun-if-changed={}", p.display());
    }
}
