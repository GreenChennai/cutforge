// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! token 生成与 RT-1 会话变更摘要(.cutforge/session-summary.json;
//! T1.1 拆分自 lib.rs,纯移动)。

use cutforge_core::engine::{Answer, Query};
use cutforge_core::oplog::ActorKind;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 随机 token(pid+纳秒时钟 hash;无第三方依赖纪律)。E1-3:cli serve 复用。
pub fn new_token() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::process::id().hash(&mut h);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
    format!("{:016x}", h.finish())
}

// ---------------- RT-1:会话变更摘要(.cutforge/session-summary.json) ----------------
//
// 编辑器改完之后,Agent(CutFlow 侧)要能"读懂这次会话改了什么"。摘要记录:
// 服务启动 rev → 当前 rev 区间 + actor=human(user)的 Op 清单。落盘时机为
// **每次成功写之后增量写**——与"关闭服务时留一份"验收等价,且进程被 Ctrl+C/
// 崩溃杀死时不丢账(幂等:同一 rev 区间重复覆盖写,不追加)。

struct SessionJournal {
    started_at: String,
    rev_from: u64,
    rev_to: u64,
    ops: Vec<Value>,
}

fn sessions() -> &'static std::sync::Mutex<std::collections::BTreeMap<PathBuf, SessionJournal>> {
    static S: OnceLock<std::sync::Mutex<std::collections::BTreeMap<PathBuf, SessionJournal>>> =
        OnceLock::new();
    S.get_or_init(|| std::sync::Mutex::new(std::collections::BTreeMap::new()))
}

pub(crate) fn session_summary_path(root: &Path) -> PathBuf {
    root.join(".cutforge/session-summary.json")
}

fn disk_rev(root: &Path) -> u64 {
    std::fs::read_to_string(root.join(".cutforge/rev"))
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0)
}

pub(crate) fn session_journal_begin(root: &Path) {
    if let Ok(mut m) = sessions().lock() {
        let rev = disk_rev(root);
        m.insert(
            root.to_path_buf(),
            SessionJournal {
                started_at: cutforge_core::timeutil::now_rfc3339(),
                rev_from: rev,
                rev_to: rev,
                ops: Vec::new(),
            },
        );
    }
    let _ = write_session_summary(root);
}

/// 数据面上一次成功写之后调用:把 (cursor, rev_now] 区间内 actor=user 的 Op 追加进摘要。
pub(crate) fn session_journal_note(root: &Path, rev_now: u64) {
    let cursor = sessions()
        .lock()
        .ok()
        .and_then(|m| m.get(root).map(|j| j.rev_to));
    let Some(cursor) = cursor else { return };
    if rev_now <= cursor {
        return;
    }
    let new_ops: Vec<Value> = match cutforge_io::Workspace::open(root) {
        Ok(ws) => match ws.engine().query(Query::OpLogTail {
            since_rev: Some(cursor),
            actor_kind: Some(ActorKind::User),
        }) {
            Answer::Ops(ops) => ops
                .iter()
                .filter_map(|op| serde_json::to_value(op).ok())
                .collect(),
            _ => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    if let Ok(mut m) = sessions().lock()
        && let Some(j) = m.get_mut(root)
    {
        j.ops.extend(new_ops);
        j.rev_to = rev_now;
    }
    let _ = write_session_summary(root);
}

fn write_session_summary(root: &Path) -> std::io::Result<()> {
    let snapshot = sessions().lock().ok().and_then(|m| {
        m.get(root).map(|j| {
            json!({
                "kind": "cutforge-session-summary",
                "doc": "本次服务会话的变更摘要:actor=human(user)的 Op 清单 + rev 区间;供 CutFlow 侧变更识别(RT-1)",
                "startedAt": j.started_at,
                "updatedAt": cutforge_core::timeutil::now_rfc3339(),
                "revFrom": j.rev_from,
                "revTo": j.rev_to,
                "userOpCount": j.ops.len(),
                "ops": j.ops,
            })
        })
    });
    let Some(doc) = snapshot else { return Ok(()) };
    let mut buf = serde_json::to_vec_pretty(&doc)?;
    buf.push(b'\n');
    cutforge_io::atomic::atomic_write(&session_summary_path(root), &buf)
}
