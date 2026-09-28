//! 磁盘新鲜度指纹(T1.8/AC-1.8 性能专项):常驻工作区复用与守护免开的判别基础。
//!
//! 指纹覆盖 `Workspace::load` 的**全部磁盘输入**:project.json(`_meta` 含内)、
//! notes.json、三份非工程真相源(wordline/cutlist/cutlist.applied,按盘面布局)、
//! `.cutforge/rev` 内容与 `.cutforge/oplog` 文件清单(文件名 + 长度)。
//! 指纹一致 ⇒ 重新 open 将得到一致的 Workspace ⇒ 复用不改变任何语义;
//! 任一输入变化(外部改动 / 其他进程写入 / 崩溃恢复面)⇒ 指纹必变 ⇒ 走重开。
//! 字节级(长度 + FNV-1a 哈希),不依赖 mtime(mtime 在 Windows 上精度与
//! 触碰语义不可靠);单次指纹成本 ~0.5ms(1k 片段工程实测)。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// 单文件签名:字节长度 + FNV-1a 64 哈希(零依赖;长度先行,快速否决)。
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSig {
    len: u64,
    hash: u64,
}

/// 工程磁盘新鲜度指纹:`Workspace::load` 全部磁盘输入的紧凑摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskFingerprint {
    /// `.cutforge/rev` 文本(缺失 = 空串;rev 变化必然伴随成功写入)。
    rev: String,
    /// project.json(缺文件 = None,此时工作区本就打不开)。
    project: Option<FileSig>,
    /// notes.json(缺失 = None,与 load 的"缺失 = 空存储"口径一致)。
    notes: Option<FileSig>,
    /// 非工程真相源(wordline/cutlist/cutlist.applied;按盘面布局,缺失 = None)。
    truths: Vec<Option<FileSig>>,
    /// oplog 文件清单(文件名,字节长度;按名升序;目录缺失 = 空)。
    oplog: Vec<(String, u64)>,
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

impl DiskFingerprint {
    /// 指纹的紧凑十六进制键(下游缓存键消费;T2.4 render_frame 单帧缓存的
    /// 新鲜度输入)。对 Debug 表示再做一次 FNV-1a——指纹结构体字段私有,
    /// 消费方拿键不拿内容,键与指纹一一对应(同指纹同键,异指纹异键)。
    pub fn cache_key(&self) -> String {
        format!("{:016x}", fnv1a(format!("{self:?}").as_bytes()))
    }
}

fn sig_of(path: &Path) -> Option<FileSig> {
    let data = std::fs::read(path).ok()?;
    Some(FileSig { len: data.len() as u64, hash: fnv1a(&data) })
}

/// 计算工程磁盘指纹;project.json 不可读(无工程/不可达)→ None。
pub fn disk_fingerprint(root: &Path) -> Option<DiskFingerprint> {
    let project = sig_of(&crate::paths::project_path(root))?;
    let truths = crate::paths::FILE_TRUTHS_NEW
        .iter()
        .chain(crate::paths::FILE_TRUTHS_LEGACY.iter())
        .map(|(_, rel)| sig_of(&root.join(rel)))
        .collect();
    let oplog_dir = root.join(".cutforge/oplog");
    let mut oplog: Vec<(String, u64)> = std::fs::read_dir(&oplog_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((e.file_name().to_string_lossy().into_owned(), meta.len()))
        })
        .collect();
    oplog.sort();
    Some(DiskFingerprint {
        rev: std::fs::read_to_string(root.join(".cutforge/rev")).unwrap_or_default(),
        project: Some(project),
        notes: sig_of(&root.join(crate::paths::NOTES_REL)),
        truths,
        oplog,
    })
}

// ---------------- 本进程最近一次写入登记(守护免开判别) ----------------

/// 进程级登记:root → 最近一次**本进程 persist 完成**后的磁盘指纹。
/// 用途:同步守护(watcher)轮询到 project.json 变化时,若指纹与本登记一致,
/// 说明变化来自本进程已完成的写入——锁内合并必然空转,免开(仍照常 bump 事件);
/// 不一致(外部改动)→ 照旧 open_exclusive + merge_from_disk。
fn registry() -> &'static Mutex<BTreeMap<PathBuf, DiskFingerprint>> {
    static REG: OnceLock<Mutex<BTreeMap<PathBuf, DiskFingerprint>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// persist 完成后登记:以**当下磁盘实况**为准(自证写入已完成且一致)。
pub fn note_local_write(root: &Path) {
    if let Some(fp) = disk_fingerprint(root)
        && let Ok(mut map) = registry().lock() {
            map.insert(root.to_path_buf(), fp);
        }
}

/// 当前磁盘指纹是否等于本进程最近一次写入的指纹(一致 = 变化来自自己)。
pub fn is_last_local_write(root: &Path, fp: &DiskFingerprint) -> bool {
    registry()
        .lock()
        .map(|map| map.get(root).is_some_and(|last| last == fp))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{atomic, fsutil, paths};

    #[test]
    fn fingerprint_stable_and_sensitive() {
        let root = fsutil::temp_dir("cutforge-fresh");
        fsutil::ensure(&root.join(paths::TIMELINE)).unwrap();
        fsutil::ensure(&root.join(".cutforge/oplog")).unwrap();
        atomic::atomic_write(&root.join(paths::PROJECT_REL), b"{\"a\":1}").unwrap();
        let f1 = disk_fingerprint(&root).expect("project 在盘,指纹必有");
        let f1b = disk_fingerprint(&root).expect("重复计算应一致");
        assert_eq!(f1, f1b, "盘面未变,指纹必须稳定");

        // project.json 变 → 指纹变
        atomic::atomic_write(&root.join(paths::PROJECT_REL), b"{\"a\":2}").unwrap();
        assert_ne!(f1, disk_fingerprint(&root).unwrap(), "project 字节变,指纹必变");

        // rev 变 → 指纹变(同长度也必须区分:哈希而非仅长度)
        atomic::atomic_write(&root.join(".cutforge/rev"), b"1\n").unwrap();
        let f2 = disk_fingerprint(&root).unwrap();
        atomic::atomic_write(&root.join(".cutforge/rev"), b"2\n").unwrap();
        assert_ne!(f2, disk_fingerprint(&root).unwrap(), "rev 同长度内容变,指纹必变");

        // oplog 追增(新文件/同文件变长)→ 指纹变
        atomic::append_line(&root.join(".cutforge/oplog/20260927.jsonl"), "{}\n").unwrap();
        assert_ne!(f2, disk_fingerprint(&root).unwrap(), "oplog 追加,指纹必变");

        // 无工程 → None
        assert!(disk_fingerprint(&root.join("不存在")).is_none());
        fsutil::cleanup(&root);
    }

    #[test]
    fn cache_key_tracks_fingerprint_changes() {
        let root = fsutil::temp_dir("cutforge-fresh-key");
        fsutil::ensure(&root.join(paths::TIMELINE)).unwrap();
        fsutil::ensure(&root.join(".cutforge/oplog")).unwrap();
        atomic::atomic_write(&root.join(paths::PROJECT_REL), b"{\"a\":1}").unwrap();
        let k1 = disk_fingerprint(&root).unwrap().cache_key();
        assert_eq!(k1, disk_fingerprint(&root).unwrap().cache_key(), "盘面未变,键必须稳定");
        assert_eq!(k1.len(), 16, "键 = 16 位十六进制(FNV-1a 64)");
        atomic::atomic_write(&root.join(paths::PROJECT_REL), b"{\"a\":2}").unwrap();
        assert_ne!(k1, disk_fingerprint(&root).unwrap().cache_key(), "工程一笔之差,键必变");
        fsutil::cleanup(&root);
    }

    #[test]
    fn local_write_registry_roundtrip() {
        let root = fsutil::temp_dir("cutforge-fresh-reg");
        fsutil::ensure(&root.join(paths::TIMELINE)).unwrap();
        atomic::atomic_write(&root.join(paths::PROJECT_REL), b"{\"a\":1}").unwrap();
        note_local_write(&root);
        let own = disk_fingerprint(&root).unwrap();
        assert!(is_last_local_write(&root, &own), "登记后的当前盘面应判为本地写入");

        // 外部改动:磁盘指纹随之改变 → 与登记不一致,不得判为本地写入
        atomic::atomic_write(&root.join(paths::PROJECT_REL), b"{\"a\":9}").unwrap();
        let foreign = disk_fingerprint(&root).unwrap();
        assert_ne!(own, foreign);
        assert!(!is_last_local_write(&root, &foreign), "外部改动不得判为本地写入");

        // 本进程再次写入并登记 → 恢复判为本地写入(守护免开的前提)
        note_local_write(&root);
        let own2 = disk_fingerprint(&root).unwrap();
        assert!(is_last_local_write(&root, &own2));
        fsutil::cleanup(&root);
    }
}
