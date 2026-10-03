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
        if let Some(url) = &args.attach {
            // 附着模式:不持子进程;健康探测交给壳的连接泵
            return Ok(Kernel {
                child: None,
                port: args.port,
                token: args.token.clone(),
                root: args.root.clone(),
            }
            .with_attach_note(url.clone()));
        }
        let cli = args.cli.clone().or_else(find_cli).ok_or_else(|| {
            "未找到 cutforge-cli(设 CUTFORGE_CLI 或先 cargo build -p cutforge-cli)".to_string()
        })?;

        let mut child = Command::new(&cli)
            .args([
                "serve".as_ref(),
                "--root".as_ref(),
                args.root.as_os_str(),
                "--port".as_ref(),
                args.port.to_string().as_ref(),
                "--token".as_ref(),
                args.token.as_ref(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("拉起 {} 失败:{e}", cli.display()))?;

        // 健康等待:只读 project_get 直到 200(内核起服务通常 <2s)
        let rpc = Rpc::new(
            format!("http://127.0.0.1:{}", args.port),
            &args.token,
            args.root.display().to_string(),
        );
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if rpc
                .call("project_get", serde_json::json!({}), Duration::from_secs(2))
                .is_ok()
            {
                break;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                return Err("内核 30s 未就绪(查看 cutforge-cli serve 是否报错)".into());
            }
            // 子进程提前退出 = 启动失败(工程不存在等)
            if let Ok(Some(_)) = child.try_wait() {
                return Err(
                    "内核进程提前退出(root 工程目录不存在?先用 cutforge-cli new 创建)".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(300));
        }

        Ok(Kernel {
            child: Some(child),
            port: args.port,
            token: args.token.clone(),
            root: args.root.clone(),
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

/// 定位 cutforge-cli:CUTFORGE_CLI 环境变量 → 仓内 target(release/debug)。
fn find_cli() -> Option<PathBuf> {
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
