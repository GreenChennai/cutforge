// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 文本/字幕处理器(册四 A4 T4.7;实现集中在 subtitle_ops)。
//! A-01 自 dispatch.rs 巨 match 逐字迁移(行为零变化)。

use crate::handlers::{CommandHandler, HandlerCtx};
use serde_json::Value;

/// text_add:文本片段插入( textStyle 面与 clip_update 同契约)。
pub(crate) struct TextAdd;

impl CommandHandler for TextAdd {
    fn name(&self) -> &'static str {
        "text_add"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        crate::subtitle_ops::text_add_tool(cx.ws(), args, &actor, opts)
    }
}

/// subtitle_import:SRT/ASS 导入为字幕轨片段。
pub(crate) struct SubtitleImport;

impl CommandHandler for SubtitleImport {
    fn name(&self) -> &'static str {
        "subtitle_import"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let ws_root = cx.ws_root;
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        crate::subtitle_ops::subtitle_import_tool(cx.ws(), ws_root, args, &actor, opts)
    }
}

/// subtitle_export:SRT/ASS 导出(只读工程 + 派生物落盘,不产 Op 不改 IR)。
pub(crate) struct SubtitleExport;

impl CommandHandler for SubtitleExport {
    fn name(&self) -> &'static str {
        "subtitle_export"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let ws_root = cx.ws_root;
        crate::subtitle_ops::subtitle_export_tool(cx.ws(), ws_root, args)
    }
}

/// subtitle_replace:全片字幕查找替换。
pub(crate) struct SubtitleReplace;

impl CommandHandler for SubtitleReplace {
    fn name(&self) -> &'static str {
        "subtitle_replace"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        crate::subtitle_ops::subtitle_replace_tool(cx.ws(), args, &actor, opts)
    }
}
