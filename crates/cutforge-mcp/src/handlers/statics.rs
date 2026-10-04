// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 静态契约面处理器(免工程根):capability_matrix / plugin_validate。
//! A-01 自 dispatch.rs :40-54 逐字迁移(行为零变化)。

use crate::handlers::{CommandHandler, HandlerCtx, Stage};
use crate::registry::{capability_matrix, envelope};
use serde_json::{Value, json};

/// 能力对等矩阵:静态查询,无需工程根(ADR-0003/M8-5 单一真相源)。
pub(crate) struct CapabilityMatrix;

impl CommandHandler for CapabilityMatrix {
    fn name(&self) -> &'static str {
        "capability_matrix"
    }

    fn stage(&self, _args: &Value) -> Stage {
        Stage::Static
    }

    fn handle(&self, _args: &Value, _cx: &mut HandlerCtx) -> Value {
        envelope(
            true,
            "OK",
            "能力对等矩阵(实码口径,单一真相源)",
            json!({
                "matrix": capability_matrix(),
            }),
        )
    }
}

/// 册七 T7.2:插件 manifest 校验(纯契约面,免工程根;
/// 权限模型见 docs/PLUGIN-SPEC.md)。
pub(crate) struct PluginValidate;

impl CommandHandler for PluginValidate {
    fn name(&self) -> &'static str {
        "plugin_validate"
    }

    fn stage(&self, _args: &Value) -> Stage {
        Stage::Static
    }

    fn handle(&self, args: &Value, _cx: &mut HandlerCtx) -> Value {
        crate::plugin::plugin_validate_tool(args)
    }
}
