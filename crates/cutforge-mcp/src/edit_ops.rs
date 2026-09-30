// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 册四 A4 T4.2:时间线编辑全工具的服务端实现。
//!
//! 语义裁决全部在内核 Command(经 Workspace::apply 唯一写入口,undo/redo/replay 天然支持):
//! - `clip_trim`(trim/roll/slip/slide)→ `Command::ClipTrim`(四模式语义见 command.rs);
//! - `clip_split_all` → `Command::ClipSplitAll`(单 Op 全轨分割);
//! - `track_update` → `Command::TrackUpdate`(TrackPatch 按字段合并);
//! - `clip_gap_delete` → `Command::ClipGapDelete`(单 Op 原子闭合间隙);
//! - `clip_copy` → 服务端会话态剪贴板(Workspace.clipboard_set;不产 Op 不升 rev);
//! - `clip_paste_at` → 剪贴板读出 + 新 id 分配 + 既有 `Command::ClipInsert`
//!   (与 clip_add/clip_duplicate 同一先例:派发层组合,不新增引擎逻辑)。
//!
//! 本模块只做参数抽取、素材约束探测与剪贴板会话态,禁止旁路写工程文件。

use crate::registry::envelope;
use cutforge_core::command::{Command, TrackPatch, TrimEdge, TrimMode};
use cutforge_core::engine::ApplyOpts;
use cutforge_core::model::TrackKind;
use cutforge_core::oplog::Actor;
use cutforge_io::Workspace;
use serde_json::{json, Value};

/// clip_trim(mode: trim|roll|slip|slide):deltaMs 有符号毫秒;trim/roll 必给 edge(in|out)。
/// slip 的素材时长约束(sourceIn+duration ≤ 素材长)在此以 ffprobe 承接
/// (内核无媒体知识;与 clip_add 的 durationMs 探测同一依赖口径)。
pub fn clip_trim_tool(ws: &mut Workspace, args: &Value, actor: &Actor, opts: ApplyOpts) -> Value {
    let Some(clip_id) = args["clipId"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({}));
    };
    let Some(mode_s) = args["mode"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 mode(trim|roll|slip|slide)", json!({}));
    };
    let mode = match mode_s {
        "trim" => TrimMode::Trim,
        "roll" => TrimMode::Roll,
        "slip" => TrimMode::Slip,
        "slide" => TrimMode::Slide,
        other => {
            return envelope(false, "PRECONDITION_FAILED",
                &format!("未知 mode: {other}(允许 trim|roll|slip|slide)"), json!({}))
        }
    };
    let edge = match (args["edge"].as_str(), mode) {
        (Some("in"), _) => Some(TrimEdge::In),
        (Some("out"), _) => Some(TrimEdge::Out),
        (Some(other), _) => {
            return envelope(false, "PRECONDITION_FAILED",
                &format!("未知 edge: {other}(允许 in|out;slip/slide 无需 edge)"), json!({}))
        }
        (None, TrimMode::Trim | TrimMode::Roll) => {
            return envelope(false, "PRECONDITION_FAILED", "trim/roll 模式必须给 edge(in|out)", json!({}))
        }
        (None, _) => None,
    };
    let Some(delta) = args["deltaMs"].as_i64() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 deltaMs(有符号毫秒数)", json!({}));
    };
    if delta == 0 {
        return envelope(false, "PRECONDITION_FAILED", "deltaMs 为 0 无变更,拒绝空操作", json!({}));
    }
    // slip 正向平移的素材末尾约束:sourceIn_new + duration ≤ 素材时长
    if mode == TrimMode::Slip && delta > 0 {
        let (src, dur, source_in) = {
            let p = ws.project();
            match p.find_clip(clip_id) {
                Some((ti, ci)) => {
                    let c = &p.tracks[ti].clips[ci];
                    (c.src.clone(), c.duration_ms, c.source_in_ms.unwrap_or(0))
                }
                None => {
                    return envelope(false, "PRECONDITION_FAILED",
                        &format!("clip 不存在: {clip_id}"), json!({}))
                }
            }
        };
        if let Some(src) = src {
            if !cutforge_io::probe::ffprobe_available() {
                return envelope(false, "DEP_MISSING",
                    "slip 需素材时长约束校验,但 ffprobe 不可用:安装 ffprobe / 设 CUTFORGE_FFPROBE", json!({}));
            }
            match cutforge_io::probe::probe(&ws.root().join(&src)) {
                Ok(info) => {
                    let need = source_in as i128 + delta as i128 + dur as i128;
                    if need > info.duration_ms() as i128 {
                        return envelope(false, "GUARD_FAILED", &format!(
                            "slip 越出素材末尾: 需 sourceIn+duration ≤ {need}ms,素材({src})仅 {}ms",
                            info.duration_ms()), json!({}));
                    }
                }
                Err(e) => {
                    return envelope(false, "DEP_MISSING",
                        &format!("素材时长探测失败({src}): {e}"), json!({}))
                }
            }
        }
    }
    crate::dispatch::finish_apply(ws.apply(
        Command::ClipTrim {
            clip_id: clip_id.into(),
            mode,
            edge: edge.unwrap_or(TrimEdge::In),
            delta_ms: delta,
        },
        actor.clone(),
        opts,
    ))
}

/// clip_split_all:播放头处所有轨命中的片段一次全分割(单 Op;无命中幂等回执)。
pub fn clip_split_all_tool(ws: &mut Workspace, args: &Value, actor: &Actor, opts: ApplyOpts) -> Value {
    let Some(t_ms) = args["tMs"].as_u64() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 tMs(播放头毫秒)", json!({}));
    };
    crate::dispatch::finish_apply(ws.apply(Command::ClipSplitAll { t_ms }, actor.clone(), opts))
}

/// track_update:轨道属性 patch(TrackPatch 按字段合并,None=不改);
/// 静音/独奏的渲染混音联动候 BE3,本工具先保证契约链就位。
/// 册五 T5.3:eq(整组替换)/ dyn(整对象替换);显式 null = 清除。
pub fn track_update_tool(ws: &mut Workspace, args: &Value, actor: &Actor, opts: ApplyOpts) -> Value {
    let Some(track_id) = args["trackId"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 trackId", json!({}));
    };
    let p = &args["patch"];
    if !p.is_object() {
        return envelope(false, "PRECONDITION_FAILED",
            "缺 patch(对象:name/locked/mute/solo/hidden/heightPx/color/eq/dyn 按需给出)", json!({}));
    }
    // eq/dyn 结构非法显式拒绝(零幻觉面);null = 清除(eqClear/dynClear 承接)
    let mut parse_err: Option<String> = None;
    let eq = p.get("eq").and_then(|v| {
        if v.is_null() {
            return None;
        }
        serde_json::from_value::<Vec<cutforge_core::model::EqBand>>(v.clone())
            .map_err(|e| parse_err = Some(format!("patch.eq 非法: {e}")))
            .ok()
    });
    let dyn_ = p.get("dyn").and_then(|v| {
        if v.is_null() {
            return None;
        }
        serde_json::from_value::<cutforge_core::model::TrackDyn>(v.clone())
            .map_err(|e| parse_err = Some(format!("patch.dyn 非法: {e}")))
            .ok()
    });
    if let Some(msg) = parse_err {
        return envelope(false, "SCHEMA_INVALID", &msg, json!({}));
    }
    let patch = TrackPatch {
        name: p["name"].as_str().map(String::from),
        locked: p["locked"].as_bool(),
        mute: p["mute"].as_bool(),
        solo: p["solo"].as_bool(),
        hidden: p["hidden"].as_bool(),
        height_px: p["heightPx"].as_u64(),
        color: p["color"].as_str().map(String::from),
        eq,
        eq_clear: p.get("eq").map(Value::is_null).unwrap_or(false),
        dyn_,
        dyn_clear: p.get("dyn").map(Value::is_null).unwrap_or(false),
    };
    if patch.is_empty() {
        return envelope(false, "PRECONDITION_FAILED",
            "patch 至少给 name/locked/mute/solo/hidden/heightPx/color/eq/dyn 之一", json!({}));
    }
    crate::dispatch::finish_apply(ws.apply(Command::TrackUpdate { track_id: track_id.into(), patch }, actor.clone(), opts))
}

/// clip_gap_delete:删除指定轨上包含 tMs 的间隙,后继整体左移闭合(单 Op 原子)。
pub fn clip_gap_delete_tool(ws: &mut Workspace, args: &Value, actor: &Actor, opts: ApplyOpts) -> Value {
    let (Some(track_id), Some(t_ms)) = (args["trackId"].as_str(), args["tMs"].as_u64()) else {
        return envelope(false, "PRECONDITION_FAILED", "缺 trackId/tMs", json!({}));
    };
    crate::dispatch::finish_apply(ws.apply(
        Command::ClipGapDelete { track_id: track_id.into(), t_ms },
        actor.clone(),
        opts,
    ))
}

/// clip_copy:片段深拷贝入服务端会话态剪贴板(不产 Op、不升 rev、不可撤销)。
/// 剪贴板驻留在 resident 会话态(按 root 键控),跨失败调用与工作区重开存活。
pub fn clip_copy_tool(ws: &mut Workspace, root_key: &str, args: &Value) -> Value {
    let Some(clip_id) = args["clipId"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 clipId", json!({}));
    };
    let p = ws.project();
    let Some((ti, ci)) = p.find_clip(clip_id) else {
        return envelope(false, "PRECONDITION_FAILED", &format!("clip 不存在: {clip_id}"), json!({}));
    };
    let clip = p.tracks[ti].clips[ci].clone();
    let kind = p.tracks[ti].kind;
    crate::resident::clipboard_set(root_key, vec![clip], kind);
    envelope(true, "OK", "已复制到会话剪贴板", json!({
        "clipId": clip_id,
        "copied": 1,
        "clipboard": {"clips": 1, "kind": kind_str(kind)},
    }))
}

/// clip_paste_at:带属性粘贴剪贴板内容到指定轨+时间点。
/// 属性(volume/speed/transition/…)原样携带;id 在目标轨重分配;
/// 跨 kind 拒绝(GUARD_FAILED);落点重叠由内核 enforce_no_overlap 拒绝。
pub fn clip_paste_at_tool(ws: &mut Workspace, root_key: &str, args: &Value, actor: &Actor, opts: ApplyOpts) -> Value {
    let (Some(track_id), Some(start_ms)) = (args["trackId"].as_str(), args["startMs"].as_u64()) else {
        return envelope(false, "PRECONDITION_FAILED", "缺 trackId/startMs", json!({}));
    };
    let Some(ti) = ws.project().find_track(track_id) else {
        return envelope(false, "PRECONDITION_FAILED", &format!("track 不存在: {track_id}"), json!({}));
    };
    let Some(cb) = crate::resident::clipboard_get(root_key) else {
        return envelope(false, "PRECONDITION_FAILED", "剪贴板为空(先 clip_copy)", json!({}));
    };
    if ws.project().tracks[ti].kind != cb.kind {
        return envelope(false, "GUARD_FAILED", &format!(
            "跨 kind 粘贴拒绝: 剪贴板来源 {} vs 目标轨 {track_id}({})",
            kind_str(cb.kind), kind_str(ws.project().tracks[ti].kind)), json!({}));
    }
    // 当前形态:clip_copy 单片段入板;数组形态为多片段批量粘贴预留(候后续册)
    let mut clip = cb.clips[0].clone();
    clip.id = cutforge_core::model::Project::next_clip_id(&ws.project().tracks[ti]);
    clip.start_ms = start_ms;
    let request_id = args["requestId"].as_str().map(String::from);
    crate::dispatch::finish_apply(ws.apply(
        Command::ClipInsert { to_track: track_id.into(), clip, request_id },
        actor.clone(),
        opts,
    ))
}

fn kind_str(k: TrackKind) -> &'static str {
    match k {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
        TrackKind::Text => "text",
    }
}

/// 关键帧采样投影(IR v3,T5.1 求值单源纪律):per-property 采样点集
/// [{property, samples:[[tMs,v],…]}];网格 100ms ∪ 关键帧时刻,壳只画点零插值。
pub(crate) fn keyframe_samples_projection(clip: &cutforge_core::model::Clip) -> Value {
    use cutforge_core::keyframes::{sample_property, FIXED_PROPERTIES};
    let Some(kfs) = &clip.keyframes else { return json!([]) };
    let mut out: Vec<Value> = Vec::new();
    let mut props: Vec<&str> = FIXED_PROPERTIES.to_vec();
    // fx 键(按目录三态裁决后的可打点参数)动态并入
    for k in kfs {
        if k.property.starts_with("fx.")
            && !props.iter().any(|p| *p == k.property)
            && !out.iter().any(|e| e["property"] == json!(k.property))
        {
            out.push(json!({
                "property": k.property,
                "samples": sample_property(clip, &k.property, 100)
                    .into_iter().map(|(t, v)| json!([t, v])).collect::<Vec<_>>(),
            }));
        }
    }
    for p in props.drain(..) {
        if kfs.iter().any(|k| k.property == p) {
            out.push(json!({
                "property": p,
                "samples": sample_property(clip, p, 100)
                    .into_iter().map(|(t, v)| json!([t, v])).collect::<Vec<_>>(),
            }));
        }
    }
    Value::Array(out)
}

/// E2-2:时间线投影的逐 clip 全字段(endMs 在服务端算好;壳零时间线语义)。
pub(crate) fn timeline_projection(project: &cutforge_core::model::Project) -> Vec<Value> {
    let mut rows = Vec::new();
    for t in &project.tracks {
        for c in &t.clips {
            rows.push(json!({
                "id": c.id, "track": t.id,
                "trackKind": match t.kind {
                    cutforge_core::model::TrackKind::Video => "video",
                    cutforge_core::model::TrackKind::Audio => "audio",
                    cutforge_core::model::TrackKind::Text => "text",
                },
                "src": c.src, "startMs": c.start_ms, "endMs": c.start_ms + c.duration_ms,
                "durationMs": c.duration_ms, "sourceInMs": c.source_in_ms,
                "speed": c.speed, "volume": c.volume, "opacity": c.opacity,
                "scale": c.scale, "position": c.position, "overlay": c.overlay,
                "motion": c.motion, "text": c.text, "freezeMs": c.freeze_ms,
                "transition": c.transition,
                // 册四 A4 T4.4/T4.9:速度/时间与变换字段随投影下放(壳检查器/轨道展示消费;
                // 时长语义单一真相源 = cutforge_core::model::speed_segments,渲染同源)
                "speedCurve": c.speed_curve, "reverse": c.reverse,
                "rotation": c.rotation, "crop": c.crop, "flip": c.flip,
                // 册四 A4 T4.7/T4.8:文本样式/花字/降噪/变调随投影下放(壳检查器消费;
                // textStyle/huazi/denoise/pitch 均已入 ClipPatch,可编辑非只读)
                "textStyle": c.text_style, "huazi": c.huazi, "font": c.font,
                // 册四收口(候 BE 了断):特效栈随投影下放——壳侧 projector.js 的
                // withFxReadback 只读桥自此退化(投影含 fx 键后合并恒空操作)
                "fx": c.fx,
                "denoise": c.denoise, "pitch": c.pitch,
                // 关键帧(IR v3,T5.1):原始数组 + 采样点集(求值单源;壳零插值)
                "keyframes": c.keyframes, "keyframeSamples": keyframe_samples_projection(c),
                // E4-3 只读展示面:渲染已支持但 ClipPatch 未承接的分散字段,原样下放
                // (transition/motion 已于 ClipPatch 扩展后承接,不再列只读)
                "fade": c.fade,
                "punchIn": c.punch_in, "role": c.role,
            }));
        }
    }
    rows
}

pub(crate) fn count_sfx_near(project: &cutforge_core::model::Project, t_ms: u64, window: u64) -> usize {
    project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .filter(|c| c.role == Some(cutforge_core::model::Role::Sfx))
        .filter(|c| {
            let end = c.start_ms + c.duration_ms;
            c.start_ms <= t_ms + window && t_ms <= end + window
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 册四收口(候 BE 了断):投影必须含 fx 键——挂了特效的片段下发整对象,
    /// 未挂的下发 null(键恒在;壳侧 withFxReadback 桥据此自然退化为空操作)。
    #[test]
    fn timeline_projection_carries_fx_key_always() {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "proj-fx", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "startMs": 0, "durationMs": 2000,
                 "fx": {"combo": [{"fx": "fx.blur", "params": {"radius": 4}}]}},
                {"id": "V1-002", "startMs": 2000, "durationMs": 2000}
            ]}]
        })).expect("夹具必须过 v2 校验");
        let rows = timeline_projection(&project);
        assert_eq!(rows.len(), 2);
        let with = rows.iter().find(|r| r["id"] == json!("V1-001")).unwrap();
        let without = rows.iter().find(|r| r["id"] == json!("V1-002")).unwrap();
        assert_eq!(with["fx"]["combo"].as_array().unwrap().len(), 1, "挂特效必须整对象下放");
        assert_eq!(with["fx"]["combo"][0]["fx"], json!("fx.blur"));
        assert_eq!(without["fx"], json!(Value::Null), "未挂特效 fx 键必须为 null(键不可缺席)");
    }

    /// IR v3(T5.1):投影携带 keyframes 原始数组 + keyframeSamples 采样点集
    /// (求值单源;壳零插值);无关键帧 clip 两键恒在(null / 空数组)。
    #[test]
    fn timeline_projection_carries_keyframes_and_samples() {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "3.0.0", "slug": "proj-kf", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "startMs": 0, "durationMs": 2000, "keyframes": [
                    // interp 语义 = 本帧→下一帧区间 → 缓动标注打区间首帧
                    {"property": "position.x", "timeMs": 0, "value": 0.5, "interp": "easeIn"},
                    {"property": "position.x", "timeMs": 1000, "value": 0.7},
                    {"property": "opacity", "timeMs": 0, "value": 0.0},
                    {"property": "opacity", "timeMs": 800, "value": 1.0}
                ]},
                {"id": "V1-002", "startMs": 2000, "durationMs": 2000}
            ]}]
        })).expect("夹具必须过 v3 校验");
        let rows = timeline_projection(&project);
        let with = rows.iter().find(|r| r["id"] == json!("V1-001")).unwrap();
        let without = rows.iter().find(|r| r["id"] == json!("V1-002")).unwrap();
        assert_eq!(with["keyframes"].as_array().unwrap().len(), 4, "原始数组整组下放");
        let samples = with["keyframeSamples"].as_array().unwrap();
        assert_eq!(samples.len(), 2, "position.x 与 opacity 两组");
        let px = samples.iter().find(|s| s["property"] == json!("position.x")).unwrap();
        let sarr = px["samples"].as_array().unwrap();
        // 网格 100ms ∪ 关键帧时刻 0/1000;末点 = 时长 2000
        assert!(sarr.iter().any(|pt| pt[0] == json!(0)), "t=0 网格点必须在样本");
        assert!(sarr.iter().any(|pt| pt[0] == json!(1000)), "关键帧时刻入样本");
        assert_eq!(sarr.last().unwrap()[0], json!(2000), "末样本 = 片段时长");
        // 采样值 = 求值器同点输出(单源):2000ms 处端点外延 = 0.7
        let last = sarr.last().unwrap()[1].as_f64().unwrap();
        assert!((last - 0.7).abs() < 1e-12, "外延取末值: {last}");
        // easeIn 中点应显著偏离线性 0.6(壳画曲线形状,不重算公式)
        let at500 = sarr.iter().find(|pt| pt[0] == json!(500)).unwrap()[1].as_f64().unwrap();
        assert!((at500 - 0.6).abs() > 0.02, "easeIn 采样必须偏离线性: {at500}");
        // 无关键帧 clip:键恒在,null / 空数组
        assert_eq!(without["keyframes"], json!(Value::Null));
        assert_eq!(without["keyframeSamples"], json!([]));
    }
}

