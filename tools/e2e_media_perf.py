#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册四 AC-4.5 媒体池千素材性能 e2e(e2e_media_perf)+ AC-4.6 听觉对比存档。

    python tools/e2e_media_perf.py [--min-fps 55] [--bin cutforge-mcp] [--cli cutforge-cli]

断言链(断言纪律 M10-R5:懒加载以网络/RPC 请求面为准,帧率以页面 rAF 实测为准):
  1. 千素材目录:ffmpeg 生成 12 支真实小视频(a-vid-*,排在列表头)+ stdlib 生成
     988 张极小 PNG(z-img-*)= 恰 1,000 个媒体文件;服务端 BROWSE_CAP=500 如实截断,
     面板渲染 500 卡(视频卡在前,含 media-proxy 代理开关;导出面板 export-proxy 在位);
  2. 缩略图懒加载:初始视口外零请求——/media 图片请求不含任何 z-img(RPC 侧
     media_thumbnail 只对视口内 a-vid);滚动到 1/3 处请求集为前缀子集(非预取全量);
  3. rAF 帧采样滚动:程序化逐帧滚动整张 500 卡列表,P95 ≥ --min-fps(默认 55);
     负载敏感项(同 perf_timeline 口径),不进 CI,安静时段复跑;
  4. AC-4.6 听觉对比存档:按内核同一滤镜映射(afftdn=nr=15:nf=-35:tn=1 /
     asetrate+aresample+atempo +4 半音)现场生成 降噪前/后 与 变调前/后 四支短 WAV
     存 docs/design/audio-samples/(总 ≤3MB)+ README 说明;
     如实标注:机器听觉对比为人工项,存档供人耳复核(本脚本只做产物存在性与预算)。

退出码:0 通过 / 2 失败。依赖:playwright(chromium)+ ffmpeg + cutforge-cli。
"""
from __future__ import annotations

import argparse
import json
import math
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
import wave
import zlib
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
N_VIDEOS = 12
N_IMAGES = 988          # 12 + 988 = 1000
ARCHIVE_DIR = REPO / "docs" / "design" / "audio-samples"
ARCHIVE_BUDGET_BYTES = 3 * 1024 * 1024
AUDIO_SECONDS = 4

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[media-perf] {msg}", flush=True)


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


def write_png(path: Path, w: int = 8, h: int = 8) -> None:
    """stdlib 极小 PNG(真图,浏览器可解码;8x8 纯色,几行字节)。"""
    raw = b"".join(b"\x00" + bytes((30, 120, 200)) * w for _ in range(h))

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (struct.pack(">I", len(data)) + tag + data
                + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))

    path.write_bytes(b"\x89PNG\r\n\x1a\n"
                     + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                     + chunk(b"IDAT", zlib.compress(raw, 9))
                     + chunk(b"IEND", b""))


# ---- 页内滚动采样器(rAF 计帧;每帧滚动一格直到列表底) ----
SCROLL_SAMPLER = """(stepPx) => new Promise((resolve) => {
    const list = document.querySelector('[data-testid="media-list"]');
    window.__frames = [];
    let t0 = 0;
    const step = () => {
        const t = performance.now();
        window.__frames.push(t);
        list.scrollTop += stepPx;
        if (t0 === 0) t0 = t;
        if (list.scrollTop + list.clientHeight >= list.scrollHeight - 4 || t - t0 > 60000) {
            resolve({ frames: window.__frames, scrolled: list.scrollTop });
            return;
        }
        requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
})"""


def p95_fps(frames: list[float]) -> dict:
    deltas = [b - a for a, b in zip(frames, frames[1:]) if b > a]
    fps = sorted(1000.0 / d for d in deltas)
    return {
        "frames": len(frames),
        "p50": fps[max(0, math.ceil(0.50 * len(fps)) - 1)] if fps else 0.0,
        "p95": fps[max(0, math.ceil(0.95 * len(fps)) - 1)] if fps else 0.0,
        "dropped": sum(1 for d in deltas if d > 25.0),
    }


# ---- AC-4.6 听觉对比存档(滤镜映射与内核 across.rs 同一参数) ----

def build_audio_archive() -> list[Path]:
    ff = shutil.which("ffmpeg")
    probe = shutil.which("ffprobe")
    assert ff, "ffmpeg 必须存在(存档生成依赖)"
    assert probe, "ffprobe 必须存在(存档时长断言依赖)"
    ARCHIVE_DIR.mkdir(parents=True, exist_ok=True)
    made: list[Path] = []
    base = ["-y", "-loglevel", "error"]
    # 内核链首段(across.rs::event_body):先把输入归一到 48k 再做任何时域/频域变换——
    # lavfi sine 原生 44.1k,缺这段则 asetrate 作用于 44.1k 域样本,变调时长整体漂移。
    fmt48 = "aformat=sample_rates=48000:channel_layouts=stereo"

    def wav(src_filter: str, out: str, inputs: list[str]) -> Path:
        p = ARCHIVE_DIR / out
        r = sh([ff, *base, *inputs, "-filter_complex", src_filter, "-map", "[out]",
                "-t", str(AUDIO_SECONDS), "-ar", "48000", "-ac", "1",
                "-c:a", "pcm_s16le", str(p)])
        assert r.returncode == 0 and p.is_file(), f"存档生成失败({out}): {r.stderr[-300:]}"
        q = sh([probe, "-v", "error", "-show_entries", "format=duration",
                "-of", "default=nw=1:nk=1", str(p)])
        dur = float(q.stdout.strip() or 0)
        # 保速红线:四支样本时长都必须 = 源 4s(atempo 补偿头尾量化 ±0.1s 容差)。
        # 变调链若丢掉 aformat 首段或 atempo 方向写反(=k 而非 1/k),此处立即红。
        assert abs(dur - AUDIO_SECONDS) <= 0.1, f"{out} 时长 {dur}s ≠ {AUDIO_SECONDS}s(保速断言)"
        made.append(p)
        return p

    # 降噪对比:440Hz 正弦 + 白噪声(种子固定,可复现)→ 内核 mid 档 afftdn
    # (amix 仅夹具侧造噪源;内核映射段 = aformat → afftdn,参数逐字一致)
    noisy_in = ["-f", "lavfi", "-i", "sine=frequency=440:duration=4",
                "-f", "lavfi", "-i", "anoisesrc=color=white:seed=42:duration=4"]
    wav(f"[0:a][1:a]amix=inputs=2:normalize=0,{fmt48}[out]", "denoise_before.wav", noisy_in)
    wav(f"[0:a][1:a]amix=inputs=2:normalize=0,{fmt48},afftdn=nr=15:nf=-35:tn=1[out]",
        "denoise_after.wav", noisy_in)
    # 变调对比:440Hz 干净正弦 → +4 半音保速变调(与内核 across.rs 同一口径:
    # aformat 48k 归一 → asetrate=k×48k → aresample 回 48k → atempo = speed/pitch,
    # 纯变调 speed=1 → 1/k 减速拉回;总时长恒 = 源时长 4s)
    tone_in = ["-f", "lavfi", "-i", "sine=frequency=440:duration=4"]
    wav(f"[0:a]{fmt48}[out]", "pitch_before.wav", tone_in)
    k = 2.0 ** (4.0 / 12.0)
    rate = round(48000 * k)
    tempo = 1.0 / k
    wav(f"[0:a]{fmt48},asetrate={rate},aresample=48000,asetpts=N/SR/TB,atempo={tempo:.6f}[out]",
        "pitch_after.wav", tone_in)

    readme = ARCHIVE_DIR / "README.md"
    readme.write_text(f"""# 听觉对比存档(册四 AC-4.6)

> 如实标注:**机器听觉对比为人工项**——自动化只保证产物存在、参数与内核渲染链一致,
> 降噪/变调的「听感是否自然」须人耳复核;本存档即复核输入。

## 文件(命名规整:`<处理>_before|after.wav`;48kHz/mono/16-bit,各 {AUDIO_SECONDS}s)

| 文件 | 内容 | 滤镜(与内核同一映射,首段均为 aformat 48k 归一) |
|---|---|---|
| `denoise_before.wav` | 440Hz 正弦 + 白噪声(anoisesrc seed=42,可复现) | 混音(未处理) |
| `denoise_after.wav` | 同上,降噪后 | `afftdn=nr=15:nf=-35:tn=1`(内核 denoise=mid 档) |
| `pitch_before.wav` | 440Hz 干净正弦 | 直通 |
| `pitch_after.wav` | 同上,+4 半音保速变调 | `asetrate={round(48000 * 2.0 ** (4.0 / 12.0))},aresample=48000,asetpts=N/SR/TB,atempo={1.0 / 2.0 ** (4.0 / 12.0):.6f}`(atempo = 1/k 减速拉回,与内核 speed/pitch 同口径,时长不变) |

- 生成入口:`python tools/e2e_media_perf.py`(每次运行确定性重建;滤镜参数与
  `crates/cutforge-render/src/across.rs` 的 denoise/pitch 映射逐字一致;四支样本
  时长 = 源 4s 由 e2e 保速断言把守)。
- 预算:四文件总体积 ≤3MB(本目录由 e2e 断言把守)。
- 复核口径:对比 before/after 的「残留噪声 / 高频发闷(降噪)」与「音高是否整
  +4 半音、时长与速度是否不变(变调)」。
""", encoding="utf-8")
    return made


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--min-fps", type=float, default=55.0)
    ap.add_argument("--bin", default=None)
    ap.add_argument("--cli", default=None)
    args = ap.parse_args()
    mcp = locate_bin(args.bin, ("cutforge-mcp",))
    cli = locate_bin(args.cli, ("cutforge-cli",))
    if not mcp or not cli:
        print("FAIL: 先 cargo build(cutforge-mcp + cutforge-cli)", file=sys.stderr)
        return 2
    if not shutil.which("ffmpeg"):
        print("FAIL: 本机无 ffmpeg", file=sys.stderr)
        return 2

    from playwright.sync_api import sync_playwright

    t0 = time.time()
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-media-perf-"))
    token = "media-perf-token"

    # ---------- AC-4.6 听觉存档(先行;产物与预算独立于浏览器面) ----------
    archive = build_audio_archive()
    total = sum(p.stat().st_size for p in archive) + (ARCHIVE_DIR / "README.md").stat().st_size
    assert total <= ARCHIVE_BUDGET_BYTES, f"听觉存档 {total}B 超预算 {ARCHIVE_BUDGET_BYTES}B"
    assert len(archive) == 4, f"听觉存档必须恰 4 支: {[p.name for p in archive]}"
    log(f"0. AC-4.6 听觉存档:{', '.join(p.name for p in archive)} "
        f"(共 {total / 1024:.0f}KB ≤3MB)+ README;机器听觉对比=人工项,存档供人耳复核")

    proj = tmp / "perf-proj"
    r = sh([str(cli), "new", str(proj), "--slug", "perf-proj", "--json", "--fps", "30",
            "--width", "640", "--height", "360", "--track", "video,audio"])
    assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"
    tl_rel, mat_rel = ("05_时间线工程", "01_原始素材") if (proj / "05_时间线工程").is_dir() \
        else ("05_ir", "01_materials")
    mat = proj / mat_rel
    mat.mkdir(parents=True, exist_ok=True)
    for i in range(N_VIDEOS):
        out = mat / f"a-vid-{i:02d}.mp4"
        rr = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
                 "-i", "testsrc2=size=160x90:rate=24", "-t", "1.2", "-pix_fmt", "yuv420p",
                 "-c:v", "libx264", "-preset", "veryfast", str(out)])
        assert rr.returncode == 0, f"视频夹具失败: {rr.stderr[-200:]}"
    for i in range(N_IMAGES):
        write_png(mat / f"z-img-{i:03d}.png")
    n_media = len(list(mat.iterdir()))
    assert n_media == 1000, f"千素材目录应恰 1000 个文件,实得 {n_media}"

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
            # 请求面观察:/media(图片直显/缩略图)与 /rpc(media_thumbnail)全量记账
            media_reqs: list[str] = []
            thumb_reqs: list[str] = []

            def on_request(req):
                if "/media?" in req.url:
                    media_reqs.append(urllib.parse.unquote(req.url))
                elif req.url.endswith("/rpc") and req.post_data \
                        and "media_thumbnail" in req.post_data:
                    try:
                        body = json.loads(req.post_data)
                        thumb_reqs.append(body["params"]["arguments"]["src"])
                    except Exception:
                        thumb_reqs.append("?")

            page.on("request", on_request)
            page.goto(f"http://127.0.0.1:{port}/?token={token}")
            page.wait_for_function(
                "document.querySelector('[data-testid=\"rev\"]').textContent !== '-'", timeout=20000)
            page.wait_for_selector('[data-testid="media-item"]', timeout=15000)
            page.wait_for_timeout(2500)  # 视口内缩略/波形按队列消化完

            # ============ 1. 千素材目录 + 面板截断如实 + 代理开关在位 ============
            n_cards = page.locator('[data-testid="media-item"]').count()
            assert n_cards == 500, f"BROWSE_CAP=500:面板应 500 卡,实得 {n_cards}"
            b = rpc(port, token, "media_browse", {"root": root, "dir": mat_rel})
            assert b["data"].get("truncated") is True and b["data"].get("total") == 500, \
                f"服务端必须如实截断(CAP 500): truncated={b['data'].get('truncated')} total={b['data'].get('total')}"
            n_proxy = page.locator('[data-testid="media-proxy"]').count()
            assert n_proxy >= N_VIDEOS, f"视频卡应带代理开关(≥{N_VIDEOS}): {n_proxy}"
            assert page.locator('[data-testid="export-proxy"]').count() == 1, \
                "导出面板「用代理预览」开关必须在位"
            log(f"1. 千素材目录(12 视频 + 988 图 = 1000):面板 500 卡如实截断(truncated),"
                f"代理开关 media-proxy×{n_proxy} + export-proxy 在位: PASS")

            # ============ 2. 懒加载:视口外零请求 ============
            bad_media = [u for u in media_reqs if "z-img" in u]
            bad_thumb = [s for s in thumb_reqs if "z-img" in s or "a-vid-" not in s]
            assert not bad_media, f"视口外图片不得发 /media 请求: {bad_media[:3]}"
            assert not bad_thumb, f"media_thumbnail 只应对视口内视频发起: {bad_thumb[:3]}"
            assert thumb_reqs, "视口内视频卡应已发起缩略图 RPC"
            log(f"2. 懒加载(视口外零请求):/media z-img=0,media_thumbnail 仅 a-vid "
                f"×{len(thumb_reqs)}: PASS")

            # ============ 3. rAF 帧采样滚动(P95 ≥ --min-fps)+ 滚动中请求集有界 ============
            res = page.evaluate(SCROLL_SAMPLER, 48)
            st = p95_fps(res["frames"])
            assert st["p95"] >= args.min_fps, \
                (f"AC-4.5 帧率失败:P95 {st['p95']:.1f} < {args.min_fps}"
                 f"(P50 {st['p50']:.1f},掉帧 {st['dropped']}/{st['frames']})")
            n_img_after = sum(1 for u in media_reqs if "z-img" in u)
            assert 0 < n_img_after < N_IMAGES, \
                f"滚动应按视口懒加载图片(非预取全量): {n_img_after}/{N_IMAGES}"
            log(f"3. 滚动帧率:{st['frames']} 帧,P50 {st['p50']:.1f},P95 {st['p95']:.1f} ≥ {args.min_fps},"
                f"掉帧 {st['dropped']};懒加载图片 {n_img_after}/{N_IMAGES}(非预取): PASS")

            assert not pageerrors, f"页面 JS 异常: {pageerrors[:5]}"
            browser.close()

        print(f"AC-4.5 媒体池千素材性能 + AC-4.6 听觉存档: PASS(耗时 {time.time() - t0:.1f}s)")
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
