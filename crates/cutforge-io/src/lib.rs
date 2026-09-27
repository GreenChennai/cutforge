// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! CutForge IO 层:Workspace 把"文件真相源"(计划书 4.2)与内核命令通道接在一起。
//!
//! 唯一写入路径(4.2 八步;编排者为 `workspace::apply`,所有落盘经 `atomic::atomic_write`):
//! 1. 申请工程锁(`open_exclusive` 可让锁覆盖 open→apply→persist 全程,P0-5)→
//! 2/3. Engine 校验前置与不变量 → 4. 新内容先落盘(先文件后记账)→
//! 5. Op 追加 oplog → 6. rev 落盘 → 7. 释放锁。
//!
//! 八步已编译成显式步骤函数(每步一个具名函数,主流程依次调用):
//! 步骤函数表见 `workspace/apply.rs`,磁盘侧各落盘步骤见 `workspace/persist.rs`。
//!
//! `_meta` 旁路(ADR-0002):CutFlow 真实 IR 顶层带 `_meta`(实现细节字段,被
//! v2 schema `additionalProperties:false` 拒收)。open 时剥出入内存旁路,
//! persist 时原样回写——CutForge 打得开任何 CutFlow 工程,写回不丢它。
//!
//! `.cutforge/` 可安全删除重建(仅丢同步历史)——`workspace_state_rebuildable` 测试保证。
//!
//! 模块布局(T1.2,原单文件 lib.rs 纯移动拆分;导出面不变):
//! `workspace/{open,query,apply,sync,notes,persist,bases,conflicts}.rs`
//! (既有子模块 paths/atomic/lock/backup/probe/watcher/scaffold/stage/fsutil 保持)。

pub mod atomic;
pub mod backup;
pub mod fsutil;
pub mod lock;
pub mod paths;
pub mod probe;
pub mod scaffold;
pub mod stage;
pub mod watcher;

mod workspace;

// 目录契约唯一真相源在 `paths`(0.5.0 目录中文化,与 CutFlow rs_paths.py 同构);
// 这里原样再导出,老调用面(API 兼容)不破。
pub use paths::{CUTLIST_APPLIED_REL, CUTLIST_REL, NOTES_REL, PROJECT_REL, WORDLINE_REL};

// Workspace 主类型与配套 API:实码在 workspace/ 子模块,此处只组导出面
// (旧路径 `cutforge_io::Workspace`/`BASES_REL`/`tests_fixture` 保持原样)。
pub use workspace::Workspace;
pub use workspace::bases::BASES_REL;
pub use workspace::open::tests_fixture;

// 非工程真相源文件与其在工程目录内的相对路径:唯一登记处在 `paths`
// (FILE_TRUTHS_NEW / FILE_TRUTHS_LEGACY,按盘面布局择一)。
