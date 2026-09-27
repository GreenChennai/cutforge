// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! OpLog 回放(计划书 4.4 可回放):从工程 + 完整 OpLog 重建出与逐步 apply
//! 语义一致的状态;并按日志语义确定性重建撤销/重做双栈(io 打开工程共用)。

use super::{apply_value_at, Engine, Reject};
use crate::model::Project;
use crate::oplog::{Op, OpKind};

impl Engine {
    /// 从工程 + 完整 OpLog 回放,得到与逐步 apply 语义一致的状态(计划书 4.4 可回放)。
    /// undo/redo Op 的 after 本身就是当时的状态转移,故全部照序应用;
    /// 文件级 Op 路由到 file_states(ADR-0001),不与工程文档混淆;
    /// 撤销栈按日志语义模拟重建(Undo 弹栈、Redo 压回,auto 类跳过)。
    pub fn replay(base: Project, ops: &[Op]) -> Result<Self, Reject> {
        let mut eng = Engine::new(base).map_err(Reject::SchemaInvalid)?;
        eng.rev = 0;
        for op in ops {
            if op.target.file == "project.json" {
                let mut project = eng.project.clone();
                apply_value_at(&mut project, &op.target.path, op.after.clone())
                    .map_err(Reject::InvariantViolation)?;
                if let Err(errs) = project.to_validated_value() {
                    return Err(Reject::SchemaInvalid(errs));
                }
                eng.project = project;
            } else {
                eng.file_states.insert(op.target.file.clone(), op.after.clone());
            }
            eng.rev = op.rev.unwrap_or(eng.rev + 1);
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
