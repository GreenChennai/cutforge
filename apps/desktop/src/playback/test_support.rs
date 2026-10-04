//! 测试专用工具定位与素材生成(仅 cfg(test) 编译)。
//!
//! 生产代码纪律:playback 模块**不读环境变量**;测试按工单纪律允许读
//! CUTFORGE_FFMPEG / CUTFORGE_FFPROBE(与接线方同款约定),缺省回落 PATH,
//! 不可用时优雅跳过并打印原因。临时文件一律落系统临时目录(TMP 指向 D:\Temp)。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// 定位工具:先环境变量(接线方同款约定)后 PATH;不可用返回 None
pub(crate) fn find_tool(env_key: &str, name: &str) -> Option<PathBuf> {
    if let Ok(p) = std::env::var(env_key) {
        let pb = PathBuf::from(p);
        if works(&pb) {
            return Some(pb);
        }
    }
    let pb = PathBuf::from(name);
    works(&pb).then_some(pb)
}

fn works(path: &Path) -> bool {
    Command::new(path)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// 系统临时目录下唯一工作目录(纪律:临时文件一律落 TMP,不写 C 盘)
pub(crate) fn tmp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir =
        std::env::temp_dir().join(format!("cf-playback-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建测试临时目录失败");
    dir
}

/// 生成 2s testsrc2 320x240 30fps 测试素材(mpeg4 编码,ffmpeg 原生自带)
pub(crate) fn gen_testsrc2(ffmpeg: &Path, dir: &Path) -> PathBuf {
    gen_testsrc2_sized(ffmpeg, dir, "320x240", "2")
}

/// 生成任意尺寸/时长 testsrc2 测试素材(mpeg4 编码;size 形如 "1920x1080")
pub(crate) fn gen_testsrc2_sized(ffmpeg: &Path, dir: &Path, size: &str, secs: &str) -> PathBuf {
    let dst = dir.join(format!("cf-testsrc2-{size}.mp4"));
    let out = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size={size}:rate=30"),
            "-t",
            secs,
            "-c:v",
            "mpeg4",
            "-y",
        ])
        .arg(&dst)
        .output()
        .expect("ffmpeg 生成测试素材失败(子进程无法运行)");
    assert!(
        out.status.success(),
        "生成测试素材失败:{}",
        String::from_utf8_lossy(&out.stderr)
    );
    dst
}

/// 清理测试临时目录(尽力而为)
pub(crate) fn cleanup(dir: &Path) {
    std::fs::remove_dir_all(dir).ok();
}
