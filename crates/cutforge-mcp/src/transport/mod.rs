// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 传输通道:stdio(主)与内嵌 HTTP(辅),共用 dispatch 的 JSON-RPC 面。
//! T1.6 起 HTTP 侧分五件:http(请求读取/连接纪律 + 辅通道)、static_files(静态面)、
//! events(SSE 事件面)、rest(/api/v1 版本化 REST 面,册七 T7.1)、stdio(stdio 主通道)。

pub(crate) mod events;
pub(crate) mod http;
pub(crate) mod rest;
pub(crate) mod static_files;
pub(crate) mod stdio;

/// 请求读取的两种失败(两通道共用):连接不完整/对端已走 → 直接回收不回写;
/// 头或体超上限 → 回 413 再关。
pub(crate) enum ReadFail {
    Closed,
    TooLarge,
}
