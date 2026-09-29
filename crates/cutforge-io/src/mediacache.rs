// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 媒体派生物内容寻址缓存(册四 A4 T4.1/T4.8):缩略图 / 代理 / 波形 peaks
//! 三类缓存共用同一键派生与"改素材即 miss"口径(与 BE3a 目录注册表模式一致)。
//!
//! **缓存键设计**:key = hash(rel 路径 | mtime 秒 | size 字节 [, 语义维度])。
//! mtime+size 择义:文件内容变更几乎必然改变 mtime/size 之一(与 BE3a 的
//! 素材目录口径一致;hash 全内容对大素材过贵,mtime+size 是诚实折衷——
//! 同秒内同尺寸的内容替换不在防护面,报告如实声明)。
//! 语义维度(如缩略图 atMs/宽度、peaks 档位)直接拼进键串。
//!
//! 目录契约:`.cutforge/thumb-cache/` `.cutforge/proxy/` `.cutforge/peaks-cache/`
//! (与 `.cutforge/render-cache` 平级;整个 `.cutforge/` 可删可重建)。
//! 本模块只提供键派生与路径/落点解析,不执行 ffmpeg(MCP 层职责),
//! 代理查找给渲染端(plan::swap_to_proxies)复用同一实现。

use std::path::Path;

/// 缩略图缓存根(相对工程目录)。
pub const THUMB_DIR: &str = ".cutforge/thumb-cache";
/// 代理缓存根(相对工程目录)。
pub const PROXY_DIR: &str = ".cutforge/proxy";
/// 波形 peaks 缓存根(相对工程目录)。
pub const PEAKS_DIR: &str = ".cutforge/peaks-cache";

/// 统一媒体键:hash(rel|mtime|size|extra…);16 位十六进制(DefaultHasher 文本哈希,
/// 与 render-cache 的 key_hex 同族;键内拼入用途前缀防跨用途碰撞)。
pub fn media_key(use_tag: &str, rel: &str, mtime_secs: u64, size: u64, extra: &[&str]) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    format!("{use_tag}|{rel}|{mtime_secs}|{size}|{}", extra.join("|")).hash(&mut h);
    format!("{:016x}", h.finish())
}

/// 素材戳:(mtime 秒, size 字节);文件不可读 → None(调用方如实 miss/报错)。
pub fn source_stamp(path: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(path).ok()?;
    let mtime = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Some((mtime, m.len()))
}

/// 代理落点(相对工程目录):`.cutforge/proxy/<key>.mp4`(key 只含素材身份,
/// 不含分辨率——代理分辨率恒为源的一半,规格变化走渲染器版本换代)。
pub fn proxy_rel(rel: &str, mtime_secs: u64, size: u64) -> String {
    format!("{}/{}.mp4", PROXY_DIR, media_key("proxy", rel, mtime_secs, size, &[]))
}

/// 代理在位查找:返回代理相对路径(仅当文件真实存在;渲染端 use_proxy 消费)。
pub fn proxy_lookup(project_dir: &Path, src_rel: &str) -> Option<String> {
    let (mtime, size) = source_stamp(&project_dir.join(src_rel))?;
    let rel = proxy_rel(src_rel, mtime, size);
    project_dir.join(&rel).is_file().then_some(rel)
}

/// 缩略图落点(相对工程目录):`.cutforge/thumb-cache/<key>.png`
/// (语义维度:atMs + 宽度——不同抽帧点/宽度各自成键)。
pub fn thumb_rel(rel: &str, mtime_secs: u64, size: u64, at_ms: u64, width: u32) -> String {
    format!(
        "{}/{}.png",
        THUMB_DIR,
        media_key("thumb", rel, mtime_secs, size, &[&at_ms.to_string(), &width.to_string()])
    )
}

/// peaks 落点(相对工程目录):`.cutforge/peaks-cache/<key>.json`
/// (语义维度:桶数档位)。
pub fn peaks_rel(rel: &str, mtime_secs: u64, size: u64, buckets: u32) -> String {
    format!(
        "{}/{}.json",
        PEAKS_DIR,
        media_key("peaks", rel, mtime_secs, size, &[&buckets.to_string()])
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_key_is_deterministic_and_discriminating() {
        let a = media_key("thumb", "01_素材/a.mp4", 1000, 123, &["500", "320"]);
        let b = media_key("thumb", "01_素材/a.mp4", 1000, 123, &["500", "320"]);
        assert_eq!(a, b, "同输入同键(确定性)");
        assert_eq!(a.len(), 16);
        // 任一维度变化 → 键变("改素材即 miss"的机械保证)
        assert_ne!(a, media_key("thumb", "01_素材/a.mp4", 1001, 123, &["500", "320"]), "mtime 变");
        assert_ne!(a, media_key("thumb", "01_素材/a.mp4", 1000, 124, &["500", "320"]), "size 变");
        assert_ne!(a, media_key("thumb", "01_素材/b.mp4", 1000, 123, &["500", "320"]), "路径变");
        assert_ne!(a, media_key("thumb", "01_素材/a.mp4", 1000, 123, &["600", "320"]), "atMs 变");
        assert_ne!(a, media_key("proxy", "01_素材/a.mp4", 1000, 123, &["500", "320"]), "用途前缀隔离");
    }

    #[test]
    fn rel_paths_follow_directory_contract() {
        let p = proxy_rel("01_素材/a.mp4", 7, 9);
        assert!(p.starts_with(".cutforge/proxy/"), "{p}");
        assert!(p.ends_with(".mp4"));
        let t = thumb_rel("01_素材/a.mp4", 7, 9, 500, 320);
        assert!(t.starts_with(".cutforge/thumb-cache/") && t.ends_with(".png"), "{t}");
        let k = peaks_rel("01_素材/a.mp3", 7, 9, 2000);
        assert!(k.starts_with(".cutforge/peaks-cache/") && k.ends_with(".json"), "{k}");
    }

    #[test]
    fn source_stamp_missing_file_is_none() {
        assert!(source_stamp(Path::new("Z:/绝不存在/x.mp4")).is_none());
    }
}
