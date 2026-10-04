//! 内核子进程管理:spawn `cutforge-cli serve --root … --port … --token …`,
//! 健康等待(只读 project_get 轮询),Drop 随壳退出。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::Args;
use crate::rpc::Rpc;

pub struct Kernel {
    child: Option<Child>,
    pub port: u16,
    #[allow(dead_code)] // 附着模式透传给 Rpc 前的持有位(main 消费)
    pub token: String,
    #[allow(dead_code)] // 透传给状态栏显示
    pub root: PathBuf,
}

impl Kernel {
    /// 按启动参数拉起(或附着)内核。失败返回可展示的错误(壳仍开窗)。
    pub fn start(args: &Args) -> Result<Kernel, String> {
        let root = args
            .root
            .clone()
            .ok_or("内部错误:Kernel::start 要求 --root")?;
        if let Some(url) = &args.attach {
            // 附着模式:不持子进程;健康探测交给壳的连接泵
            return Ok(Kernel {
                child: None,
                port: args.port,
                token: args.token.clone(),
                root,
            }
            .with_attach_note(url.clone()));
        }
        let cli = args.cli.clone().or_else(find_cli).ok_or_else(|| {
            "未找到 cutforge-cli(设 CUTFORGE_CLI 或先 cargo build -p cutforge-cli)".to_string()
        })?;

        // 端口 0 = 系统分配临时端口。固定端口在多实例/孤儿内核场景必撞车:
        // taskkill /F 强杀桌面壳不触发 Drop,内核子进程存活并占着旧端口,
        // 新实例的 RPC 会打到僵尸内核(曾致多轮播放"冻结"假象)。
        let port = if args.port == 0 {
            pick_ephemeral_port()?
        } else {
            args.port
        };

        let mut child = Command::new(&cli)
            .args([
                "serve".as_ref(),
                "--root".as_ref(),
                root.as_os_str(),
                "--port".as_ref(),
                port.to_string().as_ref(),
                "--token".as_ref(),
                args.token.as_ref(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("拉起 {} 失败:{e}", cli.display()))?;

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
                let _ = child.kill();
                return Err("内核 30s 未就绪(查看 cutforge-cli serve 是否报错)".into());
            }
            // 子进程提前退出 = 启动失败(端口被占/可执行文件坏等)
            if let Ok(Some(_)) = child.try_wait() {
                return Err(
                    "内核进程提前退出(端口被占用?可 --port 换口或 --attach 接已有实例)".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(300));
        }

        Ok(Kernel {
            child: Some(child),
            port,
            token: args.token.clone(),
            root,
        })
    }

    fn with_attach_note(self, _url: String) -> Self {
        self
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for Kernel {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
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
    // 本壳位于 <repo>/apps/desktop,仓内 target 在 <repo>/target
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2)?;
    for profile in ["release", "debug"] {
        let mut p = repo.join("target").join(profile).join("cutforge-cli");
        p.set_extension(std::env::consts::EXE_EXTENSION);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}
