//! M8-3 门禁:渲染三修的对拍夹具(P0-3/P0-4 + sfx adelay)。
//! 修复前:① 缓存键不含 canvas → 9x16 先渲、16x9 直接命中前者分段(文件名说谎);
//! ② 人声只取第一个 voice 素材的完整源音频 → 多段工程音画必然错位;
//! ③ sfx 不做 adelay → 全部音效 0 秒同时炸响。
//! 三条在本文件全部作为硬断言;ffmpeg 缺失即失败(不得静默跳过)。

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

fn ffmpeg_ok() -> bool {
    Command::new("ffmpeg").arg("-version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn ff(args: &[&str], cwd: &Path) {
    let out = Command::new("ffmpeg").args(args).current_dir(cwd).output().expect("ffmpeg 必须存在");
    assert!(out.status.success(), "ffmpeg 失败: {}", String::from_utf8_lossy(&out.stderr));
}

/// 生成合成素材:带音频的测试视频 + 纯音效。
fn make_media(dir: &Path) {
    ff(&[
        "-y", "-v", "error",
        "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30:duration=3",
        "-f", "lavfi", "-i", "sine=frequency=440:duration=3",
        "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
        "voice.mp4",
    ], dir);
    ff(&[
        "-y", "-v", "error",
        "-f", "lavfi", "-i", "sine=frequency=880:duration=0.5",
        "-c:a", "libmp3lame", "sfx.mp3",
    ], dir);
}

fn write_project(dir: &Path, v: &Value) {
    std::fs::create_dir_all(dir.join("05_ir")).unwrap();
    std::fs::write(dir.join("05_ir/project.json"), serde_json::to_string_pretty(v).unwrap()).unwrap();
}

fn probe_resolution(p: &Path) -> (u32, u32) {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-select_streams", "v:0", "-show_streams"])
        .arg(p).output().expect("ffprobe 必须存在");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let w = v["streams"][0]["width"].as_u64().unwrap() as u32;
    let h = v["streams"][0]["height"].as_u64().unwrap() as u32;
    (w, h)
}

/// silencedetect:返回 (silence_start…silence_end) 区间列表(秒)。
fn silence_regions(p: &Path) -> Vec<(f64, f64)> {
    let out = Command::new("ffmpeg")
        .args(["-nostats", "-i"])
        .arg(p)
        .args(["-af", "silencedetect=noise=-45dB:d=0.3", "-f", "null", "-"])
        .output().expect("ffmpeg 必须存在");
    let err = String::from_utf8_lossy(&out.stderr);
    let mut starts: Vec<f64> = Vec::new();
    let mut regions = Vec::new();
    for line in err.lines() {
        if let Some(pos) = line.find("silence_start:") {
            if let Ok(t) = line[pos + 14..].trim().split_whitespace().next().unwrap_or("").parse::<f64>() {
                starts.push(t);
            }
        }
        if let Some(pos) = line.find("silence_end:") {
            if let Ok(t) = line[pos + 13..].trim().split_whitespace().next().unwrap_or("").parse::<f64>() {
                if let Some(s) = starts.pop() {
                    regions.push((s, t));
                }
            }
        }
    }
    regions
}

fn project_root_tag(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cutforge-m83-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn render_matrix_fixture() {
    assert!(ffmpeg_ok(), "ffmpeg 不可用:M8-3 是硬门禁,禁止跳过");

    // ---- 用例 1:双画幅互不污染(P0-3) ----
    let dir = project_root_tag("canvas");
    make_media(&dir);
    let proj = json!({
        "version": 1, "schemaVersion": "2.0.0", "slug": "m83", "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "tracks": [
            {"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 2000,
                 "sourceInMs": 0, "role": "voice", "volume": 1.0}
            ]},
            {"id": "A1", "kind": "audio", "clips": []}
        ]
    });
    write_project(&dir, &proj);
    let mut p: cutforge_core::model::Project =
        serde_json::from_value(json!({"version":1,"schemaVersion":"2.0.0","slug":"m83","fps":30,
            "canvas":{"width":1080,"height":1920},
            "tracks":[{"id":"V1","kind":"video","clips":[
                {"id":"V1-001","src":"voice.mp4","startMs":0,"durationMs":2000,"sourceInMs":0,"role":"voice","volume":1.0}]},
                {"id":"A1","kind":"audio","clips":[]}]})).unwrap();
    let out1 = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
    assert_eq!(probe_resolution(&out1.output), (1080, 1920), "9x16 变体分辨率必须为真");
    p.canvas = cutforge_core::model::Canvas { width: 1920, height: 1080 };
    let out2 = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
    assert_eq!(probe_resolution(&out2.output), (1920, 1080),
        "P0-3:16x9 变体不得命中 9x16 的分段缓存(修复前文件名 1920x1080、画面 1080x1920)");
    // 同画幅重渲:缓存命中(键稳定)
    let out3 = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
    assert!(out3.cache_hits >= 1, "同画幅重复渲染必须命中段缓存");
    let _ = (&out1, &out2, &out3);

    // ---- 用例 2:人声逐段落点(P0-4)+ 首段 1.2s 起播 ----
    let dir2 = project_root_tag("voice");
    make_media(&dir2);
    let v: Value = json!({
        "version": 1, "schemaVersion": "2.0.0", "slug": "m83v", "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "tracks": [
            {"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "voice.mp4", "startMs": 1200, "durationMs": 1800,
                 "sourceInMs": 0, "role": "voice", "volume": 1.0}
            ]},
            {"id": "A1", "kind": "audio", "clips": []}
        ]
    });
    write_project(&dir2, &v);
    let p2: cutforge_core::model::Project = serde_json::from_value(v).unwrap();
    let out = cutforge_render::render(&p2, &dir2, None, &mut |_| {}).unwrap();
    let regions = silence_regions(&out.output);
    // 成片 0–1.2s 应为静音(修复前:人声从 0 秒起播)。留 0.25s 编解码容差。
    let head_silence = regions.iter().any(|(s, e)| *s <= 0.3 && *e >= 0.95);
    assert!(head_silence, "P0-4:人声段必须按 clip.start_ms 落点(startMs=1200 → 头部静音),实际静音区: {regions:?}");

    // ---- 用例 3:sfx adelay 落点(修复前全部 0 秒炸响) ----
    let dir3 = project_root_tag("sfx");
    make_media(&dir3);
    let v: Value = json!({
        "version": 1, "schemaVersion": "2.0.0", "slug": "m83s", "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "tracks": [
            {"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 2500,
                 "sourceInMs": 0, "role": "voice", "volume": 0}
            ]},
            {"id": "A1", "kind": "audio", "clips": [
                {"id": "A1-001", "src": "sfx.mp3", "startMs": 1500, "durationMs": 500,
                 "role": "sfx", "volume": 0.9}
            ]}
        ]
    });
    write_project(&dir3, &v);
    let p3: cutforge_core::model::Project = serde_json::from_value(v).unwrap();
    let out = cutforge_render::render(&p3, &dir3, None, &mut |_| {}).unwrap();
    let regions = silence_regions(&out.output);
    let head_silence = regions.iter().any(|(s, e)| *s <= 0.3 && *e >= 1.25);
    assert!(head_silence, "sfx 必须在 startMs=1500 起播(0–1.5s 静音),实际静音区: {regions:?}");

    for d in [&dir, &dir2, &dir3] {
        let _ = std::fs::remove_dir_all(d);
    }
}
