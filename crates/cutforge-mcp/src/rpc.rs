// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! JSON-RPC 传输面(册五 T5.2 拆分自 dispatch.rs——行数红线 A1-3,纯移动):
//! 处理一条 JSON-RPC 请求(stdio/HTTP 两通道共用);工具调用经
//! [`crate::dispatch::dispatch_with_actor`] 单一派发表,归因 actor 显式传递。

use crate::dispatch::dispatch_with_actor;

/// E6-3/B14 查询类:只读打开不排他锁;名单外写操作仍走 open_exclusive 全程锁。
/// (册五 T5.2 自 dispatch.rs 同域纯移动——行数红线 A1-3。)
pub(crate) fn is_readonly_tool(name: &str) -> bool {
    matches!(
        name,
        "project_get" | "wordline_get" | "cutlist_get" | "notes_list"
        | "oplog_tail" | "conflict_list" | "timeline_get"
        // 册四 T4.7:字幕导出只读工程(产物落 06_成片输出,不产 Op 不改 IR)
        // 册五 T5.5:OTIO/EDL 导出同类(派生物产物,不产 Op 不改 IR)
        | "subtitle_export" | "otio_export"
        // 册六 T6.1:工程库清单只读库根(卡片轻量派生,不产 Op 不改 IR)
        | "library_list"
        // 册六 T6.3/T6.2:导出前检查(轻探测)/ 素材库清单(库根扫描 +
        // manifest 派生索引,不触工程 IR)
        | "export_preflight" | "media_library"
        // 册七 T7.5/T7.2:改动预演(副本 dry-run,真工程零写入)/ 会话报告(只读投影)/
        // 插件 manifest 校验(纯契约面)
        | "preview_plan" | "session_report" | "plugin_validate"
    )
}

/// RT-1:该工具成功返回 rev 即视为一次会话内变更(会话摘要的采集口径)。
/// 排除:只读查询、免开工作区的静态/编排类、工程创建(不产 rev)、
/// clip_copy(会话态剪贴板写入,不产 Op 不升 rev)。
pub(crate) fn produces_rev_mutation(name: &str) -> bool {
    !(is_readonly_tool(name)
        || matches!(
            name,
            "capability_matrix" | "project_new" | "render" | "render_run" | "render_progress"
            | "render_frame" | "render_queue"
            | "media_probe" | "media_browse" | "render_probe" | "stage_status"
            | "clip_copy"
            // 册四 A4 T4.1/T4.8:派生物缓存与纯计算工具(产物非 IR,不升 rev)
            | "media_peaks" | "media_thumbnail" | "media_proxy" | "audio_beats"
            // 册五 T5.2/T5.3/T5.6:LUT 落库/示波器数据/响度计/编码探测(非 IR,不升 rev)
            | "lut_import" | "scope_data" | "audio_loudness" | "encode_probe"
            // 册五 T5.4/T5.5:多机位同步(纯计算)/字幕导出/OTIO 导出(派生物)/
            // OTIO 导入(从零建新工程,不动当前工作区 rev)
            | "subtitle_export" | "multicam_sync" | "otio_export" | "otio_import"
            // 册六 T6.1:库面目录级操作与迁移(不产 Op 不升 rev;OpLog 完整性不动)
            | "migrate_layout" | "library_manage" | "library_recover"
            // 册六 T6.3/T6.2:导出矩阵编排/预检与素材导入(拷贝落盘不产 Op;
            // 多画幅批量 = 渲染队列入队,不升 rev)
            | "export_preflight" | "export_all_variants" | "media_library" | "media_import"
            // 册七 T7.6:.cfpkg 打包/解包(目录级/容器级写面,不产 Op 不升 rev)
            | "project_package" | "project_unpackage"
        ))
}
use crate::registry::registry;
use cutforge_core::oplog::Actor;
use serde_json::{Value, json};

// ---------------- JSON-RPC 传输(两通道共用) ----------------

/// 处理一条 JSON-RPC 请求;通知(无 id)返回 None。
pub fn handle_rpc(req: &Value) -> Option<Value> {
    handle_rpc_as(req, Actor::agent("cutforge-mcp"))
}

/// JSON-RPC 带 actor 处理:壳数据面传 user(OpLog 如实归因;RT-1 摘要采集依据)。
pub fn handle_rpc_as(req: &Value, actor: Actor) -> Option<Value> {
    let method = req["method"].as_str()?;
    let id = req["id"].clone();
    if id.is_null() {
        return None; // 通知:不回应
    }
    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "cutforge-mcp", "version": env!("CARGO_PKG_VERSION")}
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": registry().iter().map(|t| json!({
            "name": t["name"], "description": t["description"],
            "inputSchema": t["inputSchema"],
        })).collect::<Vec<_>>()}),
        "tools/call" => {
            let name = req["params"]["name"].as_str().unwrap_or("");
            let args = req["params"]["arguments"].clone();
            let env = dispatch_with_actor(name, &args, actor);
            json!({
                "content": [{"type": "text", "text": env.to_string()}],
                "isError": env["ok"] != json!(true),
            })
        }
        other => {
            return Some(json!({
                "jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": format!("method not found: {other}")}
            }));
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}
