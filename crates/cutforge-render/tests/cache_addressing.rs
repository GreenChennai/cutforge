//! AC-1.4 门禁(T1.5 缓存体系统一):
//! ① 人为改 clip 后重渲,同键必 miss、零陈旧复用(含"改下一 clip 的转场 →
//!    前一段尾帧扩展必须失效"这一旧键的实体漏洞);
//! ② cache-index.json 清单与磁盘对账(条目数、大小、命中计数);
//! ③ cache gc 遵守 LRU + 容量上限。
//! 渲染用例为真渲染(与 parity 同口径:ffmpeg 缺失即失败,不得静默跳过)。

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

fn make_media(dir: &Path) {
    // 4s 人声视频(音画齐备)
    ff(&[
        "-y", "-v", "error",
        "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30:duration=4",
        "-f", "lavfi", "-i", "sine=frequency=440:duration=4",
        "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
        "voice.mp4",
    ], dir);
}

fn write_project(dir: &Path, v: &Value) {
    // 唯一落盘点纪律(M2-4):测试夹具同样走 atomic.rs
    cutforge_io::atomic::atomic_write(
        &dir.join("05_ir/project.json"),
        serde_json::to_string_pretty(v).unwrap().as_bytes(),
    )
    .unwrap();
}

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cutforge-t15-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn base_project(slug: &str) -> Value {
    json!({
        "version": 1, "schemaVersion": "2.0.0", "slug": slug, "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "tracks": [
            {"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 2000,
                 "sourceInMs": 0, "role": "voice", "volume": 1.0},
                {"id": "V1-002", "src": "voice.mp4", "startMs": 2000, "durationMs": 2000,
                 "sourceInMs": 0, "role": "voice", "volume": 1.0}
            ]},
            {"id": "A1", "kind": "audio", "clips": []}
        ]
    })
}

fn do_render(v: Value, dir: &Path) -> cutforge_render::RenderOutcome {
    let p: cutforge_core::model::Project = serde_json::from_value(v).unwrap();
    cutforge_render::render(&p, dir, None, &mut |_| {}).unwrap()
}

fn cache_root(dir: &Path) -> PathBuf {
    dir.join(".cutforge/render-cache")
}

fn probe_duration_sec(p: &Path) -> f64 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_format"])
        .arg(p).output().expect("ffprobe 必须存在");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["format"]["duration"].as_str().unwrap().parse().unwrap()
}

#[test]
fn changing_clip_never_reuses_stale_segment() {
    assert!(ffmpeg_ok(), "AC-1.4 是硬门禁,ffmpeg 缺失即失败");
    let dir = workspace("stale");
    make_media(&dir);
    let v = base_project("t15-stale");
    write_project(&dir, &v);

    // 第一次:全新缓存 → 段全 miss
    let out1 = do_render(v.clone(), &dir);
    assert_eq!(out1.cache_misses, 2, "首次渲染两段必 miss");
    assert_eq!(out1.cache_hits, 0);
    assert_eq!(out1.segments, 2);

    // 第二次:同输入重渲 → 键稳定,段全命中(正确复用,不是陈旧复用)
    let out2 = do_render(v.clone(), &dir);
    assert_eq!(out2.cache_misses, 0, "同输入重渲不得 miss");
    assert_eq!(out2.cache_hits, 2);

    // 第三次:改 clip 1 的 punchIn(仅视觉层变化)→ 该段键必须变(必 miss);
    // clip 2 未变 → 正常命中;音频 spec 未变 → mix 正确共享
    let mut v3 = v.clone();
    v3["tracks"][0]["clips"][0]["punchIn"] = json!({"factor": 1.8, "source": "manual"});
    write_project(&dir, &v3);
    let out3 = do_render(v3, &dir);
    assert_eq!(out3.cache_misses, 1, "被改的 clip 同键必 miss(零陈旧复用)");
    assert_eq!(out3.cache_hits, 1, "未改的 clip 允许正确命中");
    assert!(out3.mix_cache_hit, "音频 spec 未变 → mix 共享(画幅/视觉无关的正确复用)");

    // 第四次(R2 的实体漏洞):只改 clip 2 的转场 → clip 1 的段虽 JSON 未变,
    // 但尾帧扩展(tpad)由 clip 2 的转场决定 → 键必须变,同键必 miss
    let mut v4 = v.clone();
    v4["tracks"][0]["clips"][1]["transition"] =
        json!({"type": "fade", "durMs": 500, "reason": "topic"});
    write_project(&dir, &v4);
    let out4 = do_render(v4, &dir);
    assert_eq!(out4.cache_misses, 2, "改转场 → 前段尾帧变化,两段都必须 miss");
    assert_eq!(out4.cache_hits, 0, "旧键若仍命中即陈旧复用(修复前 seg 键不含尾帧)");
    // 转场零时间漂移依旧成立(渲染语义不变)
    let dur = probe_duration_sec(&out4.output);
    assert!((dur - 4.0).abs() <= 1.0 / 30.0, "转场吞时长: {dur}");

    // 清单与磁盘对账:seg 层 5 个键(2 基线 + 1 punch + 1 尾帧变体 + 1 转场变体),
    // 全部条目有文件、有大小;成片输出照旧在 06_成片输出
    let idx = cutforge_render::CacheIndex::load(&cache_root(&dir));
    let seg_entries: Vec<_> = idx.entries.iter().filter(|e| e.layer == "seg").collect();
    assert_eq!(seg_entries.len(), 5, "四个渲染相位应累计 5 个不同的段键");
    for e in &seg_entries {
        assert!(cache_root(&dir).join(&e.file).is_file(), "清单条目必须有实体文件: {}", e.file);
        assert!(e.size > 0, "清单条目必须记录真实大小: {}", e.file);
    }
    assert!(idx.entries.iter().any(|e| e.layer == "mix"), "mix 层必须有条目");
    // 成片输出路径与格式不变(本夹具为 0.4.x 旧布局工程 → 06_output;新布局工程为 06_成片输出)
    assert!(out1.output.starts_with(&dir), "成片必须落在工程目录内");
    assert_eq!(out1.output.parent().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()),
        Some("06_output".to_string()), "旧布局工程成片目录契约不变");
    assert_eq!(out1.output.file_name().unwrap().to_string_lossy(),
        "final_cutforge_t15-stale_1080x1920.mp4", "成片文件名格式不变");
    assert!(out1.output.is_file());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cache_gc_respects_capacity_cap_and_lru() {
    let dir = workspace("gc");
    let root = cache_root(&dir);
    cutforge_render::cache::ensure_dirs(&root).unwrap();

    // 构造三条带索引的假缓存条目(各 100 字节;LRU 时间错开)
    let mut idx = cutforge_render::CacheIndex::default();
    let mut put = |layer: &str, key: &str, used: u64| {
        let rel = idx.record(layer, key, json!({"k": key}), used);
        let full = root.join(&rel);
        cutforge_io::atomic::atomic_write(&full, &[0u8; 100]).unwrap();
        idx.set_size(layer, key, 100);
    };
    put("seg", "old", 10);
    put("mix", "mid", 20);
    put("sub", "new", 30);
    idx.save(&root).unwrap();

    // 容量 250 < 300 → 按 LRU 删最老一条
    let report = cutforge_render::cache_gc(&root, 250, cutforge_render::cache::now_secs()).unwrap();
    assert_eq!(report.capacity_bytes, 250);
    assert_eq!(report.removed, 1, "超容必须淘汰");
    assert_eq!(report.freed_bytes, 100);
    assert_eq!(report.remaining_bytes, 200, "清理后必须 ≤ 容量上限");

    let after = cutforge_render::CacheIndex::load(&root);
    assert!(after.find("seg", "old").is_none(), "LRU 最老者被淘汰");
    assert!(after.find("mix", "mid").is_some());
    assert!(after.find("sub", "new").is_some());
    assert!(after.total_bytes() <= 250, "gc 后体积必须 ≤ 容量上限");

    // 容量内再跑一次:零删除(不误伤)
    let report2 = cutforge_render::cache_gc(&root, 250, cutforge_render::cache::now_secs()).unwrap();
    assert_eq!(report2.removed, 0);

    // info 与 gc 口径一致
    let info = cutforge_render::cache_info(&root).unwrap();
    assert_eq!(info.total_indexed_bytes, after.total_bytes());
    assert_eq!(info.layers.iter().map(|l| l.entries).sum::<usize>(), after.entries.len());

    let _ = std::fs::remove_dir_all(&dir);
}
