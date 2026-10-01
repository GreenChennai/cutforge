// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 库面工具(册六 T6.1):migrate_layout / library_manage / library_list / library_recover。
//! 全部走 cutforge_io 单一实现(migrate/library/recover 模块),不旁路自写;
//! `root` 语义:migrate_layout = 工程目录,library_* = 库根目录(env CUTFORGE_PROJECTS
//! 缺省面的 CLI/壳共用同一解析)。

use crate::registry::envelope;
use cutforge_io::{library, migrate as mig, paths, recover as rec, LayoutKind};
use serde_json::{json, Value};
use std::path::Path;

/// `migrate_layout`(写,免锁面:迁移器内部自持锁):V1/V2 → V3 一次性迁移,
/// v3 工程 = 幂等 NOOP;映射目标已存在 = CONFLICT 整体拒绝。
pub(crate) fn migrate_layout_tool(root: &Path, args: &Value) -> Value {
    if args["to"].as_str() != Some("v3") {
        return envelope(false, "PRECONDITION_FAILED",
            "--to 必须为 v3(当前唯一目标布局;v2/v1 兼容读写冻结,不做回迁,ADR-0021)", json!({}));
    }
    match mig::migrate_to_v3(root) {
        Ok(r) => envelope(true, "OK",
            if r.idempotent { "已是 v3 布局,幂等 NOOP" } else { "迁移完成(v3)" },
            json!({
                "from": r.from,
                "to": "v3",
                "idempotent": r.idempotent,
                "moved": r.moved.iter().map(|(f, t)| json!({"from": f, "to": t})).collect::<Vec<_>>(),
                "removedEmptyDirs": r.removed_empty_dirs,
                "kept": r.kept,
            })),
        Err(mig::MigrateError::NotAProject(_)) => {
            envelope(false, "NO_CONFIG", &format!("不是可打开的工程: {}", root.display()), json!({}))
        }
        Err(mig::MigrateError::Conflict(m)) => envelope(false, "CONFLICT", &m, json!({})),
        Err(e) => envelope(false, "PRECONDITION_FAILED", &e.to_string(), json!({})),
    }
}

/// `library_manage`(写,action 参数化):new/rename/copy/archive/unarchive/delete/
/// export_cfproj(T6.4:导出 `.cfproj` 工程描述,关联打开面)。
/// 目录级移动/复制,OpLog 完整性不动;活进程持锁的工程拒绝移动。
pub(crate) fn library_manage_tool(root: &Path, args: &Value) -> Value {
    let Some(action) = args["action"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED",
            "缺 action(new|rename|copy|archive|unarchive|delete|export_cfproj)", json!({}));
    };
    let name = args["name"].as_str().unwrap_or_default();
    let to = args["to"].as_str().unwrap_or_default();
    let res: Result<Value, String> = (|| {
        match action {
            "new" => {
                if name.is_empty() {
                    return Err("缺 name(库内工程名)".into());
                }
                let layout = match args["layout"].as_str().unwrap_or("v2") {
                    "v2" => LayoutKind::V2,
                    "v3" => LayoutKind::V3,
                    other => return Err(format!("未知布局: {other}(允许 v2/v3,ADR-0021)")),
                };
                let mut kinds = Vec::new();
                for v in args["tracks"].as_array().cloned().unwrap_or_default() {
                    let k = match v.as_str() {
                        Some("video") => cutforge_core::model::TrackKind::Video,
                        Some("audio") => cutforge_core::model::TrackKind::Audio,
                        Some("text") => cutforge_core::model::TrackKind::Text,
                        Some("adjust") => cutforge_core::model::TrackKind::Adjust,
                        other => return Err(format!("未知轨道类型: {other:?}(允许 video/audio/text/adjust)")),
                    };
                    kinds.push(k);
                }
                if kinds.is_empty() {
                    kinds = vec![cutforge_core::model::TrackKind::Video, cutforge_core::model::TrackKind::Audio];
                }
                let slug = args["slug"].as_str().unwrap_or(name);
                let fps = args["fps"].as_u64().unwrap_or(30) as u32;
                let width = args["canvasW"].as_u64().unwrap_or(1080) as u32;
                let height = args["canvasH"].as_u64().unwrap_or(1920) as u32;
                let p = library::new_project(root, library::LibraryNewSpec {
                    name, layout, slug, fps, width, height, kinds: &kinds,
                }).map_err(|e| e.to_string())?;
                Ok(json!({"name": name, "project": p.to_string_lossy(),
                    "layout": if layout == LayoutKind::V3 { "v3" } else { "v2" }}))
            }
            "rename" => {
                if to.is_empty() {
                    return Err("缺 to(目标名)".into());
                }
                let p = library::rename(root, name, to)?;
                Ok(json!({"from": name, "to": to, "project": p.to_string_lossy(),
                    "note": "slug 不随目录名改写(库操作目录级,OpLog 不动)"}))
            }
            "copy" => {
                if to.is_empty() {
                    return Err("缺 to(目标名)".into());
                }
                let p = library::copy_project(root, name, to)?;
                Ok(json!({"from": name, "to": to, "project": p.to_string_lossy()}))
            }
            "archive" | "unarchive" | "delete" => {
                if name.is_empty() {
                    return Err(format!("缺 name({action} 的对象)"));
                }
                let p = match action {
                    "archive" => library::archive(root, name)?,
                    "unarchive" => library::unarchive(root, name)?,
                    _ => library::delete(root, name)?,
                };
                Ok(json!({"name": name, "action": action, "at": p.to_string_lossy()}))
            }
            "export_cfproj" => {
                if name.is_empty() {
                    return Err("缺 name(export_cfproj 的对象)".into());
                }
                let out = args["out"].as_str().filter(|s| !s.is_empty()).map(Path::new);
                let s = library::export_cfproj(root, name, out)?;
                Ok(json!({"name": s.name, "cfproj": s.path.to_string_lossy(),
                    "root": s.root.to_string_lossy(), "rev": s.rev,
                    "note": "描述文件不是工程真相源(root 指向工程目录);关联打开: cutforge-cli serve <该文件> --open"}))
            }
            other => Err(format!("未知 action: {other}(允许 new/rename/copy/archive/unarchive/delete/export_cfproj)")),
        }
    })();
    match res {
        Ok(data) => envelope(true, "OK", "工程库操作完成", data),
        Err(m) => envelope(false, "PRECONDITION_FAILED", &m, json!({})),
    }
}

/// `library_list`(查询,只读不持锁):卡片元数据从 project.json 轻量派生
/// (slug/fps/画幅/时长=IR 投影/rev/修改时间/缩略图);不逐工程 ffprobe。
pub(crate) fn library_list_tool(root: &Path, args: &Value) -> Value {
    let query = args["query"].as_str().filter(|s| !s.is_empty());
    let include_archived = args["includeArchived"].as_bool().unwrap_or(false);
    match library::list(root, query, include_archived) {
        Ok(cards) => envelope(true, "OK", "工程库清单", json!({
            "library": root.to_string_lossy(),
            "total": cards.len(),
            "projects": cards.iter().map(library::LibraryCard::to_value).collect::<Vec<_>>(),
        })),
        Err(e) => envelope(false, "PRECONDITION_FAILED", &e.to_string(), json!({})),
    }
}

/// `library_recover`(写):action=list 列库内可恢复工程(残留锁证据);
/// action=recover + name 对指定工程执行恢复(清残留锁 + OpLog 一致性校验)。
pub(crate) fn library_recover_tool(root: &Path, args: &Value) -> Value {
    match args["action"].as_str().unwrap_or("list") {
        "list" => {
            let stale = rec::scan_stale(root);
            envelope(true, "OK", "可恢复工程清单", json!({
                "library": root.to_string_lossy(),
                "total": stale.len(),
                "stale": stale.iter().map(|s| json!({
                    "root": s.root.to_string_lossy(),
                    "pid": s.pid, "pidAlive": s.pid_alive,
                    "ageMs": s.age_ms as u64, "session": s.session,
                })).collect::<Vec<_>>(),
            }))
        }
        "recover" => {
            let Some(name) = args["name"].as_str().filter(|s| !s.is_empty()) else {
                return envelope(false, "PRECONDITION_FAILED", "recover 需 name(库内工程名)", json!({}));
            };
            let project = root.join(name);
            if !paths::has_project(&project) {
                return envelope(false, "NO_CONFIG", &format!("工程不存在: {name}"), json!({}));
            }
            match rec::recover(&project) {
                Ok(r) => envelope(true, "OK", "恢复完成(残留锁已清,OpLog 一致性校验通过)", json!({
                    "root": r.root.to_string_lossy(), "lockCleared": r.lock_cleared,
                    "rev": r.rev, "opCount": r.op_count, "session": r.session,
                })),
                Err(e) => envelope(false, "PRECONDITION_FAILED", &e.to_string(), json!({})),
            }
        }
        other => envelope(false, "PRECONDITION_FAILED",
            &format!("未知 action: {other}(允许 list/recover)"), json!({})),
    }
}
