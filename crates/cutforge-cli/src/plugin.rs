// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 插件调用通道(册七 T7.2/ADR-0024 外部进程形态):
//! `cutforge-cli plugin-call <manifest> <tool> --args-json '{…}'`
//! 服务端裁决 = manifest 权限面 vs 工具分类(查询→read / 写→write / 编排→exec);
//! 越权 GUARD_FAILED(FORBIDDEN 语义)拒绝;放行后以 actor=plugin 走同一
//! dispatch 单表——写操作全部经 Op 通道留痕可撤销,无旁路。

use crate::{emit, Args};
use serde_json::json;

pub fn run(a: &Args) -> i32 {
    let usage = "用法: plugin-call <manifest.json> <tool> [--args-json '{…}']";
    let (Some(manifest_path), Some(tool)) = (a.positional.first(), a.positional.get(1)) else {
        return emit(a.json, false, "PRECONDITION_FAILED", usage, json!({}));
    };
    // 1) manifest 装载 + 校验(契约面与 plugin_validate 工具同一实现)
    let manifest: serde_json::Value = match std::fs::read_to_string(manifest_path)
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str(&t).map_err(|e| format!("manifest 非合法 JSON: {e}")))
    {
        Ok(v) => v,
        Err(e) => return emit(a.json, false, "NO_CONFIG", &e, json!({})),
    };
    let errs = cutforge_mcp::validate_manifest(&manifest);
    if !errs.is_empty() {
        return emit(a.json, false, "SCHEMA_INVALID", "manifest 不合法",
            json!({"manifest": manifest_path, "errors": errs}));
    }
    // 2) 权限裁决(读/写/编排 vs 声明面)
    if let Err((code, msg)) = cutforge_mcp::authorize(&manifest, tool) {
        return emit(a.json, false, &code, &msg, json!({"tool": tool}));
    }
    // 3) actor=plugin 经单一 dispatch 单表执行(写走 Op 通道,OpLog 如实归因)
    let args: serde_json::Value = a.flags.get("args-json")
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| json!({}));
    let env = cutforge_mcp::dispatch_with_actor(tool, &args, cutforge_mcp::plugin_actor(&manifest));
    // 结果协议原样输出(envelope;含 ns 加法字段),退出码按 code 族归位
    println!("{env}");
    match env["code"].as_str().unwrap_or("INTERNAL") {
        "OK" => emit(a.json, true, "OK", "插件调用完成", env["data"].clone()),
        c if c == "NO_CONFIG" || c == "DEP_MISSING" => emit(a.json, false, c, "环境缺失", env["data"].clone()),
        c => emit(a.json, false, c, env["message"].as_str().unwrap_or("插件调用失败"), env["data"].clone()),
    }
}
