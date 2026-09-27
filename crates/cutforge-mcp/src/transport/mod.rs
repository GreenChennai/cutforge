// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 传输通道:stdio(主)与内嵌 HTTP(辅),共用 dispatch 的 JSON-RPC 面。

pub(crate) mod http;
pub(crate) mod stdio;
