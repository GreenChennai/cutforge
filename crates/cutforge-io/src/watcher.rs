// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 文件监听(计划书 4.5,M2 基础版:轮询 + 去抖)。
//!
//! 忽略规则:`.cutforge/oplog/*`(自己写的)、`*.tmp`、`_内部状态/backup/*`
//! (0.4.x 旧布局 `_state/backup/*` 同样忽略)、`06_成片输出/*`(大产物)、
//! CutFlow 记账文件(`05_时间线工程/pipeline.json`、`_内部状态/*.json`,
//! O7/RT-5:跑阶段写的簿记不应给编辑器推假变更)。
//! 真相判定不依赖事件——rev 才是真相(M3 起接三路合并)。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 内部状态目录:新旧布局(0.4.x `_state` / 0.5 `_内部状态`)都伺候。
const STATE_DIRS: [&str; 2] = [crate::paths::LEGACY_STATE, crate::paths::STATE];
/// 成片输出目录:新旧布局都忽略(大产物)。
const OUTPUT_DIRS: [&str; 2] = [crate::paths::LEGACY_OUTPUT, crate::paths::OUTPUT];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Created,
    Modified,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEvent {
    pub path: PathBuf,
    pub kind: EventKind,
    pub ts_ms: u64,
}

type Snapshot = BTreeMap<PathBuf, (u64, u64)>; // path → (len, mtime_ms)

fn ignored(rel: &Path) -> bool {
    let s = rel.to_string_lossy().replace('\\', "/");
    if s.contains(".cutforge/oplog/") || s.ends_with(".cutforge/lock") {
        return true;
    }
    if OUTPUT_DIRS.iter().any(|d| s.starts_with(&format!("{d}/"))) {
        return true;
    }
    // O7/RT-5:CutFlow 记账文件——rs_run 每个阶段都会写 `_内部状态/S*.json`
    // (旧布局 `_state/S*.json`),`05_时间线工程/pipeline.json`(旧 `05_ir/`)是
    // 管线清单;这些不是工程真相源的编辑,不得惊动编辑器。
    if s == format!("{}/pipeline.json", crate::paths::TIMELINE)
        || s == format!("{}/pipeline.json", crate::paths::LEGACY_TIMELINE)
    {
        return true;
    }
    if STATE_DIRS.iter().any(|d| s.starts_with(&format!("{d}/"))) && s.ends_with(".json") {
        return true;
    }
    s.ends_with(".tmp")
}

fn mtime_ms(m: &std::fs::Metadata) -> u64 {
    use std::time::UNIX_EPOCH;
    m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn scan(root: &Path) -> Snapshot {
    let mut snap = Snapshot::new();
    walk(root, root, &mut snap);
    fn walk(root: &Path, dir: &Path, out: &mut Snapshot) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            let p = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                walk(root, &p, out);
            } else {
                let rel = p.strip_prefix(root).unwrap_or(&p).to_path_buf();
                if ignored(&rel) {
                    continue;
                }
                out.insert(rel, (meta.len(), mtime_ms(&meta)));
            }
        }
    }
    snap
}

/// 轮询式 watcher:`poll` 对比上次快照产出事件;两次 poll 间隔即去抖窗口。
pub struct Watcher {
    root: PathBuf,
    last: Snapshot,
    pub debounce: Duration,
}

impl Watcher {
    pub fn new(root: &Path, debounce_ms: u64) -> Self {
        Self { root: root.to_path_buf(), last: scan(root), debounce: Duration::from_millis(debounce_ms) }
    }

    /// 对比并产出事件(建议调用间隔 ≥ debounce)。
    pub fn poll(&mut self) -> Vec<FileEvent> {
        let cur = scan(&self.root);
        let mut events = Vec::new();
        for (rel, sig) in &cur {
            match self.last.get(rel) {
                None => events.push((rel.clone(), EventKind::Created)),
                Some(old) if old != sig => events.push((rel.clone(), EventKind::Modified)),
                _ => {}
            }
        }
        for rel in self.last.keys() {
            if !cur.contains_key(rel) {
                events.push((rel.clone(), EventKind::Removed));
            }
        }
        self.last = cur;
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        events
            .into_iter()
            .map(|(rel, kind)| FileEvent { path: self.root.join(rel), kind, ts_ms: ts })
            .collect()
    }
}

// ---------------- M9-2:常驻同步守护(事件源) ----------------

/// 同步事件 Hub:守护线程每次外部可见变更时 bump;HTTP /events 长轮询等消费。
pub struct SyncHub {
    seq: std::sync::Mutex<u64>,
    cv: std::sync::Condvar,
}

impl SyncHub {
    fn new() -> Self {
        Self { seq: std::sync::Mutex::new(0), cv: std::sync::Condvar::new() }
    }

    pub fn current(&self) -> u64 {
        *self.seq.lock().unwrap()
    }

    fn bump(&self) -> u64 {
        let mut s = self.seq.lock().unwrap();
        *s += 1;
        self.cv.notify_all();
        *s
    }

    /// 长轮询:等待 seq > since,最多 timeout;返回最新 seq(无新事件返回 None)。
    pub fn wait_since(&self, since: u64, timeout: std::time::Duration) -> Option<u64> {
        let deadline = std::time::Instant::now() + timeout;
        let mut s = self.seq.lock().unwrap();
        loop {
            if *s > since {
                return Some(*s);
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return None;
            }
            let (guard, _) = self.cv.wait_timeout(s, deadline - now).unwrap();
            s = guard;
        }
    }
}

/// 每个工程根一个守护线程(进程级注册表,重复调用返回既有 Hub)。
/// 线程职责:轮询 → project.json 外部可见变更 → 锁内 merge_from_disk → bump。
/// 约束(M8-R5):只做短临界区读合并,绝不在此线程做长时间写。
pub fn ensure_sync_daemon(root: &Path) -> std::sync::Arc<SyncHub> {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static REGISTRY: OnceLock<Mutex<BTreeMap<PathBuf, Arc<SyncHub>>>> = OnceLock::new();
    let reg = REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut map = reg.lock().unwrap();
    if let Some(hub) = map.get(root) {
        return hub.clone();
    }
    let hub = Arc::new(SyncHub::new());
    map.insert(root.to_path_buf(), hub.clone());
    let thread_root = root.to_path_buf();
    let thread_hub = hub.clone();
    std::thread::spawn(move || {
        let mut watcher = Watcher::new(&thread_root, 250);
        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let events = watcher.poll();
            let project_touched = events
                .iter()
                .any(|e| e.path.file_name().is_some_and(|n| n == "project.json"));
            if !project_touched {
                continue;
            }
            // 外部改动可见性:锁内合并(短临界区),冲突落盘不打断守护
            if let Ok(mut ws) = crate::Workspace::open_exclusive(&thread_root) {
                let _ = ws.merge_from_disk();
            }
            thread_hub.bump();
        }
    });
    hub
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil;
    use crate::paths;

    #[test]
    fn watcher_reports_changes_and_honors_ignore_rules() {
        let root = fsutil::temp_dir("cutforge-watch");
        fsutil::ensure(&root.join(paths::TIMELINE)).unwrap();
        fsutil::ensure(&root.join(".cutforge/oplog")).unwrap();
        crate::atomic::atomic_write(&root.join(paths::PROJECT_REL), b"{}").unwrap();
        let mut w = Watcher::new(&root, 50);
        assert!(w.poll().is_empty(), "初扫不产出事件");

        // 修改 + 忽略面(新布局)
        crate::atomic::atomic_write(&root.join(paths::PROJECT_REL), b"{\"a\":1}").unwrap();
        crate::atomic::atomic_write(&root.join(".cutforge/oplog/20260918.jsonl"), b"{}\n").unwrap();
        crate::atomic::atomic_write(&root.join(paths::TIMELINE).join("tmp.tmp"), b"x").unwrap();
        // O7/RT-5:CutFlow 记账文件必须被忽略(rs_run 每阶段都会写,不是编辑变更)
        crate::atomic::atomic_write(&root.join(paths::TIMELINE).join("pipeline.json"), b"{}").unwrap();
        fsutil::ensure(&root.join(paths::STATE)).unwrap();
        crate::atomic::atomic_write(&root.join(paths::STATE).join("S3.json"), b"{}").unwrap();
        // 旧布局(0.4.x)同名记账/产物/备份同样忽略
        crate::atomic::atomic_write(&root.join(paths::LEGACY_TIMELINE).join("pipeline.json"), b"{}").unwrap();
        crate::atomic::atomic_write(&root.join(paths::LEGACY_STATE).join("S4.json"), b"{}").unwrap();
        crate::atomic::atomic_write(&root.join(paths::LEGACY_OUTPUT).join("final_x.mp4"), b"x").unwrap();
        crate::atomic::atomic_write(&root.join(paths::OUTPUT).join("final_x.mp4"), b"x").unwrap();
        let ev = w.poll();
        assert_eq!(ev.len(), 1, "oplog/*.tmp/CutFlow 记账文件(新旧布局)/产物必须被忽略: {ev:?}");
        assert_eq!(ev[0].kind, EventKind::Modified);
        assert!(ev[0].path.ends_with("project.json"));

        // 删除(project.json 消失 → Removed;tmp 被忽略,从不进快照,不报事件)
        crate::atomic::remove(&root.join(paths::PROJECT_REL)).unwrap();
        let ev = w.poll();
        assert_eq!(ev.len(), 1, "只有 project.json 的 Removed: {ev:?}");
        assert_eq!(ev[0].kind, EventKind::Removed);
        fsutil::cleanup(&root);
    }
}
