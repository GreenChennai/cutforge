//! audio 单元测试(回调逻辑无声卡;真机与诊断项 #[ignore])。

use super::lock_or_recover;
use super::{AudioShared, AudioStream, CHANNELS, PRIME_FRAMES, fill_output};
use crate::playback::PlaybackError;
use crate::playback::test_support::{cleanup, find_tool, tmp_dir};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::sleep;
use std::time::Duration;

/// 纯回调单测(无声卡):预缓冲前静音等待,不消费不计数
#[test]
fn 回调_预缓冲前静音不消费() {
    let shared = Arc::new(Mutex::new(AudioShared::new()));
    let consumed = AtomicU64::new(0);
    let muted = AtomicBool::new(false);
    let primed = AtomicBool::new(false);
    let cond = Condvar::new();
    let mut out = vec![1.0f32; 1024];
    fill_output(&mut out, &shared, &cond, &consumed, &muted, &primed);
    assert!(out.iter().all(|&s| s.abs() < 1e-6), "预缓冲前应全静音");
    assert_eq!(consumed.load(Ordering::Relaxed), 0, "不消费");
    assert_eq!(lock_or_recover(&shared).underruns, 0, "预缓冲等待不算欠载");
}

/// 纯回调单测:预缓冲达标后正常填充与欠载静音
#[test]
fn 回调_预缓冲后正常填充_欠载写静音() {
    let shared = Arc::new(Mutex::new(AudioShared::new()));
    let consumed = AtomicU64::new(0);
    let muted = AtomicBool::new(false);
    let primed = AtomicBool::new(false);
    let cond = Condvar::new();
    {
        let mut st = lock_or_recover(&shared);
        for i in 0..(PRIME_FRAMES as usize + 256) * CHANNELS {
            let _ = st.ring.push((i % 7) as f32 * 0.1); // 测试注水:容量足够,不会拒绝
        }
    }
    let mut out = vec![0f32; 1024 * CHANNELS];
    fill_output(&mut out, &shared, &cond, &consumed, &muted, &primed);
    assert_eq!(consumed.load(Ordering::Relaxed), 1024);
    assert!(out[0].abs() < 1e-6, "第一个样本应为入队的 0.0");
    assert!(out.iter().any(|&s| s.abs() > 1e-6), "应填入真实样本");

    // 排空后回调 → 欠载静音,不消费
    {
        let mut st = lock_or_recover(&shared);
        while st.ring.pop().is_some() {}
    }
    let mut out2 = vec![0.5f32; 512 * CHANNELS];
    fill_output(&mut out2, &shared, &cond, &consumed, &muted, &primed);
    assert!(out2.iter().all(|&s| s.abs() < 1e-6), "欠载应写静音");
    assert_eq!(lock_or_recover(&shared).underruns, 1, "欠载计数 +1");
    assert_eq!(consumed.load(Ordering::Relaxed), 1024, "欠载不消费");
}

/// 纯回调单测:静音 = 丢弃式消费(时钟照走,解除静音不回跳)
#[test]
fn 回调_静音丢弃式消费() {
    let shared = Arc::new(Mutex::new(AudioShared::new()));
    let consumed = AtomicU64::new(0);
    let muted = AtomicBool::new(true);
    let primed = AtomicBool::new(true); // 直接置已预缓冲
    let cond = Condvar::new();
    {
        let mut st = lock_or_recover(&shared);
        for i in 0..4096 * CHANNELS {
            let _ = st.ring.push((i % 5) as f32 * 0.2); // 测试注水:容量足够,不会拒绝
        }
    }
    let mut out = vec![0f32; 512 * CHANNELS];
    fill_output(&mut out, &shared, &cond, &consumed, &muted, &primed);
    assert!(out.iter().all(|&s| s.abs() < 1e-6), "静音输出全零");
    assert_eq!(consumed.load(Ordering::Relaxed), 512, "丢弃式消费照常推进");
    assert_eq!(lock_or_recover(&shared).ring.len(), (4096 - 512) * CHANNELS);
}

/// 真机验收(本机执行一次):默认设备出声、消费计数前进、无故障
#[test]
#[ignore = "需要真实声卡与 ffmpeg;验收跑 cargo test -p cutforge-desktop --ignored"]
fn 真机_默认设备出声_消费计数前进() {
    let Some(ffmpeg) = find_tool("CUTFORGE_FFMPEG", "ffmpeg") else {
        println!("skip: 未找到可用的 ffmpeg(设 CUTFORGE_FFMPEG 或入 PATH)");
        return;
    };
    let dir = tmp_dir("aud");
    let wav = dir.join("cf-sine-1s.wav");
    let out = std::process::Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-c:a",
            "pcm_s16le",
            "-y",
        ])
        .arg(&wav)
        .output()
        .expect("ffmpeg 生成正弦素材失败");
    assert!(out.status.success(), "生成正弦素材失败");

    // 无可用音频设备的机器(远桌面/CI/驱动故障)优雅跳过:
    // 引擎语义本就是 AudioUnavailable → 回落墙钟,真机断言仅在有声卡时执行
    let a = match AudioStream::start(&ffmpeg, &wav, 0.0, 1.0) {
        Ok(a) => a,
        Err(PlaybackError::AudioUnavailable(m)) => {
            println!("skip: 本机无可用音频设备({m});接真实声卡后重跑可完成真机断言");
            cleanup(&dir);
            return;
        }
        Err(e) => panic!("打开音频设备出现非预期错误:{e}"),
    };
    sleep(Duration::from_millis(500));
    let consumed = a.consumed_samples();
    assert!(
        consumed >= 12_000,
        "500ms 应至少消费 ~250ms 样本(12000),实际 {consumed}"
    );
    assert_eq!(a.fault(), None, "真机播放不应有故障");
    let underruns = a.underruns();
    assert!(
        underruns < 200,
        "预缓冲后欠载应很少,实际 {underruns}(可含起播瞬间)"
    );
    drop(a);
    cleanup(&dir);
}

/// 诊断(接线方排障用):枚举 cpal 可见音频主机与输出设备。
/// 无声卡/远桌面会话等环境下 default_output_device 可能为 None,
/// 引擎语义为 AudioUnavailable → 回落墙钟,不视为缺陷。
#[test]
#[ignore = "诊断用:打印 cpal 主机与设备清单"]
fn 诊断_枚举cpal音频主机与设备() {
    use cpal::traits::HostTrait;
    println!("default_host = {:?}", cpal::default_host().id());
    let host = cpal::default_host();
    println!(
        "default_output_device = {:?}",
        host.default_output_device().map(|d| d.to_string())
    );
    let mut n = 0;
    match host.output_devices() {
        Ok(devs) => {
            for d in devs {
                n += 1;
                println!("  output[{n}] = {d}");
            }
        }
        Err(e) => println!("output_devices 枚举失败:{e}"),
    }
    println!("output_devices total = {n}");
}
