#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册二 AC-2.2 播放生存门禁(e2e_playback_survival):播放中被扰动,播放状态与位置零丢失。

    python tools/e2e_playback_survival.py [--bin cutforge-mcp] [--cli cutforge-cli]

场景(播放进行中逐帧采样,全部断言落在采样序列上):
  ① 外部进程直接改 project.json(watcher 驱动 workspace.changed):改主片段 volume +
     追加远处片段(标尺宽度增长 = 事件到达壳的确定性信号);
  ② 壳内编辑一笔(检查器 opacity → clip_update);
  断言(容差见 TOLERANCE 注释):
    - 播放不中断:采样窗口内 playing 标志零翻转;
    - 播放头零回退:壳时钟(#playhead-ms)单调不减;
    - 媒体 currentTime 连续:不归零、无 >0.25s 的回跳、与挂钟推进偏差 ≤0.5s;
    - 媒体元素引用不变(未被销毁重建):打点标记在每一个采样帧都健在;
    - 收尾:SSE 事件(标尺增长)到达后,元素池仍是原引用集合。
退出码:0 通过 / 2 失败。依赖:playwright(chromium)+ ffmpeg + cutforge-cli。
"""
from __future__ import annotations

import argparse
import json
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
PX_PER_MS = 0.06
SEEK_MS = 2000            # 起播位置
MEDIA_S = 60              # 素材时长:播放窗口(~8s)远小于素材
# ---- 容差口径(报告引用)----
# 漂移校正:壳 syncMedia 每 250ms 校正一次,阈值 0.12s(DRIFT_TOL_PLAY)→ 允许的
# 单帧最大回跳 = 0.25s;归零判定线 = 基点 -0.30s;挂钟推进偏差预算 = 0.50s。
BACKJUMP_MAX_S = 0.25
RESET_GUARD_S = 0.30
WALL_DRIFT_S = 0.50

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[pb-survival] {msg}", flush=True)


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


def external_tweak(ws: Path, tl_rel: str, mat_rel: str) -> None:
    """外部进程语义:直接改 project.json(主片段 volume + 追加远处片段作事件信号)。"""
    pj = ws / tl_rel / "project.json"
    doc = json.loads(pj.read_text(encoding="utf-8"))
    v1 = next(t for t in doc["tracks"] if t["id"] == "V1")
    v1["clips"][0]["volume"] = 0.66
    v1["clips"].append({"id": "V1-901", "src": f"{mat_rel}/main.mp4",
                        "startMs": 100000, "durationMs": 10000})
    pj.write_text(json.dumps(doc, ensure_ascii=False), encoding="utf-8")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=None)
    ap.add_argument("--cli", default=None)
    args = ap.parse_args()
    mcp = locate_bin(args.bin, ("cutforge-mcp",))
    cli = locate_bin(args.cli, ("cutforge-cli",))
    if not mcp or not cli:
        print("FAIL: 先 cargo build(cutforge-mcp + cutforge-cli)", file=sys.stderr)
        return 2
    if not shutil.which("ffmpeg"):
        print("FAIL: 本机无 ffmpeg(无法现场生成素材)", file=sys.stderr)
        return 2

    from playwright.sync_api import sync_playwright

    t0 = time.time()
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-pb-survival-"))
    token = "pb-survival-token"
    proj = tmp / "surv-proj"
    r = sh([str(cli), "new", str(proj), "--slug", "surv", "--json", "--fps", "30",
            "--width", "1080", "--height", "1920", "--track", "video,audio"])
    assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"
    tl_rel, mat_rel = layout_rel(proj)
    (proj / mat_rel).mkdir(parents=True, exist_ok=True)
    rr = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
             "-i", "testsrc2=size=640x360:rate=30", "-f", "lavfi",
             "-i", "sine=frequency=440:duration=10",
             "-t", str(MEDIA_S), "-pix_fmt", "yuv420p",
             "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
             str(proj / mat_rel / "main.mp4")])
    assert rr.returncode == 0, f"ffmpeg 生成素材失败:{rr.stderr[-300:]}"

    port = free_port()
    serve = subprocess.Popen(
        [str(mcp), "serve", "--root", str(proj), "--port", str(port),
         "--token", token, "--web", str(REPO / "apps" / "web")],
        stdout=subprocess.DEVNULL, stderr=open(tmp / "serve.log", "wb"))
    try:
        deadline = time.time() + 10
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
        add = rpc(port, token, "clip_add",
                  {"root": root, "trackId": "V1", "src": f"{mat_rel}/main.mp4",
                   "startMs": 0, "durationMs": MEDIA_S * 1000, "requestId": "pb-1"})
        assert add["ok"], f"clip_add: {add}"
        clip_id = rpc(port, token, "timeline_get", {"root": root})["data"]["clips"][0]["id"]
        rev0 = add["data"]["rev"]

        with sync_playwright() as pw:
            browser = pw.chromium.launch(args=["--autoplay-policy=no-user-gesture-required"])
            page = browser.new_page(viewport={"width": 1600, "height": 1000})
            pageerrors: list[str] = []
            page.on("pageerror", lambda e: pageerrors.append(str(e)))
            page.goto(f"http://127.0.0.1:{port}/?token={token}")
            page.wait_for_function(
                "document.querySelector('[data-testid=\"rev\"]').textContent !== '-'", timeout=20000)
            # 媒体元素就绪
            page.wait_for_function(
                """() => [...document.querySelectorAll('[data-testid="pv-media"] video, [data-testid="pv-media"] audio')]
                          .some((e) => e.readyState >= 2)""", timeout=20000)

            # 选中 + 起播位置
            page.click(f'[data-testid="clip"][data-id="{clip_id}"]')
            page.click('[data-testid="ruler"]', position={"x": int(SEEK_MS * PX_PER_MS), "y": 10})
            page.wait_for_timeout(400)

            # 播放
            page.keyboard.press("Space")
            page.wait_for_function(
                "document.querySelector('[data-testid=\"play-toggle\"]').textContent.includes('暂停')",
                timeout=5000)
            page.wait_for_function(
                """() => [...document.querySelectorAll('[data-testid="pv-media"] video, [data-testid="pv-media"] audio')]
                          .some((e) => !e.paused && e.currentTime > 0.5)""", timeout=8000)

            # 装采样器:rAF 逐帧记录(播放头/playing/全部媒体元素打点)
            page.evaluate("""() => {
                const S = {rows: [], on: true};
                window.__surv = S;
                const sel = '[data-testid="pv-media"] video, [data-testid="pv-media"] audio';
                const snap = () => [...document.querySelectorAll(sel)].map((e) => {
                    if (!e.__e2eMark) e.__e2eMark = 'm' + Math.random().toString(36).slice(2, 10);
                    return {mark: e.__e2eMark, ct: e.currentTime, paused: e.paused};
                });
                const loop = () => {
                    if (!S.on) return;
                    S.rows.push({
                        t: performance.now(),
                        ph: Number(document.querySelector('[data-testid="playhead-ms"]').textContent) || 0,
                        playing: document.querySelector('[data-testid="play-toggle"]').textContent.includes('暂停'),
                        media: snap(),
                    });
                    requestAnimationFrame(loop);
                };
                requestAnimationFrame(loop);
            }""")
            t_start = page.evaluate("performance.now()")
            base_marks = page.evaluate(
                """() => [...document.querySelectorAll('[data-testid="pv-media"] video, [data-testid="pv-media"] audio')]
                          .map((e) => { if (!e.__e2eMark) e.__e2eMark = 'm' + Math.random().toString(36).slice(2, 10); return e.__e2eMark; })""")
            base_ct = page.evaluate(
                """() => Math.min(...[...document.querySelectorAll('[data-testid="pv-media"] video, [data-testid="pv-media"] audio')]
                                    .filter((e) => !e.paused).map((e) => e.currentTime))""")

            # ---- ① 外部进程改 project.json(watcher → workspace.changed) ----
            time.sleep(1.0)
            t_ext = page.evaluate("performance.now()")
            external_tweak(proj, tl_rel, mat_rel)
            t_ext_write = page.evaluate("performance.now()")
            # 事件到达信号:追加片段把时间线末端推到 110000ms → 标尺宽度 ≥ 6580px
            want_w = (110000 * PX_PER_MS) - 20
            deadline = time.time() + 10
            while time.time() < deadline:
                if page.evaluate(
                    "() => parseFloat(document.querySelector('[data-testid=\"ruler\"]').style.width) || 0"
                ) >= want_w:
                    break
                time.sleep(0.1)
            t_seen = page.evaluate("performance.now()")
            width_seen = page.evaluate(
                "() => parseFloat(document.querySelector('[data-testid=\"ruler\"]').style.width) || 0")
            assert width_seen >= want_w, \
                f"workspace.changed 未在 10s 内驱动重投影(标尺宽 {width_seen:.0f} < {want_w:.0f})"
            ext_lag_ms = t_seen - t_ext_write

            # ---- ② 壳内编辑一笔(clip_update,播放不停) ----
            time.sleep(1.5)
            t_apply0 = page.evaluate("performance.now()")
            page.fill('[data-testid="field-opacity"]', "0.88")
            page.click('[data-testid="insp-apply"]')
            deadline = time.time() + 15
            rev_now = rev0
            while time.time() < deadline:
                rev_now = rpc(port, token, "project_get", {"root": root})["data"]["rev"]
                if rev_now > rev0:
                    break
                time.sleep(0.15)
            assert rev_now > rev0, "播放中壳内编辑未落账"
            cur = rpc(port, token, "timeline_get", {"root": root})["data"]["clips"][0]
            assert cur.get("opacity") == 0.88, f"播放中编辑未生效: {cur}"
            t_applied = page.evaluate("performance.now()")
            apply_lag_ms = t_applied - t_apply0

            time.sleep(2.0)
            page.evaluate("window.__surv.on = false")
            rows = page.evaluate("window.__surv.rows")
            t_end = page.evaluate("performance.now()")
            page.click('[data-testid="play-toggle"]')  # 暂停收尾(焦点可能停在检查器输入框,不走空格)
            page.wait_for_function(
                "document.querySelector('[data-testid=\"play-toggle\"]').textContent.includes('播放')",
                timeout=5000)

            # ================= 分析(全部在采样序列上) =================
            assert len(rows) > 200, f"采样过稀: {len(rows)} 帧"
            # 1. 播放不中断
            interrupts = [r2 for r2 in rows if not r2["playing"]]
            assert not interrupts, f"播放中断 {len(interrupts)} 帧"
            # 2. 播放头单调
            back_ph = max((rows[i]["ph"] - rows[i + 1]["ph"]) for i in range(len(rows) - 1))
            assert back_ph <= 0, f"播放头回退 {back_ph}ms"
            # 3. 原媒体元素:每个采样帧都健在(引用未销毁重建)+ currentTime 连续
            alive_gap = 0
            back_ct = 0.0
            min_ct = None
            for r2 in rows:
                marks = {m["mark"] for m in r2["media"]}
                alive_gap = max(alive_gap, sum(1 for mk in base_marks if mk not in marks))
                for m in r2["media"]:
                    if m["mark"] == base_marks[0]:
                        if m["paused"]:
                            raise AssertionError("主媒体元素在播放中被暂停")
                        if min_ct is None:
                            min_ct = m["ct"]
                        min_ct = min(min_ct, m["ct"])
            assert alive_gap == 0, "媒体元素在窗口内丢失(疑似销毁重建)"
            cts = [next((m["ct"] for m in r2["media"] if m["mark"] == base_marks[0]), None) for r2 in rows]
            cts = [c for c in cts if c is not None]
            for i in range(len(cts) - 1):
                back_ct = max(back_ct, cts[i] - cts[i + 1])
            assert back_ct <= BACKJUMP_MAX_S, \
                f"currentTime 回跳 {back_ct:.3f}s > {BACKJUMP_MAX_S}s 容差"
            assert min_ct >= base_ct - RESET_GUARD_S, \
                f"currentTime 疑似归零:最小 {min_ct:.3f}s < 基点 {base_ct:.3f}s - {RESET_GUARD_S}s"
            wall = (rows[-1]["t"] - rows[0]["t"]) / 1000
            drift = abs((cts[-1] - cts[0]) - wall)
            assert drift <= WALL_DRIFT_S, \
                f"媒体推进与挂钟偏差 {drift:.3f}s > {WALL_DRIFT_S}s"
            # 4. 收尾:事件到达后元素池仍含全部原引用
            final_marks = page.evaluate(
                """() => [...document.querySelectorAll('[data-testid="pv-media"] video, [data-testid="pv-media"] audio')]
                          .map((e) => e.__e2eMark || null)""")
            missing = [mk for mk in base_marks if mk not in final_marks]
            assert not missing, f"收尾元素池丢原引用: {missing}"

            fps = len(rows) / wall
            print(f"AC-2.2 播放生存: PASS")
            print(f"  采样 {len(rows)} 帧 / {wall:.2f}s(≈{fps:.0f}fps);播放中断 0;"
                  f"播放头回退 {back_ph:.0f}ms")
            print(f"  ① 外部改盘:SSE→重投影时延 {ext_lag_ms:.0f}ms(标尺宽→{width_seen:.0f}px,"
                  f"volume=0.66 已由外部写入)")
            print(f"  ② 播放中 clip_update:落账 {apply_lag_ms:.0f}ms(rev {rev0}→{rev_now},opacity=0.88)")
            print(f"  媒体 currentTime:回跳峰值 {back_ct * 1000:.1f}ms(容差 ≤{BACKJUMP_MAX_S * 1000:.0f}),"
                  f"最小 {min_ct:.3f}s(基点 {base_ct:.3f}s,归零线 -{RESET_GUARD_S}s),"
                  f"挂钟偏差 {drift * 1000:.0f}ms(容差 ≤{WALL_DRIFT_S * 1000:.0f})")
            print(f"  元素引用:原 {len(base_marks)} 个打点全程健在,收尾池 {len(final_marks)} 元素含全部原引用")
            assert not pageerrors, f"页面 JS 异常: {pageerrors[:5]}"
            browser.close()

        print(f"e2e_playback_survival: 全部 PASS(耗时 {time.time() - t0:.1f}s)")
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
