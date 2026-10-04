//! Workspace 持久化管线:引擎脏真相源落盘、写入窗口漂移检测、
//! 备份 → 原子写 → baseRev 快照 → OpLog 追加 → rev 落盘 → 标注落盘。
//! 每个落盘动作一个具名函数(4.2 八步的磁盘侧步骤),`persist` 只做编排。

use std::io;
use std::path::Path;

use cutforge_core::merge::MergeOutcome;

use super::Workspace;
use crate::backup;
use crate::paths::NOTES_REL;

impl Workspace {
    /// 引擎脏文件 → 盘面(先文件后记账;P1-9:写失败时 oplog 尚未记账)。
    /// notes.json 特殊:重载 NotesStore(撤销/重做恢复标注态)。
    pub(super) fn reconcile_files(&mut self) -> io::Result<()> {
        let truths = self.layout.truths;
        for file in self.engine.take_dirty_files() {
            if file == "notes.json" {
                let Some(v) = self.engine.file_state("notes.json").cloned() else {
                    continue;
                };
                if v != self.notes.to_value() {
                    let store =
                        cutforge_core::notes::NotesStore::from_value(&v).map_err(|errs| {
                            io::Error::other(format!(
                                "CF-005 SCHEMA_DRIFT(notes.json): {}",
                                errs.join("; ")
                            ))
                        })?;
                    self.notes = store;
                    self.notes_dirty = true;
                }
                continue;
            }
            let Some(rel) = truths.iter().find(|(n, _)| *n == file).map(|(_, r)| r) else {
                continue;
            };
            let Some(v) = self.engine.file_state(&file).cloned() else {
                continue;
            };
            if self.files.get(&file) != Some(&v) {
                let mut buf = serde_json::to_vec_pretty(&v)?;
                buf.push(b'\n');
                crate::atomic::atomic_write(&self.root.join(rel), &buf)?;
                self.files.insert(file, v);
            }
        }
        Ok(())
    }

    /// 步骤 5-7:备份 → 原子写 project.json(`_meta` 旁路回写)→ 追加 oplog →
    /// rev 落盘 → 标注落盘。真相源文件本体已在 reconcile_files 先行落盘。
    ///
    /// 编排:每步一个具名函数,依次调用;执行序与拆分前逐字一致
    /// (备份/原子写在前、OpLog 追加在后——先文件后记账,P1-9)。
    pub(super) fn persist(&mut self) -> io::Result<()> {
        self.check_window_drift()?; // 前置:写入窗口漂移检测(M9-1)
        self.backup_project()?; // 4.2 八步之 5:备份旧 project.json
        self.atomic_write_project()?; // 4.2 八步之 6:project.json 原子替换
        self.snapshot_bases()?; // M9-1:同步点 baseRev 快照 + LRU(bases.rs)
        self.append_oplog()?; // 4.2 八步之 4:Op 追加 jsonl(append-only)
        self.write_rev()?; // 4.2 八步之 6/7:rev 落盘
        self.flush_notes()?; // 标注落盘(有变化才写)
        // 自动快照(册六 T6.1):env 配置间隔,缺省开;快照是派生物,
        // 失败不影响写路径结果(与登记同口径)。
        let (interval, keep) = crate::snapshot::config_from_env();
        let _ = crate::snapshot::snapshot_if_due(&self.root, interval, keep);
        // 空闲压实(R-13③):快照在盘且 rev 越过阈值 → 截断日志 rev≤S 前缀。
        // 失败/放弃不影响写路径结果(与快照同口径,静默让位);放在本地写入
        // 登记**之前**——登记以当下磁盘实况为准,压实后的指纹才是"本进程最近
        // 写入"的基准,否则同步守护会把本次压实误判为外部改动而空开合并。
        let _ = self.compact_oplog();
        // 本进程写入登记(T1.8 性能专项):供同步守护判别"变化来自自己"而免开合并。
        // 以当下磁盘实况为准(自证写入已完成且一致);失败不影响写路径结果。
        crate::fresh::note_local_write(&self.root);
        Ok(())
    }

    /// OpLog 空闲压实(R-13③;公开维护口:persist 写尾自动调用,亦可由
    /// 测试/CLI/MCP 显式触发)。语义与两步原子协议(core::compact 模块文档):
    ///
    /// - 步 1(快照)由快照管线先行产出:最新 `.cutforge/snapshots/r<S>/` 不在
    ///   盘、或其 project.json 不可读 → 放弃(无快照宁可不压——护城河 6:
    ///   OpLog 即历史,不可压坏);
    /// - `compact::plan`(阈值 1000、旧日志缺 rev 放弃)不给计划 → 放弃;
    /// - 步 2(截断):把 `.cutforge/oplog/*.jsonl` 整行截到 rev > S
    ///   (`truncate_oplog_retained`:先全量校验后原子重写,落盘单点
    ///   `atomic_write`;分片内原子 ⇒ kill -9 只会留下「未开始/已完成」的
    ///   分片态,跨分片中间态由快照优先 open 幂等吸收 = TC-IO-SNAP-002 物理化)。
    ///
    /// 不动 project.json/rev/notes(先文件后记账次序不受扰动);内存引擎的
    /// 全量虚拟历史保持不变(persisted 游标不因截断位移:截掉的 rev≤S 前缀
    /// 全部已落盘,追加面从 `persisted` 起仍未落盘,语义自洽)。
    /// 返回是否执行了截断。
    pub fn compact_oplog(&mut self) -> io::Result<bool> {
        let Some(s_rev) = crate::snapshot::latest_snapshot_rev(&self.root) else {
            return Ok(false);
        };
        if crate::snapshot::read_snapshot_base(&self.root, s_rev).is_none() {
            return Ok(false); // 快照残缺:放弃压实,待下次快照产出后再试
        }
        // 廉价头窗门:各分片首行 rev 都已 > S ⇒ 盘面已无前缀可截(压实的常态
        // 稳态),直接跳过——避免每次 persist 对大日志做全量行解析。
        if !oplog_may_contain_prefix(&self.root, s_rev) {
            return Ok(false);
        }
        let Some(plan) = cutforge_core::compact::plan(self.engine.oplog(), Some(s_rev)) else {
            return Ok(false);
        };
        truncate_oplog_retained(&self.root, plan.snapshot_rev)?;
        Ok(true)
    }

    /// persist 前置:磁盘 != 上次同步视图 → 外部在窗口内写入。
    /// 可自动合并 → 采纳(继续写);冲突 → 冲突落盘、本地重载、报 CONFLICT。
    fn check_window_drift(&mut self) -> io::Result<()> {
        let Some(synced) = self.synced_disk.clone() else {
            return Ok(());
        };
        let Ok(text) = std::fs::read_to_string(self.root.join(self.layout.project_rel)) else {
            return Ok(());
        };
        let Ok(mut cur) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Ok(());
        };
        let cur_meta = cur.as_object_mut().and_then(|o| o.remove("_meta"));
        if let Some(m) = cur_meta {
            self.meta_bypass = Some(m);
        }
        if cur == synced {
            return Ok(());
        }
        let local = match self.engine.query(cutforge_core::engine::Query::ProjectView) {
            cutforge_core::engine::Answer::Project(v) => v,
            _ => unreachable!(),
        };
        match cutforge_core::merge::three_way_merge(&synced, &cur, &local) {
            MergeOutcome::Merged(v) => {
                self.synced_disk = Some(cur);
                if v != local {
                    self.adopt_merged(v)?;
                }
                Ok(())
            }
            MergeOutcome::Conflicts(conflicts) => {
                for c in &conflicts {
                    self.persist_conflict(c);
                }
                self.reload_from_disk()?;
                Err(io::Error::other(format!(
                    "CONFLICT: 写入窗口内外部已改动同一工程({} 项冲突),本地待写已弃用,请裁决后重试",
                    conflicts.len()
                )))
            }
        }
    }

    /// 4.2 八步之 5:备份旧 project.json(全局约定 B.8:可回滚)。
    /// 仅在确有 Op 待记账(persisted==0 或落后于 oplog)时备份,与拆分前条件一致。
    fn backup_project(&self) -> io::Result<()> {
        let ops = self.engine.oplog().ops();
        if (self.persisted == 0 || self.persisted < ops.len())
            && let Ok(old) = std::fs::read(self.root.join(self.layout.project_rel))
        {
            backup::backup_file(&self.root, self.layout.project_rel, &old)?;
        }
        Ok(())
    }

    /// 4.2 八步之 6:project.json 原子替换(`_meta` 旁路回写;唯一落盘点
    /// `atomic::atomic_write`)。写后更新同步点视图(窗口漂移检测基准)。
    fn atomic_write_project(&mut self) -> io::Result<()> {
        let value = self.engine.query(cutforge_core::engine::Query::ProjectView);
        let cutforge_core::engine::Answer::Project(mut v) = value else {
            unreachable!()
        };
        if let Some(meta) = &self.meta_bypass
            && let Some(obj) = v.as_object_mut()
        {
            obj.insert("_meta".into(), meta.clone());
        }
        let mut buf = serde_json::to_vec_pretty(&v)?;
        buf.push(b'\n');
        crate::atomic::atomic_write(&self.root.join(self.layout.project_rel), &buf)?;
        self.synced_disk = Some(v);
        Ok(())
    }

    /// 4.2 八步之 4:新 Op 追加 `.cutforge/oplog/<日>.jsonl`(按天切分;append-only)。
    /// 从 `persisted` 游标续写(实序在原子写之后,先文件后记账)。
    /// R-01 组提交:本批 Op **单次打开、逐行 write_all、返回前 sync 一次**
    /// (持久语义 = "apply 返回前 sync 一次";全部在 io 层完成,不需内核改调用点)。
    fn append_oplog(&mut self) -> io::Result<()> {
        let ops = self.engine.oplog().ops();
        if self.persisted >= ops.len() {
            return Ok(());
        }
        let day = today_compact();
        let oplog_file = self
            .root
            .join(".cutforge/oplog")
            .join(format!("{day}.jsonl"));
        let mut lines = Vec::new();
        while self.persisted < ops.len() {
            lines.push(format!(
                "{}\n",
                serde_json::to_string(&ops[self.persisted])?
            ));
            self.persisted += 1;
        }
        crate::atomic::append_lines(&oplog_file, &lines)
    }

    /// 4.2 八步之 6/7:rev 落盘(`.cutforge/rev`;open 时与 oplog 对账修复)。
    fn write_rev(&self) -> io::Result<()> {
        crate::atomic::atomic_write(
            &self.root.join(".cutforge/rev"),
            format!("{}\n", self.engine.rev()).as_bytes(),
        )
    }

    /// 标注落盘(有变化才写;notes.json 不在引擎脏文件路由内,由脏标记驱动)。
    fn flush_notes(&mut self) -> io::Result<()> {
        if self.notes_dirty {
            let mut buf = serde_json::to_vec_pretty(&self.notes.to_value())?;
            buf.push(b'\n');
            crate::atomic::atomic_write(&self.root.join(NOTES_REL), &buf)?;
            self.notes_dirty = false;
        }
        Ok(())
    }

    /// 弃用本地待写:从磁盘真相重载全部状态(外部改动获胜,4.7)。
    fn reload_from_disk(&mut self) -> io::Result<()> {
        let (engine, persisted, notes, meta_bypass, files, synced_disk, layout, repair) =
            Self::load(&self.root, true)?; // 冲突路径在写锁内,装载期修复语义与写通道一致
        self.engine = engine;
        self.persisted = persisted;
        self.notes = notes;
        self.notes_dirty = false;
        self.meta_bypass = meta_bypass;
        self.files = files;
        self.synced_disk = synced_disk;
        self.layout = layout;
        self.repair = repair;
        Ok(())
    }
}

/// 各分片是否可能仍含 `rev ≤ snapshot_rev` 的行(廉价头窗判定):只读每分片
/// 首个非空行;首行 rev 已 > S ⇒ 追加序下该分片不存在更小 rev,无前缀可截。
/// 首行不可解析/窗口内无完整行 = 保守按「可能含」处理,交由截断的全量行解析
/// 定夺(宁可不压不可压错)。午夜跨日散片(rev 较小却落在较新分片尾部)属
/// 保守误报,只影响是否进入全量判定,不影响正确性。
fn oplog_may_contain_prefix(root: &Path, snapshot_rev: u64) -> bool {
    let Ok(entries) = std::fs::read_dir(root.join(".cutforge/oplog")) else {
        return false;
    };
    for e in entries.flatten() {
        let p = e.path();
        if !p.extension().is_some_and(|x| x == "jsonl") {
            continue;
        }
        match first_line_rev(&p) {
            Some(Some(r)) if r > snapshot_rev => {}
            _ => return true,
        }
    }
    false
}

/// 分片首个非空行的 rev:`Some(Some(rev))` = 首 Op 行的 rev;
/// `Some(None)` = 首行存在但不可解析为 Op;`None` = 无完整行可读(空/超宽行)。
fn first_line_rev(path: &Path) -> Option<Option<u64>> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut head = Vec::new();
    file.take(64 * 1024).read_to_end(&mut head).ok()?;
    let line = head
        .split(|&b| b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .find(|l| !l.iter().all(|&b| b.is_ascii_whitespace()))?;
    #[derive(serde::Deserialize)]
    struct RevOnly {
        #[serde(default)]
        rev: Option<u64>,
    }
    serde_json::from_slice::<RevOnly>(line).ok().map(|r| r.rev)
}

/// UTC 紧凑日期 YYYYMMDD(oplog 按天切分;算法唯一来源 core::timeutil)。
fn today_compact() -> String {
    cutforge_core::timeutil::now_date_compact()
}

/// 步 2(截断):把 `.cutforge/oplog/*.jsonl` 整行截到 `rev > snapshot_rev`
/// 的保留面。两遍走:先全量解析校验(任一分片存在非法行/旧格式缺 rev 行 →
/// 整体放弃、零写入,不会出现半新半旧的分片组合),再逐分片 `atomic_write`
/// 原子重写(落盘单点,护城河 5;行级重写 = 截断点必落在整行边界,
/// fresh.rs 首/末行 rev+FNV 指纹在截断后自洽)。空分片保留为 0 字节文件
/// (文件名面稳定,扫描与指纹均兼容)。返回是否发生写入。
fn truncate_oplog_retained(root: &Path, snapshot_rev: u64) -> io::Result<bool> {
    let oplog_dir = root.join(".cutforge/oplog");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&oplog_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    // 第一遍:全量解析 + 组装各分片保留面(放弃路径不产生任何写入)
    let mut rewrites: Vec<(std::path::PathBuf, Vec<u8>)> = Vec::new();
    for file in files {
        let content = std::fs::read(&file)?;
        let mut kept: Vec<u8> = Vec::with_capacity(content.len());
        for line in content.split(|&b| b == b'\n') {
            if line.iter().all(|&b| b.is_ascii_whitespace()) {
                continue;
            }
            let op: cutforge_core::oplog::Op = serde_json::from_slice(line).map_err(|e| {
                io::Error::other(format!(
                    "压实放弃({}): 存在非法 Op 行,宁可不压: {e}",
                    file.display()
                ))
            })?;
            let Some(r) = op.rev else {
                return Err(io::Error::other(
                    "压实放弃: 存在缺 rev 的旧格式 Op,宁可不压(护城河 6)",
                ));
            };
            if r > snapshot_rev {
                kept.extend_from_slice(line);
                kept.push(b'\n');
            }
        }
        if kept != content {
            rewrites.push((file, kept));
        }
    }
    // 第二遍:逐分片原子重写(单点落盘;中断 = 该分片保持旧态,open 幂等吸收)
    let mut wrote = false;
    for (file, data) in rewrites {
        crate::atomic::atomic_write(&file, &data)?;
        wrote = true;
    }
    Ok(wrote)
}
