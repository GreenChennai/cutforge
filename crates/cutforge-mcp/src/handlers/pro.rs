// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 专业编辑处理器(册五 T5.4/T5.5:复合片段/多机位/场景检测/OTIO-EDL 导出;
//! 实现集中在 pro_ops)。A-01 自 dispatch.rs 巨 match 逐字迁移(行为零变化)。

use crate::handlers::{CommandHandler, HandlerCtx};
use serde_json::Value;

/// compound_create:复合片段封装(单 Op)。
pub(crate) struct CompoundCreate;

impl CommandHandler for CompoundCreate {
    fn name(&self) -> &'static str {
        "compound_create"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        crate::pro_ops::compound_create_tool(cx.ws(), args, &actor, opts)
    }
}

/// compound_unbind:复合片段摘除(还原成员)。
pub(crate) struct CompoundUnbind;

impl CommandHandler for CompoundUnbind {
    fn name(&self) -> &'static str {
        "compound_unbind"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        crate::pro_ops::compound_unbind_tool(cx.ws(), args, &actor, opts)
    }
}

/// multicam_cut:多机位切换序列展开(单 Op)。
pub(crate) struct MulticamCut;

impl CommandHandler for MulticamCut {
    fn name(&self) -> &'static str {
        "multicam_cut"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        crate::pro_ops::multicam_cut_tool(cx.ws(), args, &actor, opts)
    }
}

/// scene_detect:帧差分场景检测(纯计算;autoSplit 真切段走命令通道)。
pub(crate) struct SceneDetect;

impl CommandHandler for SceneDetect {
    fn name(&self) -> &'static str {
        "scene_detect"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let ws_root = cx.ws_root;
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        crate::pro_ops::scene_detect_tool(cx.ws(), ws_root, args, &actor, opts)
    }
}

/// otio_export:互操作导出(只读工程 + 派生物落盘,与 subtitle_export 同类)。
pub(crate) struct OtioExport;

impl CommandHandler for OtioExport {
    fn name(&self) -> &'static str {
        "otio_export"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let ws_root = cx.ws_root;
        crate::pro_ops::otio_export_tool(cx.ws(), ws_root, args)
    }
}
