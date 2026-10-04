//! decoder 集成测试(需 ffmpeg/ffprobe;缺失时优雅跳过并打印原因)。

use super::{StreamFault, VideoStream, preview_dims};
use crate::playback::PlaybackError;
use crate::playback::subprocess::STALL_TIMEOUT;
use crate::playback::test_support::{
    cleanup, find_tool, gen_testsrc2, gen_testsrc2_sized, tmp_dir,
};
use crate::playback::tools::probe_media;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// preview_dims 纯函数:短边对齐 540、偶数化、小档源不缩
#[test]
fn preview尺寸_短边对齐与偶数化() {
    // 16:9 主流档
    assert_eq!(preview_dims(1920, 1080), (960, 540));
    assert_eq!(preview_dims(3840, 2160), (960, 540));
    assert_eq!(preview_dims(1280, 720), (960, 540));
    // 竖屏:短边(宽)等比缩到 540,长边 960
    assert_eq!(preview_dims(1080, 1920), (540, 960));
    // 4:3:短边 480 已在档内 → 不缩
    assert_eq!(preview_dims(640, 480), (640, 480));
    // 奇数归偶:1366×768 → s=0.703125 → 960.47/540 → 960×540
    assert_eq!(preview_dims(1366, 768), (960, 540));
    // 2560×1080(21:9):短边 1080 → s=0.5 → 1280×540(等比,宽略超档属预期)
    assert_eq!(preview_dims(2560, 1080), (1280, 540));
}

#[test]
fn testsrc2_两秒_解码帧数与尺寸正确() {
    let Some(ffmpeg) = find_tool("CUTFORGE_FFMPEG", "ffmpeg") else {
        println!("skip: 未找到可用的 ffmpeg(设 CUTFORGE_FFMPEG 或入 PATH)");
        return;
    };
    let dir = tmp_dir("gen");
    let src = gen_testsrc2(&ffmpeg, &dir);
    let vs = VideoStream::start(&ffmpeg, &src, 0.0, 2.0, 320, 240).expect("起解码流");
    assert_eq!(vs.width(), 320);
    assert_eq!(vs.height(), 240);

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut popped = 0usize;
    while Instant::now() < deadline {
        match vs.pop_frame() {
            Some(f) => {
                assert_eq!(f.len(), 320 * 240 * 4, "帧字节数必须等于 w*h*4");
                popped += 1;
            }
            None if vs.finished() => break,
            None => sleep(Duration::from_millis(2)),
        }
    }
    assert!(
        popped >= 30,
        "2s@30fps 至少应消费 30 帧,实际 {popped}(背压策略下应全量到达)"
    );
    assert_eq!(vs.fault(), None, "正常流不应有故障");
    drop(vs); // 触发 kill+wait,不留僵尸
    cleanup(&dir);
}

/// 回归(preview 缩放解码):1080p 源解码输出必须缩到 960×540,
/// 帧内存 8.3MB → 2.0MB(消除长跑堆分配压力);DecodedFrame 尺寸口径同源。
#[test]
fn preview缩放_1080p源输出960x540() {
    let Some(ffmpeg) = find_tool("CUTFORGE_FFMPEG", "ffmpeg") else {
        println!("skip: 未找到可用的 ffmpeg(设 CUTFORGE_FFMPEG 或入 PATH)");
        return;
    };
    let dir = tmp_dir("scale");
    let src = gen_testsrc2_sized(&ffmpeg, &dir, "1920x1080", "1");
    let vs = VideoStream::start(&ffmpeg, &src, 0.0, 1.0, 1920, 1080).expect("起解码流");
    assert_eq!(vs.width(), 960, "1080p 源应缩放到预览档宽 960");
    assert_eq!(vs.height(), 540, "1080p 源应缩放到预览档高 540");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut popped = 0usize;
    while Instant::now() < deadline {
        match vs.pop_frame() {
            Some(f) => {
                assert_eq!(f.len(), 960 * 540 * 4, "缩放后帧字节数必须等于 w*h*4");
                popped += 1;
            }
            None if vs.finished() => break,
            None => sleep(Duration::from_millis(2)),
        }
    }
    assert!(
        popped >= 15,
        "1s@30fps 至少应消费 15 帧,实际 {popped}(背压策略下应全量到达)"
    );
    assert_eq!(vs.fault(), None, "缩放流不应有故障");
    drop(vs);
    cleanup(&dir);
}

/// 回归(缩放幂等):preview 尺寸再进 start 不得二次缩放(引擎 seek 换流路径)
#[test]
fn preview缩放_对预览尺寸幂等() {
    let Some(ffmpeg) = find_tool("CUTFORGE_FFMPEG", "ffmpeg") else {
        println!("skip: 未找到可用的 ffmpeg(设 CUTFORGE_FFMPEG 或入 PATH)");
        return;
    };
    let dir = tmp_dir("scale-idem");
    let src = gen_testsrc2_sized(&ffmpeg, &dir, "960x540", "1");
    let vs = VideoStream::start(&ffmpeg, &src, 0.0, 1.0, 960, 540).expect("起解码流");
    assert_eq!(vs.width(), 960);
    assert_eq!(vs.height(), 540);
    assert_eq!(vs.fault(), None);
    drop(vs);
    cleanup(&dir);
}

#[test]
fn 停流看门狗_3秒无字节_杀进程置故障() {
    let Some(ffmpeg) = find_tool("CUTFORGE_FFMPEG", "ffmpeg") else {
        println!("skip: 未找到可用的 ffmpeg(设 CUTFORGE_FFMPEG 或入 PATH)");
        return;
    };
    // rawvideo 管道输入且永不喂字节:ffmpeg 等输入,无任何输出 → 纯停流
    let args: Vec<String> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-video_size",
        "64x64",
        "-framerate",
        "10",
        "-i",
        "pipe:0",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "pipe:1",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let mut vs =
        VideoStream::spawn_raw(&ffmpeg, &args, 64, 64, Stdio::piped()).expect("起停流会话");
    let _stdin_hold = vs.take_stdin(); // 保住管道:EOF 会让 ffmpeg 正常退出

    let deadline = Instant::now() + STALL_TIMEOUT + Duration::from_secs(4);
    while Instant::now() < deadline && vs.fault().is_none() {
        sleep(Duration::from_millis(100));
    }
    assert!(
        matches!(vs.fault(), Some(StreamFault::Stalled)),
        "3s 无字节应置 Stalled,实际 {:?}",
        vs.fault()
    );
    assert!(vs.child_exited(), "看门狗必须已杀进程并 wait 收尸");
}

#[test]
fn probe_不存在的文件_快败返回探测错误() {
    let Some(ffprobe) = find_tool("CUTFORGE_FFPROBE", "ffprobe") else {
        println!("skip: 未找到可用的 ffprobe(设 CUTFORGE_FFPROBE 或入 PATH)");
        return;
    };
    let started = Instant::now();
    let err = probe_media(&ffprobe, Path::new("Z:/cf-不存在-9x9.mp4")).unwrap_err();
    assert!(matches!(err, PlaybackError::Probe(_)), "实际 {err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "坏路径应快败而非等超时"
    );
}

#[test]
fn probe_正常素材_尺寸与音轨判定() {
    let Some(ffmpeg) = find_tool("CUTFORGE_FFMPEG", "ffmpeg") else {
        println!("skip: 未找到可用的 ffmpeg");
        return;
    };
    let Some(ffprobe) = find_tool("CUTFORGE_FFPROBE", "ffprobe") else {
        println!("skip: 未找到可用的 ffprobe");
        return;
    };
    let dir = tmp_dir("probe");
    let src = gen_testsrc2(&ffmpeg, &dir);
    let info = probe_media(&ffprobe, &src).expect("probe 应成功");
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 240);
    assert!(!info.has_audio, "testsrc2 纯视频素材不应有音轨");
    cleanup(&dir);
}

/// 生成素材的 ffmpeg 与解码 ffmpeg 同源;此测试顺带锁 mpeg4 编码可用性
#[test]
fn 生成素材子命令_状态非零时报错带stderr() {
    let Some(ffmpeg) = find_tool("CUTFORGE_FFMPEG", "ffmpeg") else {
        println!("skip: 未找到可用的 ffmpeg");
        return;
    };
    let dir = tmp_dir("badgen");
    let out = Command::new(&ffmpeg)
        .args(["-f", "lavfi", "-i", "不存在源", "-t", "1", "-y"])
        .arg(dir.join("bad.mp4"))
        .output()
        .expect("ffmpeg 可执行(已由 find_tool 验证)");
    assert!(!out.status.success(), "非法 lavfi 源应失败");
    assert!(!out.stderr.is_empty(), "stderr 应有归因信息");
    cleanup(&dir);
}
