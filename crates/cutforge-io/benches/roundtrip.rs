//! M3-1 基准:同步往返延迟(AI 改动 → 编辑器可见;用户改动 → AI 可感知)。
//! harness = false:自设主函数,输出结构化 JSON(计划书 4.5/0.5 北极星指标)。
//! 阈值:AI 可见 P95 ≤ 100 ms;AI 可感知 P95 ≤ 200 ms。

use cutforge_core::command::{ClipPatch, Command};
use cutforge_core::engine::{ApplyOpts, Query};
use cutforge_core::oplog::Actor;
use cutforge_io::Workspace;
use std::time::{Duration, Instant};

fn percentile(mut v: Vec<f64>, p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[idx.min(v.len() - 1)]
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let as_json = args.iter().any(|a| a == "--json");
    let iters: usize = args
        .iter()
        .position(|a| a == "--iters")
        .and_then(|i| args.get(i + 1).and_then(|n| n.parse().ok()))
        .unwrap_or(40);

    let root = match cutforge_io::tests_fixture("bench-roundtrip") {
        Ok(r) => r,
        Err(e) => {
            eprintln!("夹具创建失败: {e}");
            return std::process::ExitCode::from(4);
        }
    };
    let actor = Actor::agent("bench");
    // 预热(文件系统缓存)
    for i in 0..3 {
        let mut ws = Workspace::open(&root).unwrap();
        let dur = 8000 + i;
        let _ = ws.apply(
            Command::ClipUpdate {
                clip_id: "V1-001".into(),
                patch: ClipPatch { duration_ms: Some(dur), ..Default::default() },
            },
            actor.clone(),
            ApplyOpts::default(),
        );
    }

    let mut ai_visible: Vec<f64> = Vec::new(); // apply+落盘+重开+查询(编辑器看到)
    let mut perceivable: Vec<f64> = Vec::new(); // 落盘后重开+oplog tail(AI 感知)
    for i in 0..iters {
        let dur = 7000 + (i as u64 % 900);
        let t0 = Instant::now();
        let mut ws = Workspace::open(&root).unwrap();
        ws.apply(
            Command::ClipUpdate {
                clip_id: "V1-001".into(),
                patch: ClipPatch { duration_ms: Some(dur), ..Default::default() },
            },
            actor.clone(),
            ApplyOpts::default(),
        )
        .unwrap();
        let t1 = Instant::now();
        let ws2 = Workspace::open(&root).unwrap();
        let _ = ws2.engine().query(Query::Timeline);
        let t2 = Instant::now();
        let ws3 = Workspace::open(&root).unwrap();
        let _ = ws3.engine().query(Query::OpLogTail { since_rev: None, actor_kind: None });
        let t3 = Instant::now();

        // AI 改动 → 编辑器可见 = 写入+落盘+重开+视图查询
        ai_visible.push((t2 - t0).as_secs_f64() * 1000.0);
        // 用户/AI 感知链 = 落盘后重开+OpLog 读取
        perceivable.push((t3 - t1).as_secs_f64() * 1000.0);
    }

    let ai_p95 = percentile(ai_visible.clone(), 0.95);
    let user_p95 = percentile(perceivable.clone(), 0.95);
    let ok = ai_p95 <= 100.0 && user_p95 <= 200.0;
    if as_json {
        println!(
            "{}",
            serde_json::json!({
                "ok": ok, "code": if ok { "OK" } else { "LATENCY_FAIL" },
                "message": format!("AI 可见 P95 {ai_p95:.1}ms(≤100)/AI 可感知 P95 {user_p95:.1}ms(≤200)"),
                "data": {
                    "iters": iters,
                    "ai_visible_p95_ms": ai_p95,
                    "user_perceivable_p95_ms": user_p95,
                    "ai_visible_p50_ms": percentile(ai_visible, 0.5),
                    "user_perceivable_p50_ms": percentile(perceivable, 0.5),
                }
            })
        );
    } else {
        println!(
            "AI 可见 P95 {ai_p95:.1}ms / AI 可感知 P95 {user_p95:.1}ms({iters} 轮) → {}",
            if ok { "PASS" } else { "FAIL" }
        );
    }
    cutforge_io::fsutil::cleanup(&root);
    let _ = Duration::ZERO;
    if ok { std::process::ExitCode::SUCCESS } else { std::process::ExitCode::from(2) }
}
