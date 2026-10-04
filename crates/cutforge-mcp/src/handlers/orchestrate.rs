// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 编排处理器(封装 CutFlow 既有脚本,子进程透传,不重实现阶段逻辑——编排纪律
//! = 参数接线)。A-01 自 dispatch.rs 巨 match 编排段逐字迁移(行为零变化);
//! render 的 ffmpeg 后端分支同走此面(E5/B6:backend 按 stage() 动态裁定)。

use crate::handlers::{CommandHandler, HandlerCtx};
use crate::orchestrate::orchestrate;
use cutforge_io::paths;
use serde_json::{Value, json};

/// 编排脚本映射(stage 面 → CutFlow 脚本;单一实现,与旧派发表逐字一致)。
fn script_for(name: &str) -> &'static str {
    match name {
        "stage_run" => "rs_run.py",
        "stage_rebuild" => "rebuild.py",
        "verify_run" => "rs_verify.py",
        "sync_check" => "rs_sync.py",
        "render" => "rs_render.py",
        _ => "rs_jy_draft.py",
    }
}

/// 编排共享体:scriptArgs 整组透传;export_jianying 缺省参数端到端接线。
fn run_stage(name: &str, args: &Value, cx: &mut HandlerCtx) -> Value {
    let ws_root = cx.ws_root;
    let script = script_for(name);
    let mut script_args = args["scriptArgs"].as_array().cloned().unwrap_or_default();
    if name == "export_jianying" && script_args.is_empty() {
        // 册六 T6.2/ADR-0023 随包收编后的端到端缺省:project 路径(三态布局
        // 感知)+ --name(契约 required)。显式 scriptArgs 仍整组透传(编排
        // 纪律 = 参数接线,不实现阶段逻辑)。
        script_args.push(json!(paths::project_path(ws_root).to_string_lossy()));
        if let Some(n) = args["name"].as_str() {
            script_args.push(json!("--name"));
            script_args.push(json!(n));
        }
    }
    orchestrate(ws_root, script, &script_args)
}

/// stage_run:阶段执行(S0-S11)。
pub(crate) struct StageRun;

impl CommandHandler for StageRun {
    fn name(&self) -> &'static str {
        "stage_run"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        run_stage(self.name(), args, cx)
    }
}

/// stage_rebuild:阶段重建。
pub(crate) struct StageRebuild;

impl CommandHandler for StageRebuild {
    fn name(&self) -> &'static str {
        "stage_rebuild"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        run_stage(self.name(), args, cx)
    }
}

/// verify_run:L1/L2 验证。
pub(crate) struct VerifyRun;

impl CommandHandler for VerifyRun {
    fn name(&self) -> &'static str {
        "verify_run"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        run_stage(self.name(), args, cx)
    }
}

/// sync_check:音画同步检查(qc 面)。
pub(crate) struct SyncCheck;

impl CommandHandler for SyncCheck {
    fn name(&self) -> &'static str {
        "sync_check"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        run_stage(self.name(), args, cx)
    }
}

/// render:ffmpeg 后端(缺省)走 CutFlow rs_render.py 编排(工作区面);
/// backend="cutforge" 在 stage() 即改道免锁面(E5/B6,见 handlers::render)。
pub(crate) struct Render;

impl CommandHandler for Render {
    fn name(&self) -> &'static str {
        "render"
    }

    fn stage(&self, args: &Value) -> super::Stage {
        if super::render::is_cutforge_backend(args) {
            super::Stage::NoLock
        } else {
            super::Stage::Workspace
        }
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        if super::render::is_cutforge_backend(args) {
            return super::render::render_cutforge_branch(args, cx);
        }
        run_stage(self.name(), args, cx)
    }
}

/// export_jianying:剪映草稿导出(ADR-0023 随包收编;缺省参数见 run_stage)。
pub(crate) struct ExportJianying;

impl CommandHandler for ExportJianying {
    fn name(&self) -> &'static str {
        "export_jianying"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        run_stage(self.name(), args, cx)
    }
}
