// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! OpLog 压实规划(R-13②③,纯函数面;文件落盘在 cutforge-io)。
//!
//! # 两步原子压实协议(快照 → 截断,kill -9 任意点中断可自愈)
//!
//! 前置:最新快照存在且覆盖到 rev `S`(io 层 `.cutforge/snapshots/r<S>/`,
//! 内含 project.json + oplog 全量副本,见 io::snapshot)。当
//! `last_rev − S > 1000`([`COMPACT_REV_THRESHOLD`])时压实:
//!
//! 1. **步 1(快照)**:io 确保快照 r<S> 在盘(io::snapshot::write_snapshot,
//!    逐文件 atomic_write;同 rev 幂等);
//! 2. **步 2(截断)**:io 把 `.cutforge/oplog/*.jsonl` 截到
//!    [`retained_ops`](rev > S 的后缀),逐文件 atomic_write。
//!
//! 崩溃自愈论证(TC-IO-SNAP-002):任一时刻的死态只有三种——
//! - 步 1 前:全量日志在盘,普通 open 语义不变;
//! - 步 1 后步 2 中(部分截断到任意边界 j):盘面日志 = 任意前缀被删的形态,
//!   快照优先的 open 只回放 `rev > S` 的后缀([`retained_ops`] 对残留前缀
//!   幂等——前缀 op 即使还在盘上也会被跳过),结果与压实前 golden 逐字节相等;
//! - 步 2 后:终态,同上。
//!
//! 截断只删 `rev ≤ S` 的前缀:撤销/重做栈不受损(增量重建见
//! `engine::rebuild_stacks_incremental`——快照副本日志重建前缀栈,再折叠后缀)。
//!
//! 旧日志兼容:任一 Op 缺 rev 字段即放弃压实(返回 None)——无法证明快照
//! 覆盖面时宁可不压(护城河 6:OpLog 即历史,不可压坏)。

use crate::oplog::{Op, OpLog};

/// 压实阈值(R-13③):快照 rev 之外的 Op 超过该值才值得压实。
pub const COMPACT_REV_THRESHOLD: u64 = 1000;

/// 一次压实的执行计划(纯数据;io 层据此做「快照+截断」两步)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactPlan {
    /// 快照覆盖到的 rev(截断保留面 = rev > snapshot_rev)。
    pub snapshot_rev: u64,
    /// `ops()` 中首个 `rev > snapshot_rev` 的下标(截断保留起点)。
    pub retain_from: usize,
}

impl CompactPlan {
    /// 截断后保留的 Op 数。
    pub fn retained_len(&self, log: &OpLog) -> usize {
        log.len().saturating_sub(self.retain_from)
    }
}

/// 压实规划:快照存在、日志全带 rev、且快照外 Op 数超阈值时给出计划;
/// 其余情形(无快照/未超阈值/旧日志缺 rev/空日志)一律 None(不压实)。
pub fn plan(log: &OpLog, snapshot_rev: Option<u64>) -> Option<CompactPlan> {
    let snapshot_rev = snapshot_rev?;
    let ops = log.ops();
    if ops.is_empty() {
        return None;
    }
    // 旧日志缺 rev:无法证明「被截前缀 ⊆ 快照覆盖面」→ 放弃压实
    if ops.iter().any(|o| o.rev.is_none()) {
        return None;
    }
    let last = log.last_rev()?;
    if last.saturating_sub(snapshot_rev) <= COMPACT_REV_THRESHOLD {
        return None;
    }
    let retain_from = ops.partition_point(|o| o.rev.unwrap_or(0) <= snapshot_rev);
    Some(CompactPlan {
        snapshot_rev,
        retain_from,
    })
}

/// 压实后的保留面:rev > snapshot_rev 的后缀切片。
/// 对"步 2 中断"的残留前缀幂等:前缀 op(rev ≤ snapshot_rev)即使仍在盘上,
/// 本切片同样将其排除——快照优先的 open 语义因此与截断进度无关。
pub fn retained_ops(log: &OpLog, snapshot_rev: u64) -> &[Op] {
    let ops = log.ops();
    let from = ops.partition_point(|o| o.rev.unwrap_or(0) <= snapshot_rev);
    &ops[from..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oplog::{Actor, Op, OpKind, OpTarget};
    use serde_json::json;

    fn mk(rev: u64) -> Op {
        Op {
            op_id: format!("op-{rev}"),
            ts: String::new(),
            actor: Actor::agent("compact-t"),
            target: OpTarget {
                file: "project.json".into(),
                path: "/slug".into(),
            },
            op_kind: OpKind::Set,
            before: json!("旧"),
            after: json!("新"),
            base_rev: format!("rev-{}", rev.saturating_sub(1)),
            rev: Some(rev),
            target_id: None,
            caused_by: None,
            summary: "压实规划夹具".into(),
            request_id: None,
            auto: None,
        }
    }

    fn log_of(revs: impl IntoIterator<Item = u64>) -> OpLog {
        let mut log = OpLog::new();
        for r in revs {
            log.push_loaded(mk(r));
        }
        log
    }

    #[test]
    fn plan_gates_threshold_snapshot_and_legacy() {
        // 未超阈值 → 不压
        let log = log_of(1..=1000);
        assert_eq!(plan(&log, Some(0)), None, "rev 外恰 1000 条不压(须 >1000)");
        // 超阈值且有快照 → 计划,保留面从 rev > 快照起
        let log = log_of(1..=1002);
        let p = plan(&log, Some(1)).expect("超阈值必须给计划");
        assert_eq!(p.snapshot_rev, 1);
        assert_eq!(p.retain_from, 1, "rev=1 已被快照覆盖");
        assert_eq!(retained_ops(&log, 1).len(), 1001);
        // 无快照 → 不压
        assert_eq!(plan(&log, None), None);
        // 旧日志缺 rev → 不压(宁可不压不可压坏)
        let mut legacy = OpLog::new();
        legacy.push_loaded({
            let mut op = mk(1);
            op.rev = None;
            op
        });
        for r in 2..=1002 {
            legacy.push_loaded(mk(r));
        }
        assert_eq!(plan(&legacy, Some(1)), None, "缺 rev 的旧日志必须放弃压实");
    }
}
