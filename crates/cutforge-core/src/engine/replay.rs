// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! OpLog 回放(计划书 4.4 可回放):从工程 + 完整 OpLog 重建出与逐步 apply
//! 语义一致的状态;并按日志语义确定性重建撤销/重做双栈(io 打开工程共用)。

use super::{Engine, Reject, write_project_op};
use crate::model::Project;
use crate::oplog::{Op, OpKind};

impl Engine {
    /// 从工程 + 完整 OpLog 回放,得到与逐步 apply 语义一致的状态(计划书 4.4 可回放)。
    /// undo/redo Op 的 after 本身就是当时的状态转移,故全部照序应用;
    /// 文件级 Op 路由到 file_states(ADR-0001),不与工程文档混淆;
    /// 撤销栈按日志语义模拟重建(Undo 弹栈、Redo 压回,auto 类跳过)。
    ///
    /// rev 链校验(BUG-07):rev 必须严格 +1 递增,断链/乱序/重号返回
    /// [`Reject::RevGap`]**(open 层应将其冒泡为「工程需修复」的用户可见状态,
    /// 而非 open 失败**;与 R-03 半行截断同入 RepairReport 模式);旧日志缺 rev
    /// 字段(serde default None)按顺推兼容,零迁移。
    pub fn replay(base: Project, ops: &[Op]) -> Result<Self, Reject> {
        let mut eng = Engine::new(base).map_err(Reject::SchemaInvalid)?;
        eng.rev = 0;
        for op in ops {
            // rev 链校验先行:断链不得半应用(状态保持可重放从干净基线起步)
            let expected = eng.rev + 1;
            eng.rev = match op.rev {
                Some(r) if r == expected => r,
                Some(r) => {
                    return Err(Reject::RevGap {
                        expected,
                        got: r,
                        op_id: op.op_id.clone(),
                    });
                }
                // 旧日志缺 rev:顺推兼容(零迁移)
                None => expected,
            };
            if op.target.file == "project.json" {
                let mut project = eng.project.clone();
                // 稳定 id 寻址(BUG-06):新 Op 按 target_id 定位,旧 Op 走指针
                write_project_op(&mut project, op, op.after.clone(), None)
                    .map_err(Reject::InvariantViolation)?;
                if let Err(errs) = project.to_validated_value() {
                    return Err(Reject::SchemaInvalid(errs));
                }
                eng.project = project;
            } else {
                eng.file_states
                    .insert(op.target.file.clone(), op.after.clone());
            }
            eng.log.push_loaded(op.clone());
        }
        let (undo_stack, redo_stack) = rebuild_stacks(ops);
        eng.undo_stack = undo_stack;
        eng.redo_stack = redo_stack;
        Ok(eng)
    }
}

/// 按日志语义重建撤销/重做双栈(Undo 弹栈、Redo 压回;io 打开工程与 replay 共用)。
/// auto 类 Op(锚点重定位等自动簿记)不入栈——撤销深度 = 真实用户手势数(ADR-0001)。
/// 重做栈同样可从日志确定性重建:M9 修复前 restore 把 redo 置空,而 MCP 每次
/// dispatch 都重开工程 → redo 跨 dispatch 永远失效(NOTHING_TO_REDO)。
pub fn rebuild_stacks(ops: &[Op]) -> (Vec<String>, Vec<String>) {
    let mut undo_stack: Vec<String> = Vec::new();
    let mut redo_stack: Vec<String> = Vec::new();
    for op in ops {
        match op.op_kind {
            OpKind::Undo => {
                if let Some(x) = undo_stack.pop() {
                    redo_stack.push(x);
                }
            }
            OpKind::Redo => {
                if let Some(x) = redo_stack.pop() {
                    undo_stack.push(x);
                }
            }
            _ if op.auto == Some(true) => {}
            _ => undo_stack.push(op.op_id.clone()),
        }
    }
    (undo_stack, redo_stack)
}

/// 兼容入口:仅撤销栈。
pub fn rebuild_undo_stack(ops: &[Op]) -> Vec<String> {
    rebuild_stacks(ops).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{ClipPatch, Command};
    use crate::engine::{ApplyOpts, sample_project};
    use crate::oplog::Actor;

    fn agent() -> Actor {
        Actor::agent("replay-tc")
    }

    fn eng_with_three_ops() -> (Engine, Vec<Op>) {
        let base = sample_project();
        let mut eng = Engine::new(base.clone()).unwrap();
        eng.apply(
            Command::ClipUpdate {
                clip_id: "V1-001".into(),
                patch: ClipPatch {
                    volume: Some(0.5),
                    ..Default::default()
                },
            },
            agent(),
            ApplyOpts::default(),
        )
        .unwrap();
        eng.apply(
            Command::ClipSplit {
                clip_id: "V1-001".into(),
                t_ms: 4000,
            },
            agent(),
            ApplyOpts::default(),
        )
        .unwrap();
        eng.apply(
            Command::ClipUpdate {
                clip_id: "V1-002".into(),
                patch: ClipPatch {
                    volume: Some(0.9),
                    ..Default::default()
                },
            },
            agent(),
            ApplyOpts::default(),
        )
        .unwrap();
        let ops = eng.oplog().ops().to_vec();
        (eng, ops)
    }

    /// TC-CORE-REPLAY-001(BUG-07):OpLog 缺一行(rev 链断链)→ 回放必须报
    /// RevGap{expected, got, op_id} 指明缺失位置,而非静默接受。
    #[test]
    fn tc_core_replay_001_missing_op_reports_rev_gap() {
        let (_, mut ops) = eng_with_three_ops();
        let full = ops.clone();
        ops.remove(1); // 抽走 rev=2 的 Op(断链)
        let r = Engine::replay(sample_project(), &ops);
        match r {
            Err(Reject::RevGap {
                expected,
                got,
                op_id,
            }) => {
                assert_eq!((expected, got), (2, 3), "缺口 = 期望 2、实际 3");
                assert_eq!(op_id, ops[1].op_id, "op_id 必须指到断链后的第一条 Op");
            }
            Ok(_) => panic!("断链被静默接受(BUG-07 现状): 必须报 RevGap"),
            Err(other) => panic!("意外错误: {other:?}"),
        }
        // 对照:完整日志回放成功(断言夹具本身无假阳性)
        assert!(
            Engine::replay(sample_project(), &full).is_ok(),
            "完整日志必须可回放"
        );
    }

    /// TC-CORE-REPLAY-002(BUG-07):rev 乱序/重号拒绝;缺 rev(旧日志)顺推兼容。
    #[test]
    fn tc_core_replay_002_duplicate_rev_rejected_missing_rev_tolerated() {
        let (_, mut ops) = eng_with_three_ops();
        let last_id = ops[2].op_id.clone();
        ops[2].rev = Some(2); // 重号:rev=2 出现两次
        let r = Engine::replay(sample_project(), &ops);
        match r {
            Err(Reject::RevGap {
                expected,
                got,
                op_id,
            }) => {
                assert_eq!((expected, got), (3, 2));
                assert_eq!(op_id, last_id);
            }
            Ok(_) => panic!("重号 rev 被静默接受(BUG-07 现状)"),
            Err(other) => panic!("意外错误: {other:?}"),
        }
        // 兼容:旧日志缺 rev 字段 → 顺推,回放成功且 rev = Op 条数
        let (_, mut legacy) = eng_with_three_ops();
        for op in &mut legacy {
            op.rev = None;
        }
        let replayed = Engine::replay(sample_project(), &legacy).unwrap();
        assert_eq!(replayed.rev(), 3, "缺 rev 顺推:rev = 条数");
    }
}
