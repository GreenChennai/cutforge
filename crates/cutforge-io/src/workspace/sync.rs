//! Workspace 写前同步:冲突停写 + 磁盘三路合并(M9-1 / 4.7)。

use std::io;

use cutforge_core::engine::Engine;
use cutforge_core::merge::{Conflict, ConflictCode, MergeOutcome};
use cutforge_core::model::Project;

use super::Workspace;

impl Workspace {
    /// 写前同步(M9-1):磁盘与本地已分叉时,先按真祖先三路合并;
    /// 不可自动合并 → 冲突落盘并停写(4.7:任何写入发生之前停止)。
    pub(super) fn pre_write_sync(&mut self) -> io::Result<()> {
        if !self.conflict_list()?.is_empty() {
            return Err(io::Error::other(
                "CONFLICT: 存在未裁决冲突(.cutforge/conflicts/),停写直至裁决",
            ));
        }
        match self.sync_with_disk() {
            Ok(_) => Ok(()),
            Err(conflicts) => Err(io::Error::other(format!(
                "CONFLICT: 外部改动与本地不可自动合并({} 项),已落 .cutforge/conflicts/",
                conflicts.len()
            ))),
        }
    }

    /// 与磁盘做一次三路合并(base = baseRev 快照链的真祖先;不再"本地充当 base")。
    /// 返回 Ok(true) = 采纳了外部改动;Err = 不可自动合并(冲突已落盘)。
    pub fn sync_with_disk(&mut self) -> Result<bool, Vec<(String, Conflict)>> {
        let disk_text = match std::fs::read_to_string(self.root.join(self.layout.project_rel)) {
            Ok(t) => t,
            Err(_) => return Ok(false),
        };
        let mut disk: serde_json::Value = match serde_json::from_str(&disk_text) {
            Ok(v) => v,
            Err(e) => {
                let c = Conflict {
                    code: ConflictCode::FieldConflict,
                    pointer: "$".into(),
                    base: None,
                    disk: Some(serde_json::Value::String(format!(
                        "CF-005 SCHEMA_DRIFT: {e}"
                    ))),
                    local: None,
                };
                let id = self.persist_conflict(&c);
                return Err(vec![(id, c)]);
            }
        };
        let disk_meta = disk.as_object_mut().and_then(|o| o.remove("_meta"));
        if let Some(m) = disk_meta {
            self.meta_bypass = Some(m);
        }
        if Some(&disk) == self.synced_disk.as_ref() {
            return Ok(false); // 快路径:磁盘与装载时一致,无外部改动
        }
        let local = match self.engine.query(cutforge_core::engine::Query::ProjectView) {
            cutforge_core::engine::Answer::Project(v) => v,
            _ => unreachable!(),
        };
        let base = self
            .load_base()
            .or_else(|| self.synced_disk.clone())
            .unwrap_or_else(|| local.clone());
        match cutforge_core::merge::three_way_merge(&base, &disk, &local) {
            MergeOutcome::Merged(v) => {
                if v == local {
                    return Ok(false);
                }
                self.adopt_merged(v).map_err(|e| {
                    vec![(
                        format!("cf-adopt-{}", self.engine.rev()),
                        Conflict {
                            code: ConflictCode::FieldConflict,
                            pointer: "$".into(),
                            base: None,
                            disk: None,
                            local: Some(serde_json::Value::String(e.to_string())),
                        },
                    )]
                })?;
                Ok(true)
            }
            MergeOutcome::Conflicts(conflicts) => {
                let persisted: Vec<(String, Conflict)> = conflicts
                    .iter()
                    .map(|c| (self.persist_conflict(c), c.clone()))
                    .collect();
                Err(persisted)
            }
        }
    }

    /// 采纳合并结果:保留 OpLog/rev/撤销栈/文件态的历史连续性(不得重置 rev)。
    pub(super) fn adopt_merged(&mut self, v: serde_json::Value) -> io::Result<()> {
        let project = Project::from_value(&v).map_err(|errs| {
            io::Error::other(format!(
                "CF-005 SCHEMA_DRIFT(合并结果): {}",
                errs.join("; ")
            ))
        })?;
        let log = self.engine.oplog().clone();
        let (undo_stack, redo_stack) = cutforge_core::engine::rebuild_stacks(log.ops());
        let file_states = self.engine.file_states().clone();
        let new_engine = Engine::restore_with_stacks(
            project,
            log,
            self.engine.rev(),
            undo_stack,
            redo_stack,
            file_states,
        )
        .map_err(|errs| io::Error::other(errs.join("; ")))?;
        self.engine = new_engine;
        self.persisted = self.engine.oplog().len();
        self.persist()?;
        Ok(())
    }
}
