//! CutForge 内核(ARL-CORE,计划书 2.1/2.6/ADR-0033)。
//!
//! 硬边界:不碰文件系统(无 std::fs)、不调 ffmpeg、不认识剪映;
//! 读与写两套接口严格分离——`Engine::query`(纯投影,可并发)与
//! `Engine::apply`(唯一写入口,产生 Op、可撤销、可审计)。
//! 内核可在 wasm32 构建(M2-5 门禁)。

pub mod anchor;
pub mod command;
pub mod engine;
pub mod merge;
pub mod model;
pub mod oplog;
pub mod timeutil;

pub use command::Command;
pub use engine::{Answer, ApplyOpts, Engine, OpReceipt, Query, Reject};
pub use model::{Backend, Canvas, Clip, ClipKind, Motion, Project, Ratio, Role, Track, TrackKind};
pub use oplog::{Actor, ActorKind, Op, OpLog};

/// 生成 opId:op-<序号>(全工程唯一,单调分配)。
pub fn format_op_id(n: u64) -> String {
    format!("op-{n}")
}

/// 生成 rev 标识:rev-<n>(单调递增)。
pub fn format_rev(n: u64) -> String {
    format!("rev-{n}")
}
