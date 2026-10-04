// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! `cutforge-cli cache {info,gc,clear}`(T1.5):渲染缓存治理面。
//!
//! 文件契约与 crates/cutforge-render/src/cache.rs 保持一致:
//! `<工程>/.cutforge/render-cache/{seg,mix,compose,overlay,sub}/tmp` + `cache-index.json`
//! (条目 = 输入键/文件/大小/创建时间/最近使用/命中数)。
//! 之所以是"按契约重读"而非直接调用渲染库:check-deps 门禁(计划书 2.3)规定
//! cutforge-cli 只许依赖 core/io/mcp,渲染库不可引入(依赖纪律优先于 DRY;
//! 契约漂移由两侧单测互为镜像锁定,变更须同步)。
//!
//! 删除一律走 cutforge_io::atomic::remove(check-write-paths M2-4 纪律)。

use crate::{Args, emit};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// 渲染缓存根(相对工程目录;与 cutforge-render 侧同契约)。
const CACHE_ROOT: &str = ".cutforge/render-cache";
/// 分层目录。
const LAYERS: [&str; 5] = ["seg", "mix", "compose", "overlay", "sub"];
/// 清单文件名。
const INDEX_FILE: &str = "cache-index.json";
/// gc 缺省容量上限:10GB。
const DEFAULT_CAPACITY_BYTES: u64 = 10 * 1024 * 1024 * 1024;
/// 孤儿文件(清单外)超龄线:新于此可能是并发渲染在写文件,不动。
const ORPHAN_TTL_SECS: u64 = 24 * 3600;

/// 清单条目(只消费渲染侧落盘的 cache-index.json;字段缺失按 0 兜底)。
/// `raw` 保留原始 JSON:gc 重写清单时透传,不丢 createdAt/hits/input 等字段。
#[derive(Debug, Clone)]
struct Entry {
    raw: Value,
    layer: String,
    key: String,
    file: String,
    size: u64,
    last_used_at: u64,
}

fn load_entries(cache_dir: &Path) -> Vec<Entry> {
    std::fs::read_to_string(cache_dir.join(INDEX_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| {
            v.get("entries")?.as_array().map(|a| {
                a.iter()
                    .filter_map(|e| {
                        Some(Entry {
                            raw: e.clone(),
                            layer: e.get("layer")?.as_str()?.to_string(),
                            key: e.get("key")?.as_str()?.to_string(),
                            file: e.get("file")?.as_str()?.to_string(),
                            size: e.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
                            last_used_at: e.get("lastUsedAt").and_then(|x| x.as_u64()).unwrap_or(0),
                        })
                    })
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_default()
}

fn save_entries(cache_dir: &Path, entries: &[Entry]) -> Result<(), String> {
    let doc = json!({
        "version": 1,
        "entries": entries.iter().map(|e| if e.raw.is_null() { json!({
            "layer": e.layer, "key": e.key, "file": e.file, "size": e.size,
            "lastUsedAt": e.last_used_at,
        }) } else { e.raw.clone() }).collect::<Vec<_>>(),
    });
    cutforge_io::atomic::atomic_write(&cache_dir.join(INDEX_FILE), doc.to_string().as_bytes())
        .map_err(|e| e.to_string())
}

/// 遍历缓存根下全部文件(绝对路径 + 大小;跳过清单自身)。
fn walk_files(root: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .file_name()
                .is_some_and(|n| n.to_string_lossy() != INDEX_FILE)
            {
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                out.push((p, size));
            }
        }
    }
    out
}

fn mtime_secs(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 相对路径归一化为正斜杠(与清单 `file` 字段约定一致;Windows strip_prefix 产反斜杠)。
fn normalize_rel(p: &Path, root: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// cache info:各层条目/体积 + 清单外孤儿统计(只读)。
fn info(cache_dir: &Path) -> Result<Value, String> {
    let entries = load_entries(cache_dir);
    let mut layers = Vec::new();
    for l in LAYERS {
        let es: Vec<&Entry> = entries.iter().filter(|e| e.layer == *l).collect();
        layers.push(json!({
            "layer": l,
            "entries": es.len(),
            "bytes": es.iter().map(|e| e.size).sum::<u64>(),
        }));
    }
    let indexed: std::collections::HashSet<&str> =
        entries.iter().map(|e| e.file.as_str()).collect();
    let mut orphan_files = 0usize;
    let mut orphan_bytes = 0u64;
    for (p, size) in walk_files(cache_dir) {
        let rel = normalize_rel(&p, cache_dir);
        if !indexed.contains(rel.as_str()) {
            orphan_files += 1;
            orphan_bytes += size;
        }
    }
    Ok(json!({
        "cacheDir": cache_dir.to_string_lossy(),
        "layers": layers,
        "orphans": {"files": orphan_files, "bytes": orphan_bytes},
        "totalBytes": entries.iter().map(|e| e.size).sum::<u64>(),
        "capacityBytes": DEFAULT_CAPACITY_BYTES,
    }))
}

/// cache gc:超龄孤儿先清,再按 LRU + 容量上限淘汰(与渲染侧 cache_gc 同策略)。
fn gc(cache_dir: &Path, capacity_bytes: u64) -> Result<Value, String> {
    let now = now_secs();
    let mut entries = load_entries(cache_dir);
    entries.retain(|e| cache_dir.join(&e.file).is_file());
    let indexed: std::collections::HashSet<&str> =
        entries.iter().map(|e| e.file.as_str()).collect();
    let mut removed = 0usize;
    let mut freed = 0u64;
    // 第一刀:超龄孤儿(旧版固定文件名遗留 / tmp 超龄件)
    for (p, size) in walk_files(cache_dir) {
        let rel = normalize_rel(&p, cache_dir);
        if indexed.contains(rel.as_str()) {
            continue;
        }
        if now.saturating_sub(mtime_secs(&p)) <= ORPHAN_TTL_SECS {
            continue;
        }
        cutforge_io::atomic::remove(&p).map_err(|e| e.to_string())?;
        removed += 1;
        freed += size;
    }
    // 第二刀:容量压力下 LRU(清单条目按 lastUsedAt;新孤儿按 mtime 一并参评)
    let mut candidates: Vec<(u64, String, u64)> = entries
        .iter()
        .map(|e| (e.last_used_at, e.file.clone(), e.size))
        .collect();
    for (p, size) in walk_files(cache_dir) {
        let rel = normalize_rel(&p, cache_dir);
        if !indexed.contains(rel.as_str()) {
            candidates.push((mtime_secs(&p), rel, size));
        }
    }
    candidates.sort_by_key(|(rank, rel, _)| (*rank, rel.clone()));
    let mut total: u64 = candidates.iter().map(|(_, _, s)| *s).sum();
    for (_, rel, size) in &candidates {
        if total <= capacity_bytes {
            break;
        }
        cutforge_io::atomic::remove(&cache_dir.join(rel)).map_err(|e| e.to_string())?;
        entries.retain(|e| e.file != *rel);
        total = total.saturating_sub(*size);
        removed += 1;
        freed += size;
    }
    entries.retain(|e| cache_dir.join(&e.file).is_file());
    save_entries(cache_dir, &entries)?;
    Ok(json!({
        "removed": removed,
        "freedBytes": freed,
        "remainingBytes": total,
        "capacityBytes": capacity_bytes,
    }))
}

/// cache clear --all:清空各层与 tmp 的全部文件(含清单;目录保留)。
fn clear(cache_dir: &Path) -> Result<Value, String> {
    let mut removed = 0usize;
    let mut freed = 0u64;
    for (p, size) in walk_files(cache_dir) {
        cutforge_io::atomic::remove(&p).map_err(|e| e.to_string())?;
        removed += 1;
        freed += size;
    }
    // 清单一并清空(空清单)
    save_entries(cache_dir, &[])?;
    Ok(json!({"removed": removed, "freedBytes": freed}))
}

/// 子命令入口。用法:`cache <工程目录> {info,gc,clear} [--max-gb N] [--all]`
/// (动作与工程目录先后顺序不敏感)。
pub fn run(a: &Args) -> i32 {
    const USAGE: &str = "用法: cache <工程目录> {info|gc|clear} [--max-gb N] [--all]";
    let action = a
        .positional
        .iter()
        .find(|p| matches!(p.as_str(), "info" | "gc" | "clear"));
    let root = a
        .positional
        .iter()
        .find(|p| !matches!(p.as_str(), "info" | "gc" | "clear"));
    let (Some(action), Some(root)) = (action, root) else {
        return emit(
            a.json,
            false,
            "PRECONDITION_FAILED",
            USAGE,
            serde_json::json!({}),
        );
    };
    let cache_dir = Path::new(root).join(CACHE_ROOT);
    match action.as_str() {
        "info" => match info(&cache_dir) {
            Ok(data) => emit(a.json, true, "OK", "渲染缓存统计", data),
            Err(e) => emit(a.json, false, "INTERNAL", &e, serde_json::json!({})),
        },
        "gc" => {
            let capacity = match a.flags.get("max-gb").map(|s| s.parse::<f64>()) {
                Some(Ok(gb)) if gb >= 0.0 => (gb * 1024.0 * 1024.0 * 1024.0) as u64,
                Some(Ok(_)) | Some(Err(_)) => {
                    return emit(
                        a.json,
                        false,
                        "PRECONDITION_FAILED",
                        "--max-gb 需非负数字",
                        serde_json::json!({}),
                    );
                }
                None => DEFAULT_CAPACITY_BYTES,
            };
            match gc(&cache_dir, capacity) {
                Ok(data) => emit(a.json, true, "OK", "缓存已按 LRU+容量清理", data),
                Err(e) => emit(a.json, false, "INTERNAL", &e, serde_json::json!({})),
            }
        }
        "clear" => {
            if !a.flags.contains_key("all") {
                return emit(
                    a.json,
                    false,
                    "PRECONDITION_FAILED",
                    "clear 为破坏性操作,必须显式 --all(清空全部层与清单)",
                    serde_json::json!({}),
                );
            }
            match clear(&cache_dir) {
                Ok(data) => emit(a.json, true, "OK", "缓存已清空", data),
                Err(e) => emit(a.json, false, "INTERNAL", &e, serde_json::json!({})),
            }
        }
        _ => unreachable!("action 已由 find 过滤"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempRoot(PathBuf);
    impl TempRoot {
        fn new(tag: &str) -> TempRoot {
            let dir =
                std::env::temp_dir().join(format!("cf-cli-cache-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("seg")).unwrap();
            std::fs::create_dir_all(dir.join("mix")).unwrap();
            TempRoot(dir)
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 写假缓存文件 + 追加清单条目(模拟渲染侧落盘形态)。
    fn put(root: &Path, entries: &mut Vec<Entry>, layer: &str, key: &str, size: usize, used: u64) {
        let rel = format!("{layer}/{key}.mp4");
        let data = vec![0u8; size];
        cutforge_io::atomic::atomic_write(&root.join(&rel), &data).unwrap();
        entries.push(Entry {
            raw: serde_json::json!({"layer": layer, "key": key, "file": rel, "size": size,
                "createdAt": used, "lastUsedAt": used, "hits": 0}),
            layer: layer.into(),
            key: key.into(),
            file: rel,
            size: size as u64,
            last_used_at: used,
        });
    }

    #[test]
    fn info_counts_layers_and_orphans() {
        let root = TempRoot::new("info");
        let mut entries = Vec::new();
        put(&root.0, &mut entries, "seg", "s1", 10, 1);
        put(&root.0, &mut entries, "seg", "s2", 20, 2);
        put(&root.0, &mut entries, "mix", "m1", 30, 3);
        save_entries(&root.0, &entries).unwrap();
        let junk = vec![0u8; 5];
        cutforge_io::atomic::atomic_write(&root.0.join("subbed.mp4"), &junk).unwrap();
        let doc = info(&root.0).unwrap();
        let layers = doc["layers"].as_array().unwrap();
        let seg = layers.iter().find(|l| l["layer"] == "seg").unwrap();
        assert_eq!(
            (
                seg["entries"].as_u64().unwrap(),
                seg["bytes"].as_u64().unwrap()
            ),
            (2, 30)
        );
        assert_eq!(
            doc["orphans"]["files"].as_u64().unwrap(),
            1,
            "旧版遗留应记为孤儿"
        );
        assert_eq!(doc["totalBytes"].as_u64().unwrap(), 60);
    }

    #[test]
    fn gc_enforces_capacity_with_lru_order() {
        let root = TempRoot::new("gc");
        let mut entries = Vec::new();
        put(&root.0, &mut entries, "seg", "old", 100, 10);
        put(&root.0, &mut entries, "mix", "mid", 100, 20);
        put(&root.0, &mut entries, "seg", "new", 100, 30);
        save_entries(&root.0, &entries).unwrap();
        let doc = gc(&root.0, 250).unwrap();
        assert_eq!(doc["removed"].as_u64().unwrap(), 1);
        assert_eq!(doc["remainingBytes"].as_u64().unwrap(), 200);
        let after = load_entries(&root.0);
        assert!(!after.iter().any(|e| e.key == "old"), "LRU 最老者被淘汰");
        assert_eq!(after.len(), 2);
        // 容量内零删除
        let doc2 = gc(&root.0, 250).unwrap();
        assert_eq!(doc2["removed"].as_u64().unwrap(), 0);
    }

    #[test]
    fn clear_removes_everything_including_index_entries() {
        let root = TempRoot::new("clear");
        let mut entries = Vec::new();
        put(&root.0, &mut entries, "seg", "s1", 64, 1);
        save_entries(&root.0, &entries).unwrap();
        let doc = clear(&root.0).unwrap();
        assert_eq!(doc["removed"].as_u64().unwrap(), 1);
        assert_eq!(doc["freedBytes"].as_u64().unwrap(), 64);
        assert!(load_entries(&root.0).is_empty(), "清空后清单为空");
        let doc2 = info(&root.0).unwrap();
        assert_eq!(doc2["totalBytes"].as_u64().unwrap(), 0);
    }
}
