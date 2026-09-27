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
}
