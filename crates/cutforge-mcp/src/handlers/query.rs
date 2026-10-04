// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 只读查询投影处理器(E6-3:只读打开,不持排他锁)。A-01 自 dispatch.rs
//! 巨 match 只读段逐字迁移(行为零变化);统一走工作区面(只读零锁语义
//! 由派发层 with_resident 承接)。

use crate::cutlist_ops::read_truth;
use crate::handlers::{CommandHandler, HandlerCtx};
use crate::registry::envelope;
use cutforge_core::engine::{Answer, Query};
use cutforge_core::oplog::ActorKind;
use cutforge_io::paths;
use serde_json::{Value, json};

/// 工程视图(E2-2:投影由服务端算好下放)。
pub(crate) struct ProjectGet;

impl CommandHandler for ProjectGet {
    fn name(&self) -> &'static str {
        "project_get"
    }

    fn handle(&self, _args: &Value, cx: &mut HandlerCtx) -> Value {
        let ws = cx.ws();
        match ws.engine().query(Query::ProjectView) {
            Answer::Project(v) => envelope(
                true,
                "OK",
                "工程视图",
                json!({"project": v, "rev": ws.rev()}),
            ),
            _ => unreachable!(),
        }
    }
}

/// 口播逐字真相(只读)。
pub(crate) struct WordlineGet;

impl CommandHandler for WordlineGet {
    fn name(&self) -> &'static str {
        "wordline_get"
    }

    fn handle(&self, _args: &Value, cx: &mut HandlerCtx) -> Value {
        read_truth(
            &cx.ws_root.join(
                paths::truth_rel_on_disk(cx.ws_root, "wordline.json")
                    .unwrap_or(paths::WORDLINE_REL),
            ),
            "wordline",
        )
    }
}

/// 粗剪决策清单(applied=true 读回执副本)。
pub(crate) struct CutlistGet;

impl CommandHandler for CutlistGet {
    fn name(&self) -> &'static str {
        "cutlist_get"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let applied = args["applied"].as_bool().unwrap_or(false);
        let name = if applied {
            "cutlist.applied.json"
        } else {
            "cutlist.json"
        };
        let rel = paths::truth_rel_on_disk(cx.ws_root, name).unwrap_or(paths::CUTLIST_REL);
        read_truth(&cx.ws_root.join(rel), "cutlist")
    }
}

/// 标注清单(状态/作者过滤)。
pub(crate) struct NotesList;

impl CommandHandler for NotesList {
    fn name(&self) -> &'static str {
        "notes_list"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let ws = cx.ws();
        let state = args["state"]
            .as_str()
            .and_then(|s| serde_json::from_str(&format!("\"{s}\"")).ok());
        let author = args["author"].as_str().map(|s| match s {
            "agent" => cutforge_core::notes::NoteAuthor::Agent,
            _ => cutforge_core::notes::NoteAuthor::User,
        });
        let items = ws.notes().filter(state, author);
        envelope(
            true,
            "OK",
            "标注清单",
            json!({
                "notes": items, "total": ws.notes().notes().len(),
                "orphans": ws.notes().orphans().len(),
            }),
        )
    }
}

/// OpLog tail(sinceRev/actor 过滤,倒序截取)。
pub(crate) struct OplogTail;

impl CommandHandler for OplogTail {
    fn name(&self) -> &'static str {
        "oplog_tail"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let ws = cx.ws();
        match ws.engine().query(Query::OpLogTail {
            since_rev: args["sinceRev"].as_u64(),
            actor_kind: args["actor"].as_str().map(|s| match s {
                "user" => ActorKind::User,
                "script" => ActorKind::Script,
                "plugin" => ActorKind::Plugin,
                _ => ActorKind::Agent,
            }),
        }) {
            Answer::Ops(ops) => {
                let limit = args["limit"].as_u64().unwrap_or(50) as usize;
                let slice: Vec<_> = ops.iter().rev().take(limit).rev().cloned().collect();
                envelope(
                    true,
                    "OK",
                    "OpLog tail",
                    json!({"ops": slice, "count": ops.len(), "rev": ws.rev()}),
                )
            }
            _ => unreachable!(),
        }
    }
}

/// 冲突清单(三路合并显式冲突语义,护城河 #3)。
pub(crate) struct ConflictList;

impl CommandHandler for ConflictList {
    fn name(&self) -> &'static str {
        "conflict_list"
    }

    fn handle(&self, _args: &Value, cx: &mut HandlerCtx) -> Value {
        match cx.ws().conflict_list() {
            Ok(items) => {
                let rows: Vec<Value> = items
                    .iter()
                    .map(|(id, c)| json!({"conflictId": id, "code": c.code.code(), "pointer": c.pointer}))
                    .collect();
                envelope(true, "OK", "冲突清单", json!({"conflicts": rows}))
            }
            Err(e) => envelope(false, "INTERNAL", &e.to_string(), json!({})),
        }
    }
}

/// 时间线投影(E2-2:投影由服务端算好下放(endMs 等);壳零时间线语义,kf 采样同此)。
pub(crate) struct TimelineGet;

impl CommandHandler for TimelineGet {
    fn name(&self) -> &'static str {
        "timeline_get"
    }

    fn handle(&self, _args: &Value, cx: &mut HandlerCtx) -> Value {
        let ws = cx.ws();
        envelope(
            true,
            "OK",
            "时间线投影",
            json!({
                "clips": crate::edit_ops::timeline_projection(ws.project()),
                "rev": ws.rev(),
            }),
        )
    }
}

/// 会话报告(册七 T7.5:改动摘要,Markdown+JSON 双形态;只读投影)。
pub(crate) struct SessionReport;

impl CommandHandler for SessionReport {
    fn name(&self) -> &'static str {
        "session_report"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        crate::ai_ops::session_report_tool(cx.ws(), args)
    }
}
