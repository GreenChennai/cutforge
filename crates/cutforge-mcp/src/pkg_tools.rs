// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! `.cfpkg` 工程打包/解包工具(册七 T7.6):project_package(写)/ project_unpackage(写)。
//! 单一实现在 `cutforge_io::cfpkg`(zip 原语 `cutforge_io::zipstore`),本模块只做
//! 参数接线与错误码映射——不建第二套打包逻辑。两工具均免开工作区(不产 Op 不改 IR,
//! 与 migrate_layout 同类;打包持工程锁在 io 层内自持)。

use crate::registry::envelope;
use cutforge_io::cfpkg::{self, PkgError};
use serde_json::{json, Value};
use std::path::Path;

/// PkgError → 5.4 错误码(诚实映射:容器坏 = SCHEMA_INVALID,目标已存在 = CONFLICT,
/// 工程缺 = NO_CONFIG,活进程持锁 = PRECONDITION_FAILED,意外 IO = INTERNAL)。
fn pkg_error(e: PkgError) -> Value {
    match e {
        PkgError::NotAProject(p) => {
            envelope(false, "NO_CONFIG", &format!("不是可打开的工程: {}", p.display()), json!({}))
        }
        PkgError::InvalidPkg(m) => envelope(false, "SCHEMA_INVALID", &m, json!({})),
        PkgError::Conflict(m) => envelope(false, "CONFLICT", &m, json!({})),
        PkgError::Locked(m) => envelope(false, "PRECONDITION_FAILED", &m, json!({})),
        PkgError::Io(err) => envelope(false, "INTERNAL", &err.to_string(), json!({})),
    }
}

/// `project_package`(写,免锁面):工程 → `.cfpkg` 单文件容器(manifest + project
/// 真相源 + oplog + media 引用素材 + 可选 exports)。includeMedia 缺省 true;
/// includeExports 缺省 false;out 缺省 `<root>/<slug>.cfpkg`。
pub(crate) fn project_package_tool(root: &Path, args: &Value) -> Value {
    let include_media = args["includeMedia"].as_bool().unwrap_or(true);
    let include_exports = args["includeExports"].as_bool().unwrap_or(false);
    let out = args["out"].as_str().filter(|s| !s.is_empty()).map(Path::new);
    match cfpkg::pack(root, out, include_media, include_exports) {
        Ok(r) => {
            let total = r.counts.0 + r.counts.1 + r.counts.2 + r.counts.3 + 1; // + manifest
            envelope(true, "OK", "工程已打包(.cfpkg)", json!({
                "out": r.out.to_string_lossy(),
                "format": cfpkg::FORMAT,
                "formatVersion": cfpkg::FORMAT_VERSION,
                "name": r.name,
                "schemaVersion": r.schema_version,
                "rev": r.rev,
                "sourceLayout": r.source_layout,
                "counts": {"project": r.counts.0, "oplog": r.counts.1,
                           "media": r.counts.2, "exports": r.counts.3},
                "files": total,
                "missing": r.missing,
                "note": if r.missing.is_empty() {
                    "解包:project_unpackage(src=该文件,root=新工程目录);布局语义见 docs/PROJECT-FORMAT.md"
                } else {
                    "引用素材有缺失(见 missing);解包后工程可打开但缺素材段需重导"
                },
            }))
        }
        Err(e) => pkg_error(e),
    }
}

/// `project_unpackage`(写,免锁面):`.cfpkg` → 新工程目录(真相源落 v3 契约位,
/// 素材原位还原)。root = 新工程目录(必须不存在;otio_import/project_new 同纪律),
/// src = 容器路径。清单校验(format/formatVersion/project 真相源)+ zip-slip 防线。
pub(crate) fn project_unpackage_tool(root: &Path, args: &Value) -> Value {
    let Some(src) = args["src"].as_str().filter(|s| !s.is_empty()) else {
        return envelope(false, "PRECONDITION_FAILED", "缺 src(.cfpkg 容器路径)", json!({}));
    };
    let src_path = Path::new(src);
    if !src_path.is_file() {
        return envelope(false, "NO_CONFIG", &format!("容器不存在: {src}"), json!({}));
    }
    match cfpkg::unpack(src_path, root) {
        Ok(r) => envelope(true, "OK", "解包完成(v3 布局)", json!({
            "dest": r.dest.to_string_lossy(),
            "name": r.name,
            "sourceLayout": r.source_layout,
            "files": r.files,
            "media": r.media,
            "missing": r.missing,
            "hint": format!("打开:cutforge-cli serve {} 或 cutforge-mcp serve --root {}",
                r.dest.display(), r.dest.display()),
        })),
        Err(e) => pkg_error(e),
    }
}
