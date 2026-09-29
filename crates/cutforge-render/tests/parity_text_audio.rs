//! 册四 A4-BE3b 实渲夹具(AC-4.4/AC-4.6):文本/字幕/花字/卡拉OK 渲染可见性、
//! 降噪/变调音频链、track mute/solo/hidden 渲染联动、代理 useProxy 分叉。
//! 与 parity_matrix 同风格:ffmpeg 实测像素/流断言,缺失即失败,禁止静默跳过。
//! 单测面(textass 确定性/SRT 往返/链序参数)在各自模块;本文件锁**成片证据**。

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

fn ff_out(args: &[&str]) -> (String, String) {
    let out = Command::new("ffmpeg").args(args).current_dir(Path::new(".")).output().expect("ffmpeg 必须存在");
    assert!(out.status.success(), "ffmpeg 失败: {}", String::from_utf8_lossy(&out.stderr));
    (String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
}

fn stream_duration_sec(p: &Path, sel: &str) -> f64 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", sel, "-print_format", "json", "-show_entries", "stream=duration"])
        .arg(p).output().expect("ffprobe 必须存在");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["streams"][0]["duration"].as_str().unwrap().parse().unwrap()
}

/// 整帧原始字节(rawvideo rgb24)。
fn frame_bytes(p: &Path, at_sec: f64) -> Vec<u8> {
    let out = Command::new("ffmpeg").args([
        "-ss", &format!("{at_sec}"), "-i", p.to_str().unwrap(),
        "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-",
    ]).output().expect("ffmpeg 必须存在");
    assert!(!out.stdout.is_empty(), "无帧输出: {} at {at_sec}s", p.display());
    out.stdout
}

/// 音频窗口 RMS(dB;af 可给任意前置滤镜,如 bandpass/highpass)。
fn rms_of_window(p: &Path, from: f64, to: f64, af: &str) -> f64 {
    let chain = if af.is_empty() {
        format!("atrim={from}:{to},astats=metadata=1")
    } else {
        format!("{af},atrim={from}:{to},astats=metadata=1")
    };
    let (_o, err) = ff_out(&["-i", p.to_str().unwrap(), "-af", &chain, "-f", "null", "-"]);
    let pos = err.rfind("RMS level dB:").expect(err.as_str());
    err[pos + 13..].split_whitespace().next().unwrap().parse().unwrap()
}

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cf-text-audio-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_project(dir: &Path, v: &Value) {
    cutforge_io::atomic::atomic_write(
        &dir.join("05_时间线工程/project.json"),
        serde_json::to_string_pretty(v).unwrap().as_bytes(),
    ).unwrap();
}

fn parse_project(v: Value) -> cutforge_core::model::Project {
    serde_json::from_value(v).unwrap()
}

/// 纯黑底 2s 视频(无声;文本可见性底色)。
fn make_black(dir: &Path, name: &str) {
    ff(&[
        "-y", "-v", "error",
        "-f", "lavfi", "-i", "color=c=black:size=320x240:rate=30:duration=2",
        "-c:v", "libx264", "-preset", "veryfast", name,
    ], dir);
}

/// 纯黑底 + 正弦(音画齐备;时长 4s——变调+变速组合夹具需 2x 速读源)。
fn make_black_voice(dir: &Path, name: &str, freq: u32) {
    ff(&[
        "-y", "-v", "error",
        "-f", "lavfi", "-i", "color=c=black:size=320x240:rate=30:duration=4",
        "-f", "lavfi", "-i", &format!("sine=frequency={freq}:duration=4"),
        "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest", name,
    ], dir);
}

/// 指定采样带内亮像素数(白字在黑底)。
fn bright_pixels_in_band(frame: &[u8], w: usize, y0: usize, y1: usize, thr: u8) -> usize {
    let mut n = 0usize;
    for y in y0..y1.min(240) {
        for x in (0..w).step_by(2) {
            let i = (y * w + x) * 3;
            let luma = ((frame[i] as u32 * 299 + frame[i + 1] as u32 * 587 + frame[i + 2] as u32 * 114) / 1000) as u8;
            if luma > thr {
                n += 1;
            }
        }
    }
    n
}

fn base_project(slug: &str) -> Value {
    json!({
        "version": 1, "schemaVersion": "2.0.0", "slug": slug, "fps": 30,
        "canvas": {"width": 320, "height": 240},
        "tracks": [
            {"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "black.mp4", "startMs": 0, "durationMs": 2000, "volume": 0}
            ]}
        ]
    })
}

fn plan_of(p: &cutforge_core::model::Project, dir: &Path) -> cutforge_render::RenderPlan {
    cutforge_render::RenderPlan::build(p, dir, None)
}

#[test]
fn parity_text_audio_matrix() {
    assert!(ffmpeg_ok(), "ffmpeg 不可用:本夹具是硬门禁,禁止跳过");
    let mut achieved: Vec<String> = Vec::new();

    // ---- ① 文本片段渲染可见(AC-4.4 像素断言:文字出现区域非黑,窗外归零) ----
    {
        let dir = workspace("text");
        make_black(&dir, "black.mp4");
        let mut v = base_project("text-visible");
        v["tracks"].as_array_mut().unwrap().push(json!(
            {"id": "T1", "kind": "text", "clips": [
                {"id": "T1-001", "startMs": 500, "durationMs": 1000, "text": "TEST",
                 "textStyle": {"fontSize": 56, "color": "#FFFFFF", "align": "bottomCenter"}}
            ]}
        ));
        write_project(&dir, &v);
        let p = parse_project(v);
        let with = cutforge_render::render_frame(&p, &dir, None, 1000, cutforge_render::FrameFormat::Png).unwrap();
        let f = frame_bytes(&with.output, 0.0);
        // bottomCenter(MarginV=24)+56px 字号 → 文字带约 y∈[140,232)
        let bright = bright_pixels_in_band(&f, 320, 140, 232, 180);
        assert!(bright >= 40, "文本带应有大量亮像素(白字黑底),实得 {bright}");
        // 整片管线(与单帧双通道):窗外(1.9s > 片段终点 1.5s)同带应几乎全黑
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let bright_out = bright_pixels_in_band(&frame_bytes(&out.output, 1.9), 320, 140, 232, 180);
        assert!(bright_out * 5 < bright, "文本窗外同带应无字:窗内 {bright} vs 窗外 {bright_out}");
        achieved.push("① 文本片段渲染可见(单帧/整片双通道;带内亮像素断言,窗外归零)".into());
    }

    // ---- ② 文本轨 mute/hidden 静默联动(BE3b 文本面) ----
    {
        let dir = workspace("textmute");
        make_black(&dir, "black.mp4");
        let mk = |flag: &str, slug: &str| {
            let mut v = base_project(slug);
            let mut t = json!({"id": "T1", "kind": "text", "clips": [
                {"id": "T1-001", "startMs": 0, "durationMs": 2000, "text": "TEST",
                 "textStyle": {"fontSize": 56, "color": "#FFFFFF"}}
            ]});
            t[flag] = json!(true);
            v["tracks"].as_array_mut().unwrap().push(t);
            parse_project(v)
        };
        write_project(&dir, &json!(null)); // 占位避免借用混乱:下方每例独立写盘
        for (flag, slug) in [("mute", "text-mute"), ("hidden", "text-hidden")] {
            let p = mk(flag, slug);
            let with = cutforge_render::render_frame(&p, &dir, None, 1000, cutforge_render::FrameFormat::Png).unwrap();
            let bright = bright_pixels_in_band(&frame_bytes(&with.output, 0.0), 320, 140, 232, 180);
            assert!(bright < 5, "{flag} 文本轨必须无字,实得 {bright} 亮像素");
        }
        achieved.push("② 文本轨 mute/hidden 静默联动(mute=hidden=文本静默,带内零亮像素)".into());
    }

    // ---- ③ 花字模板 ≥2 实渲(hz.box 底衬色块 + hz.gradient):像素可分 ----
    {
        let dir = workspace("huazi");
        make_black(&dir, "black.mp4");
        let mk = |huazi: Value, slug: &str| {
            let mut v = base_project(slug);
            v["tracks"].as_array_mut().unwrap().push(json!(
                {"id": "T1", "kind": "text", "clips": [
                    {"id": "T1-001", "startMs": 0, "durationMs": 2000, "text": "字",
                     "textStyle": {"fontSize": 56, "color": "#FFFFFF", "x": 160, "y": 120},
                     "huazi": huazi}
                ]}
            ));
            write_project(&dir, &v);
            parse_project(v)
        };
        // ③-a hz.box(BorderStyle=3 + 底衬 &H00E5FF00 → RGB(0,255,229) 青黄):字周大片色块
        let p_box = mk(json!({"template": "hz.box"}), "hz-box");
        let box_frame = cutforge_render::render_frame(&p_box, &dir, None, 1000, cutforge_render::FrameFormat::Png).unwrap();
        let f = frame_bytes(&box_frame.output, 0.0);
        let mut cyanish = 0usize;
        for y in (60usize..200).step_by(2) {
            for x in (80usize..240).step_by(2) {
                let i = (y * 320 + x) * 3;
                if f[i] < 100 && f[i + 1] > 150 && f[i + 2] > 120 {
                    cyanish += 1;
                }
            }
        }
        assert!(cyanish >= 20, "hz.box 底衬色块应可见(青黄像素),实得 {cyanish}");
        // ③-b hz.gradient(渐变):同点位渲染与 box 像素可分(模板真实生效)
        let p_grad = mk(json!({"template": "hz.gradient"}), "hz-grad");
        let grad = cutforge_render::render_frame(&p_grad, &dir, None, 1000, cutforge_render::FrameFormat::Png).unwrap();
        assert_ne!(frame_bytes(&box_frame.output, 0.0), frame_bytes(&grad.output, 0.0), "不同花字模板必须产生不同像素");
        achieved.push("③ 花字 ≥2 实渲(hz.box 底衬色块像素断言 + hz.gradient 渲染面分叉)".into());
    }

    // ---- ④ 卡拉OK实渲(\kf 高亮色随时间推进:首帧白/末帧橙) ----
    {
        let dir = workspace("karaoke");
        make_black(&dir, "black.mp4");
        let mut v = base_project("karaoke");
        v["tracks"].as_array_mut().unwrap().push(json!(
            {"id": "T1", "kind": "text", "clips": [
                {"id": "T1-001", "startMs": 0, "durationMs": 2000, "text": "卡拉OK字",
                 "textStyle": {"fontSize": 48, "karaoke": true, "x": 160, "y": 120},
                 "huazi": {"template": "hz.karaoke"}}
            ]}
        ));
        write_project(&dir, &v);
        let p = parse_project(v);
        let head = cutforge_render::render_frame(&p, &dir, None, 100, cutforge_render::FrameFormat::Png).unwrap();
        let tail = cutforge_render::render_frame(&p, &dir, None, 1900, cutforge_render::FrameFormat::Png).unwrap();
        // 中心采样:开头以未唱色(白)为主,结尾以已唱色(hz.karaoke highlight &H0040FF → RGB(255,64,0))为主
        let classify = |path: &Path| -> (usize, usize) {
            let f = frame_bytes(path, 0.0);
            let (mut white, mut orange) = (0usize, 0usize);
            for y in (90usize..150).step_by(2) {
                for x in (60usize..260).step_by(2) {
                    let i = (y * 320 + x) * 3;
                    let (r, g, b) = (f[i], f[i + 1], f[i + 2]);
                    if r > 200 && g > 200 && b > 200 {
                        white += 1;
                    }
                    if r > 150 && g < 130 && b < 80 {
                        orange += 1;
                    }
                }
            }
            (white, orange)
        };
        let (w0, o0) = classify(&head.output);
        let (w1, o1) = classify(&tail.output);
        assert!(o1 > o0 + 10, "卡拉OK末帧已唱高亮(橙)应显著多于首帧: head={o0} tail={o1}");
        assert!(w0 >= w1, "卡拉OK首帧未唱色(白)应不少于末帧: head={w0} tail={w1}");
        achieved.push("④ 卡拉OK \\kf 实渲(hz.karaoke 已唱高亮色随时间推进,首/末帧颜色面分叉)".into());
    }

    // ---- ⑤ 降噪 afftdn(链参数 + HF 频段 ≥3dB 压制 + 时长零漂移) ----
    {
        let dir = workspace("denoise");
        // 源:黑视频 + 响亮 440Hz 正弦压低幅白噪声(降噪的真实工况;音画齐备)
        ff(&[
            "-y", "-v", "error",
            "-f", "lavfi", "-i", "color=c=black:size=320x240:rate=30:duration=2",
            "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
            "-f", "lavfi", "-i", "anoisesrc=colour=white:amplitude=0.008:duration=2",
            "-filter_complex", "[1:a][2:a]amix=inputs=2:normalize=0[a]",
            "-map", "0:v", "-map", "[a]",
            "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest", "noise.mp4",
        ], &dir);
        let mk = |denoise: Value, slug: &str| {
            let mut v = base_project(slug);
            v["tracks"][0]["clips"][0] = json!(
                {"id": "V1-001", "src": "noise.mp4", "startMs": 0, "durationMs": 2000,
                 "volume": 1.0, "denoise": denoise}
            );
            write_project(&dir, &v);
            parse_project(v)
        };
        // 参数链断言:denoise=mid 必须在混音命令产出 afftdn(位置在链首 aformat 后)
        let p_mid = mk(json!("mid"), "dn-chain");
        let args = cutforge_render::steps::mix_pass_a_args(&plan_of(&p_mid, &dir), Path::new("x.m4a"));
        assert!(args.iter().any(|a| a.contains("afftdn=nr=15:nf=-35:tn=1")), "混音链必须含 afftdn(mid)");
        let out_off = cutforge_render::render(&mk(json!("off"), "dn-off"), &dir, None, &mut |_| {}).unwrap();
        let out_high = cutforge_render::render(&mk(json!("high"), "dn-high"), &dir, None, &mut |_| {}).unwrap();
        // 时长零漂移
        let d_off = stream_duration_sec(&out_off.output, "a:0");
        let d_high = stream_duration_sec(&out_high.output, "a:0");
        assert!((d_off - 2.0).abs() < 0.1 && (d_high - 2.0).abs() < 0.1, "降噪时长零漂移: {d_off}/{d_high}");
        // 5kHz 频段(纯噪声区):high 档应显著低于 off
        let hf_off = rms_of_window(&out_off.output, 0.5, 1.5, "highpass=f=3000");
        let hf_high = rms_of_window(&out_high.output, 0.5, 1.5, "highpass=f=3000");
        assert!(hf_high < hf_off - 3.0, "降噪后 HF 噪声频段应压 ≥3dB: off={hf_off} high={hf_high}");
        achieved.push("⑤ 降噪 afftdn(mid 链参数 + 高档 HF 频段 ≥3dB 压制 + 时长零漂移)".into());
    }

    // ---- ⑥ 保速变调(asetrate+aresample+atempo 补偿;与 speed 组合;时长零漂移) ----
    {
        let dir = workspace("pitch");
        make_black_voice(&dir, "black.mp4", 440);
        let mk = |pitch: f64, speed: f64, slug: &str| {
            let mut v = base_project(slug);
            v["tracks"][0]["clips"][0] = json!(
                {"id": "V1-001", "src": "black.mp4", "startMs": 0, "durationMs": 2000,
                 "role": "voice", "volume": 1.0, "pitch": pitch, "speed": speed}
            );
            write_project(&dir, &v);
            parse_project(v)
        };
        // 链参数:+12 半音 = asetrate=96000 + atempo 0.5 补偿(保速)
        let p12 = mk(12.0, 1.0, "pitch+12");
        let args = cutforge_render::steps::mix_pass_a_args(&plan_of(&p12, &dir), Path::new("x.m4a"));
        let j = args.join("\u{1}");
        assert!(j.contains("asetrate=96000"), "+12 半音必须 asetrate=96000: {j}");
        assert!(j.contains("atempo=0.500000"), "变调后必须 atempo 补偿保速: {j}");
        // 与 speed 组合:speed=2 + pitch=+12 → atempo = 2/2 = 1(atempo 链恒等省略)
        let pcombo = mk(12.0, 2.0, "pitch+speed");
        let j = cutforge_render::steps::mix_pass_a_args(&plan_of(&pcombo, &dir), Path::new("x.m4a")).join("\u{1}");
        assert!(j.contains("asetrate=96000") && !j.contains("atempo="), "组合语义: atempo=2/2=1 恒等省略: {j}");
        // 时长零漂移(变调/变调+变速)
        let out = cutforge_render::render(&p12, &dir, None, &mut |_| {}).unwrap();
        let d = stream_duration_sec(&out.output, "a:0");
        assert!((d - 2.0).abs() < 0.1, "变调保速:时长恒 2s,实得 {d}");
        let out2 = cutforge_render::render(&pcombo, &dir, None, &mut |_| {}).unwrap();
        let d2 = stream_duration_sec(&out2.output, "a:0");
        assert!((d2 - 2.0).abs() < 0.1, "变调+变速组合:总时长恒 durationMs,实得 {d2}");
        achieved.push("⑥ 保速变调(+12 链参数 asetrate=96000+atempo 补偿;speed 组合语义;时长零漂移)".into());
    }

    // ---- ⑦ track mute/solo 音频联动(频段能量) ----
    {
        let dir = workspace("tracks");
        ff(&["-y", "-v", "error", "-f", "lavfi", "-i", "sine=frequency=880:duration=2",
             "-c:a", "aac", "a880.m4a"], &dir);
        make_black_voice(&dir, "voice440.mp4", 440);
        let mk = |a1_flags: Value, slug: &str| {
            let mut v = base_project(slug);
            v["tracks"][0]["clips"][0] = json!(
                {"id": "V1-001", "src": "voice440.mp4", "startMs": 0, "durationMs": 2000,
                 "role": "voice", "volume": 1.0}
            );
            let mut a1 = json!({"id": "A1", "kind": "audio", "clips": [
                {"id": "A1-001", "src": "a880.m4a", "startMs": 0, "durationMs": 2000, "volume": 1.0}
            ]});
            if !a1_flags.is_null() {
                for (k, val) in a1_flags.as_object().unwrap() {
                    a1[k] = val.clone();
                }
            }
            v["tracks"].as_array_mut().unwrap().push(a1);
            write_project(&dir, &v);
            parse_project(v)
        };
        let base = cutforge_render::render(&mk(json!(null), "tr-base"), &dir, None, &mut |_| {}).unwrap();
        // mute 音频轨:880 频段应消失
        let muted = cutforge_render::render(&mk(json!({"mute": true}), "tr-mute"), &dir, None, &mut |_| {}).unwrap();
        let b880_base = rms_of_window(&base.output, 0.5, 1.5, "bandpass=f=880:w=60");
        let b880_mute = rms_of_window(&muted.output, 0.5, 1.5, "bandpass=f=880:w=60");
        assert!(b880_mute < b880_base - 6.0, "mute 音频轨后 880Hz 频段应消失: base={b880_base} mute={b880_mute}");
        // solo 音频轨:非 solo 的视频轨(440)应消失;solo 轨 880 保持
        let solo = cutforge_render::render(&mk(json!({"solo": true}), "tr-solo"), &dir, None, &mut |_| {}).unwrap();
        let b440_base = rms_of_window(&base.output, 0.5, 1.5, "bandpass=f=440:w=60");
        let b440_solo = rms_of_window(&solo.output, 0.5, 1.5, "bandpass=f=440:w=60");
        assert!(b440_solo < b440_base - 6.0, "solo 音频轨后非 solo(440Hz)应消失: base={b440_base} solo={b440_solo}");
        let b880_solo = rms_of_window(&solo.output, 0.5, 1.5, "bandpass=f=880:w=60");
        assert!(b880_solo >= b880_base - 3.0, "solo 轨 880Hz 保持: base={b880_base} solo={b880_solo}");
        achieved.push("⑦ track mute/solo 音频联动(mute 轨频段消失 ≥6dB;solo 独占,非 solo 频段消失)".into());
    }

    // ---- ⑧ track hidden 视频联动(视觉面) ----
    {
        let dir = workspace("hidden");
        ff(&["-y", "-v", "error", "-f", "lavfi", "-i", "color=c=red:size=320x240:rate=30:duration=2",
             "-c:v", "libx264", "-preset", "veryfast", "red.mp4"], &dir);
        ff(&["-y", "-v", "error", "-f", "lavfi", "-i", "color=c=blue:size=320x240:rate=30:duration=2",
             "-c:v", "libx264", "-preset", "veryfast", "blue.mp4"], &dir);
        let mut v = base_project("tr-hidden");
        v["tracks"][0]["hidden"] = json!(true);
        v["tracks"][0]["clips"] = json!([
            {"id": "V1-001", "src": "red.mp4", "startMs": 0, "durationMs": 2000, "volume": 0}
        ]);
        v["tracks"].as_array_mut().unwrap().push(json!(
            {"id": "V2", "kind": "video", "clips": [
                {"id": "V2-001", "src": "blue.mp4", "startMs": 0, "durationMs": 2000, "volume": 0}
            ]}
        ));
        write_project(&dir, &v);
        let p = parse_project(v);
        let out = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let f = frame_bytes(&out.output, 1.0);
        let i = (120 * 320 + 160) * 3;
        let (r, _g, b) = (f[i], f[i + 1], f[i + 2]);
        assert!(b > 150 && r < 110, "hidden 视频轨不进合成(只见蓝): ({r},{_g},{b})");
        assert_eq!(stream_duration_sec(&out.output, "v:0"), 2.0);
        achieved.push("⑧ track hidden 视频联动(隐藏轨红场被排除,合成面只剩可见轨蓝场)".into());
    }

    // ---- ⑨ useProxy 渲染分叉(代理存在 → seg 分键 + 预览成片仍产出) ----
    {
        let dir = workspace("proxy");
        make_black(&dir, "black.mp4");
        let v = base_project("use-proxy");
        write_project(&dir, &v);
        let p = parse_project(v);
        let (mtime, size) = cutforge_io::mediacache::source_stamp(&dir.join("black.mp4")).unwrap();
        let proxy_rel = cutforge_io::mediacache::proxy_rel("black.mp4", mtime, size);
        std::fs::create_dir_all(dir.join(".cutforge/proxy")).unwrap();
        ff(&["-y", "-v", "error", "-i", "black.mp4", "-vf", "scale=160:120",
             "-c:v", "libx264", "-preset", "veryfast", &proxy_rel], &dir);
        let out_orig = cutforge_render::render(&p, &dir, None, &mut |_| {}).unwrap();
        let out_proxy = cutforge_render::render_with(&p, &dir, None, true, &mut |_| {}).unwrap();
        assert!(out_orig.output.is_file() && out_proxy.output.is_file());
        let segs = std::fs::read_dir(dir.join(".cutforge/render-cache/seg")).unwrap().count();
        assert!(segs >= 2, "代理/原片必须分键(seg 层 ≥2 份): {segs}");
        achieved.push("⑨ useProxy 代理预览(存在代理→分键渲染;opt-in 不悄悄降质)".into());
    }

    println!("A4-BE3b 文本/音频 parity achieved {} 项:", achieved.len());
    for a in &achieved {
        println!("  ✅ {a}");
    }
    assert!(achieved.len() >= 9, "九项夹具全跑");
}
