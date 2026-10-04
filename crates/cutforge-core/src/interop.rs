// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 互操作(册五 T5.5/ADR-0019):OTIO 手写最小 JSON 子集 + EDL(CMX3600)手写导出。
//!
//! **OTIO = 手写最小子集**(ADR-0019,否决引 otio crate——C4 最小依赖纪律):
//! 固定覆盖 Timeline / Track(Video/Audio)/ Clip(source_range + 相对路径
//! media_reference)/ Gap / Marker / Transition(基础型)/ Stack(复合片段 ↔
//! 嵌套 Stack 映射)。导出与导入是同一份确定性映射函数的两侧;子集外的 OTIO
//! 特性(效果栈/时间变形/深层元数据/其他轨型)导入时**诚实跳过并 WARN 留痕,
//! 不静默丢**;往返语义 = 出→入→再出语义等价([`otio_semantic_eq`],name/id
//! 为不透明句柄不参与比较,时间/媒体/结构逐键等价)。
//!
//! **EDL = CMX3600 手写导出**(纯文本,人工可读判定,AC-5.5 外部解析人工验证):
//! 视频轨片段逐事件(C/D),转场映射为 D(溶解)事件;音频/文本/调整层轨在
//! 头注释里如实声明略过(不冒充全量)。
//!
//! 纪律:全部纯函数(工程/JSON 进、JSON/文本出,无 IO 无时钟无随机);
//! 时间精度 = 毫秒 ↔ 秒(f64,RationalTime rate=fps),round-trip 逐毫秒还原。

use crate::model::{Clip, CompoundSpec, Marker, Project, TrackKind, Transition};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

/// fps 允许集(schema 同源;OTIO rate 落集外时取最近值并 WARN)。
const FPS_ALLOWED: [u32; 5] = [24, 25, 30, 50, 60];
/// 转场基础枚举(schema transition.type 同源;子集内类型原样映射)。
const BASIC_TRANSITIONS: [&str; 7] = [
    "fade",
    "wipeleft",
    "wipeup",
    "slideleft",
    "circleopen",
    "cut",
    "none",
];

fn rt(ms: u64, rate: u32) -> Value {
    json!({"OTIO_SCHEMA": "RationalTime.0", "rate": rate, "value": ms as f64 / 1000.0})
}

fn rt_zero(rate: u32) -> Value {
    json!({"OTIO_SCHEMA": "RationalTime.0", "rate": rate, "value": 0.0})
}

fn time_range(start_ms: u64, dur_ms: u64, rate: u32) -> Value {
    json!({
        "OTIO_SCHEMA": "TimeRange.1",
        "start_time": rt(start_ms, rate),
        "duration": rt(dur_ms, rate),
    })
}

/// RationalTime → 毫秒(value 秒 × 1000 四舍五入;rate 仅作信息,不参与换算——
/// 子集口径:CutForge 时间轴恒毫秒,OTIO rate 只承载 fps 元信息)。
fn rt_ms(v: &Value) -> Option<u64> {
    let sec = v.get("value").and_then(Value::as_f64)?;
    let ms = (sec * 1000.0).round();
    if ms < 0.0 { None } else { Some(ms as u64) }
}

fn time_range_ms(v: &Value) -> Option<(u64, u64)> {
    let start = rt_ms(v.get("start_time")?)?;
    let dur = rt_ms(v.get("duration")?)?;
    Some((start, dur))
}

// ---------------- 导出:工程 → OTIO JSON ----------------

/// 工程 → OTIO JSON(手写最小子集;返回 (OTIO 文档, WARN 列表))。
/// 确定性:同工程两次导出逐字节一致(键序由 json! 构造序决定,无遍历 HashMap)。
pub fn otio_export(project: &Project) -> (Value, Vec<String>) {
    let mut warns = Vec::new();
    let rate = project.fps;
    let mut track_items = Vec::new();
    for t in &project.tracks {
        match t.kind {
            TrackKind::Video | TrackKind::Audio => {}
            TrackKind::Text | TrackKind::Adjust => {
                warns.push(format!(
                    "轨 {}({}) 在 OTIO 子集外(仅 Video/Audio),导出略过(留痕不静默丢);",
                    t.id,
                    t.kind.letter()
                ));
                continue;
            }
        }
        let mut children: Vec<Value> = Vec::new();
        let mut clips: Vec<&Clip> = t.clips.iter().collect();
        clips.sort_by_key(|c| (c.start_ms, c.id.clone()));
        let mut cursor: u64 = 0;
        for c in clips {
            if c.start_ms > cursor {
                // 前置/片段间间隙 → Gap(子集内;导入按占位还原 startMs)
                children.push(json!({
                    "OTIO_SCHEMA": "Gap.1",
                    "name": "gap",
                    "source_range": time_range(cursor, c.start_ms - cursor, rate),
                }));
            }
            if let Some(tr) = &c.transition {
                let (ttype, dur_ms) = transition_export(tr);
                children.push(json!({
                    "OTIO_SCHEMA": "Transition.1",
                    "name": ttype,
                    "transition_type": ttype,
                    "in_offset": rt(dur_ms, rate),
                    "out_offset": rt_zero(rate),
                    "metadata": {"cutforge": {"durMs": dur_ms}},
                }));
            }
            children.push(clip_export(c, rate, &mut warns));
            cursor = cursor.max(c.start_ms + c.duration_ms);
        }
        track_items.push(json!({
            "OTIO_SCHEMA": "Track.1",
            "name": t.id,
            "kind": if t.kind == TrackKind::Video { "Video" } else { "Audio" },
            "children": children,
        }));
    }
    let markers: Vec<Value> = project
        .markers
        .iter()
        .flatten()
        .map(|m: &Marker| {
            json!({
                "OTIO_SCHEMA": "Marker.1",
                "name": m.label,
                "marked_range": time_range(m.ms, 0, rate),
            })
        })
        .collect();
    let timeline = json!({
        "OTIO_SCHEMA": "Timeline.1",
        "name": project.slug,
        "global_start_time": rt_zero(rate),
        "metadata": {"cutforge": {
            "generator": "cutforge otio-min-subset (ADR-0019)",
            "schemaVersion": project.schema_version,
            "fps": rate,
        }},
        "tracks": {
            "OTIO_SCHEMA": "Stack.1",
            "name": "tracks",
            "children": track_items,
            "markers": markers,
        },
    });
    (timeline, warns)
}

fn transition_export(tr: &Transition) -> (String, u64) {
    let ttype = tr.type_.clone().unwrap_or_else(|| "fade".into());
    let dur = tr.dur_ms.unwrap_or(0.0).round().max(0.0) as u64;
    (ttype, dur)
}

/// 单片段 → OTIO Item:普通片段 = Clip + ExternalReference(相对路径);
/// 复合片段 = Clip + 嵌套 Stack(子时间线单 Video 轨,局部时间域原样);
/// 无 src 无 compound 的片段(文本误挂视频轨等)跳过 + WARN。
fn clip_export(c: &Clip, rate: u32, warns: &mut Vec<String>) -> Value {
    let mut item = Map::new();
    item.insert("OTIO_SCHEMA".into(), json!("Clip.1"));
    item.insert("name".into(), json!(c.id));
    let dur = c.duration_ms;
    item.insert(
        "source_range".into(),
        time_range(c.source_in_ms.unwrap_or(0), dur, rate),
    );
    if let Some(cp) = &c.compound {
        // 复合 ↔ Stack:子时间线 = 单 Video 轨(ADR-0019 单轨语义),局部时间域
        let mut inner_children: Vec<Value> = Vec::new();
        let mut inner: Vec<&Clip> = cp.clips.iter().collect();
        inner.sort_by_key(|c| (c.start_ms, c.id.clone()));
        let mut cursor = 0u64;
        for ic in inner {
            if ic.start_ms > cursor {
                inner_children.push(json!({
                    "OTIO_SCHEMA": "Gap.1",
                    "name": "gap",
                    "source_range": time_range(cursor, ic.start_ms - cursor, rate),
                }));
            }
            if let Some(tr) = &ic.transition {
                let (ttype, dur_ms) = transition_export(tr);
                inner_children.push(json!({
                    "OTIO_SCHEMA": "Transition.1",
                    "name": ttype,
                    "transition_type": ttype,
                    "in_offset": rt(dur_ms, rate),
                    "out_offset": rt_zero(rate),
                    "metadata": {"cutforge": {"durMs": dur_ms}},
                }));
            }
            inner_children.push(clip_export(ic, rate, warns));
            cursor = cursor.max(ic.start_ms + ic.duration_ms);
        }
        item.insert(
            "children".into(),
            json!({
                "OTIO_SCHEMA": "Stack.1",
                "name": "compound",
                "children": [{
                    "OTIO_SCHEMA": "Track.1",
                    "name": "compound_video",
                    "kind": "Video",
                    "children": inner_children,
                }],
            }),
        );
    } else if let Some(src) = &c.src {
        item.insert(
            "media_reference".into(),
            json!({
                "OTIO_SCHEMA": "ExternalReference.1",
                "name": src,
                "target_url": src,
            }),
        );
    } else {
        warns.push(format!(
            "clip {} 无 src 无 compound(疑似文本/占位片段),OTIO 导出略过(留痕不静默丢);",
            c.id
        ));
    }
    Value::Object(item)
}

// ---------------- 导入:OTIO JSON → 工程 ----------------

/// OTIO JSON → 工程(手写最小子集;子集外元素 WARN 留痕,不静默丢)。
/// 返回 (工程, WARN 列表);结构非法(缺 Timeline/tracks)→ Err。
pub fn otio_import(v: &Value) -> Result<(Project, Vec<String>), Vec<String>> {
    let mut warns = Vec::new();
    subset_scan(v, "$", &mut warns);
    if v.get("OTIO_SCHEMA").and_then(Value::as_str) != Some("Timeline.1") {
        return Err(vec![format!(
            "OTIO 根必须是 Timeline.1(实际 {:?});子集口径见 docs/PROJECT-FORMAT.md",
            v.get("OTIO_SCHEMA").and_then(Value::as_str)
        )]);
    }
    let slug = v
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("otio-import")
        .to_string();
    let tracks = v
        .get("tracks")
        .ok_or_else(|| vec!["OTIO 缺 tracks(Stack)".into()])?;
    // fps:metadata.cutforge.fps 优先;落允许集外取缺省 30 并 WARN
    let mut rate = v
        .pointer("/metadata/cutforge/fps")
        .and_then(Value::as_u64)
        .map(|n| n as u32)
        .unwrap_or(30);
    if !FPS_ALLOWED.contains(&rate) {
        rate = 30;
        warns.push(format!("fps 不在允许集 {FPS_ALLOWED:?},取缺省 30(留痕);"));
    }
    let mut project = Project {
        version: 1,
        schema_version: v
            .pointer("/metadata/cutforge/schemaVersion")
            .and_then(Value::as_str)
            .unwrap_or("3.0.0")
            .to_string(),
        slug,
        fps: rate,
        canvas: crate::model::Canvas {
            width: 1080,
            height: 1920,
        },
        backends: vec![crate::model::Backend::Ffmpeg],
        notes: "notes.json".into(),
        tracks: Vec::new(),
        bgm: None,
        outputs: None,
        markers: None,
        subtitle: None,
        join_crossfade_ms: None,
    };
    if let Some(sv) = v
        .pointer("/metadata/cutforge/schemaVersion")
        .and_then(Value::as_str)
    {
        project.schema_version = sv.to_string();
    } else {
        project.schema_version = "3.0.0".into();
    }
    // markers(Stack.markers)
    if let Some(ms) = tracks
        .get("markers")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
    {
        let mut out = Vec::new();
        for m in ms {
            if m.get("OTIO_SCHEMA").and_then(Value::as_str) != Some("Marker.1") {
                continue;
            }
            let label = m
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let (start, _dur) = m
                .get("marked_range")
                .and_then(time_range_ms)
                .unwrap_or((0, 0));
            out.push(Marker { ms: start, label });
        }
        project.markers = Some(out);
    }
    // 轨(Stack.children);id 全局占用表(导入产物 id 不重复,确定性分配)
    let mut used: BTreeSet<String> = BTreeSet::new();
    let Some(children) = tracks.get("children").and_then(Value::as_array) else {
        return Err(vec!["OTIO Stack 缺 children".into()]);
    };
    for tr in children {
        if tr.get("OTIO_SCHEMA").and_then(Value::as_str) != Some("Track.1") {
            warns.push(format!(
                "子集外 Stack 子项({:?})跳过(留痕不静默丢);",
                tr.get("OTIO_SCHEMA").and_then(Value::as_str)
            ));
            continue;
        }
        let kind = match tr.get("kind").and_then(Value::as_str) {
            Some("Video") => TrackKind::Video,
            Some("Audio") => TrackKind::Audio,
            other => {
                warns.push(format!(
                    "轨 kind={other:?} 在子集外(仅 Video/Audio),整轨跳过(留痕不静默丢);"
                ));
                continue;
            }
        };
        let name = tr.get("name").and_then(Value::as_str).unwrap_or("");
        let track_id = if project.find_track(name).is_none() && track_id_valid(name) {
            name.to_string()
        } else {
            next_track_id_for(&project, kind)
        };
        let mut t_cursor = 0u64;
        let mut clips: Vec<Clip> = Vec::new();
        let mut pending_transition: Option<(String, u64)> = None;
        let mut clip_cursor = 0usize; // 子时间线递归时自增的虚拟序号(确定性 id 分配)
        for item in tr
            .get("children")
            .and_then(Value::as_array)
            .map(|a| a.as_slice())
            .unwrap_or(&[])
        {
            match item.get("OTIO_SCHEMA").and_then(Value::as_str) {
                Some("Gap.1") => {
                    if let Some((_s, dur)) = item.get("source_range").and_then(time_range_ms) {
                        t_cursor += dur;
                    }
                }
                Some("Transition.1") => {
                    let ttype = item
                        .get("transition_type")
                        .and_then(Value::as_str)
                        .unwrap_or("fade");
                    let dur = item
                        .pointer("/metadata/cutforge/durMs")
                        .and_then(Value::as_u64)
                        .or_else(|| item.get("in_offset").and_then(rt_ms))
                        .unwrap_or(0);
                    match map_transition_type(ttype, &mut warns) {
                        Some(t) => pending_transition = Some((t, dur)),
                        None => warns.push(format!(
                            "转场类型 {ttype:?} 在子集外,降级 fade(留痕不静默丢);"
                        )),
                    }
                }
                Some("Clip.1") => {
                    let (clip, next_cursor, next_seq) = clip_import(
                        item,
                        t_cursor,
                        &track_id,
                        clip_cursor,
                        &mut used,
                        &mut warns,
                        0,
                    );
                    if let Some(c) = clip {
                        let mut c = c;
                        if let Some((t, dur)) = pending_transition.take() {
                            c.transition = Some(Transition {
                                type_: Some(t),
                                dur_ms: Some(dur as f64),
                                reason: None,
                                fx: None,
                            });
                        }
                        t_cursor = next_cursor;
                        clip_cursor = next_seq;
                        clips.push(c);
                    }
                }
                other => {
                    warns.push(format!("子集外轨内条目({other:?})跳过(留痕不静默丢);"));
                }
            }
        }
        project.tracks.push(crate::model::Track {
            id: track_id,
            kind,
            name: None,
            locked: None,
            mute: None,
            solo: None,
            hidden: None,
            height_px: None,
            color: None,
            eq: None,
            dyn_: None,
            clips,
        });
    }
    let errs = project.validate_keyframes();
    if !errs.is_empty() {
        return Err(errs);
    }
    Ok((project, warns))
}

fn track_id_valid(id: &str) -> bool {
    let mut ch = id.chars();
    let (Some(c), rest) = (ch.next(), ch.as_str()) else {
        return false;
    };
    matches!(c, 'V' | 'A' | 'T' | 'X')
        && !rest.is_empty()
        && rest.chars().all(|d| d.is_ascii_digit())
}

fn next_track_id_for(project: &Project, kind: TrackKind) -> String {
    project.next_track_id(kind)
}

/// 转场类型映射:子集内原样;SMPTE_Dissolve → fade;其余 None(调用方 WARN 降级)。
fn map_transition_type(t: &str, _warns: &mut Vec<String>) -> Option<String> {
    match t {
        "SMPTE_Dissolve" | "SMPTE_Wipe" => Some("fade".into()),
        other if BASIC_TRANSITIONS.contains(&other) => Some(other.to_string()),
        _ => None,
    }
}

/// OTIO Clip.1 → 工程 Clip(时域:record 起点 = t_cursor;durationMs = 源 duration;
/// sourceInMs = source_range.start_time)。嵌套 Stack → 复合片段(首条 Video 子轨)。
/// 返回 (片段(None=跳过), 推进后的 record 游标, 递增后的虚拟序号)。
#[allow(clippy::too_many_arguments)]
fn clip_import(
    item: &Value,
    t_cursor: u64,
    track_id: &str,
    mut seq: usize,
    used: &mut BTreeSet<String>,
    warns: &mut Vec<String>,
    depth: usize,
) -> (Option<Clip>, u64, usize) {
    let Some((start_ms, dur_ms)) = item.get("source_range").and_then(time_range_ms) else {
        warns.push("Clip 缺 source_range,跳过(留痕不静默丢);".into());
        return (None, t_cursor, seq);
    };
    let name = item.get("name").and_then(Value::as_str).unwrap_or("");
    let id = fresh_id(name, track_id, &mut seq, used);
    let mut clip = Clip {
        id,
        source_hash: None,
        src: None,
        start_ms: t_cursor,
        duration_ms: dur_ms.max(1),
        source_in_ms: Some(start_ms),
        speed: None,
        speed_curve: None,
        reverse: None,
        rotation: None,
        crop: None,
        flip: None,
        volume: None,
        role: None,
        text: None,
        text_style: None,
        huazi: None,
        font: None,
        denoise: None,
        pitch: None,
        position: None,
        scale: None,
        reframe: None,
        motion: None,
        transition: None,
        overlay: None,
        opacity: None,
        fade: None,
        loop_: None,
        punch_in: None,
        freeze_ms: None,
        fx: None,
        keyframes: None,
        grade: None,
        compound: None,
    };
    // 嵌套 Stack → 复合片段(首条 Video 子轨;其余子轨 WARN)
    if let Some(stack) = item.get("children") {
        if depth >= 1 {
            warns.push("复合嵌套超深(子集上限两级),内层按普通片段略过(留痕);".into());
        } else if let Some(inner_track) =
            stack
                .get("children")
                .and_then(Value::as_array)
                .and_then(|a| {
                    a.iter()
                        .find(|t| t.get("kind").and_then(Value::as_str) == Some("Video"))
                })
        {
            let mut inner_cursor = 0u64;
            let mut inner_clips: Vec<Clip> = Vec::new();
            let mut pending: Option<(String, u64)> = None;
            for ie in inner_track
                .get("children")
                .and_then(Value::as_array)
                .map(|a| a.as_slice())
                .unwrap_or(&[])
            {
                match ie.get("OTIO_SCHEMA").and_then(Value::as_str) {
                    Some("Gap.1") => {
                        if let Some((_s, dur)) = ie.get("source_range").and_then(time_range_ms) {
                            inner_cursor += dur;
                        }
                    }
                    Some("Transition.1") => {
                        let ttype = ie
                            .get("transition_type")
                            .and_then(Value::as_str)
                            .unwrap_or("fade");
                        let dur = ie
                            .pointer("/metadata/cutforge/durMs")
                            .and_then(Value::as_u64)
                            .or_else(|| ie.get("in_offset").and_then(rt_ms))
                            .unwrap_or(0);
                        match map_transition_type(ttype, warns) {
                            Some(t) => pending = Some((t, dur)),
                            None => warns.push(format!("内层转场 {ttype:?} 子集外,降级 fade;")),
                        }
                    }
                    Some("Clip.1") => {
                        let (ic, nc, ns) =
                            clip_import(ie, inner_cursor, "V1", seq, used, warns, depth + 1);
                        if let Some(mut c) = ic {
                            if let Some((t, dur)) = pending.take() {
                                c.transition = Some(Transition {
                                    type_: Some(t),
                                    dur_ms: Some(dur as f64),
                                    reason: None,
                                    fx: None,
                                });
                            }
                            inner_cursor = nc;
                            seq = ns;
                            inner_clips.push(c);
                        }
                    }
                    _ => warns.push("内层子集外条目跳过(留痕不静默丢);".into()),
                }
            }
            if inner_clips.is_empty() {
                warns.push("嵌套 Stack 无 Video 子轨片段,复合壳按空略过(留痕);".into());
            } else {
                clip.compound = Some(CompoundSpec {
                    canvas: None,
                    clips: inner_clips,
                });
            }
        } else {
            warns.push("嵌套 Stack 无 Video 子轨,略过(留痕不静默丢);".into());
        }
    } else if let Some(mr) = item.get("media_reference") {
        clip.src = mr
            .get("target_url")
            .and_then(Value::as_str)
            .map(String::from);
        if clip.src.is_none() {
            warns.push(format!(
                "clip {} 的 media_reference 缺 target_url(留痕);",
                clip.id
            ));
        }
    }
    (Some(clip), t_cursor + dur_ms.max(1), seq)
}

/// 确定性 id:优先沿用合法且未占用的 OTIO name;否则 <track>-<seq 三位零填> 起步重试。
fn fresh_id(name: &str, track_id: &str, seq: &mut usize, used: &mut BTreeSet<String>) -> String {
    if track_id_valid(name) && !used.contains(name) {
        used.insert(name.to_string());
        return name.to_string();
    }
    loop {
        let id = format!("{track_id}-{:03}", *seq + 1);
        *seq += 1;
        if !used.contains(&id) {
            used.insert(id.clone());
            return id;
        }
    }
}

/// 子集外扫描:递归收集不在子集 schema 名单内的 OTIO_SCHEMA 与已知对象上的
/// 子集外键(逐节点 WARN 留痕;不阻断)。metadata 整体豁免(承载 cutforge
/// 往返字段;外部 metadata 键不深扫=留痕一次,不逐叶爆)。
fn subset_scan(v: &Value, path: &str, warns: &mut Vec<String>) {
    const KNOWN_SCHEMAS: [&str; 10] = [
        "Timeline.1",
        "Stack.1",
        "Track.1",
        "Clip.1",
        "Gap.1",
        "Transition.1",
        "Marker.1",
        "RationalTime.0",
        "TimeRange.1",
        "ExternalReference.1",
    ];
    const KNOWN_KEYS: [&str; 18] = [
        "OTIO_SCHEMA",
        "name",
        "kind",
        "children",
        "source_range",
        "media_reference",
        "target_url",
        "transition_type",
        "markers",
        "global_start_time",
        "start_time",
        "duration",
        "rate",
        "value",
        "in_offset",
        "out_offset",
        "marked_range",
        "tracks",
    ];
    match v {
        Value::Object(m) => {
            if let Some(s) = m.get("OTIO_SCHEMA").and_then(Value::as_str)
                && !KNOWN_SCHEMAS.contains(&s)
            {
                warns.push(format!("子集外 OTIO_SCHEMA {s} @{path}(留痕不静默丢);"));
            }
            for (k, val) in m {
                if k == "metadata" {
                    continue;
                }
                if !KNOWN_KEYS.contains(&k.as_str()) {
                    warns.push(format!("子集外键 {k} @{path}(留痕不静默丢);"));
                }
                subset_scan(val, &format!("{path}/{k}"), warns);
            }
        }
        Value::Array(a) => {
            for (i, el) in a.iter().enumerate() {
                subset_scan(el, &format!("{path}[{i}]"), warns);
            }
        }
        _ => {}
    }
}

/// OTIO 语义等价(往返判据,AC-5.5):剥掉 name(与 id 同为不透明句柄——
/// 复合内层 id 在导入时确定性重分配)后逐键相等;时间值经毫秒取整后比较。
pub fn otio_semantic_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(ma), Value::Object(mb)) => {
            let keys: BTreeSet<&String> = ma.keys().filter(|k| *k != "name").collect();
            keys == mb.keys().filter(|k| *k != "name").collect()
                && keys.iter().all(|k| otio_semantic_eq(&ma[*k], &mb[*k]))
        }
        (Value::Array(aa), Value::Array(ab)) => {
            aa.len() == ab.len() && aa.iter().zip(ab).all(|(x, y)| otio_semantic_eq(x, y))
        }
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() < 1e-9,
            _ => x == y,
        },
        _ => a == b,
    }
}

// ---------------- EDL(CMX3600)导出 ----------------

/// ms → CMX3600 时间码 HH:MM:SS:FF(fps 整数帧率,非丢帧)。
fn tc(ms: u64, fps: u32) -> String {
    let frames = (ms as f64 * fps as f64 / 1000.0).round() as u64;
    let ff = frames % fps as u64;
    let total_sec = frames / fps as u64;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        total_sec / 3600,
        total_sec % 3600 / 60,
        total_sec % 60,
        ff
    )
}

/// 片段的有效转场帧数(EDL D 事件时长):type 非 cut/none 且 durMs>0 →
/// round(durMs×fps/1000) 钳非负;无转场 → 0(映射 C 事件)。
/// (子集口径:与渲染端 effective_transition_ms 的两侧钳制不同——EDL 是
/// 清单交换不是渲染图,只取声明时长;注释字段同步给出 durMs。)
fn effective_transition_frames(c: &Clip, fps: u32) -> u64 {
    let Some(tr) = &c.transition else { return 0 };
    match tr.type_.as_deref() {
        Some("cut") | Some("none") => return 0,
        _ => {}
    }
    let dur = tr.dur_ms.unwrap_or(0.0);
    if dur <= 0.0 {
        return 0;
    }
    ((dur * fps as f64) / 1000.0).round() as u64
}

/// 工程 → CMX3600 EDL 文本(手写;视频轨逐事件 C/D,转场映射为 D 溶解事件;
/// 音频/文本/调整层轨在头注释如实声明略过)。生成器与版本写入头注释(AC-5.5)。
pub fn edl_export(project: &Project) -> String {
    let fps = project.fps;
    let mut out = String::new();
    out.push_str(&format!("TITLE: {}\n", project.slug));
    out.push_str(&format!(
        "* generated by cutforge {} (CMX3600 subset: video tracks only; ADR-0019)\n",
        env!("CARGO_PKG_VERSION")
    ));
    out.push_str("* timebase: non-drop; source/rec times in HH:MM:SS:FF\n");
    for t in &project.tracks {
        if t.kind != TrackKind::Video {
            let kind = t.kind.letter();
            out.push_str(&format!(
                "* SKIPPED TRACK: {} (track {} not in CMX3600 subset)\n",
                t.id, kind
            ));
        }
    }
    out.push_str("FCM: NON-DROP FRAME\n\n");
    let mut events: Vec<(&Clip, &str)> = Vec::new();
    for t in &project.tracks {
        if t.kind != TrackKind::Video {
            continue;
        }
        for c in &t.clips {
            if c.overlay.is_some() {
                out.push_str(&format!(
                    "* SKIPPED CLIP: {} (overlay segment, not a record event)\n",
                    c.id
                ));
                continue;
            }
            events.push((c, &t.id));
        }
    }
    events.sort_by_key(|(c, _)| (c.start_ms, c.id.clone()));
    for (i, (c, track_id)) in events.iter().enumerate() {
        let num = format!("{:03}", i + 1);
        let reel = track_id.to_string();
        let src_in = c.source_in_ms.unwrap_or(0);
        let src_out = src_in + c.duration_ms.max(1);
        let rec_in = c.start_ms;
        let rec_out = rec_in + c.duration_ms.max(1);
        // 转场映射:有效转场(xfade,非 cut/none 且 dur>0)→ D 溶解事件,时长 = 帧数
        let dur_frames = effective_transition_frames(c, fps);
        if dur_frames > 0 {
            out.push_str(&format!(
                "{num}  {reel}  V     D    {:03}       {} {} {} {}\n",
                dur_frames.min(999),
                tc(src_in, fps),
                tc(src_out, fps),
                tc(rec_in, fps),
                tc(rec_out, fps),
            ));
        } else {
            out.push_str(&format!(
                "{num}  {reel}  V     C        {} {} {} {}\n",
                tc(src_in, fps),
                tc(src_out, fps),
                tc(rec_in, fps),
                tc(rec_out, fps),
            ));
        }
        out.push_str(&format!(
            "* FROM CLIP NAME: {}\n",
            c.src.as_deref().unwrap_or("(compound)")
        ));
        if let Some(cp) = &c.compound {
            out.push_str(&format!(
                "* NESTED STACK: {} inner clip(s) (compound clip, ADR-0019)\n",
                cp.clips.len()
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    // include! 相对当前文件(src/)解析,POSIX/Windows 一致(#[path] 在非 mod.rs
    // 文件内会相对不存在的 stem 目录解析,POSIX 失败——CI ubuntu 实证)。
    include!("interop_tests.rs");
}
