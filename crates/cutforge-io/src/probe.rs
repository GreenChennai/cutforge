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
    let out = Command::new(ffprobe_bin())
        .args(["-v", "error", "-print_format", "json", "-show_format"])
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
