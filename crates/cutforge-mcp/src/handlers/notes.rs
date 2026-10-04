// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 标注面处理器:notes_add / notes_resolve / notes_reject(标注线程协议)+
//! note_reply(册七 T7.5 多轮追加,实现集中在 ai_ops)。
//! A-01 自 dispatch.rs 巨 match 逐字迁移(行为零变化)。

use crate::cutlist_ops::notes_op_error;
use crate::handlers::{CommandHandler, HandlerCtx};
use crate::registry::envelope;
use cutforge_core::anchor::{Anchor, AnchorKind};
use cutforge_core::oplog::Actor;
use serde_json::{Value, json};

/// notes_add:标注创建(锚点五类:clip/track/time/word/subtitleCard;缺省 clip)。
pub(crate) struct NotesAdd;

impl CommandHandler for NotesAdd {
    fn name(&self) -> &'static str {
        "notes_add"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let ws = cx.ws();
        let (Some(anchor_v), Some(body)) = (args["anchor"].as_object(), args["body"].as_str())
        else {
            return envelope(false, "PRECONDITION_FAILED", "缺 anchor/body", json!({}));
        };
        let kind = match anchor_v
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("clip")
        {
            "track" => AnchorKind::Track,
            "time" => AnchorKind::Time,
            "word" => AnchorKind::Word,
            "subtitleCard" => AnchorKind::SubtitleCard,
            _ => AnchorKind::Clip,
        };
        let anchor = Anchor {
            kind,
            // 注意:serde Map 的 Index 在键缺失时 panic,必须用 .get()(M10 e2e 实测)
            ref_: anchor_v
                .get("ref")
                .and_then(|v| v.as_str())
                .map(String::from),
            t_ms: anchor_v.get("tMs").and_then(|v| v.as_u64()).unwrap_or(0),
            span: None,
        };
        let author = if args["author"].as_str() == Some("agent") {
            cutforge_core::notes::NoteAuthor::Agent
        } else {
            cutforge_core::notes::NoteAuthor::User
        };
        let tags = args["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        match ws.notes_add(
            anchor,
            body.into(),
            author,
            tags,
            actor,
            args["requestId"].as_str().map(String::from),
        ) {
            Ok(id) => envelope(
                true,
                "OK",
                "标注已创建",
                json!({"noteId": id, "rev": ws.rev()}),
            ),
            Err(e) => envelope(false, "INTERNAL", &e.to_string(), json!({})),
        }
    }
}

/// notes_resolve:标注结案(reply + 关联 opIds)。
pub(crate) struct NotesResolve;

impl CommandHandler for NotesResolve {
    fn name(&self) -> &'static str {
        "notes_resolve"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let ws = cx.ws();
        let (Some(note_id), Some(reply)) = (args["noteId"].as_str(), args["reply"].as_str()) else {
            return envelope(false, "PRECONDITION_FAILED", "缺 noteId/reply", json!({}));
        };
        let op_ids = args["opIds"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        match ws.notes_resolve(note_id, reply.into(), op_ids, actor) {
            Ok(()) => envelope(true, "OK", "标注已结案", json!({"noteId": note_id})),
            Err(e) => notes_op_error(e),
        }
    }
}

/// notes_reject:标注否决。
pub(crate) struct NotesReject;

impl CommandHandler for NotesReject {
    fn name(&self) -> &'static str {
        "notes_reject"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor: Actor = cx.actor.clone();
        let ws = cx.ws();
        let (Some(note_id), Some(reason)) = (args["noteId"].as_str(), args["reason"].as_str())
        else {
            return envelope(false, "PRECONDITION_FAILED", "缺 noteId/reason", json!({}));
        };
        match ws.notes_reject(note_id, reason.into(), actor) {
            Ok(()) => envelope(true, "OK", "标注已否决", json!({"noteId": note_id})),
            Err(e) => notes_op_error(e),
        }
    }
}

/// note_reply(册七 T7.5):同一标注多轮追加回复(线程 id = 标注 id);
/// 不改 state、不碰 resolved_by——讨论史与结案回执分离。
pub(crate) struct NoteReply;

impl CommandHandler for NoteReply {
    fn name(&self) -> &'static str {
        "note_reply"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        crate::ai_ops::note_reply_tool(cx.ws(), args, &actor)
    }
}
