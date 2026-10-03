// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 素材库与导入(册六 T6.2;FE1 登记的「拷贝导入通道候 BE」欠账收口):
//! - `media_library`(查询):本地素材库 manifest——库根 `library.json`
//!   (类型/标签/时长/引用),扫描合并(在位文件 ↔ manifest 条目按引用对齐,
//!   标签持久、字节/时长刷新);action=list(缺省,扫描+过滤)/tag(改条目标签);
//!   manifest 是**派生索引**(与 media_peaks 写缓存同类):扫描幂等,标签是
//!   手工面,不触工程 IR;
//! - `media_import`(写):外部素材**拷贝导入**工程——src = 绝对路径或素材库
//!   相对引用;落点随布局(v3 = media/;v2 = 01_原始素材/;v1 = 01_materials/),
//!   同名同内容幂等覆盖、同名异内容追加序号;tmp + rename 原子落盘;
//!   不产 Op 不改 IR(与 lut_import 同类写面)。
//!
//! 素材库根:env `CUTFORGE_MEDIA` 缺省 `%USERPROFILE%\CutForge\Media`
//! (与工程库根 env CUTFORGE_PROJECTS 同风格)。

use crate::registry::envelope;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// manifest 文件名(库根)。
pub const MANIFEST: &str = "library.json";

/// 素材库根缺省面(env CUTFORGE_MEDIA → %USERPROFILE%\CutForge\Media)。
pub fn default_media_root() -> PathBuf {
    if let Some(v) = std::env::var_os("CUTFORGE_MEDIA")
        && !v.is_empty()
    {
        return PathBuf::from(v);
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(home).join("CutForge").join("Media");
    }
    std::env::temp_dir().join("CutForge").join("Media")
}

/// 扩展名 → 类型(video/audio/image/lut/text);未知 → None(不入索引)。
fn kind_of(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "mp4" | "mov" | "mkv" | "webm" => Some("video"),
        "mp3" | "wav" | "m4a" | "aac" | "flac" | "ogg" => Some("audio"),
        "png" | "jpg" | "jpeg" | "webp" | "gif" => Some("image"),
        "cube" => Some("lut"),
        "ass" | "srt" | "vtt" => Some("text"),
        _ => None,
    }
}

/// 正斜杠相对路径(跨平台引用口径)。
fn fwd(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// 扫描素材库(深度 ≤3,跳点目录),与 manifest 合并:在位文件 ↔ 条目按 ref
/// 对齐——标签持久保留,bytes/时长刷新;消失文件除名;新文件时长探测
/// (ffprobe 可用时;条目上限 200,超出截断并 WARN)。返回 (doc, warnings)。
fn scan_library(root: &Path) -> Result<(Value, Vec<String>), String> {
    if !root.is_dir() {
        return Err(format!("素材库根不存在: {}", root.display()));
    }
    let mut warnings: Vec<String> = Vec::new();
    // 旧 manifest 标签对齐表(ref → tags)
    let mut old_tags: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let manifest_path = root.join(MANIFEST);
    if let Ok(text) = std::fs::read_to_string(&manifest_path)
        && let Ok(doc) = serde_json::from_str::<Value>(&text)
        && let Some(entries) = doc["entries"].as_array()
    {
        for e in entries {
            if let (Some(r), Some(t)) = (e["ref"].as_str(), e["tags"].as_array()) {
                old_tags.insert(
                    r.to_string(),
                    t.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect(),
                );
            }
        }
    }
    // 有界遍历(深度 ≤3)
    let mut files: Vec<(PathBuf, &'static str)> = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if !name.starts_with('.') && depth < 3 {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            let Some(ext) = p.extension().and_then(|x| x.to_str()) else {
                continue;
            };
            if let Some(kind) = kind_of(ext) {
                files.push((p, kind));
            }
        }
    }
    files.sort();
    if files.len() > 200 {
        files.truncate(200);
        warnings.push("素材超 200 项,索引截断(轻量索引口径)".into());
    }
    let probe_ok = cutforge_io::probe::ffprobe_available();
    let mut entries: Vec<Value> = Vec::new();
    for (p, kind) in &files {
        let Ok(rel) = p.strip_prefix(root) else {
            continue;
        };
        let ref_s = fwd(rel);
        let bytes = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let duration = if matches!(*kind, "video" | "audio") && probe_ok {
            cutforge_io::probe::probe(p).ok().map(|i| i.duration_ms())
        } else {
            None
        };
        entries.push(json!({
            "name": p.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default(),
            "ref": ref_s,
            "kind": kind,
            "tags": old_tags.get(&ref_s).cloned().unwrap_or_default(),
            "durationMs": duration,
            "bytes": bytes,
        }));
    }
    let doc = json!({"version": 1, "entries": entries});
    // 幂等写(内容变化才落盘,避免 mtime 抖动)
    let body = serde_json::to_string_pretty(&doc).unwrap_or_default();
    let changed = std::fs::read_to_string(&manifest_path)
        .map(|t| t != body)
        .unwrap_or(true);
    if changed && let Err(e) = cutforge_io::atomic::atomic_write(&manifest_path, body.as_bytes()) {
        warnings.push(format!("manifest 落盘失败: {e}"));
    }
    Ok((doc, warnings))
}

/// media_library 工具面:root = 素材库根(env CUTFORGE_MEDIA 缺省面的 CLI/壳共用
/// 同一解析)。action=list(缺省)扫描 + kind/tag/query 过滤;action=tag 改条目标签。
pub fn media_library_tool(root: &Path, args: &Value) -> Value {
    match args["action"].as_str().unwrap_or("list") {
        "list" => {
            let (doc, warnings) = match scan_library(root) {
                Ok(v) => v,
                Err(e) => return envelope(false, "NO_CONFIG", &e, json!({})),
            };
            let kind = args["kind"].as_str();
            let tag = args["tag"].as_str().filter(|s| !s.is_empty());
            let query = args["query"].as_str().filter(|s| !s.is_empty());
            let all = doc["entries"].as_array().cloned().unwrap_or_default();
            let entries: Vec<Value> = all
                .into_iter()
                .filter(|e| kind.is_none_or(|k| e["kind"].as_str() == Some(k)))
                .filter(|e| {
                    tag.is_none_or(|t| {
                        e["tags"]
                            .as_array()
                            .is_some_and(|ts| ts.iter().any(|v| v.as_str() == Some(t)))
                    })
                })
                .filter(|e| {
                    query.is_none_or(|q| {
                        e["name"].as_str().is_some_and(|n| n.contains(q))
                            || e["ref"].as_str().is_some_and(|r| r.contains(q))
                    })
                })
                .collect();
            let total = entries.len();
            let mut data = json!({
                "library": root.to_string_lossy(),
                "manifest": MANIFEST,
                "total": total,
                "entries": entries,
                "heuristic": true,
            });
            if !warnings.is_empty() {
                data["warnings"] = json!(warnings);
            }
            envelope(true, "OK", "素材库清单", data)
        }
        "tag" => {
            let Some(entry_ref) = args["entry"].as_str().filter(|s| !s.is_empty()) else {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    "tag 需 entry(素材库相对引用)",
                    json!({}),
                );
            };
            let tags: Vec<String> = args["tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if !root.join(entry_ref).is_file() {
                return envelope(
                    false,
                    "NO_CONFIG",
                    &format!("素材不在位: {entry_ref}"),
                    json!({}),
                );
            }
            let (mut doc, warnings) = match scan_library(root) {
                Ok(v) => v,
                Err(e) => return envelope(false, "NO_CONFIG", &e, json!({})),
            };
            let mut hit = false;
            if let Some(entries) = doc["entries"].as_array_mut() {
                for e in entries.iter_mut() {
                    if e["ref"].as_str() == Some(entry_ref) {
                        e["tags"] = json!(tags);
                        hit = true;
                    }
                }
            }
            if !hit {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    &format!("素材不在索引内(类型不受支持?): {entry_ref}"),
                    json!({}),
                );
            }
            let body = serde_json::to_string_pretty(&doc).unwrap_or_default();
            if let Err(e) = cutforge_io::atomic::atomic_write(&root.join(MANIFEST), body.as_bytes())
            {
                return envelope(
                    false,
                    "INTERNAL",
                    &format!("manifest 落盘失败: {e}"),
                    json!({}),
                );
            }
            let mut data = json!({"entry": entry_ref, "tags": tags, "manifest": MANIFEST});
            if !warnings.is_empty() {
                data["warnings"] = json!(warnings);
            }
            envelope(true, "OK", "标签已更新", data)
        }
        other => envelope(
            false,
            "PRECONDITION_FAILED",
            &format!("未知 action: {other}(允许 list/tag)"),
            json!({}),
        ),
    }
}

/// media_import 工具面:src = 绝对路径或素材库相对引用(libraryRoot 参数,
/// 缺省 default_media_root);拷入工程(布局感知落点),同名同内容幂等覆盖、
/// 同名异内容追加序号;返回工程内相对路径(可直接喂 clip_add)。
pub fn media_import_tool(root: &Path, args: &Value) -> Value {
    let Some(src) = args["src"].as_str().filter(|s| !s.is_empty()) else {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            "缺 src(绝对路径或素材库相对引用)",
            json!({}),
        );
    };
    if !cutforge_io::paths::has_project(root) {
        return envelope(
            false,
            "NO_CONFIG",
            &format!("工程不存在: {}", root.display()),
            json!({}),
        );
    }
    // 源解析:绝对路径在位优先;否则素材库相对引用(libraryRoot 缺省根)
    let (source, from) = {
        let p = PathBuf::from(src);
        if p.is_absolute() && p.is_file() {
            (p, "path")
        } else {
            let lib_root = args["libraryRoot"]
                .as_str()
                .map(PathBuf::from)
                .unwrap_or_else(default_media_root);
            let cand = lib_root.join(src);
            if cand.is_file() {
                (cand, "library")
            } else {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    &format!("素材不可达(既非在位绝对路径,也非素材库引用): {src}"),
                    json!({}),
                );
            }
        }
    };
    let Some(ext) = source.extension().and_then(|x| x.to_str()) else {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            "素材无扩展名,类型不可判",
            json!({}),
        );
    };
    let Some(kind) = kind_of(ext) else {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            &format!("不支持的素材类型: .{ext}(允许视频/音频/图片/lut/字幕)"),
            json!({}),
        );
    };
    // 落点(布局感知:册六 ADR-0021 三态)
    let dest_dir = match cutforge_io::paths::detect_layout(root) {
        cutforge_io::LayoutKind::V3 => root.join(cutforge_io::paths::V3_MEDIA),
        cutforge_io::LayoutKind::V2 => root.join(cutforge_io::paths::MATERIALS),
        cutforge_io::LayoutKind::Legacy => root.join(cutforge_io::paths::LEGACY_MATERIALS),
    };
    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        return envelope(
            false,
            "INTERNAL",
            &format!("素材目录创建失败: {e}"),
            json!({}),
        );
    }
    let file_name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "import.bin".into());
    let payload = match std::fs::read(&source) {
        Ok(b) => b,
        Err(e) => return envelope(false, "NO_CONFIG", &format!("素材不可读: {e}"), json!({})),
    };
    // 同名防混:同内容幂等覆盖;异内容追加 -2..-999(与 lut_import 同纪律)
    let rel_of = |p: &Path| -> String { fwd(p.strip_prefix(root).unwrap_or(p)) };
    let mut rel = rel_of(&dest_dir.join(&file_name));
    let mut target = dest_dir.join(&file_name);
    if target.is_file() && std::fs::read(&target).map(|b| b != payload).unwrap_or(true) {
        let stem = Path::new(&file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "import".into());
        let ext_s = ext.to_string();
        for n in 2..1000 {
            let cand = dest_dir.join(format!("{stem}-{n}.{ext_s}"));
            rel = rel_of(&cand);
            target = cand;
            if !target.is_file()
                || std::fs::read(&target)
                    .map(|b| b == payload)
                    .unwrap_or(false)
            {
                break;
            }
        }
    }
    // 原子落盘:tmp + rename(目录级唯一落盘点纪律;大文件先 tmp 后原子换名)
    let tmp = root.join(format!(
        ".cutforge/import-tmp-{}-{}",
        std::process::id(),
        file_name
    ));
    if let Some(dir) = tmp.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&tmp, &payload) {
        return envelope(false, "INTERNAL", &format!("导入暂存失败: {e}"), json!({}));
    }
    if let Err(e) = cutforge_io::atomic::rename(&tmp, &target) {
        let _ = cutforge_io::atomic::remove(&tmp);
        return envelope(false, "INTERNAL", &format!("导入落盘失败: {e}"), json!({}));
    }
    let bytes = payload.len() as u64;
    let duration = if matches!(kind, "video" | "audio") && cutforge_io::probe::ffprobe_available() {
        cutforge_io::probe::probe(&target)
            .ok()
            .map(|i| i.duration_ms())
    } else {
        None
    };
    let mut data = json!({
        "src": rel,
        "kind": kind,
        "bytes": bytes,
        "importedFrom": from,
        "note": "拷贝导入(原文件不动);src 可直接喂 clip_add/bgm_set",
    });
    if let Some(d) = duration {
        data["durationMs"] = json!(d);
    }
    envelope(true, "OK", "素材已导入", data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cf-medlib-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// kind_of 分类面:六类扩展名各自归位;未知 None。
    #[test]
    fn kind_detection() {
        assert_eq!(kind_of("mp4"), Some("video"));
        assert_eq!(kind_of("MOV"), Some("video"));
        assert_eq!(kind_of("mp3"), Some("audio"));
        assert_eq!(kind_of("png"), Some("image"));
        assert_eq!(kind_of("cube"), Some("lut"));
        assert_eq!(kind_of("ass"), Some("text"));
        assert_eq!(kind_of("exe"), None);
    }

    /// list:扫描建 manifest(类型/时长/引用/字节);tag 持久;过滤面;消失除名。
    #[test]
    fn scan_tag_filter_roundtrip() {
        let lib = tmp_dir("scan");
        std::fs::write(lib.join("bgm.mp3"), b"fakeaudio").unwrap();
        std::fs::create_dir_all(lib.join("sfx")).unwrap();
        std::fs::write(lib.join("sfx/pop.wav"), b"fakeaudio2").unwrap();
        std::fs::write(lib.join("skip.exe"), b"nope").unwrap();
        let root_s = lib.to_string_lossy().to_string();
        // list:两个可索引条目,manifest 落盘
        let resp = media_library_tool(&lib, &json!({"root": root_s}));
        assert_eq!(resp["code"], json!("OK"), "{resp}");
        assert_eq!(resp["data"]["total"], json!(2));
        assert!(lib.join(MANIFEST).is_file(), "manifest 必须落盘");
        // tag:bgm 打标;tag 过滤只回 bgm
        let resp = media_library_tool(
            &lib,
            &json!({"root": root_s, "action": "tag", "entry": "bgm.mp3", "tags": ["calm", "loop"]}),
        );
        assert_eq!(resp["code"], json!("OK"), "{resp}");
        let resp = media_library_tool(&lib, &json!({"root": root_s, "tag": "calm"}));
        assert_eq!(resp["data"]["total"], json!(1));
        assert_eq!(resp["data"]["entries"][0]["ref"], json!("bgm.mp3"));
        // 重扫:标签持久(merge 语义)
        let resp = media_library_tool(&lib, &json!({"root": root_s}));
        let bgm = resp["data"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["ref"] == json!("bgm.mp3"))
            .unwrap();
        assert_eq!(
            bgm["tags"],
            json!(["calm", "loop"]),
            "重扫标签必须保留: {bgm}"
        );
        // kind 过滤
        let resp = media_library_tool(&lib, &json!({"root": root_s, "kind": "audio"}));
        assert_eq!(resp["data"]["total"], json!(2));
        // 消失除名
        std::fs::remove_file(lib.join("bgm.mp3")).unwrap();
        let resp = media_library_tool(&lib, &json!({"root": root_s}));
        assert_eq!(resp["data"]["total"], json!(1), "消失素材必须除名: {resp}");
        std::fs::remove_dir_all(&lib).ok();
    }

    /// media_import:库引用拷入工程(v3 = media/);同名同内容幂等;同名异内容
    /// 追加序号;工程不存在 NO_CONFIG;不支持类型拒绝。
    #[test]
    fn import_copies_into_layout_dir() {
        let lib = tmp_dir("imp-lib");
        std::fs::write(lib.join("take.mp4"), b"fakevideo").unwrap();
        std::fs::write(lib.join("bad.exe"), b"nope").unwrap();
        let proj = tmp_dir("imp-proj");
        std::fs::create_dir_all(proj.join(".cutforge")).unwrap();
        let lib_s = lib.to_string_lossy().to_string();
        let proj_s = proj.to_string_lossy().to_string();
        // 工程不存在(根下没有可识别布局的 project.json)→ NO_CONFIG
        let resp = media_import_tool(
            &proj,
            &json!({"root": proj_s, "src": "take.mp4", "libraryRoot": lib_s}),
        );
        assert_eq!(resp["code"], json!("NO_CONFIG"), "空根须拒: {resp}");
        std::fs::write(proj.join("project.json"), b"{}").unwrap(); // V3 标记文件(根 project.json)
        // 不支持类型
        let resp = media_import_tool(
            &proj,
            &json!({"root": proj_s, "src": "bad.exe", "libraryRoot": lib_s}),
        );
        assert_eq!(resp["code"], json!("PRECONDITION_FAILED"), "{resp}");
        // 拷入(v3 → media/)
        let resp = media_import_tool(
            &proj,
            &json!({"root": proj_s, "src": "take.mp4", "libraryRoot": lib_s}),
        );
        assert_eq!(resp["code"], json!("OK"), "{resp}");
        assert_eq!(resp["data"]["src"], json!("media/take.mp4"), "{resp}");
        assert_eq!(resp["data"]["kind"], json!("video"));
        assert_eq!(resp["data"]["importedFrom"], json!("library"));
        assert!(proj.join("media/take.mp4").is_file(), "必须真实落盘");
        // 同名同内容 = 幂等覆盖(不追加序号)
        let resp = media_import_tool(
            &proj,
            &json!({"root": proj_s, "src": "take.mp4", "libraryRoot": lib_s}),
        );
        assert_eq!(resp["data"]["src"], json!("media/take.mp4"));
        // 同名异内容 → 序号
        std::fs::write(lib.join("take.mp4"), b"different-bytes").unwrap();
        let resp = media_import_tool(
            &proj,
            &json!({"root": proj_s, "src": "take.mp4", "libraryRoot": lib_s}),
        );
        assert_eq!(resp["data"]["src"], json!("media/take-2.mp4"), "{resp}");
        assert!(proj.join("media/take-2.mp4").is_file());
        // 绝对路径源
        let abs = lib.join("take.mp4");
        let resp = media_import_tool(
            &proj,
            &json!({"root": proj_s, "src": abs.to_string_lossy()}),
        );
        assert_eq!(resp["data"]["importedFrom"], json!("path"));
        std::fs::remove_dir_all(&lib).ok();
        std::fs::remove_dir_all(&proj).ok();
    }

    /// 落点随布局:v2 工程 → 01_原始素材/。
    #[test]
    fn import_targets_v2_materials_dir() {
        let lib = tmp_dir("imp-v2-lib");
        std::fs::write(lib.join("s.wav"), b"fake").unwrap();
        let proj = tmp_dir("imp-v2");
        std::fs::create_dir_all(proj.join("05_时间线工程")).unwrap();
        std::fs::write(proj.join("05_时间线工程/project.json"), b"{}").unwrap();
        let resp = media_import_tool(
            &proj,
            &json!({
                "root": proj.to_string_lossy(), "src": "s.wav",
                "libraryRoot": lib.to_string_lossy()
            }),
        );
        assert_eq!(resp["code"], json!("OK"), "{resp}");
        assert_eq!(resp["data"]["src"], json!("01_原始素材/s.wav"), "{resp}");
        assert!(proj.join("01_原始素材/s.wav").is_file());
        std::fs::remove_dir_all(&lib).ok();
        std::fs::remove_dir_all(&proj).ok();
    }
}
