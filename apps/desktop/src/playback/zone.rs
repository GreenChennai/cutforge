//! zone 预览缓存键与新鲜度判定(I1,工单 docs/tickets/I1-S1;M2 Kdenlive
//! Timeline Preview 模式的键面)。
//!
//! 纯函数纪律:除 fs 元数据检查外**无 I/O、无 RPC**——创建/删除缓存与 marker
//! 文件是渲染编排方(接线方)的职责,本模块只读元数据判新鲜。
//!
//! 缓存布局约定(与 docs/upstream/05 §I1 M2 一致):
//! - 缓存目录:`<root>/.cutforge/preview-cache/`;
//! - zone 文件:`<fingerprint>_<startMs>_<endMs>.mp4`(键即文件名词干);
//! - 指纹 marker:`<fingerprint>.fresh`(工程任何提交 → 指纹变化 → 旧 marker
//!   自然失配,等价于"脏标记",失效语义与 render 内容寻址缓存同源)。

use std::path::{Path, PathBuf};

/// zone 缓存键:指纹 + 区间毫秒,稳定纯拼接(同输入恒同输出,可作文件名词干)
pub fn zone_key(fingerprint: &str, start_ms: u64, end_ms: u64) -> String {
    format!("{fingerprint}_{start_ms}_{end_ms}")
}

/// 指纹 marker 路径(私有:布局约定集中一处)
fn fresh_marker(cache_dir: &Path, fingerprint: &str) -> PathBuf {
    cache_dir.join(format!("{fingerprint}.fresh"))
}

/// 指纹缓存是否新鲜:仅查 marker 文件元数据(存在即新鲜)。
/// 指纹不匹配(marker 是别的指纹/不存在)→ false,调用方应重渲。
pub fn is_fresh(cache_dir: &Path, fingerprint: &str) -> bool {
    fresh_marker(cache_dir, fingerprint).exists()
}

/// 预览缓存目录(root 为工程目录)
pub fn preview_cache_dir(root: &Path) -> PathBuf {
    root.join(".cutforge").join("preview-cache")
}

#[cfg(test)]
mod tests {
    use super::{is_fresh, preview_cache_dir, zone_key};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// D:\Temp 下唯一临时目录(纪律:临时文件一律落系统临时目录,不写 C 盘外路径)
    fn tmp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("cf-zone-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).expect("建临时目录失败");
        dir
    }

    #[test]
    fn 键稳定且输入敏感() {
        let a = zone_key("abc123", 1_000, 5_000);
        // 稳定:同输入恒同输出
        assert_eq!(a, zone_key("abc123", 1_000, 5_000));
        // 敏感:指纹/入点/出点任一不同 → 键不同
        assert_ne!(a, zone_key("abc124", 1_000, 5_000));
        assert_ne!(a, zone_key("abc123", 1_001, 5_000));
        assert_ne!(a, zone_key("abc123", 1_000, 5_001));
        // 边界值不产生歧义拼接
        assert_ne!(zone_key("f", 1, 23), zone_key("f", 12, 3));
    }

    #[test]
    fn 指纹不匹配判不新鲜() {
        let dir = tmp_dir("fresh");
        let cache = preview_cache_dir(&dir);
        assert!(!is_fresh(&cache, "fp1"), "无 marker 必不新鲜");
        fs::create_dir_all(&cache).unwrap();
        // 别的指纹 marker 不算命中
        fs::write(cache.join("fp0.fresh"), "").unwrap();
        assert!(!is_fresh(&cache, "fp1"), "指纹不匹配必不新鲜");
        // 本指纹 marker 命中(仅元数据检查)
        fs::write(cache.join("fp1.fresh"), "").unwrap();
        assert!(is_fresh(&cache, "fp1"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 缓存目录约定路径() {
        let dir = PathBuf::from("R");
        let want = PathBuf::from("R").join(".cutforge").join("preview-cache");
        assert_eq!(preview_cache_dir(&dir), want);
    }
}
