// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! cutforge-mcp 二进制入口:inspect(契约导出)/ serve-stdio(主通道)/
//! serve-http(辅通道,仅 127.0.0.1 + token)/ serve(编辑器,E1)/ run-script(脚本宿主接线)。

use cutforge_mcp::{default_web_dir, new_token, serve_http, serve_stdio, serve_workspace};
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
        "serve" => {
            // M10/E1/E6-1:cutforge-mcp serve [--root <工程>] [--port 8787] [--token T] [--web D] [--open]
            // 无 --root 时与 cli serve 同一交互列工程行为(单一实现:pick_project_interactive);
            // 非交互环境必须显式给目录。
            use std::io::IsTerminal as _;
            let root = args.iter().position(|a| a == "--root").and_then(|i| args.get(i + 1));
            let root = match root {
                Some(r) => std::path::PathBuf::from(r),
                None => {
                    if !std::io::stdin().is_terminal() {
                        eprintln!("用法: serve [--root <工程目录>] [--port N] [--token T] [--web 目录] [--open];非交互环境必须给工程目录");
                        std::process::exit(2);
                    }
                    match cutforge_mcp::pick_project_interactive() {
                        Some(p) => p,
                        None => {
                            eprintln!("未找到候选工程(查找:CUTFORGE_PROJECTS 或当前目录下两层内的 05_ir/project.json);或先新建:cutforge-cli new <目录>");
                            std::process::exit(3);
                        }
                    }
                }
            };
            let port: u16 = args.iter().position(|a| a == "--port")
                .and_then(|i| args.get(i + 1).and_then(|v| v.parse().ok()))
                .unwrap_or(8787);
            let token = args.iter().position(|a| a == "--token")
                .and_then(|i| args.get(i + 1).cloned())
                .unwrap_or_else(new_token);
            let web = args.iter().position(|a| a == "--web")
                .and_then(|i| args.get(i + 1).map(std::path::PathBuf::from))
                .unwrap_or_else(default_web_dir);
            let open = args.iter().any(|a| a == "--open");
            serve_workspace(&root, port, &token, &web, open)
        }
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
            eprintln!("用法: cutforge-mcp <inspect|serve [--root R --port N --token T --web D]|serve-stdio|serve-http --port N --token T|run-script --root R --file F>");
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
