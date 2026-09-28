#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册二 AC-2.4 时间线性能门禁(e2e_perf_timeline):1,000 clips / 8 轨合成工程上新壳的表现。

    python tools/e2e_perf_timeline.py [--min-fps 55] [--max-clip-nodes 100]
                                      [--bin cutforge-mcp] [--cli cutforge-cli] [--sweep-step 64]

断言链:
  1. 夹具:cli new 起骨架 → 8 轨 × 125 clips = 1,000 clips 直接组装 project.json
     (构造口径同 tools/bench/bench.py,合法性由打开时 Rust v2 契约兜底);
  2. 虚拟化:1k clips 下 DOM 中 [data-testid="clip"] 节点数 ≤ --max-clip-nodes
     (视口外不渲染;滚动到最远端后复查一次,首尾两窗皆虚拟化);
  3. 帧采样:页面内 rAF 计数采样,横扫滚动全程(逐帧推进 scrollLeft)+ 一次
     clip 拖拽(ghost 跟手 → clip_move 落账);帧间隔 → fps 序列,P95 ≥ --min-fps。
     机器负载敏感:--min-fps 可放宽,报告始终记录实测值;
  4. 功能闭环:拖拽产生 clip_move(rev 上涨),虚拟化不破坏编辑链路。

跨平台:pathlib;二进制定位带无 .exe 回退;只依赖 playwright + stdlib(+ ffmpeg
现场造夹具素材,与 e2e_from_zero 同口径);ubuntu/CI 可跑。perf 本地跑为主。
退出码:0 通过 / 2 失败。
"""
from __future__ import annotations

import argparse
import json
import math
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
FPS = 30
TRACKS = [("V1", "video"), ("V2", "video"), ("V3", "video"), ("V4", "video"),
          ("A1", "audio"), ("A2", "audio"), ("A3", "audio"), ("A4", "audio")]
CLIPS_PER_TRACK = 125
CLIP_MS = 5000            # 每轨 625s:时间线远长于视口窗口,虚拟化真正生效
MEDIA_S = 10
SWEEP_STEP_DEFAULT = 64   # px/帧;≈560 帧扫完 ~36k px 内容宽

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[tl-perf] {msg}", flush=True)


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def rpc(port: int, token: str, name: str, args: dict, timeout: float = 30) -> dict:
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": name, "arguments": args}}).encode()
    last: Exception | None = None
    for _ in range(3):
        try:
            req = urllib.request.Request(
                f"http://127.0.0.1:{port}/rpc", data=body,
                headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"})
            out = json.loads(urllib.request.urlopen(req, timeout=timeout).read())
            return json.loads(out["result"]["content"][0]["text"])
        except ConnectionResetError as e:
            last = e
            time.sleep(0.4)
    raise last  # type: ignore[misc]


def locate_bin(args_bin: str | None, names: tuple[str, ...]) -> Path | None:
    if args_bin:
        p = Path(args_bin)
        return p if p.is_file() else None
    for name in names:
        for prof in ("debug", "release"):
            for cand in (REPO / "target" / prof / f"{name}.exe", REPO / "target" / prof / name):
                if cand.is_file():
                    return cand
    return None


def sh(cmd: list[str], timeout: float = 600) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True,
                          encoding="utf-8", errors="replace", timeout=timeout)


def layout_rel(proj: Path) -> tuple[str, str]:
    if (proj / "05_时间线工程").is_dir():
        return "05_时间线工程", "01_原始素材"
    return "05_ir", "01_materials"


def project_value_1k(mat_rel: str) -> dict:
    tracks = []
    for tid, kind in TRACKS:
        clips = [{"id": f"{tid}-{i + 1:03d}", "src": f"{mat_rel}/synth.mp4",
                  "startMs": i * CLIP_MS, "durationMs": CLIP_MS}
                 for i in range(CLIPS_PER_TRACK)]
        tracks.append({"id": tid, "kind": kind, "name": tid, "clips": clips})
    return {"version": 1, "schemaVersion": "2.0.0", "slug": "perf-1k",
            "fps": FPS, "canvas": {"width": 1080, "height": 1920}, "tracks": tracks}


def p95(xs: list[float]) -> float:
    ys = sorted(xs)
    return ys[max(0, math.ceil(0.95 * len(ys)) - 1)]


def pct(xs: list[float], q: float) -> float:
    ys = sorted(xs)
    return ys[max(0, math.ceil(q * len(ys)) - 1)]


def fps_stats(frames: list[float]) -> dict:
    deltas = [b - a for a, b in zip(frames, frames[1:]) if b > a]
    fps = [1000.0 / d for d in deltas]
    wall = (frames[-1] - frames[0]) / 1000 if len(frames) > 1 else 0.0
    return {
        "frames": len(frames),
        "avg": (len(deltas) / wall) if wall > 0 else 0.0,
        "p10": pct(fps, 0.10) if fps else 0.0,
        "p50": pct(fps, 0.50) if fps else 0.0,
        "p95": p95(fps) if fps else 0.0,
        "dropped": sum(1 for d in deltas if d > 25.0),  # >25ms 即 <40fps 的帧
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--min-fps", type=float, default=55.0)
    ap.add_argument("--max-clip-nodes", type=int, default=100)
    ap.add_argument("--sweep-step", type=int, default=SWEEP_STEP_DEFAULT)
    ap.add_argument("--bin", default=None)
    ap.add_argument("--cli", default=None)
    args = ap.parse_args()
    mcp = locate_bin(args.bin, ("cutforge-mcp",))
    cli = locate_bin(args.cli, ("cutforge-cli",))
    if not mcp or not cli:
        print("FAIL: 先 cargo build(cutforge-mcp + cutforge-cli)", file=sys.stderr)
        return 2
    if not shutil.which("ffmpeg"):
        print("FAIL: 本机无 ffmpeg(夹具素材需现场合成,口径同 e2e_from_zero)", file=sys.stderr)
        return 2

    from playwright.sync_api import sync_playwright

    t0 = time.time()
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-tl-perf-"))
    token = "tl-perf-token"
    proj = tmp / "perf-1k"
    r = sh([str(cli), "new", str(proj), "--slug", "perf-1k", "--json", "--fps", str(FPS),
            "--width", "1080", "--height", "1920", "--track", "video,audio"])
    assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"
    tl_rel, mat_rel = layout_rel(proj)
    (proj / mat_rel).mkdir(parents=True, exist_ok=True)
    rr = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
             "-i", "testsrc2=size=320x180:rate=30", "-f", "lavfi",
             "-i", "sine=frequency=440:duration=10",
             "-t", str(MEDIA_S), "-pix_fmt", "yuv420p",
             "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
             str(proj / mat_rel / "synth.mp4")])
    assert rr.returncode == 0, f"ffmpeg 生成夹具素材失败:{rr.stderr[-300:]}"
    pj = project_value_1k(mat_rel)
    assert sum(len(t["clips"]) for t in pj["tracks"]) == 1000
    (proj / tl_rel / "project.json").write_text(json.dumps(pj, ensure_ascii=False), encoding="utf-8")

    port = free_port()
    serve = subprocess.Popen(
        [str(mcp), "serve", "--root", str(proj), "--port", str(port),
         "--token", token, "--web", str(REPO / "apps" / "web")],
        stdout=subprocess.DEVNULL, stderr=open(tmp / "serve.log", "wb"))
    try:
        deadline = time.time() + 15
        ready = False
        while time.time() < deadline:
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{port}/session?token={token}", timeout=1).read()
                ready = True
                break
            except Exception:
                time.sleep(0.15)
        assert ready, "serve 未就绪"
        root = str(proj)

        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            page = browser.new_page(viewport={"width": 1600, "height": 900})
            pageerrors: list[str] = []
            page.on("pageerror", lambda e: pageerrors.append(str(e)))
            page.goto(f"http://127.0.0.1:{port}/?token={token}")
            page.wait_for_function(
                "document.querySelector('[data-testid=\"rev\"]').textContent !== '-'", timeout=30000)

            # ---- 1. 服务端夹具回读 + 首窗虚拟化 ----
            tl = rpc(port, token, "timeline_get", {"root": root})
            assert tl["ok"] and len(tl["data"]["clips"]) == 1000, \
                f"服务端应回读 1000 clips,实得 {len(tl['data']['clips'])}"
            lanes = page.locator('[data-testid^="track-lane-"]').count()
            assert lanes == 8, f"轨道行应 8 条,实得 {lanes}"
            nodes_head = page.locator('[data-testid="clip"]').count()
            assert nodes_head <= args.max_clip_nodes, \
                f"AC-2.4 虚拟化失败:首窗 clip 节点 {nodes_head} > {args.max_clip_nodes}(共 1000)"
            log(f"1. 夹具 1,000 clips / 8 轨;首窗虚拟化:DOM clip 节点 {nodes_head} ≤ {args.max_clip_nodes}: PASS")

            # 媒体池就绪后采样(避免首帧解码抖动污染口径)
            page.wait_for_function(
                """() => { const m = [...document.querySelectorAll('[data-testid="pv-media"] video')];
                           return m.length >= 4 && m.some((e) => e.readyState >= 2); }""", timeout=20000)
            page.wait_for_timeout(600)

            # ---- 2. rAF 采样器 + 滚动横扫全程 ----
            page.evaluate("""() => {
                window.__frames = [];
                window.__framesOn = true;
                const loop = (t) => { if (!window.__framesOn) return; window.__frames.push(t); requestAnimationFrame(loop); };
                requestAnimationFrame(loop);
            }""")
            page.evaluate("""(step) => {
                const wrap = document.querySelector('[data-testid="timeline-wrap"]');
                const tick = () => {
                    if (!window.__framesOn) return;
                    const maxSl = wrap.scrollWidth - wrap.clientWidth;
                    if (wrap.scrollLeft >= maxSl - 1) { window.__sweepDone = true; return; }
                    wrap.scrollLeft = Math.min(wrap.scrollLeft + step, maxSl);
                    requestAnimationFrame(tick);
                };
                window.__sweepDone = false;
                requestAnimationFrame(tick);
            }""", args.sweep_step)
            page.wait_for_function("window.__sweepDone === true", timeout=180000)
            sweep_max_sl = page.evaluate(
                "() => { const w = document.querySelector('[data-testid=\"timeline-wrap\"]'); return w.scrollLeft; }")
            nodes_tail = page.locator('[data-testid="clip"]').count()
            assert nodes_tail <= args.max_clip_nodes, \
                f"AC-2.4 虚拟化失败:远端窗 clip 节点 {nodes_tail} > {args.max_clip_nodes}"
            idx_after_sweep = page.evaluate("window.__frames.length")
            log(f"2. 滚动横扫全程(scrollLeft 0→{sweep_max_sl}px);远端窗 clip 节点 {nodes_tail} ≤ {args.max_clip_nodes}: PASS")

            # ---- 3. 末段拖拽(ghost 跟手 → clip_move 落账) ----
            last_id = f"V1-{CLIPS_PER_TRACK:03d}"
            page.evaluate(
                "() => { const w = document.querySelector('[data-testid=\"timeline-wrap\"]');"
                "        w.scrollLeft = Math.max(0, 37200 - 700); }")
            page.wait_for_timeout(300)
            box = page.locator(f'[data-testid="clip"][data-id="{last_id}"]').bounding_box()
            assert box, f"未找到 {last_id}(滚动定位失败)"
            rev_before = rpc(port, token, "project_get", {"root": root})["data"]["rev"]
            page.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
            page.mouse.down()
            for i in range(1, 31):
                page.mouse.move(box["x"] + box["width"] / 2 + i * 10,
                                box["y"] + box["height"] / 2)
                time.sleep(0.016)
            page.mouse.up()
            deadline = time.time() + 15
            rev_after = rev_before
            while time.time() < deadline:
                rev_after = rpc(port, token, "project_get", {"root": root})["data"]["rev"]
                if rev_after > rev_before:
                    break
                time.sleep(0.2)
            assert rev_after > rev_before, "拖拽未产生 clip_move(rev 未上涨)"
            disk = rpc(port, token, "timeline_get", {"root": root})["data"]["clips"]
            moved = next(c for c in disk if c["id"] == last_id)
            assert moved["startMs"] >= 620000, f"拖拽落点异常: {moved}"
            idx_after_drag = page.evaluate("window.__frames.length")
            log(f"3. 拖拽 {last_id} +300px → clip_move 落账(rev {rev_before}→{rev_after},"
                f"startMs→{moved['startMs']}): PASS")
            page.evaluate("window.__framesOn = false")
            frames = page.evaluate("window.__frames")

            # ---- 4. 帧率统计与判定 ----
            sweep_st = fps_stats(frames[:idx_after_sweep])
            drag_st = fps_stats(frames[idx_after_sweep:idx_after_drag])
            all_st = fps_stats(frames)
            assert all_st["p95"] >= args.min_fps, \
                (f"AC-2.4 帧率失败:P95 {all_st['p95']:.1f} < {args.min_fps}"
                 f"(sweep P95 {sweep_st['p95']:.1f},drag P95 {drag_st['p95']:.1f},"
                 f"掉帧 {all_st['dropped']})")
            assert not pageerrors, f"页面 JS 异常: {pageerrors[:5]}"
            browser.close()

        print("AC-2.4 时间线性能: PASS")
        print(f"  虚拟化:1k clips → DOM clip 节点 首 {nodes_head} / 远端 {nodes_tail}"
              f"(阈值 ≤{args.max_clip_nodes})")
        print(f"  滚动横扫: {sweep_st['frames']} 帧,avg {sweep_st['avg']:.1f}fps,"
              f"P50 {sweep_st['p50']:.1f},P95 {sweep_st['p95']:.1f},掉帧 {sweep_st['dropped']}")
        print(f"  拖拽: {drag_st['frames']} 帧,avg {drag_st['avg']:.1f}fps,"
              f"P50 {drag_st['p50']:.1f},P95 {drag_st['p95']:.1f},掉帧 {drag_st['dropped']}")
        print(f"  总判定:P95 {all_st['p95']:.1f}fps ≥ --min-fps {args.min_fps}")
        print(f"e2e_perf_timeline: 全部 PASS(耗时 {time.time() - t0:.1f}s)")
        return 0
    finally:
        serve.terminate()
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except AssertionError as e:
        print(f"FAIL: {e}", file=sys.stderr)
        sys.exit(2)
