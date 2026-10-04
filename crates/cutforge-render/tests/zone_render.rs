// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! I1-M2 zone 预渲实测夹具:区间半分辨率、内容寻址命中、守卫面、hidden 轨
//! 渲染尊重性(04 缺口④)与 CLI 子进程接线。与 parity_* 同风格:ffmpeg 实测
//! 像素/流断言,缺失即失败,禁止静默跳过。纯函数面(键/画幅/事件形状)在
//! zone.rs 内嵌单测,本文件锁**成片证据**。

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

fn ffmpeg_ok() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn ff(args: &[&str], cwd: &Path) {
    let out = Command::new("ffmpeg")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("ffmpeg 必须存在");
    assert!(
        out.status.success(),
        "ffmpeg 失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// 视频流 (宽, 高, 时长秒)。
fn video_stream(p: &Path) -> (u32, u32, f64) {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,duration",
        ])
        .arg(p)
        .output()
        .expect("ffprobe 必须存在");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let s = &v["streams"][0];
    (
        s["width"].as_u64().unwrap() as u32,
        s["height"].as_u64().unwrap() as u32,
        s["duration"].as_str().unwrap().parse().unwrap(),
    )
}

/// 整帧原始字节(rawvideo rgb24;字节序 = 行优先,行宽 = 流宽 × 3)。
fn frame_bytes(p: &Path, at_sec: f64) -> Vec<u8> {
    let out = Command::new("ffmpeg")
        .args([
            "-ss",
            &format!("{at_sec}"),
            "-i",
            p.to_str().unwrap(),
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output()
        .expect("ffmpeg 必须存在");
    assert!(
        !out.stdout.is_empty(),
        "无帧输出: {} at {at_sec}s",
        p.display()
    );
    out.stdout
}

fn pixel(f: &[u8], w: usize, x: usize, y: usize) -> (u8, u8, u8) {
    let i = (y * w + x) * 3;
    (f[i], f[i + 1], f[i + 2])
}

/// PNG 单帧 → 像素字节(rawvideo rgb24;定格噪声断言用)。
fn png_pixels(p: &Path) -> Vec<u8> {
    let out = Command::new("ffmpeg")
        .args([
            "-i",
            p.to_str().unwrap(),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output()
        .expect("ffmpeg 必须存在");
    assert!(!out.stdout.is_empty(), "PNG 解码无像素: {}", p.display());
    out.stdout
}

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cf-zone-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_project(dir: &Path, v: &Value) {
    cutforge_io::atomic::atomic_write(
        &dir.join("05_时间线工程/project.json"),
        serde_json::to_string_pretty(v).unwrap().as_bytes(),
    )
    .unwrap();
}

fn load_project(dir: &Path) -> cutforge_core::model::Project {
    let text = std::fs::read_to_string(cutforge_io::paths::project_path(dir)).unwrap();
    let mut v: Value = serde_json::from_str(&text).unwrap();
    if let Some(obj) = v.as_object_mut() {
        obj.remove("_meta");
    }
    cutforge_core::model::migrate_from_value(&v).unwrap()
}

/// 最小工程夹具:单轨单段 3s testsrc2(320x240@30;半分辨率 160x120 可整除)。
fn fixture(tag: &str) -> PathBuf {
    let dir = workspace(tag);
    ff(
        &[
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=3",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "clip.mp4",
        ],
        &dir,
    );
    write_project(
        &dir,
        &json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "zone-fixture", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "clip.mp4", "startMs": 0, "durationMs": 3000, "volume": 0},
            ]}],
        }),
    );
    dir
}

// ---- I1-M2:区间半分辨率预渲(时长/画幅/缓存命中/不落成片目录) ----

#[test]
fn zone_render_real_interval_then_cache_hit() {
    assert!(
        ffmpeg_ok(),
        "ffmpeg 必须存在(与 parity 夹具同口径:缺失即失败)"
    );
    let dir = fixture("basic");
    let project = load_project(&dir);

    let o1 =
        cutforge_render::render_zone(&project, &dir, None, 0, 2000, &mut |_| {}).expect("首渲必成");
    assert!(!o1.cached);
    assert_eq!((o1.start_ms, o1.end_ms), (0, 2000), "整百区间量化回原值");
    assert!(o1.output.is_file(), "产物必须存在: {}", o1.output.display());
    assert!(
        o1.output.starts_with(dir.join(".cutforge/preview-cache")),
        "产物必须落 preview-cache"
    );
    // 区间 = 时间窗裁剪:时长 ≈ 2s(±0.15s 编码边界容差)
    let (w, h, dur) = video_stream(&o1.output);
    assert!(
        (1.85..=2.15).contains(&dur),
        "区间 [0,2000) 产物时长应 ≈2s,实得 {dur}"
    );
    assert_eq!((w, h), (160, 120), "半分辨率:工程 320x240 → 预渲 160x120");
    // 出口重定向:成片目录绝不落 zone 产物(06_成片输出保持空)
    let out_dir = dir.join("06_成片输出");
    let leaked = std::fs::read_dir(&out_dir)
        .map(|rd| rd.flatten().count())
        .unwrap_or(0);
    assert_eq!(leaked, 0, "zone 预渲不得写 06_成片输出");

    // 同指纹同区间 → 命中不重渲(同路径同键)
    let o2 =
        cutforge_render::render_zone(&project, &dir, None, 0, 2000, &mut |_| {}).expect("二渲必成");
    assert!(o2.cached, "同指纹同区间必须命中 preview-cache");
    assert_eq!(o1.output, o2.output);
    assert_eq!(o1.key, o2.key);

    // 区间变 → 键变 → 新产物(32 命中 100ms 网格:1500 与 0/2000 不同键)
    let o3 = cutforge_render::render_zone(&project, &dir, None, 1500, 3000, &mut |_| {})
        .expect("区间 B 必成");
    assert!(!o3.cached && o3.key != o1.key, "不同区间不得复用同键产物");
    let (_, _, dur3) = video_stream(&o3.output);
    assert!(
        (1.35..=1.65).contains(&dur3),
        "[1500,3000) 窗口裁到 1.5s,实得 {dur3}"
    );

    // 改一笔工程 → 指纹变 → 同区间必 miss(绝不给陈旧 zone)
    let mut v: Value = serde_json::from_str(
        &std::fs::read_to_string(cutforge_io::paths::project_path(&dir)).unwrap(),
    )
    .unwrap();
    v["slug"] = json!("zone-fixture-edited");
    write_project(&dir, &v);
    let project2 = load_project(&dir);
    let o4 = cutforge_render::render_zone(&project2, &dir, None, 0, 2000, &mut |_| {})
        .expect("改后重渲必成");
    assert!(!o4.cached, "改一笔即 miss,不得复用陈旧 zone");
    assert_ne!(o4.key, o1.key, "指纹入键");
    std::fs::remove_dir_all(&dir).ok();
}

// ---- 守卫面(先于 ffmpeg,零管线触发) ----

#[test]
fn zone_render_guards_precondition() {
    let dir = fixture("guard");
    let project = load_project(&dir);
    // 区间倒置/零长
    let err =
        cutforge_render::render_zone(&project, &dir, None, 2000, 2000, &mut |_| {}).unwrap_err();
    assert!(err.starts_with("PRECONDITION:"), "{err}");
    let err =
        cutforge_render::render_zone(&project, &dir, None, 3000, 1000, &mut |_| {}).unwrap_err();
    assert!(err.starts_with("PRECONDITION:"), "{err}");
    // 越出内容末端:窗口内无视频片段(时间线标称 3000ms)
    let err =
        cutforge_render::render_zone(&project, &dir, None, 5000, 8000, &mut |_| {}).unwrap_err();
    assert!(err.starts_with("PRECONDITION:"), "{err}");
    assert!(err.contains("3000"), "报错须给出时间线标称时长: {err}");
    // 工程不可读 → NO_CONFIG
    let err =
        cutforge_render::render_zone(&project, Path::new("Z:/不存在"), None, 0, 2000, &mut |_| {})
            .unwrap_err();
    assert!(err.starts_with("NO_CONFIG:"), "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

// ---- 04 缺口④:track hidden 渲染尊重性(render_frame + zone 双通道实测) ----
// 整片 render 通道已由 parity_text_audio ⑧ 钉死;本夹具补单帧与 zone 两通道:
// hidden 视频轨(红场)必须被排除,合成面只见可见轨(蓝场);对照工程(无 hidden)
// 出红场,证明像素探针真看得见被隐藏的内容——断言不因探针失明而虚绿。

#[test]
fn track_hidden_is_respected_by_frame_and_zone() {
    assert!(ffmpeg_ok(), "ffmpeg 必须存在(缺失即失败)");
    let dir = workspace("hidden");
    ff(
        &[
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:size=320x240:rate=30:duration=2",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "red.mp4",
        ],
        &dir,
    );
    ff(
        &[
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:size=320x240:rate=30:duration=2",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "blue.mp4",
        ],
        &dir,
    );
    let mk = |hidden: bool| {
        json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "tr-hidden", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [
                {"id": "V1", "kind": "video", "hidden": hidden, "clips": [
                    {"id": "V1-001", "src": "red.mp4", "startMs": 0, "durationMs": 2000, "volume": 0}
                ]},
                {"id": "V2", "kind": "video", "clips": [
                    {"id": "V2-001", "src": "blue.mp4", "startMs": 0, "durationMs": 2000, "volume": 0}
                ]}
            ]
        })
    };
    // 对照:不 hidden → 红场(V1 在上,合成面见红)——探针有效性证明
    write_project(&dir, &mk(false));
    let visible = load_project(&dir);
    let ctl =
        cutforge_render::render_frame(&visible, &dir, None, 500, cutforge_render::FrameFormat::Png)
            .expect("对照帧必成");
    let (r, _g, b) = pixel(&frame_bytes(&ctl.output, 0.0), 320, 160, 120);
    assert!(
        r > 150 && b < 110,
        "对照工程应见红场(V1 未隐藏): ({r},{_g},{b})"
    );
    // hidden=true:单帧通道 → 蓝场(hidden 视觉面被尊重)
    write_project(&dir, &mk(true));
    let hidden = load_project(&dir);
    let frame =
        cutforge_render::render_frame(&hidden, &dir, None, 500, cutforge_render::FrameFormat::Png)
            .expect("hidden 帧必成");
    let (r, _g, b) = pixel(&frame_bytes(&frame.output, 0.0), 320, 160, 120);
    assert!(
        b > 150 && r < 110,
        "hidden 视频轨不进单帧合成(只见蓝): ({r},{_g},{b})"
    );
    // zone 通道 → 同样只见蓝场(窗口化不复活隐藏轨)
    let zone = cutforge_render::render_zone(&hidden, &dir, None, 0, 2000, &mut |_| {})
        .expect("hidden zone 必成");
    let (w, _h, _d) = video_stream(&zone.output);
    let (r, _g, b) = pixel(
        &frame_bytes(&zone.output, 1.0),
        w as usize,
        (w / 2) as usize,
        60,
    );
    assert!(
        b > 150 && r < 110,
        "hidden 视频轨不进 zone 预渲(只见蓝): ({r},{_g},{b})"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// ---- CLI 接线:--zone-start/--zone-end 子进程面(ZONE_OK + 完成事件) ----

#[test]
fn zone_cli_spawns_and_reports() {
    assert!(ffmpeg_ok(), "ffmpeg 必须存在(缺失即失败)");
    let dir = fixture("cli");
    let out = Command::new(env!("CARGO_BIN_EXE_cutforge-render"))
        .args([
            "--root",
            dir.to_str().unwrap(),
            "--zone-start",
            "0",
            "--zone-end",
            "2000",
        ])
        .output()
        .expect("cutforge-render 子进程必须可启动");
    assert!(
        out.status.success(),
        "zone CLI 失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // 末行 JSON 完成事件(mode=zone;契约字段齐)
    let ev = stdout
        .lines()
        .rev()
        .find_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v.get("mode").and_then(|m| m.as_str()) == Some("zone"))
        .expect("stdout 须有 mode=zone 完成事件");
    assert_eq!(ev["startMs"], json!(0));
    assert_eq!(ev["endMs"], json!(2000));
    assert_eq!(
        ev["rendererVersion"],
        json!(cutforge_render::RENDERER_VERSION)
    );
    assert_eq!(ev["canvas"], json!([160, 120]));
    assert!(stdout.contains("ZONE_OK "), "须有 ZONE_OK 行: {stdout}");
    // 旗标单只给出 → 用法错误(exit 3)
    let half = Command::new(env!("CARGO_BIN_EXE_cutforge-render"))
        .args(["--root", dir.to_str().unwrap(), "--zone-start", "0"])
        .output()
        .expect("子进程必须可启动");
    assert_eq!(half.status.code(), Some(3), "单只旗标必须用法错误退出");
    std::fs::remove_dir_all(&dir).ok();
}

// ---- I1 渲染缺口修:源窗越界 / 空素材 / 空隙+部分转场的混合工程 ----
// 复现面(接线方 C-FE2 残余):片段源窗超出素材实际长度(或空隙+部分转场使
// 合成片短于时间线标称)时,ffmpeg -ss 越合成 EOF → rc=0 无输出 → INTERNAL。
// 修复后:源耗尽段末帧定格补满、混合工程 concat 保时间域,出帧而非报错。

/// 源窗超素材长:clip 标称 3s、素材实长 1s → 段步末帧定格补满;
/// render / render_frame / render_zone 三通道全部出帧,定格区画面 = 末帧。
#[test]
fn source_window_overflow_freezes_instead_of_internal() {
    assert!(ffmpeg_ok(), "ffmpeg 必须存在(缺失即失败)");
    let dir = workspace("overflow");
    ff(
        &[
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=1",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "short.mp4",
        ],
        &dir,
    );
    write_project(
        &dir,
        &json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "overflow", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "short.mp4", "startMs": 0, "durationMs": 3000, "volume": 0}
            ]}]
        }),
    );
    let p = load_project(&dir);
    // 整片:成片时长 = 标称 3s(定格补长),不再产出短段
    let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).expect("越界工程整片必成");
    let (_w, _h, dur) = video_stream(&out.output);
    assert!(
        (2.85..=3.15).contains(&dur),
        "定格补长后成片应 ≈3s,实得 {dur}"
    );
    // 单帧:源耗尽区(1.4s > 素材实长 1s)出帧,且与定格末端同画面(末帧克隆;
    // 有损编码对静止克隆帧有逐帧量化噪声,断言按像素近似相同,非字节/PNG 大小)
    let mid =
        cutforge_render::render_frame(&p, &dir, None, 1400, cutforge_render::FrameFormat::Png)
            .expect("源耗尽区单帧必成(修复前 INTERNAL)");
    let tail =
        cutforge_render::render_frame(&p, &dir, None, 2900, cutforge_render::FrameFormat::Png)
            .expect("定格末端单帧必成");
    let a = png_pixels(&mid.output);
    let b = png_pixels(&tail.output);
    assert_eq!(a.len(), b.len(), "同画幅像素数必须一致");
    let diff = a.iter().zip(b.iter()).filter(|(x, y)| x != y).count();
    assert!(
        diff * 100 < a.len(),
        "定格区两帧应同为末帧画面(允许编码噪声),差异字节 {diff}"
    );
    // zone:同一越界工程预渲不再空产出
    let zone = cutforge_render::render_zone(&p, &dir, None, 0, 3000, &mut |_| {})
        .expect("越界工程 zone 必成");
    let (_zw, _zh, zdur) = video_stream(&zone.output);
    assert!(
        (2.85..=3.15).contains(&zdur),
        "zone 定格补长后应 ≈3s,实得 {zdur}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// 空素材(0 字节):段步提前 PRECONDITION(可读 message),不再 ffmpeg 炸 INTERNAL。
#[test]
fn empty_source_asset_is_precondition() {
    assert!(ffmpeg_ok(), "ffmpeg 必须存在(缺失即失败)");
    let dir = workspace("emptysrc");
    std::fs::write(dir.join("zero.mp4"), b"").unwrap();
    write_project(
        &dir,
        &json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "emptysrc", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "zero.mp4", "startMs": 0, "durationMs": 2000, "volume": 0}
            ]}]
        }),
    );
    let p = load_project(&dir);
    for (what, r) in [
        (
            "render_frame",
            cutforge_render::render_frame(&p, &dir, None, 500, cutforge_render::FrameFormat::Png)
                .err()
                .unwrap_or_else(|| "expected err".into()),
        ),
        (
            "render_zone",
            cutforge_render::render_zone(&p, &dir, None, 0, 2000, &mut |_| {})
                .err()
                .unwrap_or_else(|| "expected err".into()),
        ),
        (
            "render",
            cutforge_render::render(&p, &dir, None, &mut |_| {})
                .err()
                .unwrap_or_else(|| "expected err".into()),
        ),
    ] {
        assert!(
            r.starts_with("PRECONDITION:"),
            "{what} 须 PRECONDITION,实得 {r}"
        );
        assert!(r.contains("空文件"), "{what} 报错须可读(点名空文件): {r}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// 空隙 + 部分转场的混合工程(cf-demo 形态:三段、段 2-3 间转场、段 1-2 间空隙):
/// 修复前 xfade 链遇 d=0 边界丢段(合成片 13s < 段和 25.5s),后段抽帧越 EOF
/// INTERNAL;修复后 concat 保时间域,全时间线可出帧,已声明转场降级 WARN 留痕。
#[test]
fn mixed_gap_transition_project_renders_all_clips() {
    assert!(ffmpeg_ok(), "ffmpeg 必须存在(缺失即失败)");
    let dir = workspace("mixed");
    ff(
        &[
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=3",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "a.mp4",
        ],
        &dir,
    );
    ff(
        &[
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=green:size=320x240:rate=30:duration=2",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "b.mp4",
        ],
        &dir,
    );
    write_project(
        &dir,
        &json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "mixed", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 3000, "volume": 0},
                {"id": "V1-002", "src": "b.mp4", "startMs": 5000, "durationMs": 2000, "volume": 0,
                 "transition": {"type": "fade", "durMs": 500}},
                {"id": "V1-003", "src": "a.mp4", "startMs": 7000, "durationMs": 3000, "volume": 0}
            ]}]
        }),
    );
    let p = load_project(&dir);
    // 整片:concat 保时间域(空隙折叠为前段末帧定格),三段全在
    let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).expect("混合工程整片必成");
    let (_w, _h, dur) = video_stream(&out.output);
    // 段和 3+2+3=8s;V1-002 的尾帧 500ms 在 concat 下一并保留(xfade 消费不再发生)
    assert!(
        (7.8..=8.7).contains(&dur),
        "concat 后成片 ≈ 段和(8s),实得 {dur}"
    );
    // 后段抽帧(V1-003 中点 8.5s)必须出帧——修复前越合成 EOF INTERNAL
    let frame =
        cutforge_render::render_frame(&p, &dir, None, 8500, cutforge_render::FrameFormat::Png)
            .expect("后段单帧必成(修复前 INTERNAL)");
    assert_eq!(
        &std::fs::read(&frame.output).unwrap()[..8],
        b"\x89PNG\r\n\x1a\n"
    );
    // zone 同面
    let zone = cutforge_render::render_zone(&p, &dir, None, 7000, 10000, &mut |_| {})
        .expect("后段 zone 必成");
    let (_zw, _zh, zdur) = video_stream(&zone.output);
    assert!(
        (2.85..=3.15).contains(&zdur),
        "zone [7000,10000) 应 ≈3s,实得 {zdur}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// ---- I1 缺口修:mix 步音轨可用性(纯视频素材 [N:a] 零匹配炸图) ----

/// 音频窗口 RMS(dB;af 可给任意前置滤镜)。
fn rms_of_window(p: &Path, from: f64, to: f64, af: &str) -> f64 {
    let chain = if af.is_empty() {
        format!("atrim={from}:{to},astats=metadata=1")
    } else {
        format!("{af},atrim={from}:{to},astats=metadata=1")
    };
    let out = Command::new("ffmpeg")
        .args(["-i", p.to_str().unwrap(), "-af", &chain, "-f", "null", "-"])
        .output()
        .expect("ffmpeg 必须存在");
    let err = String::from_utf8_lossy(&out.stderr);
    let Some(pos) = err.rfind("RMS level dB:") else {
        panic!("astats 未输出 RMS level dB(窗口无样本?): {err}");
    };
    err[pos + 13..]
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

/// 纯视频时间线(clip 音量缺省 = 自然音量 → mix 有事件但素材无音轨):
/// zone / 整片 / 单帧三出口全部出产物;成片含静音音轨,时长 = 标称。
/// 修复前三出口全在 mix 步炸 `[0:a] matches no streams`。
#[test]
fn silent_video_only_pipeline_renders_all_exits() {
    assert!(ffmpeg_ok(), "ffmpeg 必须存在(缺失即失败)");
    let dir = workspace("silentmix");
    ff(
        &[
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=2",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "novoice.mp4",
        ],
        &dir,
    );
    // 素材确认纯视频(无音轨)
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_entries",
            "stream=codec_type",
            "novoice.mp4",
        ])
        .output()
        .expect("ffprobe 必须存在");
    assert!(
        !String::from_utf8_lossy(&probe.stdout).contains("audio"),
        "夹具素材必须无音轨"
    );
    write_project(
        &dir,
        &json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "silentmix", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "novoice.mp4", "startMs": 0, "durationMs": 2000}
            ]}]
        }),
    );
    let p = load_project(&dir);
    // zone
    let zone = cutforge_render::render_zone(&p, &dir, None, 0, 2000, &mut |_| {})
        .expect("纯视频 zone 必成(修复前 mix 步炸)");
    assert!(zone.output.is_file());
    // 整片
    let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).expect("纯视频整片必成");
    let (_w, _h, dur) = video_stream(&out.output);
    assert!((1.85..=2.15).contains(&dur), "整片时长 ≈2s,实得 {dur}");
    // 单帧
    let frame =
        cutforge_render::render_frame(&p, &dir, None, 1000, cutforge_render::FrameFormat::Png)
            .expect("纯视频单帧必成");
    assert_eq!(
        &std::fs::read(&frame.output).unwrap()[..8],
        b"\x89PNG\r\n\x1a\n"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// 混合音轨:有声素材与纯视频素材混排(事件段一个有音轨一个无)——
/// amix 图只含可用输入(修复前 [1:a] 零匹配炸图),成片时长 = 标称,
/// 有声段频段能量在、纯视频段近静音。
#[test]
fn mixed_audio_availability_amix_graph_is_correct() {
    assert!(ffmpeg_ok(), "ffmpeg 必须存在(缺失即失败)");
    let dir = workspace("mixedaud");
    ff(
        &[
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-c:a",
            "aac",
            "-shortest",
            "voice.mp4",
        ],
        &dir,
    );
    ff(
        &[
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=2",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "mute.mp4",
        ],
        &dir,
    );
    write_project(
        &dir,
        &json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "mixedaud", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 2000},
                {"id": "V1-002", "src": "mute.mp4", "startMs": 2000, "durationMs": 2000}
            ]}]
        }),
    );
    let p = load_project(&dir);
    let out = cutforge_render::render(&p, &dir, None, &mut |_| {})
        .expect("混合音轨整片必成(修复前 [1:a] 零匹配炸图)");
    let (_w, _h, dur) = video_stream(&out.output);
    assert!((3.85..=4.15).contains(&dur), "成片时长 ≈4s,实得 {dur}");
    // 响度抽查:有声段 440Hz 能量在;纯视频段(2.0s 起)近静音
    let voiced = rms_of_window(&out.output, 0.5, 1.5, "bandpass=f=440:w=60");
    let silent = rms_of_window(&out.output, 2.5, 3.5, "");
    assert!(voiced > -60.0, "有声段 440Hz 能量应在: {voiced} dB");
    assert!(
        silent < voiced - 20.0,
        "纯视频段应近静音: voiced={voiced} silent={silent}"
    );
    // zone 同面(窗口含无音轨段)
    let zone = cutforge_render::render_zone(&p, &dir, None, 1000, 3000, &mut |_| {})
        .expect("混合音轨 zone 必成");
    assert!(zone.output.is_file());
    std::fs::remove_dir_all(&dir).ok();
}
