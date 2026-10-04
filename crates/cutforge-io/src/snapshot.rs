// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 自动快照(册六 T6.1):按可配置间隔把 project.json + OpLog 打包到
//! `.cutforge/snapshots/r<rev>/`,LRU 上限淘汰,**缺省开**(R-13①:快照机制
//! 缺省关 = 不存在;现缺省 5 分钟间隔,env 显式置 0 才关)。
//!
//! - 配置:env `CUTFORGE_SNAPSHOT_INTERVAL_MS`(显式 0 = 关;未设 = 缺省 5min)
//!   与 `CUTFORGE_SNAPSHOT_KEEP`(保留份数,缺省 10)。解析唯一定义点
//!   [`config_from`](纯函数,测试无 env 竞态)。
//! - 快照 = 目录 `r<rev>/` 内 `project.json`(字节原样)+ `oplog/`(全量 jsonl);
//!   rev 单调,同名目录已存在即跳过(幂等)。
//! - 落盘点收敛 `atomic::atomic_write`(复制字节,不经 rename 跨语义);
//!   快照是派生物:丢失可从 oplog/rev 重建,不入真相源(ADR-0013 同口径)。

use crate::atomic;
use crate::paths;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 快照根(相对工程根)。
pub const SNAPSHOTS_DIR: &str = ".cutforge/snapshots";
/// LRU 保留份数缺省值。
pub const DEFAULT_KEEP: usize = 10;
/// 缺省快照间隔(R-13①:快照**缺省开**,5 分钟;env 显式 0 才关)。
pub const DEFAULT_INTERVAL_MS: u64 = 300_000;

/// 纯函数配置面(env 解析的唯一定义点,测试无 env 竞态):
/// `interval_raw` = env `CUTFORGE_SNAPSHOT_INTERVAL_MS` 的取值(None = 未设);
/// `keep_raw` = env `CUTFORGE_SNAPSHOT_KEEP` 的取值。
/// 未设 → 缺省开(5min);显式 0/非法 → 关(escape hatch 保留)。
pub fn config_from(
    interval_raw: Option<&str>,
    keep_raw: Option<&str>,
) -> (Option<Duration>, usize) {
    let interval = match interval_raw {
        None => Some(Duration::from_millis(DEFAULT_INTERVAL_MS)),
        Some(v) => v
            .parse::<u64>()
            .ok()
            .filter(|ms| *ms > 0)
            .map(Duration::from_millis),
    };
    let keep = keep_raw
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|k| *k > 0)
        .unwrap_or(DEFAULT_KEEP);
    (interval, keep)
}

/// env 配置面:`(interval, keep)`;env 显式设 0 → 关,未设 → 缺省间隔(开)。
pub fn config_from_env() -> (Option<Duration>, usize) {
    let interval = std::env::var("CUTFORGE_SNAPSHOT_INTERVAL_MS").ok();
    let keep = std::env::var("CUTFORGE_SNAPSHOT_KEEP").ok();
    config_from(interval.as_deref(), keep.as_deref())
}

/// 距上次快照是否到期(以最新快照目录的 mtime 为时钟;无快照 = 到期)。
fn due(snapshots_dir: &Path, interval: Duration) -> bool {
    match latest_snapshot(snapshots_dir) {
        None => true,
        Some((_, meta)) => meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|age_at| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();
                now.saturating_sub(age_at) >= interval
            })
            .unwrap_or(true),
    }
}

/// 最新快照目录(按 rev 数值序)。
fn latest_snapshot(snapshots_dir: &Path) -> Option<(u64, std::fs::Metadata)> {
    list_snapshots(snapshots_dir).into_iter().next_back()
}

/// 全部快照目录(按 rev 数值升序;名字 `r<rev>` 解析失败忽略)。
fn list_snapshots(snapshots_dir: &Path) -> Vec<(u64, std::fs::Metadata)> {
    let mut out: Vec<(u64, std::fs::Metadata)> = std::fs::read_dir(snapshots_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let rev = name.strip_prefix('r')?.parse::<u64>().ok()?;
            Some((rev, e.metadata().ok()?))
        })
        .collect();
    out.sort_by_key(|(rev, _)| *rev);
    out
}

/// 快照一次(条件已满足时);返回快照目录或 None(未到期/已存在/rev 读不到)。
pub fn snapshot_if_due(
    root: &Path,
    interval: Option<Duration>,
    keep: usize,
) -> io::Result<Option<PathBuf>> {
    let Some(interval) = interval else {
        return Ok(None);
    };
    let snapshots_dir = root.join(SNAPSHOTS_DIR);
    if !due(&snapshots_dir, interval) {
        return Ok(None);
    }
    write_snapshot(root, &snapshots_dir, keep).map(Some)
}

/// 无条件快照当前 rev(到期判定外的直写口;persist 管线不直呼,测试与手动面用)。
pub fn write_snapshot(root: &Path, snapshots_dir: &Path, keep: usize) -> io::Result<PathBuf> {
    let rev_text = std::fs::read_to_string(root.join(".cutforge/rev")).unwrap_or_default();
    let rev = rev_text.trim().parse::<u64>().unwrap_or(0);
    let dest = snapshots_dir.join(format!("r{rev}"));
    if dest.is_dir() {
        return Ok(dest); // rev 单调:同 rev 快照幂等
    }
    // project.json 字节原样(按盘面布局定位,三态皆认)
    let project = paths::project_path(root);
    let bytes = std::fs::read(&project)
        .map_err(|e| io::Error::new(e.kind(), format!("快照失败(project.json 不可读): {e}")))?;
    atomic::atomic_write(&dest.join("project.json"), &bytes)?;
    // oplog 全量复制
    let oplog_dir = root.join(".cutforge/oplog");
    if let Ok(entries) = std::fs::read_dir(&oplog_dir) {
        for e in entries.flatten() {
            if e.path().extension().is_some_and(|x| x == "jsonl")
                && let Ok(data) = std::fs::read(e.path())
            {
                atomic::atomic_write(&dest.join("oplog").join(e.file_name()), &data)?;
            }
        }
    }
    // LRU:超出 keep 淘汰最旧(rev 最小)
    let snaps = list_snapshots(snapshots_dir);
    if snaps.len() > keep {
        for (rev, _) in snaps.iter().take(snaps.len() - keep) {
            let _ = std::fs::remove_dir_all(snapshots_dir.join(format!("r{rev}")));
        }
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LayoutKind;
    use crate::{Workspace, fsutil};

    fn make_project_with_ops(dir: &Path, ops: usize) {
        crate::scaffold::scaffold_project_layout(
            dir,
            "快照体",
            30,
            1080,
            1920,
            &[cutforge_core::model::TrackKind::Video],
            LayoutKind::V2,
        )
        .unwrap();
        let mut ws = Workspace::open_exclusive(dir).unwrap();
        for i in 0..ops {
            ws.apply(
                cutforge_core::command::Command::TrackAdd {
                    kind: cutforge_core::model::TrackKind::Audio,
                    request_id: Some(format!("snap-{i}")),
                },
                cutforge_core::oplog::Actor::agent("snap"),
                Default::default(),
            )
            .unwrap();
        }
        drop(ws);
    }

    #[test]
    fn snapshot_write_lru_and_explicit_off() {
        let root = fsutil::temp_dir("snapshot-lru");
        make_project_with_ops(&root, 3);
        let snaps = root.join(SNAPSHOTS_DIR);
        // 显式连打 15 份(rev 单调?rev 只有 3——手工多份需改 rev 文件模拟多次会话)
        for rev in 1..=15u64 {
            atomic::atomic_write(&root.join(".cutforge/rev"), format!("{rev}\n").as_bytes())
                .unwrap();
            let d = write_snapshot(&root, &snaps, 10).unwrap();
            assert!(d.join("project.json").is_file());
        }
        let left = list_snapshots(&snaps);
        assert_eq!(left.len(), 10, "LRU 上限 10");
        assert_eq!(left.first().unwrap().0, 6, "最旧 5 份被淘汰");
        assert_eq!(left.last().unwrap().0, 15);
        // 快照内容 = project.json + oplog 全量
        let last = snaps.join("r15");
        assert!(
            last.join("oplog").read_dir().unwrap().count() >= 1,
            "oplog 随包"
        );
        // 同 rev 重复快照幂等
        assert_eq!(write_snapshot(&root, &snaps, 10).unwrap(), last);
        // 缺省关:interval None → 永不写
        assert_eq!(snapshot_if_due(&root, None, 10).unwrap(), None);
        fsutil::cleanup(&root);
    }

    #[test]
    fn snapshot_due_gate_by_interval() {
        let root = fsutil::temp_dir("snapshot-due");
        make_project_with_ops(&root, 1);
        // R-13①:快照缺省开后,persist 期间可能已自动落过首份快照;
        // 本用例验证的是"到期门",清掉派生物从干净态起测。
        let _ = std::fs::remove_dir_all(root.join(SNAPSHOTS_DIR));
        let _snaps = root.join(SNAPSHOTS_DIR);
        // 长间隔:刚写过(首份即落),未到期不再写
        assert!(
            snapshot_if_due(&root, Some(Duration::from_secs(3600)), 10)
                .unwrap()
                .is_some(),
            "无快照时首打必落"
        );
        assert_eq!(
            snapshot_if_due(&root, Some(Duration::from_secs(3600)), 10).unwrap(),
            None,
            "未到期不重复"
        );
        // 零间隔:到期,但同 rev 幂等
        let d = snapshot_if_due(&root, Some(Duration::ZERO), 10).unwrap();
        assert!(d.is_some());
        fsutil::cleanup(&root);
    }
}
