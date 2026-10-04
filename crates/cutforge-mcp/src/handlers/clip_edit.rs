// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 片段结构编辑处理器(切分/删除/移动/复制 + 册四 A4 T4.2 时间线编辑全工具,
//! 实现集中在 edit_ops)。A-01 自 dispatch.rs 巨 match 逐字迁移(行为零变化)。

use crate::dispatch::finish_apply;
use crate::handlers::{CommandHandler, HandlerCtx};
use crate::registry::envelope;
use cutforge_core::command::Command;
use serde_json::{Value, json};

/// clip_split:位置切分(Command::ClipSplit)。
pub(crate) struct ClipSplit;

impl CommandHandler for ClipSplit {
    fn name(&self) -> &'static str {
        "clip_split"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        let ws = cx.ws();
        match args["clipId"].as_str().zip(args["tMs"].as_u64()) {
            Some((clip_id, t_ms)) => finish_apply(ws.apply(
                Command::ClipSplit {
                    clip_id: clip_id.into(),
                    t_ms,
                },
                actor,
                opts,
            )),
            None => envelope(false, "PRECONDITION_FAILED", "缺 clipId/tMs", json!({})),
        }
    }
}

/// clip_delete:删除片段(Command::ClipDelete)。
pub(crate) struct ClipDelete;

impl CommandHandler for ClipDelete {
    fn name(&self) -> &'static str {
        "clip_delete"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        let ws = cx.ws();
        match args["clipId"].as_str() {
            Some(clip_id) => finish_apply(ws.apply(
                Command::ClipDelete {
                    clip_id: clip_id.into(),
                },
                actor,
                opts,
            )),
            None => envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({})),
        }
    }
}

/// clip_move:跨轨移动(Command::ClipMove;toTrack 可选)。
pub(crate) struct ClipMove;

impl CommandHandler for ClipMove {
    fn name(&self) -> &'static str {
        "clip_move"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        let ws = cx.ws();
        let (Some(clip_id), Some(start_ms)) = (args["clipId"].as_str(), args["startMs"].as_u64())
        else {
            return envelope(false, "PRECONDITION_FAILED", "缺 clipId/startMs", json!({}));
        };
        finish_apply(ws.apply(
            Command::ClipMove {
                clip_id: clip_id.into(),
                new_start_ms: start_ms,
                to_track: args["toTrack"].as_str().map(String::from),
            },
            actor,
            opts,
        ))
    }
}

/// clip_duplicate:同轨/跨轨复制(kind 守护 GUARD_FAILED;新 id 按目标轨序号)。
pub(crate) struct ClipDuplicate;

impl CommandHandler for ClipDuplicate {
    fn name(&self) -> &'static str {
        "clip_duplicate"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        let ws = cx.ws();
        let (Some(clip_id), Some(start_ms)) = (args["clipId"].as_str(), args["startMs"].as_u64())
        else {
            return envelope(false, "PRECONDITION_FAILED", "缺 clipId/startMs", json!({}));
        };
        let src_track = ws.project().find_clip(clip_id).map(|(ti, _)| ti);
        let Some(src_ti) = src_track else {
            return envelope(
                false,
                "PRECONDITION_FAILED",
                &format!("clip 不存在: {clip_id}"),
                json!({}),
            );
        };
        let to_track = args["toTrack"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| ws.project().tracks[src_ti].id.clone());
        let Some(ti) = ws.project().find_track(&to_track) else {
            return envelope(
                false,
                "PRECONDITION_FAILED",
                &format!("track 不存在: {to_track}"),
                json!({}),
            );
        };
        let mut clip =
            ws.project().tracks[src_ti].clips[ws.project().find_clip(clip_id).unwrap().1].clone();
        if ws.project().tracks[ti].kind != ws.project().tracks[src_ti].kind {
            return envelope(false, "GUARD_FAILED", "跨 kind 复制拒绝", json!({}));
        }
        clip.id = cutforge_core::model::Project::next_clip_id(&ws.project().tracks[ti]);
        clip.start_ms = start_ms;
        let request_id = args["requestId"].as_str().map(String::from);
        finish_apply(ws.apply(
            Command::ClipInsert {
                to_track,
                clip,
                request_id,
            },
            actor,
            opts,
        ))
    }
}

/// clip_trim(册四 A4 T4.2:trim/roll/slip/slide;实现集中在 edit_ops)。
pub(crate) struct ClipTrim;

impl CommandHandler for ClipTrim {
    fn name(&self) -> &'static str {
        "clip_trim"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        crate::edit_ops::clip_trim_tool(cx.ws(), args, &actor, opts)
    }
}

/// clip_split_all:全轨位置切分。
pub(crate) struct ClipSplitAll;

impl CommandHandler for ClipSplitAll {
    fn name(&self) -> &'static str {
        "clip_split_all"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        crate::edit_ops::clip_split_all_tool(cx.ws(), args, &actor, opts)
    }
}

/// clip_gap_delete:轨道间隙闭合(后继整体左移)。
pub(crate) struct ClipGapDelete;

impl CommandHandler for ClipGapDelete {
    fn name(&self) -> &'static str {
        "clip_gap_delete"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        crate::edit_ops::clip_gap_delete_tool(cx.ws(), args, &actor, opts)
    }
}

/// clip_copy:会话态剪贴板写入(不产 Op 不升 rev)。
pub(crate) struct ClipCopy;

impl CommandHandler for ClipCopy {
    fn name(&self) -> &'static str {
        "clip_copy"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let root_str = cx.root_str;
        crate::edit_ops::clip_copy_tool(cx.ws(), root_str, args)
    }
}

/// clip_paste_at:剪贴板定点粘贴。
pub(crate) struct ClipPasteAt;

impl CommandHandler for ClipPasteAt {
    fn name(&self) -> &'static str {
        "clip_paste_at"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let root_str = cx.root_str;
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        crate::edit_ops::clip_paste_at_tool(cx.ws(), root_str, args, &actor, opts)
    }
}
