#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册五 AC-5.2 调色壳侧闭环 e2e(e2e_color)。

    python tools/e2e_color.py [--bin cutforge-mcp] [--cli cutforge-cli]

断言链(断言纪律 M10-R5:UI 动作只作驱动,断言以服务端状态 rev/oplog/投影/渲染帧为准):
  1. 基线帧(未调色):中性灰素材 render_frame 三通道均值相等(±6,编解码容差);
  2. 色轮拖(Lift):grade-wheel-lift 画布拖拽(零 Op)→ 松手整对象写回单 Op,
     投影 clip.grade.lift 三通道逐值对拍(wheelToRgb 像素域换算);
  3. render_frame 帧色偏(E-FE1 冒烟同法,页内 canvas 采样):调色后帧
     meanR − meanB 显著为正(红抬升);
  4. LUT 导入应用:grade-lut-import(lut_import 真调用,.cube 拷入 .cutforge/luts/)
     → grade.lut 落投影(单 Op)→ 帧整体压暗(LUT=全通道×0.25,实测递进断言);
  5. 示波器三画布非空:scopes-toggle 开面板 → scopes-sample 采样精确帧(含调色)
     → 亮度波形/矢量/直方图三画布均有非背景像素;
  6. 分屏割线拖动:compare-toggle 开 A/B(基准帧 img.src 就绪为信号)→
     compare-divider 横拖 → 分割百分比随位移实时变化(50% → 实测值);
  7. 拷贝粘贴 grade:grade-copy(V1-001)→ 选中 V1-002 → grade-paste 单 Op,
     投影两片段 grade 逐字段等价;
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
FPS = 30
WHEEL_SIZE, WHEEL_R = 116, 50   # grade-wheel.js 常量(SIZE / R = SIZE/2 − 8)
RENDER_TIMEOUT_S = 120

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[color] {msg}", flush=True)


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def rpc(port: int, token: str, name: str, args: dict, timeout: float = 60) -> dict:
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


def make_gray_video(ws: Path, name: str, seconds: int, size: str = "640x360") -> None:
    """暗灰夹具(0x262626):未调色帧三通道均值相等;且落在 colorbalance 阴影加权域
    (中性灰 128 实测对 lift 零响应——阴影槽只作用于暗部,实测钉坑)。"""
    r = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
            "-i", f"color=c=0x262626:size={size}:rate=30",
            "-t", str(seconds), "-pix_fmt", "yuv420p",
            "-c:v", "libx264", "-preset", "veryfast", str(ws / name)])
    assert r.returncode == 0 and (ws / name).is_file(), f"ffmpeg 生成灰底夹具失败:{r.stderr[-300:]}"


def write_lut(ws: Path, name: str = "test.cube") -> str:
    """4³ .cube 夹具:全通道 ×0.25(压暗;行序 b→g→r,r 最内,与 Rust 解析器一致)。"""
    rows = []
    for b in range(4):
        for g in range(4):
            for r_ in range(4):
                rows.append(f"{(r_ / 3) * 0.25:.6f} {(g / 3) * 0.25:.6f} {(b / 3) * 0.25:.6f}")
    (ws / name).write_text('TITLE "e2e g-halve"\nLUT_3D_SIZE 4\n' + "\n".join(rows) + "\n",
                           encoding="utf-8")
    return name


def server_rev(port: int, token: str, root: str) -> int:
    return rpc(port, token, "project_get", {"root": root})["data"]["rev"]


def oplog_count(port: int, token: str, root: str) -> int:
    return rpc(port, token, "oplog_tail", {"root": root, "limit": 1})["data"]["count"]


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


def clip_row(port: int, token: str, root: str, clip_id: str) -> dict:
    tl = rpc(port, token, "timeline_get", {"root": root})["data"]["clips"]
    return next(c for c in tl if c["id"] == clip_id)


def render_frame_png(port: int, token: str, root: str, at_ms: int) -> bytes:
    r = rpc(port, token, "render_frame", {"root": root, "atMs": at_ms, "format": "png"},
            timeout=RENDER_TIMEOUT_S)
    assert r["ok"], f"render_frame({at_ms}): {r}"
    p = Path(r["data"]["path"])
    assert p.is_file(), f"帧产物缺失: {p}"
    return p.read_bytes()


MEAN_RGB_PROBE = """async (b64) => {
  const img = new Image();
  await new Promise((res, rej) => { img.onload = res; img.onerror = () => rej(new Error("img"));
                                    img.src = "data:image/png;base64," + b64; });
  const c = document.createElement("canvas");
  c.width = img.width; c.height = img.height;
  const ctx = c.getContext("2d");
  ctx.drawImage(img, 0, 0);
  const d = ctx.getImageData(0, 0, c.width, c.height).data;
  let r = 0, g = 0, b = 0;
  const n = d.length / 4;
  for (let i = 0; i < d.length; i += 4) { r += d[i]; g += d[i + 1]; b += d[i + 2]; }
  return { r: r / n, g: g / n, b: b / n };
}"""


def mean_rgb(page, png: bytes) -> dict:
    return page.evaluate(MEAN_RGB_PROBE, base64.b64encode(png).decode("ascii"))


def expand_group(page, name: str) -> None:
    grp = page.locator(f'[data-testid="insp-group-{name}"]')
    for _ in range(3):
        if "collapsed" not in (grp.get_attribute("class") or ""):
            return
        grp.locator("legend").click()
        page.wait_for_timeout(250)
    assert "collapsed" not in (grp.get_attribute("class") or ""), f"检查器组「{name}」无法展开"


WHEEL_MOVE_PX = 6   # 拖拽位移(> gesture-kit THRESHOLD_PX=3;终值按末位指针换算)


def wheel_drag(page, testid: str, dx: float, dy: float) -> tuple[float, float]:
    """色轮拖拽:内部像素坐标换算(中心 + (dx,dy) → +6px 松手;返回末位偏移)。

    gesture-kit 位移过阈值(3px)才算拖拽、松手才走 onCommit;按下即应用但
    未过阈值的「点击」不提交(色轮无 click 语义)——e2e 必须真实拖过阈值。
    """
    loc = page.locator(f'[data-testid="{testid}"]')
    loc.scroll_into_view_if_needed()
    box = loc.bounding_box()
    assert box, f"{testid} 不可达"
    fx, fy = dx + WHEEL_MOVE_PX, dy
    cx = box["x"] + (WHEEL_SIZE / 2 + dx) / WHEEL_SIZE * box["width"]
    cy = box["y"] + (WHEEL_SIZE / 2 + dy) / WHEEL_SIZE * box["height"]
    tx = box["x"] + (WHEEL_SIZE / 2 + fx) / WHEEL_SIZE * box["width"]
    page.mouse.move(cx, cy)
    page.mouse.down()
    page.mouse.move(tx, cy, steps=4)
    page.mouse.up()
    return fx, fy


def wheel_channels_of(dx: float, dy: float, neutral: float, strength: float,
                      lo: float, hi: float) -> list[float]:
    """grade-wheel.js wheelToRgb + compToChannel 的 Python 同式换算。"""
    import math
    rho = min(1.0, math.hypot(dx, dy) / WHEEL_R)
    theta = math.atan2(dy, dx)
    px, py = rho * math.cos(theta), rho * math.sin(theta)
    r = (2 * px) / 3
    b = (-r + py / 0.8660254037844386) / 2
    g = (-r - py / 0.8660254037844386) / 2
    rnd4 = lambda v: min(hi, max(lo, neutral + strength * v)).__round__(4)  # noqa: E731
    return [rnd4(r), rnd4(g), rnd4(b)]


def canvas_nonbg_pixels(page, testid: str) -> int:
    """画布非背景像素数(以左上角像素为背景基准,逐像素通道差 >10 计入)。"""
    return page.evaluate(
        """(tid) => {
             const cv = document.querySelector(`[data-testid="${tid}"]`);
             const ctx = cv.getContext('2d');
             const d = ctx.getImageData(0, 0, cv.width, cv.height).data;
             const bg = [d[0], d[1], d[2]];
             let n = 0;
             for (let i = 0; i < d.length; i += 4) {
               if (Math.abs(d[i] - bg[0]) > 10 || Math.abs(d[i+1] - bg[1]) > 10
                   || Math.abs(d[i+2] - bg[2]) > 10) n += 1;
             }
             return n;
           }""", testid)


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
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-e2e-color-"))
    token = "color-e2e-token"
    proj = tmp / "color-proj"
    r = sh([str(cli), "new", str(proj), "--slug", "color-proj", "--json", "--fps", str(FPS),
            "--width", "640", "--height", "360", "--track", "video"])
    assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"
    tl_rel, mat_rel = ("05_时间线工程", "01_原始素材") if (proj / "05_时间线工程").is_dir() \
        else ("05_ir", "01_materials")
    (proj / mat_rel).mkdir(parents=True, exist_ok=True)
    make_gray_video(proj / mat_rel, "gray.mp4", 20)
    lut_rel = write_lut(proj / mat_rel)
    # V1 两段(拷贝粘贴 grade 需第二片段)
    pj = {"version": 1, "schemaVersion": "3.0.0", "slug": "color-proj", "fps": FPS,
          "canvas": {"width": 640, "height": 360},
          "tracks": [{"id": "V1", "kind": "video", "name": "V1", "clips": [
              {"id": "V1-001", "src": f"{mat_rel}/gray.mp4", "startMs": 0, "durationMs": 4000},
              {"id": "V1-002", "src": f"{mat_rel}/gray.mp4", "startMs": 4000, "durationMs": 4000},
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
            expand_group(page, "调色")

            ops0 = oplog_count(port, token, root)
            rev0 = server_rev(port, token, root)
            assert ops0 == rev0 == 0, f"夹具初始账目应为 0: ops={ops0} rev={rev0}"

            # ============ 1. 基线帧(未调色):三通道均值相等 ============
            base = mean_rgb(page, render_frame_png(port, token, root, 500))
            spread = max(base.values()) - min(base.values())
            assert spread <= 6, f"中性灰未调色帧三通道须相等(±6): {base}"
            log(f"1. 基线帧:R/G/B = {base['r']:.1f}/{base['g']:.1f}/{base['b']:.1f}(散差 {spread:.1f}): PASS")

            # ============ 2. 色轮拖(Lift):零 Op 拖拽 → 松手单 Op 整对象写回 ============
            ops = oplog_count(port, token, root)
            fx, fy = wheel_drag(page, "grade-wheel-lift", 40.0, 0.0)
            ops = settle_ops(port, token, root, ops)
            expect_lift = wheel_channels_of(fx, fy, 0.0, 1.0, -1.0, 1.0)
            row = clip_row(port, token, root, "V1-001")
            assert row.get("grade") and row["grade"].get("lift"), f"色轮写回须落 grade.lift: {row.get('grade')}"
            got = row["grade"]["lift"]
            assert all(abs(a - b) < 1e-6 for a, b in zip(got, expect_lift)), \
                f"grade.lift 逐通道对拍: got={got} expect={expect_lift}"
            assert oplog_count(port, token, root) == ops, "色轮松手必须恰好一 Op"
            log(f"2. 色轮拖 Lift:+40px → lift={got},恰 1 Op: PASS")

            # ============ 3. render_frame 帧色偏(红抬升) ============
            graded = mean_rgb(page, render_frame_png(port, token, root, 500))
            rb = graded["r"] - graded["b"]
            assert rb > 15, f"红抬升色偏须显著(meanR−meanB > 15): {graded}(基线 {base})"
            log(f"3. 帧色偏:meanR−meanB = {rb:.1f}(基线 {spread:.1f}): PASS")

            # ============ 4. LUT 导入应用:lut_import → grade.lut → 帧 G 塌陷 ============
            page.fill('[data-testid="grade-lut-src"]', f"{mat_rel}/{lut_rel}")
            ops = oplog_count(port, token, root)
            page.click('[data-testid="grade-lut-import"]')
            ops = settle_ops(port, token, root, ops)
            row = clip_row(port, token, root, "V1-001")
            assert row["grade"].get("lut") == ".cutforge/luts/test.cube", \
                f"grade.lut 须落投影: {row['grade']}"
            assert oplog_count(port, token, root) == ops, "LUT 应用必须恰好一 Op"
            lutted = mean_rgb(page, render_frame_png(port, token, root, 500))
            assert lutted["g"] < graded["g"] * 0.6 and lutted["g"] < lutted["r"], \
                f"LUT(G×0.25)后 G 须塌陷: 前 {graded} → 后 {lutted}"
            log(f"4. LUT 导入应用:grade.lut 落投影,帧 R {graded['r']:.1f}→{lutted['r']:.1f}、G {graded['g']:.1f}→{lutted['g']:.1f}: PASS")

            # ============ 5. 示波器三画布非空(采样精确帧,含调色) ============
            page.click('[data-testid="scopes-toggle"]')
            page.wait_for_timeout(300)
            page.click('[data-testid="scopes-sample"]')
            page.wait_for_function(
                """() => document.querySelector('[data-testid="scopes-status"]').textContent.startsWith('已采样')""",
                timeout=30000)
            for tid, label in (("scope-wave", "亮度波形"), ("scope-vector", "矢量"), ("scope-hist", "直方图")):
                n = canvas_nonbg_pixels(page, tid)
                assert n > 20, f"示波器「{label}」画布须非空: 非背景像素 {n}"
                log(f"5. 示波器「{label}」非背景像素 {n}: PASS")
            # 三类数据域标注(精确帧含调色)
            note = page.inner_text('[data-testid="scopes-note"]')
            assert "精确帧" in note, f"示波器数据域标注须为精确帧(含调色): {note!r}"

            # ============ 6. 分屏割线拖动:50% → 随位移变化 ============
            # 就绪信号 = 基准帧 img.src 非空(setOpen 自动抓基准)。状态条文本不作为
            # 信号:compare.js 的 statusEl 从未赋值(恒 null,全部 if (statusEl) 守卫
            # 跳过)——壳侧真实缺陷,apps/web 本册禁碰,已登记 FE 遗留,e2e 不锚定。
            page.click('[data-testid="compare-toggle"]')
            page.wait_for_function(
                """() => { const i = document.querySelector('[data-testid="compare-img-base"]');
                           return i && !!i.getAttribute('src'); }""", timeout=120000)
            before = page.evaluate(
                """() => document.querySelector('[data-testid="compare-divider"]').style.left""")
            dloc = page.locator('[data-testid="compare-divider"]')
            dloc.scroll_into_view_if_needed()
            dbox = dloc.bounding_box()
            assert dbox, "分割线不可达"
            dx0, dy0 = dbox["x"] + dbox["width"] / 2, dbox["y"] + dbox["height"] / 2
            page.mouse.move(dx0, dy0)
            page.mouse.down()
            page.mouse.move(dx0 + 200, dy0, steps=6)
            page.mouse.up()
            after = page.evaluate(
                """() => document.querySelector('[data-testid="compare-divider"]').style.left""")
            assert before != after and after.endswith("%"), f"割线拖动须改变分割比: {before} → {after}"
            log(f"6. 分屏割线拖动:left {before} → {after}(+200px): PASS")
            page.click('[data-testid="compare-toggle"]')  # 关面板复位
            page.click('[data-testid="scopes-toggle"]')

            # ============ 7. 拷贝粘贴 grade:V1-001 → V1-002 单 Op 等价 ============
            page.click('[data-testid="grade-copy"]')
            page.wait_for_timeout(200)
            assert oplog_count(port, token, root) == ops, "grade-copy 不得产 Op"
            page.click('[data-testid="clip"][data-id="V1-002"]')
            page.wait_for_timeout(400)
            expand_group(page, "调色")
            ops = oplog_count(port, token, root)
            page.click('[data-testid="grade-paste"]')
            ops = settle_ops(port, token, root, ops)
            g1 = clip_row(port, token, root, "V1-001")["grade"]
            g2 = clip_row(port, token, root, "V1-002")["grade"]
            assert g2 and g2 == g1, f"粘贴后 grade 须逐字段等价: V1-001={g1} V1-002={g2}"
            assert oplog_count(port, token, root) == ops, "grade-paste 必须恰好一 Op"
            log(f"7. 拷贝粘贴 grade:V1-002 获得整对象(lift+lut),恰 1 Op: PASS")

            # ============ 8. 账目闭合 ============
            rev = server_rev(port, token, root)
            assert oplog_count(port, token, root) == rev, \
                f"OpLog({oplog_count(port, token, root)}) 与 rev({rev}) 不一致"
            assert not pageerrors, f"页面 JS 异常: {pageerrors[:5]}"
            browser.close()

        print(f"AC-5.2 调色壳侧闭环: PASS(耗时 {time.time() - t0:.1f}s)")
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
