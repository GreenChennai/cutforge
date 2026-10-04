// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 单一派发表:所有通道(stdio/HTTP/脚本宿主)共用的工具分派、JSON-RPC 面、
//! 参数解析与 5.4 错误映射(T1.1 拆分自 lib.rs,纯移动)。

use crate::cutlist_ops::{apply_cut_merge_patch, notes_op_error, read_truth};
use crate::edit_ops;
use crate::library_tools::{
    library_list_tool, library_manage_tool, library_recover_tool, migrate_layout_tool,
};
use crate::orchestrate::orchestrate;
use crate::progress::{
    existing_rel, render_cutforge_sync, render_frame_tool, render_progress, render_run_async,
};
use crate::registry::{capability_matrix, envelope, tool_def};
use crate::subtitle_ops;
use crate::tools_nolock::{
    media_browse_tool, media_probe_tool, project_new_tool, render_probe_tool, stage_status_tool,
};
use cutforge_core::anchor::{Anchor, AnchorKind};
use cutforge_core::command::{BgmPatch, ClipPatch, Command, MotionPatch, TransitionPatch};
use cutforge_core::engine::{Answer, ApplyOpts, Query};
use cutforge_core::oplog::{Actor, ActorKind};
use cutforge_io::paths;
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
    if name == "capability_matrix" {
        // 静态查询,无需工程根
        return envelope(
            true,
            "OK",
            "能力对等矩阵(实码口径,单一真相源)",
            json!({
                "matrix": capability_matrix(),
            }),
        );
    }
    // 册七 T7.2:插件 manifest 校验(纯契约面,免工程根;权限模型见 docs/PLUGIN-SPEC.md)
    if name == "plugin_validate" {
        return crate::plugin::plugin_validate_tool(args);
    }
    let Some(root_str) = args["root"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 root(工程目录)", json!({}));
    };
    let ws_root = PathBuf::from(root_str);

    // E5/B6:render 按 backend 分派——cutforge 后端调本地 cutforge-render 子进程
    // (不打开工作区不持锁);ffmpeg 后端(默认)走 CutFlow 编排。render_run/
    // render_progress 同理免锁(E5-3 异步 + 轮询;T5.6 队列化)。ass 服务端过滤
    // (壳纯度);useProxy 显式 opt-in;T5.6 渲染选项缺省零变化。
    let use_proxy = args["useProxy"].as_bool().unwrap_or(false);
    let render_extra = crate::progress::build_render_extra(args, true);
    if name == "render" && args["backend"].as_str() == Some("cutforge") {
        return render_cutforge_sync(
            &ws_root,
            existing_rel(&ws_root, args["ass"].as_str()),
            use_proxy,
            &render_extra,
        );
    }
    if name == "render_run" {
        // 册六 T6.3:frame-png 出口 = 单帧管线复用(atMs = inMs;同步单帧,
        // 与 render_frame 同口径),其余格式走异步导出队列。
        if args["format"].as_str() == Some("frame-png") {
            let mut fargs = args.clone();
            if fargs["atMs"].is_null()
                && let Some(in_ms) = fargs["inMs"].as_u64()
            {
                fargs["atMs"] = json!(in_ms);
            }
            return render_frame_tool(&ws_root, &fargs);
        }
        return render_run_async(
            &ws_root,
            existing_rel(&ws_root, args["ass"].as_str()),
            use_proxy,
            render_extra,
        );
    }
    if name == "render_progress" {
        let Some(run_id) = args["runId"].as_str() else {
            return envelope(false, "PRECONDITION_FAILED", "缺 runId", json!({}));
        };
        return render_progress(run_id);
    }
    // T2.4 单帧精确预览:同步出帧(帧缓存键含工作区指纹),不持锁
    if name == "render_frame" {
        return render_frame_tool(&ws_root, args);
    }
    // I1-M2 时间线区间半分辨率预渲(preview-cache 内容寻址;免开工作区不持锁)
    if name == "preview_zone_render" {
        return crate::progress::preview_zone_render_tool(&ws_root, args);
    }
    // 册五 T5.6 渲染队列(免开工作区;任务表为服务进程内存态)
    if name == "render_queue" {
        return crate::progress::render_queue_tool(&ws_root, args);
    }

    // ---- 免开工作区的工具(E6-3/B14:只读/创建类不持排他锁) ----
    match name {
        "project_new" => return project_new_tool(&ws_root, args),
        "media_probe" => return media_probe_tool(&ws_root, args),
        "media_browse" => return media_browse_tool(&ws_root, args),
        "render_probe" => return render_probe_tool(&ws_root),
        "stage_status" => return stage_status_tool(&ws_root),
        // 册四 T4.1/T4.8 媒体池/音频工具(派生物缓存与纯计算,不改工程 IR 不持锁)
        "media_peaks" => return crate::media_tools::media_peaks_tool(&ws_root, args),
        "media_thumbnail" => return crate::media_tools::media_thumbnail_tool(&ws_root, args),
        // BUG-19:批量缩略图(一次请求多帧,磁盘缓存命中合并;壳侧 4N 次 IPC → 1 次)
        "media_thumbs" => return crate::media_tools::media_thumbs_tool(&ws_root, args),
        "media_proxy" => return crate::media_tools::media_proxy_tool(&ws_root, args),
        "audio_beats" => return crate::media_tools::audio_beats_tool(&ws_root, args),
        // 册五 T5.2/T5.3/T5.6:调色 LUT/示波器/响度计/编码探测(同口径免锁)
        "lut_import" => return crate::grade_tools::lut_import_tool(&ws_root, args),
        "scope_data" => return crate::grade_tools::scope_data_tool(&ws_root, args),
        "audio_loudness" => return crate::grade_tools::audio_loudness_tool(&ws_root, args),
        "encode_probe" => return crate::grade_tools::encode_probe_tool(&ws_root, args),
        // 册五 T5.4/T5.5:多机位同步分析(纯计算)/ OTIO 导入(从零建工程,免锁)
        "multicam_sync" => return crate::pro_ops::multicam_sync_tool(&ws_root, args),
        "otio_import" => return crate::pro_ops::otio_import_tool(&ws_root, args),
        // 册六 T6.1:布局迁移(工程目录)/ 工程库与崩溃恢复(root = 库根;免锁面,
        // 目录级操作由 io 层自持锁)
        "migrate_layout" => return migrate_layout_tool(&ws_root, args),
        "library_manage" => return library_manage_tool(&ws_root, args),
        "library_list" => return library_list_tool(&ws_root, args),
        "library_recover" => return library_recover_tool(&ws_root, args),
        // 册六 T6.3:导出前检查(轻探测)/ 多画幅批量(编排入队;均免开工作区)
        "export_preflight" => return crate::export_tools::export_preflight_tool(&ws_root, args),
        "export_all_variants" => {
            return crate::export_tools::export_all_variants_tool(&ws_root, args);
        }
        // 册六 T6.2:素材库 manifest(库根)+ 素材拷贝导入(工程;免开工作区,
        // 不产 Op 不改 IR,与 lut_import 同类写面)
        "media_library" => return crate::media_library::media_library_tool(&ws_root, args),
        "media_import" => return crate::media_library::media_import_tool(&ws_root, args),
        // 册七 T7.6:.cfpkg 工程打包/解包(免开工作区;打包持锁在 io 层自持,
        // 解包写全新目录拒绝覆盖——目录级写面,不产 Op 不改 IR)
        "project_package" => return crate::pkg_tools::project_package_tool(&ws_root, args),
        "project_unpackage" => return crate::pkg_tools::project_unpackage_tool(&ws_root, args),
        _ => {}
    }

    // 册七 T7.5:AI 改动预演/批准应用——plan 面自管工作区:预演在副本工程上全链
    // dry-run(不落盘不产真 Op),应用逐项重入本单表(causedBy 链关联 planId)。
    match name {
        "preview_plan" => return crate::ai_ops::preview_plan_tool(&ws_root, args, actor),
        "apply_plan" => return crate::ai_ops::apply_plan_tool(root_str, args, actor),
        _ => {}
    }

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
        dispatch_on_ws(name, args, actor, root_str, &ws_root, ws)
    })
}

/// 工作区面派发体(stdio/HTTP/脚本宿主与 preview_plan 副本预演共用):单一实现
/// 纪律——预演不建第二套业务逻辑,只是在副本工程上重入本函数(T1.1 风格拆分自
/// dispatch_with_actor 闭包,行为零变化)。
pub(crate) fn dispatch_on_ws(
    name: &str,
    args: &Value,
    actor: Actor,
    root_str: &str,
    ws_root: &Path,
    ws: &mut cutforge_io::Workspace,
) -> Value {
    let opts = ApplyOpts {
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
    };

    // 常驻同步守护(M9-2)与锁纪律在外层 dispatch_with_actor(只读零锁/写全程锁);
    // 本函数只承载工作区面上的工具分支。
    match name {
        // ---------- 只读查询(E6-3:只读打开,不持排他锁) ----------
        "project_get" => match ws.engine().query(Query::ProjectView) {
            Answer::Project(v) => envelope(
                true,
                "OK",
                "工程视图",
                json!({"project": v, "rev": ws.rev()}),
            ),
            _ => unreachable!(),
        },
        "wordline_get" => read_truth(
            &ws_root.join(
                paths::truth_rel_on_disk(ws_root, "wordline.json").unwrap_or(paths::WORDLINE_REL),
            ),
            "wordline",
        ),
        "cutlist_get" => {
            let applied = args["applied"].as_bool().unwrap_or(false);
            let name = if applied {
                "cutlist.applied.json"
            } else {
                "cutlist.json"
            };
            let rel = paths::truth_rel_on_disk(ws_root, name).unwrap_or(paths::CUTLIST_REL);
            read_truth(&ws_root.join(rel), "cutlist")
        }
        "notes_list" => {
            let state = args["state"]
                .as_str()
                .and_then(|s| serde_json::from_str(&format!("\"{s}\"")).ok());
            let author = args["author"].as_str().map(|s| match s {
                "agent" => cutforge_core::notes::NoteAuthor::Agent,
                _ => cutforge_core::notes::NoteAuthor::User,
            });
            let items = ws.notes().filter(state, author);
            envelope(
                true,
                "OK",
                "标注清单",
                json!({
                    "notes": items, "total": ws.notes().notes().len(),
                    "orphans": ws.notes().orphans().len(),
                }),
            )
        }
        "oplog_tail" => match ws.engine().query(Query::OpLogTail {
            since_rev: args["sinceRev"].as_u64(),
            actor_kind: args["actor"].as_str().map(|s| match s {
                "user" => ActorKind::User,
                "script" => ActorKind::Script,
                "plugin" => ActorKind::Plugin,
                _ => ActorKind::Agent,
            }),
        }) {
            Answer::Ops(ops) => {
                let limit = args["limit"].as_u64().unwrap_or(50) as usize;
                let slice: Vec<_> = ops.iter().rev().take(limit).rev().cloned().collect();
                envelope(
                    true,
                    "OK",
                    "OpLog tail",
                    json!({"ops": slice, "count": ops.len(), "rev": ws.rev()}),
                )
            }
            _ => unreachable!(),
        },
        "conflict_list" => match ws.conflict_list() {
            Ok(items) => {
                let rows: Vec<Value> = items
                    .iter()
                    .map(|(id, c)| json!({"conflictId": id, "code": c.code.code(), "pointer": c.pointer}))
                    .collect();
                envelope(true, "OK", "冲突清单", json!({"conflicts": rows}))
            }
            Err(e) => envelope(false, "INTERNAL", &e.to_string(), json!({})),
        },
        "timeline_get" => {
            // E2-2:投影由服务端算好下放(endMs 等);壳零时间线语义(kf 采样同此)
            envelope(
                true,
                "OK",
                "时间线投影",
                json!({
                    "clips": crate::edit_ops::timeline_projection(ws.project()),
                    "rev": ws.rev(),
                }),
            )
        }

        // ---------- 写操作(全部经 Workspace 命令通道) ----------
        "clip_add" => {
            // E3-1/E3-2:走既有 Command::ClipInsert;durationMs 缺省由 probe 自动填。
            let (Some(track_id), Some(src), Some(start_ms)) = (
                args["trackId"].as_str(),
                args["src"].as_str(),
                args["startMs"].as_u64(),
            ) else {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    "缺 trackId/src/startMs",
                    json!({}),
                );
            };
            if ws.project().find_track(track_id).is_none() {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    &format!("track 不存在: {track_id}"),
                    json!({}),
                );
            }
            // 素材路径复用 /media 的 canonicalize 校验(不建并行实现;E3 风险面对策)
            if let Err(msg) = resolve_within_root(ws_root, src) {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    &format!("素材路径不合法({src}): {msg}"),
                    json!({}),
                );
            }
            let source_in = args["sourceInMs"].as_u64().unwrap_or(0);
            let duration_ms = match args["durationMs"].as_u64() {
                Some(d) => d,
                None => {
                    if !cutforge_io::probe::ffprobe_available() {
                        return envelope(
                            false,
                            "DEP_MISSING",
                            "durationMs 缺省且 ffprobe 不可用:显式给 durationMs,或安装 ffprobe / 设 CUTFORGE_FFPROBE",
                            json!({}),
                        );
                    }
                    match cutforge_io::probe::probe(&ws_root.join(src)) {
                        Ok(info) => (info.duration_ms().saturating_sub(source_in)).max(1),
                        Err(e) => {
                            return envelope(
                                false,
                                "DEP_MISSING",
                                &format!("媒体时长探测失败: {e}"),
                                json!({}),
                            );
                        }
                    }
                }
            };
            let clip_id = next_clip_id_for(ws.project(), track_id);
            let mut clip_json = json!({
                "id": clip_id, "src": src, "startMs": start_ms,
                "durationMs": duration_ms, "sourceInMs": source_in,
            });
            if let Some(v) = args["volume"].as_f64() {
                clip_json["volume"] = json!(v);
            }
            let clip: cutforge_core::model::Clip = match serde_json::from_value(clip_json) {
                Ok(c) => c,
                Err(e) => return envelope(false, "SCHEMA_INVALID", &e.to_string(), json!({})),
            };
            let request_id = args["requestId"].as_str().map(String::from);
            finish_apply(ws.apply(
                Command::ClipInsert {
                    to_track: track_id.into(),
                    clip,
                    request_id,
                },
                actor,
                opts,
            ))
        }
        "clip_update" => {
            let Some(clip_id) = args["clipId"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({}));
            };
            let p = &args["patch"];
            // 册四 T4.4/T4.9:速度曲线/倒放/旋转/裁剪/翻转(与 ClipPatch 字段面同
            // commit;speedCurve/crop 整组替换,越界由 schema SCHEMA_INVALID 拒)
            let speed_curve = p["speedCurve"].as_array().map(|arr| {
                arr.iter()
                    .filter_map(|pt| {
                        Some(cutforge_core::model::SpeedPoint {
                            at_ms: pt["atMs"].as_u64()?,
                            speed: pt["speed"].as_f64()?,
                        })
                    })
                    .collect::<Vec<_>>()
            });
            let crop = p["crop"].as_object().map(|_| cutforge_core::model::Crop {
                x: p["crop"]["x"].as_u64().unwrap_or(0),
                y: p["crop"]["y"].as_u64().unwrap_or(0),
                w: p["crop"]["w"].as_u64().unwrap_or(0),
                h: p["crop"]["h"].as_u64().unwrap_or(0),
            });
            // 花字(册四 T4.7 收口):null/{} = 清除(huazi_clear,undo 可还原);
            // 非空 = 整替换;非法结构显式拒绝(零幻觉面);缺席 = 不改。
            let (huazi, huazi_clear) = match p.get("huazi") {
                None => (None, false),
                Some(Value::Null) => (None, true),
                Some(v) if v.is_object() => {
                    if v.as_object().is_some_and(|o| o.is_empty()) {
                        (None, true)
                    } else {
                        match serde_json::from_value::<cutforge_core::text_style::Huazi>(v.clone())
                        {
                            Ok(h) => (Some(h), false),
                            Err(e) => {
                                return envelope(
                                    false,
                                    "SCHEMA_INVALID",
                                    &format!("patch.huazi 非法: {e}"),
                                    json!({}),
                                );
                            }
                        }
                    }
                }
                Some(_) => {
                    return envelope(false, "SCHEMA_INVALID", "patch.huazi 必须是对象", json!({}));
                }
            };
            let patch = ClipPatch {
                start_ms: p["startMs"].as_u64(),
                duration_ms: p["durationMs"].as_u64(),
                source_in_ms: p["sourceInMs"].as_u64(),
                speed: p["speed"].as_f64(),
                speed_curve,
                reverse: p["reverse"].as_bool(),
                rotation: p["rotation"].as_f64(),
                crop,
                flip: p["flip"].as_str().map(String::from),
                volume: p["volume"].as_f64(),
                denoise: p["denoise"].as_str().map(String::from),
                pitch: p["pitch"].as_f64(),
                opacity: p["opacity"].as_f64(),
                scale: p["scale"].as_f64(),
                text: p["text"].as_str().map(String::from),
                freeze_ms: p["freezeMs"].as_u64(),
                // 嵌套子 patch:对象缺席/显式 null 均为"不改";给出则按字段合并
                transition: p.get("transition").filter(|t| t.is_object()).map(|t| {
                    TransitionPatch {
                        type_: t["type"].as_str().map(String::from),
                        dur_ms: t["durMs"].as_f64(),
                        reason: t["reason"].as_str().map(String::from),
                        fx: t["fx"].as_str().map(String::from),
                    }
                }),
                motion: p
                    .get("motion")
                    .filter(|t| t.is_object())
                    .map(|t| MotionPatch {
                        in_: t["in"].as_str().map(String::from),
                        in_ms: t["inMs"].as_f64(),
                        out: t["out"].as_str().map(String::from),
                        out_ms: t["outMs"].as_f64(),
                        in_fx: t["inFx"].as_str().map(String::from),
                        out_fx: t["outFx"].as_str().map(String::from),
                    }),
                // 片段特效(册四 T4.6):整对象替换(combo 上限 3 由 schema 层界)
                fx: p.get("fx").filter(|t| t.is_object()).and_then(|f| {
                    serde_json::from_value::<cutforge_core::model::FxSpec>(f.clone())
                        .map_err(|_| ())
                        .ok()
                }),
                // 关键帧(IR v3):整组替换;非法显式拒绝(与 textStyle/huazi 同口径)
                keyframes: match p.get("keyframes") {
                    None | Some(Value::Null) => None,
                    Some(v) if v.is_array() => match serde_json::from_value(v.clone()) {
                        Ok(kf) => Some(kf),
                        Err(e) => {
                            return envelope(
                                false,
                                "SCHEMA_INVALID",
                                &format!("patch.keyframes 非法: {e}"),
                                json!({}),
                            );
                        }
                    },
                    _ => {
                        return envelope(
                            false,
                            "SCHEMA_INVALID",
                            "patch.keyframes 必须是数组",
                            json!({}),
                        );
                    }
                },
                // 片段调色(册五 T5.2):整对象替换;null/{} = 清除(同 huazi 模式)
                grade: match p.get("grade") {
                    None | Some(Value::Null) => None,
                    Some(v) if v.is_object() => {
                        if v.as_object().is_some_and(|o| o.is_empty()) {
                            None
                        } else {
                            match serde_json::from_value::<cutforge_core::model::Grade>(v.clone()) {
                                Ok(g) => Some(g),
                                Err(e) => {
                                    return envelope(
                                        false,
                                        "SCHEMA_INVALID",
                                        &format!("patch.grade 非法: {e}"),
                                        json!({}),
                                    );
                                }
                            }
                        }
                    }
                    Some(_) => {
                        return envelope(
                            false,
                            "SCHEMA_INVALID",
                            "patch.grade 必须是对象",
                            json!({}),
                        );
                    }
                },
                grade_clear: matches!(p.get("grade"), Some(Value::Null))
                    || p.get("grade")
                        .and_then(|v| v.as_object())
                        .is_some_and(|o| o.is_empty()),
                // 复合片段(T5.4):整对象替换;缺席不改;显式 null 拒绝(摘除走 unbind)
                compound: match p.get("compound") {
                    Some(Value::Null) => {
                        return envelope(
                            false,
                            "SCHEMA_INVALID",
                            "patch.compound = null 拒绝(摘除走 compound_unbind)",
                            json!({}),
                        );
                    }
                    None => None,
                    Some(v) if v.is_object() => {
                        match serde_json::from_value::<cutforge_core::model::CompoundSpec>(
                            v.clone(),
                        ) {
                            Ok(c) => Some(c),
                            Err(e) => {
                                return envelope(
                                    false,
                                    "SCHEMA_INVALID",
                                    &format!("patch.compound 非法: {e}(摘除走 compound_unbind)"),
                                    json!({}),
                                );
                            }
                        }
                    }
                    _ => {
                        return envelope(
                            false,
                            "SCHEMA_INVALID",
                            "patch.compound 必须是对象(摘除走 compound_unbind)",
                            json!({}),
                        );
                    }
                },
                // 文本样式(册四 T4.7):整对象替换;静默丢弃是幻觉面,非法显式拒绝。
                text_style: match p.get("textStyle") {
                    None | Some(Value::Null) => None,
                    Some(v) if v.is_object() => match serde_json::from_value(v.clone()) {
                        Ok(ts) => Some(ts),
                        Err(e) => {
                            return envelope(
                                false,
                                "SCHEMA_INVALID",
                                &format!("patch.textStyle 非法: {e}"),
                                json!({}),
                            );
                        }
                    },
                    Some(_) => {
                        return envelope(
                            false,
                            "SCHEMA_INVALID",
                            "patch.textStyle 必须是对象",
                            json!({}),
                        );
                    }
                },
                // 花字语义已在上方 (huazi, huazi_clear) 解出:null/{} = 清除,非空对象 = 整替换
                huazi,
                huazi_clear,
            };
            finish_apply(ws.apply(
                Command::ClipUpdate {
                    clip_id: clip_id.into(),
                    patch,
                },
                actor,
                opts,
            ))
        }
        "transition_set" => {
            // 设置片段转场(clip.transition):type 必给(durMs/fx/reason 可选,按字段合并);
            // 显式硬切/关闭用 type="cut"/"none"(schema 语义),枚举外值由 schema 层拒。
            // 册四 T4.5:全量转场目录经 fx="tr.<id>"(或裸 id)直通,目录经 GET /catalogs;
            // 未注册 fxId 渲染端降级 type(缺省 fade)并 WARN。
            let Some(clip_id) = args["clipId"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({}));
            };
            let Some(t) = args["type"].as_str() else {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    "缺 type(基础枚举 fade/wipeleft/wipeup/slideleft/circleopen/cut/none;全量 58 项目录走 fx=tr.<id>,GET /catalogs)",
                    json!({}),
                );
            };
            let patch = ClipPatch {
                transition: Some(TransitionPatch {
                    type_: Some(t.into()),
                    dur_ms: args["durMs"].as_f64(),
                    reason: args["reason"].as_str().map(String::from),
                    fx: args["fx"].as_str().map(String::from),
                }),
                ..Default::default()
            };
            finish_apply(ws.apply(
                Command::ClipUpdate {
                    clip_id: clip_id.into(),
                    patch,
                },
                actor,
                opts,
            ))
        }
        "motion_set" => {
            // 设置片段入场/出场动效(clip.motion):至少给 in/inMs/out/outMs/inFx/outFx 之一,
            // 按字段合并;册四 T4.6 枚举扩至真实渲染目录(fx-catalog motion.*),
            // inFx/outFx = mo.<id> 直通别名(优先于枚举,未注册降级并 WARN)。
            let Some(clip_id) = args["clipId"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({}));
            };
            let motion = MotionPatch {
                in_: args["in"].as_str().map(String::from),
                in_ms: args["inMs"].as_f64(),
                out: args["out"].as_str().map(String::from),
                out_ms: args["outMs"].as_f64(),
                in_fx: args["inFx"].as_str().map(String::from),
                out_fx: args["outFx"].as_str().map(String::from),
            };
            if motion.is_empty() {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    "motion_set 至少给 in/inMs/out/outMs/inFx/outFx 之一",
                    json!({}),
                );
            }
            let patch = ClipPatch {
                motion: Some(motion),
                ..Default::default()
            };
            finish_apply(ws.apply(
                Command::ClipUpdate {
                    clip_id: clip_id.into(),
                    patch,
                },
                actor,
                opts,
            ))
        }
        "bgm_set" => {
            // 工程级背景乐(doc.bgm;不经 ClipPatch):src 为字符串=设置/合并;
            // src 显式 null=清除;src 缺省=仅调 gainDb/ducking/loop(工程尚无 bgm 时须先给 src)。
            if args.get("src").is_some_and(Value::is_null) {
                return finish_apply(ws.apply(Command::BgmClear, actor, opts));
            }
            let patch = BgmPatch {
                src: args["src"].as_str().map(String::from),
                gain_db: args["gainDb"].as_f64(),
                ducking: args["ducking"].as_bool(),
                loop_: args["loop"].as_bool(),
                // ducking 侧链参数(册五 T5.3):缺省 = 既有常量,不给即行为零变化
                duck_threshold: args["duckThreshold"].as_f64(),
                duck_ratio: args["duckRatio"].as_f64(),
                duck_attack_ms: args["duckAttackMs"].as_f64(),
                duck_release_ms: args["duckReleaseMs"].as_f64(),
            };
            if patch.is_empty() {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    "bgm_set 至少给 src/gainDb/ducking/loop/duck* 之一(清除背景乐用 src:null)",
                    json!({}),
                );
            }
            // 音源路径与 clip_add 同一校验(不建并行实现)
            if let Some(src) = patch.src.as_deref()
                && let Err(msg) = resolve_within_root(ws_root, src)
            {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    &format!("bgm 路径不合法({src}): {msg}"),
                    json!({}),
                );
            }
            finish_apply(ws.apply(Command::BgmSet { patch }, actor, opts))
        }
        "clip_split" => match args["clipId"].as_str().zip(args["tMs"].as_u64()) {
            Some((clip_id, t_ms)) => finish_apply(ws.apply(
                Command::ClipSplit {
                    clip_id: clip_id.into(),
                    t_ms,
                },
                actor,
                opts,
            )),
            None => envelope(false, "PRECONDITION_FAILED", "缺 clipId/tMs", json!({})),
        },
        "clip_delete" => match args["clipId"].as_str() {
            Some(clip_id) => finish_apply(ws.apply(
                Command::ClipDelete {
                    clip_id: clip_id.into(),
                },
                actor,
                opts,
            )),
            None => envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({})),
        },
        "clip_move" => {
            let (Some(clip_id), Some(start_ms)) =
                (args["clipId"].as_str(), args["startMs"].as_u64())
            else {
                return envelope(false, "PRECONDITION_FAILED", "缺 clipId/startMs", json!({}));
            };
            finish_apply(ws.apply(
                Command::ClipMove {
                    clip_id: clip_id.into(),
                    new_start_ms: start_ms,
                    to_track: args["toTrack"].as_str().map(String::from),
                },
                actor,
                opts,
            ))
        }
        "clip_duplicate" => {
            let (Some(clip_id), Some(start_ms)) =
                (args["clipId"].as_str(), args["startMs"].as_u64())
            else {
                return envelope(false, "PRECONDITION_FAILED", "缺 clipId/startMs", json!({}));
            };
            let src_track = ws.project().find_clip(clip_id).map(|(ti, _)| ti);
            let Some(src_ti) = src_track else {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    &format!("clip 不存在: {clip_id}"),
                    json!({}),
                );
            };
            let to_track = args["toTrack"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| ws.project().tracks[src_ti].id.clone());
            let Some(ti) = ws.project().find_track(&to_track) else {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    &format!("track 不存在: {to_track}"),
                    json!({}),
                );
            };
            let mut clip = ws.project().tracks[src_ti].clips
                [ws.project().find_clip(clip_id).unwrap().1]
                .clone();
            if ws.project().tracks[ti].kind != ws.project().tracks[src_ti].kind {
                return envelope(false, "GUARD_FAILED", "跨 kind 复制拒绝", json!({}));
            }
            clip.id = cutforge_core::model::Project::next_clip_id(&ws.project().tracks[ti]);
            clip.start_ms = start_ms;
            let request_id = args["requestId"].as_str().map(String::from);
            finish_apply(ws.apply(
                Command::ClipInsert {
                    to_track,
                    clip,
                    request_id,
                },
                actor,
                opts,
            ))
        }
        "track_add" => {
            let Some(kind) = args["kind"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 kind", json!({}));
            };
            let kind = match kind {
                "video" => cutforge_core::model::TrackKind::Video,
                "audio" => cutforge_core::model::TrackKind::Audio,
                "text" => cutforge_core::model::TrackKind::Text,
                other => {
                    return envelope(
                        false,
                        "PRECONDITION_FAILED",
                        &format!("未知 kind: {other}"),
                        json!({}),
                    );
                }
            };
            let request_id = args["requestId"].as_str().map(String::from);
            finish_apply(ws.apply(Command::TrackAdd { kind, request_id }, actor, opts))
        }
        "subtitle_set" | "subtitle_retime" => {
            let Some(clip_id) = args["clipId"].as_str() else {
                return envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({}));
            };
            let patch = if name == "subtitle_set" {
                ClipPatch {
                    text: args["text"].as_str().map(String::from),
                    ..Default::default()
                }
            } else {
                ClipPatch {
                    start_ms: args["startMs"].as_u64(),
                    duration_ms: args["durationMs"].as_u64(),
                    ..Default::default()
                }
            };
            finish_apply(ws.apply(
                Command::ClipUpdate {
                    clip_id: clip_id.into(),
                    patch,
                },
                actor,
                opts,
            ))
        }
        "overlay_add" => {
            let (Some(track_id), Some(src), Some(at_ms), Some(dur)) = (
                args["trackId"].as_str(),
                args["element"]["src"].as_str(),
                args["element"]["atMs"].as_u64(),
                args["element"]["durationMs"].as_u64(),
            ) else {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    "缺 trackId/element.src/atMs/durationMs",
                    json!({}),
                );
            };
            let clip_id = next_clip_id_for(ws.project(), track_id);
            let clip_json = json!({
                "id": clip_id, "src": src, "startMs": at_ms, "durationMs": dur,
                "volume": 0, "overlay": args["element"]["overlay"],
            });
            let clip: cutforge_core::model::Clip = match serde_json::from_value(clip_json) {
                Ok(c) => c,
                Err(e) => return envelope(false, "SCHEMA_INVALID", &e.to_string(), json!({})),
            };
            let request_id = args["requestId"].as_str().map(String::from);
            finish_apply(ws.apply(
                Command::ClipInsert {
                    to_track: track_id.into(),
                    clip,
                    request_id,
                },
                actor,
                opts,
            ))
        }
        "sfx_add" => {
            let (Some(t_ms), Some(src)) = (args["tMs"].as_u64(), args["src"].as_str()) else {
                return envelope(false, "PRECONDITION_FAILED", "缺 tMs/src", json!({}));
            };
            // 密度护栏:±15s 内已有 ≥2 个 sfx → GUARD_FAILED(计划书 5.2)
            let sfx_nearby = crate::edit_ops::count_sfx_near(ws.project(), t_ms, 15_000);
            if sfx_nearby >= 2 {
                return envelope(
                    false,
                    "GUARD_FAILED",
                    &format!("音效密度超限:{t_ms}ms ±15s 内已有 {sfx_nearby} 个"),
                    json!({}),
                );
            }
            let track_id = ws
                .project()
                .tracks
                .iter()
                .find(|t| t.kind == cutforge_core::model::TrackKind::Audio)
                .map(|t| t.id.clone())
                .unwrap_or_else(|| "A1".into());
            let clip_id = next_clip_id_for(ws.project(), &track_id);
            let clip_json = json!({
                "id": clip_id, "src": src, "startMs": t_ms, "durationMs": 400,
                "role": "sfx", "volume": args["volume"].as_f64().unwrap_or(0.8),
            });
            let clip: cutforge_core::model::Clip = match serde_json::from_value(clip_json) {
                Ok(c) => c,
                Err(e) => return envelope(false, "SCHEMA_INVALID", &e.to_string(), json!({})),
            };
            let request_id = args["requestId"].as_str().map(String::from);
            finish_apply(ws.apply(
                Command::ClipInsert {
                    to_track: track_id,
                    clip,
                    request_id,
                },
                actor,
                opts,
            ))
        }
        "notes_add" => {
            let (Some(anchor_v), Some(body)) = (args["anchor"].as_object(), args["body"].as_str())
            else {
                return envelope(false, "PRECONDITION_FAILED", "缺 anchor/body", json!({}));
            };
            let kind = match anchor_v
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("clip")
            {
                "track" => AnchorKind::Track,
                "time" => AnchorKind::Time,
                "word" => AnchorKind::Word,
                "subtitleCard" => AnchorKind::SubtitleCard,
                _ => AnchorKind::Clip,
            };
            let anchor = Anchor {
                kind,
                // 注意:serde Map 的 Index 在键缺失时 panic,必须用 .get()(M10 e2e 实测)
                ref_: anchor_v
                    .get("ref")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                t_ms: anchor_v.get("tMs").and_then(|v| v.as_u64()).unwrap_or(0),
                span: None,
            };
            let author = if args["author"].as_str() == Some("agent") {
                cutforge_core::notes::NoteAuthor::Agent
            } else {
                cutforge_core::notes::NoteAuthor::User
            };
            let tags = args["tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            match ws.notes_add(
                anchor,
                body.into(),
                author,
                tags,
                actor,
                args["requestId"].as_str().map(String::from),
            ) {
                Ok(id) => envelope(
                    true,
                    "OK",
                    "标注已创建",
                    json!({"noteId": id, "rev": ws.rev()}),
                ),
                Err(e) => envelope(false, "INTERNAL", &e.to_string(), json!({})),
            }
        }
        "notes_resolve" => {
            let (Some(note_id), Some(reply)) = (args["noteId"].as_str(), args["reply"].as_str())
            else {
                return envelope(false, "PRECONDITION_FAILED", "缺 noteId/reply", json!({}));
            };
            let op_ids = args["opIds"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            match ws.notes_resolve(note_id, reply.into(), op_ids, actor) {
                Ok(()) => envelope(true, "OK", "标注已结案", json!({"noteId": note_id})),
                Err(e) => notes_op_error(e),
            }
        }
        "notes_reject" => {
            let (Some(note_id), Some(reason)) = (args["noteId"].as_str(), args["reason"].as_str())
            else {
                return envelope(false, "PRECONDITION_FAILED", "缺 noteId/reason", json!({}));
            };
            match ws.notes_reject(note_id, reason.into(), actor) {
                Ok(()) => envelope(true, "OK", "标注已否决", json!({"noteId": note_id})),
                Err(e) => notes_op_error(e),
            }
        }
        // 册七 T7.5:标注线程化(同一标注多轮追加;线程 id = 标注 id)
        "note_reply" => crate::ai_ops::note_reply_tool(ws, args, &actor),
        // 册七 T7.5:会话报告(改动摘要,Markdown+JSON 双形态)
        "session_report" => crate::ai_ops::session_report_tool(ws, args),
        "cut_apply" => {
            let patch = args["patch"].as_object().cloned();
            let Some(patch) = patch else {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    "缺 patch(merge-patch 对象)",
                    json!({}),
                );
            };
            apply_cut_merge_patch(ws, &Value::Object(patch))
        }
        "undo" | "redo" => {
            let batch = args["batch"].as_u64().unwrap_or(1);
            let mut last = envelope(true, "OK", "无操作", json!({}));
            for _ in 0..batch {
                last = if name == "undo" {
                    finish_apply(ws.undo(actor.clone()))
                } else {
                    finish_apply(ws.redo(actor.clone()))
                };
                if last["ok"] != json!(true) {
                    break;
                }
            }
            last
        }

        // ---------- 时间线编辑全工具(册四 A4 T4.2;实现集中在 edit_ops) ----------
        "clip_trim" => edit_ops::clip_trim_tool(ws, args, &actor, opts),
        "clip_split_all" => edit_ops::clip_split_all_tool(ws, args, &actor, opts),
        "track_update" => edit_ops::track_update_tool(ws, args, &actor, opts),
        "clip_gap_delete" => edit_ops::clip_gap_delete_tool(ws, args, &actor, opts),
        "clip_copy" => edit_ops::clip_copy_tool(ws, root_str, args),
        "clip_paste_at" => edit_ops::clip_paste_at_tool(ws, root_str, args, &actor, opts),

        // ---------- 专业编辑工具(册五 T5.4;实现集中在 pro_ops) ----------
        "compound_create" => crate::pro_ops::compound_create_tool(ws, args, &actor, opts),
        "compound_unbind" => crate::pro_ops::compound_unbind_tool(ws, args, &actor, opts),
        "multicam_cut" => crate::pro_ops::multicam_cut_tool(ws, args, &actor, opts),
        "scene_detect" => crate::pro_ops::scene_detect_tool(ws, ws_root, args, &actor, opts),
        // 互操作导出(册五 T5.5;只读工程 + 派生物落盘,与 subtitle_export 同类)
        "otio_export" => crate::pro_ops::otio_export_tool(ws, ws_root, args),

        // ---------- 文本/字幕(册四 A4 T4.7;实现集中在 subtitle_ops) ----------
        "text_add" => subtitle_ops::text_add_tool(ws, args, &actor, opts),
        "subtitle_import" => subtitle_ops::subtitle_import_tool(ws, ws_root, args, &actor, opts),
        "subtitle_export" => subtitle_ops::subtitle_export_tool(ws, ws_root, args),
        "subtitle_replace" => subtitle_ops::subtitle_replace_tool(ws, args, &actor, opts),

        // ---------- 编排(封装 CutFlow 脚本,不重实现) ----------
        "stage_run" | "stage_rebuild" | "verify_run" | "sync_check" | "render"
        | "export_jianying" => {
            let script = match name {
                "stage_run" => "rs_run.py",
                "stage_rebuild" => "rebuild.py",
                "verify_run" => "rs_verify.py",
                "sync_check" => "rs_sync.py",
                "render" => "rs_render.py",
                _ => "rs_jy_draft.py",
            };
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

        other => envelope(
            false,
            "INTERNAL",
            &format!("工具已注册但未实现: {other}"),
            json!({}),
        ),
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
fn next_clip_id_for(project: &cutforge_core::model::Project, track_id: &str) -> String {
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
