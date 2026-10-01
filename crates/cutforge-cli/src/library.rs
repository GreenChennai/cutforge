//! 库面与管理子命令(册六 T6.1):`library`(七操作)/ `migrate`(布局 v3 迁移)/
//! `recover`(崩溃恢复)。全部走 cutforge_io 单一实现,不旁路自写;
//! 输出 {ok, code, message, data} 结果协议(emit 与错误码归 CLI lib.rs 同源)。

use crate::emit;
use cutforge_io::{library, migrate as mig, recover as rec, LayoutKind};
use std::path::{Path, PathBuf};

/// 库根解析(--library 旗标覆盖 env/缺省)。
fn lib_root(flag: Option<&String>) -> PathBuf {
    match flag {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => library::library_root(),
    }
}

fn layout_of(flag: Option<&String>) -> Result<LayoutKind, String> {
    match flag.map(|s| s.as_str()).unwrap_or("v2") {
        "v2" => Ok(LayoutKind::V2),
        "v3" => Ok(LayoutKind::V3),
        other => Err(format!("未知布局: {other}(允许 v2/v3;过渡期缺省 v2,ADR-0021)")),
    }
}

fn track_kinds(flag: Option<&String>) -> Result<Vec<cutforge_core::model::TrackKind>, String> {
    let mut kinds = Vec::new();
    if let Some(v) = flag {
        for t in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let kind = match t {
                "video" => cutforge_core::model::TrackKind::Video,
                "audio" => cutforge_core::model::TrackKind::Audio,
                "text" => cutforge_core::model::TrackKind::Text,
                "adjust" => cutforge_core::model::TrackKind::Adjust,
                other => return Err(format!("未知轨道类型: {other}")),
            };
            kinds.push(kind);
        }
    }
    if kinds.is_empty() {
        kinds = vec![cutforge_core::model::TrackKind::Video, cutforge_core::model::TrackKind::Audio];
    }
    Ok(kinds)
}

/// `library <action> …`:`list|search|new|rename|copy|archive|unarchive|delete`。
/// 库根 = `--library R` > env CUTFORGE_PROJECTS > %USERPROFILE%\CutForge\Projects。
pub fn library_cmd(a: &crate::Args) -> i32 {
    let Some(action) = a.positional.first().cloned() else {
        return emit(a.json, false, "PRECONDITION_FAILED",
            "用法: library <list|search|new|rename|copy|archive|unarchive|delete> […] [--library R]",
            serde_json::json!({"actions": ["list", "search", "new", "rename", "copy", "archive", "unarchive", "delete"]}));
    };
    let library = lib_root(a.flags.get("library"));
    let rest = a.positional.get(1).cloned().unwrap_or_default();
    let to = a.flags.get("to").cloned().unwrap_or_default();
    let result: Result<serde_json::Value, String> = (|| {
        match action.as_str() {
            "list" => {
                let q = a.flags.get("query").map(|s| s.as_str()).filter(|s| !s.is_empty());
                let include_archived = a.flags.contains_key("archived");
                let cards = library::list(&library, q, include_archived).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({
                    "library": library.to_string_lossy(),
                    "total": cards.len(),
                    "projects": cards.iter().map(cutforge_io::library::LibraryCard::to_value).collect::<Vec<_>>(),
                }))
            }
            "search" => {
                if rest.is_empty() {
                    return Err("用法: library search <关键词>".into());
                }
                let cards = library::list(&library, Some(&rest), a.flags.contains_key("archived"))
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::json!({"library": library.to_string_lossy(), "query": rest,
                    "total": cards.len(),
                    "projects": cards.iter().map(cutforge_io::library::LibraryCard::to_value).collect::<Vec<_>>()}))
            }
            "new" => {
                if rest.is_empty() {
                    return Err("用法: library new <工程名> [--layout v2|v3] [--slug S] [--fps 30] [--width 1080] [--height 1920] [--track video,audio]".into());
                }
                let layout = layout_of(a.flags.get("layout")).map_err(|e| e.to_string())?;
                let kinds = track_kinds(a.flags.get("track")).map_err(|e| e.to_string())?;
                let slug = a.flags.get("slug").cloned().unwrap_or_else(|| rest.clone());
                let fps = a.flags.get("fps").and_then(|s| s.parse().ok()).unwrap_or(30);
                let width = a.flags.get("width").and_then(|s| s.parse().ok()).unwrap_or(1080);
                let height = a.flags.get("height").and_then(|s| s.parse().ok()).unwrap_or(1920);
                let p = library::new_project(&library, library::LibraryNewSpec {
                    name: &rest, layout, slug: &slug, fps, width, height, kinds: &kinds,
                }).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({"name": rest, "project": p.to_string_lossy(),
                    "layout": if layout == LayoutKind::V3 { "v3" } else { "v2" }}))
            }
            "rename" => {
                if rest.is_empty() || to.is_empty() {
                    return Err("用法: library rename <旧名> --to <新名>".into());
                }
                let p = library::rename(&library, &rest, &to)?;
                Ok(serde_json::json!({"from": rest, "to": to, "project": p.to_string_lossy(),
                    "note": "slug 不随目录名改写(库操作目录级,OpLog 不动)"}))
            }
            "copy" => {
                if rest.is_empty() || to.is_empty() {
                    return Err("用法: library copy <源名> --to <目标名>".into());
                }
                let p = library::copy_project(&library, &rest, &to)?;
                Ok(serde_json::json!({"from": rest, "to": to, "project": p.to_string_lossy()}))
            }
            "archive" | "unarchive" | "delete" => {
                if rest.is_empty() {
                    return Err(format!("用法: library {action} <工程名>"));
                }
                let p = match action.as_str() {
                    "archive" => library::archive(&library, &rest)?,
                    "unarchive" => library::unarchive(&library, &rest)?,
                    _ => library::delete(&library, &rest)?,
                };
                Ok(serde_json::json!({"name": rest, "action": action, "at": p.to_string_lossy()}))
            }
            other => Err(format!("未知 action: {other}(允许 list/search/new/rename/copy/archive/unarchive/delete)")),
        }
    })();
    match result {
        Ok(data) => emit(a.json, true, "OK", "工程库操作完成", data),
        Err(e) => emit(a.json, false, "PRECONDITION_FAILED", &e, serde_json::json!({})),
    }
}

/// `migrate <工程目录> --to v3`(册六 ADR-0021):一次性迁移,幂等 NOOP,冲突整体拒绝。
pub fn migrate_cmd(a: &crate::Args) -> i32 {
    let Some(root) = a.positional.first() else {
        return emit(a.json, false, "PRECONDITION_FAILED", "用法: migrate <工程目录> --to v3", serde_json::json!({}));
    };
    if a.flags.get("to").map(|s| s.as_str()) != Some("v3") {
        return emit(a.json, false, "PRECONDITION_FAILED",
            "--to 必须为 v3(当前唯一目标布局;v2/v1 兼容读写冻结,不做回迁,ADR-0021)",
            serde_json::json!({}));
    }
    match mig::migrate_to_v3(Path::new(root)) {
        Ok(r) => {
            emit(a.json, true, "OK",
                if r.idempotent { "已是 v3 布局,幂等 NOOP" } else { "迁移完成(v3)" },
                serde_json::json!({
                    "from": r.from, "idempotent": r.idempotent,
                    "moved": r.moved.iter().map(|(f, t)| serde_json::json!({"from": f, "to": t}))
                        .collect::<Vec<_>>(),
                    "removedEmptyDirs": r.removed_empty_dirs,
                    "kept": r.kept,
                }))
        }
        Err(mig::MigrateError::NotAProject(_)) => {
            emit(a.json, false, "NO_CONFIG", &format!("不是可打开的工程: {root}"), serde_json::json!({}))
        }
        Err(mig::MigrateError::Conflict(m)) => emit(a.json, false, "CONFLICT", &m, serde_json::json!({})),
        Err(e) => emit(a.json, false, "PRECONDITION_FAILED", &e.to_string(), serde_json::json!({})),
    }
}

/// `recover [<工程目录>] [--library R]`:无位置参数 = 列库内可恢复工程;
/// 给工程目录 = 执行恢复(清残留锁 + OpLog 一致性校验)。
pub fn recover_cmd(a: &crate::Args) -> i32 {
    match a.positional.first() {
        Some(root) => match rec::recover(Path::new(root)) {
            Ok(r) => emit(a.json, true, "OK", "恢复完成(残留锁已清,OpLog 一致性校验通过)", serde_json::json!({
                "root": r.root.to_string_lossy(), "lockCleared": r.lock_cleared,
                "rev": r.rev, "opCount": r.op_count, "session": r.session,
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                emit(a.json, false, "NO_CONFIG", &format!("不是可打开的工程: {root}"), serde_json::json!({}))
            }
            Err(e) => emit(a.json, false, "PRECONDITION_FAILED", &e.to_string(), serde_json::json!({})),
        },
        None => {
            let library = lib_root(a.flags.get("library"));
            let stale = rec::scan_stale(&library);
            emit(a.json, true, "OK", "可恢复工程清单", serde_json::json!({
                "library": library.to_string_lossy(),
                "total": stale.len(),
                "stale": stale.iter().map(|s| serde_json::json!({
                    "root": s.root.to_string_lossy(),
                    "pid": s.pid, "pidAlive": s.pid_alive,
                    "ageMs": s.age_ms as u64, "session": s.session,
                })).collect::<Vec<_>>(),
                "hint": "执行恢复: cutforge-cli recover <工程目录>",
            }))
        }
    }
}
