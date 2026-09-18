# ADR-0001:文件级 Op 按 target.file 路由逆写(撤销语义为真)

日期:2026-09-19(M8)· 状态:已接受 · 关联:ITERATION-PLAN-v2.0 D1/P0-1

## 背景

V1 的 undo/redo 对一切 Op 都把 `before` 值按 `target.path` 回写到 **project** 文档。
对文件级 Op(notes.json / cutlist.json 的变更)这是静默伪造:notes 的 `/items` 指针
不存在于 Project(插入后被反序列化丢弃)、cutlist 的 `/` 是纯 no-op——undo 返回
Ok、rev 上涨、OpLog 记录了回退,**文件原封不动**(P0-1,已实机复现)。
同时,锚点重定位这类自动簿记 Op 混进撤销栈,撤销深度被系统性灌水。

## 决策

1. **按 `target.file` 路由逆写**,而非把文件级 Op 排除出撤销栈:
   - `project.json` → 沿用指针回写(既有机制);
   - 其余真相源 → Engine 新增 `file_states` 内存态,undo/redo 直接以 `before`/`after`
     覆写对应文件态,由 IO 层"先文件后记账"落盘。
   - 拒绝盲写:文件态当前值不等于该 Op 的 after(undo)/before(redo)时如实报错
     (LIFO 基准被外部扰动 → `GUARD_FAILED`),不做"就近猜测"。
2. **重定位类 Op 不入撤销栈**:Op 新增可选 `auto` 标记(oplog schema 同步收录),
   `ApplyOpts.non_undoable` 产生它;撤销深度 = 真实用户手势数。重开工程时
   `rebuild_undo_stack` 按 `auto` 跳过,语义与内存态一致。

## 否决的替代方案

- **文件级 Op 排除出撤销栈**:实现最简单,但"AI 加了标注又撤销"不可表达,
  违背审计完整性(计划书 4.4),且撤销深度语义变得更难解释。
- **文件字节快照栈(undo 恢复字节)**:能字节级还原,但跨重开即失效,
  且与 OpLog 审计链(语义域)重复一套真相。退而求其次:门禁按 canonical JSON
  语义等价断言(notes_add→undo 盘面还原),重序列化的缩进/键序不属语义域。

## 后果

- 撤销/重做语义为真,`file_level_undo`(内核 4 例)+ IO 侧 notes/cutlist 夹具锁定。
- Op 契约增加可选 `auto`(附加,向后兼容);oplog.schema.json 与生成式校验器同步。
- IO 层新增 reconcile(先文件后记账),顺带修复 P1-9(先记账后写文件的审计分叉)。
