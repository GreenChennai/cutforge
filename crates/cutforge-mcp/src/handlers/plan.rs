// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! AI 改动预演/批准处理器(册七 T7.5;plan 面自管工作区:预演在副本工程上全链
//! dry-run 不落盘,应用逐项重入本单表——causedBy 链关联 planId)。
//! A-01 自 dispatch.rs :156-162 逐字迁移(行为零变化);实现体在 ai_ops.rs。

use crate::handlers::{CommandHandler, HandlerCtx, Stage};
use serde_json::Value;

/// preview_plan(查询面):副本 dry-run,真工程不落盘不产 Op。
pub(crate) struct PreviewPlan;

impl CommandHandler for PreviewPlan {
    fn name(&self) -> &'static str {
        "preview_plan"
    }

    fn stage(&self, _args: &Value) -> Stage {
        Stage::NoLock
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        crate::ai_ops::preview_plan_tool(cx.ws_root, args, cx.actor.clone())
    }
}

/// apply_plan(写面):仅执行被批准项(缺省拒绝,无越权写入)。
pub(crate) struct ApplyPlan;

impl CommandHandler for ApplyPlan {
    fn name(&self) -> &'static str {
        "apply_plan"
    }

    fn stage(&self, _args: &Value) -> Stage {
        Stage::NoLock
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        crate::ai_ops::apply_plan_tool(cx.root_str, args, cx.actor.clone())
    }
}
