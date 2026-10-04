// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! `/api/v1` 版本化 Editor API(册七 T7.1/ADR-0025):URL 版本 REST 面。
//!
//! **单一真相源纪律**:本模块不实现任何业务逻辑——`POST /api/v1/tools/<tool>` 把
//! 参数对象原样转发进既有 `dispatch_with_actor` 单表(与 /rpc、stdio 三通道同一
//! 注册表,工具集差异恒为 0,M4-1);GET 别名只是常用查询的**同参转发**(查询串
//! 即参数,类型粗 coerce);事件流 `/api/v1/events` 由调用侧并入既有 /events 分支
//! (SSE 单一实现,transport::events)。鉴权与数据面同(Bearer token / ?token=),
//! 静态白名单不含本面。
//!
//! HTTP 状态码 = envelope.code 的确定性映射(协议字段不因换皮漂移):
//! OK→200;NO_CONFIG→404;CONFLICT/JIANYING_RUNNING→409;DEP_MISSING→503;
//! INTERNAL→500;SCHEMA_INVALID/PRECONDITION_FAILED/GUARD_FAILED/
//! GREEN_SCREEN_INPUT→400。未知工具/未知端点→404(code=INTERNAL,与 /rpc
//! "未知工具"语义一致)。

use crate::dispatch::dispatch_with_actor;
use crate::registry::envelope;
use crate::transport::http::{HttpResp, pct_decode};
use cutforge_core::oplog::Actor;
use serde_json::{Value, json};

/// GET 别名 → 工具名(常用只读查询的同参转发;参数经查询串透传)。
const GET_ALIASES: &[(&str, &str)] = &[
    ("project", "project_get"),
    ("timeline", "timeline_get"),
    ("notes", "notes_list"),
    ("oplog", "oplog_tail"),
    ("cutlist", "cutlist_get"),
    ("wordline", "wordline_get"),
    ("capabilities", "capability_matrix"),
];

/// 事件流别名(events 由调用侧并入 SSE/长轮询分支,单一实现)。
pub(crate) fn is_events_path(path: &str) -> bool {
    path == "/events" || path == "/api/v1/events"
}

/// 本模块处理的 /api/v1 面(events 除外——那在调用侧提前分流)。
pub(crate) fn is_api_v1(path: &str) -> bool {
    path == "/api/v1" || path.starts_with("/api/v1/")
}

/// envelope.code → HTTP 状态(确定性映射;见模块头)。
fn status_of(code: &str) -> &'static str {
    match code {
        "OK" => "200 OK",
        "NO_CONFIG" => "404 Not Found",
        "CONFLICT" | "JIANYING_RUNNING" => "409 Conflict",
        "DEP_MISSING" => "503 Service Unavailable",
        "INTERNAL" => "500 Internal Server Error",
        _ => "400 Bad Request",
    }
}

fn resp_json(status: &'static str, v: Value) -> HttpResp {
    HttpResp {
        status,
        ctype: "application/json".into(),
        extra: String::new(),
        body: super::http::RespBody::Bytes(v.to_string().into_bytes()),
    }
}

/// 查询串参数值粗 coerce:true/false/整数/浮点 → 对应 JSON 类型,否则原样字符串
/// (dispatch 各分支按 as_str/as_u64/as_bool 取用,与 /rpc JSON 参数同一读取面)。
fn coerce(s: &str) -> Value {
    match s {
        "true" => return json!(true),
        "false" => return json!(false),
        _ => {}
    }
    if let Ok(v) = s.parse::<u64>() {
        return json!(v);
    }
    if let Ok(v) = s.parse::<i64>() {
        return json!(v);
    }
    if let Ok(v) = s.parse::<f64>() {
        return json!(v);
    }
    json!(s)
}

/// 查询串 → 参数对象(a=b&a2=b2;pct 解码;root 显式给出时优先于绑定根)。
fn args_from_query(query: &str, bound_root: Option<&str>) -> Value {
    let mut args = serde_json::Map::new();
    for kv in query.split('&') {
        let Some((k, v)) = kv.split_once('=') else {
            continue;
        };
        if k.is_empty() || k == "token" {
            continue; // token 走鉴权面,不进工具参数
        }
        args.insert(k.to_string(), coerce(&pct_decode(v)));
    }
    if !args.contains_key("root")
        && let Some(r) = bound_root
    {
        args.insert("root".into(), json!(r));
    }
    Value::Object(args)
}

/// `/api/v1` 统一入口(events 已在调用侧分流)。bound_root = 工作区通道绑定的
/// 工程根(workspace_svc 有、辅通道无):工具入参与查询串缺 root 时注入,
/// 显式给的 root 不覆盖。actor 随通道归因(工作区 = editor,辅通道 = agent)。
pub(crate) fn handle_api_v1(
    path: &str,
    query: &str,
    is_post: bool,
    body: &str,
    bound_root: Option<&str>,
    actor: Actor,
) -> HttpResp {
    let sub = path.strip_prefix("/api/v1").unwrap_or("");
    let sub = sub.trim_start_matches('/');
    // GET /api/v1/tools → 工具清单(名/kind/描述;与 tools/list 同源注册表)
    if sub == "tools" {
        if !is_post {
            return resp_json(
                "200 OK",
                json!({
                    "ok": true, "code": "OK", "ns": crate::code_namespace("OK"),
                    "message": "工具清单(schemas/mcp-tools.json 单一真相源)",
                    "data": {"tools": crate::registry().iter().map(|t| json!({
                        "name": t["name"], "kind": t["kind"], "description": t["description"],
                    })).collect::<Vec<_>>()},
                }),
            );
        }
        return resp_json(
            "405 Method Not Allowed",
            envelope(
                false,
                "PRECONDITION_FAILED",
                "GET /api/v1/tools 只读;工具调用走 POST /api/v1/tools/<tool>",
                json!({}),
            ),
        );
    }
    // POST /api/v1/tools/<tool> → 统一工具入口(转发既有 dispatch 单表)
    if let Some(tool) = sub.strip_prefix("tools/") {
        if !is_post {
            return resp_json(
                "405 Method Not Allowed",
                envelope(
                    false,
                    "PRECONDITION_FAILED",
                    "工具调用必须 POST(参数对象为体)",
                    json!({}),
                ),
            );
        }
        let tool = pct_decode(tool);
        let parsed: Value = if body.trim().is_empty() {
            json!({})
        } else {
            match serde_json::from_str(body) {
                Ok(v @ Value::Object(_)) => v,
                Ok(_) => {
                    return resp_json(
                        "400 Bad Request",
                        envelope(
                            false,
                            "SCHEMA_INVALID",
                            "请求体必须是 JSON 对象(参数表)",
                            json!({}),
                        ),
                    );
                }
                Err(e) => {
                    return resp_json(
                        "400 Bad Request",
                        envelope(
                            false,
                            "SCHEMA_INVALID",
                            &format!("请求体不是合法 JSON: {e}"),
                            json!({}),
                        ),
                    );
                }
            }
        };
        let mut args = parsed;
        if args["root"].is_null()
            && let Some(root) = bound_root
        {
            args["root"] = json!(root);
        }
        let env = dispatch_with_actor(&tool, &args, actor);
        let status = if env["code"] == json!("INTERNAL")
            && env["message"]
                .as_str()
                .is_some_and(|m| m.starts_with("未知工具"))
        {
            "404 Not Found"
        } else {
            status_of(env["code"].as_str().unwrap_or("INTERNAL"))
        };
        return resp_json(status, env);
    }
    // GET 别名:常用查询同参转发
    if !is_post && let Some((_, tool)) = GET_ALIASES.iter().find(|(a, _)| *a == sub) {
        let args = args_from_query(query, bound_root);
        let env = dispatch_with_actor(tool, &args, actor);
        return resp_json(status_of(env["code"].as_str().unwrap_or("INTERNAL")), env);
    }
    // POST 到别名 / 未知端点 → 405 / 404(envelope 协议字段齐)
    if is_post && GET_ALIASES.iter().any(|(a, _)| *a == sub) {
        return resp_json(
            "405 Method Not Allowed",
            envelope(
                false,
                "PRECONDITION_FAILED",
                "该端点只读(GET);写操作走 POST /api/v1/tools/<tool>",
                json!({}),
            ),
        );
    }
    resp_json(
        "404 Not Found",
        envelope(
            false,
            "INTERNAL",
            &format!("未知端点: /api/v1/{sub}(见 docs/api/openapi.json)"),
            json!({}),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 状态码映射表(确定性;协议字段不因换皮漂移)。
    #[test]
    fn status_mapping_is_deterministic() {
        assert_eq!(status_of("OK"), "200 OK");
        assert_eq!(status_of("NO_CONFIG"), "404 Not Found");
        assert_eq!(status_of("CONFLICT"), "409 Conflict");
        assert_eq!(status_of("JIANYING_RUNNING"), "409 Conflict");
        assert_eq!(status_of("DEP_MISSING"), "503 Service Unavailable");
        assert_eq!(status_of("INTERNAL"), "500 Internal Server Error");
        for c in [
            "SCHEMA_INVALID",
            "PRECONDITION_FAILED",
            "GUARD_FAILED",
            "GREEN_SCREEN_INPUT",
        ] {
            assert_eq!(status_of(c), "400 Bad Request", "{c}");
        }
    }

    /// 查询串 coerce:布尔/整数/浮点/字符串/百分号解码;token 不进参数。
    #[test]
    fn query_coercion_and_token_filter() {
        let a = args_from_query(
            "state=open&sinceRev=7&limit=50&ratio=0.5&q=%E4%B8%AD&token=x",
            None,
        );
        assert_eq!(a["state"], json!("open"));
        assert_eq!(a["sinceRev"], json!(7));
        assert_eq!(a["limit"], json!(50));
        assert_eq!(a["ratio"], json!(0.5));
        assert_eq!(a["q"], json!("中"));
        assert!(a.get("token").is_none(), "token 不进工具参数");
        // 绑定根注入;显式 root 不覆盖
        let b = args_from_query("", Some("/ws"));
        assert_eq!(b["root"], json!("/ws"));
        let c = args_from_query("root=/other", Some("/ws"));
        assert_eq!(c["root"], json!("/other"));
    }

    /// 别名与未知工具:capability_matrix 别名 OK;未知端点 404;GET 工具入口 405。
    #[test]
    fn alias_forward_and_unknown_paths() {
        let r = handle_api_v1(
            "/api/v1/capabilities",
            "",
            false,
            "",
            None,
            Actor::agent("t"),
        );
        assert_eq!(r.status, "200 OK");
        let env: Value = serde_json::from_slice(r.body_bytes().unwrap()).unwrap();
        assert_eq!(env["ok"], json!(true));
        // 未知工具 → 404 + INTERNAL(未知工具语义)
        let r = handle_api_v1(
            "/api/v1/tools/无此工具",
            "",
            true,
            "{}",
            None,
            Actor::agent("t"),
        );
        assert_eq!(r.status, "404 Not Found");
        // 未知端点 → 404
        let r = handle_api_v1("/api/v1/无此端点", "", false, "", None, Actor::agent("t"));
        assert_eq!(r.status, "404 Not Found");
        // GET 工具入口 → 405
        let r = handle_api_v1(
            "/api/v1/tools/project_get",
            "",
            false,
            "",
            None,
            Actor::agent("t"),
        );
        assert_eq!(r.status, "405 Method Not Allowed");
        // 非法体 → 400 + SCHEMA_INVALID
        let r = handle_api_v1(
            "/api/v1/tools/project_get",
            "",
            true,
            "[1,2]",
            None,
            Actor::agent("t"),
        );
        assert_eq!(r.status, "400 Bad Request");
        let env: Value = serde_json::from_slice(r.body_bytes().unwrap()).unwrap();
        assert_eq!(env["code"], json!("SCHEMA_INVALID"));
    }
}
