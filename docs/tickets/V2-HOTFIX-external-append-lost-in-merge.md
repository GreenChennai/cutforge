# V2-HOTFIX:外部直写 project.json 的追加被 sync_with_disk 三路合并静默丢弃(阻塞 web-e2e A2 第 13 步)

归属:crates/cutforge-core/src/merge.rs(本轮禁区,由内核轮修复;本工单由 MCP 轮排查移交)。

## 现象
- `tools/e2e_ui_smoke.py` 第 13 步(SSE 毒化 → 长轮询 → 外部插入)确定性红:
  `external_append_clip(V1-901, 60000, 10000)` 后壳重投影永远拿 2 clips(标尺 1420 不动)。
- 本地 3/3 复现 = CI run 37243668955 同签名。

## 最小复现(与本仓 serve,python 探针已实证)
1. `cutforge-cli new proj`(V2 布局)+ `clip_add` ×2(rev=2);
2. **外部直写** `05_时间线工程/project.json`:V1.clips 追加 `{"id":"V1-901",...}`;
3. 等 watcher 守护(≤4s 退避)触发 `open_exclusive + merge_from_disk`;
4. 实测:rpc `project_get` clips 仍 = `[V1-001, V1-002]`,**磁盘被 persist 覆写回 2 clips**,
   rev 不变(=2),外部追加永久丢失。四轮前(e4605ec 之前)该场景工作(ui_smoke 全过)。

## 定位(证据)
- 守护线程诊断(临时 eprintln,已还原)实证:`project_touched=true → own_write=false → bumped`,
  即 `sync_with_disk` 被调;随后磁盘被覆写回 local 态 → **`three_way_merge(base, disk, local)`
  输出 == local(外部追加被丢)**,或输出含外部态但应用/persist 路径丢弃。
- 三方输入:base = `.cutforge/bases` 最新快照(2 clips,clip_add 后);disk = 外部写(3 clips);
  local = 装载态 = 快照 + oplog 重放(2 clips)。表 2 行(`is_unchanged(base, l)` → 采 disk)
  与 `merge_id_array`(disk 独有元素 `_ => push`)静态读均应返回 3——**嫌疑集中在
  e4605ec(BUG-08 序敏感重写)后的实际三方输入与静态推演不符处**:
  建议先在 `sync_with_disk` 打印 base/disk/local 的 tracks id 序列实测定案。
- 提交:e4605ec「fix(core,io,mcp): 审查报告 v2 第 1 波」(2026-10-05 02:50),merge.rs 的
  BUG-08 序敏感重写(id_seq/same_id_set/reorder_conflict)是本轮 diff 面。

## 验收
- 上述最小复现:外部追加后 rpc 与磁盘均含 V1-901(且 rev 语义符合 sync 设计);
- `python tools/e2e_ui_smoke.py` 第 13 步绿(连跑 3 遍);
- `python tools/e2e_events.py` 不回退。

## 关联
- MCP 轮已交付的加固不受此回归影响、修复后立即生效:
  壳长轮询降级面双路 resync 对账兜底(成功 5 轮/失败 2 次合成 resync 全量刷新)、
  `/events` 响应 `Cache-Control: no-store`、pollOnce `cache:'no-store'`。
