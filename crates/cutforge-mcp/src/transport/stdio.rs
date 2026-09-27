// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! stdio 主通道:逐行读 JSON-RPC,逐行写响应(T1.1 拆分自 lib.rs,纯移动)。

use crate::dispatch::handle_rpc;
use serde_json::{json, Value};
use std::io::Write as _;

/// stdio 主通道:逐行读 JSON-RPC,逐行写响应。
pub fn serve_stdio() -> i32 {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in std::io::BufRead::lines(stdin.lock()) {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Value>(&line) {
            Ok(req) => handle_rpc(&req).map(|r| r.to_string()).unwrap_or_default(),
            Err(e) => json!({
                "jsonrpc": "2.0", "id": null,
                "error": {"code": -32700, "message": format!("parse error: {e}")}
            }).to_string(),
        };
        if !resp.is_empty() {
            let _ = writeln!(stdout, "{resp}");
            let _ = stdout.flush();
        }
    }
    0
}
