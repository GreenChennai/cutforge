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
mod batch;
mod invariants;
mod pro_cmds;
mod projection;
mod replay;
mod revert;
mod undo;
mod validate;

pub use apply::{ApplyOpts, OpReceipt};
pub use invariants::Reject;
pub use projection::{Answer, Query, canonical_json};
pub use replay::{rebuild_stacks, rebuild_stacks_incremental, rebuild_undo_stack};

use crate::command::ClipPatch;
use crate::model::Project;
use crate::oplog::OpLog;
use serde_json::{Value, json};

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
            project,
            log: OpLog::new(),
            rev: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
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
            project,
            log,
            rev,
            undo_stack,
            redo_stack: Vec::new(),
            file_states,
            dirty_files: std::collections::BTreeSet::new(),
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
        Ok(Self {
            project,
            log,
            rev,
            undo_stack,
            redo_stack,
            file_states,
            dirty_files: std::collections::BTreeSet::new(),
        })
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

    /// doctor 入口(R-12②):全量契约校验(to_validated_value 同口径)。
    /// 发布版 apply 走增量校验后,全量校验保留在装载面(Engine::new/restore/
    /// replay 首帧、undo/redo)与本显式入口;cli/mcp doctor 接线由对应层完成。
    pub fn validate_full(&self) -> Result<(), Vec<String>> {
        self.project.to_validated_value().map(|_| ())
    }
}

/// 在 Project 上按 JSON Pointer 路径设值(undo/redo/replay 的通用机制)。
fn apply_value_at(project: &mut Project, pointer: &str, value: Value) -> Result<(), String> {
    let mut v = serde_json::to_value(&*project).map_err(|e| e.to_string())?;
    let segs: Vec<&str> = pointer
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let mut payload = Some(value);
    let mut cur: &mut serde_json::Value = &mut v;
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        if last {
            match cur {
                serde_json::Value::Object(m) => {
                    m.insert(
                        (*seg).to_string(),
                        payload.take().unwrap_or(serde_json::Value::Null),
                    );
                }
                serde_json::Value::Array(a) => {
                    let idx: usize = seg
                        .parse()
                        .map_err(|_| format!("指针段 '{seg}' 非数组下标"))?;
                    if idx >= a.len() {
                        return Err(format!("指针 '{pointer}' 越界({idx} ≥ {})", a.len()));
                    }
                    a[idx] = payload.take().unwrap_or(serde_json::Value::Null);
                }
                _ => return Err(format!("指针 '{pointer}' 终点不是容器")),
            }
        } else {
            cur = match cur {
                serde_json::Value::Object(m) => m
                    .get_mut(*seg)
                    .ok_or_else(|| format!("指针 '{pointer}' 缺键 '{seg}'"))?,
                serde_json::Value::Array(a) => {
                    let idx: usize = seg
                        .parse()
                        .map_err(|_| format!("指针段 '{seg}' 非数组下标"))?;
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

/// 按 JSON Pointer 读 Project 当前值(undo/redo 前置校验用)。
fn project_value_at(project: &Project, pointer: &str) -> Result<Value, String> {
    let v = serde_json::to_value(project).map_err(|e| e.to_string())?;
    let mut cur = &v;
    for seg in pointer
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
    {
        cur = match cur {
            // 缺键视同 Null:与 before/after 对缺席字段的 Null 记法一致
            // (如撤销 bgm_clear 时当前态无 /bgm 键,期望值恰为 Null)
            serde_json::Value::Object(m) => match m.get(seg) {
                Some(v) => v,
                None => return Ok(serde_json::Value::Null),
            },
            serde_json::Value::Array(a) => {
                let idx: usize = seg
                    .parse()
                    .map_err(|_| format!("指针段 '{seg}' 非数组下标"))?;
                a.get(idx)
                    .ok_or_else(|| format!("指针 '{pointer}' 越界({idx} ≥ {})", a.len()))?
            }
            _ => return Err(format!("指针 '{pointer}' 中段 '{seg}' 处不是容器")),
        };
    }
    Ok(cur.clone())
}

/// Op 的 project.json 回写(undo/redo/replay 共用;BUG-06/A-03 稳定 id 寻址):
/// - `op.target_id = Some(id)`(新格式):按 clip id 定位——结构漂移(数组重排等)
///   免疫;下标只在 Op 记录瞬间用一次;
/// - `op.target_id = None`(旧格式):JSON Pointer 下标寻址(旧日志零迁移);
/// - `expect = Some(want)`:前置校验当前态 == want(undo 撤前校 after、redo 校
///   before),不一致返回含 **CF-002** 的错误(拒绝盲写);replay 传 None。
pub(super) fn write_project_op(
    project: &mut Project,
    op: &crate::oplog::Op,
    value: Value,
    expect: Option<&Value>,
) -> Result<(), String> {
    if let Some(id) = op.target_id.as_deref() {
        let (ti, ci) = project
            .find_clip(id)
            .ok_or_else(|| format!("CF-002 目标 clip {id} 不存在(已被删除或 id 变更),拒绝盲写"))?;
        let ptr = format!("/tracks/{ti}/clips/{ci}");
        if let Some(want) = expect {
            let cur = project_value_at(project, &ptr)?;
            if cur != *want {
                return Err(format!(
                    "CF-002 撤销/重做基准不一致:clip {id} 当前态已偏离预期(外部改动或非 LIFO 历史),拒绝盲写"
                ));
            }
        }
        apply_value_at(project, &ptr, value)?;
        reorder_clip_at(project, ti, ci);
        return Ok(());
    }
    if let Some(want) = expect {
        let cur = project_value_at(project, &op.target.path)?;
        if cur != *want {
            return Err(format!(
                "CF-002 撤销/重做基准不一致:{} 当前态已偏离待回写 Op 的预期(外部改动或重放),拒绝盲写",
                op.target.path
            ));
        }
    }
    let ptr = op.target.path.clone();
    let clip_obj = is_clip_pointer(&ptr);
    apply_value_at(project, &ptr, value)?;
    // 旧格式 clip 对象指针写回后同样归位(历史跨位 start 更新在重放时补齐数组序;
    // 值不变,只定序——与 mutate 内重插同规则)
    if clip_obj && let Some((ti, ci)) = parse_clip_pointer(&ptr) {
        reorder_clip_at(project, ti, ci);
    }
    Ok(())
}

/// `/tracks/{ti}/clips/{ci}` 形判(clip 对象指针;write_project_op 归位重排判据)。
fn is_clip_pointer(path: &str) -> bool {
    parse_clip_pointer(path).is_some()
}

fn parse_clip_pointer(path: &str) -> Option<(usize, usize)> {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.len() != 4 || segs[0] != "tracks" || segs[2] != "clips" {
        return None;
    }
    Some((segs[1].parse().ok()?, segs[3].parse().ok()?))
}

/// 把 (ti, ci) 处的 clip 按 start_ms 归位(remove + 二分重插,与 mutate 内
/// 跨位更新的重插同规则)。撤销/重做/回放写回跨位历史对象后恢复轨道升序。
/// 同轨无同 start 兄弟(enforce_no_overlap),插入位置无歧义。
fn reorder_clip_at(project: &mut Project, ti: usize, ci: usize) {
    let clip = project.tracks[ti].clips[ci].clone();
    project.tracks[ti].clips.remove(ci);
    let pos = project.tracks[ti]
        .clips
        .partition_point(|c| c.start_ms < clip.start_ms);
    project.tracks[ti].clips.insert(pos, clip);
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
    ClipPatch {
        duration_ms: Some(duration_ms),
        ..Default::default()
    }
}
