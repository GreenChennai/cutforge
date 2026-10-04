//! Workspace 冲突(4.7):三方快照落盘、清单读取、兼容合并入口。

use std::io;
use std::path::PathBuf;

use cutforge_core::merge::{Conflict, ConflictCode};

use super::Workspace;

impl Workspace {
    /// 当前持久化的冲突清单。
    pub fn conflict_list(&self) -> io::Result<Vec<(String, Conflict)>> {
        let dir = self.root.join(".cutforge/conflicts");
        let mut out = Vec::new();
        if !dir.is_dir() {
            return Ok(out);
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        for f in files {
            let text = std::fs::read_to_string(&f)?;
            let v: serde_json::Value = serde_json::from_str(&text)?;
            let id = f
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let conflict = serde_json::from_value::<PersistedConflict>(v)?.into_conflict();
            out.push((id, conflict));
        }
        Ok(out)
    }

    /// 三路合并(兼容入口):磁盘文件与内存模型按 baseRev 快照合并。
    /// 冲突时写入 `.cutforge/conflicts/<id>.json` 三方快照并停写(4.7)。
    /// `_meta` 旁路:磁盘若带新版 `_meta`(CutFlow 重生成),采纳之(ADR-0002)。
    pub fn merge_from_disk(&mut self) -> Result<Option<u64>, Vec<(String, Conflict)>> {
        self.sync_with_disk()?;
        Ok(Some(self.engine.rev()))
    }

    /// 冲突三方快照落盘(4.7:冲突产生时在任何写入发生之前停止)。
    pub(super) fn persist_conflict(&self, c: &Conflict) -> String {
        let id = format!(
            "cf-{}-{}",
            self.engine.rev(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let payload = serde_json::json!({
            "conflictId": id,
            "code": c.code.code(),
            "pointer": c.pointer,
            "base": c.base,
            "disk": c.disk,
            "local": c.local,
            "createdAt": cutforge_core::timeutil::now_rfc3339(),
        });
        let dir = self.root.join(".cutforge/conflicts");
        let _ = std::fs::create_dir_all(&dir);
        let _ = crate::atomic::atomic_write(
            &dir.join(format!("{id}.json")),
            payload.to_string().as_bytes(),
        );
        id
    }
}

/// 持久化冲突快照的磁盘形态(`.cutforge/conflicts/<id>.json`)。
#[derive(serde::Deserialize)]
struct PersistedConflict {
    code: String,
    pointer: String,
    #[serde(default)]
    base: Option<serde_json::Value>,
    #[serde(default)]
    disk: Option<serde_json::Value>,
    #[serde(default)]
    local: Option<serde_json::Value>,
}

impl PersistedConflict {
    fn into_conflict(self) -> Conflict {
        let code = match self.code.as_str() {
            "CF-002" => ConflictCode::DeleteModify,
            "CF-003" => ConflictCode::DupId,
            _ => ConflictCode::FieldConflict,
        };
        Conflict {
            code,
            pointer: self.pointer,
            base: self.base,
            disk: self.disk,
            local: self.local,
        }
    }
}
