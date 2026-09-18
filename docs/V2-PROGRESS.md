# V2 迭代进度总账(M8 → M13)

> 唯一执行依据:`docs/ITERATION-PLAN-v2.0.md`。本文件是滚动台账:每阶段收尾时,
> 复盘发现的问题与可改进项**强制折入下一阶段任务单**(全权委托条款)。
> 门禁纪律不变:每指标可判定、可存档、不得为通过而放宽。

## M8 · 止血与信任修复(2026-09-19 完成)

**交付**:5 个 P0 全清 + M8-5 诚实性,门禁 M8-1~M8-5 全绿。

| 门禁 | 判定 | 结果 |
|---|---|---|
| M8-1 文件级撤销为真 | `file_level_undo`(内核 4 例)+ IO 侧 notes/cutlist 还原 + auto 不入撤销栈 | ✅ |
| M8-2 真实工程可打开 | 夹具 = CutFlow rs_ir.py 真实生成(含 `_meta`),open/apply/reopen/roundtrip 后 `_meta` 逐字节相等 | ✅ |
| M8-3 渲染三修 | `render_matrix_fixture`:双画幅分辨率互不污染 / 人声 startMs 落点 / sfx 1500ms 起播;ffmpeg 缺失即失败 | ✅ |
| M8-4 并发零丢失 | 双线程 × 5 写,rev=oplog=2N,盘面不分叉 | ✅ |
| M8-5 协议诚实 | 错误码穷举(占位符清零)、矩阵单一真相源(7/15 诚实计数锁定)、6 编排工具协议完整、python 探测 | ✅ |

**全量回归**:cargo 91/91(基线 77 → +14 门禁)、pytest 3/3、gates M0/M1 绿。
**提交**:59f7ed6(P0-1/2/5)→ f4765da(P0-3/4)→ e5d6f5e(M8-5)+ ADR-0001~0003。

### 复盘:M8 新发现(全部折入 M9+)

| # | 发现 | 处置 |
|---|---|---|
| M8-R1 | "字节级还原"门禁对外来文件格式不可达(undo 走语义域重序列化),已降为 canonical 语义等价并记录于 ADR-0001 | M9 评估可选的会话内字节快照日志;不阻塞 |
| M8-R2 | CI rust job 此前**无 ffmpeg**——渲染类门禁在 CI 上根本无法运行,这是 V1 虚标能溜过门禁的基建性原因 | 已装(本次);M11-1 全量对拍依赖它,须保持 |
| M8-R3 | `render_variants` 仍每变体全链重跑(计划书 6.5 的"共享缓存只分叉 encode"未兑现) | M11 真分叉 |
| M8-R4 | `merge_from_disk` 若在 `open_exclusive` 之外被调用则无锁保护 | M9 watcher 调用点必须显式持锁,或收编为 Workspace 内部方法 |
| M8-R5 | MCP dispatch 对只读工具也走 `open_exclusive`(防 TOCTOU),长操作(渲染编排)持锁会阻塞并发工具 | M9-2 watcher 设计约束:merge 只在短临界区;渲染编排保持子进程(不持 workspace 锁) |
| M8-R6 | `stage_status` 仍只查 `_state` 文件存在性(计划书结合面⑧) | M9-3 正题 |
| M8-R7 | `tests/__pycache__`、`tools/**/__pycache__` 曾被跟踪 | 已解除跟踪 + .gitignore |

## M9 · 主链路贯通(2026-09-19 完成)

**交付**(M9-1~M9-4 全绿):

| 门禁 | 判定 | 结果 |
|---|---|---|
| M9-1 冲突真触发 | `conflict_real_repro_and_stop_writes`:写入窗口内外部同字段异改 → CF-001 + 本地待写弃用 + 停写;`window_drift_different_fields_auto_merge` 异字段自动合并;baseRev 快照链 `.cutforge/bases/`(LRU 32,40 次写后 ≤32 份) | ✅ |
| M9-2 外部改动可见 | `external_edit_visible_within_1s`:守护线程 + SyncHub 长轮询;HTTP `/events?root=&since=` 推 `workspace.changed`;MCP dispatch 幂等注册守护 | ✅ |
| M9-3 同口径 + keep 重算 | cutforge:stage_status 优先 rs_run --status(降级如实标注)、cut_apply 服务端重算 keep/removedMs(金样 = rs_cut.finalize_cutlist 真实生成,6 例对拍);CutFlow:rs_run outputs_hash 带外改写检测、B8 护栏识别 CutForge 编辑痕迹(schemaVersion)、REBUILD/SKILL cutlist 脏提示改道 rs_cut --apply | ✅ |
| M9-4 桥升版 + 双侧冒烟 | rs_editor id 回退(V1#2+idSource)、rs_gate probe 真检 gate.py、rs_oplog 半行截断对齐、SKILL.md 登记四桥;cutforge 新增 4 桥冒烟 pytest,CutFlow 新建 CI(.github/workflows/gate.yml)浅克隆 cutforge 跑桥探针+烟测 | ✅ |

**回归**:cutforge cargo 97/97(+6)、pytest 7/7、gates M0/M1 绿;CutFlow 300/300。

### 架构决策(M9,已实施)

- **冲突触发的真实窗口**:同步点快照(`synced_disk`)+ persist 前漂移检测。
  "每写必先预合并 + 立即持久化"使两个 cutforge 进程之间结构性地不会产生 CF-001
  (后写者总以真祖先看到先写者);真正可触发的是**锁外外部写者**(CutFlow 管线/文本
  编辑器)落在 pre-merge 与 persist 之间——此时三路合并,同字段异改 → 冲突落盘、
  本地弃用、停写。快照 LRU 上限 32(计划书 V2-R4;未用 diff 存储,偏离已记录于 M8 折入项)。
- **v1 工程独占打开即升级规范形**:使外部检测与守护合并都有稳定的 v2 基准。

### 复盘:M9 新发现(折入后续阶段)

| # | 发现 | 处置 |
|---|---|---|
| M9-R1 | CutFlow `derive_keep` 对**尾部 remove** 产出的 keep 不覆盖片尾,`finalize_cutlist` 与 schema 断言会双双拒绝——结尾静音删除场景在现行语义下不可表达(CutFlow 自身矛盾,与本次改动无关的存量问题) | 折入 CutFlow 待办:需 ADR 澄清"keep 覆盖片尾"语义(允许尾删 vs 禁尾删);金样夹具已避开该形态 |
| M9-R2 | MCP stage_status 走 rs_run --status 时是子进程调用(~秒级);编辑器高频轮询应改走 `/events`+oplog rev,不要反复拉 stage_status | M10 Web 壳遵守 |
| M9-R3 | 守护线程与 MCP dispatch 各自 open_exclusive:冲突落盘可能由守护先写入,dispatch 停写提示需引导用户看 conflict_list(已实现,UI 侧待展示) | M10 冲突面板 |
| M9-R4 | `/events` 只推 project.json 变更;notes.json/cutlist.json 的外部改动(如 rs_cut --apply)不产生事件 | M10 扩展事件面 |

## M10 · 编辑能力(任务单)

1. **M9-1 baseRev 快照链**:`.cutforge/bases/<rev>.json`,persist 成功后保存"与磁盘
   同步点"的本地值;`merge_from_disk` 用真祖先做三路合并。
   门禁 M9-1:外部改 A 字段 + 本地改 A 字段 → 必触发 CF-001(当前实现该测试必红)。
   - 折入 M8-R4:merge 收编进锁内语义。
   - 存储口径:全量快照 + LRU 上限(32 个)——计划书 R4 建议只存 diff,但 JSON diff
     无既有基建、工程大小为几十 KB 量级,LRU 全量更简单且同样防膨胀(偏离已记录)。
2. **M9-2 watcher 接线**:常驻进程内轮询线程(mtime+hash 去抖)→ 锁内
   merge_from_disk → 冲突落盘 → HTTP `/events` 长轮询推 `workspace.changed`。
   门禁 M9-2:外部手改 project.json 后 ≤1s 事件可见且视图一致。
   - 折入 M8-R5:watcher 只做短临界区读合并,不做写。
3. **M9-3 阶段状态同口径 + cut_apply 语义**(双仓):
   - stage_status 解析 `_state/S*.json` 内容(done/stale/failed);
   - cut_apply 按 cuts[].action 重算 keep/removedMs(与 rs_cut --apply 契约对拍);
   - cutlist 脏提示指向 `rs_cut --apply`(CutFlow rs_run 阶段映射改动);
   - rs_run S3 记录输出 hash 识别带外改写(CutFlow 侧);
   - B8 护栏识别 `schemaVersion`/轨道 id 为 CutForge 写入(CutFlow rs_ir `_manual_edits`)。
   门禁 M9-3:MCP 改 action → rs_cut --apply 消费 → S3 重建 → IR 反映,无静默丢弃。
4. **M9-4 桥升版 + 双侧冒烟**:
   - rs_editor timeline 对 v1 IR 的 id 回退 `V1#2` + idSource 标注(CutFlow);
   - rs_gate --probe 真检 gate.py 存在(CutFlow);
   - rs_oplog 半行语义对齐(截断,CutFlow 侧);
   - SKILL.md 登记四桥(CutFlow);docs/FLOW.md/README 同步 M8 架构变化(M8-R2 折入);
   - cutforge CI 桥冒烟(pytest 调四桥脚本);CutFlow CI 浅克隆 cutforge 冒烟。
   门禁 M9-4:双侧 CI 桥冒烟绿。
