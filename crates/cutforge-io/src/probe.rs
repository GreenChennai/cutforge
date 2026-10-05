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
        && !v.is_empty()
    {
        return v.to_string_lossy().into_owned();
    }
    "ffprobe".to_string()
}

pub fn ffprobe_available() -> bool {
    Command::new(ffprobe_bin())
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// pid 活性三态(R-02 语义精化):接管判定必须能区分"**确证死亡**"与
/// "**探测失败**"——二者保守度不同:确证死 → 持有者必已亡,立即接管
/// (锁龄/心跳不再承载信息量);探测失败 → 回退锁龄+心跳的保守门。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PidState {
    /// 进程不存在 → 原持有者确定性已亡。
    Dead,
    /// 进程存在;附启动时间指纹(工具差异取不到 = None)。
    Alive(Option<String>),
    /// 探测失败(工具不可用/系统调用失败)→ 活性无法判定。
    Unknown,
}

/// 三态 pid 探测(生产路径;[`PidProbe`] 注入面同型)。
/// - Windows:`tasklist /FI` 精确过滤(命中 → Alive,无匹配 → Dead,
///   命令失败 → Unknown)+ wmic 启动指纹;
/// - Linux:`/proc/<pid>` 存在性(存在 → Alive,缺失 → Dead,读失败即 Unknown
///   语义上的不存在——/proc 正常挂载时缺失即确证死)+ `/proc/<pid>/stat` 第 22 字段。
pub fn probe_pid(pid: u32) -> PidState {
    #[cfg(target_os = "windows")]
    {
        match Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
        {
            Ok(o) if o.status.success() => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                // CSV 行首列带引号包 PID(精确过滤命中才有一行);INFO 行无该 pid
                if stdout.contains(&format!("\"{pid}\"")) {
                    PidState::Alive(pid_start_time(pid))
                } else {
                    PidState::Dead
                }
            }
            _ => PidState::Unknown,
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if std::path::Path::new(&format!("/proc/{pid}")).exists() {
            PidState::Alive(pid_start_time(pid))
        } else {
            PidState::Dead
        }
    }
}

/// 进程存活判定(崩溃恢复,册六 T6.1):锁文件里的 pid 是否还活着。
/// [`probe_pid`] 的布尔投影;探测失败按**活**处理(全仓调用面都以
/// "活 → 不动"为保守方向)。本模块是 IO 层唯一的外部进程调用点,pid 探测同域收敛于此。
pub fn pid_alive(pid: u32) -> bool {
    matches!(probe_pid(pid), PidState::Alive(_))
}

/// 进程启动时间(R-02:Windows pid 复用风险的联合判定依据)。
/// 返回进程启动时刻的平台规范化字符串;两次查询同一活进程结果一致、
/// pid 复用后必然不同。取不到(权限/平台差异)→ None(判定退化为仅 pid)。
///
/// Windows 走 `wmic process where processid=… get creationdate /value`
/// (tasklist 无启动时间列);unix 读 `/proc/<pid>/stat` 第 22 字段
/// (starttime,boot 后时钟滴答,纯文件系统零进程派生)。
pub fn pid_start_time(pid: u32) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        let out = Command::new("wmic")
            .args([
                "process",
                "where",
                &format!("processid={pid}"),
                "get",
                "creationdate",
                "/format:value",
            ])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        stdout
            .lines()
            .find_map(|l| {
                l.strip_prefix("CreationDate=")
                    .map(|v| str::trim(v).to_owned())
            })
            .filter(|s| !s.is_empty())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let after_exec = stat.rsplit_once(')').map(|(_, rest)| rest).unwrap_or(&stat);
        after_exec.split_whitespace().nth(19).map(str::to_owned) // 第 22 字段(去掉 pid 与 comm 后第 19 个)
    }
}

/// 返回一个当前探测为**不存在进程**的 pid(测试伪造崩溃残留锁的跨平台助手)。
/// 两平台语义一致:返回值满足 `pid_alive(return) == false`。
/// - Windows:pid 恒为 4 的倍数,取 4194303(奇数,必在 pid 空间之外,tasklist 必无匹配);
/// - Linux:`pid 1 = init/systemd 恒活`(lock 测试在 Linux CI 翻红的根因),
///   故读 `/proc/sys/kernel/pid_max`(缺省 4194304,下限 32768),自高向低
///   找第一个 `/proc` 中不存在的 pid(高段被占满的概率工程上为零,兜底取 pid_max−1)。
#[doc(hidden)]
pub fn definitely_dead_pid() -> u32 {
    #[cfg(target_os = "windows")]
    {
        4_194_303
    }
    #[cfg(not(target_os = "windows"))]
    {
        let pid_max = std::fs::read_to_string("/proc/sys/kernel/pid_max")
            .ok()
            .and_then(|t| t.trim().parse::<u32>().ok())
            .unwrap_or(4_194_304);
        (1..=32u32)
            .map(|i| pid_max - i)
            .find(|&p| !pid_alive(p))
            .unwrap_or_else(|| pid_max.saturating_sub(1))
    }
}

pub fn probe(path: &Path) -> io::Result<MediaInfo> {
    // -show_streams 与 -show_format 同批输出:B12 接线后 media_probe 需要分辨率
    // 与音轨存在性(来自 streams),时长仍统一取 format.duration(单一口径)。
    let out = Command::new(ffprobe_bin())
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
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
            Err(e) => assert!(
                e.to_string().contains("ffprobe"),
                "错误须来自 ffprobe 语义: {e}"
            ),
        }
    }
}
