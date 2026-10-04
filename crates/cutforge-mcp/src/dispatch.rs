// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 命令通道统一派发(A-01 注册表化):所有通道(stdio/HTTP/脚本宿主)共用的
//! 通道入口、执行面路由(静态/免锁/工作区)、幂等预检与 5.4 错误映射。
//! 工具分派体已按域拆入 `handlers/`(注册表单一真相源,新增命令 = 一个文件 +
//! 注册一行);本文件只承载通道骨架与共用助手——拆分行为逐字节零变化,
//! tool_parity 黄金对拍为证(T1.1 拆分自 lib.rs,A-01 拆分自本文件巨 match)。

use crate::handlers::{HandlerCtx, Stage, lookup};
use crate::registry::{envelope, tool_def};
use cutforge_core::engine::ApplyOpts;
use cutforge_core::oplog::Actor;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

// ---------------- 派发 ----------------

/// 命令通道统一派发(stdio/HTTP/脚本宿主共用);root = args["root"]。
pub fn dispatch(name: &str, args: &Value) -> Value {
    dispatch_with_actor(name, args, Actor::agent("cutforge-mcp"))
}

/// 带显式 actor 的派发:壳传 `Actor::user("editor")`——手势在 OpLog 如实归因
/// (RT-1 会话摘要按 actor=human 过滤);其余通道 agent 归因。
pub fn dispatch_with_actor(name: &str, args: &Value, actor: Actor) -> Value {
    if tool_def(name).is_none() {
        return envelope(false, "INTERNAL", &format!("未知工具: {name}"), json!({}));
    }
    let Some(handler) = lookup(name) else {
        // 注册面缺口(JSON 契约有、处理器缺):与旧派发表 fallthrough 同形同码;
        // registry_matches_json_contract_exactly 测试保证此分支对正式工具不可达。
        return envelope(
            false,
            "INTERNAL",
            &format!("工具已注册但未实现: {name}"),
            json!({}),
        );
    };
    match handler.stage(args) {
        Stage::Static => handler.handle(args, &mut HandlerCtx::rootless(actor)),
        Stage::NoLock => {
            // E5/B6/E6-3/B14:渲染系/只读/创建类免开工作区——root 必给,
            // 但不进常驻工作区(不持排他锁、不做幂等预检;与旧派发表前置
            // 检查顺序逐分支等价)。
            let Some(root_str) = args["root"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 root(工程目录)", json!({}));
            };
            let ws_root = PathBuf::from(root_str);
            handler.handle(args, &mut HandlerCtx::nolock(root_str, &ws_root, actor))
        }
        Stage::Workspace => {
            let Some(root_str) = args["root"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 root(工程目录)", json!({}));
            };
            let ws_root = PathBuf::from(root_str);
            // 常驻同步守护(M9-2):外部改动 ≤1s 可见;常驻缓存(T1.8)指纹一致即复用。
            // 查询类只读零锁;写类 open_for_write + apply 内临时全程锁,合并/停写语义原样。
            let readonly = is_readonly_tool(name);
            // R-14:幂等 O(1) 预检——常驻索引在指纹复核窗口内与 OpLog 全集一致,命中即
            // 短路为与引擎幂等路径同形的回执(免开工作区锁/免全量扫描);未命中一律
            // 放行,引擎内 has_request_id 全量判定仍是正确性底线。
            if !readonly
                && let Some(rid) = args["requestId"].as_str()
                && let Some(rev) = crate::resident::idempotent_precheck(root_str, &ws_root, rid)
            {
                return envelope(
                    true,
                    "OK",
                    "已应用",
                    json!({"opIds": [], "rev": rev, "idempotent": true}),
                );
            }
            crate::resident::with_resident(root_str, &ws_root, readonly, |ws| {
                let mut cx = HandlerCtx::workspace(root_str, &ws_root, actor, apply_opts(args), ws);
                handler.handle(args, &mut cx)
            })
        }
    }
}

/// ApplyOpts 自 args 派生(requestId/summary/causedBy/expectRev)——旧
/// dispatch_on_ws 的构造原样(幂等/并发前置,护城河 #2)。
fn apply_opts(args: &Value) -> ApplyOpts {
    ApplyOpts {
        request_id: args["requestId"].as_str().map(String::from),
        summary: args["summary"].as_str().map(String::from),
        caused_by: args["causedBy"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        expect_rev: args["expectRev"].as_u64(),
        ..Default::default()
    }
}

pub(crate) fn finish_apply(r: Result<cutforge_core::engine::OpReceipt, std::io::Error>) -> Value {
    match r {
        Ok(rec) => envelope(
            true,
            "OK",
            "已应用",
            json!({"opIds": rec.op_ids, "rev": rec.rev, "idempotent": rec.idempotent}),
        ),
        Err(e) => reject_to_envelope(e.to_string()),
    }
}

fn reject_to_envelope(msg: String) -> Value {
    let code = if msg.starts_with("PRECONDITION_FAILED") {
        "PRECONDITION_FAILED"
    } else if msg.starts_with("CONFLICT") {
        "CONFLICT"
    } else if msg.starts_with("SCHEMA_INVALID") {
        "SCHEMA_INVALID"
    } else if msg.starts_with("GUARD_FAILED") {
        "GUARD_FAILED"
    } else if msg.starts_with("NOTHING_TO_") {
        "PRECONDITION_FAILED"
    } else {
        "INTERNAL"
    };
    envelope(false, code, &msg, json!({}))
}

// E6-3/B14 查询类名单(实现在 rpc.rs 同域纯移动——行数红线 A1-3)
pub(crate) use crate::rpc::{is_readonly_tool, produces_rev_mutation};

/// 下一个片段 id(轨道内序号递增;轨道不存在时回退 <轨id>-999 的显式占位)。
pub(crate) fn next_clip_id_for(project: &cutforge_core::model::Project, track_id: &str) -> String {
    project
        .find_track(track_id)
        .map(|ti| cutforge_core::model::Project::next_clip_id(&project.tracks[ti]))
        .unwrap_or_else(|| format!("{track_id}-999"))
}

/// E2-1 的 canonicalize 校验函数化:/media、clip_add、media_probe 等共用
/// (相对路径、拒 `..`、canonicalize 后仍在工程根内);新端点禁止另造并行实现。
pub(crate) fn resolve_within_root(root: &Path, rel: &str) -> Result<PathBuf, &'static str> {
    if rel.is_empty() {
        return Err("路径为空");
    }
    if Path::new(rel).is_absolute() || rel.split(['/', '\\']).any(|seg| seg == "..") {
        return Err("非法路径");
    }
    let Ok(canon_t) = root.join(rel).canonicalize() else {
        return Err("路径不存在或不可达");
    };
    let Ok(canon_r) = root.canonicalize() else {
        return Err("工程根不可达");
    };
    if !canon_t.starts_with(&canon_r) {
        return Err("路径越出工程根");
    }
    Ok(canon_t)
}

// JSON-RPC 传输面(册五 T5.2 拆分自本文件——行数红线 A1-3,纯移动;
// `pub use` 保持 `cutforge_mcp::dispatch::handle_rpc*` 路径逐字不变)
pub use crate::rpc::{handle_rpc, handle_rpc_as};
