# V2-W2-KERNEL2 性能底盘与测试布局

范围:crates/cutforge-core/**(apply/undo/replay/mod)+ tools/gates/a1.py 仅一行豁免授权(与主会话确认后)。

条目(报告 v2):
- R-12(P2):apply 去全量 clone(Op 逆向回滚)+ schema 增量校验(受影响子树+全局不变量),
  全量校验降级 debug/doctor;**bench 验收:1k 片段×500 命令耗时降 ≥80%**,回滚正确性
  与快照方案等价(TC-CORE-APPLY-099 逐字节相等)。前置已备:target_id 稳定寻址+write_project_op。
- R-13②③(P1):OpLog 压实(rev>1000 且快照存在,快照+截断原子两步,kill -9 中断可自愈);
  rebuild_stacks 从最近快照增量重建;TC-PERF-OPEN-001(10 万 Op open <2s)、TC-IO-SNAP-001/002。
  与 io 层 repair.rs 已落地接口协同(oplog 压实后 repair 指纹口径不变)。
- A-05(P2):apply.rs:794 include! 清理——测试按标准布局迁 engine/tests/ 或 cfg(test) 模块;
  行数门禁对 #[cfg(test)] 模块与 tests/ 目录豁免的 gate.py 改动**由第 4 波 GATES 收口**,本工单
  只做移动+在报告注明需要豁免的目录清单。
前置:BUG-01~09 已落(clip_ops.rs/target_id/RevGap);护城河 9 条不得破坏;红-绿循环。
状态:待第 1 波集成后开工
