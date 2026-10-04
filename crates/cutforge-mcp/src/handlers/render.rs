// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 渲染系工具处理器(E5/B6/T2.4/I1-M2/T5.6:免开工作区,不持排他锁)。
//! A-01 自 dispatch.rs :60-110 逐字迁移(行为零变化);实现体在 progress.rs。

use crate::handlers::{CommandHandler, HandlerCtx, Stage};
use crate::progress::{
    existing_rel, render_cutforge_sync, render_frame_tool, render_progress, render_run_async,
};
use serde_json::{Value, json};

/// render_run(册六 T6.3):frame-png 出口 = 单帧管线复用(atMs = inMs;同步单帧,
/// 与 render_frame 同口径),其余格式走异步导出队列。
pub(crate) struct RenderRun;

impl CommandHandler for RenderRun {
    fn name(&self) -> &'static str {
        "render_run"
    }

    fn stage(&self, _args: &Value) -> Stage {
        Stage::NoLock
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        if args["format"].as_str() == Some("frame-png") {
            let mut fargs = args.clone();
            if fargs["atMs"].is_null()
                && let Some(in_ms) = fargs["inMs"].as_u64()
            {
                fargs["atMs"] = json!(in_ms);
            }
            return render_frame_tool(cx.ws_root, &fargs);
        }
        render_run_async(
            cx.ws_root,
            existing_rel(cx.ws_root, args["ass"].as_str()),
            args["useProxy"].as_bool().unwrap_or(false),
            crate::progress::build_render_extra(args, true),
        )
    }
}

/// render_progress(E5-3 异步轮询)。
pub(crate) struct RenderProgress;

impl CommandHandler for RenderProgress {
    fn name(&self) -> &'static str {
        "render_progress"
    }

    fn stage(&self, _args: &Value) -> Stage {
        Stage::NoLock
    }

    fn handle(&self, args: &Value, _cx: &mut HandlerCtx) -> Value {
        let Some(run_id) = args["runId"].as_str() else {
            return crate::registry::envelope(false, "PRECONDITION_FAILED", "缺 runId", json!({}));
        };
        render_progress(run_id)
    }
}

/// render_frame(T2.4 单帧精确预览:同步出帧,帧缓存键含工作区指纹),不持锁。
pub(crate) struct RenderFrame;

impl CommandHandler for RenderFrame {
    fn name(&self) -> &'static str {
        "render_frame"
    }

    fn stage(&self, _args: &Value) -> Stage {
        Stage::NoLock
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        render_frame_tool(cx.ws_root, args)
    }
}

/// preview_zone_render(I1-M2 时间线区间半分辨率预渲;preview-cache 内容寻址;
/// 免开工作区不持锁)。
pub(crate) struct PreviewZoneRender;

impl CommandHandler for PreviewZoneRender {
    fn name(&self) -> &'static str {
        "preview_zone_render"
    }

    fn stage(&self, _args: &Value) -> Stage {
        Stage::NoLock
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        crate::progress::preview_zone_render_tool(cx.ws_root, args)
    }
}

/// render_queue(册五 T5.6 渲染队列;任务表为服务进程内存态 + jsonl 持久化)。
pub(crate) struct RenderQueue;

impl CommandHandler for RenderQueue {
    fn name(&self) -> &'static str {
        "render_queue"
    }

    fn stage(&self, _args: &Value) -> Stage {
        Stage::NoLock
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        crate::progress::render_queue_tool(cx.ws_root, args)
    }
}

/// render 按 backend 分派的共享判定:E5/B6——cutforge 后端调本地 cutforge-render
/// 子进程(不打开工作区不持锁);ffmpeg 后端(缺省)走 CutFlow 编排(工作区面,
/// 见 orchestrate::Render)。stage() 与本谓词必须同源(派发面一致性)。
pub(crate) fn is_cutforge_backend(args: &Value) -> bool {
    args["backend"].as_str() == Some("cutforge")
}

/// render(cutforge 后端分支体):同步整片渲染(ass 服务端过滤壳纯度;
/// useProxy 显式 opt-in;T5.6 渲染选项缺省零变化)。
pub(crate) fn render_cutforge_branch(args: &Value, cx: &mut HandlerCtx) -> Value {
    let use_proxy = args["useProxy"].as_bool().unwrap_or(false);
    let render_extra = crate::progress::build_render_extra(args, true);
    render_cutforge_sync(
        cx.ws_root,
        existing_rel(cx.ws_root, args["ass"].as_str()),
        use_proxy,
        &render_extra,
    )
}
