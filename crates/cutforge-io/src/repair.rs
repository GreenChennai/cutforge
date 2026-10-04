// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 显式修复模式(R-03/R-04):oplog 半行截断与"先文件后记账"崩溃窗口的
//! 结构化修复报告。修复绝不静默:修复前自动备份 `.cutforge/` 到
//! `.cutforge/recovery-{ts}/`,修复结果落 `.cutforge/repair-report.json`,
//! 并经 `Workspace::repair_report()` 面向 UI/MCP 冒泡为用户可见状态。

use crate::atomic;
use cutforge_core::timeutil;
use std::io;
use std::path::{Path, PathBuf};

/// 修复报告(R-03/R-04 共用):一次打开装载期发现并(在写通道打开下)执行的
/// 全部修复动作的结构化记录。字段全部面向用户呈现,不做任何静默吞并。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairReport {
    /// 工程根。
    pub root: PathBuf,
    /// 发生半行截断的 oplog 分片文件名(按装载序;空 = 无截断)。
    pub truncated_files: Vec<String>,
    /// 丢失操作数:修复前记账 rev(disk rev)− 最后完整 Op 的 rev。
    pub lost_ops: u64,
    /// 差异自愈(R-04)补记的 ReconciledOp rev 列表。
    pub reconciled_revs: Vec<u64>,
    /// 修复前 `.cutforge/` 的备份目录(无修复动作 = None)。
    pub backup_path: Option<PathBuf>,
    /// 报告时间(RFC3339)。
    pub ts: String,
}

impl RepairReport {
    /// 是否存在任何修复动作。
    pub fn has_actions(&self) -> bool {
        !self.truncated_files.is_empty() || self.lost_ops > 0 || !self.reconciled_revs.is_empty()
    }

    /// 用户可读摘要(UI/MCP 直接呈现;绝不静默的出口)。
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.truncated_files.is_empty() {
            parts.push(format!(
                "oplog 半行截断已修复(分片:{})",
                self.truncated_files.join(", ")
            ));
        }
        if self.lost_ops > 0 {
            parts.push(format!("丢失 {0} 个操作", self.lost_ops));
        }
        if !self.reconciled_revs.is_empty() {
            parts.push(format!(
                "差异自愈:补记 rev {} 的记账(该变更已落盘但记账丢失,不可再撤销)",
                self.reconciled_revs
                    .iter()
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        if let Some(b) = &self.backup_path {
            parts.push(format!("修复前备份:{}", b.display()));
        }
        if parts.is_empty() {
            "无需修复".to_string()
        } else {
            format!("工程已自动修复:{}", parts.join(";"))
        }
    }

    pub(crate) fn to_json(&self) -> String {
        let truncated: Vec<String> = self
            .truncated_files
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect();
        let reconciled: Vec<String> = self.reconciled_revs.iter().map(|r| r.to_string()).collect();
        let backup = match &self.backup_path {
            Some(p) => format!("\"{}\"", p.display().to_string().replace('\\', "/")),
            None => "null".to_string(),
        };
        format!(
            "{{\"root\":\"{}\",\"truncatedFiles\":[{}],\"lostOps\":{},\"reconciledRevs\":[{}],\"backupPath\":{},\"ts\":\"{}\"}}\n",
            self.root.display().to_string().replace('\\', "/"),
            truncated.join(","),
            self.lost_ops,
            reconciled.join(","),
            backup,
            self.ts
        )
    }
}

/// 修复报告落盘位置。
pub fn report_path(root: &Path) -> PathBuf {
    root.join(".cutforge/repair-report.json")
}

/// 把修复报告写入 `.cutforge/repair-report.json`(失败不阻塞修复本体)。
pub fn persist_report(root: &Path, report: &RepairReport) {
    let _ = atomic::atomic_write(&report_path(root), report.to_json().as_bytes());
}

/// 修复前备份:把 `.cutforge/` 复制到 `.cutforge/recovery-{ts}-{4位序}/`
/// (跳过既有 `recovery-*` 目录避免递归)。目录名带唯一后缀:同秒内连续两次
/// 修复(截断修复 → 再开又发现新截断面)不得互相覆盖备份——BUG-12 同款教训。
/// 返回备份目录。
pub fn backup_state_dir(root: &Path) -> io::Result<PathBuf> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let src = root.join(".cutforge");
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_millis();
    let suffix = format!(
        "{:04x}",
        (n.wrapping_mul(0x9E37_79B9) ^ t) as u16 as u32 & 0xffff
    );
    let dest = src.join(format!(
        "recovery-{}-{suffix}",
        timeutil::now_datetime_compact()
    ));
    copy_dir_skipping_recovery(&src, &dest)?;
    Ok(dest)
}

fn copy_dir_skipping_recovery(src: &Path, dest: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for e in std::fs::read_dir(src)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("recovery-") {
            continue;
        }
        let p = e.path();
        let target = dest.join(&name);
        if p.is_dir() {
            copy_dir_skipping_recovery(&p, &target)?;
        } else if let Ok(data) = std::fs::read(&p) {
            atomic::atomic_write(&target, &data)?;
        }
    }
    Ok(())
}

/// ReconciledOp(R-04 差异自愈)的**记账 Op 构造**。
///
/// 语义:project/rev 记账已到 `rev`,但 oplog 缺该 rev 的 Op(先文件后记账的
/// 崩溃窗口)。补记一条屏障 Op:目标指向恢复标记文件(不在任何真相源路由内),
/// 撤销栈据此在其处停住——`Engine::undo` 对未知 file 的撤销会以
/// "撤销基准不一致……拒绝盲写" 明确拒绝,实现"undo 到此处不可再退"。
///
/// 结构契约(内核与壳集成方):
/// `op_kind = "resolve-conflict"`,`auto` 不置位(必须入撤销栈作屏障),
/// `summary` 以 `[差异自愈]` 开头,`before/after = null`,`target.path = "/__reconciled__"`。
pub(crate) fn reconciled_op(rev: u64, base_rev: u64, op_id: String) -> cutforge_core::oplog::Op {
    use cutforge_core::oplog::{Actor, Op, OpKind, OpTarget};
    Op {
        op_id,
        ts: timeutil::now_rfc3339(),
        actor: Actor::script("cutforge-io/recovery"),
        target: OpTarget {
            file: RECOVERY_MARKER_FILE.into(),
            path: "/__reconciled__".into(),
        },
        op_kind: OpKind::ResolveConflict,
        before: serde_json::Value::Null,
        after: serde_json::Value::Null,
        base_rev: cutforge_core::format_rev(base_rev),
        rev: Some(rev),
        // 稳定 id 寻址字段(BUG-06/A-03,core 并行新增):恢复屏障无 clip 语义,恒 None
        target_id: None,
        caused_by: None,
        summary: format!("[差异自愈] rev {rev} 的变更已落盘但记账丢失,补记屏障(不可再撤销)"),
        request_id: None,
        auto: None,
    }
}

/// ReconciledOp 的屏障标记文件名(不在 `paths` 任何真相源清单内,
/// `Engine::undo` 按其路由必然拒绝,构成"到此不可再退"的硬屏障)。
pub(crate) const RECOVERY_MARKER_FILE: &str = "__recovered_rev_gap__";

/// oplog 半行截断检测的行级结果。
pub(crate) struct OplogLoad {
    pub log: cutforge_core::oplog::OpLog,
    /// 发生截断的文件名(按装载序)。
    pub truncated_files: Vec<String>,
    /// 各分片"最后完整行"的字节终点(物理截断修复用):文件名 → 截断字节长。
    pub truncation_points: Vec<(PathBuf, u64)>,
}

/// 扫描 `.cutforge/oplog/*.jsonl`:解析完整 Op;半行(上次写入中断)记录
/// 截断点与分片名,不计入日志。与旧装载语义一致:截断只中断当前分片,
/// 后续分片照常装载(§2.3 R-03 措辞修正)。
pub(crate) fn scan_oplog(root: &Path) -> OplogLoad {
    let mut out = OplogLoad {
        log: cutforge_core::oplog::OpLog::new(),
        truncated_files: Vec::new(),
        truncation_points: Vec::new(),
    };
    let oplog_dir = root.join(".cutforge/oplog");
    let Ok(entries) = std::fs::read_dir(&oplog_dir) else {
        return out;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    for file in files {
        let content = match std::fs::read(&file) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mut good_end: usize = 0;
        let mut truncated = false;
        for line in split_lines(&content) {
            // 行终点含行尾换行(截断后文件仍以 \n 结尾——否则下一次 append
            // 会把新行拼接到末行,修复越修越坏)
            let line_end = |l: &[u8]| {
                let mut end = l.as_ptr() as usize - content.as_ptr() as usize + l.len();
                if content.get(end) == Some(&b'\n') {
                    end += 1;
                }
                end
            };
            let s = match std::str::from_utf8(line) {
                Ok(s) => s,
                Err(_) => {
                    truncated = true;
                    break;
                }
            };
            if s.trim().is_empty() {
                good_end = line_end(line);
                continue;
            }
            match serde_json::from_str::<cutforge_core::oplog::Op>(s) {
                Ok(op) => {
                    out.log.push_loaded(op);
                    good_end = line_end(line);
                }
                Err(_) => {
                    truncated = true;
                    break;
                }
            }
        }
        if truncated {
            out.truncated_files.push(
                file.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
            out.truncation_points.push((file.clone(), good_end as u64));
        }
    }
    out
}

/// 按换行切分,保留行内容(不含 \n;\r\n 的 \r 留给 trim)。
fn split_lines(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, &b) in data.iter().enumerate() {
        if b == b'\n' {
            out.push(&data[start..i]);
            start = i + 1;
        }
    }
    if start < data.len() {
        out.push(&data[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil;

    #[test]
    fn report_summary_and_json_roundtrip_shape() {
        let root = fsutil::temp_dir("repair-meta");
        fsutil::ensure(&root).unwrap();
        let r = RepairReport {
            root: root.clone(),
            truncated_files: vec!["20260101.jsonl".into()],
            lost_ops: 2,
            reconciled_revs: vec![5],
            backup_path: Some(root.join(".cutforge/recovery-x")),
            ts: "2026-10-04T00:00:00Z".into(),
        };
        assert!(r.has_actions());
        let s = r.summary();
        assert!(s.contains("20260101.jsonl") && s.contains("丢失 2 个操作") && s.contains("rev 5"));
        let j = r.to_json();
        assert!(j.contains("\"lostOps\":2") && j.contains("\"reconciledRevs\":[5]"));
        assert!(r.summary().contains("备份"));
        fsutil::cleanup(&root);
    }

    #[test]
    fn scan_oplog_detects_truncation_and_keeps_later_files() {
        let root = fsutil::temp_dir("repair-scan");
        fsutil::ensure(&root.join(".cutforge/oplog")).unwrap();
        let op = |rev: u64| {
            let o = reconciled_op(rev, rev.saturating_sub(1), format!("op-{rev}"));
            serde_json::to_string(&o).unwrap()
        };
        let f1 = root.join(".cutforge/oplog/20260101.jsonl");
        let f2 = root.join(".cutforge/oplog/20260102.jsonl");
        std::fs::write(&f1, format!("{}\n{}\n", op(1), op(2))).unwrap();
        // 分片 2 尾部半行(中间分片损坏形态:截断的是较后分片,且无后续分片)
        std::fs::write(&f2, format!("{}\n{{\"op_id\":\"op-3", op(3))).unwrap();
        let loaded = scan_oplog(&root);
        assert_eq!(loaded.log.len(), 3, "完整行全装载");
        assert_eq!(loaded.truncated_files, vec!["20260102.jsonl".to_string()]);
        assert_eq!(loaded.truncation_points.len(), 1);
        // 物理截断点必须落在行尾换行之后(截断后文件仍以 \n 结尾,
        // 否则下一次 append 把新行拼接到末行——修复越修越坏)
        let (file, end) = &loaded.truncation_points[0];
        let data = std::fs::read(file).unwrap();
        let kept = &data[..(*end as usize)];
        assert_eq!(
            kept.last(),
            Some(&b'\n'),
            "截断保留面必须以换行结尾: {:?}",
            String::from_utf8_lossy(&kept[kept.len().saturating_sub(40)..])
        );
        // 按截断点截断后再扫:零截断、零丢失
        std::fs::write(file, kept).unwrap();
        let rescanned = scan_oplog(&root);
        assert!(
            rescanned.truncated_files.is_empty(),
            "截断修复后不得再报截断"
        );
        assert_eq!(rescanned.log.len(), 3);
        fsutil::cleanup(&root);
    }

    /// 恢复备份目录唯一性:同秒两次 backup_state_dir 不得同名互覆(BUG-12 同款)。
    #[test]
    fn recovery_backup_dirs_unique_within_same_second() {
        let root = fsutil::temp_dir("repair-bak-uniq");
        fsutil::ensure(&root.join(".cutforge/oplog")).unwrap();
        let a = backup_state_dir(&root).unwrap();
        let b = backup_state_dir(&root).unwrap();
        assert_ne!(a, b, "同秒两次修复备份不得共用目录");
        assert!(a.is_dir() && b.is_dir());
        fsutil::cleanup(&root);
    }
}
