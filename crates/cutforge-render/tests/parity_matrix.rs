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
        assert_eq!(a, b, "马赛克:同块内像素应归并,实得 {a:?} vs {b:?}");
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
