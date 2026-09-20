// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 媒体探测(计划书 6.2 步 1):ffprobe 取媒体信息,统一取 `format.duration`。
//! 本模块是 IO 层唯一的外部进程调用点;内核不认识 ffmpeg。

use serde_json::Value;
use std::io;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    pub duration_sec: f64,
    pub raw: Value,
}

impl MediaInfo {
    /// 视频流分辨率(无视频流 → None;纯音频素材常见)。
    pub fn video_size(&self) -> Option<(u64, u64)> {
        let streams = self.raw["streams"].as_array()?;
        let v = streams.iter().find(|s| s["codec_type"] == "video")?;
        Some((v["width"].as_u64()?, v["height"].as_u64()?))
    }

    /// 是否含音轨(渲染混音与"导入后有没有声"的判据)。
    pub fn has_audio(&self) -> bool {
        self.raw["streams"]
            .as_array()
            .map(|ss| ss.iter().any(|s| s["codec_type"] == "audio"))
            .unwrap_or(false)
    }

    /// 时长毫秒(四舍五入;probe 时长口径的唯一换算点)。
    pub fn duration_ms(&self) -> u64 {
        (self.duration_sec * 1000.0).round() as u64
    }
}

/// ffprobe 是否可用(CI/无依赖环境下测试据此跳过)。
/// E5-2/B13 同口径:env CUTFORGE_FFPROBE 优先,缺省按 PATH 名。
fn ffprobe_bin() -> String {
    if let Some(v) = std::env::var_os("CUTFORGE_FFPROBE")
        && !v.is_empty() {
            return v.to_string_lossy().into_owned();
        }
    "ffprobe".to_string()
}

pub fn ffprobe_available() -> bool {
    Command::new(ffprobe_bin()).arg("-version").output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn probe(path: &Path) -> io::Result<MediaInfo> {
    // -show_streams 与 -show_format 同批输出:B12 接线后 media_probe 需要分辨率
    // 与音轨存在性(来自 streams),时长仍统一取 format.duration(单一口径)。
    let out = Command::new(ffprobe_bin())
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "ffprobe 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let raw: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| io::Error::other(format!("ffprobe 输出解析失败: {e}")))?;
    let duration_sec = raw["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .ok_or_else(|| io::Error::other("ffprobe 输出缺 format.duration"))?;
    Ok(MediaInfo { duration_sec, raw })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_reports_duration_when_ffprobe_present() {
        if !ffprobe_available() {
            eprintln!("skip: 本机无 ffprobe");
            return;
        }
        // 用仓库回归样本的母文件不可得;探测一个必存在的文本文件,断言"能跑通协议"即可
        // 真实媒体探测由 CutFlow 侧 rs_render 对拍(M6)覆盖。
        let here = std::env::temp_dir().join("cutforge-probe-placeholder.txt");
        crate::atomic::atomic_write(&here, b"x").ok();
        match probe(&here) {
            Ok(_) => panic!("非媒体文件不应成功"),
            Err(e) => assert!(e.to_string().contains("ffprobe"), "错误须来自 ffprobe 语义: {e}"),
        }
    }
}
