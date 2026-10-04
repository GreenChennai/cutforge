// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! `cutforge-cli doctor`(T1.7):工程环境诊断面。
//!
//! 每项检查失败时给**可复制执行的修复命令**(fix 字段;人类可读面以 `修复:` 前缀输出)。
//! 检查项对齐 serve 启动自检(workspace_svc::serve_preflight)的既有口径并按 T1.7 扩展:
//! project / ffmpeg / ffprobe(既有面)+ web 资源完整性 / 缓存目录可写 / 端口占用(T1.7 新增)。
//! 输出走 crate::emit 结果协议(三面同码,ns 由 cutforge_mcp 派生);判定器风格
//! 对齐 check-shell-purity 等:全绿 → OK,任一失败 → DOCTOR_FAILED(退出码 2)。
//!
//! 写路径纪律:可写性探测只经 cutforge_io::atomic(唯一落盘点,check-write-paths 口径),
//! 目录创建用 create_dir_all(不在旁路写入扫描面内)。

use crate::{Args, emit};
use serde_json::json;
use std::path::{Path, PathBuf};

/// serve 默认端口(与 lib.rs serve_cmd、编辑器壳口径一致)。
const DEFAULT_PORT: u16 = 8787;
/// web 资源完整性基线(缺一即静态面不完整;与 apps/web 现状一致)。
const WEB_FILES: [&str; 3] = ["index.html", "app.js", "style.css"];
/// 端口探测的换端口搜索窗(serve_cmd 同款:+1..+20)。
const PORT_WINDOW: u16 = 20;

/// 一项检查的结论:`fix` 非空 = 失败且给出可复制执行的修复命令。
struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
    fix: String,
}

impl Check {
    fn pass(name: &'static str, detail: String) -> Check {
        Check {
            name,
            ok: true,
            detail,
            fix: String::new(),
        }
    }
    fn fail(name: &'static str, detail: String, fix: String) -> Check {
        Check {
            name,
            ok: false,
            detail,
            fix,
        }
    }
}

/// ① 工程可识别(目录契约 0.5:05_时间线工程/project.json,兼容旧 05_ir/)。
fn check_project(root: &Path) -> Check {
    if cutforge_io::paths::has_project(root) {
        return Check::pass(
            "project",
            format!(
                "工程可识别: {}",
                cutforge_io::paths::project_path(root).display()
            ),
        );
    }
    Check::fail(
        "project",
        format!(
            "不是 CutForge 工程(缺 05_时间线工程/project.json,兼容旧 05_ir/): {}",
            root.display()
        ),
        format!("cutforge-cli new \"{}\"", root.display()),
    )
}

/// 外部进程是否可用(以 `-version` 探测;对齐 serve_preflight::bin_on_path 口径)。
fn bin_available(bin: &str, env_key: &str) -> bool {
    let via_env = std::env::var_os(env_key).is_some_and(|v| !v.is_empty());
    if via_env {
        return true;
    }
    std::process::Command::new(bin)
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// ② ffmpeg / ffprobe 可用(渲染与素材探测的外部依赖;CUTFORGE_FFMPEG/CUTFORGE_FFPROBE 优先)。
/// ADR-0022:解析顺序 = env 显式 → 系统 PATH → 内嵌随包(安装器勾选组件,默认勾;
/// 命中方式 = 安装器写 env,运行时探测面零新代码)。缺失给三选一修复指引。
fn check_bin(name: &'static str, bin: &str, env_key: &str) -> Check {
    if bin_available(bin, env_key) {
        let via_env = std::env::var_os(env_key).is_some_and(|v| !v.is_empty());
        return Check::pass(
            name,
            if via_env {
                format!("env {env_key}")
            } else {
                "PATH 可见".into()
            },
        );
    }
    Check::fail(
        name,
        format!("{bin} 不可用(素材时长探测/渲染导出依赖它;ADR-0022 解析顺序 env→PATH→内嵌随包)"),
        format!(
            "三选一:① 重跑安装器勾选「ffmpeg 内嵌组件」(默认勾选,纯净机装完即用);\
             ② winget install --id Gyan.FFmpeg -e ;③ 安装后 setx {env_key} \"C:\\path\\to\\{bin}.exe\""
        ),
    )
}

/// ③(T1.7 新增)web 资源完整性:web 目录下 index.html/app.js/style.css 缺一不可。
fn check_web(root: &Path, web_dir: &Path) -> Check {
    let missing: Vec<&str> = WEB_FILES
        .iter()
        .filter(|f| !web_dir.join(f).is_file())
        .copied()
        .collect();
    if missing.is_empty() {
        return Check::pass("web", format!("web 资源齐备: {}", web_dir.display()));
    }
    // 修复命令给具体路径:仓库内 apps/web 存在则直接指向它,否则教设 CUTFORGE_WEB
    let repo_web = crate::repo_root().join("apps/web");
    let fix = if repo_web.join("index.html").is_file() {
        format!(
            "cutforge-cli serve \"{}\" --web \"{}\"",
            root.display(),
            repo_web.display()
        )
    } else {
        format!("set CUTFORGE_WEB=\"<含 {} 的目录>\"", WEB_FILES.join("/"))
    };
    Check::fail(
        "web",
        format!(
            "web 资源缺失({}): {}",
            missing.join(", "),
            web_dir.display()
        ),
        fix,
    )
}

/// ④(T1.7 新增)缓存目录可写:`.cutforge/render-cache` 父目录可写性。
/// 探测 = 建目录 + 经 atomic.rs 写删探针文件(唯一落盘点纪律;真实落盘,不做只读位猜测)。
fn check_cache_writable(root: &Path) -> Check {
    if !root.is_dir() {
        return Check::fail(
            "cache",
            "工程目录不存在,缓存目录无从谈起".into(),
            check_project(root).fix,
        );
    }
    let cache_dir = root.join(".cutforge/render-cache");
    if let Err(e) = std::fs::create_dir_all(&cache_dir) {
        return Check::fail(
            "cache",
            format!("缓存目录不可建({}): {e}", cache_dir.display()),
            format!(
                "icacls \"{}\" /grant \"%USERNAME%\":F",
                root.join(".cutforge").display()
            ),
        );
    }
    let probe = cache_dir.join(".doctor-probe");
    let write = cutforge_io::atomic::atomic_write(&probe, b"ok")
        .map_err(|e| e.to_string())
        .and_then(|()| cutforge_io::atomic::remove(&probe).map_err(|e| e.to_string()));
    match write {
        Ok(()) => Check::pass("cache", format!("缓存目录可写: {}", cache_dir.display())),
        Err(e) => Check::fail(
            "cache",
            format!("缓存目录不可写({}): {e}", cache_dir.display()),
            format!(
                "attrib -r \"{}\" ;或 icacls \"{}\" /grant \"%USERNAME%\":F",
                cache_dir.display(),
                cache_dir.display()
            ),
        ),
    }
}

/// 端口是否可绑定(探测即放手;竞态由 serve 启动自检兜底,与 serve_cmd 同口径)。
fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// serve 将实际采用的端口探测窗内的首个空闲端口(全占则 None)。
fn first_free_port(from: u16) -> Option<u16> {
    (from..from.saturating_add(PORT_WINDOW)).find(|p| port_free(*p))
}

/// ⑤(T1.7 新增)端口占用:serve 端口被占时给排查命令与具体换端口建议。
fn check_port(root: &Path, port: u16) -> Check {
    if port_free(port) {
        return Check::pass("port", format!("端口空闲: {port}"));
    }
    let alt = first_free_port(port.saturating_add(1))
        .map(|p| format!("cutforge-cli serve \"{}\" --port {p}", root.display()))
        .unwrap_or_else(|| "换个网段内端口: cutforge-cli serve <工程目录> --port <端口>".into());
    Check::fail(
        "port",
        format!("端口 {port} 已被占用(可能是此前的 serve 未退)"),
        format!("netstat -ano | findstr :{port}  →  taskkill /PID <占用进程PID> /F ;或 {alt}"),
    )
}

/// --bundle 诊断包(册六 T6.4):单文件 zip = doctor.json + environment.txt +
/// 会话/摘要/rev 快照(在位才收,单件 512KB 上限;工程真相源 project.json 一并快照——
/// 「导出失败」类问题的最小取证面)。返回 (落点, 字节数, 条目数)。
fn write_bundle(
    root: &Path,
    data: &serde_json::Value,
    out: Option<&String>,
) -> Result<(PathBuf, usize, usize), String> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let out_path = match out.filter(|s| !s.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => root.join(format!(".cutforge/doctor-bundle-{secs}.zip")),
    };
    let doctor_doc =
        json!({"generatedAt": cutforge_core::timeutil::now_rfc3339(), "diagnosis": data});
    let mut entries = vec![
        crate::bundle::Entry {
            name: "doctor.json".into(),
            data: serde_json::to_vec_pretty(&doctor_doc).map_err(|e| e.to_string())?,
        },
        crate::bundle::Entry {
            name: "environment.txt".into(),
            data: crate::bundle::environment_text(root).into_bytes(),
        },
    ];
    // 日志与快照面:在位才收(诊断不虚构);单件上限 512KB(oplog 尾巴足够定位问题)
    const CAP: u64 = 512 * 1024;
    for (name, path) in [
        ("session.json", root.join(".cutforge/session")),
        (
            "session-summary.json",
            root.join(".cutforge/session-summary.json"),
        ),
        ("rev.txt", root.join(".cutforge/rev")),
        ("project.json", cutforge_io::paths::project_path(root)),
    ] {
        if let Some(bytes) = read_capped(&path, CAP) {
            entries.push(crate::bundle::Entry {
                name: name.into(),
                data: bytes,
            });
        }
    }
    let n = entries.len();
    let bytes = crate::bundle::write_bundle(&out_path, entries)?;
    Ok((out_path, bytes, n))
}

/// 读文件前 cap 字节(诊断包收集面;不存在/不可读 → None,不虚构)。
fn read_capped(path: &Path, cap: u64) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let f = std::fs::File::open(path).ok()?;
    let take = f.metadata().ok()?.len().min(cap) as usize;
    let mut buf = Vec::with_capacity(take);
    f.take(cap).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// 入口。用法:`doctor <工程目录> [--port N] [--web 目录] [--bundle 路径] [--json]`。
pub fn run(a: &Args) -> i32 {
    const USAGE: &str = "用法: doctor <工程目录> [--port N] [--web 目录] [--bundle 路径] [--json]";
    let Some(root_s) = a.positional.first().cloned() else {
        return emit(a.json, false, "PRECONDITION_FAILED", USAGE, json!({}));
    };
    let root = PathBuf::from(&root_s);
    let web = a
        .flags
        .get("web")
        .map(PathBuf::from)
        .unwrap_or_else(cutforge_mcp::default_web_dir);
    let port = a
        .flags
        .get("port")
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);
    let checks = vec![
        check_project(&root),
        check_bin("ffmpeg", "ffmpeg", "CUTFORGE_FFMPEG"),
        check_bin("ffprobe", "ffprobe", "CUTFORGE_FFPROBE"),
        check_web(&root, &web),
        check_cache_writable(&root),
        check_port(&root, port),
    ];
    let passed = checks.iter().filter(|c| c.ok).count();
    // 人类可读面:逐项 ✓/✗ + 修复命令(失败项必有 fix,这是 T1.7 的硬承诺)
    if !a.json {
        for c in &checks {
            if c.ok {
                println!("✓ {}: {}", c.name, c.detail);
            } else {
                println!("✗ {}: {}\n  修复: {}", c.name, c.detail, c.fix);
            }
        }
    }
    let mut data = json!({
        "root": root.display().to_string(),
        "port": port,
        "webDir": web.display().to_string(),
        "checks": checks.iter().map(|c| json!({
            "name": c.name, "ok": c.ok, "detail": c.detail,
            "fix": if c.fix.is_empty() { serde_json::Value::Null } else { json!(c.fix) },
        })).collect::<Vec<_>>(),
        "passed": passed,
        "total": checks.len(),
    });
    // --bundle(T6.4):诊断包随任一结论落盘(诊断面即取证面;失败项也能打包带修)
    let bundle_note = match a.flags.get("bundle") {
        Some(_) => {
            match write_bundle(&root, &data, a.flags.get("bundle")) {
                Ok((path, bytes, n)) => {
                    let note = format!(";诊断包: {}({bytes} B,{n} 件)", path.display());
                    if let Some(obj) = data.as_object_mut() {
                        obj.insert("bundle".into(), json!({"path": path.display().to_string(), "bytes": bytes, "entries": n}));
                    }
                    note
                }
                Err(e) => {
                    return emit(
                        a.json,
                        false,
                        "INTERNAL",
                        &format!("诊断包失败: {e}"),
                        json!({}),
                    );
                }
            }
        }
        None => String::new(),
    };
    let tail_msg = |m: &str| -> String { format!("{m}{bundle_note}") };
    if passed == checks.len() {
        emit(
            a.json,
            true,
            "OK",
            &tail_msg(&format!("工程环境诊断:{passed}/{} 项通过", checks.len())),
            data,
        )
    } else {
        emit(
            a.json,
            false,
            "DOCTOR_FAILED",
            &tail_msg(&format!(
                "工程环境诊断:{passed}/{} 项通过(失败项见 checks[].fix,均可复制执行)",
                checks.len()
            )),
            data,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 临时工程根(cache.rs TempRoot 同款;进程 id 防并行串台)。
    struct TempRoot(PathBuf);
    impl TempRoot {
        fn new(tag: &str) -> TempRoot {
            let dir =
                std::env::temp_dir().join(format!("cf-cli-doctor-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempRoot(dir)
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// T1.7 硬承诺:任何失败项都必须带可复制执行的修复命令(非空 fix)。
    #[test]
    fn every_failed_check_carries_copyable_fix() {
        let root = TempRoot::new("fix");
        // 空目录:project/web 必失败;端口用真实占住的一个(确定性,不赌空闲);
        // cache 对空目录会建齐缓存目录而通过(诊断即搭建,与 serve 口径一致),
        // 其失败面由下方专项测试覆盖。
        let guard = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = guard.local_addr().unwrap().port();
        for c in [
            check_project(&root.0),
            check_web(&root.0, &root.0),
            check_cache_writable(&root.0),
            check_port(&root.0, port),
        ] {
            if !c.ok {
                assert!(!c.fix.is_empty(), "{} 失败必须给修复命令(T1.7)", c.name);
            }
        }
        assert!(!check_project(&root.0).ok, "空目录 project 必失败");
        assert!(!check_web(&root.0, &root.0).ok, "空目录 web 必失败");
        assert!(!check_port(&root.0, port).ok, "被占端口必失败");
        drop(guard);
    }

    /// web 资源完整性:三件套齐 → 过;缺 app.js/style.css → 败且 fix 指向 --web。
    #[test]
    fn web_check_reports_missing_files_with_fix() {
        let root = TempRoot::new("web");
        std::fs::write(root.0.join("index.html"), b"<html>").unwrap();
        let c = check_web(&root.0, &root.0);
        assert!(!c.ok, "缺 app.js/style.css 应失败: {}", c.detail);
        assert!(
            c.detail.contains("app.js") && c.detail.contains("style.css"),
            "缺失清单要逐个点名: {}",
            c.detail
        );
        assert!(
            c.fix.contains("--web") || c.fix.contains("CUTFORGE_WEB"),
            "修复命令要可执行: {}",
            c.fix
        );
        for f in WEB_FILES {
            std::fs::write(root.0.join(f), b"x").unwrap();
        }
        assert!(check_web(&root.0, &root.0).ok, "三件套齐备应通过");
    }

    /// 缓存目录:正常目录可写(真实落盘探针);目录不存在时报失败且不越权创建工程根。
    #[test]
    fn cache_check_probes_writability() {
        let root = TempRoot::new("cache");
        assert!(check_cache_writable(&root.0).ok, "正常目录应可写");
        assert!(
            root.0.join(".cutforge/render-cache").is_dir(),
            "探测应建齐缓存目录"
        );
        // 不存在的工程根:失败、给修复,且不得把工程根连带创建出来
        let missing = root.0.join("无此工程");
        let c = check_cache_writable(&missing);
        assert!(!c.ok && !c.fix.is_empty());
        assert!(!missing.exists(), "诊断不得越权创建工程目录");
    }

    /// 端口占用:占住的端口报失败,修复命令给排查命令 + 换端口建议;放手后转为通过。
    #[test]
    fn port_check_reports_occupied_with_alternative() {
        let root = TempRoot::new("port");
        // 确定性:先探得一个空闲口,再亲手占住它
        let freed = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = freed.local_addr().unwrap().port();
        drop(freed);
        let guard = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
        let c = check_port(&root.0, port);
        assert!(!c.ok, "被占端口应失败: {}", c.detail);
        assert!(
            c.fix.contains(&format!("findstr :{port}")),
            "修复命令要含排查命令: {}",
            c.fix
        );
        assert!(
            c.fix.contains("--port"),
            "修复命令要含换端口建议: {}",
            c.fix
        );
        // 放手即空闲:通过
        drop(guard);
        assert!(
            check_port(&root.0, port).ok,
            "释放后同端口应转为空闲: {port}"
        );
    }

    /// project 检查:空目录 → 失败且修复命令是可复制的 new。
    #[test]
    fn project_check_missing_gives_new_command() {
        let root = TempRoot::new("project");
        let c = check_project(&root.0);
        assert!(!c.ok);
        assert!(
            c.fix.starts_with("cutforge-cli new"),
            "修复命令要可复制执行: {}",
            c.fix
        );
        assert!(
            c.fix.contains(&root.0.display().to_string()),
            "修复命令要带具体目录: {}",
            c.fix
        );
    }
}
