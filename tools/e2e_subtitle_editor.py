#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册四 AC-4.4 字幕编辑器全流程 e2e(e2e_subtitle_editor)。

    python tools/e2e_subtitle_editor.py [--bin cutforge-mcp] [--cli cutforge-cli]

断言链(断言纪律 M10-R5:UI 动作只作驱动,断言以服务端状态/渲染帧为准):
  1. SRT 导入(UI)→ 单 Op;SRT 导出(UI)→ 工程内文件与夹具 byte 级相等(往返零丢失);
  2. 批量替换(UI)→ 单 Op 落盘;零匹配幂等(零 Op);
  3. 新建文本(UI T 按钮)→ textStyle(RPC 确定性样式)→ render_frame 帧上文字位置
     = textStyle.x/y(topLeft 锚点,画布像素;页内 canvas 采样,FE 冒烟同法);
  4. 画布拖位置(pv-text-dragzone)→ 单 Op textStyle.x/y → 重渲帧位置随动(帧证);
  5. 卡拉OK(字幕页签 sub-karaoke)→ \\kf 逐字推进:25% 与 75% 两帧主色前沿单调前进;
  6. 花字模板(UI huazi-card/huazi-apply)→ 渲染帧换色(帧证);patch.huazi = {} 显式
     清除(册四收口 BE 语义)→ 投影 huazi 消失 + 帧回退纯文本;
  7. 投影含 fx 键(册四收口 BE 证据):timeline_get 逐 clip fx 键恒在(未挂 = null,
     挂载后 = 整对象);账目闭合(oplog == rev)+ 零页面异常。

退出码:0 通过 / 2 失败。依赖:playwright(chromium)+ ffmpeg + cutforge-cli + cutforge-render。
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
PX_PER_MS = 0.06
FPS = 30
POS_TOL_PX = 12        # 帧上文字位置断言容差(AA/字面边距)
SRT_FIXTURE = (
    "1\n00:00:05,000 --> 00:00:06,500\n第一句字幕\n"
    "\n2\n00:00:07,000 --> 00:00:08,200\n第二句字幕\n\n"
)

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

# 页内像素采样(FE 冒烟同法:canvas 2d 读取渲染帧;三类色板一次采样)
PIXEL_PROBE = """async (b64) => {
  const img = new Image();
  await new Promise((res, rej) => { img.onload = res; img.onerror = () => rej(new Error("img")); 
                                    img.src = "data:image/png;base64," + b64; });
  const c = document.createElement("canvas");
  c.width = img.width; c.height = img.height;
  const ctx = c.getContext("2d");
  ctx.drawImage(img, 0, 0);
  const d = ctx.getImageData(0, 0, c.width, c.height).data;
  const pick = (fn) => {
    let n = 0, minx = 1e9, miny = 1e9, maxx = -1, maxy = -1;
    for (let y = 0; y < c.height; y++) for (let x = 0; x < c.width; x++) {
      const i = (y * c.width + x) * 4;
      if (fn(d[i], d[i + 1], d[i + 2])) {
        n++;
        if (x < minx) minx = x; if (y < miny) miny = y;
        if (x > maxx) maxx = x; if (y > maxy) maxy = y;
      }
    }
    return { n, minx, miny, maxx, maxy };
  };
  return {
    green: pick((r, g, b) => g > 150 && r < 110 && b < 110),
    cyan: pick((r, g, b) => g > 170 && b > 170 && r < 110),
    white: pick((r, g, b) => r > 200 && g > 200 && b > 200),
  };
}"""


def log(msg: str) -> None:
    print(f"[subtitle-editor] {msg}", flush=True)


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def rpc(port: int, token: str, name: str, args: dict, timeout: float = 20) -> dict:
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


def make_black_video(ws: Path, name: str, seconds: int, size: str = "640x360") -> None:
    """纯黑底视频夹具:帧上只有文本像素可分色(位置/卡拉OK/花字全靠色板分割)。"""
    r = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
            "-i", f"color=c=black:size={size}:rate=30", "-f", "lavfi",
            "-i", "sine=frequency=440:duration=" + str(seconds),
            "-t", str(seconds), "-pix_fmt", "yuv420p",
            "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
            str(ws / name)])
    assert r.returncode == 0 and (ws / name).is_file(), f"ffmpeg 生成黑底夹具失败:{r.stderr[-300:]}"


def oplog_count(port: int, token: str, root: str) -> int:
    return rpc(port, token, "oplog_tail", {"root": root, "limit": 1})["data"]["count"]


def wait_ops_grow(port: int, token: str, root: str, prev: int, timeout_s: float = 15.0) -> int:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        n = oplog_count(port, token, root)
        if n > prev:
            return n
        time.sleep(0.15)
    raise AssertionError(f"OpLog 在 {timeout_s}s 内未从 {prev} 上涨")


def text_clips(port: int, token: str, root: str) -> list[dict]:
    tl = rpc(port, token, "timeline_get", {"root": root})["data"]["clips"]
    return sorted([c for c in tl if c["track"] == "T1"], key=lambda c: c["startMs"])


def render_frame_png(port: int, token: str, root: str, at_ms: int) -> bytes:
    r = rpc(port, token, "render_frame", {"root": root, "atMs": at_ms, "format": "png"})
    assert r["ok"], f"render_frame({at_ms}): {r}"
    p = Path(r["data"]["path"])
    assert p.is_file(), f"帧产物缺失: {p}"
    return p.read_bytes()


def sample_pixels(page, png: bytes) -> dict:
    return page.evaluate(PIXEL_PROBE, base64.b64encode(png).decode("ascii"))


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
        print("FAIL: 未找到 cutforge-render(帧证依赖)", file=sys.stderr)
        return 2
    if not shutil.which("ffmpeg"):
        print("FAIL: 本机无 ffmpeg", file=sys.stderr)
        return 2

    from playwright.sync_api import sync_playwright

    t0 = time.time()
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-sub-editor-"))
    token = "sub-editor-token"
    proj = tmp / "sub-proj"
    r = sh([str(cli), "new", str(proj), "--slug", "sub-proj", "--json", "--fps", str(FPS),
            "--width", "640", "--height", "360", "--track", "video,text"])
    assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"
    tl_rel, mat_rel = ("05_时间线工程", "01_原始素材") if (proj / "05_时间线工程").is_dir() \
        else ("05_ir", "01_materials")
    (proj / mat_rel).mkdir(parents=True, exist_ok=True)
    make_black_video(proj / mat_rel, "black.mp4", 9)
    pj = {"version": 1, "schemaVersion": "2.0.0", "slug": "sub-proj", "fps": FPS,
          "canvas": {"width": 640, "height": 360},
          "tracks": [
              {"id": "V1", "kind": "video", "name": "V1", "clips": [
                  {"id": "V1-001", "src": f"{mat_rel}/black.mp4", "startMs": 0, "durationMs": 9000}]},
              {"id": "T1", "kind": "text", "name": "T1", "clips": []},
          ]}
    (proj / tl_rel / "project.json").write_text(json.dumps(pj, ensure_ascii=False), encoding="utf-8")
    (proj / "subs.srt").write_text(SRT_FIXTURE, encoding="utf-8")

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
            page.wait_for_timeout(500)

            # ============ 1. 投影含 fx 键(册四收口 BE 证据;未挂 = null 键恒在) ============
            rows = rpc(port, token, "timeline_get", {"root": root})["data"]["clips"]
            assert rows and all("fx" in c for c in rows), \
                f"投影必须逐 clip 携带 fx 键: {[sorted(c.keys()) for c in rows[:2]]}"
            assert all(c["fx"] is None for c in rows), "未挂特效时 fx 必须为 null"
            log(f"1. 投影含 fx 键(册四收口):{len(rows)} clip 全部携带 fx=null(键恒在): PASS")

            # ============ 2. SRT 导入(UI)→ 单 Op ============
            ops0 = oplog_count(port, token, root)
            page.click('[data-testid="tab-subtitles"]')
            page.wait_for_selector('[data-testid="sub-import-src"]', timeout=5000)
            page.fill('[data-testid="sub-import-src"]', "subs.srt")
            page.click('[data-testid="sub-import-run"]')
            deadline = time.time() + 10
            while time.time() < deadline:
                if len(text_clips(port, token, root)) == 2:
                    break
                time.sleep(0.2)
            imported = text_clips(port, token, root)
            assert len(imported) == 2, f"导入应得 2 行: {imported}"
            ops = wait_ops_grow(port, token, root, ops0)
            assert oplog_count(port, token, root) == ops, "导入必须单 Op"
            assert [c["startMs"] for c in imported] == [5000, 7000], f"导入落点: {imported}"
            log(f"2. SRT 导入(UI subs.srt):恰 1 Op,2 行 @5000/@7000: PASS")

            # ============ 3. SRT 导出(UI)→ byte 级往返 ============
            page.select_option('[data-testid="sub-export-format"]', "srt")
            page.click('[data-testid="sub-export-run"]')
            out_rel = None
            deadline = time.time() + 10
            while time.time() < deadline:
                cand = proj / "06_成片输出" / "subtitles_T1.srt"
                if cand.is_file():
                    out_rel = cand
                    break
                time.sleep(0.2)
            assert out_rel, "导出文件未落盘(06_成片输出/subtitles_T1.srt)"
            exported = out_rel.read_text(encoding="utf-8")
            assert exported == SRT_FIXTURE, \
                f"SRT 往返必须 byte 级相等:\n{exported!r}\nvs\n{SRT_FIXTURE!r}"
            log(f"3. SRT 导出(UI)→ {out_rel.name}:与导入夹具 byte 级相等(往返零丢失): PASS")

            # ============ 4. 批量替换(UI)→ 单 Op;零匹配幂等 ============
            page.fill('[data-testid="sub-find"]', "第二句")
            page.fill('[data-testid="sub-replace"]', "柒贰句")
            page.click('[data-testid="sub-replace-run"]')
            wait_ops_grow(port, token, root, ops)
            ops = oplog_count(port, token, root)
            rows2 = text_clips(port, token, root)
            replaced = next(c for c in rows2 if c["startMs"] == 7000)
            assert replaced["text"] == "柒贰句字幕", f"替换未落盘: {replaced}"
            assert next(c for c in rows2 if c["startMs"] == 5000)["text"] == "第一句字幕", \
                "非命中行不得被改"
            page.fill('[data-testid="sub-find"]', "不存在词")
            page.click('[data-testid="sub-replace-run"]')
            time.sleep(0.8)
            assert oplog_count(port, token, root) == ops, "零匹配替换必须幂等零 Op"
            log(f"4. 批量替换(UI):恰 1 Op,第二句→柒贰句;零匹配幂等零 Op: PASS")

            # ============ 5. 新建文本(UI T 按钮)+ 确定性样式 → 帧证位置 ============
            page.click('[data-testid="tab-timeline"]')
            page.click('[data-testid="ruler"]', position={"x": int(2000 * PX_PER_MS), "y": 10})
            page.wait_for_function(
                "(ms) => Math.abs(Number(document.querySelector('[data-testid=\"playhead-ms\"]').textContent) - ms) <= 34",
                arg=2000, timeout=5000)
            ops0 = oplog_count(port, token, root)
            page.click('[data-testid="text-add"]')
            ops = wait_ops_grow(port, token, root, ops0)
            main_id = rpc(port, token, "timeline_get", {"root": root})["data"]
            main = next(c for c in text_clips(port, token, root) if c["startMs"] == 2000)
            main_id = main["id"]
            assert main["durationMs"] == 3000, f"T 按钮默认 3s: {main}"
            assert oplog_count(port, token, root) == ops, "text_add 必须单 Op"
            # 确定性样式:纯绿 / topLeft 锚 / 无描边无阴影 → 帧上唯一可分割色板
            st = rpc(port, token, "clip_update", {
                "root": root, "clipId": main_id,
                "patch": {"textStyle": {"fontSize": 64, "color": "#00FF00", "align": "topLeft",
                                        "x": 120, "y": 90, "outlineWidth": 0, "shadow": 0}}})
            assert st["ok"], f"textStyle 落地: {st}"
            ops = oplog_count(port, token, root)
            blob = sample_pixels(page, render_frame_png(port, token, root, 2500))["green"]
            assert blob["n"] > 80, f"绿字必须可见: {blob}"
            assert abs(blob["minx"] - 120) <= POS_TOL_PX and abs(blob["miny"] - 90) <= POS_TOL_PX, \
                f"帧上文字位置应为 topLeft@(120,90): {blob}"
            log(f"5. 新建文本(T 按钮)+ textStyle:render_frame@2500 帧上文字 topLeft="
                f"({blob['minx']},{blob['miny']}) ≈ (120,90) ±{POS_TOL_PX}: PASS")

            # ============ 6. 画布拖位置(pv-text-dragzone)→ 单 Op + 帧证随动 ============
            page.click(f'[data-testid="clip"][data-id="{main_id}"]')  # 选中 → 把手层出现
            page.wait_for_selector('[data-testid="pv-text-dragzone"]', timeout=5000)
            scale = page.evaluate(
                """() => { const c = document.getElementById('pv-canvas');
                           const r = c.getBoundingClientRect();
                           return { sx: c.width / r.width, sy: c.height / r.height }; }""")
            zone = page.locator('[data-testid="pv-text-dragzone"]').bounding_box()
            assert zone, "文本拖拽面不可达"
            dx_px, dy_px = 60, 40
            page.mouse.move(zone["x"] + zone["width"] / 2, zone["y"] + zone["height"] / 2)
            page.mouse.down()
            page.mouse.move(zone["x"] + zone["width"] / 2 + dx_px,
                            zone["y"] + zone["height"] / 2 + dy_px, steps=4)
            page.mouse.up()
            ops = wait_ops_grow(port, token, root, ops)
            main = next(c for c in text_clips(port, token, root) if c["id"] == main_id)
            want_x = 120 + round(dx_px * scale["sx"])
            want_y = 90 + round(dy_px * scale["sy"])
            assert abs(main["textStyle"]["x"] - want_x) <= 2 and abs(main["textStyle"]["y"] - want_y) <= 2, \
                f"拖位置应落 textStyle≈({want_x},{want_y}): {main['textStyle']}"
            blob = sample_pixels(page, render_frame_png(port, token, root, 2500))["green"]
            assert abs(blob["minx"] - want_x) <= POS_TOL_PX and abs(blob["miny"] - want_y) <= POS_TOL_PX, \
                f"拖位置后帧上文字应 ≈({want_x},{want_y}): {blob}"
            log(f"6. 画布拖位置(+{dx_px},+{dy_px}px):恰 1 Op,textStyle≈({want_x},{want_y}),"
                f"重渲帧 ({blob['minx']},{blob['miny']}) 随动: PASS")

            # ============ 7. 卡拉OK(字幕页签 sub-karaoke)→ 帧上主色前沿推进 ============
            page.click('[data-testid="tab-subtitles"]')
            page.wait_for_selector('[data-testid="sub-row"]', timeout=5000)
            row = page.locator('[data-testid="sub-row"]').filter(has_text="双击片段编辑文本").first
            row.locator('[data-testid="sub-karaoke"]').click()
            ops = wait_ops_grow(port, token, root, ops)
            main = next(c for c in text_clips(port, token, root) if c["id"] == main_id)
            assert main["textStyle"]["karaoke"] is True, f"卡拉OK开关未落盘: {main['textStyle']}"
            # 片段 [2000,5000) 3s 逐字 \kf:23% 帧与 73% 帧的主色(绿)前沿必须单调前进
            k25 = sample_pixels(page, render_frame_png(port, token, root, 2700))["green"]
            k75 = sample_pixels(page, render_frame_png(port, token, root, 4200))["green"]
            assert k25["n"] > 80 and k75["n"] > 80, f"卡拉OK两帧都必须有绿字: {k25} | {k75}"
            front25, front75 = k25["maxx"], k75["maxx"]
            width = max(1, blob["maxx"] - blob["minx"])
            assert front75 > front25 + 0.25 * width, \
                f"卡拉OK推进不足: 前沿 23%帧={front25} vs 73%帧={front75}(字宽 {width})"
            log(f"7. 卡拉OK(sub-karaoke):恰 1 Op;主色前沿 23%帧={front25} → 73%帧={front75}"
                f"(字宽 {width}px)单调推进: PASS")

            # ============ 8. 花字模板(UI hz.neon)→ 帧换色;patch.huazi={} 显式清除 ============
            row = page.locator('[data-testid="sub-row"]').filter(has_text="第一句字幕").first
            row.click()  # 选中导入行(字幕页签面板替换中栏,选中新在投影仍生效)
            # 检查器在工作台列(时间线页签面板内)→ 切回时间线页签再操作花字库
            page.click('[data-testid="tab-timeline"]')
            # 检查器·文本组折叠态 → 点 legend 展开(重试至花字卡可见)
            deadline = time.time() + 8
            while time.time() < deadline:
                grp = page.locator('[data-testid="insp-group-文本"]')
                if grp.count() and "collapsed" in (grp.get_attribute("class") or ""):
                    grp.locator("legend").click()
                    page.wait_for_timeout(250)
                if page.locator('[data-testid="huazi-card"][data-huazi="hz.neon"]').is_visible():
                    break
                page.wait_for_timeout(250)
            page.wait_for_selector('[data-testid="huazi-card"][data-huazi="hz.neon"]', timeout=5000)
            page.click('[data-testid="huazi-card"][data-huazi="hz.neon"]')
            page.click('[data-testid="huazi-apply"]')
            ops = wait_ops_grow(port, token, root, ops)
            imported1 = next(c for c in text_clips(port, token, root) if c["startMs"] == 5000)
            assert imported1["huazi"]["template"] == "hz.neon", f"花字未挂载: {imported1['huazi']}"
            f_hz = sample_pixels(page, render_frame_png(port, token, root, 5800))
            assert f_hz["cyan"]["n"] > 80, f"霓虹花字(青色系)必须上帧: {f_hz['cyan']}"
            assert f_hz["white"]["n"] < f_hz["cyan"]["n"], "花字主色应取代默认白"
            # 册四收口 BE 语义:patch.huazi = {} 显式清除(单 Op;undo 可还原)
            clear = rpc(port, token, "clip_update",
                        {"root": root, "clipId": imported1["id"], "patch": {"huazi": {}}})
            assert clear["ok"] and len(clear["data"]["opIds"]) == 1, f"{{}} 清除必须单 Op: {clear}"
            ops = oplog_count(port, token, root)
            imported1 = next(c for c in text_clips(port, token, root) if c["startMs"] == 5000)
            assert imported1.get("huazi") is None, f"清除后 huazi 必须消失: {imported1.get('huazi')}"
            f_plain = sample_pixels(page, render_frame_png(port, token, root, 5800))
            assert f_plain["cyan"]["n"] < 80, f"清除后帧必须回退纯文本: {f_plain['cyan']}"
            log(f"8. 花字 hz.neon(UI):上帧青色 {f_hz['cyan']['n']}px;patch.huazi={{}} 清除后 "
                f"青色 {f_plain['cyan']['n']}px(回退纯文本): PASS")

            # ============ 9. fx 投影:挂载后整对象下放(收口证据闭环) ============
            r2 = rpc(port, token, "clip_update",
                     {"root": root, "clipId": "V1-001", "patch": {"fx": {"combo": [{"fx": "fx.mono"}]}}})
            assert r2["ok"], f"fx 挂载: {r2}"
            rows = rpc(port, token, "timeline_get", {"root": root})["data"]["clips"]
            v1 = next(c for c in rows if c["id"] == "V1-001")
            assert v1["fx"] == {"combo": [{"fx": "fx.mono"}]}, f"挂载后 fx 必须整对象下放: {v1['fx']}"
            assert all("fx" in c for c in rows), "fx 键必须逐 clip 恒在"
            assert all(c["fx"] is None for c in rows if c["id"] != "V1-001"), "未挂 clip fx 仍为 null"
            log(f"9. fx 投影(挂载后):V1-001 fx=mono 整对象,其余 clip fx=null(键恒在): PASS")

            # ============ 10. 账目闭合 ============
            rev = rpc(port, token, "project_get", {"root": root})["data"]["rev"]
            assert oplog_count(port, token, root) == rev, "OpLog 与 rev 必须一致"
            assert not pageerrors, f"页面 JS 异常: {pageerrors[:5]}"
            browser.close()

        print(f"AC-4.4 字幕编辑器全流程: PASS(耗时 {time.time() - t0:.1f}s)")
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
