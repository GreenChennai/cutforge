//! 内核子进程管理:spawn `cutforge-cli serve --root … --port … --token …`,
//! 健康等待(只读探针轮询),Drop 随壳退出。
//!
//! BUG-20(审查报告 v2):①stderr 管道落盘 `<root>/.cutforge/logs/kernel-*.log`
//! (诊断面,非真相源;壳写日志不属于工程数据通道);②HTTP 探针即心跳——
//! 看门狗线程每 3s 探测,连续 3 次失败判定断连并**同端口自动重启**(临时端口
//! 重启会断所有 Rpc 持有者,固定原端口重启则持有者自动复用);③health 标志
//! 供状态栏/设置页消费。附着模式( --attach)不持子进程,无看门狗。

use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::Args;
use crate::rpc::Rpc;

const WATCHDOG_POLL: Duration = Duration::from_secs(3);
const WATCHDOG_FAILS_TO_RESTART: u32 = 3;
const RESTART_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// 看门狗/落盘线程与 Drop 共享的内核控制面。
struct KernelShared {
    child: Mutex<Option<Child>>,
    /// false = 看门狗判定断连(重启中/重启失败),true = 健康
    health: AtomicBool,
    /// 看门狗退出闸(Drop 置位;看门狗每周期与每次 respawn 后检查)
    stop: AtomicBool,
    restart_count: AtomicU64,
}

pub struct Kernel {
    shared: Option<Arc<KernelShared>>,
    pub port: u16,
    #[allow(dead_code)] // 附着模式透传给 Rpc 前的持有位(main 消费)
    pub token: String,
    #[allow(dead_code)] // 透传给状态栏显示
    pub root: PathBuf,
    /// 诊断日志路径(设置页/错误卡可展开;None = 附着模式或尚未产出)
    pub stderr_log: Option<PathBuf>,
}

impl Kernel {
    /// 按启动参数拉起(或附着)内核。失败返回可展示的错误(壳仍开窗)。
    pub fn start(args: &Args) -> Result<Kernel, String> {
        let root = args
            .root
            .clone()
            .ok_or("内部错误:Kernel::start 要求 --root")?;
        if args.attach.is_some() {
            // 附着模式:不持子进程;健康探测交给壳的连接泵(无看门狗/无重启权)
            return Ok(Kernel {
                shared: None,
                port: args.port,
                token: args.token.clone(),
                root,
                stderr_log: None,
            });
        }
        let cli = args.cli.clone().or_else(find_cli).ok_or_else(|| {
            "未找到 cutforge-cli(设 CUTFORGE_CLI 或先 cargo build -p cutforge-cli)".to_string()
        })?;

        // 端口 0 = 系统分配临时端口。固定端口在多实例/孤儿内核场景必撞车:
        // taskkill /F 强杀桌面壳不触发 Drop,内核子进程存活并占着旧端口,
        // 新实例的 RPC 会打到僵尸内核(曾致多轮播放"冻结"假象)。
        // 看门狗重启固定复用首次分配的端口(Rpc 持有者零感知)。
        let port = if args.port == 0 {
            pick_ephemeral_port()?
        } else {
            args.port
        };

        let (child, stderr_log) = spawn_serve(&cli, &root, port, &args.token, &root)?;
        let shared = Arc::new(KernelShared {
            child: Mutex::new(Some(child)),
            health: AtomicBool::new(true),
            stop: AtomicBool::new(false),
            restart_count: AtomicU64::new(0),
        });

        // 健康等待:就绪探针只验证 HTTP 服务面(/ui-fields),不要求工程合法
        // ——工程校验失败开窗后在状态栏展示,壳不为坏工程白等超时(内核起服务通常 <2s)
        let rpc = Rpc::new(
            format!("http://127.0.0.1:{}", port),
            &args.token,
            root.display().to_string(),
        );
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if rpc.probe().is_ok() {
                break;
            }
            if Instant::now() > deadline {
                if let Ok(mut guard) = shared.child.lock()
                    && let Some(mut c) = guard.take()
                {
                    let _ = c.kill();
                    let _ = c.wait();
                }
                return Err("内核 30s 未就绪(日志见 .cutforge/logs/)".into());
            }
            // 子进程提前退出 = 启动失败(端口被占/可执行文件坏等)
            if let Ok(mut guard) = shared.child.lock()
                && let Some(c) = guard.as_mut()
                && c.try_wait().is_ok_and(|st| st.is_some())
            {
                return Err(
                    "内核进程提前退出(端口被占用?可 --port 换口或 --attach 接已有实例)".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(300));
        }

        spawn_watchdog(
            Arc::clone(&shared),
            cli,
            root.clone(),
            port,
            args.token.clone(),
            rpc,
        );
        Ok(Kernel {
            shared: Some(shared),
            port,
            token: args.token.clone(),
            root,
            stderr_log: Some(stderr_log),
        })
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// 内核健康标志的共享句柄(设置页/状态栏消费;附着模式返回常真标志)。
    pub fn health_flag(&self) -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(
            self.shared
                .as_ref()
                .map(|s| s.health.load(Ordering::Relaxed))
                .unwrap_or(true),
        ))
    }

    /// 看门狗累计重启次数的共享句柄(同上)。
    pub fn restart_counter(&self) -> Arc<AtomicU64> {
        Arc::new(AtomicU64::new(
            self.shared
                .as_ref()
                .map(|s| s.restart_count.load(Ordering::Relaxed))
                .unwrap_or(0),
        ))
    }

    /// 诊断日志路径(错误卡/设置页展开;附着模式 None)。
    pub fn stderr_log(&self) -> Option<PathBuf> {
        self.stderr_log.clone()
    }
}

impl Drop for Kernel {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            // 先停看门狗(防重启竞态),再收尸当前子进程(含看门狗刚 respawn 的)
            shared.stop.store(true, Ordering::Relaxed);
            if let Ok(mut guard) = shared.child.lock()
                && let Some(mut c) = guard.take()
            {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

/// spawn serve + stderr 落盘线程。返回子进程与日志路径。
fn spawn_serve(
    cli: &Path,
    root: &Path,
    port: u16,
    token: &str,
    log_root: &Path,
) -> Result<(Child, PathBuf), String> {
    let mut cmd = Command::new(cli);
    cmd.args([
        "serve".as_ref(),
        "--root".as_ref(),
        root.as_os_str(),
        "--port".as_ref(),
        port.to_string().as_ref(),
        "--token".as_ref(),
        token.as_ref(),
    ])
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("拉起 {} 失败:{e}", cli.display()))?;

    // stderr 落盘:.cutforge/logs/kernel-{unixts}.log(诊断面;一次打开逐行追加)
    let log_path = log_root.join(".cutforge/logs");
    let _ = fs::create_dir_all(&log_path);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let log_path = log_path.join(format!("kernel-{ts}.log"));
    let log_file = File::create(&log_path)
        .map_err(|e| format!("内核日志创建失败 {}: {e}", log_path.display()))?;
    if let Some(stderr) = child.stderr.take()
        && std::env::var("CUTFORGE_NO_STDERRLOG").is_err()
    {
        let mut writer = log_file;
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                let st = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                use std::io::Write as _;
                let _ = writeln!(writer, "[{st}] {line}");
            }
            // 进程退出 = 管道关闭,线程自然结束(Drop 侧 kill+wait 收尸)
        });
    }
    Ok((child, log_path))
}

/// 看门狗:每 [`WATCHDOG_POLL`] 探测一次;连续
/// [`WATCHDOG_FAILS_TO_RESTART`] 次失败 → 杀旧子进程 + **同端口**重启 +
/// 15s 探针等待;成功回健康。stop 置位即退出(Drop 先于 kill 检查)。
fn spawn_watchdog(
    shared: Arc<KernelShared>,
    cli: PathBuf,
    root: PathBuf,
    port: u16,
    token: String,
    rpc: Rpc,
) {
    std::thread::spawn(move || {
        let mut fails: u32 = 0;
        loop {
            if shared.stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(WATCHDOG_POLL);
            if shared.stop.load(Ordering::Relaxed) {
                return;
            }
            if rpc.probe().is_ok() {
                fails = 0;
                shared.health.store(true, Ordering::Relaxed);
                continue;
            }
            fails += 1;
            if fails < WATCHDOG_FAILS_TO_RESTART {
                continue;
            }
            shared.health.store(false, Ordering::Relaxed);
            // 重启流程:杀旧(多半已死)→ 同端口 respawn → 探针等待
            if let Ok(mut guard) = shared.child.lock() {
                if let Some(mut old) = guard.take() {
                    let _ = old.kill();
                    let _ = old.wait();
                }
                if shared.stop.load(Ordering::Relaxed) {
                    return;
                }
                match spawn_serve(&cli, &root, port, &token, &root) {
                    Ok((child, _)) => {
                        *guard = Some(child);
                        let probe = Rpc::new(
                            format!("http://127.0.0.1:{}", port),
                            &token,
                            root.display().to_string(),
                        );
                        let deadline = Instant::now() + RESTART_PROBE_TIMEOUT;
                        while Instant::now() < deadline {
                            if probe.probe().is_ok() {
                                shared.restart_count.fetch_add(1, Ordering::Relaxed);
                                shared.health.store(true, Ordering::Relaxed);
                                fails = 0;
                                break;
                            }
                            if shared.stop.load(Ordering::Relaxed) {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(300));
                        }
                    }
                    Err(_) => {
                        // respawn 失败(端口被占/工具消失):下轮重试,健康保持 false
                    }
                }
            }
            // 重启尝试后失败计数清零(下轮重新累计;本轮成败已由 health 表达)
        }
    });
}

/// 取一个系统空闲端口(bind :0 后即弃;内核随即便在该端口起服务)。
/// 极小概率的抢占窗口对桌面应用可接受,胜过固定端口必撞。
fn pick_ephemeral_port() -> Result<u16, String> {
    let l = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("分配临时端口失败:{e}"))?;
    l.local_addr()
        .map(|a| a.port())
        .map_err(|e| format!("分配临时端口失败:{e}"))
}

/// 定位 cutforge-cli:CUTFORGE_CLI 环境变量 → 仓内 target(release/debug)。
pub fn find_cli() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CUTFORGE_CLI") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let mut candidates = vec![
        root.join("release/cutforge-cli.exe"),
        root.join("debug/cutforge-cli.exe"),
    ];
    candidates.retain(|p| p.is_file());
    candidates.first().cloned()
}
