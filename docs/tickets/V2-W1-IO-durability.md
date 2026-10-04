# V2-W1-IO 耐久与并发(io 层)

范围:crates/cutforge-io/**(atomic/lock/open/persist/fresh/watcher/backup/recover/probe/snapshot 缺省开)。

条目:R-01(fsync 纪律)、R-02(锁接管 pid+心跳)、R-03(半行修复模式+RepairReport)、
R-04(差异自愈 ReconciledOp)、R-05(oplog 指纹首尾 rev+末行哈希)、R-10(watcher 退避+线程退出)。
纪律:fsync 性能预案(group commit)按报告;与 BUG-07(内核 RevGap)衔接的修复模式冒泡为用户可见状态;
报告文件:C:\Users\Administrator\Desktop\CutForge-迭代审查报告-v2.md。

状态:进行中
