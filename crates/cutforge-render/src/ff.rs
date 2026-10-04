// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! ffmpeg/ffprobe 子进程执行器(R-07):超时看门狗 + stderr 流式尾部 4KB。
//!
//! 写法模板:apps/desktop/src/playback/{decoder,subprocess}.rs 的看门狗
//! (超时 kill + wait 收尸;ADR-0009 纯 std,不引依赖)。差异说明:decoder 的
//! 看门狗是独立线程(主线程要持续读帧);本模块主线程本来就在等子进程,
//! 等待方即看门狗——轮询 `try_wait` + 到点 `kill`+`wait`,语义等价且无共享
//! 状态竞争。
//!
//! 纪律:
//! - stdin 恒 null(与原 `Command::output()` 语义一致,ffmpeg 不吃交互输入);
//! - stdout 全量缓冲(消费方仅 ffprobe JSON 等小产物,与旧行为一致),
//!   stderr 流式读、只留尾部 4KB 环形——海量进度/verbose 日志不再整体进内存
//!   (TC-RENDER-FF-002);
//! - 超时 kill:到点 kill + wait 收尸,不留僵尸;错误 = [`RenderError::FfmpegTimeout`]
//!   (阶段名随错误上行,TC-RENDER-FF-001);
//! - 超时缺省 10min;encode 按预估时长×3 放大且不低于缺省([`encode_timeout`]);
//!   `RenderOptions.ff_timeout_secs` 是编排层(mcp/CLI)的显式透传接口。

use crate::plan::RenderOptions;
use std::fmt;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// 缺省超时(工单口径:10min)。
pub const FF_TIMEOUT_DEFAULT: Duration = Duration::from_secs(600);
/// stderr 尾部保留字节(工单口径:4KB 环形)。
pub const STDERR_TAIL_BYTES: usize = 4096;
/// 看门狗轮询间隔。
const WATCHDOG_POLL: Duration = Duration::from_millis(100);

/// 渲染子进程错误(R-07):超时显式分类,阶段名随错误上行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// 超过时限,看门狗已 kill + wait(阶段名 + stderr 尾部供归因)。
    FfmpegTimeout { stage: String, stderr_tail: String },
    /// 子进程退出码非 0(工具名 + stderr 尾部)。
    Failed { tool: String, stderr_tail: String },
    /// 启动失败(工具不存在/权限等)。
    Spawn { tool: String, message: String },
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::FfmpegTimeout { stage, stderr_tail } => write!(
                f,
                "TIMEOUT: stage={stage} 子进程超时,看门狗已 kill;stderr 尾部: {stderr_tail}"
            ),
            RenderError::Failed { tool, stderr_tail } => {
                // 错误文本形状与旧 run_ff 一致("{tool} 失败: …"),既有解析面零变化
                let tail: String = stderr_tail.trim().chars().take(800).collect();
                write!(f, "{tool} 失败: {tail}")
            }
            RenderError::Spawn { tool, message } => write!(f, "启动 {tool} 失败: {message}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// encode 步超时(工单口径:预估时长×3,不低于缺省 10min——短视频也给足基线,
/// 长片按内容放大,避免恒定 10min 误杀长渲染)。
pub fn encode_timeout(total_ms: u64) -> Duration {
    let est = Duration::from_millis(total_ms.saturating_mul(3));
    est.max(FF_TIMEOUT_DEFAULT)
}

/// 渲染会话的有效超时(编排层透传面):显式 `ff_timeout_secs` 完全覆盖,
/// 否则缺省 10min(encode 另按 [`encode_timeout`] 放大)。
pub fn plan_ff_timeout(opts: &RenderOptions) -> Duration {
    opts.ff_timeout_secs
        .map(Duration::from_secs)
        .unwrap_or(FF_TIMEOUT_DEFAULT)
}

/// 渲染会话 encode 步的有效超时(编排层透传面)。
pub fn plan_encode_timeout(opts: &RenderOptions, total_ms: u64) -> Duration {
    match opts.ff_timeout_secs {
        Some(s) => Duration::from_secs(s),
        None => encode_timeout(total_ms),
    }
}

/// stderr 尾部环形(固定上限;按块流入,内存 O(4KB),不受行结构影响——
/// 单条超长行也不会撑大缓冲)。
struct TailRing {
    buf: Vec<u8>,
}

impl TailRing {
    fn new() -> Self {
        Self {
            buf: Vec::with_capacity(STDERR_TAIL_BYTES + 1024),
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
        let overflow = self.buf.len().saturating_sub(STDERR_TAIL_BYTES);
        if overflow > 0 {
            self.buf.drain(..overflow);
        }
    }

    fn tail(&self) -> String {
        String::from_utf8_lossy(&self.buf).into_owned()
    }
}

/// 核心:带阶段名 + 超时看门狗地执行子进程,返回 (stdout 全量, stderr 尾部)。
/// 超时 → kill + wait 后返回 [`RenderError::FfmpegTimeout`]。
pub(crate) fn run_ff_stage(
    stage: &str,
    dir: Option<&Path>,
    tool: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<(String, String), RenderError> {
    let program = crate::ff_bin(tool);
    let mut cmd = Command::new(&program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let mut child = cmd.spawn().map_err(|e| RenderError::Spawn {
        tool: tool.to_string(),
        message: e.to_string(),
    })?;
    let stdout_pipe = child.stdout.take().expect("stdout 已 piped");
    let stderr_pipe = child.stderr.take().expect("stderr 已 piped");

    // stdout:全量(消费方为 ffprobe JSON 等小产物;与旧 .output() 行为一致)
    let th_out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::BufReader::new(stdout_pipe).read_to_end(&mut buf);
        buf
    });
    // stderr:流式尾部环(1KB 块推进,内存恒 O(4KB))
    let th_err = std::thread::spawn(move || {
        let mut ring = TailRing::new();
        let mut chunk = [0u8; 1024];
        let mut rd = stderr_pipe;
        loop {
            match rd.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => ring.push(&chunk[..n]),
            }
        }
        ring.tail()
    });

    // 看门狗:主线程轮询 try_wait,到点 kill + wait 收尸(playback 看门狗同款语义)
    let deadline = std::time::Instant::now() + timeout;
    let mut timed_out = false;
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                let now = std::time::Instant::now();
                if now >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break Ok(std::process::ExitStatus::default());
                }
                std::thread::sleep(WATCHDOG_POLL.min(deadline.saturating_duration_since(now)));
            }
            Err(e) => {
                break Err(RenderError::Spawn {
                    tool: tool.to_string(),
                    message: e.to_string(),
                });
            }
        }
    };

    // 读线程随管道 EOF(child 退出/被杀)收敛,join 防泄漏
    let stdout_s = th_out
        .join()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let stderr_tail = th_err.join().unwrap_or_default();

    match outcome {
        Ok(_) if timed_out => Err(RenderError::FfmpegTimeout {
            stage: stage.to_string(),
            stderr_tail,
        }),
        Ok(status) if status.success() => Ok((stdout_s, stderr_tail)),
        Ok(_) => Err(RenderError::Failed {
            tool: tool.to_string(),
            stderr_tail,
        }),
        Err(e) => Err(e),
    }
}

/// 执行 ffmpeg/ffprobe(缺省 10min 超时;错误归一为字符串,阶段名在文本内)。
pub fn run_ff(stage: &str, tool: &str, args: &[&str]) -> Result<String, String> {
    let (out, _) =
        run_ff_stage(stage, None, tool, args, FF_TIMEOUT_DEFAULT).map_err(|e| e.to_string())?;
    Ok(out)
}

/// 双通道捕获版本(loudnorm 测量 JSON 输出在 stderr **尾部**,4KB 环完整容纳)。
pub fn run_ff_capture(stage: &str, tool: &str, args: &[&str]) -> Result<(String, String), String> {
    run_ff_stage(stage, None, tool, args, FF_TIMEOUT_DEFAULT).map_err(|e| e.to_string())
}

/// 指定工作目录版本(字幕烧录的滤镜相对路径依赖 cwd = 缓存根)。
pub fn run_ff_in(stage: &str, dir: &Path, tool: &str, args: &[&str]) -> Result<String, String> {
    let (out, _) = run_ff_stage(stage, Some(dir), tool, args, FF_TIMEOUT_DEFAULT)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// 显式超时版本(encode 预估×3 / RenderOptions.ff_timeout_secs 透传)。
pub fn run_ff_timeout(
    stage: &str,
    dir: Option<&Path>,
    tool: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let (out, _) = run_ff_stage(stage, dir, tool, args, timeout).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_timeout_scales_with_duration_and_floor() {
        assert_eq!(encode_timeout(0), FF_TIMEOUT_DEFAULT, "下限 10min");
        assert_eq!(
            encode_timeout(60_000),
            FF_TIMEOUT_DEFAULT,
            "3min < 10min → 用缺省"
        );
        assert_eq!(
            encode_timeout(3_600_000),
            Duration::from_secs(10_800),
            "1h 素材 → 3h 预算"
        );
    }

    #[test]
    fn failed_display_keeps_legacy_shape() {
        let e = RenderError::Failed {
            tool: "ffmpeg".into(),
            stderr_tail: "boom".into(),
        };
        assert_eq!(e.to_string(), "ffmpeg 失败: boom", "旧错误文本形状不变");
        let e = RenderError::FfmpegTimeout {
            stage: "encode".into(),
            stderr_tail: String::new(),
        };
        let s = e.to_string();
        assert!(s.contains("TIMEOUT") && s.contains("stage=encode"), "{s}");
    }

    // ---- 跨平台 mock(CI rust-gates 在 ubuntu 跑,无 powershell)----
    // 只依赖两平台恒在的原语:Windows(ping/powershell)、Unix(sleep/sh/head,
    // /dev/zero 为 coreutils+procfs 标准;sh 在 ubuntu runner 恒在)。断言口径
    // 两平台一致:超时 kill+阶段名 / 尾部 4KB 环内容 / 内存峰值预算。

    /// 挂起 mock(FF-001):Windows `ping -n 30`(≈29s)/ Unix `sleep 30`。
    fn hang_mock() -> (&'static str, Vec<&'static str>) {
        if cfg!(windows) {
            ("ping", vec!["-n", "30", "127.0.0.1"])
        } else {
            ("sleep", vec!["30"])
        }
    }

    /// 大量 stderr mock(FF-002):约 100MB 泼向 stderr;返回期望尾字符。
    /// Windows:powershell 1000×100KB 的 'x';Unix:`sh -c 'head -c 100000000
    /// /dev/zero >&2'`(head 默认写 stdout,必须经 shell 重定向到 stderr;
    /// NUL 流量级等效,旧全量缓冲实现同样会 +100MB)。
    fn bulk_stderr_mock() -> (&'static str, Vec<&'static str>, char) {
        if cfg!(windows) {
            let script = "$b='x'*100000; for($i=0;$i -lt 1000;$i++){[Console]::Error.Write($b)}";
            ("powershell", vec!["-NoProfile", "-Command", script], 'x')
        } else {
            ("sh", vec!["-c", "head -c 100000000 /dev/zero >&2"], '\u{0}')
        }
    }

    /// 定量 stderr mock(尾部环测试):恰好 >4KB、以 "0123456789" 收尾(无换行,
    /// 精确断言尾部字节)。
    fn tail_probe_mock() -> (&'static str, Vec<&'static str>) {
        if cfg!(windows) {
            let script = "$s='0123456789'*600; [Console]::Error.Write($s)";
            ("powershell", vec!["-NoProfile", "-Command", script])
        } else {
            // 667 × "0123456789" = 6670 字节;printf 不带换行 → 尾部精确可断言;
            // 重定向作用于整个 while 复合命令(>4KB 进 stderr)
            let script = "i=0; while [ $i -lt 667 ]; do printf 0123456789; i=$((i+1)); done >&2";
            ("sh", vec!["-c", script])
        }
    }

    /// TC-RENDER-FF-001:mock 挂起 → 到点 kill + `FfmpegTimeout{stage}`。
    /// (挂起 mock ≈ 29~30s;看门狗 0.5s 即杀。红注:旧 run_ff 无超时参数,
    /// 该测试在旧接口下只能永久悬挂——接口级缺失,无法安全演示行为红。)
    #[test]
    fn tc_render_ff_001_hang_is_killed_with_stage() {
        let (tool, args) = hang_mock();
        let t0 = std::time::Instant::now();
        let err = run_ff_stage("tc-ff-001", None, tool, &args, Duration::from_millis(500))
            .expect_err("挂起 mock 必须以超时失败");
        assert!(
            matches!(err, RenderError::FfmpegTimeout { ref stage, .. } if stage == "tc-ff-001"),
            "错误必须是 FfmpegTimeout 且带阶段名: {err}"
        );
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "超时必须及时 kill(不等 mock 自然结束): {:?}",
            t0.elapsed()
        );
    }

    /// TC-RENDER-FF-002:100MB stderr → 进程内存峰值增量 <50MB。
    /// 全局分配器计数(cfg(test) 下本测试二进制生效);旧实现 .output() 全量
    /// 缓冲会 +100MB,流式尾部环应 <1MB(50MB 预算吸收并行测试噪声)。
    #[test]
    fn tc_render_ff_002_mass_stderr_bounded_memory() {
        let (tool, args, tail_char) = bulk_stderr_mock();
        alloc_probe::reset_peak();
        let before = alloc_probe::peak();
        let (_, tail) = run_ff_stage("tc-ff-002", None, tool, &args, Duration::from_secs(180))
            .expect("mock 必须正常退出");
        let peak_delta = alloc_probe::peak().saturating_sub(before);
        assert!(
            peak_delta < 50 * 1024 * 1024,
            "100MB stderr 下进程峰值增量必须 <50MB: {peak_delta}"
        );
        assert!(
            tail.len() <= STDERR_TAIL_BYTES + 1024,
            "尾部环上限: {}",
            tail.len()
        );
        assert!(tail.ends_with(tail_char), "尾部必须保留最后字节");
    }

    /// stderr 尾部环:超量输入只留最后 4KB,且以最后字节收尾。
    #[test]
    fn stderr_tail_keeps_last_bytes() {
        let (tool, args) = tail_probe_mock();
        let (_, tail) = run_ff_stage("tc-ff-tail", None, tool, &args, Duration::from_secs(60))
            .expect("mock 必须正常退出");
        assert!(
            tail.len() <= STDERR_TAIL_BYTES + 1024,
            "尾部环上限: {}",
            tail.len()
        );
        assert!(tail.ends_with("0123456789"), "以最后字节收尾: …{tail}");
        assert!(tail.len() >= 4096, "4KB 环应装满: {}", tail.len());
    }

    /// 内存探测分配器(cfg(test) 下作为本测试二进制的全局分配器)。
    /// 参照 crates 内既有"零依赖"纪律,仅 std;峰值只在本测试内解读
    /// (reset 到当前存量,读增量),并留 50MB 预算吸收并行测试噪声。
    mod alloc_probe {
        use std::alloc::{GlobalAlloc, Layout, System};
        use std::sync::atomic::{AtomicUsize, Ordering};

        static CUR: AtomicUsize = AtomicUsize::new(0);
        static PEAK: AtomicUsize = AtomicUsize::new(0);

        pub struct Counting;

        unsafe impl GlobalAlloc for Counting {
            #[allow(unsafe_op_in_unsafe_fn)]
            unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                let p = System.alloc(layout);
                if !p.is_null() {
                    let cur = CUR.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
                    PEAK.fetch_max(cur, Ordering::SeqCst);
                }
                p
            }

            #[allow(unsafe_op_in_unsafe_fn)]
            unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
                CUR.fetch_sub(layout.size(), Ordering::SeqCst);
                System.dealloc(p, layout);
            }

            #[allow(unsafe_op_in_unsafe_fn)]
            unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
                let np = System.realloc(p, layout, new_size);
                if !np.is_null() {
                    let cur = CUR
                        .fetch_add(new_size.wrapping_sub(layout.size()), Ordering::SeqCst)
                        .wrapping_add(new_size.wrapping_sub(layout.size()));
                    PEAK.fetch_max(cur, Ordering::SeqCst);
                }
                np
            }
        }

        #[global_allocator]
        static ALLOC: Counting = Counting;

        pub fn reset_peak() {
            PEAK.store(CUR.load(Ordering::SeqCst), Ordering::SeqCst);
        }

        pub fn peak() -> usize {
            PEAK.load(Ordering::SeqCst)
        }
    }
}
