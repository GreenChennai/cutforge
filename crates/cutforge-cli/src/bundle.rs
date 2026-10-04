// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! `doctor --bundle`(册六 T6.4):诊断包 = 单文件 zip(doctor.json + environment.txt
//! + 会话/摘要日志 + rev 快照),给「端口起不来/导出失败」一类问题的一键取证面。
//!
//! zip 读写原语为**零依赖手写 store 形**(method 0 不压缩,UTF-8 文件名,CRC-32
//! 校验),册七 T7.6 起提取为 `cutforge_io::zipstore` 单一实现(.cfpkg 共用)——
//! 新增 Cargo 依赖违反依赖纪律,而诊断包的体量(文本为主,KB 级)使压缩收益趋零。
//! 落盘走 `cutforge_io::atomic::atomic_write` 唯一落盘点(whole-bytes 一次成包)。

use std::fmt::Write as _;
use std::path::Path;

// zip 原语单一实现在 cutforge_io::zipstore(册七 T7.6 提取,.cfpkg 打包/解包共用);
// `Entry` 原样再导出——doctor.rs 沿用 `crate::bundle::Entry` 旧路径零改动。
pub use cutforge_io::zipstore::Entry;
use cutforge_io::zipstore::zip_store;

/// 打包并原子落盘(唯一落盘点纪律;返回字节数)。
pub fn write_bundle(out_path: &Path, entries: Vec<Entry>) -> Result<usize, String> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let zip = zip_store(&entries, secs);
    let n = zip.len();
    cutforge_io::atomic::atomic_write(out_path, &zip)
        .map_err(|e| format!("诊断包写入失败({}): {e}", out_path.display()))?;
    Ok(n)
}

/// 环境信息(人类可读;诊断包第 2 件):系统/架构/依赖版本/CUTFORGE_* env 面。
pub fn environment_text(root: &Path) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "CutForge 诊断包 · 环境信息");
    let _ = writeln!(s, "generatedAt: {}", cutforge_core::timeutil::now_rfc3339());
    let _ = writeln!(s, "root: {}", root.display());
    if let Ok(exe) = std::env::current_exe() {
        let _ = writeln!(s, "exe: {}", exe.display());
    }
    let _ = writeln!(s, "os: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    let _ = writeln!(s, "family: {}", std::env::consts::FAMILY);
    for (label, bin, key) in [
        ("ffmpeg", "ffmpeg", "CUTFORGE_FFMPEG"),
        ("ffprobe", "ffprobe", "CUTFORGE_FFPROBE"),
    ] {
        let via_env = std::env::var_os(key).is_some_and(|v| !v.is_empty());
        let version = std::process::Command::new(bin)
            .arg("-version")
            .output()
            .ok()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
            .unwrap_or_else(|| "不可用".into());
        let _ = writeln!(s, "{label}: {version}(env {key}: {via_env})");
    }
    let _ = writeln!(s, "── 相关环境变量(有则记值,无则记缺) ──");
    for key in [
        "CUTFORGE_FFMPEG",
        "CUTFORGE_FFPROBE",
        "CUTFORGE_WEB",
        "CUTFORGE_PROJECTS",
        "CUTFORGE_MEDIA",
        "CUTFORGE_RENDER",
        "CUTFORGE_SNAPSHOT_INTERVAL_MS",
    ] {
        match std::env::var_os(key) {
            Some(v) => {
                let _ = writeln!(s, "{key} = {}", v.to_string_lossy());
            }
            None => {
                let _ = writeln!(s, "{key} = (未设)");
            }
        }
    }
    // 外部管线变量只记**在位性**不记值(诊断包可能外发,别把机器路径带出去)
    for key in ["CUTFLOW_REPO", "CUTFLOW_CONFIG"] {
        let _ = writeln!(
            s,
            "{key} = {}",
            if std::env::var_os(key).is_some() {
                "(在位,值略)"
            } else {
                "(未设)"
            }
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// write_bundle:zip 落盘 + 结构可回读(原语级单测在 cutforge_io::zipstore,
    /// 此处锚定 cli 侧装配面)。
    #[test]
    fn write_bundle_roundtrip() {
        let dir = std::env::temp_dir().join(format!("cf-bundle-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let out = dir.join("doctor-bundle-test.zip");
        let n = write_bundle(
            &out,
            vec![
                Entry {
                    name: "doctor.json".into(),
                    data: b"{\"ok\":true}".to_vec(),
                },
                Entry {
                    name: "environment.txt".into(),
                    data: environment_text(&dir).into_bytes(),
                },
            ],
        )
        .unwrap();
        assert!(n > 0);
        let zip = std::fs::read(&out).unwrap();
        assert_eq!(&zip[0..4], &[0x50, 0x4B, 0x03, 0x04]);
        let back = cutforge_io::zipstore::zip_read_all(&zip).unwrap();
        assert_eq!(back[0].name, "doctor.json");
        assert_eq!(back[0].data, b"{\"ok\":true}".to_vec());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
