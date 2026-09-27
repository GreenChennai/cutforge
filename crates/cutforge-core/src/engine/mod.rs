// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 引擎(计划书 2.6/4.2/4.4):查询接口与命令接口严格分离。
//!
//! - `query`:纯投影,无副作用,可并发;
//! - `apply`/`undo`/`redo`:唯一写入口——校验 → 变更 → schema 验证 →
//!   生成 Op(含 baseRev)入 OpLog → rev 递增。
//!   步骤"校验前置条件"是'所见即所得'能成立的唯一原因:相对 baseRev
//!   已失效的写入会被拒绝,不存在静默覆盖(北极星指标)。
//!
//! 子模块划分(T1.3 纯移动拆分,对外路径经本文件 `pub use` 保持不变):
//! - `invariants`:拒绝原因 `Reject` 与不变量校验;
//! - `apply`:命令写入口(`apply`/`record_file_change`)与命令 → 变更翻译(`mutate`);
//! - `undo`:撤销/重做;
//! - `replay`:OpLog 回放与撤销/重做栈重建;
//! - `projection`:查询投影与状态语义 hash。

mod apply;
mod invariants;
mod projection;
mod replay;
mod undo;

pub use apply::{ApplyOpts, OpReceipt};
pub use invariants::Reject;
pub use projection::{canonical_json, Answer, Query};
pub use replay::{rebuild_stacks, rebuild_undo_stack};

use crate::command::ClipPatch;
use crate::model::Project;
use crate::oplog::OpLog;
use serde_json::{json, Value};

pub struct Engine {
    project: Project,
    log: OpLog,
    rev: u64,
    undo_stack: Vec<String>,
    redo_stack: Vec<String>,
    /// 非工程真相源(notes.json/cutlist.json 等)的内存态:文件级 Op 的
    /// 撤销/重做路由目标(ADR-0001:按 target.file 逆写,而非伪造指针回写)。
    file_states: std::collections::BTreeMap<String, Value>,
    /// 自上次 drain 以来被写脏的真相源文件(IO 层据此落盘,先文件后记账)。
    dirty_files: std::collections::BTreeSet<String>,
}

impl Engine {
    pub fn new(project: Project) -> Result<Self, Vec<String>> {
        project.to_validated_value()?;
        Ok(Self {
            project, log: OpLog::new(), rev: 0,
            undo_stack: Vec::new(), redo_stack: Vec::new(),
            file_states: std::collections::BTreeMap::new(),
            dirty_files: std::collections::BTreeSet::new(),
        })
    }

    /// IO 层加载既有工程时用:恢复(项目, OpLog, rev, 撤销栈, 文件态)五元组。
    pub fn restore(
        project: Project,
        log: OpLog,
        rev: u64,
        undo_stack: Vec<String>,
        file_states: std::collections::BTreeMap<String, Value>,
    ) -> Result<Self, Vec<String>> {
        project.to_validated_value()?;
        Ok(Self {
            project, log, rev, undo_stack,
            redo_stack: Vec::new(),
            file_states, dirty_files: std::collections::BTreeSet::new(),
        })
    }

    /// restore 的双栈版本(io 打开工程时用,重做栈跨 dispatch 可用)。
    #[allow(clippy::too_many_arguments)]
    pub fn restore_with_stacks(
        project: Project,
        log: OpLog,
        rev: u64,
        undo_stack: Vec<String>,
        redo_stack: Vec<String>,
        file_states: std::collections::BTreeMap<String, Value>,
    ) -> Result<Self, Vec<String>> {
        project.to_validated_value()?;
        Ok(Self { project, log, rev, undo_stack, redo_stack, file_states, dirty_files: std::collections::BTreeSet::new() })
    }

    pub fn rev(&self) -> u64 {
        self.rev
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn oplog(&self) -> &OpLog {
        &self.log
    }

    /// 某真相源文件的当前内存态(撤销/重做路由与 IO 落盘的依据)。
    pub fn file_state(&self, file: &str) -> Option<&Value> {
        self.file_states.get(file)
    }

    /// 全部文件态快照(打开/合并时传递给新 Engine)。
    pub fn file_states(&self) -> &std::collections::BTreeMap<String, Value> {
        &self.file_states
    }

    /// 取走并清空脏文件清单(先文件后记账:IO 层先落盘这些文件再追加 oplog)。
    pub fn take_dirty_files(&mut self) -> Vec<String> {
        let files: Vec<String> = self.dirty_files.iter().cloned().collect();
        self.dirty_files.clear();
        files
    }
}

/// 在 Project 上按 JSON Pointer 路径设值(undo/redo/replay 的通用机制)。
fn apply_value_at(project: &mut Project, pointer: &str, value: Value) -> Result<(), String> {
    let mut v = serde_json::to_value(&*project).map_err(|e| e.to_string())?;
    let segs: Vec<&str> = pointer.trim_start_matches('/').split('/').filter(|s| !s.is_empty()).collect();
    let mut payload = Some(value);
    let mut cur: &mut serde_json::Value = &mut v;
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        if last {
            match cur {
                serde_json::Value::Object(m) => {
                    m.insert((*seg).to_string(), payload.take().unwrap_or(serde_json::Value::Null));
                }
                serde_json::Value::Array(a) => {
                    let idx: usize = seg.parse().map_err(|_| format!("指针段 '{seg}' 非数组下标"))?;
                    if idx >= a.len() {
                        return Err(format!("指针 '{pointer}' 越界({idx} ≥ {})", a.len()));
                    }
                    a[idx] = payload.take().unwrap_or(serde_json::Value::Null);
                }
                _ => return Err(format!("指针 '{pointer}' 终点不是容器")),
            }
        } else {
            cur = match cur {
                serde_json::Value::Object(m) => {
                    m.get_mut(*seg).ok_or_else(|| format!("指针 '{pointer}' 缺键 '{seg}'"))?
                }
                serde_json::Value::Array(a) => {
                    let idx: usize = seg.parse().map_err(|_| format!("指针段 '{seg}' 非数组下标"))?;
                    if idx >= a.len() {
                        return Err(format!("指针 '{pointer}' 越界({idx} ≥ {})", a.len()));
                    }
                    &mut a[idx]
                }
                _ => return Err(format!("指针 '{pointer}' 中段 '{seg}' 处不是容器")),
            };
        }
    }
    *project = serde_json::from_value(v).map_err(|e| format!("回放反序列化失败: {e}"))?;
    Ok(())
}

/// 便捷:构造测试样本工程。
pub fn sample_project() -> Project {
    let v = json!({
        "version": 1, "schemaVersion": "2.0.0",
        "slug": "engine-样本", "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "tracks": [
            {"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 8400,
                 "sourceInMs": 12000, "role": "voice", "volume": 1.0},
                {"id": "V1-002", "src": "a.mp4", "startMs": 8400, "durationMs": 6200}
            ]},
            {"id": "A1", "kind": "audio", "clips": [
                {"id": "A1-001", "src": "sfx.mp3", "startMs": 8400, "durationMs": 400,
                 "role": "sfx", "volume": 0.8}
            ]}
        ]
    });
    Project::from_value(&v).expect("样本工程必须合法")
}

/// 命令便捷构造(测试用)。
pub fn patch(duration_ms: u64) -> ClipPatch {
    ClipPatch { duration_ms: Some(duration_ms), ..Default::default() }
}
