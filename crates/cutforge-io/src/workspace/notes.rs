//! Workspace 标注(4.9):创建/结案/否决 + 变更后锚点重定位联动。

use std::io;

use cutforge_core::engine::ApplyOpts;
use cutforge_core::notes::{NoteAuthor, NotesStore};
use cutforge_core::oplog::{Actor, OpKind};

use super::reject_to_io;
use super::Workspace;

impl Workspace {
    pub fn notes(&self) -> &NotesStore {
        &self.notes
    }

    /// 创建标注(user 提需求 / agent 反向提问),写入 notes.json 并登记 Op。
    pub fn notes_add(
        &mut self,
        anchor: cutforge_core::anchor::Anchor,
        body: String,
        author: NoteAuthor,
        tags: Vec<String>,
        actor: Actor,
        request_id: Option<String>,
    ) -> io::Result<String> {
        // 幂等:同 request_id 已登记过 → 不再新增;从该 Op 的 after 恢复既有标注 id
        if let Some(rid) = request_id.as_deref()
            && self.engine.oplog().has_request_id(rid)
        {
            let id = self
                .engine
                .oplog()
                .ops()
                .iter()
                .rev()
                .find(|o| o.request_id.as_deref() == Some(rid))
                .and_then(|o| o.after["items"].as_array())
                .and_then(|items| items.last())
                .and_then(|n| n["id"].as_str())
                .unwrap_or_default()
                .to_string();
            return Ok(id);
        }
        self.pre_write_sync()?;
        let before = self.notes.to_value();
        let note_id = self.notes.next_id();
        self.notes.add(anchor, body, author, tags);
        self.record_notes_change(
            &before, OpKind::Insert, actor,
            format!("创建标注 {note_id}"), None, request_id, false,
        )?;
        Ok(note_id)
    }

    /// 结案回执(绑定 opIds;同内容重复结案视为幂等成功,其余错误如实上报)。
    pub fn notes_resolve(
        &mut self,
        note_id: &str,
        reply: String,
        op_ids: Vec<String>,
        actor: Actor,
    ) -> io::Result<()> {
        let before = self.notes.to_value();
        match self.notes.resolve(note_id, reply, op_ids) {
            Ok(_) => {}
            Err(cutforge_core::notes::NoteReject::AlreadyResolved(_)) => return Ok(()), // 幂等回执
            Err(cutforge_core::notes::NoteReject::UnknownNote(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("PRECONDITION_FAILED: 标注 {note_id} 不存在"),
                ))
            }
            Err(e) => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("PRECONDITION_FAILED: {e:?}")))
            }
        }
        self.record_notes_change(&before, OpKind::Set, actor, format!("标注 {note_id} 结案"), None, None, false)?;
        Ok(())
    }

    pub fn notes_reject(&mut self, note_id: &str, reason: String, actor: Actor) -> io::Result<()> {
        let before = self.notes.to_value();
        match self.notes.reject(note_id, reason) {
            Ok(_) => {}
            Err(cutforge_core::notes::NoteReject::UnknownNote(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("PRECONDITION_FAILED: 标注 {note_id} 不存在"),
                ))
            }
            Err(e) => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("PRECONDITION_FAILED: {e:?}")))
            }
        }
        self.record_notes_change(&before, OpKind::Set, actor, format!("标注 {note_id} 否决"), None, None, false)?;
        Ok(())
    }

    /// apply/undo/redo 成功后:open 态标注按 3.6 规则重定位;有变化则登记 auto Op。
    /// 重定位是自动簿记:`auto` Op,不入撤销栈(ADR-0001)。
    /// 落盘交给调用方随后的 `persist()` 统一收口(T1.8 性能专项:消除锚点重定位
    /// 场景的二次 persist——重定位 Op 与主 Op 同批记账,磁盘终态逐字节一致)。
    pub(super) fn sync_notes_after_change(&mut self, actor: Actor) -> io::Result<()> {
        let before = self.notes.to_value();
        let (moved, orphaned) = self.notes.relocate_all(self.engine.project(), 500);
        if moved > 0 || orphaned > 0 {
            self.record_notes_op(
                &before,
                OpKind::Set,
                actor,
                format!("锚点重定位:跟随/重挂 {moved},转孤儿 {orphaned}"),
                None, None, true,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn record_notes_op(
        &mut self,
        before: &serde_json::Value,
        kind: OpKind,
        actor: Actor,
        summary: String,
        caused_by: Option<Vec<String>>,
        request_id: Option<String>,
        non_undoable: bool,
    ) -> io::Result<()> {
        let after = self.notes.to_value();
        let opts = ApplyOpts {
            caused_by: caused_by.unwrap_or_default(),
            summary: Some(summary),
            request_id,
            non_undoable,
            ..Default::default()
        };
        self.engine
            .record_file_change("notes.json", "/items", before.clone(), after, kind, actor, opts)
            .map_err(reject_to_io)?;
        self.notes_dirty = true;
        Ok(())
    }

    /// 登记标注变更并立即落盘(notes_add/resolve/reject 的唯一落盘点)。
    #[allow(clippy::too_many_arguments)]
    fn record_notes_change(
        &mut self,
        before: &serde_json::Value,
        kind: OpKind,
        actor: Actor,
        summary: String,
        caused_by: Option<Vec<String>>,
        request_id: Option<String>,
        non_undoable: bool,
    ) -> io::Result<()> {
        self.record_notes_op(before, kind, actor, summary, caused_by, request_id, non_undoable)?;
        self.persist()
    }
}
