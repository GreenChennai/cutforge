// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 查询投影(计划书 2.6):`Engine::query` 纯投影、无副作用、可并发;
//! 另含状态语义 hash 与规范化 JSON(测试与 M3 回放等价门禁的基础)。

use super::Engine;
use crate::oplog::Op;
use serde_json::Value;

#[derive(Debug, Clone)]
pub enum Query {
    /// 全工程视图(序列化 Value)。
    ProjectView,
    /// 单片段。
    Clip { id: String },
    /// 单轨道。
    Track { id: String },
    /// 时间线概览:(clip_id, start_ms, end_ms, track_id)。
    Timeline,
    /// OpLog tail。
    OpLogTail {
        since_rev: Option<u64>,
        actor_kind: Option<crate::oplog::ActorKind>,
    },
    /// 当前 rev。
    Rev,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Project(Value),
    Clip(Option<Value>),
    Track(Option<Value>),
    Timeline(Vec<(String, u64, u64, String)>),
    Ops(Vec<Op>),
    Rev(u64),
}

impl Engine {
    /// 查询接口:纯投影。
    pub fn query(&self, q: Query) -> Answer {
        match q {
            Query::ProjectView => {
                Answer::Project(serde_json::to_value(&self.project).unwrap_or(Value::Null))
            }
            Query::Clip { id } => Answer::Clip(self.project.find_clip(&id).map(|(ti, ci)| {
                serde_json::to_value(&self.project.tracks[ti].clips[ci]).unwrap_or(Value::Null)
            })),
            Query::Track { id } => {
                Answer::Track(self.project.find_track(&id).map(|ti| {
                    serde_json::to_value(&self.project.tracks[ti]).unwrap_or(Value::Null)
                }))
            }
            Query::Timeline => Answer::Timeline(
                self.project
                    .tracks
                    .iter()
                    .flat_map(|t| {
                        t.clips.iter().map(move |c| {
                            (
                                c.id.clone(),
                                c.start_ms,
                                c.start_ms + c.duration_ms,
                                t.id.clone(),
                            )
                        })
                    })
                    .collect(),
            ),
            Query::OpLogTail {
                since_rev,
                actor_kind,
            } => Answer::Ops(
                self.log
                    .tail(since_rev, actor_kind)
                    .into_iter()
                    .cloned()
                    .collect(),
            ),
            Query::Rev => Answer::Rev(self.rev),
        }
    }

    /// 状态语义 hash(测试与 M3 回放等价门禁的基础)。
    pub fn state_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let v = serde_json::to_value(&self.project).unwrap_or(Value::Null);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        canonical_json(&v).hash(&mut h);
        h.finish()
    }
}

/// 规范化 JSON 文本(键序无关,数值保持 Value 语义)。
pub fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .into_iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap(),
                        canonical_json(&m[k])
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(a) => {
            let inner: Vec<String> = a.iter().map(canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::engine::{ApplyOpts, sample_project};
    use crate::oplog::{Actor, ActorKind};

    #[test]
    fn oplog_tail_filters() {
        let mut eng = Engine::new(sample_project()).unwrap();
        eng.apply(
            Command::ClipDelete {
                clip_id: "A1-001".into(),
            },
            Actor::user("用户"),
            ApplyOpts::default(),
        )
        .unwrap();
        eng.apply(
            Command::ClipDelete {
                clip_id: "V1-002".into(),
            },
            Actor::agent("AI"),
            ApplyOpts::default(),
        )
        .unwrap();
        match eng.query(Query::OpLogTail {
            since_rev: Some(0),
            actor_kind: Some(ActorKind::Agent),
        }) {
            Answer::Ops(ops) => {
                assert_eq!(ops.len(), 1);
                assert_eq!(ops[0].actor.id, "AI");
            }
            other => panic!("意外: {other:?}"),
        }
        match eng.query(Query::OpLogTail {
            since_rev: None,
            actor_kind: None,
        }) {
            Answer::Ops(ops) => assert_eq!(ops.len(), 2),
            other => panic!("意外: {other:?}"),
        }
    }
}
