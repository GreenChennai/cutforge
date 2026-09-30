//! M11-1 门禁:能力矩阵从纸面到实码——每项能力一个合成用例,ffmpeg 实测断言。
//! 覆盖:转场(零漂移)/变速/punch-in/overlay/BGM ducking/afade/文件名消毒/
//! 文本轨不消费/多画幅真分叉(mix 共享);册四 A4-BE2 增五项:speedCurve 曲线
//! (时长对拍=分段积分)/reverse 倒放(短片,首末帧对调)/rotation 90°(朝向互换)/
//! crop(源域裁剪)/flip(镜像)。ffmpeg 缺失即失败,不得静默跳过。
//! 证据回填 docs/capability-matrix.json(status/evidence,M11-1)。

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

fn ffmpeg_ok() -> bool {
    Command::new("ffmpeg").arg("-version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn ff(args: &[&str], cwd: &Path) -> String {
    let out = Command::new("ffmpeg").args(args).current_dir(cwd).output().expect("ffmpeg 必须存在");
    assert!(out.status.success(), "ffmpeg 失败: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn ff_out(args: &[&str], cwd: &Path) -> (String, String) {
    let out = Command::new("ffmpeg").args(args).current_dir(cwd).output().expect("ffmpeg 必须存在");
    assert!(out.status.success(), "ffmpeg 失败: {}", String::from_utf8_lossy(&out.stderr));
    (String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
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
    // 2s BGM
    ff(&[
        "-y", "-v", "error",
        "-f", "lavfi", "-i", "sine=frequency=220:duration=2",
        "-c:a", "libmp3lame", "bgm.mp3",
    ], dir);
    // 红色方块 logo
    ff(&[
        "-y", "-v", "error",
        "-f", "lavfi", "-i", "color=c=red:size=60x60",
        "-frames:v", "1", "logo.png",
    ], dir);
}

fn probe_duration_sec(p: &Path) -> f64 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_format"])
        .arg(p).output().expect("ffprobe 必须存在");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["format"]["duration"].as_str().unwrap().parse().unwrap()
}

/// **视频流**时长(册四 BE3a:容器时长 = max(音,画),会被音频 mux 撑大而说谎——
/// 转场截断虫曾借此逃过夹具①;零漂移断言一律以视频流为准)。
fn video_stream_duration_sec(p: &Path) -> f64 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-print_format", "json",
               "-show_entries", "stream=duration"])
        .arg(p).output().expect("ffprobe 必须存在");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["streams"][0]["duration"].as_str().unwrap().parse().unwrap()
}

/// **音频流**时长(acrossfade 链零漂移断言用)。
fn audio_stream_duration_sec(p: &Path) -> f64 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a:0", "-print_format", "json",
               "-show_entries", "stream=duration"])
        .arg(p).output().expect("ffprobe 必须存在");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["streams"][0]["duration"].as_str().unwrap().parse().unwrap()
}

/// 视频流帧数。
fn video_frame_count(p: &Path) -> u64 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-print_format", "json",
               "-show_entries", "stream=nb_frames"])
        .arg(p).output().expect("ffprobe 必须存在");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["streams"][0]["nb_frames"].as_str().unwrap().parse().unwrap()
}

/// 整帧原始字节(rawvideo rgb24;帧级差异断言用)
fn frame_bytes(p: &Path, at_sec: f64) -> Vec<u8> {
    let out = Command::new("ffmpeg").args([
        "-ss", &format!("{at_sec}"), "-i", p.to_str().unwrap(),
        "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-",
    ]).current_dir(Path::new(".")).output().expect("ffmpeg 必须存在");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!out.stdout.is_empty(), "无帧输出: {} at {at_sec}s", p.display());
    out.stdout
}

/// 音频窗口 RMS(dB)
fn rms_of_window(p: &Path, from: f64, to: f64) -> f64 {
    let (_o, err) = ff_out(&[
        "-i", p.to_str().unwrap(),
        "-af", &format!("atrim={from}:{to},astats=metadata=1"), "-f", "null", "-",
    ], Path::new("."));
    let pos = err.rfind("RMS level dB:").expect(err.as_str());
    err[pos + 13..].split_whitespace().next().unwrap().parse().unwrap()
}

fn write_project(dir: &Path, slug: &str, v: &Value) -> PathBuf {
    let _ = slug; // slug 已在 v 内;参数仅为可读性
    // 唯一落盘点纪律(M2-4):测试夹具同样走 atomic.rs(临时文件+rename 原子替换,父目录自建)
    let p = dir.join("05_ir/project.json");
    cutforge_io::atomic::atomic_write(&p, serde_json::to_string_pretty(v).unwrap().as_bytes()).unwrap();
    p
}

fn parse_project(v: Value) -> cutforge_core::model::Project {
    serde_json::from_value(v).unwrap()
}

fn base_video_clips() -> Value {
    json!([
        {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 2000,
         "sourceInMs": 0, "role": "voice", "volume": 1.0},
        {"id": "V1-002", "src": "voice.mp4", "startMs": 2000, "durationMs": 2000,
         "sourceInMs": 0, "role": "voice", "volume": 1.0}
    ])
}

fn base_project(slug: &str) -> Value {
    json!({
        "version": 1, "schemaVersion": "2.0.0", "slug": slug, "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "tracks": [
            {"id": "V1", "kind": "video", "clips": base_video_clips()},
            {"id": "A1", "kind": "audio", "clips": []}
        ]
    })
}

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cutforge-m11-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------- 册四 A4-BE2 夹具素材与采样工具 ----------------

/// rgb24 整帧上取像素(坐标系 = 画布 1080x1920)。
fn px(frame: &[u8], x: usize, y: usize, w: usize) -> (u8, u8, u8) {
    let i = (y * w + x) * 3;
    (frame[i], frame[i + 1], frame[i + 2])
}

fn is_reddish(p: (u8, u8, u8)) -> bool {
    p.0 > 150 && p.1 < 110 && p.2 < 110
}

fn is_blueish(p: (u8, u8, u8)) -> bool {
    p.2 > 150 && p.0 < 110 && p.1 < 110
}

/// 两色分段源:前段 c0(时长 d0 秒)+ 后段 c1(d1 秒),320x240@30 带 440Hz 人声。
/// 用于 speedCurve(色变时刻 = 分段积分的可观测锚点)与 reverse(首末帧对调)。
fn make_two_tone(dir: &Path, name: &str, c0: &str, c1: &str, d0: f64, d1: f64) {
    ff(&[
        "-y", "-v", "error",
        "-f", "lavfi", "-i", &format!("color=c={c0}:size=320x240:rate=30:duration={d0}"),
        "-f", "lavfi", "-i", &format!("color=c={c1}:size=320x240:rate=30:duration={d1}"),
        "-f", "lavfi", "-i", &format!("sine=frequency=440:duration={}", d0 + d1),
        "-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0[v]",
        "-map", "[v]", "-map", "2:a",
        "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest", name,
    ], dir);
}

/// 左红右蓝源(hstack):320x240,左半 red 右半 blue,带人声。
/// 用于 rotation(转置后红上蓝下)/crop(裁右半全蓝)/flip(镜像后红右蓝左)。
fn make_left_red_right_blue(dir: &Path, name: &str, dur: f64) {
    ff(&[
        "-y", "-v", "error",
        "-f", "lavfi", "-i", &format!("color=c=red:size=160x240:rate=30:duration={dur}"),
        "-f", "lavfi", "-i", &format!("color=c=blue:size=160x240:rate=30:duration={dur}"),
        "-f", "lavfi", "-i", &format!("sine=frequency=440:duration={dur}"),
        "-filter_complex", "[0:v][1:v]hstack=inputs=2[v]",
        "-map", "[v]", "-map", "2:a",
        "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest", name,
    ], dir);
}

#[test]
fn parity_matrix_full() {
    assert!(ffmpeg_ok(), "ffmpeg 不可用:M11-1 是硬门禁,禁止跳过");
    const FRAME: f64 = 1.0 / 30.0;
    let mut achieved: Vec<&str> = Vec::new();

    // ---- ① 转场 xfade:尾帧扩展零时间漂移(ADR-0023 口径) ----
    {
        let dir = workspace("xfade");
        make_media(&dir);
        let mut v = base_project("m11-xfade");
        v["tracks"][0]["clips"][1]["transition"] =
            json!({"type": "fade", "durMs": 500, "reason": "topic"});
        write_project(&dir, "m11-xfade", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let dur = probe_duration_sec(&out.output);
        // 整链总时长必须保持 sum(dur)=4.0s(转场不吞时长,尾帧扩展找平)
        assert!((dur - 4.0).abs() <= FRAME, "转场吞时长: {dur}");
        // 视频流时长与帧数(册四 BE3a 修复的锁:offset=名义时长;旧实现 offset 含
        // 尾帧 → 后段整段被截,容器时长被音频撑大而假绿)
        let vdur = video_stream_duration_sec(&out.output);
        assert!((vdur - 4.0).abs() <= FRAME, "视频流时长漂移: {vdur}");
        assert_eq!(video_frame_count(&out.output), 120, "视频流帧数 = Σdur×fps");
        achieved.push("5 转场:xfade 链 + 尾帧扩展零漂移(容器与视频流双 4.000s,120 帧)");
    }

    // ---- ② 变速 setpts/atempo:2x 速度 → 段输出减半,音画同步 ----
    {
        let dir = workspace("speed");
        make_media(&dir);
        let mut v = base_project("m11-speed");
        v["tracks"][0]["clips"] = json!([
            {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 2000,
             "sourceInMs": 0, "role": "voice", "volume": 1.0, "speed": 2.0},
            {"id": "V1-002", "src": "voice.mp4", "startMs": 2000, "durationMs": 2000,
             "sourceInMs": 0, "role": "voice", "volume": 1.0, "speed": 2.0}
        ]);
        write_project(&dir, "m11-speed", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let dur = probe_duration_sec(&out.output);
        assert!((dur - 4.0).abs() <= 0.1, "2x 变速后总时长应仍为 4s(每段消费 4s 源): {dur}");
        achieved.push("2 变速:setpts/atempo 消费(2x → 时长保持语义)");
    }

    // ---- ③ punch-in:紧构图 → 中心画面与无 punch 显著不同 ----
    {
        let dir = workspace("punch");
        make_media(&dir);
        let mut v = base_project("m11-punch");
        v["tracks"][0]["clips"][0]["punchIn"] = json!({"factor": 1.6, "source": "manual"});
        write_project(&dir, "m11-punch", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let f_punch = frame_bytes(&out.output, 0.5);
        // 基准(无 punch)
        let v2 = base_project("m11-punch-base");
        write_project(&dir, "m11-punch-base", &v2);
        let p2 = parse_project(v2);
        let out2 = cutforge_render::render(&p2, &dir, None, &mut |_| {}).unwrap();
        let f_base = frame_bytes(&out2.output, 0.5);
        assert_ne!(f_punch, f_base, "punch-in 紧构图画面必须改变");
        achieved.push("12 punch-in:中心紧构图(中心 YAVG 改变)");
    }

    // ---- ④ overlay 合成:红色 logo 落点画面改变 ----
    {
        let dir = workspace("overlay");
        make_media(&dir);
        let mut v = base_project("m11-overlay");
        // overlay 叠加层走独立变体轨(rs_brand 口径):不占主时间线
        v["tracks"].as_array_mut().unwrap().push(json!(
            {"id": "V2", "kind": "video", "clips": [
                {"id": "V2-001", "src": "logo.png", "startMs": 0, "durationMs": 2000,
                 "volume": 0, "overlay": {"x": 40, "y": 40, "w": 60, "h": 60, "opacity": 1.0}}
            ]}
        ));
        write_project(&dir, "m11-overlay", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let f_logo = frame_bytes(&out.output, 0.5);
        let f_after = frame_bytes(&out.output, 2.5); // overlay 时间窗外(clip 2s 后)
        assert_ne!(f_logo, f_after, "overlay 时间窗未生效");
        achieved.push("4 位置/缩放/旋转:overlay 绝对像素落点 + 时间窗(旋转自 A4-BE2 起入契约,见旋转夹具)");
    }

    // ---- ⑤ BGM ducking:人声区间 BGM 被压(duck on/off 能量差) ----
    {
        let dir = workspace("duck");
        make_media(&dir);
        let mk = |ducking: bool, slug: &str| {
            let mut v = json!({
                "version": 1, "schemaVersion": "2.0.0", "slug": slug, "fps": 30,
                "canvas": {"width": 1080, "height": 1920},
                "bgm": {"src": "bgm.mp3", "gainDb": -6, "ducking": ducking, "loop": true},
                "tracks": [
                    {"id": "V1", "kind": "video", "clips": [
                        {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 2000,
                         "sourceInMs": 0, "role": "voice", "volume": 0.0}
                    ]},
                    {"id": "A1", "kind": "audio", "clips": [
                        {"id": "A1-001", "src": "voice.mp4", "startMs": 500, "durationMs": 1500,
                         "sourceInMs": 0, "role": "voice", "volume": 1.0}
                    ]}
                ]
            });
            write_project(&dir, slug, &v);
            let _ = &mut v;
            parse_project(v)
        };
        let p_on = mk(true, "m11-duck-on");
        let out_on = cutforge_render::render(&p_on, &dir, None, &mut |_| {}).unwrap();
        let p_off = mk(false, "m11-duck-off");
        let out_off = cutforge_render::render(&p_off, &dir, None, &mut |_| {}).unwrap();
        // 全频段被人声主导;用带通隔离 BGM 频段(220Hz)测压制量(实测 ≈7dB)
        let bgm_band_rms = |p: &Path| {
            let (_o, err) = ff_out(&[
                "-i", p.to_str().unwrap(),
                "-af", "bandpass=f=220:w=60,atrim=0.8:1.6,astats=metadata=1", "-f", "null", "-",
            ], Path::new("."));
            let pos = err.rfind("RMS level dB:").expect(err.as_str());
            err[pos + 13..].split_whitespace().next().unwrap().parse().unwrap()
        };
        let rms_on: f64 = bgm_band_rms(&out_on.output);
        let rms_off: f64 = bgm_band_rms(&out_off.output);
        assert!(rms_on < rms_off - 3.0,
            "ducking 开启时 BGM 频段应被压制 ≥3dB: on={rms_on} off={rms_off}");
        achieved.push("9 BGM ducking:sidechain 侧链(on/off 能量差可测)");
    }

    // ---- ⑥ afade:淡入 800ms → 头部静音区间延长 ----
    {
        let dir = workspace("afade");
        make_media(&dir);
        let mut v = base_project("m11-afade");
        v["tracks"][0]["clips"] = json!([
            {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 3000,
             "sourceInMs": 0, "role": "voice", "volume": 1.0, "fade": {"inMs": 800, "outMs": 0}}
        ]);
        write_project(&dir, "m11-afade", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        // 淡入不是静音:断言头部 200ms RMS 显著低于稳态区间(线性淡入 800ms)
        let rms_head = rms_of_window(&out.output, 0.02, 0.2);
        let rms_mid = rms_of_window(&out.output, 1.5, 2.5);
        assert!(rms_head < rms_mid - 6.0,
            "afade in 800ms:头部应显著低于稳态(≥6dB): head={rms_head} mid={rms_mid}");
        achieved.push("3 音量/淡入淡出:afade 消费(音量 M8 已生效)");
    }

    // ---- ⑦ 文件名消毒 + ⑧ 文本轨不消费(与 CutFlow 同口径) ----
    {
        let dir = workspace("sanit");
        make_media(&dir);
        let mut v = base_project("bad*slug:<>");
        v["tracks"].as_array_mut().unwrap().push(json!(
            {"id": "T1", "kind": "text", "clips": [
                {"id": "T1-001", "startMs": 0, "durationMs": 1000, "text": "字幕走 ASS 链"}
            ]}
        ));
        write_project(&dir, "bad*slug:<>", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        assert!(out.output.is_file(), "消毒后文件名必须可用: {}", out.output.display());
        assert!(!out.output.to_string_lossy().contains('*'), "文件名不得含敌对字符");
        achieved.push("14 多画幅/文件名消毒:敌对字符替换为 _");
        achieved.push("13 文本轨:文本片段经 textass 生成 ASS 烧录可见(册四 BE3b;像素证据见 parity_text_audio ①)");
    }

    // ---- ⑨ 多画幅真分叉(T1.10):画幅无关层(mix)全部仅一份;video/encode 层按画幅分叉 ----
    {
        let dir = workspace("fork");
        make_media(&dir);
        let v = base_project("m11-fork");
        write_project(&dir, "m11-fork", &v);
        let p = parse_project(v);
        let outs = cutforge_render::render_variants(&p, &dir, &["9x16", "16x9"]);
        for (r, res) in &outs {
            assert!(res.is_ok(), "变体 {r} 失败");
        }
        // T1.5 分层缓存:seg/mix/compose/overlay/sub 各自目录内数产物
        let count_layer = |layer: &str| -> usize {
            std::fs::read_dir(dir.join(".cutforge/render-cache").join(layer))
                .expect("缓存分层目录必须存在")
                .flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.ends_with(".mp4") || n.ends_with(".m4a")
                })
                .count()
        };
        // 画幅无关层:mix(纯音频)两变体共享 → 全局仅一份(真分叉的核心判据)
        assert_eq!(count_layer("mix"), 1, "两个画幅必须共享同一份 mix(画幅无关层仅一份)");
        // 画幅相关层:video/encode 真分叉(compose/sub 每画幅一份;seg = 2 clip × 2 画幅)
        assert_eq!(count_layer("compose"), 2, "compose 必须按画幅分叉");
        assert_eq!(count_layer("sub"), 2, "sub 合流必须按画幅分叉");
        assert_eq!(count_layer("seg"), 4, "seg = 2 clip × 2 画幅(键含 canvas)");
        assert_eq!(count_layer("overlay"), 0, "无叠加层 → overlay 层零产物");
        // 两个成片都产出且路径不同(encode 层分叉)
        let output0 = outs[0].1.as_ref().unwrap();
        let output1 = outs[1].1.as_ref().unwrap();
        assert_ne!(output0, output1, "两画幅成片路径必须不同");
        achieved.push("14 多画幅真分叉:画幅无关层(mix)仅一份,video/encode 层按画幅分叉");
    }

    // ---- ⑩ speedCurve 曲线变速(册四 T4.4):时长对拍 = 分段积分,两段曲线实渲 ----
    {
        let dir = workspace("curve");
        // 源:红 0–2.5s + 蓝 2.5–4.0s(色变时刻是分段积分的可观测锚点)
        make_two_tone(&dir, "curve_src.mp4", "red", "blue", 2.5, 1.5);
        // 曲线 [0,1s)@1.0 + [1s,2s]@3.0(区间均值渲染 → 段 2 常速 2.0):
        // 源消耗 = 1000×1.0 + 1000×2.0 = 3000ms(分段积分);蓝变在源 2500ms
        // → 时间线 t = 1000 + (2500−1000)/2 = 1750ms。恒速 1x 假解下 1.9s 仍是红。
        let mut v = base_project("m11-curve");
        v["tracks"][0]["clips"] = json!([
            {"id": "V1-001", "src": "curve_src.mp4", "startMs": 0, "durationMs": 2000,
             "sourceInMs": 0, "role": "voice", "volume": 1.0,
             "speedCurve": [
                {"atMs": 0, "speed": 1.0}, {"atMs": 1000, "speed": 1.0},
                {"atMs": 2000, "speed": 3.0}
             ]}
        ]);
        write_project(&dir, "m11-curve", &v);
        let p = parse_project(v);
        // 投影口径:endMs = startMs+durationMs = 2000;渲染时长必须与之一致(红线)
        assert_eq!(
            p.tracks[0].clips[0].start_ms + p.tracks[0].clips[0].duration_ms, 2000,
            "内核 endMs 口径"
        );
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let dur = probe_duration_sec(&out.output);
        assert!((dur - 2.0).abs() <= 0.1, "曲线段总时长必须 = durationMs(分段拼接): {dur}");
        let f_early = frame_bytes(&out.output, 0.5);
        let f_late = frame_bytes(&out.output, 1.9);
        let w = 1080usize; // base_project 画布宽(画幅归一后帧宽)
        assert!(is_reddish(px(&f_early, 540, 960, w)), "0.5s 应为红(段 1 @1.0x)");
        assert!(is_blueish(px(&f_late, 540, 960, w)), "1.9s 应为蓝(段 2 @2.0x 已越过源 2.5s 分界;恒速假解下仍是红)");
        achieved.push("16 曲线变速 speedCurve:时长对拍=分段积分(两段曲线,色变时刻落在 1.75s 预测点)");
    }

    // ---- ⑪ reverse 倒放(册四 T4.4):短片首末帧对调(reverse 整段缓冲,夹具用短片段) ----
    {
        let dir = workspace("reverse");
        // 短片 1.2s(内存约束口径:schema/渲染链注释均声明长素材先切短再倒)
        make_two_tone(&dir, "rev_src.mp4", "red", "blue", 0.6, 0.6);
        let mut v = base_project("m11-reverse");
        v["tracks"][0]["clips"] = json!([
            {"id": "V1-001", "src": "rev_src.mp4", "startMs": 0, "durationMs": 1200,
             "sourceInMs": 0, "role": "voice", "volume": 1.0, "reverse": true}
        ]);
        write_project(&dir, "m11-reverse", &v);
        let p = parse_project(v);
        // 音频链同步锁:areverse 先于 atempo(mix 步;视频 reverse 先于变速由 ⑩链序单测锁定)
        let plan = cutforge_render::RenderPlan::build(&p, &dir, None);
        let mix_args = cutforge_render::steps::mix_pass_a_args(&plan, Path::new("x.m4a"));
        assert!(
            mix_args.iter().any(|a| a.contains("areverse,asetpts=N/SR/TB")),
            "倒放音频必须走 areverse+asetpts 链"
        );
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let dur = probe_duration_sec(&out.output);
        assert!((dur - 1.2).abs() <= 0.1, "倒放不改时长: {dur}");
        let w = 1080usize; // base_project 画布宽(画幅归一后帧宽)
        // 首末帧对调:时间线开头显示源结尾(蓝),结尾显示源开头(红)
        let f_head = frame_bytes(&out.output, 0.1);
        let f_tail = frame_bytes(&out.output, 1.0);
        assert!(is_blueish(px(&f_head, 540, 960, w)), "倒放后 0.1s 应为源尾(蓝)");
        assert!(is_reddish(px(&f_tail, 540, 960, w)), "倒放后 1.0s 应为源头(红)");
        achieved.push("17 倒放 reverse:首末帧对调(视频 reverse+PTS 重盖;音频 areverse 链同步锁定)");
    }

    // ---- ⑫ rotation 90°(册四 T4.9):transpose 精确宽高互换,朝向像素断言 ----
    {
        let dir = workspace("rotate");
        make_left_red_right_blue(&dir, "lr_src.mp4", 1.5);
        let mut v = base_project("m11-rotate");
        v["tracks"][0]["clips"] = json!([
            {"id": "V1-001", "src": "lr_src.mp4", "startMs": 0, "durationMs": 1500,
             "sourceInMs": 0, "role": "voice", "volume": 1.0, "rotation": 90.0}
        ]);
        write_project(&dir, "m11-rotate", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let w = 1080usize; // base_project 画布宽(画幅归一后帧宽)
        // 源左红右蓝;90° 顺时针(transpose=1)后红在上、蓝在下。
        // 320x240 旋转成 240x320 → fit 1080x1920 → 内容 1080x1440,y ∈ [240,1680)。
        let f = frame_bytes(&out.output, 0.5);
        assert!(is_reddish(px(&f, 540, 400, w)), "90° 顺时针:顶部应为红(源左侧),实得 {:?}", px(&f, 540, 400, w));
        assert!(is_blueish(px(&f, 540, 1500, w)), "90° 顺时针:底部应为蓝(源右侧),实得 {:?}", px(&f, 540, 1500, w));
        achieved.push("4/18 旋转 rotation:90° transpose 宽高互换(左红右蓝→红上蓝下,朝向像素断言)");
    }

    // ---- ⑬ crop 源域裁剪(册四 T4.9):右半矩形裁剪 → 画面全蓝 ----
    {
        let dir = workspace("crop");
        make_left_red_right_blue(&dir, "lr_src.mp4", 1.5);
        let mut v = base_project("m11-crop");
        v["tracks"][0]["clips"] = json!([
            {"id": "V1-001", "src": "lr_src.mp4", "startMs": 0, "durationMs": 1500,
             "sourceInMs": 0, "role": "voice", "volume": 1.0,
             "crop": {"x": 160, "y": 0, "w": 160, "h": 240}}
        ]);
        write_project(&dir, "m11-crop", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let w = 1080usize; // base_project 画布宽(画幅归一后帧宽)
        // 裁掉左半(红)后仅剩右半(蓝),缩放铺满画幅宽度
        let f = frame_bytes(&out.output, 0.5);
        for (x, y) in [(200usize, 700usize), (540, 960), (880, 1200)] {
            assert!(is_blueish(px(&f, x, y, w)), "crop 右半后 ({x},{y}) 应为蓝,实得 {:?}", px(&f, x, y, w));
        }
        achieved.push("19 裁剪 crop:源域像素矩形(裁左留右,红被裁出画外)");
    }

    // ---- ⑭ flip 水平镜像(册四 T4.9):左红右蓝 → 红右蓝左 ----
    {
        let dir = workspace("flip");
        make_left_red_right_blue(&dir, "lr_src.mp4", 1.5);
        let mut v = base_project("m11-flip");
        v["tracks"][0]["clips"] = json!([
            {"id": "V1-001", "src": "lr_src.mp4", "startMs": 0, "durationMs": 1500,
             "sourceInMs": 0, "role": "voice", "volume": 1.0, "flip": "h"}
        ]);
        write_project(&dir, "m11-flip", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let w = 1080usize; // base_project 画布宽(画幅归一后帧宽)
        let f = frame_bytes(&out.output, 0.5);
        // hflip 后:左侧=源右半(蓝),右侧=源左半(红)
        assert!(is_blueish(px(&f, 200, 960, w)), "hflip 后左侧应为蓝");
        assert!(is_reddish(px(&f, 880, 960, w)), "hflip 后右侧应为红,实得 {:?}", px(&f, 880, 960, w));
        achieved.push("20 翻转 flip:水平镜像(hflip,左右色块对调)");
    }

    // ---- ⑮ 转场目录每类 ≥2 实渲(册四 T4.5 / AC-4.3):目录直通 + 视频流零漂移 ----
    {
        let dir = workspace("cats");
        make_media(&dir);
        // 小画幅夹具(时长断言与画幅无关;渲染量收敛)
        let small = |slug: &str, tr: &str| {
            let v = json!({
                "version": 1, "schemaVersion": "2.0.0", "slug": slug, "fps": 30,
                "canvas": {"width": 320, "height": 240},
                "tracks": [{"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 2000,
                     "sourceInMs": 0, "role": "voice", "volume": 1.0},
                    {"id": "V1-002", "src": "voice.mp4", "startMs": 2000, "durationMs": 2000,
                     "sourceInMs": 0, "role": "voice", "volume": 1.0,
                     "transition": {"type": "fade", "fx": tr, "durMs": 400}}
                ]}]
            });
            write_project(&dir, slug, &v);
            parse_project(v)
        };
        // 每分类 2 项,全部经 fx=tr.<id> 直通(目录路径而非旧枚举)
        let sweep: Vec<(&str, &str)> = vec![
            ("基础", "tr.fade"), ("基础", "tr.dissolve"),
            ("滑动", "tr.slideleft"), ("滑动", "tr.coverright"),
            ("擦除", "tr.wipeleft"), ("擦除", "tr.wiperight"),
            ("图形", "tr.circleopen"), ("图形", "tr.zoomin"),
            ("模糊", "tr.hblur"), ("模糊", "tr.pixelize"),
        ];
        for (cat, tr) in sweep {
            let id = tr.trim_start_matches("tr.");
            let p = small(&format!("m11-cat-{id}"), tr);
            let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
            let vdur = video_stream_duration_sec(&out.output);
            assert!((vdur - 4.0).abs() <= FRAME, "[{cat}/{id}] 视频流零漂移: {vdur}");
            assert_eq!(video_frame_count(&out.output), 120, "[{cat}/{id}] 帧数 = Σdur×fps");
            // 转场窗中间帧存在(渲染非空且可 seek)
            let _ = frame_bytes(&out.output, 2.0);
        }
        achieved.push("21 转场库 50+(AC-4.3):五分类各 2 项经 tr.* 目录直通实渲,视频流零漂移(120 帧/4s)");
    }

    // ---- ⑯ acrossfade 音频链(册四 T4.5 / M11-R1):声画同窗 + 音频流零漂移 ----
    {
        let dir = workspace("across");
        make_media(&dir);
        let mut v = base_project("m11-across");
        v["tracks"][0]["clips"][1]["transition"] =
            json!({"type": "fade", "durMs": 500, "reason": "topic"});
        write_project(&dir, "m11-across", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        // 音频流时长 = Σdur(acrossfade 链总长 = Σ(dur+tail)−ΣD,构造性零漂移)
        let adur = audio_stream_duration_sec(&out.output);
        assert!((adur - 4.0).abs() <= FRAME, "acrossfade 音频流漂移: {adur}");
        let vdur = video_stream_duration_sec(&out.output);
        assert!((vdur - 4.0).abs() <= FRAME, "acrossfade 视频流漂移: {vdur}");
        // 边界声画同窗:转场窗 [2.0,2.5] 处音频有交叉淡变凹陷(tri 曲线),
        // 稳态区间能量显著更高
        let rms_boundary = rms_of_window(&out.output, 2.05, 2.45);
        let rms_steady = rms_of_window(&out.output, 0.5, 1.5);
        assert!(
            rms_boundary < rms_steady - 3.0,
            "转场窗音频应有交叉淡变凹陷(≥3dB): boundary={rms_boundary} steady={rms_steady}"
        );
        achieved.push("22 acrossfade 音频链(M11-R1):音/视频流双零漂移 + 转场窗交叉淡变凹陷可测");
    }

    // ---- ⑰ 特效库 ≥4 实渲(册四 T4.6):像素级断言 ----
    {
        let dir = workspace("fx");
        make_media(&dir);
        let mk = |slug: &str, fx: Value| {
            let v = json!({
                "version": 1, "schemaVersion": "2.0.0", "slug": slug, "fps": 30,
                "canvas": {"width": 320, "height": 240},
                "tracks": [{"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 2000,
                     "sourceInMs": 0, "role": "voice", "volume": 1.0, "fx": fx}
                ]}]
            });
            write_project(&dir, slug, &v);
            parse_project(v)
        };
        let w = 320usize;
        // 基准(无 fx)
        let base = cutforge_render::render(&mk("m11-fx-base", json!(null)), &dir, None, &mut |_| {}).unwrap();
        let f_base = frame_bytes(&base.output, 1.0);
        // ① fx.mono 黑白:全帧去色(R≈G≈B)
        let mono = cutforge_render::render(&mk("m11-fx-mono", json!({"combo": [{"fx": "fx.mono"}]})), &dir, None, &mut |_| {}).unwrap();
        let f_mono = frame_bytes(&mono.output, 1.0);
        let mut max_channel_gap = 0u32;
        for px_idx in (0..(w * 240 * 3)).step_by(3 * 7) {
            let (r, g, b) = (f_mono[px_idx] as u32, f_mono[px_idx + 1] as u32, f_mono[px_idx + 2] as u32);
            max_channel_gap = max_channel_gap.max(r.max(g).max(b) - r.min(g).min(b));
        }
        assert!(max_channel_gap <= 8, "黑白滤镜后通道差应≤8(去色),实得 {max_channel_gap}");
        assert_ne!(f_mono, f_base, "黑白滤镜必须真实改变像素");
        // ② fx.vignette 暗角:白场源(画布 320x240)角部显著暗于中心
        ff(&[
            "-y", "-v", "error", "-f", "lavfi", "-i", "color=c=white:size=320x240:rate=30:duration=2",
            "-c:v", "libx264", "-preset", "veryfast", "white.mp4",
        ], &dir);
        let v = json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "m11-fx-vig", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "white.mp4", "startMs": 0, "durationMs": 2000,
                 "sourceInMs": 0, "volume": 0, "fx": {"combo": [{"fx": "fx.vignette"}]}}
            ]}]
        });
        write_project(&dir, "m11-fx-vig", &v);
        let vig = cutforge_render::render(&parse_project(v), &dir, None, &mut |_| {}).unwrap();
        let f_vig = frame_bytes(&vig.output, 1.0);
        let corner = f_vig[(10 * w + 10) * 3] as u32;
        let center = f_vig[(120 * w + 160) * 3] as u32;
        assert!(corner + 20 < center, "暗角:角部({corner})应显著暗于中心({center})");
        // ③ fx.grain 胶片颗粒:与基准逐帧不同(时变噪声)
        let grain = cutforge_render::render(&mk("m11-fx-grain", json!({"combo": [{"fx": "fx.grain", "params": {"strength": 40}}]})), &dir, None, &mut |_| {}).unwrap();
        let f_grain = frame_bytes(&grain.output, 1.0);
        assert_ne!(f_grain, f_base, "颗粒噪声必须真实改变像素");
        // ④ fx.mosaic 马赛克:同块内像素归并(testsrc2 渐变域内相邻像素差消失)
        let mos = cutforge_render::render(&mk("m11-fx-mos", json!({"combo": [{"fx": "fx.mosaic", "params": {"block": 16}}]})), &dir, None, &mut |_| {}).unwrap();
        let f_mos = frame_bytes(&mos.output, 1.0);
        // 画布 320x240 → 归一后整帧铺满;取 (16,16) 块内两点(避开块边界)
        let px = |f: &[u8], x: usize, y: usize| -> (u8, u8, u8) {
            let i = (y * w + x) * 3;
            (f[i], f[i + 1], f[i + 2])
        };
        let (a, b) = (px(&f_mos, 20, 20), px(&f_mos, 27, 27));
        // 册五 T5.6:encode 步带 bt709 标签重编码(ADR-0020 决策 1),成片多一代
        // x264 量化(默认 crf23)——块内断言从逐位相等放宽为 ±4 容差(块归并语义
        // 不变:渐变域相邻像素差 ~10+ 级,量化漂移 ≤4 不可混淆)
        let near = |x: (u8, u8, u8), y: (u8, u8, u8)| {
            x.0.abs_diff(y.0) <= 4 && x.1.abs_diff(y.1) <= 4 && x.2.abs_diff(y.2) <= 4
        };
        assert!(near(a, b), "马赛克:同块内像素应归并(±4),实得 {a:?} vs {b:?}");
        assert_ne!(f_mos, f_base, "马赛克必须真实改变像素");
        achieved.push("23 特效库(册四 T4.6):mono 去色/vignette 角部衰减/grain 时变噪声/mosaic 块归并,像素级实渲断言");
    }

    // ---- ⑱ motion 动画实渲(册四 T4.6):入场淡入 + 滑入,首帧像素断言 ----
    {
        let dir = workspace("motion");
        make_media(&dir);
        let mk = |slug: &str, motion: Value| {
            let v = json!({
                "version": 1, "schemaVersion": "2.0.0", "slug": slug, "fps": 30,
                "canvas": {"width": 320, "height": 240},
                "tracks": [{"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 2000,
                     "sourceInMs": 0, "role": "voice", "volume": 1.0, "motion": motion}
                ]}]
            });
            write_project(&dir, slug, &v);
            parse_project(v)
        };
        let w = 320usize;
        // ① fadeIn:开头帧亮度显著低于稳态(黑场淡入)
        let fin = cutforge_render::render(&mk("m11-mo-fin", json!({"in": "fadeIn", "inMs": 600})), &dir, None, &mut |_| {}).unwrap();
        let f_head = frame_bytes(&fin.output, 0.05);
        let f_mid = frame_bytes(&fin.output, 1.5);
        let luma = |f: &[u8]| -> u32 {
            (0..f.len()).step_by(3 * 11).map(|i| f[i] as u32).sum::<u32>()
        };
        assert!(luma(&f_head) * 3 < luma(&f_mid), "fadeIn 首帧应显著暗于稳态");
        // ② slideInLeft:内容自左缘滑入(x:-W→0)——未完成时右侧尚为黑底,稳态后铺满
        let sli = cutforge_render::render(&mk("m11-mo-sli", json!({"in": "slideInLeft", "inMs": 600})), &dir, None, &mut |_| {}).unwrap();
        let f_slide = frame_bytes(&sli.output, 0.1);
        let right_dark = f_slide[(120 * w + 290) * 3] < 40;
        assert!(right_dark, "slideInLeft 未完成时右侧应为黑(内容自左滑入),实得 {:?}", &f_slide[(120 * w + 290) * 3..(120 * w + 290) * 3 + 3]);
        let f_settled = frame_bytes(&sli.output, 1.5);
        assert_ne!(&f_slide[(120 * w + 290) * 3..(120 * w + 290) * 3 + 3], &f_settled[(120 * w + 290) * 3..(120 * w + 290) * 3 + 3],
            "滑入完成后右缘应被内容填充");
        // 动画不改时长
        assert!((video_stream_duration_sec(&sli.output) - 2.0).abs() <= FRAME, "motion 不改时长");
        achieved.push("24 动效库 motion(册四 T4.6):fadeIn 首帧亮度断言 + slideInLeft 黑底平移断言,时长不变");
    }

    // 汇总证据(供矩阵回填)
    println!("M11-1 achieved {} 项:", achieved.len());
    for a in &achieved {
        println!("  ✅ {a}");
    }
    assert!(achieved.len() >= 18, "九项既有 + 五项 A4-BE2 + 四项 A4-BE3a(目录转场/acrossfade/特效/动效)");
}

// ---------------- 册五 T5.1 关键帧对拍矩阵(AC-5.1:逐样本实渲对拍) ----------------

/// 采样帧通道值(rgb24)。
fn channel(frame: &[u8], x: usize, y: usize, w: usize, ch: usize) -> u8 {
    frame[(y * w + x) * 3 + ch]
}

/// 行扫描:红→蓝边界位置(左红右蓝源在位移后的可观测锚点)。
fn red_blue_boundary_x(frame: &[u8], y: usize, w: usize) -> Option<usize> {
    for x in 1..w {
        let (a, b) = (px(frame, x - 1, y, w), px(frame, x, y, w));
        if is_reddish(a) && is_blueish(b) {
            return Some(x);
        }
    }
    None
}

fn luma_avg(frame: &[u8], _w: usize, step: usize) -> f64 {
    let n = frame.len() / (3 * step);
    let sum: u64 = (0..frame.len())
        .step_by(3 * step)
        .map(|i| {
            let (r, g, b) = (frame[i] as u64, frame[i + 1] as u64, frame[i + 2] as u64);
            (r * 299 + g * 587 + b * 114) / 1000
        })
        .sum();
    sum as f64 / n as f64
}

/// T5.1 关键帧五夹具 + 逐样本对拍:position 位移(两时刻像素)/scale 缩放
/// (尺寸)/opacity 淡入(中点合成)/volume 淡出(响度)/bezier vs linear 可区分;
/// 对拍夹具:多属性关键帧集(位置 easeInOut+hold × opacity 线性 × 位置.y)四采样点,
/// Rust 求值器预测 vs ffmpeg 实渲(边界像素位移 + alpha 合成)阈值内一致——
/// 求值单源纪律的渲染面证据(ADR-0018 决策 3)。ffmpeg 缺失即失败,禁止跳过。
#[test]
fn parity_keyframes_matrix() {
    assert!(ffmpeg_ok(), "ffmpeg 不可用:关键帧对拍是硬门禁,禁止跳过");
    let mut achieved: Vec<&str> = Vec::new();

    // ---- K1 position 位移:0.5→0.7 @ [0,1s),两时刻边界像素断言 ----
    {
        let dir = workspace("kf-pos");
        make_left_red_right_blue(&dir, "lr_src.mp4", 2.0);
        let v = json!({
            "version": 1, "schemaVersion": "3.0.0", "slug": "kf-pos", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "lr_src.mp4", "startMs": 0, "durationMs": 2000,
                 "sourceInMs": 0, "role": "voice", "volume": 0,
                 "keyframes": [
                    {"property": "position.x", "timeMs": 0, "value": 0.5},
                    {"property": "position.x", "timeMs": 1000, "value": 0.7}
                 ]}
            ]}]
        });
        write_project(&dir, "kf-pos", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let w = 320usize;
        // 求值器预测:offset = (pos.x(t)-0.5)*320;边界 = 160+offset
        let t05 = cutforge_core::keyframes::eval_property(&p.tracks[0].clips[0], "position.x", 500.0).unwrap();
        let t15 = cutforge_core::keyframes::eval_property(&p.tracks[0].clips[0], "position.x", 1500.0).unwrap();
        let f05 = frame_bytes(&out.output, 0.5);
        let f15 = frame_bytes(&out.output, 1.5);
        let b05 = red_blue_boundary_x(&f05, 120, w).expect("0.5s 帧必须有红蓝边界");
        let b15 = red_blue_boundary_x(&f15, 120, w).expect("1.5s 帧必须有红蓝边界");
        let e05 = (160.0 + (t05 - 0.5) * 320.0).round() as i64;
        let e15 = (160.0 + (t15 - 0.5) * 320.0).round() as i64;
        assert!((b05 as i64 - e05).abs() <= 3, "0.5s 边界: 实测 {b05} 预测 {e05}(求值 {t05})");
        assert!((b15 as i64 - e15).abs() <= 3, "1.5s 边界: 实测 {b15} 预测 {e15}(外延 {t15})");
        assert!(b15 > b05, "位移必须单调右移: {b05} → {b15}");
        achieved.push("K1 position 关键帧位移:两时刻红蓝边界 vs 求值器预测(±3px)");
    }

    // ---- K2 scale 缩放:1.0→2.0 @ [0,1s),白块尺寸断言 ----
    {
        let dir = workspace("kf-scale");
        // 黑底白块源:box [0..200)x[0..150)(5/8 × 5/8,非象限,缩放可观测)
        ff(&[
            "-y", "-v", "error",
            "-f", "lavfi", "-i", "color=c=black:size=320x240:rate=30:duration=2",
            "-vf", "drawbox=x=0:y=0:w=200:h=150:color=white:t=fill",
            "-c:v", "libx264", "-preset", "veryfast", "box_src.mp4",
        ], &dir);
        let v = json!({
            "version": 1, "schemaVersion": "3.0.0", "slug": "kf-scale", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "box_src.mp4", "startMs": 0, "durationMs": 2000,
                 "sourceInMs": 0, "volume": 0,
                 "keyframes": [
                    {"property": "scale", "timeMs": 0, "value": 1.0},
                    {"property": "scale", "timeMs": 1000, "value": 2.0}
                 ]}
            ]}]
        });
        write_project(&dir, "kf-scale", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let w = 320usize;
        // 采样点(230,165):z=1 → 源外黑;z≈1.4(0.2s) 仍外黑;z=2(1.5s) → 源(195,142) 白
        let f_early = frame_bytes(&out.output, 0.2);
        let f_late = frame_bytes(&out.output, 1.5);
        let luma_at = |f: &[u8]| -> u32 {
            let (r, g, b) =
                (channel(f, 230, 165, w, 0), channel(f, 230, 165, w, 1), channel(f, 230, 165, w, 2));
            (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000
        };
        assert!(luma_at(&f_early) < 60, "0.2s(z≈1.4) 采样点应仍在白块外(黑)");
        assert!(luma_at(&f_late) > 200, "1.5s(z=2.0) 采样点应落入放大的白块内");
        achieved.push("K2 scale 关键帧缩放:zoompan 居中放大,采样点黑→白(尺寸断言)");
    }

    // ---- K3 opacity 淡入:0→1 @ [0,1s),中点合成断言 ----
    {
        let dir = workspace("kf-opa");
        ff(&[
            "-y", "-v", "error",
            "-f", "lavfi", "-i", "color=c=white:size=320x240:rate=30:duration=2",
            "-c:v", "libx264", "-preset", "veryfast", "white.mp4",
        ], &dir);
        let v = json!({
            "version": 1, "schemaVersion": "3.0.0", "slug": "kf-opa", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "white.mp4", "startMs": 0, "durationMs": 2000,
                 "sourceInMs": 0, "volume": 0,
                 "keyframes": [
                    {"property": "opacity", "timeMs": 0, "value": 0.0},
                    {"property": "opacity", "timeMs": 1000, "value": 1.0}
                 ]}
            ]}]
        });
        write_project(&dir, "kf-opa", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let w = 320usize;
        let head = frame_bytes(&out.output, 0.05);
        let mid = frame_bytes(&out.output, 0.5);
        let tail = frame_bytes(&out.output, 1.5);
        let lh = luma_avg(&head, w, 11);
        let lm = luma_avg(&mid, w, 11);
        let lt = luma_avg(&tail, w, 11);
        assert!(lh < 60.0, "淡入起点应接近黑: {lh}");
        assert!((lm - 127.0).abs() <= 25.0, "中点(α=0.5)合成 = 半亮灰: {lm}");
        assert!(lt > 200.0, "淡入完成应接近白: {lt}");
        achieved.push("K3 opacity 关键帧淡入:geq alpha 黑底合成,起点黑/中点半亮/末点白");
    }

    // ---- K4 volume 淡出:1→0 @ [0,1s),响度断言 ----
    {
        let dir = workspace("kf-vol");
        make_media(&dir); // voice.mp4 440Hz 人声
        let v = json!({
            "version": 1, "schemaVersion": "3.0.0", "slug": "kf-vol", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 2000,
                 "sourceInMs": 0, "role": "voice", "volume": 1.0,
                 "keyframes": [
                    {"property": "volume", "timeMs": 0, "value": 1.0},
                    {"property": "volume", "timeMs": 1000, "value": 0.0}
                 ]}
            ]}]
        });
        write_project(&dir, "kf-vol", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        // 早窗(t≈0.2 音量 0.8)vs 晚窗(t≈0.8 音量 0.2):线性包络差 ≈ 12dB
        let rms_early = rms_of_window(&out.output, 0.1, 0.3);
        let rms_late = rms_of_window(&out.output, 0.7, 0.9);
        assert!(
            rms_early > rms_late + 9.0,
            "volume 关键帧淡出:早窗({rms_early}) 应比晚窗({rms_late}) 高 ≥9dB"
        );
        // 1s 后外延 = 0(静音)
        let rms_tail = rms_of_window(&out.output, 1.2, 1.8);
        assert!(rms_tail < rms_early - 20.0, "外延末值 0 → 静音: tail={rms_tail} early={rms_early}");
        achieved.push("K4 volume 关键帧:线性包络响度差(早/晚窗 ≥9dB)+ 末值外延静音");
    }

    // ---- K5 bezier vs linear 可区分:同区间缓动,中点采样差异断言 ----
    {
        let dir = workspace("kf-bez");
        let _ = ff(&[
            "-y", "-v", "error",
            "-f", "lavfi", "-i", "color=c=white:size=320x240:rate=30:duration=2",
            "-c:v", "libx264", "-preset", "veryfast", "white.mp4",
        ], &dir);
        let mk = |slug: &str, bez: bool| {
            let first = if bez {
                json!({"property": "opacity", "timeMs": 0, "value": 0.0,
                       "interp": "bezier", "bezier": [0.1, 0.8, 0.2, 1.0]})
            } else {
                json!({"property": "opacity", "timeMs": 0, "value": 0.0})
            };
            let v = json!({
                "version": 1, "schemaVersion": "3.0.0", "slug": slug, "fps": 30,
                "canvas": {"width": 320, "height": 240},
                "tracks": [{"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "white.mp4", "startMs": 0, "durationMs": 2000,
                     "sourceInMs": 0, "volume": 0,
                     "keyframes": [first, {"property": "opacity", "timeMs": 1000, "value": 1.0}]}
                ]}]
            });
            write_project(&dir, slug, &v);
            parse_project(v)
        };
        let lin = cutforge_render::render(&mk("kf-bez-lin", false), &dir, None, &mut |_| {}).unwrap();
        let bez = cutforge_render::render(&mk("kf-bez-bez", true), &dir, None, &mut |_| {}).unwrap();
        let w = 320usize;
        let f_lin = frame_bytes(&lin.output, 0.5);
        let f_bez = frame_bytes(&bez.output, 0.5);
        let l_lin = luma_avg(&f_lin, w, 11);
        let l_bez = luma_avg(&f_bez, w, 11);
        // 求值器预测:bezier(0.1,0.8,0.2,1.0) 在 progress 0.5 处 ≈ 0.8;线性 = 0.5
        let pred = cutforge_core::keyframes::bezier_progress([0.1, 0.8, 0.2, 1.0], 0.5);
        assert!(pred > 0.65, "该控制柄中点应显著快于线性: {pred}");
        assert!(l_bez > l_lin + 40.0, "bezier({l_bez}) 与 linear({l_lin}) 中点亮度差必须可区分(≥40)");
        achieved.push("K5 贝塞尔 vs 线性:同区间中点亮度差可区分(求值器预测 ≈0.8 vs 0.5)");
    }

    // ---- K6 逐样本对拍:多属性关键帧集(位置 easeInOut+hold × opacity 线性 × 位置.y), ----
    //      四采样点:Rust 求值 vs ffmpeg 实渲(边界位移 ±3px + alpha 合成 ±0.08)
    {
        let dir = workspace("kf-parity");
        make_left_red_right_blue(&dir, "lr_src.mp4", 2.5);
        let v = json!({
            "version": 1, "schemaVersion": "3.0.0", "slug": "kf-parity", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "lr_src.mp4", "startMs": 0, "durationMs": 2500,
                 "sourceInMs": 0, "role": "voice", "volume": 0,
                 "keyframes": [
                    {"property": "position.x", "timeMs": 0, "value": 0.5, "interp": "easeInOut"},
                    {"property": "position.x", "timeMs": 2000, "value": 0.8, "interp": "hold"},
                    {"property": "position.y", "timeMs": 500, "value": 0.55},
                    {"property": "opacity", "timeMs": 300, "value": 0.1},
                    {"property": "opacity", "timeMs": 1800, "value": 0.9}
                 ]}
            ]}]
        });
        write_project(&dir, "kf-parity", &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let clip = &p.tracks[0].clips[0];
        let w = 320usize;
        // 采样点选 alpha≥0.4 的时域(opacity [300,1800) 线性 0.1→0.9;更早的样本
        // alpha 太低,红蓝边界低于色类判定阈,位移观测不成立——alpha 断言另有覆盖)
        for t_ms in [1000u64, 1400, 1800, 2200] {
            let t = t_ms as f64;
            let pos = cutforge_core::keyframes::eval_property(clip, "position.x", t).unwrap();
            let opa = cutforge_core::keyframes::eval_property(clip, "opacity", t).unwrap();
            let f = frame_bytes(&out.output, t / 1000.0);
            // 位置:红蓝边界 = 160 + (pos-0.5)*320(±3px 容差:缩放插值/帧取整)。
            // 边界用**红通道梯度最大降幅**定位(不依赖色类阈值:alpha<1 时
            // r=b=255×α,梯度降幅 = 255×α,远超噪声,对半透明帧稳健)
            let drop_at = |x: usize| -> i64 {
                channel(&f, x - 1, 120, w, 0) as i64 - channel(&f, x, 120, w, 0) as i64
            };
            let (mut bb, mut best) = (1usize, 0i64);
            for x in 1..w {
                if drop_at(x) > best {
                    best = drop_at(x);
                    bb = x;
                }
            }
            let b = bb;
            assert!(best >= 60, "{t_ms}ms 边界梯度不足(alpha 过低?): drop={best}");
            let e = (160.0 + (pos - 0.5) * 320.0).round() as i64;
            assert!((b as i64 - e).abs() <= 3, "{t_ms}ms 位置: 实测 {b} vs 求值 {e}(pos={pos})");
            // opacity:取边界左侧 40px(内容内)红通道 = 255×alpha(黑底直乘)
            let sx = (b as i64 - 40).max(2) as usize;
            let r = channel(&f, sx, 120, w, 0) as f64 / 255.0;
            assert!(
                (r - opa).abs() <= 0.08,
                "{t_ms}ms alpha: 实测 {r:.3} vs 求值 {opa:.3}"
            );
        }
        achieved.push("K6 逐样本对拍:四采样点位置(±3px)与 alpha(±0.08)双属性 Rust↔ffmpeg 一致");
    }

    println!("T5.1 关键帧 parity achieved {} 项:", achieved.len());
    for a in &achieved {
        println!("  OK {a}");
    }
    assert!(achieved.len() >= 6, "关键帧夹具:位置/缩放/淡入/响度/贝塞尔可区分 + 逐样本对拍");
}

// ---------------- 册五 T5.2 调色 + T5.3 音频工作站对拍矩阵(AC-5.2/AC-5.3) ----------------

/// 带通道容差的像素近似(成片带 bt709 标签重编码,一代 x264 量化 ±4 内)。
fn px_near(a: (u8, u8, u8), b: (u8, u8, u8), tol: i32) -> bool {
    (a.0 as i32 - b.0 as i32).abs() <= tol
        && (a.1 as i32 - b.1 as i32).abs() <= tol
        && (a.2 as i32 - b.2 as i32).abs() <= tol
}

/// 频段 RMS(dB;T5.3 EQ 的频域断言:带通 → astats)。
fn band_rms_db(p: &Path, from: f64, to: f64, lo: u32, hi: u32) -> f64 {
    let fc = (lo + hi) as f64 / 2.0;
    let (_o, err) = ff_out(&[
        "-i", p.to_str().unwrap(),
        // 双级 bandpass(0.2 oct)压泄漏:单级 12dB/oct 时邻频能量盖过深谷
        "-af", &format!("atrim={from}:{to},bandpass=f={fc}:width_type=o:w=0.2,bandpass=f={fc}:width_type=o:w=0.2,astats=metadata=1"),
        "-f", "null", "-",
    ], Path::new("."));
    let pos = err.rfind("RMS level dB:").expect(err.as_str());
    err[pos + 13..].split_whitespace().next().unwrap().parse().unwrap()
}

/// 成片 LUFS(loudnorm 测量,与 audio_loudness 工具同源参数)。
fn lufs_of(p: &Path) -> f64 {
    let (_o, err) = ff_out(&[
        "-hide_banner", "-nostats", "-i", p.to_str().unwrap(),
        "-filter_complex", "loudnorm=I=-14:TP=-1.0:print_format=json", "-f", "null", "-",
    ], Path::new("."));
    let (s, e) = (err.rfind('{').expect(err.as_str()), err.rfind('}').expect(err.as_str()));
    let m: Value = serde_json::from_str(&err[s..=e]).unwrap();
    m["input_i"].as_str().unwrap().parse().unwrap()
}

/// 输出视频流色彩标签(ffprobe;ADR-0020 复验口径)。
fn color_tags(p: &Path) -> (String, String, String) {
    let o = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-print_format", "json",
               "-show_entries", "stream=color_primaries,color_transfer,color_space"])
        .arg(p).output().expect("ffprobe 必须存在");
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let s = &v["streams"][0];
    (
        s["color_primaries"].as_str().unwrap_or("none").into(),
        s["color_transfer"].as_str().unwrap_or("none").into(),
        s["color_space"].as_str().unwrap_or("none").into(),
    )
}

/// 生成 3D .cube(等式中性缩放:f(r,g,b) = (min(1, r×k), g, b);N=4 紧凑夹具)。
fn write_scale_red_cube(dir: &Path, rel: &str, k: f64) {
    let n = 4u32;
    let mut text = String::from("TITLE \"parity scale-red\"\nLUT_3D_SIZE 4\n");
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                let (fr, fg, fb) = (
                    r as f64 / (n - 1) as f64,
                    g as f64 / (n - 1) as f64,
                    b as f64 / (n - 1) as f64,
                );
                text.push_str(&format!("{:.6} {fg:.6} {fb:.6}\n", (fr * k).min(1.0)));
            }
        }
    }
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// T5.2 调色五夹具 + T5.3 音频三夹具(AC-5.2/AC-5.3):
/// G1 LGG 色轮偏移(阴影红抬,像素色偏断言)+ 输出 bt709 标签复验;
/// G2 曲线提亮(master 点集,亮度断言);G3 LUT 前后帧差(.cube 红增益);
/// G4 饱和度归零=灰度(fx.mono 同断言口径);G5 grade×关键帧组合不冲突;
/// A1 轨道 EQ 频段能量(440/1000 双音,频带 RMS 比值断言);
/// A2 轨道压缩动态范围收窄(响/静双段 RMS 差断言);
/// A3 响度单:loudnormTarget=-16 输出实测偏差 ≤1LU(AC-5.3 数值断言)。
/// ffmpeg 缺失即失败,禁止跳过。
#[test]
fn parity_grade_audio_matrix() {
    assert!(ffmpeg_ok(), "ffmpeg 不可用:调色/音频对拍是硬门禁,禁止跳过");
    let mut achieved: Vec<&str> = Vec::new();
    let w = 320usize;

    // ---- G1 LGG 色轮偏移:lift=[0.3,0,0](阴影红抬)→ 暗灰源红通道抬升 ----
    // (采样点选**阴影域暗像素**:colorbalance 的通道权重按像素亮度计,
    //  纯饱和极值点(如纯蓝 0,0,255)是已知的钳制盲区——G5 的 gain 通路覆盖极值色)
    {
        let dir = workspace("grade-lgg");
        make_two_tone(&dir, "dark.mp4", "0x202020", "0x202020", 2.0, 2.0);
        let mk = |slug: &str, grade: Value| {
            let mut c0 = json!({
                "id": "V1-001", "src": "dark.mp4", "startMs": 0, "durationMs": 2000,
                "sourceInMs": 0, "volume": 0
            });
            if !grade.is_null() {
                c0["grade"] = grade;
            }
            let v = json!({
                "version": 1, "schemaVersion": "3.0.0", "slug": slug, "fps": 30,
                "canvas": {"width": 320, "height": 240},
                "tracks": [{"id": "V1", "kind": "video", "clips": [c0]}]
            });
            write_project(&dir, slug, &v);
            parse_project(v)
        };
        let base = cutforge_render::render(&mk("lgg-base", json!(null)), &dir, None, &mut |_| {}).unwrap();
        let graded = cutforge_render::render(&mk("lgg-g", json!({"lift": [0.3, 0.0, 0.0]})), &dir, None, &mut |_| {}).unwrap();
        let fb = frame_bytes(&base.output, 1.0);
        let fg = frame_bytes(&graded.output, 1.0);
        let (pb, pg) = (px(&fb, 160, 120, w), px(&fg, 160, 120, w));
        assert!(pg.0 > pb.0 + 20, "LGG lift 红:暗像素红通道应抬升,基线 {pb:?} → 调色 {pg:?}");
        assert!(
            (pg.1 as i32 - pb.1 as i32).abs() <= 10 && (pg.2 as i32 - pb.2 as i32).abs() <= 10,
            "绿蓝通道不动(纯红阴影抬升): 基线 {pb:?} → 调色 {pg:?}"
        );
        // ADR-0020:输出色彩标签复验(bt709 三枚举)
        let (prim, trc, space) = color_tags(&graded.output);
        assert_eq!(
            (prim.as_str(), trc.as_str(), space.as_str()),
            ("bt709", "bt709", "bt709"),
            "输出必须携带 bt709 完整标签(ADR-0020 决策 1)"
        );
        achieved.push("G1 LGG 色轮偏移:lift 红抬暗像素色偏断言 + bt709 标签 ffprobe 复验");
    }

    // ---- G2 曲线提亮:master 0.5→0.8,灰源中点亮度 ~128→~204 ----
    {
        let dir = workspace("grade-curve");
        make_two_tone(&dir, "gray.mp4", "gray", "gray", 2.0, 2.0);
        let mk = |slug: &str, grade: Value| {
            let mut c0 = json!({
                "id": "V1-001", "src": "gray.mp4", "startMs": 0, "durationMs": 2000,
                "sourceInMs": 0, "volume": 0
            });
            c0["grade"] = grade;
            let v = json!({
                "version": 1, "schemaVersion": "3.0.0", "slug": slug, "fps": 30,
                "canvas": {"width": 320, "height": 240},
                "tracks": [{"id": "V1", "kind": "video", "clips": [c0]}]
            });
            write_project(&dir, slug, &v);
            parse_project(v)
        };
        let base = cutforge_render::render(&mk("curve-base", json!(null)), &dir, None, &mut |_| {}).unwrap();
        let graded = cutforge_render::render(
            &mk("curve-g", json!({"curves": {"master": [[0.0, 0.0], [0.5, 0.8], [1.0, 1.0]]}})),
            &dir, None, &mut |_| {},
        ).unwrap();
        let lb = luma_avg(&frame_bytes(&base.output, 1.0), w, 11);
        let lg = luma_avg(&frame_bytes(&graded.output, 1.0), w, 11);
        assert!((lb - 128.0).abs() <= 12.0, "灰源基线中点应 ≈128: {lb}");
        assert!(
            lg > lb + 50.0 && (lg - 204.0).abs() <= 25.0,
            "曲线 0.5→0.8 提亮:实测 {lg}(期望 ≈204,基线 {lb})"
        );
        achieved.push("G2 曲线提亮:master 点集编译 curves,中点亮度 128→≈204(±25)");
    }

    // ---- G3 LUT:红增益 .cube → 灰源红通道抬升且帧面改变(内容哈希入键) ----
    {
        let dir = workspace("grade-lut");
        write_scale_red_cube(&dir, ".cutforge/luts/sr.cube", 1.5);
        make_two_tone(&dir, "gray.mp4", "gray", "gray", 2.0, 2.0);
        let mk = |slug: &str, grade: Value| {
            let mut c0 = json!({
                "id": "V1-001", "src": "gray.mp4", "startMs": 0, "durationMs": 2000,
                "sourceInMs": 0, "volume": 0
            });
            c0["grade"] = grade;
            let v = json!({
                "version": 1, "schemaVersion": "3.0.0", "slug": slug, "fps": 30,
                "canvas": {"width": 320, "height": 240},
                "tracks": [{"id": "V1", "kind": "video", "clips": [c0]}]
            });
            write_project(&dir, slug, &v);
            parse_project(v)
        };
        let base = cutforge_render::render(&mk("lut-base", json!(null)), &dir, None, &mut |_| {}).unwrap();
        let graded = cutforge_render::render(
            &mk("lut-g", json!({"lut": ".cutforge/luts/sr.cube"})),
            &dir, None, &mut |_| {},
        ).unwrap();
        let fb = frame_bytes(&base.output, 1.0);
        let fg = frame_bytes(&graded.output, 1.0);
        assert_ne!(fb, fg, "LUT 应用后帧面必须改变");
        let pb = px(&fb, 160, 120, w);
        let pg = px(&fg, 160, 120, w);
        assert!(
            pg.0 > pb.0 + 20 && px_near((0, pg.1, pg.2), (0, pb.1, pb.2), 12),
            "红增益 LUT:红通道抬升、绿蓝不动,基线 {pb:?} → LUT {pg:?}"
        );
        achieved.push("G3 LUT 应用:.cube 红增益帧差 + 分通道方向断言(lut3d)");
    }

    // ---- G4 饱和度归零 = 灰度(fx.mono 同断言口径:全帧 R==G==B) ----
    {
        let dir = workspace("grade-sat0");
        make_left_red_right_blue(&dir, "lr_src.mp4", 2.0);
        let v = json!({
            "version": 1, "schemaVersion": "3.0.0", "slug": "grade-sat0", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [{
                "id": "V1-001", "src": "lr_src.mp4", "startMs": 0, "durationMs": 2000,
                "sourceInMs": 0, "volume": 0,
                "grade": {"saturation": 0}
            }]}]
        });
        write_project(&dir, "grade-sat0", &v);
        let out = cutforge_render::render(&parse_project(v), &dir, None, &mut |_| {}).unwrap();
        let f = frame_bytes(&out.output, 1.0);
        for (x, y) in [(40usize, 60usize), (160, 120), (280, 180)] {
            let p = px(&f, x, y, w);
            assert!(
                p.0.abs_diff(p.1) <= 4 && p.1.abs_diff(p.2) <= 4,
                "饱和度 0 → 灰度(R≈G≈B,±4 量化容差): ({x},{y}) = {p:?}"
            );
        }
        achieved.push("G4 饱和度归零=灰度:红蓝源全采样点 R≈G≈B(fx.mono 同口径)");
    }

    // ---- G5 grade × 关键帧组合不冲突:opacity 淡入关键帧 + grade 同片段共存 ----
    {
        let dir = workspace("grade-kf");
        make_two_tone(&dir, "gray.mp4", "gray", "gray", 2.0, 2.0);
        let v = json!({
            "version": 1, "schemaVersion": "3.0.0", "slug": "grade-kf", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [{
                "id": "V1-001", "src": "gray.mp4", "startMs": 0, "durationMs": 2000,
                "sourceInMs": 0, "volume": 0,
                "grade": {"gain": [1.0, 1.0, 2.0]},
                "keyframes": [
                    {"property": "opacity", "timeMs": 0, "value": 0.0},
                    {"property": "opacity", "timeMs": 1000, "value": 1.0}
                ]
            }]}]
        });
        write_project(&dir, "grade-kf", &v);
        let out = cutforge_render::render(&parse_project(v), &dir, None, &mut |_| {}).unwrap();
        let f_head = frame_bytes(&out.output, 0.05);
        let f_tail = frame_bytes(&out.output, 1.5);
        let lh = luma_avg(&f_head, w, 11);
        let lt = luma_avg(&f_tail, w, 11);
        assert!(lh < 60.0, "淡入关键帧仍生效(起点近黑): {lh}");
        // 灰(128)×蓝增益 bb=2 → (128,128,255):luma = (299·128+587·128+114·255)/1000 ≈ 142
        assert!((lt - 142.0).abs() <= 12.0, "grade 蓝增益在 kf 段图内仍生效(末帧蓝抬): {lt}");
        let pt = px(&f_tail, 160, 120, w);
        assert!(pt.2 > pt.0 + 60, "蓝增益方向正确(gain bb=2): {pt:?}");
        achieved.push("G5 grade×关键帧组合:kf 段图内调色链共存,淡入+蓝增益双生效");
    }

    // ---- A1 轨道 EQ:440Hz 谷(-18dB)→ 440/1000 频带 RMS 比值下降 ≥8dB ----
    {
        let dir = workspace("audio-eq");
        make_media(&dir); // voice.mp4(画面用;volume 0 不进混音)
        // 双音源:440 + 1000 等幅 amix(aac)
        ff(&[
            "-y", "-v", "error",
            "-f", "lavfi", "-i", "sine=frequency=440:duration=3",
            "-f", "lavfi", "-i", "sine=frequency=1000:duration=3",
            "-filter_complex", "amix=inputs=2:normalize=0",
            "-c:a", "aac", "tone2.m4a",
        ], &dir);
        let mk = |slug: &str, eq: Value| {
            let v = json!({
                "version": 1, "schemaVersion": "3.0.0", "slug": slug, "fps": 30,
                "canvas": {"width": 320, "height": 240},
                "tracks": [
                    {"id": "V1", "kind": "video", "clips": [{
                        "id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 3000,
                        "sourceInMs": 0, "volume": 0
                    }]},
                    {"id": "A1", "kind": "audio", "eq": eq, "clips": [{
                        "id": "A1-001", "src": "tone2.m4a", "startMs": 0, "durationMs": 3000,
                        "volume": 1.0
                    }]}
                ]
            });
            write_project(&dir, slug, &v);
            parse_project(v)
        };
        let base = cutforge_render::render(&mk("eq-base", json!(null)), &dir, None, &mut |e| eprintln!("BASE-EV {e}")).unwrap();
        let cut = cutforge_render::render(
            &mk("eq-cut", json!([{"type": "peaking", "freq": 440, "gain": -18, "q": 1.0}])),
            &dir, None, &mut |e| eprintln!("CUT-EV {e}"),
        ).map_err(|e| format!("EQ-CUT-FAIL: {e}")).unwrap();
        let ratio = |o: &cutforge_render::RenderOutcome| {
            // 窄带 ±25Hz(带通默认 12dB/oct,泄漏可忽略);q=1 的 peaking 在中心
            // 频率处为全量 -18dB
            band_rms_db(&o.output, 0.5, 2.5, 415, 465) - band_rms_db(&o.output, 0.5, 2.5, 975, 1025)
        };
        let (r0, r1) = (ratio(&base), ratio(&cut));
        assert!(
            r0 - r1 >= 8.0,
            "EQ 440Hz -18dB:频带比应下降 ≥8dB,基线 {r0:.1} → EQ {r1:.1}"
        );
        achieved.push("A1 轨道 EQ:peaking 440Hz -18dB,440/1000 频带 RMS 比下降 ≥8dB(频域)");
    }

    // ---- A2 轨道压缩:响/静双段 RMS 差收窄 ≥3dB ----
    {
        let dir = workspace("audio-dyn");
        make_media(&dir);
        let mk = |slug: &str, dyn_: Value| {
            let v = json!({
                "version": 1, "schemaVersion": "3.0.0", "slug": slug, "fps": 30,
                "canvas": {"width": 320, "height": 240},
                "tracks": [
                    {"id": "V1", "kind": "video", "clips": [{
                        "id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 3000,
                        "sourceInMs": 0, "volume": 0
                    }]},
                    // 响段 volume 1.5(正弦源峰值 -18dBFS → RMS ≈ -17.5,阈上 ~9.5dB)
                    // 静段 volume 0.25(RMS ≈ -33,阈下 ~6dB)——压缩只折响段
                    {"id": "A1", "kind": "audio", "dyn": dyn_, "clips": [
                        {"id": "A1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 1500,
                         "sourceInMs": 0, "role": "voice", "volume": 1.5},
                        {"id": "A1-002", "src": "voice.mp4", "startMs": 1500, "durationMs": 1500,
                         "sourceInMs": 0, "role": "voice", "volume": 0.25}
                    ]}
                ]
            });
            write_project(&dir, slug, &v);
            parse_project(v)
        };
        let base = cutforge_render::render(&mk("dyn-base", json!(null)), &dir, None, &mut |_| {}).unwrap();
        let comp = cutforge_render::render(
            &mk("dyn-c", json!({"thresholdDb": -27, "ratio": 8, "attackMs": 5, "releaseMs": 100})),
            &dir, None, &mut |_| {},
        ).unwrap();
        let spread = |o: &cutforge_render::RenderOutcome| {
            rms_of_window(&o.output, 0.3, 1.3) - rms_of_window(&o.output, 1.8, 2.8)
        };
        let (s0, s1) = (spread(&base), spread(&comp));
        assert!(s0 >= 8.0, "响/静段基线动态范围应 ≥8dB: {s0:.1}");
        assert!(
            s1 <= s0 - 3.0,
            "压缩后动态范围应收窄 ≥3dB:基线 {s0:.1} → 压缩 {s1:.1}"
        );
        achieved.push("A2 轨道压缩:thresholdDb -27/ratio 8,响静段动态范围收窄 ≥3dB");
    }

    // ---- A3 响度单:loudnormTarget=-16 → 输出 LUFS 偏差 ≤1LU(AC-5.3) ----
    {
        let dir = workspace("audio-lufs");
        make_media(&dir);
        let v = json!({
            "version": 1, "schemaVersion": "3.0.0", "slug": "audio-lufs", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [{
                "id": "V1-001", "src": "voice.mp4", "startMs": 0, "durationMs": 3000,
                "sourceInMs": 0, "role": "voice", "volume": 1.0
            }]}]
        });
        write_project(&dir, "audio-lufs", &v);
        let p = parse_project(v);
        let out = cutforge_render::render_with_opts(
            &p, &dir, None, false,
            cutforge_render::plan::RenderOptions { loudnorm_i: Some(-16.0), ..Default::default() },
            &mut |_| {},
        ).unwrap();
        let lufs = lufs_of(&out.output);
        assert!(
            (lufs - (-16.0)).abs() <= 1.0,
            "AC-5.3:目标 -16 的输出实测 {lufs:.2} LUFS,偏差必须 ≤1LU"
        );
        achieved.push("A3 响度单:loudnormTarget=-16 输出实测偏差 ≤1LU(AC-5.3 数值断言)");
    }

    println!("T5.2/T5.3 调色+音频 parity achieved {} 项:", achieved.len());
    for a in &achieved {
        println!("  OK {a}");
    }
    assert!(achieved.len() >= 8, "调色五夹具 + 音频三夹具全数落档");
}
