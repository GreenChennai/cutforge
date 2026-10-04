// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 片段写入面处理器(全部经 Workspace 命令通道:Command → apply → Op → OpLog,
//! 护城河 #1)。A-01 自 dispatch.rs 巨 match 写操作段逐字迁移(行为零变化):
//! 缺参 PRECONDITION_FAILED / 契约 SCHEMA_INVALID / 守护 GUARD_FAILED 错误码
//! 与消息逐字保留。

use crate::dispatch::{finish_apply, next_clip_id_for, resolve_within_root};
use crate::handlers::{CommandHandler, HandlerCtx};
use crate::registry::envelope;
use cutforge_core::command::{ClipPatch, Command, MotionPatch, TransitionPatch};
use serde_json::{Value, json};

/// clip_add(E3-1/E3-2):走既有 Command::ClipInsert;durationMs 缺省由 probe 自动填;
/// 素材路径复用 /media 的 canonicalize 校验(不建并行实现)。
pub(crate) struct ClipAdd;

impl CommandHandler for ClipAdd {
    fn name(&self) -> &'static str {
        "clip_add"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        let ws_root = cx.ws_root;
        let ws = cx.ws();
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
}

/// clip_update(册四 T4.4-T4.9):速度曲线/倒放/旋转/裁剪/翻转/花字/关键帧/
/// 片段调色/复合片段/文本样式——嵌套对象整组替换,非法结构显式拒绝(零幻觉面)。
pub(crate) struct ClipUpdate;

impl CommandHandler for ClipUpdate {
    fn name(&self) -> &'static str {
        "clip_update"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
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
                    match serde_json::from_value::<cutforge_core::text_style::Huazi>(v.clone()) {
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
            transition: p
                .get("transition")
                .filter(|t| t.is_object())
                .map(|t| TransitionPatch {
                    type_: t["type"].as_str().map(String::from),
                    dur_ms: t["durMs"].as_f64(),
                    reason: t["reason"].as_str().map(String::from),
                    fx: t["fx"].as_str().map(String::from),
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
                    return envelope(false, "SCHEMA_INVALID", "patch.grade 必须是对象", json!({}));
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
                    match serde_json::from_value::<cutforge_core::model::CompoundSpec>(v.clone()) {
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
        finish_apply(cx.ws().apply(
            Command::ClipUpdate {
                clip_id: clip_id.into(),
                patch,
            },
            actor,
            opts,
        ))
    }
}

/// transition_set:设置片段转场(clip.transition):type 必给(durMs/fx/reason 可选,
/// 按字段合并);显式硬切/关闭用 type="cut"/"none"(schema 语义),枚举外值由 schema
/// 层拒。册四 T4.5:全量转场目录经 fx="tr.<id>"(或裸 id)直通。
pub(crate) struct TransitionSet;

impl CommandHandler for TransitionSet {
    fn name(&self) -> &'static str {
        "transition_set"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
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
        finish_apply(cx.ws().apply(
            Command::ClipUpdate {
                clip_id: clip_id.into(),
                patch,
            },
            actor,
            opts,
        ))
    }
}

/// motion_set:设置片段入场/出场动效(clip.motion):至少给 in/inMs/out/outMs/
/// inFx/outFx 之一,按字段合并;册四 T4.6 枚举扩至真实渲染目录(fx-catalog
/// motion.*),inFx/outFx = mo.<id> 直通别名(优先于枚举,未注册降级并 WARN)。
pub(crate) struct MotionSet;

impl CommandHandler for MotionSet {
    fn name(&self) -> &'static str {
        "motion_set"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
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
        finish_apply(cx.ws().apply(
            Command::ClipUpdate {
                clip_id: clip_id.into(),
                patch,
            },
            actor,
            opts,
        ))
    }
}

/// subtitle_set / subtitle_retime 共享体(文本替换 / 时间重排,均走 ClipUpdate)。
fn subtitle_patch_handler(name: &str, args: &Value, cx: &mut HandlerCtx) -> Value {
    let actor = cx.actor.clone();
    let opts = cx.opts.clone();
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
    finish_apply(cx.ws().apply(
        Command::ClipUpdate {
            clip_id: clip_id.into(),
            patch,
        },
        actor,
        opts,
    ))
}

/// subtitle_set:文本替换(册四 T4.7)。
pub(crate) struct SubtitleSet;

impl CommandHandler for SubtitleSet {
    fn name(&self) -> &'static str {
        "subtitle_set"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        subtitle_patch_handler(self.name(), args, cx)
    }
}

/// subtitle_retime:startMs/durationMs 重排。
pub(crate) struct SubtitleRetime;

impl CommandHandler for SubtitleRetime {
    fn name(&self) -> &'static str {
        "subtitle_retime"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        subtitle_patch_handler(self.name(), args, cx)
    }
}

/// overlay_add:画中画元素插入(trackId + element 面)。
pub(crate) struct OverlayAdd;

impl CommandHandler for OverlayAdd {
    fn name(&self) -> &'static str {
        "overlay_add"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        let ws = cx.ws();
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
}

/// sfx_add:音效插入(±15s 密度护栏 ≥2 → GUARD_FAILED,计划书 5.2)。
pub(crate) struct SfxAdd;

impl CommandHandler for SfxAdd {
    fn name(&self) -> &'static str {
        "sfx_add"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        let ws = cx.ws();
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
}

/// cut_apply:粗剪 merge-patch 应用(cutlist_ops 单一实现)。
pub(crate) struct CutApply;

impl CommandHandler for CutApply {
    fn name(&self) -> &'static str {
        "cut_apply"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let patch = args["patch"].as_object().cloned();
        let Some(patch) = patch else {
            return envelope(
                false,
                "PRECONDITION_FAILED",
                "缺 patch(merge-patch 对象)",
                json!({}),
            );
        };
        crate::cutlist_ops::apply_cut_merge_patch(cx.ws(), &Value::Object(patch))
    }
}

/// undo / redo 共享体(批量步进;首败即停,回执如实)。
fn undo_redo_handler(name: &str, args: &Value, cx: &mut HandlerCtx) -> Value {
    let actor = cx.actor.clone();
    let ws = cx.ws();
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

/// undo:撤销栈回退(OpLog 重建语义,护城河 #6)。
pub(crate) struct Undo;

impl CommandHandler for Undo {
    fn name(&self) -> &'static str {
        "undo"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        undo_redo_handler(self.name(), args, cx)
    }
}

/// redo:重做栈推进。
pub(crate) struct Redo;

impl CommandHandler for Redo {
    fn name(&self) -> &'static str {
        "redo"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        undo_redo_handler(self.name(), args, cx)
    }
}
