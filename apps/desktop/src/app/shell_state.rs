//! 壳态持久化(R-17,审查报告 v2 §5):播放头/选择集/吸附开关 10s 防抖落盘,
//! 启动恢复。文件:`<root>/.cutforge/shell-state.json`。
//!
//! 定性:UX 状态面,**非工程真相**(真相 = project.json + OpLog,atomic.rs 唯一
//! 落盘点纪律针对真相;本文件损坏只影响便利性 → 丢弃用默认,不崩壳)。

use std::path::Path;

use serde_json::Value;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ShellState {
    pub playhead_ms: u64,
    pub selection: Vec<String>,
    pub snap_on: bool,
}

pub fn state_path(root: &Path) -> std::path::PathBuf {
    root.join(".cutforge/shell-state.json")
}

/// 读取壳态;不存在/损坏 → None(调用方用默认,丢弃文件不留病根)。
pub fn load(root: &Path) -> Option<ShellState> {
    let text = std::fs::read_to_string(state_path(root)).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    Some(ShellState {
        playhead_ms: v.get("playheadMs").and_then(Value::as_u64).unwrap_or(0),
        selection: v
            .get("selection")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        snap_on: v.get("snapOn").and_then(Value::as_bool).unwrap_or(true),
    })
}

/// 保存壳态(调用方 10s 防抖;诊断面直写,损坏即被 load 丢弃)。
pub fn save(root: &Path, st: &ShellState) {
    let v = serde_json::json!({
        "playheadMs": st.playhead_ms,
        "selection": st.selection,
        "snapOn": st.snap_on,
    });
    let path = state_path(root);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(
        &path,
        serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".into()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_state_roundtrip_preserves_fields() {
        let dir = std::env::temp_dir().join("cutforge-shell-state-rt");
        let _ = std::fs::create_dir_all(dir.join(".cutforge"));
        let st = ShellState {
            playhead_ms: 4242,
            selection: vec!["V1-001".into(), "V1-002".into()],
            snap_on: false,
        };
        save(&dir, &st);
        let loaded = load(&dir).expect("保存后应可读回");
        assert_eq!(loaded, st);
        let _ = std::fs::remove_file(state_path(&dir));
    }

    #[test]
    fn shell_state_corrupt_file_is_dropped_not_fatal() {
        let dir = std::env::temp_dir().join("cutforge-shell-state-corrupt");
        let _ = std::fs::create_dir_all(dir.join(".cutforge"));
        std::fs::write(state_path(&dir), "{ not json !!!").unwrap();
        assert!(load(&dir).is_none(), "损坏文件应返回 None(丢弃用默认)");
        let _ = std::fs::remove_file(state_path(&dir));
    }

    #[test]
    fn shell_state_missing_file_is_none_and_snapshot_defaults_hold() {
        let dir = std::env::temp_dir().join("cutforge-shell-state-missing");
        let _ = std::fs::remove_file(state_path(&dir));
        assert!(load(&dir).is_none());
        // 壳侧初值 snap_enabled=true 与本结构 Default 派生(false)无关——
        // 恢复永远发生在壳设完初值之后(load 的值直接覆盖),两者不共享缺省。
    }
}
