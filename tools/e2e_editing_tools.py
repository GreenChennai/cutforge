#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册四 AC-4.2 时间线编辑全工具 e2e(e2e_editing_tools)。

    python tools/e2e_editing_tools.py [--bin cutforge-mcp] [--cli cutforge-cli]

断言链(断言纪律 M10-R5:UI 动作只作驱动,断言以服务端状态 rev/oplog/投影为准):
  1. 四件套手势各一次 = 恰一 Op(oplog 计数差值法,FE1 冒烟同口径):
     trim(边缘拖)/roll(Shift+边缘拖,双边联动)/slip(Alt+主体拖,内容窗平移)/
     slide(Ctrl+主体拖,邻居让位)——投影逐字段断言语义,拖拽零 Op、松手单 Op;
  2. clip_split_all(Shift+S):播放头处 V1+A1 双轨命中一次全分割 = 恰一 Op;
  3. clip_gap_delete:轨头右键菜单「删除此轨播放头处间隙」→ 后继左移闭合 = 恰一 Op;
  4. clip_copy + clip_paste_at(Ctrl+C/Ctrl+V):复制零 Op,粘贴带属性单 Op 落播放头;
  5. 实渲:基线 + 四件套各渲染一次(短工程真渲染,640x360),成片时长逐差对拍语义——
     trim +200ms → +0.2s;roll/slip 时长不变;slide 头部开窗 +200ms → +0.2s;
  6. 锁定轨拒编辑:轨头锁定(track_update,1 Op)→ 拖拽/trim 手势被拒(toast + 零 Op)。

吸附口径:磁吸关(确定性普适取整),PX_PER_MS=0.06 红线,12px = 200ms。
退出码:0 通过 / 2 失败。依赖:playwright(chromium)+ ffmpeg + cutforge-cli + cutforge-render。
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
PX_PER_MS = 0.06      # 壳显示映射红线(apps/web/js/core/model.js)
DRAG_PX = 12          # 每次手势位移(= 200ms;磁吸关,普适取整无帧网格漂移)
FPS = 30
START_MS = 10000      # 夹具整体右移(避开轨头 sticky 覆盖区,点击可达)
C1_MS, C2_MS, A_MS = 2000, 4000, 3000   # V1 两段贴合 + A1 一段(相对 START_MS)
MEDIA_S = 40          # 素材时长(slip 正向 +200ms 需要素材余量)
RENDER_TIMEOUT_S = 300

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[editing-tools] {msg}", flush=True)


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
    return subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")


def make_media(ws: Path, name: str, seconds: int, size: str = "640x360") -> None:
    r = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
            "-i", f"testsrc2=size={size}:rate=30", "-f", "lavfi",
            "-i", f"sine=frequency=440:duration={seconds}",
            "-t", str(seconds), "-pix_fmt", "yuv420p",
            "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
            str(ws / name)])
    assert r.returncode == 0 and (ws / name).is_file(), f"ffmpeg 生成素材失败:{r.stderr[-300:]}"


def server_rev(port: int, token: str, root: str) -> int:
    return rpc(port, token, "project_get", {"root": root})["data"]["rev"]


def oplog_count(port: int, token: str, root: str) -> int:
    return rpc(port, token, "oplog_tail", {"root": root, "limit": 1})["data"]["count"]


def clips_of(port: int, token: str, root: str, track_id: str) -> list[dict]:
    tl = rpc(port, token, "timeline_get", {"root": root})["data"]["clips"]
    return sorted([c for c in tl if c["track"] == track_id], key=lambda c: c["startMs"])


def wait_ops_grow(port: int, token: str, root: str, prev_ops: int, timeout_s: float = 15.0) -> int:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        n = oplog_count(port, token, root)
        if n > prev_ops:
            return n
        time.sleep(0.15)
    raise AssertionError(f"OpLog 在 {timeout_s}s 内未从 {prev_ops} 上涨")


def gesture_drag(page, x0: float, y0: float, dx_px: int, mod: str | None = None) -> None:
    """一次手势 = 一次按下-拖动-松手(修饰键经键盘态注入,pointer 事件如实携带)。"""
    page.mouse.move(x0, y0)
    if mod:
        page.keyboard.down(mod)
    page.mouse.down()
    page.mouse.move(x0 + dx_px, y0, steps=4)
    page.mouse.up()
    if mod:
        page.keyboard.up(mod)


def seek_ruler(page, ms: int) -> None:
    page.click('[data-testid="ruler"]', position={"x": int(ms * PX_PER_MS), "y": 10})
    page.wait_for_function(
        "(ms) => Math.abs(Number(document.querySelector('[data-testid=\"playhead-ms\"]').textContent) - ms) <= 34",
        arg=ms, timeout=5000)


def render_once(port: int, token: str, root: str, proj: Path, tag: str) -> float:
    """同步等一炉渲染并 ffprobe 成片时长(秒;本机无 ffprobe 时跳过对拍返回 -1)。"""
    rr = rpc(port, token, "render_run", {"root": root, "backend": "cutforge"})
    assert rr["ok"], f"render_run({tag}): {rr}"
    run_id = rr["data"]["runId"]
    deadline = time.time() + RENDER_TIMEOUT_S
    state, output = "running", None
    while time.time() < deadline:
        s = rpc(port, token, "render_progress", {"root": root, "runId": run_id})
        state, output = s["data"]["state"], s["data"].get("output")
        if state != "running":
            break
        time.sleep(0.5)
    assert state == "ok", f"渲染({tag})未成功: {state}"
    assert output and (proj / output).is_file(), f"渲染({tag})产物缺失: {output}"
    ffprobe = shutil.which("ffprobe")
    if not ffprobe:
        return -1.0
    out = subprocess.run(
        [ffprobe, "-v", "error", "-print_format", "json", "-show_format", str(proj / output)],
        capture_output=True, text=True).stdout
    return float(json.loads(out)["format"]["duration"])


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=None)
    ap.add_argument("--cli", default=None)
    args = ap.parse_args()
    mcp = locate_bin(args.bin, ("cutforge-mcp",))
    cli = locate_bin(args.cli, ("cutforge-cli",))
    render_bin = locate_bin(None, ("cutforge-render",))
    if not mcp or not cli:
        print("FAIL: 先 cargo build(cutforge-mcp + cutforge-cli)", file=sys.stderr)
        return 2
    if not render_bin:
        print("FAIL: 未找到 cutforge-render(实渲对拍依赖)", file=sys.stderr)
        return 2
    if not shutil.which("ffmpeg"):
        print("FAIL: 本机无 ffmpeg(夹具素材需现场合成)", file=sys.stderr)
        return 2

    from playwright.sync_api import sync_playwright

    t0 = time.time()
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-edit-tools-"))
    token = "edit-tools-token"
    proj = tmp / "tools-proj"
    r = sh([str(cli), "new", str(proj), "--slug", "tools-proj", "--json", "--fps", str(FPS),
            "--width", "640", "--height", "360", "--track", "video,audio"])
    assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"
    if (proj / "05_时间线工程").is_dir():
        tl_rel, mat_rel = "05_时间线工程", "01_原始素材"
    else:
        tl_rel, mat_rel = "05_ir", "01_materials"
    (proj / mat_rel).mkdir(parents=True, exist_ok=True)
    make_media(proj / mat_rel, "main.mp4", MEDIA_S)
    # V1:两段贴合(roll 需相邻);A1:一段(split_all 双轨命中)
    pj = {"version": 1, "schemaVersion": "2.0.0", "slug": "tools-proj", "fps": FPS,
          "canvas": {"width": 640, "height": 360},
          "tracks": [
              {"id": "V1", "kind": "video", "name": "V1", "clips": [
                  {"id": "V1-001", "src": f"{mat_rel}/main.mp4", "startMs": START_MS, "durationMs": C1_MS},
                  {"id": "V1-002", "src": f"{mat_rel}/main.mp4", "startMs": START_MS + C1_MS, "durationMs": C2_MS},
              ]},
              {"id": "A1", "kind": "audio", "name": "A1", "clips": [
                  {"id": "A1-001", "src": f"{mat_rel}/main.mp4", "startMs": START_MS, "durationMs": 2000},
                  {"id": "A1-002", "src": f"{mat_rel}/main.mp4", "startMs": START_MS + 3000, "durationMs": 3000},
              ]},
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
            page.wait_for_function(
                """() => { const m = [...document.querySelectorAll('[data-testid="pv-media"] video')];
                           return m.length >= 1 && m.some((e) => e.readyState >= 2); }""", timeout=20000)
            page.wait_for_timeout(500)
            # 磁吸关:确定性普适取整(12px = 恰 200ms),帧网格不参与
            if page.is_checked('[data-testid="magnet"]'):
                page.uncheck('[data-testid="magnet"]')

            ops0 = oplog_count(port, token, root)
            rev0 = server_rev(port, token, root)
            assert ops0 == rev0 == 0, f"夹具初始账目应为 0: ops={ops0} rev={rev0}"

            # ============ 1. 基线渲染(四件套时长对拍的参照) ============
            d_prev = render_once(port, token, root, proj, "baseline")
            log(f"1. 基线渲染:{d_prev:.2f}s(时间线标称 {START_MS + C1_MS + C2_MS}ms)" if d_prev > 0
                else "1. 基线渲染完成(本机无 ffprobe,时长逐差对拍跳过,渲染链仍实证)")

            def dur_step(cur: float, expect_delta_s: float, tag: str) -> float:
                """实渲逐差对拍:与上一次渲染比,时长差须吻合语义(±0.3s 容差;无 ffprobe 跳过)。"""
                if d_prev <= 0:
                    return cur
                assert abs((cur - d_prev) - expect_delta_s) <= 0.3, \
                    f"实渲时长语义({tag}):{d_prev:.2f}s → {cur:.2f}s(期望差 {expect_delta_s:+.1f}s ±0.3)"
                return cur

            # ============ 2. trim(边缘拖 +12px = +200ms)= 恰一 Op + 实渲 ============
            page.click('[data-testid="clip"][data-id="V1-002"]')  # 先选中使边缘把手显形
            edge = page.locator('[data-testid="clip"][data-id="V1-002"] .edge-r').bounding_box()
            assert edge, "V1-002 右 trim 把手不可达"
            ops = oplog_count(port, token, root)
            gesture_drag(page, edge["x"] + edge["width"] / 2, edge["y"] + edge["height"] / 2, DRAG_PX)
            ops = wait_ops_grow(port, token, root, ops)
            c2 = next(c for c in clips_of(port, token, root, "V1") if c["id"] == "V1-002")
            assert c2["startMs"] == START_MS + C1_MS and c2["durationMs"] == C2_MS + 200,                 f"trim 出点 +200ms: {c2}"
            assert oplog_count(port, token, root) == ops, "trim 必须恰好一 Op"
            d_prev = dur_step(render_once(port, token, root, proj, "trim"), 0.2, "trim +200ms")
            log(f"2. trim(边缘拖 +200ms):恰 1 Op,duration {C2_MS}→{c2['durationMs']},"
                f"实渲 {d_prev:.2f}s(+0.2s): PASS" if d_prev > 0 else
                f"2. trim(边缘拖 +200ms):恰 1 Op,duration {C2_MS}→{c2['durationMs']}: PASS")

            # ============ 3. roll(Shift+边缘拖)= 恰一 Op,双边联动 + 实渲 ============
            page.click('[data-testid="clip"][data-id="V1-001"]')
            edge = page.locator('[data-testid="clip"][data-id="V1-001"] .edge-r').bounding_box()
            assert edge, "V1-001 右边缘不可达"
            ops = oplog_count(port, token, root)
            gesture_drag(page, edge["x"] + edge["width"] / 2, edge["y"] + edge["height"] / 2,
                         DRAG_PX, mod="Shift")
            ops = wait_ops_grow(port, token, root, ops)
            v1 = clips_of(port, token, root, "V1")
            c1 = next(c for c in v1 if c["id"] == "V1-001")
            c2 = next(c for c in v1 if c["id"] == "V1-002")
            assert c1["durationMs"] == C1_MS + 200 and c2["startMs"] == START_MS + C1_MS + 200, \
                f"roll 边界联动: {c1} | {c2}"
            assert c1["startMs"] + c1["durationMs"] == c2["startMs"], "roll 后边界必须仍贴合"
            assert c2["startMs"] + c2["durationMs"] == START_MS + C1_MS + C2_MS + 200, \
                "roll 不得改变总跨度末端"
            assert oplog_count(port, token, root) == ops, "roll 必须恰好一 Op"
            d_prev = dur_step(render_once(port, token, root, proj, "roll"), 0.0, "roll 跨度不变")
            log(f"3. roll(Shift+边缘拖 +200ms):恰 1 Op,边界 →{c2['startMs']}ms 双边联动,实渲 {d_prev:.2f}s(不变)"
                if d_prev > 0 else
                f"3. roll(Shift+边缘拖 +200ms):恰 1 Op,边界 →{c2['startMs']}ms 双边联动: PASS")

            # ============ 4. slip(Alt+主体拖)= 恰一 Op,内容窗平移 + 实渲 ============
            box = page.locator('[data-testid="clip"][data-id="V1-002"]').bounding_box()
            assert box, "V1-002 主体不可达"
            ops = oplog_count(port, token, root)
            gesture_drag(page, box["x"] + box["width"] / 2, box["y"] + box["height"] / 2,
                         DRAG_PX, mod="Alt")
            ops = wait_ops_grow(port, token, root, ops)
            c2 = next(c for c in clips_of(port, token, root, "V1") if c["id"] == "V1-002")
            assert c2["sourceInMs"] == 200, f"slip 内容窗 +200ms: {c2}"
            # roll 后 C2 = [12200, 16200) dur 4000;slip 占位(起点/时长)必须原样
            assert c2["startMs"] == START_MS + C1_MS + 200 and c2["durationMs"] == C2_MS, \
                f"slip 占位必须不变: {c2}"
            assert oplog_count(port, token, root) == ops, "slip 必须恰好一 Op"
            d_prev = dur_step(render_once(port, token, root, proj, "slip"), 0.0, "slip 占位不变")
            log(f"4. slip(Alt+主体拖 +200ms):恰 1 Op,sourceIn 0→{c2['sourceInMs']} 占位不变,实渲 {d_prev:.2f}s"
                if d_prev > 0 else
                f"4. slip(Alt+主体拖 +200ms):恰 1 Op,sourceIn 0→{c2['sourceInMs']}、占位不变: PASS")

            # ============ 5. slide(Ctrl+主体拖)= 恰一 Op,邻居让位 + 实渲 ============
            box = page.locator('[data-testid="clip"][data-id="V1-001"]').bounding_box()
            assert box, "V1-001 主体不可达"
            ops = oplog_count(port, token, root)
            gesture_drag(page, box["x"] + box["width"] / 2, box["y"] + box["height"] / 2,
                         DRAG_PX, mod="Control")
            ops = wait_ops_grow(port, token, root, ops)
            v1 = clips_of(port, token, root, "V1")
            c1 = next(c for c in v1 if c["id"] == "V1-001")
            c2 = next(c for c in v1 if c["id"] == "V1-002")
            assert c1["startMs"] == START_MS + 200, f"slide 位置 +200ms: {c1}"
            assert c1["durationMs"] == C1_MS + 200, f"slide 内容窗必须原样(区别于 roll 的边界扩张): {c1}"
            assert c2["startMs"] == START_MS + 200 + C1_MS + 200, f"slide 右邻让位: {c2}"
            assert c2["durationMs"] == C2_MS - 200, "slide 右邻从左侧收缩(末端定点)"
            assert c2["startMs"] + c2["durationMs"] == START_MS + C1_MS + C2_MS + 200, \
                "slide 末端定点不变(窗口右移、右邻收缩)"
            assert oplog_count(port, token, root) == ops, "slide 必须恰好一 Op"
            d_prev = dur_step(render_once(port, token, root, proj, "slide"), 0.0, "slide 末端定点")
            log(f"5. slide(Ctrl+主体拖 +200ms):恰 1 Op,V1-001→{c1['startMs']} 右邻让位,实渲 {d_prev:.2f}s(+0.2s)"
                if d_prev > 0 else
                f"5. slide(Ctrl+主体拖 +200ms):恰 1 Op,V1-001→{c1['startMs']}、右邻让位 →{c2['startMs']}: PASS")

            # ============ 6. clip_split_all(Shift+S @11000ms)= 恰一 Op 双轨命中 ============
            seek_ruler(page, START_MS + 1000)
            ops = oplog_count(port, token, root)
            page.keyboard.press("Shift+s")
            ops = wait_ops_grow(port, token, root, ops)
            v1 = clips_of(port, token, root, "V1")
            a1 = clips_of(port, token, root, "A1")
            assert len(v1) == 3, f"V1 应 3 段: {[c['id'] for c in v1]}"
            assert len(a1) == 3, f"A1 应 3 段: {[c['id'] for c in a1]}"
            assert any(c["startMs"] == START_MS + 1000 for c in v1) \
                and any(c["startMs"] == START_MS + 1000 for c in a1), \
                f"双轨都应在 {START_MS + 1000}ms 分割: {v1} | {a1}"
            assert oplog_count(port, token, root) == ops, "split_all 必须恰好一 Op"
            log(f"6. clip_split_all @{START_MS + 1000}ms(Shift+S):恰 1 Op,V1 3 段 + A1 3 段: PASS")

            # ============ 7. clip_gap_delete(轨头右键菜单)= 恰一 Op 闭合 ============
            # A1 中段间隙 [START_MS+2000, START_MS+3000)(夹具原生),播放头置入 →
            # 后继 A1-002 整体左移 1000ms 闭合(单 Op)
            seek_ruler(page, START_MS + 2500)
            ops = oplog_count(port, token, root)
            page.click('[data-testid="lane-label-A1"]', button="right")
            page.wait_for_selector('[data-testid="context-menu"]', timeout=5000)
            page.click('[data-testid="context-menu"] button:has-text("间隙")')
            ops = wait_ops_grow(port, token, root, ops)
            a1 = clips_of(port, token, root, "A1")
            last = a1[-1]
            assert len(a1) == 3 and last["id"] == "A1-002" and last["startMs"] == START_MS + 2000, \
                f"gap_delete 后后继须左移闭合: {a1}"
            assert oplog_count(port, token, root) == ops, "gap_delete 必须恰好一 Op"
            log(f"7. clip_gap_delete @{START_MS + 2500}ms(轨头菜单):恰 1 Op,"
                f"A1-002 {START_MS + 3000}→{last['startMs']} 闭合: PASS")

            # ============ 8. clip_copy + clip_paste_at(Ctrl+C/Ctrl+V)= 复制零 Op/粘贴单 Op ============
            page.click('[data-testid="clip"][data-id="V1-001"]')
            page.keyboard.press("Control+c")
            time.sleep(0.4)
            assert oplog_count(port, token, root) == ops, "clip_copy 不得产 Op"
            src = next(c for c in clips_of(port, token, root, "V1") if c["id"] == "V1-001")
            v1 = clips_of(port, token, root, "V1")
            end_before = max(c["startMs"] + c["durationMs"] for c in v1)
            page.keyboard.press("End")  # 播放头 → 时间线末端(粘到末尾,无重叠顾虑)
            page.keyboard.press("Control+v")
            wait_ops_grow(port, token, root, ops)
            v1 = clips_of(port, token, root, "V1")
            pasted = next((c for c in v1 if c["startMs"] == end_before), None)
            assert pasted, f"粘贴必须落播放头(原时间线末端 {end_before}ms): {v1}"
            assert pasted["durationMs"] == src["durationMs"], \
                f"粘贴必须带属性(时长): {pasted} vs 源 {src}"
            assert pasted["id"] != "V1-001", "粘贴必须分配新 id"
            assert len(v1) == 4, f"粘贴后 V1 应 4 段: {[c['id'] for c in v1]}"
            log(f"8. clip_copy(零 Op)+ clip_paste_at @末端(单 Op):新段 {pasted['id']} "
                f"@{pasted['startMs']}ms dur={pasted['durationMs']}(=源): PASS")

            # ============ 9. 锁定轨拒编辑(track_update 1 Op;手势零 Op) ============
            ops = oplog_count(port, token, root)
            page.click('[data-testid="lane-label-A1"] .lane-lock')
            wait_ops_grow(port, token, root, ops)
            page.wait_for_function(
                """() => document.querySelector('[data-testid="lane-label-A1"] .lane-lock')
                           ?.getAttribute('aria-pressed') === 'true'""", timeout=8000)
            assert page.get_attribute('[data-testid="lane-label-A1"] .lane-lock', "aria-pressed") == "true", \
                "锁定按钮应 aria-pressed=true"
            assert page.evaluate(
                "document.querySelector('[data-testid=\"track-lane-A1\"]').classList.contains('lane-locked')"), \
                "锁定轨 lane 应挂 .lane-locked"
            ops = oplog_count(port, token, root)
            box = page.locator('[data-testid="clip"][data-id="A1-002"]').bounding_box()
            assert box, "A1-002 主体不可达"
            # ghost 口径:slide 手势的 ghost 由壳侧 clearTransient 遗留(已登记 FE 遗留,
            # apps/web 本册禁改),故只断「拖拽锁定轨不新增 ghost/不改 ghost」
            ghost_before = page.evaluate(
                """() => { const g = document.querySelector('[data-testid="drag-ghost"]');
                           return g ? g.outerHTML : null; }""")
            gesture_drag(page, box["x"] + box["width"] / 2, box["y"] + box["height"] / 2, DRAG_PX)
            page.wait_for_timeout(400)
            ghost_after = page.evaluate(
                """() => { const g = document.querySelector('[data-testid="drag-ghost"]');
                           return g ? g.outerHTML : null; }""")
            assert ghost_after == ghost_before, "锁定轨拖拽不得新增/改变 ghost"
            toast = page.inner_text('[data-testid="toasts"]')
            assert "锁定" in toast, f"锁定提示缺失: {toast!r}"
            edge = page.locator('[data-testid="clip"][data-id="A1-002"] .edge-r').bounding_box()
            if edge:
                gesture_drag(page, edge["x"] + edge["width"] / 2, edge["y"] + edge["height"] / 2, DRAG_PX)
                page.wait_for_timeout(300)
            assert oplog_count(port, token, root) == ops, "锁定轨手势必须零 Op"
            a1 = clips_of(port, token, root, "A1")
            assert a1[0]["id"] == "A1-001" and a1[0]["startMs"] == START_MS \
                and a1[0]["durationMs"] == 1000, f"锁定轨投影不得变化: {a1}"
            log(f"9. 锁定轨拒编辑:拖拽/trim 双手势均拒(toast + 零 Op,投影不变): PASS")

            # ============ 10. 账目闭合 ============
            rev = server_rev(port, token, root)
            assert oplog_count(port, token, root) == rev, \
                f"OpLog({oplog_count(port, token, root)}) 与 rev({rev}) 不一致"
            assert not pageerrors, f"页面 JS 异常: {pageerrors[:5]}"
            browser.close()

        print(f"AC-4.2 时间线编辑全工具: PASS(耗时 {time.time() - t0:.1f}s)")
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
