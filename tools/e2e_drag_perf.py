#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册三 AC-3.3 拖拽手感 e2e(e2e_drag_perf):帧率 + ghost 跟手 + 零 Op/松手单 Op + Esc 取消。

    python tools/e2e_drag_perf.py [--min-fps 55] [--follow-max-frames 2]
                                  [--bin cutforge-mcp] [--cli cutforge-cli]

断言链:
  1. 拖拽帧率:页面内 rAF 计数采样,整段 clip 拖拽(ghost 跟手 → clip_move 落账),
     帧间隔 → fps 序列,P95 ≥ --min-fps(默认 55;机器负载敏感,安静时段复跑口径同
     e2e_perf_timeline);
  2. ghost 实时跟手(「松手才动」阴性证明):拖拽中逐次 mouse.move 后在 ≤2 帧内采样
     ghost 位置,断言每次 move 后 ghost left 均已更新且随指针单调推进——位移在手势期
     可见,不经「提交才见位移」;同时断言拖拽全程服务端 rev/OpLog 零变化(拖拽零 Op);
  3. 非法落点红态:video 片段拖到 audio 轨 → ghost 挂 .invalid + 目标轨挂 .drop-invalid
     (松手不产生任何 Op);
  4. 松手单 Op:mouse.up 后 rev 恰好 +1,clip_move 落账到候选位置,ghost 消失;
  5. Esc 取消:移动拖拽中按 Esc → rev/OpLog 零变化 + ghost 消失;trim 拖拽中按 Esc →
     片段内联几何(left/width)复位回投影值(trim-preview 类摘除)。

断言纪律(M10-R5):UI 动作只作驱动,断言以服务端状态(rev/oplog)为准;
ghost/帧率属「会话手势表现」口径,以 DOM/rAF 实测为准。
跨平台:pathlib;二进制定位带无 .exe 回退;只依赖 playwright + stdlib(+ ffmpeg
现场造夹具素材,与 e2e_from_zero 同口径)。退出码:0 通过 / 2 失败。
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
PX_PER_MS = 0.06      # 壳显示映射红线(apps/web/js/core/model.js)
FPS = 30              # 夹具工程帧率(帧磁吸网格 = 33.33ms)
CLIP_MS = 10000       # 每片段 10s
MEDIA_S = 40

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[drag-perf] {msg}", flush=True)


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def rpc(port: int, token: str, name: str, args: dict, timeout: float = 10) -> dict:
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
            time.sleep(0.3)
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


def make_media(ws: Path, name: str, seconds: int, size: str = "640x360") -> None:
    r = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
            "-i", f"testsrc2=size={size}:rate=30", "-f", "lavfi",
            f"-i", f"sine=frequency=440:duration={seconds}",
            "-t", str(seconds), "-pix_fmt", "yuv420p",
            "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
            str(ws / name)])
    assert r.returncode == 0 and (ws / name).is_file(), f"ffmpeg 生成素材失败:{r.stderr[-300:]}"


def server_rev(port: int, token: str, root: str) -> int:
    return rpc(port, token, "project_get", {"root": root})["data"]["rev"]


def oplog_count(port: int, token: str, root: str) -> int:
    return rpc(port, token, "oplog_tail", {"root": root, "limit": 1})["data"]["count"]


def assert_ledger_stable(port: int, token: str, root: str, rev: int, ops: int, tag: str) -> None:
    assert server_rev(port, token, root) == rev, f"{tag}: rev 不应变化"
    assert oplog_count(port, token, root) == ops, f"{tag}: OpLog 不应变化"


def wait_ledger_grow(port: int, token: str, root: str, rev: int, timeout_s: float = 15.0) -> int:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        r = server_rev(port, token, root)
        if r > rev:
            return r
        time.sleep(0.15)
    raise AssertionError(f"rev 在 {timeout_s}s 内未从 {rev} 上涨")


def p95_fps(frames: list[float]) -> dict:
    deltas = [b - a for a, b in zip(frames, frames[1:]) if b > a]
    fps = sorted(1000.0 / d for d in deltas)
    return {
        "frames": len(frames),
        "p50": fps[max(0, math.ceil(0.50 * len(fps)) - 1)] if fps else 0.0,
        "p95": fps[max(0, math.ceil(0.95 * len(fps)) - 1)] if fps else 0.0,
        "dropped": sum(1 for d in deltas if d > 25.0),
    }


# ---- 页面内采样器(与 e2e_perf_timeline 同口径)----
RAF_SAMPLER = """() => {
    window.__frames = [];
    window.__framesOn = true;
    const loop = (t) => { if (!window.__framesOn) return; window.__frames.push(t); requestAnimationFrame(loop); };
    requestAnimationFrame(loop);
}"""

# ghost 跟手采样:move 后从「下一帧」起计 rAF 帧数,直到 ghost left 偏离 prev
# (或已到 maxFrames 仍不动)。prev 由驱动侧在 move 前读取,计时不会漏过更新帧;
# 「松手才动」的反例实现会在整个手势期保持 left 不变,这里逐 move 即刻失败。
GHOST_FOLLOW_PROBE = """([prevLeft, maxFrames]) => new Promise((resolve) => {
    const ghost = document.querySelector('[data-testid="drag-ghost"]');
    if (!ghost) { resolve({ ok: false, reason: "ghost 不在手势期", frames: -1, left: null }); return; }
    const leftOf = () => Math.round(parseFloat(ghost.style.left) || 0);
    let frames = 0;
    const tick = () => {
        frames += 1;
        const now = leftOf();
        if (now !== prevLeft || frames >= maxFrames) {
            resolve({ ok: now !== prevLeft, reason: now !== prevLeft ? "followed" : "ghost 未随 move 更新",
                      frames, left: now });
            return;
        }
        requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
})"""


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--min-fps", type=float, default=55.0)
    ap.add_argument("--follow-max-frames", type=int, default=2,
                    help="每次 move 后 ghost 位置更新允许的最大帧数(≤2 帧口径)")
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
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-drag-perf-"))
    token = "drag-perf-token"
    proj = tmp / "drag-proj"
    r = sh([str(cli), "new", str(proj), "--slug", "drag-proj", "--json", "--fps", str(FPS),
            "--width", "1080", "--height", "1920", "--track", "video,audio"])
    assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"
    tl_rel, mat_rel = layout_rel(proj)
    (proj / mat_rel).mkdir(parents=True, exist_ok=True)
    make_media(proj / mat_rel, "main.mp4", MEDIA_S)
    # V1 两段(拖 V1-002 右移无右邻 → 内核不拒重叠),A1 留空(供非法落点红态)
    pj = {"version": 1, "schemaVersion": "2.0.0", "slug": "drag-proj", "fps": FPS,
          "canvas": {"width": 1080, "height": 1920},
          "tracks": [
              {"id": "V1", "kind": "video", "name": "V1",
               "clips": [{"id": f"V1-{i:03d}", "src": f"{mat_rel}/main.mp4",
                          "startMs": (i - 1) * CLIP_MS, "durationMs": CLIP_MS}
                         for i in (1, 2)]},
              {"id": "A1", "kind": "audio", "name": "A1", "clips": []},
          ]}
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
            browser = pw.chromium.launch(args=["--autoplay-policy=no-user-gesture-required"])
            page = browser.new_page(viewport={"width": 1600, "height": 1000})
            pageerrors: list[str] = []
            page.on("pageerror", lambda e: pageerrors.append(str(e)))
            page.goto(f"http://127.0.0.1:{port}/?token={token}")
            page.wait_for_function(
                "document.querySelector('[data-testid=\"rev\"]').textContent !== '-'", timeout=20000)
            # 媒体池就绪后再采样(避免首帧解码抖动污染帧率口径,同 e2e_perf_timeline)
            page.wait_for_function(
                """() => { const m = [...document.querySelectorAll('[data-testid="pv-media"] video')];
                           return m.length >= 1 && m.some((e) => e.readyState >= 2); }""", timeout=20000)
            page.wait_for_timeout(600)

            clip_loc = f'[data-testid="clip"][data-id="V1-002"]'
            box = page.locator(clip_loc).bounding_box()
            assert box, "未找到 V1-002 片段节点"
            cx, cy = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
            rev0, ops0 = server_rev(port, token, root), oplog_count(port, token, root)

            # ============ 1. ghost 实时跟手(「松手才动」阴性证明)+ 拖拽零 Op ============
            page.evaluate(RAF_SAMPLER)
            page.mouse.move(cx, cy)
            page.mouse.down()
            page.mouse.move(cx + 12, cy)  # 过 3px 阈值,手势启动
            page.wait_for_selector('[data-testid="drag-ghost"]', timeout=3000)
            start_left = prev_left = page.evaluate(
                """() => { const g = document.querySelector('[data-testid="drag-ghost"]');
                           return Math.round(parseFloat(g.style.left) || 0); }""")
            samples = []
            for i in range(1, 13):
                page.mouse.move(cx + 12 + i * 10, cy, steps=1)
                probe = page.evaluate(GHOST_FOLLOW_PROBE, [prev_left, args.follow_max_frames])
                assert probe["ok"], (
                    f"AC-3.3 阴性证明失败:第 {i} 次 move 后 ghost 未在 {args.follow_max_frames} 帧内"
                    f"更新位置({probe})——位移不得等提交才可见")
                samples.append(probe)
                prev_left = probe["left"]
            max_frames_used = max(s["frames"] for s in samples)
            lefts = [s["left"] for s in samples]
            assert lefts == sorted(lefts) and len(set(lefts)) == len(lefts), \
                f"ghost 位置应随指针单调推进: {lefts}"
            # 末次 move 后指针在 cx+132:dMs=132/0.06=2200ms → 候选 10000+2200(帧网格整除)→ left=732px
            expect_px = round((CLIP_MS + 132 / PX_PER_MS) * PX_PER_MS)
            assert abs(lefts[-1] - expect_px) <= 3, \
                f"ghost 末位 {lefts[-1]}px 与指针映射 {expect_px}px 偏差过大(跟手失真)"
            assert_ledger_stable(port, token, root, rev0, ops0, "拖拽中")
            log(f"1. ghost 实时跟手:12 次 move 全部 ≤{max_frames_used} 帧内更新"
                f"(left {start_left}→{lefts[-1]}px,指针映射 {expect_px}px);"
                f"拖拽全程 rev/OpLog 零变化({rev0}/{ops0}): PASS")

            # ============ 2. 非法落点红态(video → audio 轨) ============
            a1 = page.locator('[data-testid="track-lane-A1"]').bounding_box()
            assert a1, "未找到 A1 轨道"
            page.mouse.move(a1["x"] + 200, a1["y"] + a1["height"] / 2)
            page.wait_for_timeout(120)
            assert page.evaluate(
                """() => document.querySelector('[data-testid="drag-ghost"]')?.classList.contains('invalid')"""), \
                "video 片段拖到 audio 轨:ghost 应挂 .invalid"
            assert page.evaluate(
                """() => document.querySelector('[data-testid="track-lane-A1"]')?.classList.contains('drop-invalid')"""), \
                "非法落点:A1 轨应挂 .drop-invalid"
            assert_ledger_stable(port, token, root, rev0, ops0, "非法落点悬停")
            # 拖回合法区,红态应解除
            page.mouse.move(cx + 150, cy)
            page.wait_for_timeout(120)
            assert not page.evaluate(
                """() => document.querySelector('[data-testid="drag-ghost"]')?.classList.contains('invalid')"""), \
                "拖回 video 轨后 ghost 红态应解除"
            log("2. 非法落点红态:video→A1 ghost.invalid + track.drop-invalid,拖回后解除,零 Op: PASS")

            # ============ 3. 松手单 Op(clip_move 落账) ============
            idx_pre_commit = page.evaluate("window.__frames.length")
            page.mouse.up()
            rev1 = wait_ledger_grow(port, token, root, rev0)
            assert oplog_count(port, token, root) == ops0 + 1, "松手应恰好产生 1 个 Op"
            disk = rpc(port, token, "timeline_get", {"root": root})["data"]["clips"]
            moved = next(c for c in disk if c["id"] == "V1-002")
            # 拖回合法区后指针在 cx+150:dMs=150/0.06=2500ms → 候选 12500ms(恰在帧网格)
            expect_start = CLIP_MS + round(150 / PX_PER_MS)
            assert moved["startMs"] != CLIP_MS and abs(moved["startMs"] - expect_start) <= 100, \
                f"clip_move 落点异常: {moved}(期望 ≈{expect_start})"
            assert page.locator('[data-testid="drag-ghost"]').count() == 0, "松手后 ghost 应消失"
            assert page.locator(f'[data-testid="clip"][data-id="V1-002"]').count() == 1, "本体节点应重绘在位"
            log(f"3. 松手单 Op:rev {rev0}→{rev1},V1-002 startMs→{moved['startMs']},ghost 消失: PASS")

            # ============ 4. 拖拽帧率(P95 ≥ --min-fps,含跟手段+落点段) ============
            page.evaluate("window.__framesOn = false")
            frames = page.evaluate("window.__frames")
            st = p95_fps(frames)
            assert st["p95"] >= args.min_fps, \
                (f"AC-3.3 帧率失败:P95 {st['p95']:.1f} < {args.min_fps}"
                 f"(P50 {st['p50']:.1f},掉帧 {st['dropped']}/{st['frames']})")
            log(f"4. 拖拽帧率:{st['frames']} 帧,P50 {st['p50']:.1f},P95 {st['p95']:.1f} ≥ {args.min_fps},"
                f"掉帧 {st['dropped']}: PASS")
            fps_stat = st

            # ============ 5. Esc 取消(移动:零 Op + ghost 消失) ============
            page.evaluate(RAF_SAMPLER)
            rev2, ops1 = server_rev(port, token, root), oplog_count(port, token, root)
            box = page.locator(clip_loc).bounding_box()
            cx, cy = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
            page.mouse.move(cx, cy)
            page.mouse.down()
            page.mouse.move(cx + 60, cy)
            page.wait_for_selector('[data-testid="drag-ghost"]', timeout=3000)
            page.keyboard.press("Escape")
            page.wait_for_timeout(200)
            assert page.locator('[data-testid="drag-ghost"]').count() == 0, "Esc 后 ghost 应消失"
            assert_ledger_stable(port, token, root, rev2, ops1, "Esc 取消(移动)")
            assert page.evaluate(
                """() => !document.querySelector('.track.drop-target, .track.drop-invalid')"""), \
                "Esc 后轨道高亮应清除"
            page.evaluate("window.__framesOn = false")
            log(f"5. Esc 取消(移动):ghost 消失 + 轨高亮清除 + rev/OpLog 零变化({rev2}/{ops1}): PASS")

            # ============ 6. Esc 取消(trim:内联几何复位) ============
            rev3, ops2 = server_rev(port, token, root), oplog_count(port, token, root)
            box = page.locator(clip_loc).bounding_box()
            edge = page.locator(f'{clip_loc} .edge-r').bounding_box()
            assert edge, "右 trim 把手不可达(先选中片段使其显形)"
            ex, ey = edge["x"] + edge["width"] / 2, edge["y"] + edge["height"] / 2
            page.mouse.move(ex, ey)
            page.mouse.down()
            page.mouse.move(ex - 80, ey)
            page.wait_for_timeout(150)
            trimmed = page.evaluate(
                """(sel) => { const el = document.querySelector(sel);
                              return { left: el.style.left, width: el.style.width,
                                       trim: el.classList.contains('trim-preview') }; }""", clip_loc)
            assert trimmed["trim"], "trim 手势期片段应挂 .trim-preview"
            page.keyboard.press("Escape")
            page.wait_for_timeout(200)
            restored = page.evaluate(
                """([sel, pxPerMs]) => { const el = document.querySelector(sel);
                   const row = null;
                   return { left: el.style.left, width: el.style.width,
                            trim: el.classList.contains('trim-preview') }; }""", [clip_loc, PX_PER_MS])
            assert not restored["trim"], "Esc 后 .trim-preview 应摘除"
            disk = rpc(port, token, "timeline_get", {"root": root})["data"]["clips"]
            cur = next(c for c in disk if c["id"] == "V1-002")
            expect_left = round(cur["startMs"] * PX_PER_MS)
            expect_width = max(6, round((cur["startMs"] + cur["durationMs"]) * PX_PER_MS)) - expect_left
            got_left = round(float(str(restored["left"]).replace("px", "")))
            got_width = round(float(str(restored["width"]).replace("px", "")))
            assert got_left == expect_left and got_width == expect_width, \
                (f"Esc 后几何未复位:left {got_left}≠{expect_left} 或 width {got_width}≠{expect_width}"
                 f"(投影 startMs={cur['startMs']} dur={cur['durationMs']})")
            assert_ledger_stable(port, token, root, rev3, ops2, "Esc 取消(trim)")
            page.evaluate("window.__framesOn = false")
            log(f"6. Esc 取消(trim):内联几何复位 left={got_left}/width={got_width}(=投影值),"
                f"rev/OpLog 零变化({rev3}/{ops2}): PASS")

            assert not pageerrors, f"页面 JS 异常: {pageerrors[:5]}"
            browser.close()

        print("AC-3.3 拖拽手感: PASS")
        print(f"  ghost 跟手:12/12 次 move ≤{max_frames_used} 帧更新(阈值 ≤{args.follow_max_frames});"
              f"拖拽零 Op;松手单 Op(rev {rev0}→{rev1})")
        print(f"  拖拽帧率:P95 {fps_stat['p95']:.1f}fps ≥ {args.min_fps}(P50 {fps_stat['p50']:.1f},"
              f"掉帧 {fps_stat['dropped']}/{fps_stat['frames']})")
        print(f"  Esc 取消:移动零 Op + trim 几何复位")
        print(f"e2e_drag_perf: 全部 PASS(耗时 {time.time() - t0:.1f}s)")
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
