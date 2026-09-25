// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 空工程模板(B11-1):CutForge 可独立起步,不再结构上锁死为 CutFlow 附属。
//!
//! 单一实现纪律:CLI `new` 子命令与 MCP `project_new` 工具都走本模块,
//! 不允许各自手写模板;落盘走 `atomic::atomic_write`(唯一写入路径)。

use crate::atomic;
use crate::fsutil;
use crate::PROJECT_REL;
use cutforge_core::model::Project;
use std::path::Path;

/// 允许的帧率集(与 project.schema.json 的 fps enum 同源;零漂移以 schema 校验兜底)。
pub const ALLOWED_FPS: [u32; 5] = [24, 25, 30, 50, 60];
/// 允许的画幅边长集(与 project.schema.json 的 canvas enum 同源)。
pub const ALLOWED_DIM: [u32; 3] = [1080, 1440, 1920];

/// 生成最小合法 IR(version 恒 1 + schemaVersion "2.0.0" + canvas/fps + 空轨道)。
/// 轨道按 `track_kinds` 顺序确定性生成 id(V1/A1/T1…);clips 恒为空数组。
/// 模板必须先过 v2 契约校验才允许落盘——"没有 schema 支撑的字段不存在"。
pub fn new_project_value(
    slug: &str,
    fps: u32,
    width: u32,
    height: u32,
    track_kinds: &[cutforge_core::model::TrackKind],
) -> Result<serde_json::Value, String> {
    use cutforge_core::model::TrackKind as K;
    if !ALLOWED_FPS.contains(&fps) {
        return Err(format!("fps {fps} 不在允许集 {ALLOWED_FPS:?}(契约枚举)"));
    }
    if !ALLOWED_DIM.contains(&width) || !ALLOWED_DIM.contains(&height) {
        return Err(format!("画幅 {width}x{height} 不在允许集 {ALLOWED_DIM:?}(契约枚举)"));
    }
    if slug.trim().is_empty() {
        return Err("slug(工程标识)必填".into());
    }
    let mut tracks = Vec::new();
    let mut count_v: u32 = 0;
    let mut count_a: u32 = 0;
    let mut count_t: u32 = 0;
    for kind in track_kinds {
        // 同类多轨:序号按 kind 计数递增(V1/V2…),与 Project::next_track_id 规则一致
        let (letter, c) = match kind {
            K::Video => { count_v += 1; ('V', count_v) }
            K::Audio => { count_a += 1; ('A', count_a) }
            K::Text => { count_t += 1; ('T', count_t) }
        };
        let id = format!("{letter}{c}");
        tracks.push(serde_json::json!({"id": id, "kind": match kind {
            K::Video => "video", K::Audio => "audio", K::Text => "text",
        }, "clips": []}));
    }
    let v = serde_json::json!({
        "version": 1,
        "schemaVersion": "2.0.0",
        "slug": slug.trim(),
        "fps": fps,
        "canvas": {"width": width, "height": height},
        "backends": ["ffmpeg", "cutforge"],
        "notes": "notes.json",
        "tracks": tracks,
    });
    // 契约优先:模板本身必须通过 v2 校验(防止模板与 schema 漂移)
    Project::from_value(&v).map_err(|errs| format!("模板未通过 v2 契约: {}", errs.join("; ")))?;
    Ok(v)
}

/// 在 `root` 新建空工程:写 `05_时间线工程/project.json`(0.5 中文目录契约,
/// 目录名唯一来源 `paths`;已存在则拒绝,绝不静默覆盖)。
/// 返回工程文件路径;目录创建不算文件写(同 fsutil 口径),文件本体走唯一落盘点。
pub fn scaffold_project(
    root: &Path,
    slug: &str,
    fps: u32,
    width: u32,
    height: u32,
    track_kinds: &[cutforge_core::model::TrackKind],
) -> std::io::Result<std::path::PathBuf> {
    let v = new_project_value(slug, fps, width, height, track_kinds)
        .map_err(std::io::Error::other)?;
    let project_path = root.join(PROJECT_REL);
    if project_path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("工程已存在,拒绝覆盖: {}", project_path.display()),
        ));
    }
    fsutil::ensure(&root.join(crate::paths::TIMELINE))?;
    let mut buf = serde_json::to_vec_pretty(&v)?;
    buf.push(b'\n');
    atomic::atomic_write(&project_path, &buf)?;
    Ok(project_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil;
    use cutforge_core::model::TrackKind;

    #[test]
    fn template_passes_schema_and_lands_on_disk() {
        let root = fsutil::temp_dir("scaffold-new");
        let kinds = [TrackKind::Video, TrackKind::Audio];
        let path = scaffold_project(&root, "从零剪", 30, 1080, 1920, &kinds).unwrap();
        assert!(path.is_file(), "project.json 必须落盘");
        // 落盘文件可被 Workspace 以 v2 契约打开
        let ws = crate::Workspace::open(&root).unwrap();
        assert_eq!(ws.rev(), 0);
        assert_eq!(ws.project().tracks.len(), 2);
        assert_eq!(ws.project().tracks[0].id, "V1");
        assert!(ws.project().tracks.iter().all(|t| t.clips.is_empty()));
        assert_eq!(ws.project().slug, "从零剪");
        fsutil::cleanup(&root);
    }

    #[test]
    fn refuses_overwrite_and_bad_enum() {
        let root = fsutil::temp_dir("scaffold-dup");
        let kinds = [TrackKind::Video];
        scaffold_project(&root, "a", 30, 1080, 1920, &kinds).unwrap();
        let err = scaffold_project(&root, "a", 30, 1080, 1920, &kinds).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists, "已存在必须拒绝覆盖");
        // fps/画幅不在契约枚举 → 模板即拒绝(不发盘)
        assert!(new_project_value("a", 90, 1080, 1920, &kinds).is_err());
        assert!(new_project_value("a", 30, 640, 480, &kinds).is_err());
        fsutil::cleanup(&root);
    }

    #[test]
    fn empty_tracks_is_valid_minimal_ir() {
        let v = new_project_value("最小", 25, 1920, 1080, &[]).unwrap();
        assert_eq!(v["tracks"].as_array().unwrap().len(), 0, "tracks 可为空数组");
        let p = Project::from_value(&v).unwrap();
        assert_eq!(p.next_track_id(TrackKind::Video), "V1", "空工程第一条视频轨 = V1");
    }
}
