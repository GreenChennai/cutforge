#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册五 AC-5.1 关键帧壳侧闭环 e2e(e2e_keyframes)。

    python tools/e2e_keyframes.py [--bin cutforge-mcp] [--cli cutforge-cli]

断言链(断言纪律 M10-R5:UI 动作只作驱动,断言以服务端状态 rev/oplog/投影为准):
  1. 秒表打点:检查器秒表(kw-toggle-opacity)@播放头 → 投影 clip.keyframes 落帧
     (timeMs 相对片段起点),拖拽零 Op、松手单 Op(与 editing_tools 同口径);
  2. 编辑器打点:曲线编辑器「+ 打点」(kw-add)@播放头 → 第二帧;列表值输入
     (kw-value-N)改值 = 单 Op;
  3. 投影采样断言:timeline_get 逐 clip 含 keyframes 原始数组 + keyframeSamples
     采样点集(网格 100ms ∪ 关键帧时刻;线性中点采样值 = 求值器同点输出);
  4. 求值一致性(轻量一帧):Rust 采样投影(线性中点 = 0.75)vs 渲染帧实测——
     render_frame 于中点,帧均亮度 ≈ 0.75 × 满透明度帧(页内 canvas 采样,
     subtitle_editor 同法);
  5. 曲线编辑器拖锚点:canvas 锚点纵拖(零 Op)→ 松手写回单 Op,投影值随像素
     域换算逐字段对拍(round4(vAt));
  6. 时间线菱形拖移:kf-row 菱形横拖 → 松手单 Op,timeMs 随 PX_PER_MS=0.06 红线
     对拍(60px = 1000ms);
  7. 插值预设:kw-interp-easeIn 点选 = 单 Op,投影 interp 落 easeIn(缓动段采样
     偏离线性,壳只画点零插值);
  8. 账目闭合:OpLog 数 = rev;页面零 JS 异常。

退出码:0 通过 / 2 失败。依赖:playwright(chromium)+ ffmpeg + cutforge-cli。
"""
from __future__ import annotations

import argparse
import base64
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
FPS = 30
START_MS = 0          # 片段起点 = 0(render_frame 合成基片时间域 = 片段跨度,
                      #  前导黑场会让绝对 at_q 越过基片 EOF 空产出——实测钉坑)
CLIP_MS = 12000       # 片段时长(关键帧时间域 0..12000)
CANVAS_W, CANVAS_H, PAD = 340, 170, 10   # kf-curve.js 画布内域常量
OPACITY_DOMAIN = (0.0, 1.0)              # PROP_META.opacity.domain
RENDER_TIMEOUT_S = 120

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[keyframes] {msg}", flush=True)


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


def wait_ops_grow(port: int, token: str, root: str, prev_ops: int, timeout_s: float = 15.0) -> int:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        n = oplog_count(port, token, root)
        if n > prev_ops:
            return n
        time.sleep(0.15)
    raise AssertionError(f"OpLog 在 {timeout_s}s 内未从 {prev_ops} 上涨")


def clip_row(port: int, token: str, root: str, clip_id: str) -> dict:
    tl = rpc(port, token, "timeline_get", {"root": root})["data"]["clips"]
    return next(c for c in tl if c["id"] == clip_id)


def opacity_kfs(row: dict) -> list[dict]:
    return [k for k in (row.get("keyframes") or []) if k["property"] == "opacity"]


def opacity_samples(row: dict) -> list[list[float]]:
    for s in row.get("keyframeSamples") or []:
        if s["property"] == "opacity":
            return s["samples"]
    return []


def seek_ruler(page, ms: int) -> None:
    page.click('[data-testid="ruler"]', position={"x": int(ms * PX_PER_MS), "y": 10})
    page.wait_for_function(
        "(ms) => Math.abs(Number(document.querySelector('[data-testid=\"playhead-ms\"]').textContent) - ms) <= 34",
        arg=ms, timeout=5000)


def expand_group(page, name: str) -> None:
    """检查器折叠组展开(重试至内容可见;subtitle_editor 同法)。"""
    grp = page.locator(f'[data-testid="insp-group-{name}"]')
    for _ in range(3):
        if "collapsed" not in (grp.get_attribute("class") or ""):
            return
        grp.locator("legend").click()
        page.wait_for_timeout(250)
    assert "collapsed" not in (grp.get_attribute("class") or ""), f"检查器组「{name}」无法展开"


def drag_canvas_point(page, testid: str, from_xy: tuple[float, float], to_xy: tuple[float, float]) -> None:
    """画布内域坐标拖拽(内部像素 → CSS 缩放换算;按下-移动-松手)。"""
    box = page.locator(f'[data-testid="{testid}"]').bounding_box()
    assert box, f"{testid} 不可达"
    sx = box["x"] + from_xy[0] / CANVAS_W * box["width"]
    sy = box["y"] + from_xy[1] / CANVAS_H * box["height"]
    tx = box["x"] + to_xy[0] / CANVAS_W * box["width"]
    ty = box["y"] + to_xy[1] / CANVAS_H * box["height"]
    page.mouse.move(sx, sy)
    page.mouse.down()
    page.mouse.move(tx, ty, steps=6)
    page.mouse.up()


def curve_xy_of(t_ms: float, v: float, dur_ms: int) -> tuple[float, float]:
    """kf-curve.js 像素域换算(xOf/yOf;t∈[0,dur]、v∈domain;PAD=10)。"""
    x = PAD + (t_ms / dur_ms) * (CANVAS_W - PAD * 2)
    lo, hi = OPACITY_DOMAIN
    y = CANVAS_H - PAD - ((v - lo) / (hi - lo)) * (CANVAS_H - PAD * 2)
    return x, y


def canvas_value_of(py: float) -> float:
    """kf-curve.js vAt:像素纵坐标 → 域值。"""
    lo, hi = OPACITY_DOMAIN
    return lo + ((CANVAS_H - PAD - py) / (CANVAS_H - PAD * 2)) * (hi - lo)


def render_frame_png(port: int, token: str, root: str, at_ms: int) -> bytes:
    r = rpc(port, token, "render_frame", {"root": root, "atMs": at_ms, "format": "png"})
    assert r["ok"], f"render_frame({at_ms}): {r}"
    p = Path(r["data"]["path"])
    assert p.is_file(), f"帧产物缺失: {p}"
    return p.read_bytes()


MEAN_LUMA_PROBE = """async (b64) => {
  const img = new Image();
  await new Promise((res, rej) => { img.onload = res; img.onerror = () => rej(new Error("img"));
                                    img.src = "data:image/png;base64," + b64; });
  const c = document.createElement("canvas");
  c.width = img.width; c.height = img.height;
  const ctx = c.getContext("2d");
  ctx.drawImage(img, 0, 0);
  const d = ctx.getImageData(0, 0, c.width, c.height).data;
  let sum = 0;
  for (let i = 0; i < d.length; i += 4) sum += 0.2126 * d[i] + 0.7152 * d[i + 1] + 0.0722 * d[i + 2];
  return sum / (d.length / 4);
}"""


def settle_ops(port: int, token: str, root: str, ops: int, quiet_s: float = 0.6,
               timeout_s: float = 15.0) -> int:
    """等 OpLog 增长并**静止**(quiet_s 无新 Op)后返回终值(单 Op 断言用)。"""
    deadline = time.time() + timeout_s
    last = ops
    last_change = time.time()
    while time.time() < deadline:
        n = oplog_count(port, token, root)
        if n > last:
            last, last_change = n, time.time()
        elif n == last and last > ops and time.time() - last_change >= quiet_s:
            return n
        elif n == last and n == ops and time.time() - last_change >= timeout_s:
            raise AssertionError(f"OpLog 在 {timeout_s}s 内未从 {ops} 上涨")
        time.sleep(0.15)
    raise AssertionError(f"OpLog {quiet_s}s 内未静止(终值 {last})")


def ops_dump(port: int, token: str, root: str) -> list[str]:
    d = rpc(port, token, "oplog_tail", {"root": root, "limit": 8})
    return [f"{o.get('target', {}).get('path')}|{o.get('summary', '')[:48]}"
            for o in d["data"].get("ops", [])]


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
        print("FAIL: 本机无 ffmpeg(夹具素材需现场合成)", file=sys.stderr)
        return 2

    from playwright.sync_api import sync_playwright

    t0 = time.time()
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-e2e-kf-"))
    token = "kf-e2e-token"
    proj = tmp / "kf-proj"
    r = sh([str(cli), "new", str(proj), "--slug", "kf-proj", "--json", "--fps", str(FPS),
            "--width", "640", "--height", "360", "--track", "video"])
    assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"
    if (proj / "05_时间线工程").is_dir():
        tl_rel, mat_rel = "05_时间线工程", "01_原始素材"
    else:
        tl_rel, mat_rel = "05_ir", "01_materials"
    (proj / mat_rel).mkdir(parents=True, exist_ok=True)
    make_media(proj / mat_rel, "main.mp4", 40)
    # 单片段:testsrc2 彩色画面(opacity 叠黑底的亮度比可直接读出求值结果)
    pj = {"version": 1, "schemaVersion": "3.0.0", "slug": "kf-proj", "fps": FPS,
          "canvas": {"width": 640, "height": 360},
          "tracks": [{"id": "V1", "kind": "video", "name": "V1", "clips": [
              {"id": "V1-001", "src": f"{mat_rel}/main.mp4",
               "startMs": START_MS, "durationMs": CLIP_MS},
          ]}]}
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
            page.click('[data-testid="clip"][data-id="V1-001"]')
            page.wait_for_timeout(300)
            expand_group(page, "画面")

            ops0 = oplog_count(port, token, root)
            rev0 = server_rev(port, token, root)
            assert ops0 == rev0 == 0, f"夹具初始账目应为 0: ops={ops0} rev={rev0}"

            # ============ 1. 秒表打点 @播放头(=片段起点,rel 0)= 单 Op ============
            seek_ruler(page, START_MS)
            ops = oplog_count(port, token, root)
            page.click('[data-testid="kw-toggle-opacity"]')
            ops = settle_ops(port, token, root, ops)
            row = clip_row(port, token, root, "V1-001")
            kfs = opacity_kfs(row)
            assert len(kfs) == 1 and kfs[0]["timeMs"] == 0 and abs(kfs[0]["value"] - 1.0) < 1e-9, \
                f"秒表打点须落 opacity@0=1.0: {kfs}"
            assert oplog_count(port, token, root) == ops, "秒表打点必须恰好一 Op"
            assert row.get("keyframeSamples"), "投影必须携带 keyframeSamples"
            log("1. 秒表打点(kw-toggle-opacity @rel 0):恰 1 Op,投影 keyframes=1 帧: PASS")

            # ============ 2. 曲线编辑器「+ 打点」@rel 4000 + 列表改值 = 单 Op ============
            seek_ruler(page, START_MS + 4000)
            page.select_option('[data-testid="kw-prop-select"]', "opacity")
            ops = oplog_count(port, token, root)
            page.click('[data-testid="kw-add"]')
            ops = settle_ops(port, token, root, ops)
            row = clip_row(port, token, root, "V1-001")
            kfs = opacity_kfs(row)
            assert [k["timeMs"] for k in kfs] == [0, 4000], f"打点后须 2 帧: {kfs}"
            assert oplog_count(port, token, root) == ops, "kw-add 必须恰好一 Op"
            # 列表值输入:第 2 帧值 → 0.5(单 Op)
            ops = oplog_count(port, token, root)
            page.fill('[data-testid="kw-value-1"]', "0.5")
            page.dispatch_event('[data-testid="kw-value-1"]', "change")
            ops = settle_ops(port, token, root, ops)
            row = clip_row(port, token, root, "V1-001")
            kfs = opacity_kfs(row)
            assert abs(kfs[1]["value"] - 0.5) < 1e-9, f"列表改值须写回 0.5: {kfs}"
            assert oplog_count(port, token, root) == ops, "列表改值必须恰好一 Op"
            log("2. 曲线编辑器 + 打点 @rel 4000、列表值改 0.5:各恰 1 Op,投影 2 帧: PASS")

            # ============ 3. 投影采样断言(网格 100ms ∪ 关键帧时刻;线性求值) ============
            samples = opacity_samples(clip_row(port, token, root, "V1-001"))
            assert samples, "keyframeSamples.opacity 缺失"
            times = [s[0] for s in samples]
            assert times[0] == 0 and times[-1] == CLIP_MS, f"采样域须覆盖 [0,{CLIP_MS}]: {times[:3]}…{times[-1]}"
            for kf_t in (0, 4000):
                assert kf_t in times, f"关键帧时刻 {kf_t} 必须在采样点集: {times}"
            grid = {t for t in times if t % 100 == 0}
            assert {0, 100, 2000, 4000} <= grid, "100ms 网格点必须在采样点集"
            at2000 = next(v for t, v in samples if t == 2000)
            assert abs(at2000 - 0.75) < 1e-9, f"线性中点采样须 = 0.75(1→0.5): {at2000}"
            log(f"3. keyframeSamples:{len(samples)} 点(网格∪帧时刻),线性中点采样 = {at2000}: PASS")

            # ============ 4. 求值一致性(轻量一帧):采样 0.75 vs 渲染帧亮度比 ============
            l_full = page.evaluate(MEAN_LUMA_PROBE, base64.b64encode(
                render_frame_png(port, token, root, START_MS)).decode("ascii"))
            l_mid = page.evaluate(MEAN_LUMA_PROBE, base64.b64encode(
                render_frame_png(port, token, root, START_MS + 2000)).decode("ascii"))
            ratio = l_mid / l_full
            assert abs(ratio - 0.75) <= 0.08, \
                f"渲染帧亮度比须 ≈ 0.75(采样投影值): {l_full:.1f} → {l_mid:.1f} ratio={ratio:.3f}"
            log(f"4. 求值一致性:render_frame@+2000 亮度比 = {ratio:.3f}(采样投影 0.75 ±0.08): PASS")

            # ============ 5. 曲线编辑器拖锚点:纵拖第 2 锚(零 Op)→ 松手单 Op ============
            # 锚 2 @rel 4000 v=0.5 → 像素 (116.67, 85);纵拖至 py=20 → v = 140/150 ≈ 0.9333
            ax, ay = curve_xy_of(4000, 0.5, CLIP_MS)
            target_v = round(canvas_value_of(20.0), 4)
            ops = oplog_count(port, token, root)
            drag_canvas_point(page, "kf-curve-canvas", (ax, ay), (ax, 20.0))
            ops = settle_ops(port, token, root, ops)
            row = clip_row(port, token, root, "V1-001")
            kfs = opacity_kfs(row)
            assert kfs[1]["timeMs"] == 4000 and abs(kfs[1]["value"] - target_v) < 0.005, \
                f"锚点纵拖写回须 timeMs=4000/value≈{target_v}: {kfs}"
            assert oplog_count(port, token, root) == ops, "锚点拖拽必须恰好一 Op(拖中零 Op)"
            log(f"5. 曲线编辑器拖锚点:v 0.5→{kfs[1]['value']}(round4(vAt)),恰 1 Op: PASS")

            # ============ 6. 时间线菱形拖移:60px = +1000ms(PX_PER_MS 红线)= 单 Op ============
            # 菱形视觉原点含 kf-row-add 按钮宽(壳侧几何),指针→时刻映射按 wrap 实时
            # 换算(与 kf-row.js applyMove 同式):期望终值 = round(中心映射值 + 60/0.06)。
            dia = page.locator('[data-testid="kw-diamond"][data-prop="opacity"][data-time-ms="4000"]')
            assert dia.count() >= 1, "时间线菱形(4000ms)不可达"
            box = dia.first.bounding_box()
            assert box, "菱形 bounding_box 不可达"
            cx, cy = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
            mapped = page.evaluate(
                """([cx, cy]) => {
                     const wrap = document.getElementById('timeline-wrap');
                     return (cx - wrap.getBoundingClientRect().left + wrap.scrollLeft) / 0.06;
                   }""", [cx, cy])
            expect_t = round(mapped + 60 / PX_PER_MS)
            ops = oplog_count(port, token, root)
            page.mouse.move(cx, cy)
            page.mouse.down()
            page.mouse.move(cx + 60, cy, steps=6)
            page.mouse.up()
            ops = settle_ops(port, token, root, ops)
            row = clip_row(port, token, root, "V1-001")
            kfs = opacity_kfs(row)
            assert kfs[1]["timeMs"] == expect_t,                 f"菱形横拖 +60px 须 +1000ms(映射基点 {mapped:.0f} → 期望 {expect_t}): {kfs}"
            assert kfs[1]["timeMs"] - 4000 >= 1000, "拖移方向必须向右(30px 映射余量之上)"
            assert oplog_count(port, token, root) == ops, "菱形拖移必须恰好一 Op"
            log(f"6. 时间线菱形拖移(60px=1000ms):4000→{kfs[1]['timeMs']}ms(视觉基点偏移为壳侧几何),恰 1 Op: PASS")

            # ============ 7. 插值预设 easeIn:单 Op;缓动段采样偏离线性 ============
            # 先把第 2 锚值拉到 0(单 Op;拉大值域让缓动形状可判读)
            ops = oplog_count(port, token, root)
            page.fill('[data-testid="kw-value-1"]', "0")
            page.dispatch_event('[data-testid="kw-value-1"]', "change")
            ops = settle_ops(port, token, root, ops)
            row = clip_row(port, token, root, "V1-001")
            assert abs(opacity_kfs(row)[1]["value"]) < 1e-9, "第 2 锚值须为 0"
            assert oplog_count(port, token, root) == ops, "列表改值必须恰好一 Op"
            # 选中第 1 锚(列表行点击 = 选中;插值预设作用于选中锚的出区间)
            page.click('[data-testid="kw-kf-list"] .kw-kf-row')
            page.wait_for_timeout(200)
            ops = oplog_count(port, token, root)
            page.click('[data-testid="kw-interp-easeIn"]')
            ops = settle_ops(port, token, root, ops)
            row = clip_row(port, token, root, "V1-001")
            kfs = opacity_kfs(row)
            assert kfs[0]["interp"] == "easeIn", f"插值预设须写在区间首帧: {kfs}"
            assert oplog_count(port, token, root) == ops, "插值切换必须恰好一 Op"
            samples = {t: v for t, v in opacity_samples(row)}
            x = 3000 / kfs[1]["timeMs"]
            lin = kfs[0]["value"] + (kfs[1]["value"] - kfs[0]["value"]) * x
            assert abs(samples[3000] - lin) > 0.05, \
                f"easeIn 采样必须偏离线性(壳零插值,形状来自服务端): {samples[3000]} vs {lin:.4f}"
            log(f"7. 插值预设 easeIn:恰 1 Op;@3000 采样 {samples[3000]:.4f} ≠ 线性 {lin:.4f}: PASS")

            # ============ 8. 账目闭合 ============
            rev = server_rev(port, token, root)
            assert oplog_count(port, token, root) == rev, \
                f"OpLog({oplog_count(port, token, root)}) 与 rev({rev}) 不一致"
            assert not pageerrors, f"页面 JS 异常: {pageerrors[:5]}"
            browser.close()

        print(f"AC-5.1 关键帧壳侧闭环: PASS(耗时 {time.time() - t0:.1f}s)")
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
