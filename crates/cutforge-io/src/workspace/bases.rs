//! baseRev 快照链(M9-1):`.cutforge/bases/<rev>.json` = rev N 时刻的工程视图。
//! 三路合并的共同祖先由此取得;快照 LRU 淘汰防膨胀(计划书 V2-R4)。

use std::io;
use std::path::{Path, PathBuf};

use super::Workspace;

/// baseRev 快照目录(M9-1):`.cutforge/bases/<rev>.json` = rev N 时刻的工程视图。
/// 三路合并的共同祖先由此取得;"本地充当 base"的死代码口径退役。
pub const BASES_REL: &str = ".cutforge/bases";
/// 快照保留上限(LRU 按 rev 淘汰,防膨胀;计划书 V2-R4)。
const BASES_KEEP: usize = 32;

impl Workspace {
    /// 从快照链取三路合并的祖先:精确 rev → ≤当前 rev 的最大者;链缺失返回 None
    /// (退化口径:以本地为 base——与 V1 兼容,但此时冲突检测天然不可触发)。
    pub(super) fn load_base(&self) -> Option<serde_json::Value> {
        let dir = self.root.join(BASES_REL);
        let mut best: Option<(u64, PathBuf)> = None;
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(n) = name
                .strip_suffix(".json")
                .and_then(|s| s.parse::<u64>().ok())
            else {
                continue;
            };
            if n <= self.engine.rev() && best.as_ref().map(|(b, _)| n > *b).unwrap_or(true) {
                best = Some((n, entry.path()));
            }
        }
        let (_, path) = best?;
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// 同步点 baseRev 快照(M9-1)+ LRU 淘汰(原 persist 中段,拆出为独立落盘步骤):
    /// 快照 = 当前 rev 的工程视图(不含 `_meta`,与契约面一致)。
    pub(super) fn snapshot_bases(&self) -> io::Result<()> {
        let rev = self.engine.rev();
        let bases_dir = self.root.join(BASES_REL);
        let _ = std::fs::create_dir_all(&bases_dir);
        let base_value = self.engine.query(cutforge_core::engine::Query::ProjectView);
        let cutforge_core::engine::Answer::Project(ref bv) = base_value else {
            unreachable!()
        };
        let mut bb = serde_json::to_vec_pretty(bv)?;
        bb.push(b'\n');
        let _ = crate::atomic::atomic_write(&bases_dir.join(format!("{rev}.json")), &bb);
        prune_bases(&bases_dir, BASES_KEEP);
        Ok(())
    }
}

/// baseRev 快照 LRU 淘汰:保留 rev 最大的 keep 份(计划书 V2-R4 防膨胀)。
fn prune_bases(dir: &Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut revs: Vec<(u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(".json")
                .and_then(|x| x.parse::<u64>().ok())
                .map(|n| (n, e.path()))
        })
        .collect();
    revs.sort_by_key(|(n, _)| *n);
    while revs.len() > keep {
        let (_, path) = revs.remove(0);
        // 唯一落盘点纪律(M2-4):删除同样收敛 atomic.rs,不走旁路 API
        let _ = crate::atomic::remove(&path);
    }
}
