# V2-W3-IO OpLog 压实接线(R-13 收口)

范围:crates/cutforge-io/**(workspace/open.rs、workspace/persist.rs)+ 真实文件级回归测试。
前置:KERNEL2 已落 core 侧 `cutforge_core::compact::{plan, retained_ops}`、
`rebuild_stacks_incremental`、`Engine::replay_from_snapshot`(接口已备,契约级测试全绿)。

## 条目(报告 v2 R-13②③ 的 io 半边)
1. open.rs:最新 `.cutforge/snapshots/r<S>/` 存在 → 快照优先装载(`replay_from_snapshot(base, S, retained_ops)`);
   retained_ops 对"步 2 中断"残留前缀幂等已由 core 侧保证;
2. persist.rs:空闲期(rev 越过 1000 且快照存在)调 `compact::plan` 触发截断
   (只删 rev≤S 前缀,原子两步:新快照已由 snapshot.rs 定时产出 → 截断日志);
   repair.rs 指纹口径(fresh.rs 首/末行 rev+FNV)在截断后必须自洽——截断点即 retain_from;
3. 真实文件级回归(io 层新增):压实前后 open 结果相等;kill -9 中断截断 → open 自愈;
   快照缺失 → 放弃压实(旧日志缺 rev 放弃口径不变);TC-IO-SNAP-003/004。
4. 债务候选登记(不修):clips_patch 多轨 Op 面只取首轨 before/after——首轨无操作
   其余轨有变更时被幂等吞掉(KERNEL2 发现的既有边界,保持零变化)。

## 纪律
红-绿循环;护城河 §3 五条(写盘单点/先文件后记账/OpLog 即历史)不得破坏;
环境同前(D:\Temp、-j 2、禁 git、fmt 窄)。报告:接线点、回归清单、退出码。
状态:待 RENDER 完工后开工
