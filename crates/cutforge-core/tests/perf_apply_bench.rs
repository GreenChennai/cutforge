// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 内核 apply 吞吐 bench(R-12 验收):1k 片段 × 500 命令的总耗时。
//!
//! - 运行:`cargo test -p cutforge-core -j 2 --test perf_apply_bench -- --ignored --nocapture`
//! - 基线/改造后数据落 `docs/bench/`(env `CUTFORGE_BENCH_OUT` 指定输出 JSON 路径);
//! - TC-PERF-APPLY-001(阈值断言,挂 CI 性能门):env `CUTFORGE_APPLY_BUDGET_MS`
//!   设置时断言 500 命令总耗时 < 预算;未设置时本用例零开销跳过(阈值判定不进
//!   常规单测,避免 CI 抖动误报)。
//!
//! 夹具对齐 docs/bench T1.8 口径:1000 clips / 8 轨(V1-V4 video、A1-A4 audio,
//! 每轨 125×240ms 顺序相接=30s);命令流为 500 笔 clip_update(音量 0.5↔0.8 交替,
//! 每笔均为真实变更、各产一条 Op)。

use cutforge_core::command::{ClipPatch, Command};
use cutforge_core::engine::{ApplyOpts, Engine};
use cutforge_core::model::Project;
use cutforge_core::oplog::Actor;
use serde_json::json;

fn agent() -> Actor {
    Actor::agent("bench")
}

/// 1000 clips / 8 轨夹具(T1.8 同款)。
fn bench_project() -> Project {
    let track = |id: &str, kind: &str| {
        let clips: Vec<_> = (1..=125u64)
            .map(|i| {
                json!({
                    "id": format!("{id}-{i:03}"),
                    "src": "a.mp4",
                    "startMs": (i - 1) * 240,
                    "durationMs": 240,
                })
            })
            .collect();
        json!({"id": id, "kind": kind, "clips": clips})
    };
    let v = json!({
        "version": 1, "schemaVersion": "2.0.0", "slug": "perf-bench", "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "tracks": [
            track("V1", "video"), track("V2", "video"),
            track("V3", "video"), track("V4", "video"),
            track("A1", "audio"), track("A2", "audio"),
            track("A3", "audio"), track("A4", "audio"),
        ]
    });
    Project::from_value(&v).expect("bench 夹具必须合法")
}

/// 全轨 clip id 的命令流顺序(跨轨轮转,贴近真实编辑的均匀触达)。
fn command_ids() -> Vec<String> {
    let mut ids = Vec::with_capacity(1000);
    for t in ["V1", "V2", "V3", "V4", "A1", "A2", "A3", "A4"] {
        for i in 1..=125u64 {
            ids.push(format!("{t}-{i:03}"));
        }
    }
    ids
}

/// 跑 500 笔 clip_update,返回总耗时毫秒。
fn run_500_updates() -> (u128, usize) {
    let mut eng = Engine::new(bench_project()).unwrap();
    let ids = command_ids();
    let t0 = std::time::Instant::now();
    let mut applied = 0usize;
    for (n, id) in ids.iter().take(500).enumerate() {
        let vol = if n % 2 == 0 { 0.5 } else { 0.8 };
        let r = eng
            .apply(
                Command::ClipUpdate {
                    clip_id: id.clone(),
                    patch: ClipPatch {
                        volume: Some(vol),
                        ..Default::default()
                    },
                },
                agent(),
                ApplyOpts::default(),
            )
            .unwrap_or_else(|e| panic!("bench 命令必须成功({id}): {e:?}"));
        assert!(!r.idempotent, "bench 命令必须是真实变更({id})");
        applied += 1;
    }
    let ms = t0.elapsed().as_millis();
    assert_eq!(eng.oplog().len(), 500, "500 命令必须各产一条 Op");
    (ms, applied)
}

/// R-12 bench(手动面):`--ignored --nocapture` 显式运行;数据入 docs/bench。
#[test]
#[ignore = "bench:显式运行(cargo test -p cutforge-core --test perf_apply_bench -- --ignored --nocapture)"]
fn bench_apply_1k_clips_500_cmds() {
    // 预热一次(页缓存/分配器),计时取第二轮
    let _ = run_500_updates();
    let (ms, n) = run_500_updates();
    let per_op_us = (ms as f64 * 1000.0) / n.max(1) as f64;
    println!(
        "R-12 bench: 1k clips × 500 clip_update => {ms} ms 总耗时,{per_op_us:.1} µs/命令(debug profile)"
    );
    if let Ok(out) = std::env::var("CUTFORGE_BENCH_OUT") {
        let doc = json!({
            "capturedAt": cutforge_core::timeutil::now_rfc3339(),
            "buildProfile": "debug(本机纪律:禁 --release)",
            "fixture": "1000 clips / 8 轨(V1-V4 video、A1-A4 audio;每轨 125×240ms=30s)",
            "workload": "500 × clip_update(volume 0.5↔0.8 交替,真实变更各产一条 Op)",
            "iters": n,
            "totalMs": ms,
            "perOpUs": per_op_us,
            "warmup": "1 轮预热后计时",
        });
        let text = serde_json::to_string_pretty(&doc).unwrap();
        std::fs::write(&out, text).unwrap_or_else(|e| panic!("写 bench 输出 {out} 失败: {e}"));
        println!("bench 数据已写入 {out}");
    }
}

/// TC-PERF-APPLY-001(阈值断言):CI 性能门挂载点。
/// env `CUTFORGE_APPLY_BUDGET_MS` 未设置 → 跳过(零开销);设置 → 断言预算。
#[test]
fn tc_perf_apply_001_budget_gate() {
    let Ok(budget) = std::env::var("CUTFORGE_APPLY_BUDGET_MS") else {
        eprintln!("TC-PERF-APPLY-001:未设 CUTFORGE_APPLY_BUDGET_MS,跳过阈值判定");
        return;
    };
    let budget: u128 = budget
        .parse()
        .expect("CUTFORGE_APPLY_BUDGET_MS 必须是毫秒整数");
    let _ = run_500_updates(); // 预热
    let (ms, _) = run_500_updates();
    assert!(
        ms <= budget,
        "TC-PERF-APPLY-001 红线:1k×500 命令耗时 {ms}ms 超预算 {budget}ms"
    );
    println!("TC-PERF-APPLY-001 绿:{ms}ms ≤ {budget}ms");
}
