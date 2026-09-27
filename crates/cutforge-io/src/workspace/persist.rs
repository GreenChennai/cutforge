//! Workspace 持久化管线:引擎脏真相源落盘、写入窗口漂移检测、
//! 备份 → 原子写 → baseRev 快照 → OpLog 追加 → rev 落盘 → 标注落盘。
//! 每个落盘动作一个具名函数(4.2 八步的磁盘侧步骤),`persist` 只做编排。

use std::io;

use cutforge_core::merge::MergeOutcome;

use super::Workspace;
use crate::backup;
use crate::paths::NOTES_REL;

impl Workspace {
    /// 引擎脏文件 → 盘面(先文件后记账;P1-9:写失败时 oplog 尚未记账)。
    /// notes.json 特殊:重载 NotesStore(撤销/重做恢复标注态)。
    pub(super) fn reconcile_files(&mut self) -> io::Result<()> {
        let truths = self.layout.truths;
        for file in self.engine.take_dirty_files() {
            if file == "notes.json" {
                let Some(v) = self.engine.file_state("notes.json").cloned() else { continue };
                if v != self.notes.to_value() {
                    let store = cutforge_core::notes::NotesStore::from_value(&v).map_err(|errs| {
                        io::Error::other(format!("CF-005 SCHEMA_DRIFT(notes.json): {}", errs.join("; ")))
                    })?;
                    self.notes = store;
                    self.notes_dirty = true;
                }
                continue;
            }
            let Some(rel) = truths.iter().find(|(n, _)| *n == file).map(|(_, r)| r) else { continue };
            let Some(v) = self.engine.file_state(&file).cloned() else { continue };
            if self.files.get(&file) != Some(&v) {
                let mut buf = serde_json::to_vec_pretty(&v)?;
                buf.push(b'\n');
                crate::atomic::atomic_write(&self.root.join(rel), &buf)?;
                self.files.insert(file, v);
            }
        }
        Ok(())
    }

    /// 步骤 5-7:备份 → 原子写 project.json(`_meta` 旁路回写)→ 追加 oplog →
    /// rev 落盘 → 标注落盘。真相源文件本体已在 reconcile_files 先行落盘。
    ///
    /// 编排:每步一个具名函数,依次调用;执行序与拆分前逐字一致
    /// (备份/原子写在前、OpLog 追加在后——先文件后记账,P1-9)。
    pub(super) fn persist(&mut self) -> io::Result<()> {
        self.check_window_drift()?;   // 前置:写入窗口漂移检测(M9-1)
        self.backup_project()?;       // 4.2 八步之 5:备份旧 project.json
        self.atomic_write_project()?; // 4.2 八步之 6:project.json 原子替换
        self.snapshot_bases()?;       // M9-1:同步点 baseRev 快照 + LRU(bases.rs)
        self.append_oplog()?;         // 4.2 八步之 4:Op 追加 jsonl(append-only)
        self.write_rev()?;            // 4.2 八步之 6/7:rev 落盘
        self.flush_notes()?;          // 标注落盘(有变化才写)
        Ok(())
    }

    /// persist 前置:磁盘 != 上次同步视图 → 外部在窗口内写入。
    /// 可自动合并 → 采纳(继续写);冲突 → 冲突落盘、本地重载、报 CONFLICT。
    fn check_window_drift(&mut self) -> io::Result<()> {
        let Some(synced) = self.synced_disk.clone() else { return Ok(()) };
        let Ok(text) = std::fs::read_to_string(self.root.join(self.layout.project_rel)) else { return Ok(()) };
        let Ok(mut cur) = serde_json::from_str::<serde_json::Value>(&text) else { return Ok(()) };
        let cur_meta = cur.as_object_mut().and_then(|o| o.remove("_meta"));
        if let Some(m) = cur_meta {
            self.meta_bypass = Some(m);
        }
        if cur == synced {
            return Ok(());
        }
        let local = match self.engine.query(cutforge_core::engine::Query::ProjectView) {
            cutforge_core::engine::Answer::Project(v) => v,
            _ => unreachable!(),
        };
        match cutforge_core::merge::three_way_merge(&synced, &cur, &local) {
            MergeOutcome::Merged(v) => {
                self.synced_disk = Some(cur);
                if v != local {
                    self.adopt_merged(v)?;
                }
                Ok(())
            }
            MergeOutcome::Conflicts(conflicts) => {
                for c in &conflicts {
                    self.persist_conflict(c);
                }
                self.reload_from_disk()?;
                Err(io::Error::other(format!(
                    "CONFLICT: 写入窗口内外部已改动同一工程({} 项冲突),本地待写已弃用,请裁决后重试",
                    conflicts.len())))
            }
        }
    }

    /// 4.2 八步之 5:备份旧 project.json(全局约定 B.8:可回滚)。
    /// 仅在确有 Op 待记账(persisted==0 或落后于 oplog)时备份,与拆分前条件一致。
    fn backup_project(&self) -> io::Result<()> {
        let ops = self.engine.oplog().ops();
        if (self.persisted == 0 || self.persisted < ops.len())
            && let Ok(old) = std::fs::read(self.root.join(self.layout.project_rel)) {
                backup::backup_file(&self.root, self.layout.project_rel, &old)?;
            }
        Ok(())
    }

    /// 4.2 八步之 6:project.json 原子替换(`_meta` 旁路回写;唯一落盘点
    /// `atomic::atomic_write`)。写后更新同步点视图(窗口漂移检测基准)。
    fn atomic_write_project(&mut self) -> io::Result<()> {
        let value = self.engine.query(cutforge_core::engine::Query::ProjectView);
        let cutforge_core::engine::Answer::Project(mut v) = value else { unreachable!() };
        if let Some(meta) = &self.meta_bypass
            && let Some(obj) = v.as_object_mut() {
                obj.insert("_meta".into(), meta.clone());
            }
        let mut buf = serde_json::to_vec_pretty(&v)?;
        buf.push(b'\n');
        crate::atomic::atomic_write(&self.root.join(self.layout.project_rel), &buf)?;
        self.synced_disk = Some(v);
        Ok(())
    }

    /// 4.2 八步之 4:新 Op 追加 `.cutforge/oplog/<日>.jsonl`(按天切分;append-only)。
    /// 从 `persisted` 游标续写(与拆分前一致;实序在原子写之后,先文件后记账)。
    fn append_oplog(&mut self) -> io::Result<()> {
        let ops = self.engine.oplog().ops();
        let day = today_compact();
        let oplog_file = self.root.join(".cutforge/oplog").join(format!("{day}.jsonl"));
        while self.persisted < ops.len() {
            let op = &ops[self.persisted];
            let line = serde_json::to_string(op)?;
            crate::atomic::append_line(&oplog_file, &format!("{line}\n"))?;
            self.persisted += 1;
        }
        Ok(())
    }

    /// 4.2 八步之 6/7:rev 落盘(`.cutforge/rev`;open 时与 oplog 对账修复)。
    fn write_rev(&self) -> io::Result<()> {
        crate::atomic::atomic_write(
            &self.root.join(".cutforge/rev"),
            format!("{}\n", self.engine.rev()).as_bytes(),
        )
    }

    /// 标注落盘(有变化才写;notes.json 不在引擎脏文件路由内,由脏标记驱动)。
    fn flush_notes(&mut self) -> io::Result<()> {
        if self.notes_dirty {
            let mut buf = serde_json::to_vec_pretty(&self.notes.to_value())?;
            buf.push(b'\n');
            crate::atomic::atomic_write(&self.root.join(NOTES_REL), &buf)?;
            self.notes_dirty = false;
        }
        Ok(())
    }

    /// 弃用本地待写:从磁盘真相重载全部状态(外部改动获胜,4.7)。
    fn reload_from_disk(&mut self) -> io::Result<()> {
        let (engine, persisted, notes, meta_bypass, files, synced_disk, layout) = Self::load(&self.root)?;
        self.engine = engine;
        self.persisted = persisted;
        self.notes = notes;
        self.notes_dirty = false;
        self.meta_bypass = meta_bypass;
        self.files = files;
        self.synced_disk = synced_disk;
        self.layout = layout;
        Ok(())
    }
}

/// UTC 紧凑日期 YYYYMMDD(oplog 按天切分;算法唯一来源 core::timeutil)。
fn today_compact() -> String {
    cutforge_core::timeutil::now_date_compact()
}
