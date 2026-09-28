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
            _ => {}
        }
        i += 1;
    }
    let Some(root) = root else {
        eprintln!("用法: cutforge-render --root <工程目录> [--ass <subtitles.ass>] [--frame <atMs> [--format png|jpeg]]");
        std::process::exit(3);
    };
    if let Some(at_ms) = frame_ms {
        // 单帧模式(T2.4):同步执行一帧,完成事件一行 + FRAME_OK <路径>
        match load_project_from_disk(&root).and_then(|p| {
            cutforge_render::render_frame(&p, Path::new(&root), ass.as_deref(), at_ms, fmt)
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
    // 整片模式(原路径,行为零变化)
    let project = match load_project_from_disk(&root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(if e.starts_with("SCHEMA_INVALID") { 2 } else { 3 });
        }
    };
    match cutforge_render::render(&project, Path::new(&root), ass.as_deref(), &mut cutforge_render::write_progress) {
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
