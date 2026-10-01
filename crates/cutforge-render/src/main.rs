// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! cutforge-render CLI:headless 渲染入口(计划书 6.5)。
//! 输出结构化进度事件(JSON 行);失败带具体步骤与原因。

use std::path::{Path, PathBuf};

/// 从盘面加载工程(cutforge-render 的两模式共用):目录契约 0.5 优先
/// 05_时间线工程/project.json(旧布局 05_ir/ 兼容);ADR-0002:真实 IR 顶层
/// `_meta` 剥出(渲染只读工程、不回写,直接丢弃)。
fn load_project_from_disk(root: &Path) -> Result<cutforge_core::model::Project, String> {
    let text = std::fs::read_to_string(cutforge_io::paths::project_path(root))
        .map_err(|e| format!("NO_CONFIG: {e}"))?;
    let mut v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("SCHEMA_INVALID: {e}"))?;
    if let Some(obj) = v.as_object_mut() {
        obj.remove("_meta");
    }
    cutforge_core::model::migrate_from_value(&v).map_err(|errors| format!("SCHEMA_INVALID: {}", errors.join("; ")))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut root: Option<PathBuf> = None;
    let mut ass: Option<PathBuf> = None;
    // T2.4 单帧模式:--frame <atMs> [--format png|jpeg];给了 --frame 即走单帧管线
    let mut frame_ms: Option<u64> = None;
    let mut fmt = cutforge_render::FrameFormat::Png;
    let mut use_proxy = false;
    // 册五 T5.6 渲染选项(缺省 = 现行为零变化)
    let mut opts = cutforge_render::plan::RenderOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => root = args.get(i + 1).map(PathBuf::from),
            "--ass" => ass = args.get(i + 1).map(PathBuf::from),
            "--frame" => frame_ms = args.get(i + 1).and_then(|v| v.parse::<u64>().ok()),
            "--format" => {
                if let Some(v) = args.get(i + 1)
                    && let Some(f) = cutforge_render::FrameFormat::parse(v) {
                        fmt = f;
                    }
            }
            // 册四 T4.1 代理预览(显式 opt-in;缺失代理的片段回落原片)
            "--use-proxy" => use_proxy = true,
            // 册五 T5.6:编码器(auto|hw|sw)/质量预设/显式参数面/响度目标/命令回显
            "--encoder" => opts.encoder = args.get(i + 1).map(String::from),
            "--quality" => opts.quality = args.get(i + 1).map(String::from),
            "--crf" => opts.crf = args.get(i + 1).and_then(|v| v.parse::<u32>().ok()),
            "--bitrate" => opts.bitrate_kbps = args.get(i + 1).and_then(|v| v.parse::<u64>().ok()),
            "--gop" => opts.gop = args.get(i + 1).and_then(|v| v.parse::<u32>().ok()),
            "--pix-fmt" => opts.pix_fmt = args.get(i + 1).map(String::from),
            "--loudnorm-target" => {
                // "I:TP" 形(如 "-16:-1.5");只给 I 亦合法
                if let Some(v) = args.get(i + 1) {
                    let mut it = v.split(':').filter_map(|n| n.parse::<f64>().ok());
                    opts.loudnorm_i = it.next();
                    opts.loudnorm_tp = it.next();
                }
            }
            "--verbose-cmd" => opts.verbose_cmd = true,
            _ => {}
        }
        i += 1;
    }
    let Some(root) = root else {
        eprintln!("用法: cutforge-render --root <工程目录> [--ass <subtitles.ass>] [--frame <atMs> [--format png|jpeg]] [--use-proxy] [--encoder auto|hw|sw] [--quality fast|balanced|quality] [--crf N] [--bitrate K] [--gop N] [--pix-fmt F] [--loudnorm-target I[:TP]] [--verbose-cmd]");
        std::process::exit(3);
    };
    if let Some(at_ms) = frame_ms {
        // 单帧模式(T2.4):同步执行一帧,完成事件一行 + FRAME_OK <路径>
        match load_project_from_disk(&root).and_then(|p| {
            cutforge_render::render_frame_opts(&p, Path::new(&root), ass.as_deref(), at_ms, fmt, use_proxy)
        }) {
            Ok(outcome) => {
                cutforge_render::write_progress(cutforge_render::frame_done_event(&outcome));
                println!("FRAME_OK {}", outcome.output.display());
            }
            Err(e) => {
                eprintln!("FRAME_FAIL: {e}");
                std::process::exit(2);
            }
        }
        return;
    }
    // 整片模式(原路径行为零变化;新选项显式给定才分叉)
    let project = match load_project_from_disk(&root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(if e.starts_with("SCHEMA_INVALID") { 2 } else { 3 });
        }
    };
    let outcome = cutforge_render::render_with_opts(
        &project,
        Path::new(&root),
        ass.as_deref(),
        use_proxy,
        opts,
        &mut cutforge_render::write_progress,
    );
    match outcome {
        Ok(outcome) => {
            for (step, ok) in &outcome.steps {
                cutforge_render::write_progress(serde_json::json!({"step": step, "ok": ok}));
            }
            cutforge_render::write_progress(serde_json::json!({
                "done": true, "output": outcome.output.to_string_lossy(),
                "cacheHits": outcome.cache_hits, "segments": outcome.segments,
            }));
            println!("RENDER_OK {}", outcome.output.display());
        }
        Err(e) => {
            eprintln!("RENDER_FAIL: {e}");
            std::process::exit(2);
        }
    }
}
