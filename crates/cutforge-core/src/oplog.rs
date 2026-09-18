// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 操作日志(计划书 3.7/4.4):Op 是一次原子变更的不可变记录,
//! 双向合并与审计的唯一依据。Op 只追加、不修改;撤销/重做也产生新 Op。
//! 排序按 rev 单调递增,不依赖文件系统时间戳。

use crate::timeutil;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActorKind {
    User,
    Agent,
    Script,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Actor {
    pub kind: ActorKind,
    pub id: String,
}

impl Actor {
    pub fn agent(id: &str) -> Self {
        Self { kind: ActorKind::Agent, id: id.into() }
    }
    pub fn user(id: &str) -> Self {
        Self { kind: ActorKind::User, id: id.into() }
    }
    pub fn script(id: &str) -> Self {
        Self { kind: ActorKind::Script, id: id.into() }
    }
}

/// opKind 封闭枚举(计划书 3.7):set/insert/delete/move/split/merge/resolve-conflict/undo/redo。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpKind {
    Set,
    Insert,
    Delete,
    Move,
    Split,
    Merge,
    ResolveConflict,
    Undo,
    Redo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpTarget {
    /// 真相源文件(oplog schema 封闭枚举:五个文件)。
    pub file: String,
    /// JSON Pointer 叶路径,如 /tracks/0/clips/3/durationMs。
    pub path: String,
}

/// 一条 Op。`before`/`after` 记录 target.path 处的完整旧值/新值(子树级),
/// 因此任何 opKind 的逆操作都是 before↔after 互换——split/merge 的无损可逆由此保证。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Op {
    pub op_id: String,
    pub ts: String,
    pub actor: Actor,
    pub target: OpTarget,
    pub op_kind: OpKind,
    pub before: Value,
    pub after: Value,
    /// 硬约束:必填。本 Op 基于的共同祖先版本(三路合并的关键)。
    pub base_rev: String,
    /// 本 Op 应用后的工程修订号(单调递增)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rev: Option<u64>,
    /// 自动登记类变更(锚点重定位等系统簿记):进审计链但**不入撤销栈**,
    /// 撤销深度因此等于真实用户手势数(ADR-0001)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caused_by: Option<Vec<String>>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// OpLog:内存中的追加式序列;持久化(.jsonl)由 cutforge-io 负责。
#[derive(Debug, Clone, Default)]
pub struct OpLog {
    ops: Vec<Op>,
    /// 已见 opId 去重集(跨天分文件后仍需去重,故内存持全集)。
    seen: BTreeSet<String>,
    next_op: u64,
}

impl OpLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// 分配下一个 opId。
    pub fn next_op_id(&mut self) -> String {
        loop {
            let id = crate::format_op_id(self.next_op);
            self.next_op += 1;
            if !self.seen.contains(&id) {
                return id;
            }
        }
    }

    /// 追加(带 opId 去重:同一 opId 重复应用必须无副作用,计划书 4.4 幂等性)。
    /// 返回 None 表示重复(opId 已存在),调用方不得重复计 rev。
    pub fn push(&mut self, mut op: Op) -> Option<&Op> {
        if self.seen.contains(&op.op_id) {
            return None;
        }
        self.seen.insert(op.op_id.clone());
        op.ts = timeutil::now_rfc3339();
        self.ops.push(op);
        self.ops.last()
    }

    /// 从磁盘加载:保留原 ts,只做去重;并抬高 opId 分配下限防撞号。
    pub fn push_loaded(&mut self, op: Op) -> bool {
        if self.seen.contains(&op.op_id) {
            return false;
        }
        if let Some(n) = op.op_id.strip_prefix("op-").and_then(|s| s.parse::<u64>().ok())
            && n >= self.next_op {
                self.next_op = n + 1;
            }
        self.seen.insert(op.op_id.clone());
        self.ops.push(op);
        true
    }

    /// 是否已有该 request_id 的 Op(非幂等写操作的去重键)。
    pub fn has_request_id(&self, rid: &str) -> bool {
        self.ops.iter().any(|o| o.request_id.as_deref() == Some(rid))
    }

    /// tail 查询:rev > since 的 Op,可按 actor.kind 过滤(AI 感知用户改动/反之)。
    pub fn tail(&self, since_rev: Option<u64>, actor_kind: Option<ActorKind>) -> Vec<&Op> {
        self.ops
            .iter()
            .filter(|o| o.rev.is_some_and(|r| since_rev.is_none_or(|s| r > s)))
            .filter(|o| actor_kind.is_none_or(|k| o.actor.kind == k))
            .collect()
    }

    pub fn last_rev(&self) -> Option<u64> {
        self.ops.last().and_then(|o| o.rev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format_op_id;
    use serde_json::json;

    fn mk(id: &str, rev: u64, kind: ActorKind) -> Op {
        Op {
            op_id: id.into(),
            ts: String::new(),
            actor: Actor { kind, id: "t".into() },
            target: OpTarget { file: "project.json".into(), path: "/slug".into() },
            op_kind: OpKind::Set,
            before: json!("旧"),
            after: json!("新"),
            base_rev: format!("rev-{rev}"),
            rev: Some(rev + 1),
            caused_by: None,
            summary: "测试".into(),
            request_id: None,
            auto: None,
        }
    }

    #[test]
    fn push_dedups_op_id() {
        let mut log = OpLog::new();
        let op = mk("op-1", 0, ActorKind::User);
        assert!(log.push(op.clone()).is_some());
        assert!(log.push(op).is_none(), "同 opId 重复应用必须无副作用");
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn request_id_and_tail_filters() {
        let mut log = OpLog::new();
        let mut op = mk(&format_op_id(log.next_op_id().parse::<u64>().unwrap_or(0)), 0, ActorKind::Agent);
        op.op_id = "op-0".into();
        op.request_id = Some("req-1".into());
        log.push(op).unwrap();
        assert!(log.has_request_id("req-1"));
        assert!(!log.has_request_id("req-2"));
        let mut op2 = mk("op-1", 1, ActorKind::User);
        op2.rev = Some(2);
        log.push(op2).unwrap();
        assert_eq!(log.tail(Some(1), None).len(), 1, "rev>1 只有第二条");
        assert_eq!(log.tail(None, None).len(), 2);
        assert_eq!(log.tail(Some(1), Some(ActorKind::User)).len(), 1);
        assert_eq!(log.tail(Some(1), Some(ActorKind::Agent)).len(), 0);
        assert_eq!(log.last_rev(), Some(2));
    }

}
