// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 字幕/文本工具后端(册四 A4 T4.7;语义裁决全在内核 Command,经 Workspace::apply
//! 唯一写入口,undo/redo/replay 天然支持):
//! - `text_add` → `Command::ClipsInsert`(单片段,单 Op 原子);
//! - `subtitle_import` → SRT/ASS 解析(cutforge_render::subtitle 纯函数,单一实现)
//!   → `Command::ClipsInsert` 整批单 Op 原子;
//! - `subtitle_export` → 文本轨 → SRT/ASS 导出(产物写盘,不产 Op 不改 IR——
//!   与 render 成片同类的派生物);
//! - `subtitle_replace` → 参数化查找替换 → `Command::ClipsPatch` 单 Op 原子。
//!
//! 本模块只做参数抽取、文本轨定位与 id 分配,禁止旁路写工程文件。

use crate::registry::envelope;
use cutforge_core::command::{ClipPatch, Command};
use cutforge_core::engine::ApplyOpts;
use cutforge_core::model::{Clip, TrackKind};
use cutforge_core::oplog::Actor;
use cutforge_core::text_style::{Huazi, TextStyle};
use cutforge_io::Workspace;
use serde_json::{Value, json};
use std::path::Path;

/// 成功回执追加 data 字段(失败原样透传)。
fn with_data(env: Value, f: impl FnOnce(&mut Value)) -> Value {
    let mut env = env;
    if env["ok"] == json!(true) {
        f(&mut env);
    }
    env
}

/// 解析 textStyle/huazi 对象参数(非法结构 → SCHEMA_INVALID,整单拒绝)。
fn parse_style_args(args: &Value) -> Result<(Option<TextStyle>, Option<Huazi>), String> {
    let ts = match args.get("textStyle") {
        None | Some(Value::Null) => None,
        Some(v) if v.is_object() => Some(
            serde_json::from_value::<TextStyle>(v.clone())
                .map_err(|e| format!("textStyle 非法: {e}"))?,
        ),
        Some(_) => return Err("textStyle 必须是对象".into()),
    };
    let hz = match args.get("huazi") {
        None | Some(Value::Null) => None,
        Some(v) if v.is_object() => Some(
            serde_json::from_value::<Huazi>(v.clone()).map_err(|e| format!("huazi 非法: {e}"))?,
        ),
        Some(_) => return Err("huazi 必须是对象".into()),
    };
    Ok((ts, hz))
}

/// 定位文本轨:显式 trackId(必须存在且 kind=text)或第一文本轨(无 → Err)。
fn locate_text_track(ws: &Workspace, track_id: Option<&str>) -> Result<String, String> {
    match track_id {
        Some(id) => match ws.project().find_track(id) {
            Some(ti) if ws.project().tracks[ti].kind == TrackKind::Text => Ok(id.to_string()),
            Some(_) => Err(format!("轨 {id} 不是文本轨")),
            None => Err(format!("track 不存在: {id}")),
        },
        None => ws
            .project()
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Text)
            .map(|t| t.id.clone())
            .ok_or_else(|| "工程无文本轨:先 track_add(kind=text)".into()),
    }
}

/// text_add:在指定时间新建文本片段到文本轨(单 Op;atMs 为播放头毫秒——
/// 播放头是壳态,FE 传值;text 即可经 clip_update 再改)。
pub fn text_add_tool(ws: &mut Workspace, args: &Value, actor: &Actor, opts: ApplyOpts) -> Value {
    let Some(text) = args["text"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 text(文本内容)", json!({}));
    };
    let Some(at_ms) = args["atMs"].as_u64() else {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            "缺 atMs(播放头毫秒;壳传值)",
            json!({}),
        );
    };
    let duration_ms = args["durationMs"].as_u64().unwrap_or(3000).max(1);
    let track_id = match locate_text_track(ws, args["trackId"].as_str()) {
        Ok(id) => id,
        Err(m) => return envelope(false, "PRECONDITION_FAILED", &m, json!({})),
    };
    let (text_style, huazi) = match parse_style_args(args) {
        Ok(v) => v,
        Err(m) => return envelope(false, "SCHEMA_INVALID", &m, json!({})),
    };
    let ti = ws.project().find_track(&track_id).unwrap();
    let clip_id = cutforge_core::model::Project::next_clip_id(&ws.project().tracks[ti]);
    let clip: Clip = match serde_json::from_value(json!({
        "id": clip_id, "startMs": at_ms, "durationMs": duration_ms,
        "text": text,
        "textStyle": text_style,
        "huazi": huazi,
    })) {
        Ok(c) => c,
        Err(e) => return envelope(false, "SCHEMA_INVALID", &e.to_string(), json!({})),
    };
    let clip_id_out = clip.id.clone();
    let env = crate::dispatch::finish_apply(ws.apply(
        Command::ClipsInsert {
            to_track: track_id,
            clips: vec![clip],
            request_id: opts.request_id.clone(),
        },
        actor.clone(),
        opts,
    ));
    with_data(env, |d| {
        // 单片段插入成功时回带 clipId(FE 直接选中新建片段)
        d["data"]["clipId"] = json!(clip_id_out);
    })
}

/// subtitle_import:SRT/ASS → 批量文本片段(整批单 Op 原子;解析经
/// cutforge_render::subtitle 单一实现,零旁路解析器)。
pub fn subtitle_import_tool(
    ws: &mut Workspace,
    root: &Path,
    args: &Value,
    actor: &Actor,
    opts: ApplyOpts,
) -> Value {
    let Some(src) = args["src"].as_str() else {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            "缺 src(字幕文件,工程内相对路径)",
            json!({}),
        );
    };
    let abs = match crate::dispatch::resolve_within_root(root, src) {
        Ok(p) => p,
        Err(msg) => {
            return envelope(
                false,
                "PRECONDITION_FAILED",
                &format!("路径不合法({src}): {msg}"),
                json!({}),
            );
        }
    };
    let text = match std::fs::read_to_string(&abs) {
        Ok(t) => t,
        Err(e) => {
            return envelope(
                false,
                "NO_CONFIG",
                &format!("字幕文件不可读({src}): {e}"),
                json!({}),
            );
        }
    };
    let Some(lines) = cutforge_render::subtitle::parse_auto(&text) else {
        return envelope(
            false,
            "SCHEMA_INVALID",
            &format!("字幕解析零条目(仅支持 SRT/ASS): {src}"),
            json!({}),
        );
    };
    let track_id = match locate_text_track(ws, args["trackId"].as_str()) {
        Ok(id) => id,
        Err(m) => return envelope(false, "PRECONDITION_FAILED", &m, json!({})),
    };
    let (text_style, huazi) = match parse_style_args(args) {
        Ok(v) => v,
        Err(m) => return envelope(false, "SCHEMA_INVALID", &m, json!({})),
    };
    let ti = ws.project().find_track(&track_id).unwrap();
    // id 批量确定性分配:探针轨本地推进(next_clip_id 的冲突视野),不触工作区
    let mut probe = ws.project().tracks[ti].clone();
    let mut clips: Vec<Clip> = Vec::with_capacity(lines.len());
    for l in &lines {
        let clip_id = cutforge_core::model::Project::next_clip_id(&probe);
        let c: Clip = match serde_json::from_value(json!({
            "id": clip_id, "startMs": l.at_ms, "durationMs": l.duration_ms,
            "text": l.text, "textStyle": text_style, "huazi": huazi,
        })) {
            Ok(c) => c,
            Err(e) => return envelope(false, "SCHEMA_INVALID", &e.to_string(), json!({})),
        };
        probe.clips.push(c.clone());
        clips.push(c);
    }
    let inserted = clips.len();
    let first_id = clips.first().map(|c| c.id.clone()).unwrap_or_default();
    let env = crate::dispatch::finish_apply(ws.apply(
        Command::ClipsInsert {
            to_track: track_id,
            clips,
            request_id: opts.request_id.clone(),
        },
        actor.clone(),
        opts,
    ));
    with_data(env, |d| {
        d["data"]["imported"] = json!(inserted);
        d["data"]["firstClipId"] = json!(first_id);
    })
}

/// subtitle_export:文本轨 → SRT/ASS 文件(派生物;不产 Op 不改 IR;
/// 缺省落 06_成片输出/,工程内路径可经 /media 加载)。
pub fn subtitle_export_tool(ws: &Workspace, root: &Path, args: &Value) -> Value {
    let Some(format) = args["format"].as_str() else {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            "缺 format(srt|ass)",
            json!({}),
        );
    };
    if !matches!(format, "srt" | "ass") {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            &format!("未知 format: {format}(允许 srt|ass)"),
            json!({}),
        );
    }
    let track_id = match locate_text_track(ws, args["trackId"].as_str()) {
        Ok(id) => id,
        Err(m) => return envelope(false, "PRECONDITION_FAILED", &m, json!({})),
    };
    let ti = ws.project().find_track(&track_id).unwrap();
    let clips: Vec<cutforge_render::subtitle::ExportClip> = ws.project().tracks[ti]
        .clips
        .iter()
        .filter(|c| c.text.as_deref().is_some_and(|t| !t.trim().is_empty()))
        .map(|c| cutforge_render::subtitle::ExportClip {
            id: c.id.clone(),
            at_ms: c.start_ms,
            duration_ms: c.duration_ms,
            text: c.text.clone().unwrap_or_default(),
        })
        .collect();
    if clips.is_empty() {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            &format!("轨 {track_id} 上无文本片段可导出"),
            json!({}),
        );
    }
    let (canvas_w, canvas_h) = (ws.project().canvas.width, ws.project().canvas.height);
    let (payload, ext) = match format {
        "srt" => (cutforge_render::subtitle::srt_export_clips(&clips), "srt"),
        _ => (
            cutforge_render::subtitle::ass_export(&clips, canvas_w, canvas_h),
            "ass",
        ),
    };
    let out = match args["out"].as_str() {
        Some(o) => {
            if o.split(['/', '\\']).any(|seg| seg == "..") || Path::new(o).is_absolute() {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    &format!("out 必须是工程内相对路径: {o}"),
                    json!({}),
                );
            }
            o.to_string()
        }
        None => format!(
            "{}/subtitles_{track_id}.{ext}",
            cutforge_io::paths::output_dir_name(root)
        ),
    };
    let abs_out = root.join(&out);
    if let Some(dir) = abs_out.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        return envelope(
            false,
            "INTERNAL",
            &format!("导出目录创建失败: {e}"),
            json!({}),
        );
    }
    if let Err(e) = cutforge_io::atomic::atomic_write(&abs_out, payload.as_bytes()) {
        return envelope(false, "INTERNAL", &format!("导出落盘失败: {e}"), json!({}));
    }
    envelope(
        true,
        "OK",
        "字幕已导出",
        json!({
            "format": format, "trackId": track_id, "count": clips.len(),
            "out": out, "bytes": payload.len(), "media": out,
        }),
    )
}

/// subtitle_replace:文本轨参数化查找替换(单 Op 原子;find 非空;trackId 缺省 =
/// 全部文本轨;正文含 find 的片段逐条 ClipPatch{text})。
pub fn subtitle_replace_tool(
    ws: &mut Workspace,
    args: &Value,
    actor: &Actor,
    opts: ApplyOpts,
) -> Value {
    let Some(find) = args["find"].as_str() else {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            "缺 find(查找串,非空)",
            json!({}),
        );
    };
    if find.is_empty() {
        return envelope(false, "PRECONDITION_FAILED", "find 不得为空串", json!({}));
    }
    let Some(replace) = args["replace"].as_str() else {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            "缺 replace(替换串;清空用空串)",
            json!({}),
        );
    };
    let targets: Vec<String> = match args["trackId"].as_str() {
        Some(id) => match locate_text_track(ws, Some(id)) {
            Ok(tid) => vec![tid],
            Err(m) => return envelope(false, "PRECONDITION_FAILED", &m, json!({})),
        },
        None => ws
            .project()
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Text)
            .map(|t| t.id.clone())
            .collect(),
    };
    if targets.is_empty() {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            "工程无文本轨:先 track_add(kind=text)",
            json!({}),
        );
    }
    let mut updates: Vec<(String, ClipPatch)> = Vec::new();
    for tid in &targets {
        let Some(ti) = ws.project().find_track(tid) else {
            continue;
        };
        for c in &ws.project().tracks[ti].clips {
            if let Some(text) = &c.text
                && text.contains(find)
            {
                let new_text = text.replace(find, replace);
                updates.push((
                    c.id.clone(),
                    ClipPatch {
                        text: Some(new_text),
                        ..Default::default()
                    },
                ));
            }
        }
    }
    if updates.is_empty() {
        return envelope(true, "OK", "无匹配文本(幂等,零 Op)", json!({"replaced": 0}));
    }
    let n = updates.len();
    let env = crate::dispatch::finish_apply(ws.apply(
        Command::ClipsPatch { updates },
        actor.clone(),
        opts,
    ));
    with_data(env, |d| {
        d["data"]["replaced"] = json!(n);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cutforge_core::model::Project;

    fn ws_with_text_track() -> (Workspace, std::path::PathBuf) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("cf-subops-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("05_时间线工程")).unwrap();
        cutforge_io::atomic::atomic_write(
            &dir.join("05_时间线工程/project.json"),
            serde_json::to_string_pretty(&json!({
                "version": 1, "schemaVersion": "2.0.0", "slug": "subops", "fps": 30,
                "canvas": {"width": 1080, "height": 1920},
                "tracks": [
                    {"id": "V1", "kind": "video", "clips": []},
                    {"id": "T1", "kind": "text", "clips": []}
                ]
            }))
            .unwrap()
            .as_bytes(),
        )
        .unwrap();
        let ws = cutforge_io::Workspace::open_exclusive(&dir).unwrap();
        (ws, dir)
    }

    fn agent() -> Actor {
        Actor::agent("test")
    }

    #[test]
    fn text_add_inserts_single_op_and_returns_clip_id() {
        let (mut ws, dir) = ws_with_text_track();
        let r = text_add_tool(
            &mut ws,
            &json!({
                "text": "你好", "atMs": 2000, "durationMs": 1500,
                "textStyle": {"fontSize": 80, "color": "#FF8800"},
                "huazi": {"template": "hz.pop"}
            }),
            &agent(),
            ApplyOpts::default(),
        );
        assert_eq!(r["code"], json!("OK"), "{r}");
        assert_eq!(r["data"]["clipId"], json!("T1-001"));
        assert_eq!(r["data"]["opIds"].as_array().unwrap().len(), 1, "单 Op");
        let v = match ws.engine().query(cutforge_core::Query::ProjectView) {
            cutforge_core::engine::Answer::Project(v) => v.clone(),
            _ => unreachable!(),
        };
        let p: Project = serde_json::from_value(v).unwrap();
        let t = &p.tracks.iter().find(|t| t.id == "T1").unwrap();
        assert_eq!(t.clips.len(), 1);
        let c = &t.clips[0];
        assert_eq!(c.text.as_deref(), Some("你好"));
        assert_eq!(c.text_style.as_ref().unwrap().font_size, Some(80.0));
        assert_eq!(c.huazi.as_ref().unwrap().template, "hz.pop");
        assert_eq!(c.start_ms, 2000);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn text_add_guards_missing_track_and_bad_args() {
        let (mut ws, dir) = ws_with_text_track();
        // 缺 text
        let r = text_add_tool(&mut ws, &json!({"atMs": 0}), &agent(), ApplyOpts::default());
        assert_eq!(r["code"], json!("PRECONDITION_FAILED"));
        // 非 text 轨拒绝
        let r = text_add_tool(
            &mut ws,
            &json!({"text": "x", "atMs": 0, "trackId": "V1"}),
            &agent(),
            ApplyOpts::default(),
        );
        assert_eq!(r["code"], json!("PRECONDITION_FAILED"), "{r}");
        // textStyle 非对象
        let r = text_add_tool(
            &mut ws,
            &json!({"text": "x", "atMs": 0, "textStyle": 5}),
            &agent(),
            ApplyOpts::default(),
        );
        assert_eq!(r["code"], json!("SCHEMA_INVALID"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn subtitle_import_roundtrip_export_srt_zero_loss() {
        let (mut ws, dir) = ws_with_text_track();
        // SRT 夹具(与 cutforge_render::subtitle 测试同规范形)
        let srt = "1\n00:00:01,000 --> 00:00:02,500\n第一句\n\n2\n00:00:03,000 --> 00:00:04,000\n第二\n两行\n\n";
        std::fs::write(dir.join("subs.srt"), srt).unwrap();
        let r = subtitle_import_tool(
            &mut ws,
            &dir,
            &json!({"src": "subs.srt"}),
            &agent(),
            ApplyOpts {
                request_id: Some("imp-1".into()),
                ..Default::default()
            },
        );
        assert_eq!(r["code"], json!("OK"), "{r}");
        assert_eq!(r["data"]["imported"], json!(2));
        assert_eq!(
            r["data"]["opIds"].as_array().unwrap().len(),
            1,
            "导入必须单 Op 原子"
        );
        // 导出 SRT:导入的规范化输出与规范形逐字节一致(往返零丢失)
        let r = subtitle_export_tool(&ws, &dir, &json!({"format": "srt", "trackId": "T1"}));
        assert_eq!(r["code"], json!("OK"), "{r}");
        let out = dir.join(r["data"]["out"].as_str().unwrap());
        let exported = std::fs::read_to_string(&out).unwrap();
        assert_eq!(exported, srt, "SRT 往返 byte 级零丢失");
        // ASS 导出也可用
        let r = subtitle_export_tool(&ws, &dir, &json!({"format": "ass"}));
        assert_eq!(r["code"], json!("OK"), "{r}");
        assert!(dir.join(r["data"]["out"].as_str().unwrap()).is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn subtitle_import_ass_and_guards() {
        let (mut ws, dir) = ws_with_text_track();
        let ass = "Dialogue: 0,0:00:00.50,0:00:01.50,Default,,0,0,0,,甲{\\b1}乙\nDialogue: 0,0:00:02.00,0:00:03.00,Default,,0,0,0,,丙\n";
        std::fs::write(dir.join("subs.ass"), ass).unwrap();
        let r = subtitle_import_tool(
            &mut ws,
            &dir,
            &json!({"src": "subs.ass"}),
            &agent(),
            ApplyOpts::default(),
        );
        assert_eq!(r["code"], json!("OK"), "{r}");
        assert_eq!(r["data"]["imported"], json!(2));
        // 非字幕文件
        std::fs::write(dir.join("bad.txt"), "完全不是字幕").unwrap();
        let r = subtitle_import_tool(
            &mut ws,
            &dir,
            &json!({"src": "bad.txt"}),
            &agent(),
            ApplyOpts::default(),
        );
        assert_eq!(r["code"], json!("SCHEMA_INVALID"));
        // 越出工程根
        let r = subtitle_import_tool(
            &mut ws,
            &dir,
            &json!({"src": "../外.srt"}),
            &agent(),
            ApplyOpts::default(),
        );
        assert_eq!(r["code"], json!("PRECONDITION_FAILED"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn subtitle_replace_is_single_op_and_undoable() {
        let (mut ws, dir) = ws_with_text_track();
        let added = text_add_tool(
            &mut ws,
            &json!({"text": "旧词阿旧词", "atMs": 0, "durationMs": 1000}),
            &agent(),
            ApplyOpts::default(),
        );
        assert_eq!(added["code"], json!("OK"), "{added}");
        let r = subtitle_replace_tool(
            &mut ws,
            &json!({"find": "旧词", "replace": "新词"}),
            &agent(),
            ApplyOpts::default(),
        );
        assert_eq!(r["code"], json!("OK"), "{r}");
        assert_eq!(r["data"]["replaced"], json!(1));
        assert_eq!(
            r["data"]["opIds"].as_array().unwrap().len(),
            1,
            "替换必须单 Op"
        );
        let text = project_text(&ws, "T1-001");
        assert_eq!(text, "新词阿新词");
        // 幂等:再替换零匹配
        let r = subtitle_replace_tool(
            &mut ws,
            &json!({"find": "旧词", "replace": "新词"}),
            &agent(),
            ApplyOpts::default(),
        );
        assert_eq!(r["data"]["replaced"], json!(0));
        // undo 整批还原
        ws.undo(agent()).unwrap();
        assert_eq!(project_text(&ws, "T1-001"), "旧词阿旧词");
        // 空 find 拒绝
        let r = subtitle_replace_tool(
            &mut ws,
            &json!({"find": "", "replace": "x"}),
            &agent(),
            ApplyOpts::default(),
        );
        assert_eq!(r["code"], json!("PRECONDITION_FAILED"));
        std::fs::remove_dir_all(&dir).ok();
    }

    fn project_text(ws: &Workspace, clip_id: &str) -> String {
        let v = match ws.engine().query(cutforge_core::Query::ProjectView) {
            cutforge_core::engine::Answer::Project(v) => v.clone(),
            _ => unreachable!(),
        };
        let p: Project = serde_json::from_value(v).unwrap();
        let (ti, ci) = p.find_clip(clip_id).unwrap();
        p.tracks[ti].clips[ci].text.clone().unwrap()
    }
}
