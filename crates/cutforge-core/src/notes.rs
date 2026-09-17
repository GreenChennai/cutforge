//! 标注系统(计划书 3.6/4.9):挂在锚点上的人机对话消息。
//! 核心纪律:锚点重定位失败转 orphan **显式保留**,禁止静默丢弃;
//! 结案回执必须带 opIds(把"标注结案"与"哪几个 Op 导致结案"绑定,可审计)。

use crate::anchor::{relocate, Anchor, AnchorKind, AnchorState};
use crate::model::Project;
use crate::timeutil;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteAuthor {
    User,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteState {
    Open,
    Resolved,
    Rejected,
    Orphan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedBy {
    pub reply: String,
    pub op_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub anchor: Anchor,
    pub body: String,
    pub author: NoteAuthor,
    pub state: NoteState,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relocated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orphan_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<ResolvedBy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NoteReject {
    UnknownNote(String),
    AlreadyResolved(String),
    EmptyReply(String),
}

#[derive(Debug, Clone, Default)]
pub struct NotesStore {
    notes: Vec<Note>,
}

impl NotesStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 解析并校验(契约优先:notes.json 必须过 notes.schema.json)。
    pub fn from_value(v: &Value) -> Result<Self, Vec<String>> {
        let errors = cutforge_schema::validate("notes", v);
        if !errors.is_empty() {
            return Err(errors);
        }
        let parsed: Vec<Note> = serde_json::from_value(v.get("items").cloned().unwrap_or(Value::Array(vec![])))
            .map_err(|e| vec![format!("notes 反序列化失败: {e}")])?;
        Ok(Self { notes: parsed })
    }

    pub fn to_value(&self) -> Value {
        serde_json::json!({ "version": 1, "items": self.notes })
    }

    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// 单调分配下一个标注 id(n-0001 起,四位零填)。
    pub fn next_id(&self) -> String {
        let mut n = 1;
        loop {
            let id = format!("n-{n:04}");
            if !self.notes.iter().any(|x| x.id == id) {
                return id;
            }
            n += 1;
        }
    }

    /// 新建标注(author=user 用户提需求 / author=agent AI 反向提问)。
    pub fn add(&mut self, anchor: Anchor, body: String, author: NoteAuthor, tags: Vec<String>) -> &Note {
        let note = Note {
            id: self.next_id(),
            anchor,
            body,
            author,
            state: NoteState::Open,
            created_at: timeutil::now_rfc3339(),
            relocated: None,
            orphan_reason: None,
            resolved_by: None,
            rejected_reason: None,
            tags: if tags.is_empty() { None } else { Some(tags) },
        };
        self.notes.push(note);
        self.notes.last().expect("刚推入必有元素")
    }

    /// 结案回执:state=resolved 并绑定 opIds(幂等:同回复重复结案无副作用)。
    pub fn resolve(&mut self, id: &str, reply: String, op_ids: Vec<String>) -> Result<&Note, NoteReject> {
        if reply.trim().is_empty() {
            return Err(NoteReject::EmptyReply(id.to_string()));
        }
        if op_ids.is_empty() {
            return Err(NoteReject::EmptyReply(format!("{id}:结案必须至少绑定一个 Op")));
        }
        let note = self.notes.iter_mut().find(|n| n.id == id).ok_or_else(|| NoteReject::UnknownNote(id.to_string()))?;
        if note.state == NoteState::Resolved {
            let same = note.resolved_by.as_ref().is_some_and(|r| r.reply == reply && r.op_ids == op_ids);
            if same {
                return Ok(note);
            }
            return Err(NoteReject::AlreadyResolved(id.to_string()));
        }
        note.state = NoteState::Resolved;
        note.orphan_reason = None;
        note.resolved_by = Some(ResolvedBy { reply, op_ids });
        Ok(note)
    }

    /// 否决标注。
    pub fn reject(&mut self, id: &str, reason: String) -> Result<&Note, NoteReject> {
        let note = self.notes.iter_mut().find(|n| n.id == id).ok_or_else(|| NoteReject::UnknownNote(id.to_string()))?;
        note.state = NoteState::Rejected;
        note.rejected_reason = Some(reason);
        Ok(note)
    }

    /// 重定位(3.6 规则表):元素位移→跟随;消失→≤nearest_ms 重挂;否则 orphan。
    /// 只处理 open 态;返回 (重定位数, 转 orphan 数)。**任何标注都不会被删除。**
    pub fn relocate_all(&mut self, project: &Project, nearest_ms: u64) -> (usize, usize) {
        let mut moved = 0usize;
        let mut orphaned = 0usize;
        for note in self.notes.iter_mut() {
            if note.state != NoteState::Open {
                continue;
            }
            if matches!(note.anchor.kind, AnchorKind::Time | AnchorKind::Word) {
                continue; // 纯时间点/word 域锚点不随元素迁移
            }
            let r = relocate(project, &note.anchor, nearest_ms);
            match r.state {
                AnchorState::Resolved => {}
                AnchorState::Relocated => {
                    if note.anchor != r.anchor {
                        note.anchor = r.anchor;
                        note.relocated = Some(true);
                        moved += 1;
                    }
                }
                AnchorState::Orphan => {
                    if note.state != NoteState::Orphan {
                        note.state = NoteState::Orphan;
                        note.orphan_reason = r.orphan_reason;
                        orphaned += 1;
                    }
                }
            }
        }
        (moved, orphaned)
    }

    /// 孤儿标注视图(编辑器"孤儿标注"面板;orphan 显式可见)。
    pub fn orphans(&self) -> Vec<&Note> {
        self.notes.iter().filter(|n| n.state == NoteState::Orphan).collect()
    }

    /// 按状态过滤(notes_list 查询)。
    pub fn filter(&self, state: Option<NoteState>, author: Option<NoteAuthor>) -> Vec<&Note> {
        self.notes
            .iter()
            .filter(|n| state.is_none_or(|s| n.state == s))
            .filter(|n| author.is_none_or(|a| n.author == a))
            .collect()
    }

    pub fn find(&self, id: &str) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    /// 可变访问(测试与 IO 层构造场景用)。
    pub fn note_mut(&mut self, id: &str) -> Option<&mut Note> {
        self.notes.iter_mut().find(|n| n.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::sample_project;

    fn clip_anchor(id: &str, t: u64) -> Anchor {
        Anchor { kind: AnchorKind::Clip, ref_: Some(id.into()), t_ms: t, span: None }
    }

    #[test]
    fn add_resolve_reject_lifecycle() {
        let mut store = NotesStore::new();
        let n1 = store.add(clip_anchor("V1-001", 4000), "这里语速太快".into(), NoteAuthor::User, vec!["节奏".into()]);
        assert_eq!(n1.id, "n-0001");
        assert_eq!(n1.state, NoteState::Open);
        let n2 = store.add(clip_anchor("V1-002", 9000), "要不要删 300ms?".into(), NoteAuthor::Agent, vec![]);
        assert_eq!(n2.id, "n-0002");

        store.resolve("n-0001", "已删 320ms 并重烧字幕".into(), vec!["op-1".into()]).unwrap();
        let r = store.find("n-0001").unwrap();
        assert_eq!(r.state, NoteState::Resolved);
        assert_eq!(r.resolved_by.as_ref().unwrap().op_ids, vec!["op-1".to_string()]);
        // 幂等:同回复重复结案 → Ok 无变化
        assert!(store.resolve("n-0001", "已删 320ms 并重烧字幕".into(), vec!["op-1".into()]).is_ok());
        // 不同回复覆盖已结案 → 拒绝
        assert!(matches!(store.resolve("n-0001", "改口".into(), vec!["op-2".into()]), Err(NoteReject::AlreadyResolved(_))));
        // 空 reply / 空 opIds 拒绝
        assert!(store.resolve("n-0002", "  ".into(), vec!["op-1".into()]).is_err());
        assert!(store.resolve("n-0002", "好".into(), vec![]).is_err());
        store.reject("n-0002", "先不动".into()).unwrap();
        assert_eq!(store.find("n-0002").unwrap().state, NoteState::Rejected);
        assert!(matches!(store.resolve("n-9999", "x".into(), vec!["op-1".into()]), Err(NoteReject::UnknownNote(_))));
    }

    #[test]
    fn roundtrip_value_and_schema() {
        let mut store = NotesStore::new();
        store.add(clip_anchor("V1-001", 4000), "正文".into(), NoteAuthor::User, vec![]);
        let v = store.to_value();
        let back = NotesStore::from_value(&v).expect("自产 notes 必须过 schema");
        assert_eq!(back.notes().len(), 1);
    }

    #[test]
    fn relocate_all_classifies_and_never_drops() {
        let mut store = NotesStore::new();
        store.add(clip_anchor("V1-002", 9000), "会跟随".into(), NoteAuthor::User, vec![]);
        store.add(clip_anchor("V1-002", 8600), "会重挂".into(), NoteAuthor::User, vec![]);
        store.add(clip_anchor("V1-002", 20000), "会孤儿".into(), NoteAuthor::User, vec![]);
        store.add(clip_anchor("V1-002", 3000), "已结案不迁移".into(), NoteAuthor::User, vec![]);
        store.resolve("n-0004", "done".into(), vec!["op-9".into()]).unwrap();

        let mut p = sample_project();
        p.tracks[0].clips.remove(1); // V1-002 消失
        let (moved, orphaned) = store.relocate_all(&p, 500);
        assert_eq!(moved, 2, "跟随 + 重挂");
        assert_eq!(orphaned, 1, "越界转 orphan");
        assert_eq!(store.notes().len(), 4, "无静默丢失");
        assert_eq!(store.orphans().len(), 1);
        assert!(store.orphans()[0].orphan_reason.as_deref().unwrap().contains("V1-002"));
        let resolved = store.find("n-0004").unwrap();
        assert_eq!(resolved.state, NoteState::Resolved, "已结案标注不参与重定位");
        assert!(resolved.relocated.is_none());
    }
}
