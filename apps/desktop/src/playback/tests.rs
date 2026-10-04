//! PlaybackEngine 门面集成测试(需 ffmpeg/ffprobe;缺失时优雅跳过)。

use super::test_support::{cleanup, find_tool, gen_testsrc2, tmp_dir};
use super::{DecodedFrame, PlaybackEngine, PlaybackError};
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

/// ffmpeg/ffprobe 任一缺失则跳过(工单:优雅跳过,打印原因)
fn find_tools() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let ffmpeg = find_tool("CUTFORGE_FFMPEG", "ffmpeg");
    let ffprobe = find_tool("CUTFORGE_FFPROBE", "ffprobe");
    match (ffmpeg, ffprobe) {
        (Some(f), Some(p)) => Some((f, p)),
        _ => {
            println!(
                "skip: 未找到可用的 ffmpeg/ffprobe(设 CUTFORGE_FFMPEG/CUTFORGE_FFPROBE 或入 PATH)"
            );
            None
        }
    }
}

#[test]
fn load_不存在的文件_进入故障态且不panic() {
    let Some((ffmpeg, ffprobe)) = find_tools() else {
        return;
    };
    let mut e = PlaybackEngine::new(ffmpeg, ffprobe).expect("工具已验证,new 应成功");
    let err = e
        .load_clip(Path::new("Z:/cf-不存在-9x9.mp4"), 0.0, 2_000.0, 30.0)
        .unwrap_err();
    assert!(matches!(err, PlaybackError::Probe(_)), "实际 {err:?}");
    assert!(e.is_faulted());
    assert!(matches!(e.last_error(), Some(PlaybackError::Probe(_))));
    assert!(e.poll_frame().is_none(), "故障态不出帧");
    // 故障后 API 全部 no-op,不 panic
    e.play();
    e.pause();
    e.set_speed(2.0);
    e.set_muted(true);
    e.seek_ms(100.0);
    let _ = e.position_ms();
    e.shutdown();
    assert!(!e.is_faulted(), "shutdown 回 Idle");
}

#[test]
fn 加载真实片段_播放_出帧_时钟前进_pts单调() {
    let Some((ffmpeg, ffprobe)) = find_tools() else {
        return;
    };
    let dir = tmp_dir("engine");
    let src = gen_testsrc2(&ffmpeg, &dir);
    let mut e = PlaybackEngine::new(ffmpeg, ffprobe).expect("new 应成功");
    e.load_clip(&src, 0.0, 2_000.0, 30.0).expect("load 应成功");
    assert!(!e.is_playing(), "加载后默认暂停");
    assert!(!e.is_faulted());

    e.play();
    assert!(e.is_playing());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut frames = 0usize;
    let mut last_pts = f64::MIN;
    while Instant::now() < deadline && frames < 30 {
        if let Some(f) = e.poll_frame() {
            assert_eq!(f.width, 320);
            assert_eq!(f.height, 240);
            assert_eq!(f.rgba.len(), 320 * 240 * 4);
            assert!(
                f.pts_ms > last_pts,
                "pts 必须单调 {last_pts} → {}",
                f.pts_ms
            );
            last_pts = f.pts_ms;
            frames += 1;
        } else {
            let _ = e.position_ms(); // 泵时钟(无音轨 → 墙钟路径)
            sleep(Duration::from_millis(2));
        }
    }
    assert!(frames >= 30, "至少应出 30 帧,实际 {frames}");
    let pos = e.position_ms();
    assert!(pos > 0.0, "播放中位置应前进,实际 {pos}");
    assert!(e.last_error().is_none());
    e.shutdown();
    cleanup(&dir);
}

#[test]
fn seek_重锚_位置立即生效() {
    let Some((ffmpeg, ffprobe)) = find_tools() else {
        return;
    };
    let dir = tmp_dir("seek");
    let src = gen_testsrc2(&ffmpeg, &dir);
    let mut e = PlaybackEngine::new(ffmpeg, ffprobe).expect("new 应成功");
    e.load_clip(&src, 0.0, 2_000.0, 30.0).expect("load 应成功");
    e.seek_ms(1_000.0);
    let pos = e.position_ms();
    assert!(
        (pos - 1_000.0).abs() < 50.0,
        "seek 后位置应≈1000ms,实际 {pos}"
    );
    // seek 出界夹取
    e.seek_ms(9_999.0);
    let pos = e.position_ms();
    assert!((pos - 2_000.0).abs() < 50.0, "越界应夹到 out_ms,实际 {pos}");
    e.shutdown();
    cleanup(&dir);
}

/// 播完自动暂停:位置钉在 out_ms,不越过(墙钟无音轨路径)
#[test]
fn 播完自动暂停_位置钉在片段尾() {
    let Some((ffmpeg, ffprobe)) = find_tools() else {
        return;
    };
    let dir = tmp_dir("end");
    let src = gen_testsrc2(&ffmpeg, &dir);
    let mut e = PlaybackEngine::new(ffmpeg, ffprobe).expect("new 应成功");
    e.load_clip(&src, 0.0, 600.0, 30.0).expect("load 应成功");
    e.play();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut pos = 0.0;
    while Instant::now() < deadline {
        pos = e.position_ms();
        if !e.is_playing() {
            break;
        }
        sleep(Duration::from_millis(20));
    }
    assert!(!e.is_playing(), "到尾应自动暂停");
    assert!(
        (pos - 600.0).abs() < 50.0,
        "位置应钉在 out_ms 附近,实际 {pos}"
    );
    assert!(!e.is_faulted(), "播完是正常停,不是故障");
    e.shutdown();
    cleanup(&dir);
}

/// DecodedFrame 构造面编译期可见性冒烟(不 spawn 任何东西)
#[test]
fn 门面类型冒烟() {
    let f = DecodedFrame {
        rgba: std::sync::Arc::new(vec![0; 4]),
        width: 1,
        height: 1,
        pts_ms: 0.0,
    };
    assert_eq!(f.rgba.len(), 4);
    let err = PlaybackError::Stalled;
    assert_eq!(err.to_string(), "解码停流(3s 无输出,已杀进程)");
}

/// 回归(I1 修复:环满背压):慢消费(50ms/帧)不得丢头部帧。
/// 旧"丢最旧"策略下,解码远快于消费 → 24 帧环灌满 → 头部帧被挤掉,
/// 播放头在第 0 帧位置却拿到第 ~24 帧内容("钟走帧停"并提前耗尽);
/// 背压后解码速度被拉平到消费速度,首帧必为第 0 帧,全帧到达,无停流误报。
#[test]
fn 慢消费背压_不丢头部帧_总数齐全_无停流误报() {
    let Some((ffmpeg, ffprobe)) = find_tools() else {
        return;
    };
    let dir = tmp_dir("pace");
    let src = gen_testsrc2(&ffmpeg, &dir);
    let mut e = PlaybackEngine::new(ffmpeg, ffprobe).expect("new 应成功");
    e.load_clip(&src, 0.0, 2_000.0, 30.0).expect("load 应成功");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut pts_seq: Vec<f64> = Vec::new();
    let mut idle_ms = 0u64; // 连续无帧时长(播完后安全退出,不等满 20s)
    while Instant::now() < deadline && pts_seq.len() < 60 {
        if let Some(f) = e.poll_frame() {
            idle_ms = 0;
            if pts_seq.is_empty() {
                assert!(
                    f.pts_ms < 1.0,
                    "首帧必须是第 0 帧(pts≈0),实际 pts={}(头部帧被丢弃)",
                    f.pts_ms
                );
            }
            pts_seq.push(f.pts_ms);
            sleep(Duration::from_millis(50)); // 慢消费,制造环满背压
        } else {
            let _ = e.position_ms(); // 泵时钟(无音轨 → 墙钟路径)
            sleep(Duration::from_millis(5));
            idle_ms += 5;
            if idle_ms > 2_000 {
                break;
            }
        }
    }
    assert!(
        !e.is_faulted(),
        "慢消费背压不得触发停流误报,实际 {:?}",
        e.last_error()
    );
    assert!(e.last_error().is_none());
    assert_eq!(
        pts_seq.len(),
        60,
        "2s@30fps 全部 60 帧应到达(无丢帧/提前 EOF)"
    );
    assert!(
        pts_seq.windows(2).all(|w| w[1] > w[0]),
        "pts 必须按帧序严格递增(无重复/乱序)"
    );
    e.shutdown();
    cleanup(&dir);
}
