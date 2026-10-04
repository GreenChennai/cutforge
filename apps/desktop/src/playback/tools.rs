//! 工具面公共设施(I1 播放引擎):ffprobe 媒体预探 + ffmpeg/ffprobe 可执行性验证。
//!
//! 纪律:工具路径由调用方注入,本模块**不读环境变量**(可测性);
//! 所有等待均有上限(探测 2s / 验证 3s),超时强杀收尸,不留僵尸。

use super::PlaybackError;
use super::subprocess;
use serde_json::Value;
use std::io::{BufReader, Read};
use std::path::Path;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

/// ffprobe 预探超时
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// 媒体预探结果(单次 ffprobe -show_streams 取视频尺寸与音轨存在性)
#[derive(Debug)]
pub(crate) struct ProbeInfo {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) has_audio: bool,
}

/// ffprobe 预探(2s 超时):首个视频流宽高 + 音轨存在性。单次调用,无缓存。
pub(crate) fn probe_media(ffprobe: &Path, src: &Path) -> Result<ProbeInfo, PlaybackError> {
    let mut child = subprocess::command(ffprobe)
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(src)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            PlaybackError::Probe(format!("ffprobe 启动失败({}): {e}", ffprobe.display()))
        })?;
    let stdout = child.stdout.take().expect("stdout 已设 piped");
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::Builder::new()
        .name("cf-probe".into())
        .spawn(move || {
            let mut s = String::new();
            let _ = BufReader::new(stdout).read_to_string(&mut s);
            let _ = tx.send(s);
        })
        .ok();

    let json = match rx.recv_timeout(PROBE_TIMEOUT) {
        Ok(json) => {
            // stdout 已 EOF,进程必在退出边缘;仍设收尸上限防极端悬挂
            reap_with_cap(&mut child, Duration::from_secs(1));
            json
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(PlaybackError::Probe(format!(
                "ffprobe 探测超时({:?}):{}",
                PROBE_TIMEOUT,
                src.display()
            )));
        }
    };

    let v: Value = serde_json::from_str(json.trim())
        .map_err(|e| PlaybackError::Probe(format!("ffprobe 输出解析失败:{e}")))?;
    let streams = v
        .get("streams")
        .and_then(Value::as_array)
        .ok_or_else(|| PlaybackError::Probe(format!("ffprobe 无 streams:{}", src.display())))?;
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .ok_or_else(|| PlaybackError::Probe(format!("无视频流:{}", src.display())))?;
    let width = video["width"].as_u64().unwrap_or(0) as u32;
    let height = video["height"].as_u64().unwrap_or(0) as u32;
    if width == 0 || height == 0 {
        return Err(PlaybackError::Probe(format!(
            "视频流宽高非法:{width}x{height}({})",
            src.display()
        )));
    }
    Ok(ProbeInfo {
        width,
        height,
        has_audio: streams.iter().any(|s| s["codec_type"] == "audio"),
    })
}

/// 工具可执行性验证:spawn `-version` 并等退出(3s 上限,超时强杀)。
/// 只验证「能跑」,不解析版本号。
pub(crate) fn verify_tool(path: &Path, label: &str) -> Result<(), PlaybackError> {
    let mut child = subprocess::command(path)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            PlaybackError::ToolMissing(format!("{label} 不可执行({}): {e}", path.display()))
        })?;
    let started = Instant::now();
    loop {
        if child.try_wait().is_ok_and(|o| o.is_some()) {
            return Ok(());
        }
        if started.elapsed() >= Duration::from_secs(3) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(PlaybackError::ToolMissing(format!(
                "{label} 执行超时({})",
                path.display()
            )));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// 有界收尸:超时强杀(避免 wait 无限阻塞)
fn reap_with_cap(child: &mut Child, cap: Duration) {
    let started = Instant::now();
    while started.elapsed() < cap {
        if child.try_wait().map(|o| o.is_some()).unwrap_or(true) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
}
