# V2-W1-KERNEL 内核数据正确性(审查报告 v2 §4)

范围:crates/cutforge-core/**(apply/model/undo/replay/merge/keyframes)+ schemas/project.schema.json(BUG-03 上界)。

条目:BUG-01/02/03/04/05/06/07/08/09(P0×3)+ R-12(若余力,bench 验收)。
纪律:先写报告列的 TC-* 用例确认红,再修(红-绿);旧 OpLog 必须兼容回放;
护城河 9 条(报告 §3)不得破坏;`source_read_ms` 收口为唯一换算真相源;
报告文件:C:\Users\Administrator\Desktop\CutForge-迭代审查报告-v2.md。

状态:进行中
