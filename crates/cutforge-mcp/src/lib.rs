// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! CutForge MCP 层(计划书 5.1–5.4)。
//!
//! **单注册表双通道**:stdio(主)与内嵌 HTTP(辅,仅 127.0.0.1 + token)共用同一
//! `dispatch`,工具集差异恒为 0(M4-1)。所有返回值都是
//! `{ok, code, message, data}` 结果协议;code 取值限于计划书 5.4 表。
//! T1.7:envelope 另派生加法字段 `ns`(命名空间 io/core/mcp/render,单一真相源
//! registry::CODE_NS;CLI/MCP/HTTP 三面同源取值),既有 code 取值逐字不变。
//! 编排类工具只封装 CutFlow 既有脚本(子进程透传),不实现任何阶段逻辑。
//!
//! T1.1(册一):原单文件 lib.rs 拆分为模块目录(纯移动重构,导出面与行为零变化):
//! - `registry`:契约常量(mcp-tools/capability-matrix/ui-fields 编译期嵌入)+ 注册表
//!   + 结果协议 envelope + 5.4 错误码表;
//! - `dispatch`:单一分发表(所有通道共用)+ JSON-RPC 面 + 参数解析与错误映射;
//! - `tools_nolock`:免开工作区工具(project_new/media_probe/media_browse/render_probe/stage_status);
//! - `orchestrate`:CutFlow 脚本编排(py_launcher/orchestrate);
//! - `progress`:cutforge-render 后端同步/异步渲染与进度轮询;
//! - `session`:new_token 与 RT-1 会话变更摘要(.cutforge/session-summary.json);
//! - `workspace_svc`:工作区常驻服务(serve 启动自检 + /rpc + /media + /events + /ui-fields);
//! - `transport::{stdio,http,static_files,events}`:双通道传输 + 静态目录面 + SSE 事件面(T1.6);

#[cfg(test)]
mod tests;

mod dispatch;
mod edit_ops;
mod grade_tools;
mod media_tools;
mod pro_ops;
mod subtitle_ops;
mod orchestrate;
mod progress;
mod registry;
mod rpc;
mod resident;
mod session;
mod tools_nolock;
mod transport;
mod workspace_svc;

// ---- 公开 API:路径与拆分前完全一致(main.rs、cutforge-cli、tests/ 零改动) ----
pub use dispatch::{dispatch, dispatch_with_actor, handle_rpc, handle_rpc_as};
pub use registry::{code_namespace, registry, tool_names, CODE_NS, CAPABILITY_MATRIX_JSON, CODES, MCP_TOOLS_JSON, UI_FIELDS_JSON};
pub use session::new_token;
pub use transport::http::serve_http;
pub use transport::stdio::serve_stdio;
pub use workspace_svc::{default_web_dir, pick_project_interactive, serve_workspace};

// ---- crate 内部胶水:仅供 #[cfg(test)] 的 tests.rs 经 `use super::*` 取用 ----
#[cfg(test)]
use dispatch::produces_rev_mutation;
#[cfg(test)]
use orchestrate::{py_launcher, script_arg_to_string};
#[cfg(test)]
use registry::capability_matrix;
