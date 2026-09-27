// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 撤销/重做(计划书 4.2/ADR-0001):按 target.file 路由逆写——
//! project.json 走指针回写,文件级真相源回滚/恢复其在 file_states 的内存态
//! (由 IO 层落盘),并各自产生 opKind=undo/redo 的新 Op。

use super::{apply_value_at, Engine, OpReceipt, Reject};
use crate::oplog::{Actor, Op, OpKind};

impl Engine {
    /// 撤销:按 target.file 路由逆写(ADR-0001)——project.json 走指针回写,
    /// 文件级真相源回滚其在 file_states 的内存态(由 IO 层落盘),产生 opKind=undo 的新 Op。
    pub fn undo(&mut self, actor: Actor) -> Result<OpReceipt, Reject> {
        let target_id = self.undo_stack.last().cloned().ok_or(Reject::NothingToUndo)?;
        let original = self.log.ops().iter().find(|o| o.op_id == target_id).cloned().ok_or(Reject::UnknownOp(target_id))?;
        self.rev += 1;
        let is_project = original.target.file == "project.json";
        if !is_project {
            // 文件级:仅当当前态恰为该 Op 的 after 才允许逆写(LIFO 语义被外部扰动时如实拒绝)
            match self.file_states.get(&original.target.file) {
                Some(cur) if *cur == original.after => {}
                _ => {
                    self.rev -= 1;
                    return Err(Reject::InvariantViolation(format!(
                        "撤销基准不一致:{} 的当前态已偏离待撤销 Op 的 after(外部改动或重放),拒绝盲写",
                        original.target.file)));
                }
            }
        }
        let undo_op = Op {
            op_id: self.log.next_op_id(),
            ts: crate::timeutil::now_rfc3339(),
            actor,
            target: original.target.clone(),
            op_kind: OpKind::Undo,
            before: original.after.clone(),
            after: original.before.clone(),
            base_rev: crate::format_rev(self.rev - 1),
            rev: Some(self.rev),
            caused_by: None,
            summary: format!("撤销 {}", original.summary),
            request_id: None,
            auto: None,
        };
        if is_project {
            let mut project = self.project.clone();
            apply_value_at(&mut project, &original.target.path, original.before.clone())
                .map_err(|e| { self.rev -= 1; Reject::InvariantViolation(e) })?;
            if let Err(errs) = project.to_validated_value() {
                self.rev -= 1;
                return Err(Reject::SchemaInvalid(errs));
            }
            self.project = project;
        } else {
            self.file_states.insert(original.target.file.clone(), original.before.clone());
            self.dirty_files.insert(original.target.file.clone());
        }
        self.log.push(undo_op);
        self.undo_stack.pop();
        self.redo_stack.push(original.op_id.clone());
        Ok(OpReceipt { op_ids: self.log.ops().last().map(|o| vec![o.op_id.clone()]).unwrap_or_default(), rev: self.rev, idempotent: false })
    }

    /// 重做:按 target.file 路由恢复被撤销 Op 的 after,产生 opKind=redo 的新 Op。
    pub fn redo(&mut self, actor: Actor) -> Result<OpReceipt, Reject> {
        let target_id = self.redo_stack.last().cloned().ok_or(Reject::NothingToRedo)?;
        let original = self.log.ops().iter().find(|o| o.op_id == target_id).cloned().ok_or(Reject::UnknownOp(target_id))?;
        self.rev += 1;
        let is_project = original.target.file == "project.json";
        if !is_project {
            match self.file_states.get(&original.target.file) {
                Some(cur) if *cur == original.before => {}
                _ => {
                    self.rev -= 1;
                    return Err(Reject::InvariantViolation(format!(
                        "重做基准不一致:{} 的当前态已偏离待重做 Op 的 before,拒绝盲写",
                        original.target.file)));
                }
            }
        }
        let redo_op = Op {
            op_id: self.log.next_op_id(),
            ts: crate::timeutil::now_rfc3339(),
            actor,
            target: original.target.clone(),
            op_kind: OpKind::Redo,
            before: original.before.clone(),
            after: original.after.clone(),
            base_rev: crate::format_rev(self.rev - 1),
            rev: Some(self.rev),
            caused_by: None,
            summary: format!("重做 {}", original.summary),
            request_id: None,
            auto: None,
        };
        if is_project {
            let mut project = self.project.clone();
            apply_value_at(&mut project, &original.target.path, original.after.clone())
                .map_err(|e| { self.rev -= 1; Reject::InvariantViolation(e) })?;
            if let Err(errs) = project.to_validated_value() {
                self.rev -= 1;
                return Err(Reject::SchemaInvalid(errs));
            }
            self.project = project;
        } else {
            self.file_states.insert(original.target.file.clone(), original.after.clone());
            self.dirty_files.insert(original.target.file.clone());
        }
        self.log.push(redo_op);
        self.redo_stack.pop();
        self.undo_stack.push(original.op_id.clone());
        Ok(OpReceipt { op_ids: self.log.ops().last().map(|o| vec![o.op_id.clone()]).unwrap_or_default(), rev: self.rev, idempotent: false })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::engine::{Answer, ApplyOpts, Query, sample_project};
    use crate::oplog::Actor;
    use serde_json::json;

    fn agent() -> Actor {
        Actor::agent("test")
    }

    /// bgm_set/bgm_clear:创建带 schema 默认、合并、无 src 拒绝、撤销/重做回环。
    #[test]
    fn bgm_set_clear_undo_redo_roundtrip() {
        let mut eng = Engine::new(sample_project()).unwrap();
        // 无 bgm 且不带 src → MissingBgm
        let r = eng.apply(Command::BgmSet { patch: crate::command::BgmPatch { gain_db: Some(-12.0), ..Default::default() } }, agent(), ApplyOpts::default());
        assert!(matches!(r, Err(Reject::MissingBgm)), "{r:?}");
        // 创建:未给出的字段落 schema 默认(gainDb=-18/ducking=true/loop=true)
        let r = eng.apply(Command::BgmSet {
            patch: crate::command::BgmPatch { src: Some("02_音乐/bgm.mp3".into()), ..Default::default() },
        }, agent(), ApplyOpts::default()).unwrap();
        assert_eq!(r.rev, 1);
        let op = &eng.oplog().ops()[0];
        assert_eq!(op.target.path, "/bgm");
        assert_eq!(op.target.file, "project.json");
        match eng.query(Query::ProjectView) {
            Answer::Project(v) => {
                assert_eq!(v["bgm"]["src"], json!("02_音乐/bgm.mp3"));
                assert_eq!(v["bgm"]["gainDb"], json!(-18.0));
                assert_eq!(v["bgm"]["ducking"], json!(true));
                assert_eq!(v["bgm"]["loop"], json!(true));
            }
            other => panic!("意外: {other:?}"),
        }
        // 合并:只改 gainDb/loop,src/ducking 保持
        eng.apply(Command::BgmSet {
            patch: crate::command::BgmPatch { gain_db: Some(-9.0), loop_: Some(false), ..Default::default() },
        }, agent(), ApplyOpts::default()).unwrap();
        match eng.query(Query::ProjectView) {
            Answer::Project(v) => {
                assert_eq!(v["bgm"]["src"], json!("02_音乐/bgm.mp3"), "src 不得被清掉");
                assert_eq!(v["bgm"]["gainDb"], json!(-9.0));
                assert_eq!(v["bgm"]["loop"], json!(false));
            }
            other => panic!("意外: {other:?}"),
        }
        // 同值 → 幂等回执,rev 不动
        let r = eng.apply(Command::BgmSet {
            patch: crate::command::BgmPatch { gain_db: Some(-9.0), ..Default::default() },
        }, agent(), ApplyOpts::default()).unwrap();
        assert!(r.idempotent);
        assert_eq!(eng.rev(), 2);
        // 撤销合并 → gainDb 回 -18;再撤销创建 → bgm 消失;重做恢复
        eng.undo(agent()).unwrap();
        match eng.query(Query::ProjectView) {
            Answer::Project(v) => assert_eq!(v["bgm"]["gainDb"], json!(-18.0)),
            other => panic!("意外: {other:?}"),
        }
        eng.undo(agent()).unwrap();
        match eng.query(Query::ProjectView) {
            Answer::Project(v) => assert!(v.get("bgm").is_none(), "撤销创建后 bgm 必须消失: {v}"),
            other => panic!("意外: {other:?}"),
        }
        eng.redo(agent()).unwrap();
        match eng.query(Query::ProjectView) {
            Answer::Project(v) => assert_eq!(v["bgm"]["gainDb"], json!(-18.0), "重做恢复创建态"),
            other => panic!("意外: {other:?}"),
        }
        // 清除:rev 上涨;再清除幂等;撤销清除恢复
        let r = eng.apply(Command::BgmClear, agent(), ApplyOpts::default()).unwrap();
        assert!(!r.idempotent);
        match eng.query(Query::ProjectView) {
            Answer::Project(v) => assert!(v.get("bgm").is_none()),
            other => panic!("意外: {other:?}"),
        }
        let r = eng.apply(Command::BgmClear, agent(), ApplyOpts::default()).unwrap();
        assert!(r.idempotent, "已无 bgm 时清除必须幂等");
        eng.undo(agent()).unwrap();
        match eng.query(Query::ProjectView) {
            Answer::Project(v) => assert_eq!(v["bgm"]["src"], json!("02_音乐/bgm.mp3"), "撤销清除必须恢复 bgm"),
            other => panic!("意外: {other:?}"),
        }
    }
}
