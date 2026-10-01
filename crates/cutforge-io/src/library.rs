// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 工程库(册六 T6.1,独立化核心):文件夹式单机工程库(达芬奇 Project Manager 简化版,
//! 无数据库)。
//!
//! - 库根:env `CUTFORGE_PROJECTS`(与 serve 交互选工程的搜索根同一约定,单一环境变量)
//!   → 缺省 `%USERPROFILE%\CutForge\Projects\`(USERPROFILE 缺失回退 HOME)。
//! - 操作全部是**目录级**移动/复制/改名:OpLog/rev/notes/`.cutforge/` 随目录整体走,
//!   完整性零影响(AC-6.1);不产 Op、不改 IR——库操作不是编辑。
//! - 归档 `.archives/`、删除 `.trash/<时间戳>-<名>/`(可捞回,不直接物理删除——
//!   个人库的删除后悔药是目录级操作能给的最低成本保险)。
//! - 卡片元数据从 project.json **轻量派生**(slug/fps/画幅/时长/rev/修改时间/缩略图),
//!   不逐工程 ffprobe(列表 N 工程 × 探测成本不可控,与 media_browse 的 PROBE_CAP 同理由);
//!   时长 = IR 投影 max(startMs+durationMs),缩略图 = `.cutforge/thumb-cache` 最新产物。

use crate::fsutil;
use crate::paths::{self, LayoutKind};
use serde_json::Value;
use std::io;
use std::path::{Path, PathBuf};

/// 归档区(库根下的隐藏目录;unarchive 移回)。
pub const ARCHIVES_DIR: &str = ".archives";
/// 回收站(删除 = 移入 `.trash/<ts>-<名>/`;物理清理由用户执行)。
pub const TRASH_DIR: &str = ".trash";

/// 工程名(库内条目名)合法性:非空、无路径分隔符、不越级、不吃隐藏区。
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 120
        && name != paths::V3_MEDIA
        && name != paths::V3_EXPORTS
        && Path::new(name).file_name().is_some_and(|f| f == name)
        && !name.contains(['/', '\\'])
        && name.split(['/', '\\']).all(|seg| seg != "..")
        && !name.starts_with('.')
}

/// 库根解析:env `CUTFORGE_PROJECTS` → `%USERPROFILE%\CutForge\Projects` → HOME 回退。
pub fn library_root() -> PathBuf {
    if let Some(v) = std::env::var_os("CUTFORGE_PROJECTS")
        && !v.is_empty() {
            return PathBuf::from(v);
        }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home).join("CutForge").join("Projects");
    }
    PathBuf::from(".cutforge-library")
}

/// 工程卡片(库视图;元数据从 project.json 轻量派生,解析失败如实标 invalid)。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct LibraryCard {
    pub name: String,
    pub path: PathBuf,
    pub archived: bool,
    /// project.json 可解析且过最小形态检查。
    pub valid: bool,
    pub slug: Option<String>,
    pub fps: Option<u32>,
    pub canvas: Option<(u64, u64)>,
    pub clip_count: Option<u64>,
    /// IR 投影时长:max(startMs+durationMs)(毫秒;不逐工程 ffprobe)。
    pub duration_ms: Option<u64>,
    /// `.cutforge/rev` 内容(oplog 记账数;崩溃恢复面同源)。
    pub rev: Option<u64>,
    /// project.json mtime(epoch ms;排序与展示用)。
    pub modified_at_ms: Option<u128>,
    /// `.cutforge/thumb-cache` 最新缩略图绝对路径(无产物 → None)。
    pub thumbnail_path: Option<PathBuf>,
    /// 活进程持锁(工程在别处打开;移动类操作会拒绝)。
    pub locked: bool,
}

impl LibraryCard {
    /// 卡片 → JSON(canvas 归一为 {width,height} 对象;CLI/MCP 同源单一实现)。
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "path": self.path.to_string_lossy(),
            "archived": self.archived,
            "valid": self.valid,
            "slug": self.slug,
            "fps": self.fps,
            "canvas": self.canvas.map(|(w, h)| serde_json::json!({"width": w, "height": h})),
            "clipCount": self.clip_count,
            "durationMs": self.duration_ms,
            "rev": self.rev,
            "modifiedAtMs": self.modified_at_ms.map(|m| m as u64),
            "thumbnailPath": self.thumbnail_path.as_ref().map(|p| p.to_string_lossy()),
            "locked": self.locked,
        })
    }
}

fn dir_m(path: &Path) -> Option<u128> {
    Some(path.metadata().ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis())
}

/// 读一个工程目录的卡片元数据(project.json 轻量派生,不做完整契约校验)。
fn card_for(path: PathBuf, name: String, archived: bool) -> LibraryCard {
    let mut card = LibraryCard {
        name, path, archived, valid: false, slug: None, fps: None, canvas: None,
        clip_count: None, duration_ms: None, rev: None, modified_at_ms: None,
        thumbnail_path: None, locked: false,
    };
    let project_file = paths::project_path(&card.path);
    card.modified_at_ms = dir_m(&project_file);
    card.locked = lock_held(&card.path);
    let Ok(text) = std::fs::read_to_string(&project_file) else { return card };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return card };
    card.slug = v["slug"].as_str().map(String::from);
    card.fps = v["fps"].as_u64().map(|n| n as u32);
    card.canvas = match (&v["canvas"]["width"], &v["canvas"]["height"]) {
        (Value::Number(w), Value::Number(h)) => Some((w.as_u64().unwrap_or(0), h.as_u64().unwrap_or(0))),
        _ => None,
    };
    if let Some(tracks) = v["tracks"].as_array() {
        let mut clips = 0u64;
        let mut max_end = 0u64;
        let empty: Vec<Value> = Vec::new();
        for t in tracks {
            for c in t["clips"].as_array().unwrap_or(&empty) {
                clips += 1;
                let end = c["startMs"].as_u64().unwrap_or(0) + c["durationMs"].as_u64().unwrap_or(0);
                max_end = max_end.max(end);
            }
        }
        card.clip_count = Some(clips);
        card.duration_ms = Some(max_end);
    }
    card.rev = std::fs::read_to_string(card.path.join(".cutforge/rev"))
        .ok()
        .and_then(|t| t.trim().parse().ok());
    // 缩略图:.cutforge/thumb-cache 最新一张(媒体缩略图派生物,media_thumbnail 生成)
    let thumb_dir = card.path.join(".cutforge/thumb-cache");
    if let Ok(rd) = std::fs::read_dir(&thumb_dir) {
        let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
        for e in rd.flatten() {
            let ext = e.path().extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
            if !matches!(ext.as_str(), "png" | "jpg" | "jpeg") {
                continue;
            }
            if let Ok(meta) = e.metadata()
                && let Ok(mt) = meta.modified()
                && best.as_ref().is_none_or(|(t, _)| mt > *t) {
                    best = Some((mt, e.path()));
                }
        }
        card.thumbnail_path = best.map(|(_, p)| p);
    }
    card.valid = card.slug.is_some() && v["tracks"].is_array();
    card
}

/// 工程是否被**活进程**持锁(锁文件在且 pid 存活);崩溃残留锁不算(恢复面处理)。
pub fn lock_held(project: &Path) -> bool {
    let lock = project.join(".cutforge/lock");
    if !lock.is_file() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(&lock) else { return true };
    let pid = text.split_whitespace().find_map(|t| t.strip_prefix("pid=").and_then(|v| v.parse::<u32>().ok()));
    match pid {
        Some(p) => crate::probe::pid_alive(p),
        None => true,
    }
}

/// 列库内工程(顶层一层;归档区按需并入;query 过滤 name/slug 子串,大小写不敏感)。
pub fn list(library: &Path, query: Option<&str>, include_archived: bool) -> io::Result<Vec<LibraryCard>> {
    let mut cards = Vec::new();
    for (dir, archived) in [(library.to_path_buf(), false), (library.join(ARCHIVES_DIR), true)] {
        if archived && !include_archived {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() || p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if !paths::has_project(&p) {
                continue; // 库内只列工程;散文件/半成品忽略
            }
            cards.push(card_for(p, name, archived));
        }
    }
    if let Some(q) = query {
        let q = q.to_lowercase();
        cards.retain(|c| {
            c.name.to_lowercase().contains(&q) || c.slug.as_deref().is_some_and(|s| s.to_lowercase().contains(&q))
        });
    }
    cards.sort_by(|a, b| b.modified_at_ms.unwrap_or(0).cmp(&a.modified_at_ms.unwrap_or(0)).then(a.name.cmp(&b.name)));
    Ok(cards)
}

/// 移动类操作的公共前置:名字合法、条目在位、无活进程锁、目标不冲突。
fn check_move(library: &Path, name: &str, target: &Path) -> Result<PathBuf, String> {
    if !valid_name(name) {
        return Err(format!("非法工程名: {name}"));
    }
    let src = library.join(name);
    if !paths::has_project(&src) {
        return Err(format!("工程不存在: {name}"));
    }
    if lock_held(&src) {
        return Err(format!("工程被活进程锁定(先关闭或恢复): {name}"));
    }
    if target.exists() {
        return Err(format!("目标已存在: {}", target.display()));
    }
    Ok(src)
}

fn move_dir(src: &Path, target: &Path) -> io::Result<()> {
    if let Some(parent) = target.parent() {
        fsutil::ensure(parent)?;
    }
    // 落盘点纪律:目录级搬移走 atomic::rename(结构性原语,册六 T6.1 收敛)
    crate::atomic::rename(src, target)
}

/// 新建工程入参(册六:参数打包,库面单一入口)。
pub struct LibraryNewSpec<'a> {
    pub name: &'a str,
    pub layout: LayoutKind,
    pub slug: &'a str,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    pub kinds: &'a [cutforge_core::model::TrackKind],
}

/// 新建工程到库(与 scaffold 单一实现;layout 显式给定,缺省面见 ADR-0021 决策 4)。
pub fn new_project(library: &Path, spec: LibraryNewSpec<'_>) -> io::Result<PathBuf> {
    if !valid_name(spec.name) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("非法工程名: {}", spec.name)));
    }
    let root = library.join(spec.name);
    if root.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("库内已存在: {}", spec.name)));
    }
    fsutil::ensure(library)?;
    crate::scaffold::scaffold_project_layout(
        &root, spec.slug, spec.fps, spec.width, spec.height, spec.kinds, spec.layout,
    )?;
    Ok(root) // 库条目语义:返回工程根(scaffold 返回工程文件路径)
}

/// 重命名(目录级改名;project.json 的 slug **不随改**——slug 是导出文件名语义,
/// 改它属编辑面,库操作保持"目录级 + OpLog 不动"纪律,报告方如实区分)。
pub fn rename(library: &Path, from: &str, to: &str) -> Result<PathBuf, String> {
    if !valid_name(to) {
        return Err(format!("非法目标名: {to}"));
    }
    let target = library.join(to);
    let src = check_move(library, from, &target)?;
    move_dir(&src, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

/// 复制工程(目录级递归复制;排除活锁与重型派生物缓存——副本按需重建,
/// oplog/rev/notes/快照随拷,撤销链完整)。
pub fn copy_project(library: &Path, from: &str, to: &str) -> Result<PathBuf, String> {
    if !valid_name(to) {
        return Err(format!("非法目标名: {to}"));
    }
    let target = library.join(to);
    let src = check_move(library, from, &target)?;
    copy_tree(&src, &target, &src).map_err(|e| e.to_string())?; // (dir, dest_root, src_root)
    Ok(target)
}

fn copy_tree(dir: &Path, dest_root: &Path, src_root: &Path) -> io::Result<()> {
    for e in std::fs::read_dir(dir)?.flatten() {
        let p = e.path();
        let rel = p.strip_prefix(src_root).unwrap_or(&p).to_path_buf();
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        // 复制排除面:活锁 + 可重建的重型派生物(渲染缓存/代理/缩略图/波形/示波器)
        if rel_str == ".cutforge/lock"
            || [".cutforge/render-cache", ".cutforge/proxy", ".cutforge/thumb-cache", ".cutforge/peaks-cache", ".cutforge/scope-cache"]
                .iter().any(|d| rel_str == *d || rel_str.starts_with(&format!("{d}/")))
        {
            continue;
        }
        let dst = dest_root.join(&rel);
        if p.is_dir() {
            fsutil::ensure(&dst)?;
            copy_tree(&p, dest_root, src_root)?;
        } else {
            fsutil::copy_file(&p, &dst)?;
        }
    }
    Ok(())
}

/// 归档(移入 `.archives/`;同名冲突拒绝)。
pub fn archive(library: &Path, name: &str) -> Result<PathBuf, String> {
    let target = library.join(ARCHIVES_DIR).join(name);
    let src = check_move(library, name, &target)?;
    move_dir(&src, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

/// 取消归档(移回库根;同名冲突拒绝)。
pub fn unarchive(library: &Path, name: &str) -> Result<PathBuf, String> {
    if !valid_name(name) {
        return Err(format!("非法工程名: {name}"));
    }
    let src = library.join(ARCHIVES_DIR).join(name);
    if !paths::has_project(&src) {
        return Err(format!("归档区无此工程: {name}"));
    }
    let target = library.join(name);
    if target.exists() {
        return Err(format!("库根已存在同名工程: {name}"));
    }
    move_dir(&src, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

/// 删除 = 移入 `.trash/<时间戳>-<名>/`(可捞回;报告真实落点,物理清理由用户决定)。
pub fn delete(library: &Path, name: &str) -> Result<PathBuf, String> {
    if !valid_name(name) {
        return Err(format!("非法工程名: {name}"));
    }
    let src = library.join(name);
    if !paths::has_project(&src) {
        return Err(format!("工程不存在: {name}"));
    }
    if lock_held(&src) {
        return Err(format!("工程被活进程锁定(先关闭或恢复): {name}"));
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let target = library.join(TRASH_DIR).join(format!("{ts}-{name}"));
    move_dir(&src, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

/// 原子写导入(library 模块内唯一写盘点纪律:测试造盘走 atomic)。
#[cfg(test)]
pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    crate::atomic::atomic_write(path, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::V3_PROJECT_REL;

    fn make_project(dir: &Path, slug: &str) {
        let kinds = [cutforge_core::model::TrackKind::Video, cutforge_core::model::TrackKind::Audio];
        crate::scaffold::scaffold_project_layout(dir, slug, 30, 1080, 1920, &kinds, LayoutKind::V2).unwrap();
    }

    #[test]
    fn seven_ops_roundtrip() {
        let lib = fsutil::temp_dir("library-ops");
        // new
        let spec = |name: &'static str| LibraryNewSpec {
            name, layout: LayoutKind::V2, slug: name, fps: 30, width: 1080, height: 1920,
            kinds: &[cutforge_core::model::TrackKind::Video, cutforge_core::model::TrackKind::Audio],
        };
        let p = new_project(&lib, spec("工程甲")).unwrap();
        assert!(p.join(paths::PROJECT_REL).is_file());
        // 重名拒绝
        assert!(new_project(&lib, spec("工程甲")).is_err());
        // 非法名拒绝
        assert!(new_project(&lib, LibraryNewSpec { name: "../逃逸", ..spec("x") }).is_err());
        // list(卡片元数据)
        let cards = list(&lib, None, false).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].slug.as_deref(), Some("工程甲"));
        assert_eq!(cards[0].canvas, Some((1080, 1920)));
        assert!(cards[0].valid && !cards[0].archived && !cards[0].locked);
        // rename
        let renamed = rename(&lib, "工程甲", "工程乙").unwrap();
        assert!(renamed.join(paths::PROJECT_REL).is_file());
        assert!(!lib.join("工程甲").exists());
        // copy(完整目录含 .cutforge)
        crate::Workspace::open_exclusive(&renamed).unwrap(); // 造 .cutforge
        let copied = copy_project(&lib, "工程乙", "工程丙").unwrap();
        assert!(copied.join(paths::PROJECT_REL).is_file());
        assert!(copied.join(".cutforge").is_dir(), "oplog/rev 随拷");
        assert!(!copied.join(".cutforge/lock").exists(), "活锁不得随拷");
        // search
        assert_eq!(list(&lib, Some("乙"), false).unwrap().len(), 1);
        assert_eq!(list(&lib, Some("工程"), false).unwrap().len(), 2);
        // archive / unarchive
        archive(&lib, "工程丙").unwrap();
        assert!(lib.join(ARCHIVES_DIR).join("工程丙").join(paths::PROJECT_REL).is_file());
        assert!(list(&lib, None, false).unwrap().iter().all(|c| c.name != "工程丙"));
        let archived = list(&lib, None, true).unwrap();
        assert!(archived.iter().any(|c| c.name == "工程丙" && c.archived));
        unarchive(&lib, "工程丙").unwrap();
        assert!(lib.join("工程丙").join(paths::PROJECT_REL).is_file());
        // delete → .trash
        let trashed = delete(&lib, "工程丙").unwrap();
        assert!(trashed.join(paths::PROJECT_REL).is_file());
        assert!(trashed.starts_with(lib.join(TRASH_DIR)));
        assert!(!lib.join("工程丙").exists());
        fsutil::cleanup(&lib);
    }

    #[test]
    fn move_ops_refuse_locked_and_missing() {
        let lib = fsutil::temp_dir("library-lock");
        let root = lib.join("p");
        make_project(&root, "p");
        // 崩溃残留锁(pid 已死)不阻塞移动;活锁阻塞
        fsutil::ensure(&root.join(".cutforge")).unwrap();
        write_atomic(&root.join(".cutforge/lock"), b"pid=4194303 ts=1").unwrap();
        assert!(!lock_held(&root), "死 pid 残留锁不算活锁");
        rename(&lib, "p", "p2").unwrap();
        // 活锁:本进程 pid(guard 绑定存活到测试结束)
        let _guard = crate::lock::acquire(&lib.join("p2"), 60_000, 0).unwrap();
        assert!(lock_held(&lib.join("p2")));
        assert!(rename(&lib, "p2", "p3").is_err(), "活进程持锁必须拒绝移动");
        assert!(archive(&lib, "p2").is_err());
        assert!(delete(&lib, "p2").is_err());
        // 不存在的工程
        assert!(rename(&lib, "无此工程", "x").is_err());
        fsutil::cleanup(&lib);
    }

    #[test]
    fn card_flags_invalid_project_and_v3() {
        let lib = fsutil::temp_dir("library-card");
        // V3 工程
        let p = lib.join("v3p");
        crate::scaffold::scaffold_project_layout(&p, "扁平卡", 25, 1920, 1080, &[], LayoutKind::V3).unwrap();
        let cards = list(&lib, None, false).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].fps, Some(25));
        assert!(cards[0].valid);
        // 非 project.json 垃圾目录不入列
        fsutil::ensure(&lib.join("垃圾")).unwrap();
        write_atomic(&lib.join("垃圾/notes.json"), b"{}").unwrap();
        assert_eq!(list(&lib, None, false).unwrap().len(), 1);
        // 坏 project.json → invalid 卡片仍列出(诚实标 invalid)
        let bad = lib.join("bad");
        fsutil::ensure(&bad).unwrap();
        write_atomic(&bad.join(V3_PROJECT_REL), b"not json").unwrap();
        let cards = list(&lib, None, false).unwrap();
        let bad_card = cards.iter().find(|c| c.name == "bad").unwrap();
        assert!(!bad_card.valid, "坏工程如实标 invalid");
        fsutil::cleanup(&lib);
    }
}
