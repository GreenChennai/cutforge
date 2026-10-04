//! Workspace 只读访问器:根路径 / 内核引擎 / 工程视图 / rev。

use std::path::Path;

use cutforge_core::engine::Engine;
use cutforge_core::model::Project;

use super::Workspace;

impl Workspace {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// 当前工程(只读视图)。
    pub fn project(&self) -> &Project {
        self.engine.project()
    }

    pub fn rev(&self) -> u64 {
        self.engine.rev()
    }

    /// 打开装载期的修复报告(R-03 半行截断 / R-04 差异自愈)。
    /// Some = 本次打开发生过修复动作(UI/MCP 必须呈现,绝不静默);
    /// 报告同样落在 `.cutforge/repair-report.json`。
    pub fn repair_report(&self) -> Option<&crate::repair::RepairReport> {
        self.repair.as_ref()
    }
}
