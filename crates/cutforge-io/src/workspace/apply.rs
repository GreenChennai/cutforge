//! Workspace 命令接口:唯一写入路径(4.2 八步)的**显式步骤函数管线**。
//!
//! `apply` 主流程按步骤依次调用,每步一个具名函数(册四扩展命令粒度的挂点):
//!
//! | 步骤 | 函数 | 对应 4.2 八步 |
//! |---|---|---|
//! | 1 | `step1_acquire_lock` | 申请工程锁(锁覆盖 open→apply→persist,P0-5) |
//! | 2 | `step2_pre_write_sync` | Engine 校验前置:冲突停写(4.7)+ 磁盘三路合并(M9-1) |
//! | 3 | `step3_engine_apply` | Engine::apply:baseRev 校验 → 变更 → schema+不变量,失败回滚;before==after 幂等短路 |
//! | 4 | `step4_flush_dirty_truths` | 新内容先落盘(先文件后记账,P1-9) |
//! | 5 | `step5_relocate_anchors` | 标注锚点重定位联动(4.9) |
//! | 6 | `step6_persist` | OpLog 追加 / 备份 / 原子写 / baseRev 快照 / rev 落盘 / 标注落盘 |
//! | 7 | (guard Drop) | 释放锁 |
//!
//! 步骤 6 内的每个磁盘动作同样是具名函数(`persist.rs`:backup_project /
//! atomic_write_project / append_oplog / write_rev / flush_notes + bases.rs:
//! snapshot_bases)。注意:磁盘写序保持拆分前原样(备份→原子写→OpLog→rev),
//! **不**按文档理想序(OpLog 追加在前)重排——崩溃恢复语义(rev/oplog 对账,
//! 见 `open.rs` 的 rev 对账)依赖该实序。

use std::io;

use cutforge_core::command::Command;
use cutforge_core::engine::{ApplyOpts, OpReceipt};
use cutforge_core::oplog::{Actor, OpKind};

use super::{Workspace, reject_to_io};
use crate::lock;

impl Workspace {
    /// 命令接口(唯一写入口的 IO 编排,4.2 八步);成功后联动标注重定位(4.9)。
    pub fn apply(&mut self, cmd: Command, actor: Actor, opts: ApplyOpts) -> io::Result<OpReceipt> {
        // 步骤 1:申请工程锁(open_exclusive 已持全程锁时不重复加,P0-5)
        let _guard = self.step1_acquire_lock()?;
        // 步骤 2:写前同步——未裁决冲突停写(4.7);磁盘分叉先三路合并(M9-1)
        self.step2_pre_write_sync()?;
        // 步骤 3:Engine::apply——baseRev 校验/变更/schema+不变量,失败回滚快照;
        // before==after 幂等短路(不升 rev、不产 Op,回执 idempotent=true)
        let receipt = self.step3_engine_apply(cmd, actor.clone(), opts)?;
        // 步骤 4:引擎脏真相源先落盘(先文件后记账;P1-9:写失败时 oplog 尚未记账)
        self.step4_flush_dirty_truths()?;
        // 步骤 5:锚点重定位联动(4.9;auto Op 留痕,内部自带一次同步落盘)
        self.step5_relocate_anchors(actor)?;
        // 步骤 6:落盘管线(备份 → 原子写 → 快照 → OpLog 追加 → rev 落盘 → 标注)
        self.step6_persist()?;
        Ok(receipt)
        // 步骤 7:释放锁 —— `_guard` Drop 自动释放(open_exclusive 全程锁不受影响)
    }

    pub fn undo(&mut self, actor: Actor) -> io::Result<OpReceipt> {
        let _guard = self.temp_lock()?;
        self.pre_write_sync()?;
        let receipt = self.engine.undo(actor.clone()).map_err(reject_to_io)?;
        self.reconcile_files()?;
        self.sync_notes_after_change(actor)?;
        self.persist()?;
        Ok(receipt)
    }

    pub fn redo(&mut self, actor: Actor) -> io::Result<OpReceipt> {
        let _guard = self.temp_lock()?;
        self.pre_write_sync()?;
        let receipt = self.engine.redo(actor.clone()).map_err(reject_to_io)?;
        self.reconcile_files()?;
        self.sync_notes_after_change(actor)?;
        self.persist()?;
        Ok(receipt)
    }

    /// 非 project.json 真相源(如 cutlist.json)的复合写:锁内登记审计 Op 并持久化。
    /// 文件本体由 reconcile 按 file_states 落盘(先文件后记账)。
    // 参数与 Op 字段一一对应(同 record_file_change 的理由)。
    #[allow(clippy::too_many_arguments)]
    pub fn record_change(
        &mut self,
        file: &str,
        path: &str,
        before: serde_json::Value,
        after: serde_json::Value,
        kind: OpKind,
        actor: Actor,
        opts: ApplyOpts,
    ) -> io::Result<OpReceipt> {
        let _guard = self.temp_lock()?;
        let receipt = self
            .engine
            .record_file_change(file, path, before, after, kind, actor, opts)
            .map_err(reject_to_io)?;
        self.reconcile_files()?;
        self.persist()?;
        Ok(receipt)
    }

    // ---------- 八步步骤函数(仅 apply 主流程按序调用) ----------

    /// 步骤 1:申请工程锁(已持 open_exclusive 全程锁时不重复加,P0-5)。
    fn step1_acquire_lock(&self) -> io::Result<Option<lock::LockGuard>> {
        self.temp_lock()
    }

    /// 步骤 2:写前同步(Engine 校验前置:冲突停写/三路合并)。
    fn step2_pre_write_sync(&mut self) -> io::Result<()> {
        self.pre_write_sync()
    }

    /// 步骤 3:Engine::apply——baseRev 前置校验 → 变更 → schema+重叠不变量
    /// (失败回滚引擎快照,拒绝码 Reject 映射 io::Error);
    /// before==after 幂等短路:不升 rev、不产 Op,回执 `idempotent=true`。
    fn step3_engine_apply(
        &mut self,
        cmd: Command,
        actor: Actor,
        opts: ApplyOpts,
    ) -> io::Result<OpReceipt> {
        self.engine.apply(cmd, actor, opts).map_err(reject_to_io)
    }

    /// 步骤 4:引擎脏真相源先落盘(先文件后记账,P1-9)。
    fn step4_flush_dirty_truths(&mut self) -> io::Result<()> {
        self.reconcile_files()
    }

    /// 步骤 5:标注锚点重定位联动(4.9;有变化则登记 auto Op 并落盘留痕)。
    fn step5_relocate_anchors(&mut self, actor: Actor) -> io::Result<()> {
        self.sync_notes_after_change(actor)
    }

    /// 步骤 6:落盘管线(备份/原子写/快照/OpLog 追加/rev/标注,各子步具名见 persist.rs)。
    fn step6_persist(&mut self) -> io::Result<()> {
        self.persist()
    }

    /// 已持有的全程锁之外临时补锁(open_exclusive 打开时不重复加锁)。
    fn temp_lock(&self) -> io::Result<Option<lock::LockGuard>> {
        if self.lock.is_some() {
            Ok(None)
        } else {
            lock::acquire(&self.root, 30_000, 20).map(Some)
        }
    }
}
