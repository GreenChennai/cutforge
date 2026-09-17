//! cutforge-mcp 二进制入口:inspect(契约导出)/ serve-stdio(主通道)/
//! serve-http(辅通道,仅 127.0.0.1 + token)/ run-script(脚本宿主接线)。

use cutforge_mcp::{serve_http, serve_stdio};
use serde_json::json;
use std::path::Path;
use std::sync::OnceLock;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let code = match cmd {
        "inspect" => {
            let doc = json!({
                "ok": true,
                "code": "OK",
                "message": "MCP 工具注册表",
                "data": {
                    "tools": cutforge_mcp::registry(),
                    "channels": {
                        "stdio": cutforge_mcp::tool_names(),
                        "embedded-http": cutforge_mcp::tool_names(),
                    }
                }
            });
            println!("{doc}");
            0
        }
        "serve-stdio" => serve_stdio(),
        "serve-http" => {
            let port: u16 = args
                .iter()
                .position(|a| a == "--port")
                .and_then(|i| args.get(i + 1).and_then(|v| v.parse().ok()))
                .unwrap_or(8787);
            let token = args
                .iter()
                .position(|a| a == "--token")
                .and_then(|i| args.get(i + 1).cloned())
                .unwrap_or_else(|| format!("dev-{}", std::process::id()));
            serve_http(port, &token)
        }
        "run-script" => {
            let Some(root) = args.iter().position(|a| a == "--root").and_then(|i| args.get(i + 1)) else {
                eprintln!("用法: run-script --root <工程> --file <script.json>");
                std::process::exit(2);
            };
            let Some(file) = args.iter().position(|a| a == "--file").and_then(|i| args.get(i + 1)) else {
                eprintln!("用法: run-script --root <工程> --file <script.json>");
                std::process::exit(2);
            };
            let Ok(text) = std::fs::read_to_string(file) else {
                eprintln!("脚本不存在: {file}");
                std::process::exit(3);
            };
            let Ok(script) = serde_json::from_str::<serde_json::Value>(&text) else {
                eprintln!("脚本非法 JSON");
                std::process::exit(2);
            };
            let policy = cutforge_script::Policy::new(Path::new(root));
            let mut host = ScriptHost { root: root.clone() };
            match cutforge_script::run_script(&script, &mut host, &policy) {
                Ok(report) => {
                    println!(
                        "{}",
                        json!({
                            "ok": report.escapes() == 0,
                            "code": if report.escapes() == 0 { "OK" } else { "GUARD_FAILED" },
                            "message": format!("执行 {} 步,拒绝 {} 次逃逸", report.dispatched, report.escapes()),
                            "data": report,
                        })
                    );
                    if report.escapes() == 0 { 0 } else { 2 }
                }
                Err(e) => {
                    println!("{}", json!({"ok": false, "code": "SCHEMA_INVALID", "message": e.to_string(), "data": {}}));
                    2
                }
            }
        }
        _ => {
            eprintln!("用法: cutforge-mcp <inspect|serve-stdio|serve-http --port N --token T|run-script --root R --file F>");
            2
        }
    };
    std::process::exit(code);
}

/// 允许清单 = 注册表全部工具名;编排类工具由沙箱策略另行默认禁用。
static ALLOWED: OnceLock<Vec<String>> = OnceLock::new();

/// 把 MCP 注册表接到脚本宿主的派发面;脚本 args 未带 root 时注入宿主工程根。
struct ScriptHost {
    root: String,
}

impl cutforge_script::ToolDispatch for ScriptHost {
    fn allowed_tools(&self) -> &[String] {
        ALLOWED.get_or_init(cutforge_mcp::tool_names)
    }
    fn call(&mut self, tool: &str, args: &serde_json::Value) -> Result<serde_json::Value, String> {
        let mut args = args.clone();
        if args.get("root").is_none() {
            args["root"] = serde_json::Value::String(self.root.clone());
        }
        Ok(cutforge_mcp::dispatch(tool, &args))
    }
}
