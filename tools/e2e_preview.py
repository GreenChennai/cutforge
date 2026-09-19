#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""E2 预览门禁:浏览器端到端(以服务端状态与媒体元素为断言依据,约定同 e2e_edit_ops)。

    python tools/e2e_preview.py [--bin target/debug/cutforge-mcp.exe] [--cli target/debug/cutforge-cli.exe]

断言链(E2-7 验收判据):
  1. /media 端点:无 token → 401;路径穿越 → 拒绝(非 200);
  2. <video>/<audio> 达到可播放状态(readyState ≥ 2);
  3. 标尺 seek 到 t=5000ms 后,媒体 currentTime 与投影映射对齐(容差 ≤ 1 帧);
  4. 预览 canvas 采样像素非全黑;
  5. 空格播放 → 播放头推进;再按暂停;
  6. check-shell-purity 仍绿(壳不持有真相)。
退出码:0 通过 / 2 失败。依赖:playwright(chromium)+ ffmpeg(现场生成媒体夹具)。
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
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
FIXTURE = REPO / "tests" / "fixtures" / "real_ir" / "project.json"
FPS = 30
FRAME_S = 1.0 / FPS
# 夹具工程第二段 clip:startMs=4000 sourceInMs=5200;t=5000 → 6.2s
SEEK_MS = 5000
EXPECT_S = 6.2

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")


def rpc(port: int, token: str, name: str, args: dict) -> dict:
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": name, "arguments": args}}).encode()
    req = urllib.request.Request(f"http://127.0.0.1:{port}/rpc", data=body,
                                 headers={"Content-Type": "application/json",
                                          "Authorization": f"Bearer {token}"})
    out = json.loads(urllib.request.urlopen(req, timeout=10).read())
    return json.loads(out["result"]["content"][0]["text"])


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def make_media(ws: Path) -> None:
    """现场生成非全黑、带音轨的媒体夹具(testsrc2 画面 + 440Hz 正弦)。
    必须带音轨:渲染混音按 [x:a] 取流,无音轨素材会令 mix 段提取直接失败。"""
    src_dir = ws / "01_materials"
    src_dir.mkdir(parents=True, exist_ok=True)
    r = subprocess.run(
        ["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
         "-i", "testsrc2=size=320x180:rate=30", "-f", "lavfi",
         "-i", "sine=frequency=440:duration=10",
         "-t", "10", "-pix_fmt", "yuv420p",
         "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
         str(src_dir / "take1.mp4")],
        capture_output=True, text=True)
    if r.returncode != 0 or not (src_dir / "take1.mp4").is_file():
        raise AssertionError(f"ffmpeg 生成媒体夹具失败:{r.stderr[-300:]}")


def check_media_endpoint(port: int, token: str, ws: Path) -> None:
    """E2-1 安全与 Range 契约(不经浏览器,直连断言)。"""
    base = f"http://127.0.0.1:{port}"
    # 无 token → 401(数据面鉴权)
    try:
        urllib.request.urlopen(f"{base}/media?path=01_materials/take1.mp4", timeout=5)
        raise AssertionError("/media 无 token 应 401")
    except urllib.error.HTTPError as e:
        assert e.code == 401, f"/media 无 token 应 401,实得 {e.code}"
    # 路径穿越 → 非 200
    for bad in ("../../etc/passwd", "..%2F..%2Fproject.json", "05_ir/project.json"):
        req = urllib.request.Request(f"{base}/media?path={bad}",
                                     headers={"Authorization": f"Bearer {token}"})
        try:
            r = urllib.request.urlopen(req, timeout=5)
            code, body_len = r.status, len(r.read())
        except urllib.error.HTTPError as e:
            code, body_len = e.code, 0
        if bad == "05_ir/project.json":
            assert code == 200, f"工程内文件应可读,实得 {code}"
        else:
            assert code != 200, f"穿越路径 {bad} 必须被拒绝,实得 {code}"
    # Range → 206 + Content-Range
    req = urllib.request.Request(f"{base}/media?path=01_materials/take1.mp4",
                                 headers={"Authorization": f"Bearer {token}",
                                          "Range": "bytes=0-1023"})
    r = urllib.request.urlopen(req, timeout=5)
    assert r.status == 206 and r.headers.get("Content-Range", "").startswith("bytes 0-1023/"), \
        f"Range 契约:r.status={r.status}, Content-Range={r.headers.get('Content-Range')}"
    assert len(r.read()) == 1024
    _ = ws


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=None)
    ap.add_argument("--cli", default=None)
    args = ap.parse_args()
    bin_path = Path(args.bin) if args.bin else REPO / "target" / "debug" / "cutforge-mcp.exe"
    if not bin_path.is_file():
        bin_path = REPO / "target" / "release" / "cutforge-mcp"
    if not bin_path.is_file():
        print("FAIL: 先 cargo build(cutforge-mcp)", file=sys.stderr)
        return 2
    cli_path = Path(args.cli) if args.cli else REPO / "target" / "debug" / "cutforge-cli.exe"
    if not cli_path.is_file():
        cli_path = REPO / "target" / "release" / "cutforge-cli"

    from playwright.sync_api import sync_playwright

    tmp = Path(tempfile.mkdtemp(prefix="cutforge-pv-e2e-"))
    token = "pv-e2e-token"
    ws = tmp / "ws"
    (ws / "05_ir").mkdir(parents=True)
    shutil.copy(FIXTURE, ws / "05_ir" / "project.json")
    make_media(ws)

    port = free_port()
    serve = subprocess.Popen(
        [str(bin_path), "serve", "--root", str(ws), "--port", str(port),
         "--token", token, "--web", str(REPO / "apps" / "web")],
        stdout=subprocess.DEVNULL, stderr=open(tmp / "serve.log", "wb"))
    try:
        deadline = time.time() + 8
        ready = False
        while time.time() < deadline:
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{port}/session?token={token}", timeout=1).read()
                ready = True
                break
            except Exception:
                time.sleep(0.15)
        assert ready, "serve 未就绪"

        check_media_endpoint(port, token, ws)
        print("E2-1 /media 端点(鉴权/穿越/Range): PASS")

        with sync_playwright() as pw:
            browser = pw.chromium.launch(args=["--autoplay-policy=no-user-gesture-required"])
            page = browser.new_page(viewport={"width": 1600, "height": 1000})
            page.goto(f"http://127.0.0.1:{port}/?token={token}")
            page.wait_for_function("document.getElementById('rev').textContent !== '-'", timeout=20000)

            # 2. 媒体就绪
            page.wait_for_function(
                """() => {
                    const m = [...document.querySelectorAll('#pv-media video, #pv-media audio')];
                    return m.length >= 4 && m.some((e) => e.readyState >= 2);
                }""", timeout=20000)
            print("E2-3 媒体元素就绪(readyState≥2): PASS")

            # 3. 标尺 seek → 媒体 currentTime 对齐(≤1 帧)
            page.click("#ruler", position={"x": int(SEEK_MS * 0.06), "y": 10})
            page.wait_for_function(
                f"""() => {{
                    const hit = [...document.querySelectorAll('#pv-media video, #pv-media audio')]
                        .some((e) => Math.abs(e.currentTime - {EXPECT_S}) <= {FRAME_S});
                    return hit;
                }}""", timeout=8000)
            cur = page.evaluate(
                """() => [...document.querySelectorAll('#pv-media video, #pv-media audio')]
                    .map((e) => Number(e.currentTime.toFixed(3)))""")
            assert any(abs(c - EXPECT_S) <= FRAME_S for c in cur), f"currentTime 对齐失败: {cur}"
            print(f"E2-3 seek 对齐(t={SEEK_MS}ms → 期望 {EXPECT_S}s,容差 {FRAME_S:.4f}s): PASS {cur}")

            # 4. canvas 非全黑
            page.wait_for_timeout(400)  # 让 rAF 画一帧
            nonblack = page.evaluate(
                """() => {
                    const cv = document.getElementById('pv-canvas');
                    const ctx = cv.getContext('2d');
                    const d = ctx.getImageData(0, 0, cv.width, cv.height).data;
                    let n = 0, total = 0;
                    for (let i = 0; i < d.length; i += 401 * 4 * 37) {
                        total++;
                        if (d[i] + d[i + 1] + d[i + 2] > 60) n++;
                    }
                    return {n, total};
                }""")
            assert nonblack["total"] > 0 and nonblack["n"] / nonblack["total"] > 0.5, \
                f"canvas 画面疑似全黑: {nonblack}"
            print(f"E2-4 canvas 非全黑(采样 {nonblack['n']}/{nonblack['total']}): PASS")

            # 5. 空格播放 → 播放头推进
            before = float(page.inner_text("#playhead-ms"))
            page.keyboard.press("Space")
            page.wait_for_timeout(800)
            after = float(page.inner_text("#playhead-ms"))
            assert after > before, f"播放头未推进:{before} → {after}"
            page.keyboard.press("Space")  # 暂停
            print(f"E2-5 空格播放推进播放头({before:.0f} → {after:.0f}ms): PASS")
            browser.close()

        # 6. E5 导出闭环:编辑器内点导出(cutforge 后端)→ 06_output 产物出现
        #    (重开一个页面:上一节浏览器已关;导出走 render_run/render_progress 异步轮询)
        with sync_playwright() as pw:
            browser = pw.chromium.launch(args=["--autoplay-policy=no-user-gesture-required"])
            page = browser.new_page(viewport={"width": 1600, "height": 1000})
            page.goto(f"http://127.0.0.1:{port}/?token={token}")
            page.wait_for_function("document.getElementById('rev').textContent !== '-'", timeout=20000)
            page.select_option("#exp-backend", "cutforge")
            page.click("#exp-run")
            page.wait_for_function(
                """() => {
                    const t = document.getElementById('exp-progress').textContent;
                    return t.includes('完成') || t.includes('失败');
                }""", timeout=300000)
            prog = page.inner_text("#exp-progress")
            assert "完成" in prog and "final_cutforge_" in prog, f"导出失败:{prog}"
            print(f"E5 编辑器内导出(cutforge 后端): PASS {prog.strip()[:90]}")
            browser.close()

        probe = rpc(port, token, "render_probe", {"root": str(ws)})
        outs = [f["file"] for f in probe["data"]["files"] if str(f["file"]).startswith("final_cutforge_")]
        assert outs, f"render_probe 未见到产物:{probe['data']['files']}"
        ffprobe = shutil.which("ffprobe")
        if ffprobe:
            dur = subprocess.run(
                [ffprobe, "-v", "error", "-print_format", "json", "-show_format",
                 str(ws / "06_output" / outs[0])],
                capture_output=True, text=True)
            d = float(json.loads(dur.stdout)["format"]["duration"])
            assert abs(d - 8.4) <= 0.4, f"成片时长 {d:.2f}s 与时间线 8.4s 偏差超 0.4s"
            print(f"E5 产物时长断言({d:.2f}s ≈ 8.4s): PASS")
        else:
            print("E5 产物时长断言: SKIP(本机无 ffprobe;CI 已装)")

        # 7. 壳纯度仍绿
        if cli_path.is_file():
            r = subprocess.run([str(cli_path), "check-shell-purity", "--json"],
                               capture_output=True, text=True, cwd=str(REPO))
            doc = json.loads(r.stdout.strip().splitlines()[-1])
            assert doc.get("ok") and not doc["data"]["violations"], f"壳纯度破坏: {doc}"
            print("E2-7 check-shell-purity 绿: PASS")
        else:
            raise AssertionError(f"未找到 cutforge-cli,壳纯度检查无法执行:{cli_path}")
        print("E2 预览门禁: 全部 PASS")
        return 0
    finally:
        serve.terminate()
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
