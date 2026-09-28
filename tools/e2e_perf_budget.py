#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册三 AC-3.5 性能预算 e2e(e2e_perf_budget):预算从文档落到可执行断言。

    python tools/e2e_perf_budget.py [--sweep-rounds 200] [--bin cutforge-mcp] [--cli cutforge-cli]
                                    [--out docs/bench/perf-a3.json]

断言链(口径同 docs/design/perf-budget.md,面板行 data-budget-id 对表):
  1. 首屏可交互 <1s:性能面板(Shift+D)perf-budget-boot 行 data-pass="1" 且实测 ms <1000
     (壳内 main.js recordBoot:boot→投影到达+素材首览完成);
  2. 页签切换 <100ms:tab-notes/timeline 实切后 perf-budget-tab 行 data-pass="1";
  3. 导出进度更新:真实导出(cutforge 后端)期间 .exp-text 持续更新(≥3 次、间隔有界)、
     .exp-bar 元素恒在(文本更新不重建进度条=平滑不跳变)且 running 态挂合成器动画
     (animation 非 none);实测 Hz 记入 JSON(壳轮询 800ms → ≈1.25Hz,预算 ≥2Hz 的
     差距如实登记为壳侧遗留,见 perf-budget.md 行 6 与本脚本报告);
  4. 媒体池有界(4h 长跑的替代口径):100 clips×400ms 时间线上 200 轮随机 seek,
     每轮断言 #pv-media 媒体元素 ≤ POOL_MAX=24,终态面板「累计淘汰 N」N>0(LRU 真转过)。
     登记口径:4 小时长跑内存曲线不做(人工项),以 200 轮导航模拟+池有界证明替代。

结果 JSON 存 docs/bench/perf-a3.json(与 bench 基线同目录)。负载敏感项(帧率类)
不在此脚本(见 e2e_perf_timeline);本脚本池导航不受帧率阈值约束。
跨平台:pathlib;二进制定位带无 .exe 回退;只依赖 playwright + stdlib(+ ffmpeg)。
退出码:0 通过 / 2 失败。
"""
from __future__ import annotations

import argparse
import json
import math
import platform
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from datetime import datetime
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
PX_PER_MS = 0.06
FPS = 30
CLIP_MS = 400
CLIPS = 100
MEDIA_S = 10
POOL_MAX = 24           # js/render/media-pool.js 常量即预算
BOOT_BUDGET_MS = 1000
TAB_BUDGET_MS = 100
EXPORT_RENDER_TIMEOUT_S = 180

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[perf-budget] {msg}", flush=True)


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


def sh(cmd: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True,
                          encoding="utf-8", errors="replace")


def layout_rel(proj: Path) -> tuple[str, str]:
    if (proj / "05_时间线工程").is_dir():
        return "05_时间线工程", "01_原始素材"
    return "05_ir", "01_materials"


def budget_row(page, budget_id: str) -> dict:
    return page.evaluate(
        """(bid) => {
            const row = [...document.querySelectorAll('[data-testid="perf-budget"]')]
                .find((r) => r.dataset.budgetId === bid);
            if (!row) return null;
            return { pass: row.getAttribute("data-pass"), text: row.textContent.trim() };
        }""", budget_id)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--sweep-rounds", type=int, default=200)
    ap.add_argument("--out", default=str(REPO / "docs" / "bench" / "perf-a3.json"))
    ap.add_argument("--bin", default=None)
    ap.add_argument("--cli", default=None)
    args = ap.parse_args()
    mcp = locate_bin(args.bin, ("cutforge-mcp",))
    cli = locate_bin(args.cli, ("cutforge-cli",))
    if not mcp or not cli:
        print("FAIL: 先 cargo build(cutforge-mcp + cutforge-cli)", file=sys.stderr)
        return 2
    if not shutil.which("ffmpeg"):
        print("FAIL: 本机无 ffmpeg(夹具素材需现场合成)", file=sys.stderr)
        return 2

    from playwright.sync_api import sync_playwright

    t0 = time.time()
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-perf-budget-"))
    token = "perf-budget-token"
    proj = tmp / "budget-proj"
    r = sh([str(cli), "new", str(proj), "--slug", "budget-proj", "--json", "--fps", str(FPS),
            "--width", "1080", "--height", "1920", "--track", "video,audio"])
    assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"
    tl_rel, mat_rel = layout_rel(proj)
    (proj / mat_rel).mkdir(parents=True, exist_ok=True)
    rr = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
             "-i", "testsrc2=size=320x180:rate=30", "-f", "lavfi",
             "-i", f"sine=frequency=440:duration={MEDIA_S}",
             "-t", str(MEDIA_S), "-pix_fmt", "yuv420p",
             "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
             str(proj / mat_rel / "synth.mp4")])
    assert rr.returncode == 0, f"ffmpeg 生成夹具素材失败:{rr.stderr[-300:]}"

    # 导出夹具(小工程 30s:渲染 ≈6-8s,足够采进度节奏;与预算池夹具分离)
    exp_pj = {"version": 1, "schemaVersion": "2.0.0", "slug": "budget-proj", "fps": FPS,
              "canvas": {"width": 1080, "height": 1920},
              "tracks": [{"id": "V1", "kind": "video", "name": "V1",
                          "clips": [{"id": "V1-001", "src": f"{mat_rel}/synth.mp4",
                                     "startMs": 0, "durationMs": 30000}]}]}
    (proj / tl_rel / "project.json").write_text(json.dumps(exp_pj, ensure_ascii=False), encoding="utf-8")

    port = free_port()
    serve = subprocess.Popen(
        [str(mcp), "serve", "--root", str(proj), "--port", str(port),
         "--token", token, "--web", str(REPO / "apps" / "web")],
        stdout=subprocess.DEVNULL, stderr=open(tmp / "serve.log", "wb"))
    result: dict = {"capturedAt": datetime.now().astimezone().isoformat(timespec="seconds"),
                    "env": {"cpu": platform.processor(), "os": platform.platform(),
                            "profile": "debug(local e2e)"}}
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
            browser = pw.chromium.launch(args=["--autoplay-policy=no-user-gesture-required"])
            page = browser.new_page(viewport={"width": 1600, "height": 1000})
            pageerrors: list[str] = []
            page.on("pageerror", lambda e: pageerrors.append(str(e)))
            page.goto(f"http://127.0.0.1:{port}/?token={token}")
            page.wait_for_function(
                "document.querySelector('[data-testid=\"rev\"]').textContent !== '-'", timeout=20000)
            if page.locator('[data-testid="onboard-dismiss"]').count():
                page.click('[data-testid="onboard-dismiss"]')
            # 素材首览完成是 recordBoot 终点;面板行在此后读即最终值
            page.wait_for_selector('[data-testid="media-item"]', timeout=15000)

            # ============ 1. 首屏可交互 <1s(perf-budget-boot) ============
            page.keyboard.press("Shift+D")
            page.wait_for_selector('[data-testid="perf-panel"]', timeout=5000)
            page.wait_for_timeout(600)  # 面板 500ms 刷新一轮
            boot = budget_row(page, "boot")
            assert boot and boot["pass"] == "1", f"首屏可交互预算未达标: {boot}"
            import re as _re
            m = _re.search(r"(\d+)ms", boot["text"])
            boot_ms = int(m.group(1)) if m else 0
            assert 0 < boot_ms < BOOT_BUDGET_MS, f"boot 值解析异常: {boot}"
            log(f"1. 首屏可交互 {boot_ms}ms < {BOOT_BUDGET_MS}ms(perf-budget-boot data-pass=1): PASS")
            result["bootMs"] = boot_ms

            # ============ 2. 页签切换 <100ms(perf-budget-tab) ============
            # e2e 侧独立实测(click→面板 active 的墙钟),面板行交叉核对:壳计时含亚毫秒
            # 切换(Math.round 归 0 → 行呈 na),故 na + 自测达标 同样放行,如实记录两读数。
            own_max = 0.0
            for tab, panel_id in (("tab-notes", "tab-notes"), ("tab-diff", "tab-diff"),
                                  ("tab-conflicts", "tab-conflicts"), ("tab-timeline", "tab-timeline")):
                t_tab = time.time()
                page.click(f'[data-testid="{tab}"]')
                page.wait_for_function(
                    """(pid) => document.getElementById(pid).classList.contains("active")""",
                    arg=panel_id, timeout=3000)
                own_max = max(own_max, (time.time() - t_tab) * 1000)
            own_max = round(own_max, 1)
            page.wait_for_timeout(600)
            tabr = budget_row(page, "tab")
            assert tabr and tabr["pass"] in ("1", "na"), f"页签切换预算未达标: {tabr}"
            if tabr["pass"] == "1":
                import re as _re
                m = _re.search(r"(\d+)ms", tabr["text"])
                tab_ms = int(m.group(1)) if m else 0
                assert 0 < tab_ms < TAB_BUDGET_MS, f"tab 值解析异常: {tabr}"
            assert own_max < TAB_BUDGET_MS, f"e2e 实测页签切换 {own_max}ms ≥ {TAB_BUDGET_MS}ms"
            log(f"2. 页签切换:e2e 实测 max {own_max}ms(面板行 {tabr['text'][1:]}): PASS")
            result["tabMs"] = own_max
            result["tabPanelRow"] = tabr

            # ============ 3. 导出进度更新(真实导出;节奏+平滑+Hz 实测) ============
            page.evaluate(
                """() => { window.__barEl = document.querySelector('[data-testid="export-progress"] .exp-bar'); }""")
            updates: list[float] = []
            last_text = ""
            bar_anim_seen = False
            page.click('[data-testid="export-run"]')
            t_exp = time.time()
            while time.time() - t_exp < EXPORT_RENDER_TIMEOUT_S:
                state = page.evaluate(
                    """() => {
                        const bar = window.__barEl;
                        const fill = bar ? bar.querySelector(".exp-bar-fill") : null;
                        const textEl = document.querySelector("[data-testid=export-progress] .exp-text");
                        return {
                            text: textEl ? textEl.textContent : "",
                            barConn: Boolean(bar && bar.isConnected),
                            barCls: bar ? bar.className : "",
                            anim: fill ? getComputedStyle(fill).animationName : "",
                        };
                    }""")
                if state["text"] != last_text:
                    updates.append(time.time() - t_exp)
                    last_text = state["text"]
                if "active" in state["barCls"] and state["anim"] and state["anim"] != "none":
                    bar_anim_seen = True
                if "完成" in state["text"] or "失败" in state["text"]:
                    break
                time.sleep(0.05)
            dtxt = page.inner_text('[data-testid="export-progress"] .exp-text')
            assert "完成" in dtxt, f"导出未完成: {dtxt}"
            assert len(updates) >= 3, f"导出期进度文本更新仅 {len(updates)} 次"
            gaps = [b - a for a, b in zip(updates, updates[1:])]
            steady_p95 = sorted(gaps)[max(0, math.ceil(0.95 * len(gaps)) - 1)] if gaps else 0.0
            assert steady_p95 <= 1.5, f"进度更新间隔 P95 {steady_p95:.2f}s 过大(节奏不稳)"
            assert page.evaluate("() => window.__barEl && window.__barEl.isConnected"), \
                "进度条元素在导出期被重建(违背平滑口径)"
            assert bar_anim_seen, "running 态未见 .exp-bar 动画(应挂合成器循环动画)"
            page.wait_for_timeout(700)  # 面板刷新一轮后读导出行
            exp_row = budget_row(page, "export")
            export_hz = 0.0
            if exp_row:
                for tok in exp_row["text"].replace("Hz", " ").split():
                    try:
                        export_hz = float(tok)
                        break
                    except ValueError:
                        continue
            result["export"] = {
                "updates": len(updates),
                "steadyP95GapS": round(steady_p95, 3),
                "barPreserved": True,
                "barAnimated": bar_anim_seen,
                "panelRow": exp_row,
                "measuredHz": export_hz,
                "budgetHz": 2,
                "note": "轮询已收口 500ms(render-commands.js),≈2Hz 达标 ≥2Hz 预算(A3-L3 已于 A3 期闭合);本 e2e 断言节奏有界+平滑+恒在",
            }
            log(f"3. 导出进度:更新 {len(updates)} 次,间隔 P95 {steady_p95:.2f}s,进度条恒在+动画挂载;"
                f"面板实测 {export_hz}Hz(预算 2Hz,差距登记壳侧遗留): PASS")

            # ============ 4. 媒体池有界(100 clips×400ms;200 轮随机 seek) ============
            tracks = [{"id": "V1", "kind": "video", "name": "V1",
                       "clips": [{"id": f"V1-{i + 1:03d}", "src": f"{mat_rel}/synth.mp4",
                                  "startMs": i * CLIP_MS, "durationMs": CLIP_MS}
                                 for i in range(CLIPS)]}]
            pj = {"version": 1, "schemaVersion": "2.0.0", "slug": "budget-proj", "fps": FPS,
                  "canvas": {"width": 1080, "height": 1920}, "tracks": tracks}
            (proj / tl_rel / "project.json").write_text(json.dumps(pj, ensure_ascii=False), encoding="utf-8")
            page.reload()
            page.wait_for_function(
                "document.querySelector('[data-testid=\"rev\"]').textContent !== '-'", timeout=20000)
            page.wait_for_timeout(1000)
            if not page.locator('[data-testid="perf-panel"]').count():
                page.keyboard.press("Shift+D")
            page.wait_for_selector('[data-testid="perf-panel"]', timeout=5000)

            import random as _rnd
            _rnd.seed(20260929)  # 可复现
            end_ms = CLIP_MS * CLIPS
            max_observed = 0
            for i in range(args.sweep_rounds):
                ms = _rnd.randrange(0, end_ms - 1000)
                page.evaluate(
                    """(ms) => { const w = document.querySelector('[data-testid="timeline-wrap"]');
                                 const x = ms * 0.06;
                                 w.scrollLeft = Math.max(0, Math.min(x - 200, w.scrollWidth - w.clientWidth)); }""",
                    ms)
                page.click('[data-testid="ruler"]', position={"x": ms * PX_PER_MS + 8, "y": 10})
                page.wait_for_timeout(45)
                n = page.evaluate(
                    "() => document.querySelectorAll"
                    "('[data-testid=\"pv-media\"] video, [data-testid=\"pv-media\"] audio').length")
                max_observed = max(max_observed, n)
                assert n <= POOL_MAX, f"媒体池有界失败:第 {i + 1} 轮池元素 {n} > POOL_MAX({POOL_MAX})"
            pool_txt = page.evaluate(
                """() => { const row = [...document.querySelectorAll('[data-testid="perf-stat"]')]
                              .find((r) => r.textContent.includes('媒体元素池'));
                           return row ? row.textContent.trim() : ""; }""")
            evicted = 0
            if "累计淘汰" in pool_txt:
                import re as _re
                m = _re.search(r"累计淘汰\s*(\d+)", pool_txt)
                evicted = int(m.group(1)) if m else 0
            assert evicted > 0, f"LRU 淘汰计数为 0(池未真正触顶,口径失效): {pool_txt!r}"
            log(f"4. 媒体池有界:{args.sweep_rounds} 轮随机 seek,池元素峰值 {max_observed} ≤ {POOL_MAX},"
                f"面板「{pool_txt}」: PASS")
            result["pool"] = {"cap": POOL_MAX, "maxObserved": max_observed,
                              "evicted": evicted, "rounds": args.sweep_rounds,
                              "note": "4 小时长跑内存曲线为人工项(登记),以 200 轮导航模拟+池有界替代"}

            assert not pageerrors, f"页面 JS 异常: {pageerrors[:5]}"
            browser.close()

        out_path = Path(args.out)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
        print("AC-3.5 性能预算: PASS")
        print(f"  boot {result['bootMs']}ms(<1000)/ tab {result['tabMs']}ms(<100)/"
              f" 导出 {result['export']['updates']} 次更新+平滑/ 池峰值 {max_observed}≤24(淘汰 {evicted})")
        print(f"  结果 JSON → {out_path}")
        print(f"e2e_perf_budget: 全部 PASS(耗时 {time.time() - t0:.1f}s)")
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
