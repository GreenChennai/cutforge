// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! cutforge-mcp 二进制入口:inspect(契约导出)/ serve-stdio(主通道)/
//! serve-http(辅通道,仅 127.0.0.1 + token)/ run-script(脚本宿主接线)。

use cutforge_mcp::{serve_http, serve_stdio, serve_workspace};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn serve_http_root(root: &Path, port: u16, token: &str, web: &Path) -> i32 {
    serve_workspace(root, port, token, web)
}

/// 随机 token(pid+纳秒时钟 hash;无第三方依赖纪律)。
fn new_token() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::process::id().hash(&mut h);
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos().hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Web 资源目录:env CUTFORGE_WEB → cargo 布局(crate 同级 apps/web)。
fn default_web_dir() -> PathBuf {
    if let Some(v) = std::env::var_os("CUTFORGE_WEB") {
        return PathBuf::from(v);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/web")
}

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
            // M10 本地服务化:cutforge-mcp serve --root <工程> [--port 8787] [--token T] [--web D]
            let Some(root) = args.iter().position(|a| a == "--root").and_then(|i| args.get(i + 1)) else {
                eprintln!("用法: serve --root <工程目录> [--port N] [--token T] [--web 目录]");
                std::process::exit(2);
            };
            let port: u16 = args.iter().position(|a| a == "--port")
                .and_then(|i| args.get(i + 1).and_then(|v| v.parse().ok()))
                .unwrap_or(8787);
            let token = args.iter().position(|a| a == "--token")
                .and_then(|i| args.get(i + 1).cloned())
                .unwrap_or_else(new_token);
            let web = args.iter().position(|a| a == "--web")
                .and_then(|i| args.get(i + 1).map(PathBuf::from))
                .unwrap_or_else(default_web_dir);
            serve_http_root(Path::new(root), port, &token, &web)
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
            eprintln!("用法: cutforge-mcp <inspect|serve --root R [--port N --token T --web D]|serve-stdio|serve-http --port N --token T|run-script --root R --file F>");
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
