// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! watch 模式(册七 T7.4):`cutforge-cli watch <工程>`——文件变更(轮询 watcher
//! 复用 cutforge_io::watcher,与常驻同步守护同一忽略规则)→ 自动重渲,防抖;
//! Ctrl+C 退出(进程级,无残留状态)。`--max-runs N` 供脚本化验收(e2e 依赖)。

use crate::{Args, emit};
use serde_json::json;
use std::path::Path;

pub fn run(a: &Args) -> i32 {
    let Some(root) = a.positional.first() else {
        return emit(
            a.json,
            false,
            "PRECONDITION_FAILED",
            "用法: watch <工程目录> [--debounce-ms 500] [--max-runs N] [--ass x.ass] [--format mp4 …渲染参数与 render_run 同名]",
            json!({}),
        );
    };
    let root = Path::new(root);
    if !cutforge_io::paths::has_project(root) {
        return emit(
            a.json,
            false,
            "NO_CONFIG",
            &format!("工程不存在: {}(缺 project.json)", root.display()),
            json!({}),
        );
    }
    if cutforge_mcp::resolve_render_bin().is_none() {
        return emit(
            a.json,
            false,
            "DEP_MISSING",
            "未找到 cutforge-render:与 cutforge-cli 同目录放置、加入 PATH,或设 CUTFORGE_RENDER",
            json!({}),
        );
    }
    let debounce_ms: u64 = a
        .flags
        .get("debounce-ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);
    let max_runs: u64 = a
        .flags
        .get("max-runs")
        .and_then(|s| s.parse().ok())
        .unwrap_or(u64::MAX);
    // 渲染参数面与 render_run 同名透传(format/quality/crf/encoder/…)
    let mut args = json!({});
    for (k, v) in &a.flags {
        args[k.as_str()] = json!(v);
    }
    let ass = a
        .flags
        .get("ass")
        .map(String::as_str)
        .filter(|s| !s.is_empty() && root.join(s).is_file());
    let extra = cutforge_mcp::build_render_extra(&args, true);

    println!(
        "[watch] {}(防抖 {debounce_ms}ms;Ctrl+C 退出)",
        root.display()
    );
    let mut watcher = cutforge_io::watcher::Watcher::new(root, debounce_ms);
    let mut runs: u64 = 0;
    // 启动即渲一次(watch 语义:进入即保证盘面有最新成片)
    loop {
        runs += 1;
        let t0 = std::time::Instant::now();
        let env = cutforge_mcp::render_cutforge_sync(root, ass, false, &extra);
        let ok = env["ok"] == json!(true);
        let output = env["data"]["output"].as_str().unwrap_or_default();
        println!(
            "[watch] 第 {runs} 次渲染:{} ({}ms){}",
            if ok { "PASS" } else { "FAIL" },
            t0.elapsed().as_millis(),
            if output.is_empty() {
                String::new()
            } else {
                format!(" → {output}")
            }
        );
        if !ok {
            println!("[watch] {}", env["message"].as_str().unwrap_or_default());
        }
        if runs >= max_runs {
            return emit(
                a.json,
                ok,
                if ok { "OK" } else { "INTERNAL" },
                &format!("watch 结束:共 {runs} 次渲染"),
                json!({"runs": runs, "lastOk": ok}),
            );
        }
        // 防抖轮询:任一非忽略文件变化 → 重渲(watcher 已忽略 oplog/成片输出/状态簿记)
        loop {
            std::thread::sleep(watcher.debounce);
            let events = watcher.poll();
            if !events.is_empty() {
                println!("[watch] 检测到 {} 处变更,重渲…", events.len());
                break;
            }
        }
    }
}
