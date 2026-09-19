// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! cutforge-render CLI:headless 渲染入口(计划书 6.5)。
//! 输出结构化进度事件(JSON 行);失败带具体步骤与原因。

use cutforge_core::model::Project;
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut root: Option<PathBuf> = None;
    let mut ass: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => root = args.get(i + 1).map(PathBuf::from),
            "--ass" => ass = args.get(i + 1).map(PathBuf::from),
            _ => {}
        }
        i += 1;
    }
    let Some(root) = root else {
        eprintln!("用法: cutforge-render --root <工程目录> [--ass <subtitles.ass>]");
        std::process::exit(3);
    };
    let text = match std::fs::read_to_string(root.join("05_ir/project.json")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("NO_CONFIG: {e}");
            std::process::exit(3);
        }
    };
    let mut v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("SCHEMA_INVALID: {e}");
            std::process::exit(2);
        }
    };
    // ADR-0002:CutFlow 真实 IR 顶层带 `_meta`(实现细节字段,被契约 additionalProperties=false 拒绝)。
    // 与 Workspace::open 同口径——进内核前剥出;渲染只读工程、不回写,直接丢弃。
    // 此前缺失这一步:真实工程经 cutforge-render 导出必 SCHEMA_INVALID(E5 最后一跳不通的根因之一)。
    if let Some(obj) = v.as_object_mut() {
        obj.remove("_meta");
    }
    let project = match cutforge_core::model::migrate_from_value(&v) {
        Ok(p) => p,
        Err(errors) => {
            eprintln!("SCHEMA_INVALID: {}", errors.join("; "));
            std::process::exit(2);
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
