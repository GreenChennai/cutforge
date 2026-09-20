#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""阶段二验收判据 e2e:「从零剪」全链 + 并发安全(纯服务端路径,全程不依赖 CutFlow)。

    python tools/e2e_from_zero.py [--bin cutforge-mcp] [--cli cutforge-cli]

断言链(副文档 02 §5 判据 1/3 + RT-1):
  1. B11:CLI `new` 在空白目录建工程(schemaVersion/稳定 id/canvas/fps);
  2. E3:media_browse 列出素材;/media/browse 与 /ui-fields 数据面端点(鉴权 401/200);
     clip_add 插入素材,durationMs 缺省由 ffprobe 探测自动填;
  3. E4:改 ≥4 个字段(逐条 clip_update,每条产生可撤销 Op);
  4. RT-1:.cutforge/session-summary.json 记录本次会话 user Op 清单 + rev 区间;
  5. E5:编辑器内导出(cutforge 后端)出片,render_probe 见产物,时长对拍;
  6. E6-3:长渲染期间 project_get/timeline_get/clip_update 均不被阻塞(逐项计时 <3s);
退出码:0 通过 / 2 失败。依赖:ffmpeg(现场生成媒体);cutforge-render 同目录可定位。
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
FPS = 30
MEDIA_S = 24          # 素材时长:足够长,渲染窗口内可做并发探测
EDIT_FIELDS = ["volume", "opacity", "scale", "durationMs"]   # E4 判据:≥4 字段
TRIM_MS = 20000       # durationMs 裁剪目标(0 → 20000;服务端投影为准)

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")


def rpc(port: int, token: str, name: str, args: dict, timeout: float = 10) -> dict:
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": name, "arguments": args}}).encode()
    req = urllib.request.Request(f"http://127.0.0.1:{port}/rpc", data=body,
                                 headers={"Content-Type": "application/json",
                                          "Authorization": f"Bearer {token}"})
    out = json.loads(urllib.request.urlopen(req, timeout=timeout).read())
    return json.loads(out["result"]["content"][0]["text"])


def http_get(url: str, token: str | None = None, timeout: float = 5):
    headers = {"Authorization": f"Bearer {token}"} if token else {}
    req = urllib.request.Request(url, headers=headers)
    try:
        r = urllib.request.urlopen(req, timeout=timeout)
        return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, b""


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=None)
    ap.add_argument("--cli", default=None)
    args = ap.parse_args()
    bin_path = Path(args.bin) if args.bin else REPO / "target" / "debug" / "cutforge-mcp.exe"
    if not bin_path.is_file():
        bin_path = REPO / "target" / "release" / "cutforge-mcp.exe"
    if not bin_path.is_file():
        print("FAIL: 先 cargo build(cutforge-mcp)", file=sys.stderr)
        return 2
    cli_path = Path(args.cli) if args.cli else next((REPO / c for c in (
        "target/debug/cutforge-cli.exe", "target/debug/cutforge-cli",
        "target/release/cutforge-cli.exe", "target/release/cutforge-cli")
        if (REPO / c).is_file()), None)
    ffprobe = shutil.which("ffprobe")
    if not shutil.which("ffmpeg"):
        print("FAIL: 本机无 ffmpeg(无法现场生成素材)", file=sys.stderr)
        return 2

    tmp = Path(tempfile.mkdtemp(prefix="cutforge-zero-e2e-"))
    token = "zero-e2e-token"
    proj = tmp / "fromzero"
    try:
        # ---------- 1. B11:空白目录新建工程(CLI 子命令;不依赖 CutFlow) ----------
        if not (cli_path and cli_path.is_file()):
            print("FAIL: 未找到 cutforge-cli(B11-1 判据要求 CLI new 建工程;先 cargo build)", file=sys.stderr)
            return 2
        r = subprocess.run(
            [str(cli_path), "new", str(proj), "--slug", "fromzero", "--json",
             "--fps", "30", "--width", "1080", "--height", "1920", "--track", "video,audio"],
            capture_output=True, text=True, encoding="utf-8", errors="replace")
        assert r.returncode == 0, f"cli new 失败: rc={r.returncode} {r.stdout} {r.stderr}"
        doc = json.loads(r.stdout.strip().splitlines()[-1])
        assert doc["ok"], f"cli new: {doc}"
        (proj / "01_materials").mkdir(parents=True, exist_ok=True)
        project_json = proj / "05_ir" / "project.json"
        assert project_json.is_file(), "new 必须生成 05_ir/project.json"
        pj = json.loads(project_json.read_text(encoding="utf-8"))
        assert pj["schemaVersion"] == "2.0.0", f"schemaVersion: {pj.get('schemaVersion')}"
        assert [t["id"] for t in pj["tracks"]] == ["V1", "A1"], f"稳定 id: {pj['tracks']}"
        assert all(t["clips"] == [] for t in pj["tracks"]), "空工程轨道必须为空"
        print("B11-1/B11-2 新建空工程(模板/schemaVersion/稳定 id): PASS")

        # ---------- 素材夹具 ----------
        src = proj / "01_materials" / "main.mp4"
        rr = subprocess.run(
            ["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
             "-i", "testsrc2=size=640x360:rate=30", "-f", "lavfi",
             "-i", f"sine=frequency=440:duration={MEDIA_S}",
             "-t", str(MEDIA_S), "-pix_fmt", "yuv420p",
             "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest", str(src)],
            capture_output=True, text=True)
        assert rr.returncode == 0, f"ffmpeg 生成素材失败:{rr.stderr[-300:]}"

        # ---------- 起服务 ----------
        port = free_port()
        serve = subprocess.Popen(
            [str(bin_path), "serve", "--root", str(proj), "--port", str(port),
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
            root_s = str(proj)

            # ---------- 2a. /ui-fields + /media/browse 端点(数据面鉴权) ----------
            code, _ = http_get(f"http://127.0.0.1:{port}/ui-fields")
            assert code == 401, f"/ui-fields 无 token 应 401,实得 {code}"
            code, body = http_get(f"http://127.0.0.1:{port}/ui-fields", token)
            assert code == 200, f"/ui-fields 带 token 应 200,实得 {code}"
            uf = json.loads(body)
            assert "editable" in uf and len(uf["editable"]) >= 4, f"ui-fields 分组: {uf.keys()}"
            print(f"/ui-fields 单一真相源下发(分组 {len(uf['editable'])}): PASS")

            code, _ = http_get(f"http://127.0.0.1:{port}/media/browse?dir=01_materials")
            assert code == 401, f"/media/browse 无 token 应 401,实得 {code}"
            code, body = http_get(f"http://127.0.0.1:{port}/media/browse?dir=01_materials", token)
            assert code == 200, f"/media/browse 带 token 应 200,实得 {code}"
            files = json.loads(body)["files"]
            assert any(f["path"].endswith("main.mp4") for f in files), f"browse: {files}"
            print("/media/browse 端点(鉴权/列表): PASS")

            # ---------- 2b. media_browse + media_probe 工具 ----------
            b = rpc(port, token, "media_browse", {"root": root_s, "dir": "01_materials"})
            assert b["ok"] and b["data"]["total"] >= 1, f"media_browse: {b}"
            item = next(f for f in b["data"]["files"] if f["path"].endswith("main.mp4"))
            p = rpc(port, token, "media_probe", {"root": root_s, "src": item["path"]})
            assert p["ok"], f"media_probe: {p}"
            assert abs(p["data"]["durationMs"] - MEDIA_S * 1000) <= 800, f"探测时长: {p}"
            assert p["data"]["hasAudio"] and (p["data"]["width"], p["data"]["height"]) == (640, 360), f"probe: {p}"
            print(f"media_probe(时长/分辨率/音轨): PASS {p['data']['durationMs']}ms 640x360 audio")

            # ---------- 2c. clip_add 导入素材(durationMs 缺省 → 探测自动填) ----------
            add_args = {"root": root_s, "trackId": "V1", "src": item["path"], "startMs": 0,
                        "requestId": "zero-1"}
            if not ffprobe:
                add_args["durationMs"] = MEDIA_S * 1000  # 无 ffprobe 机器:显式时长(CI 有,不走此支)
            add = rpc(port, token, "clip_add", add_args)
            assert add["ok"], f"clip_add: {add}"
            rev_after_add = add["data"]["rev"]
            tl = rpc(port, token, "timeline_get", {"root": root_s})
            clip = tl["data"]["clips"][0]
            assert clip["src"].endswith("main.mp4") and clip["track"] == "V1", f"timeline: {clip}"
            if ffprobe:
                assert abs(clip["durationMs"] - MEDIA_S * 1000) <= 800, \
                    f"durationMs 缺省须由探测自动填: {clip['durationMs']}"
            print(f"E3 clip_add 导入素材(rev→{rev_after_add},时长探测自动填): PASS")

            # ---------- 3. E4:改 ≥4 个字段(逐条 clip_update,可撤销 Op) ----------
            for i, patch in enumerate([
                {"volume": 0.8}, {"opacity": 0.9}, {"scale": 1.1}, {"durationMs": TRIM_MS},
            ]):
                r2 = rpc(port, token, "clip_update",
                         {"root": root_s, "clipId": clip["id"], "patch": patch,
                          "summary": f"从零剪 e2e 第 {i + 1} 字段"})
                assert r2["ok"], f"clip_update {patch}: {r2}"
            tl = rpc(port, token, "timeline_get", {"root": root_s})
            c2 = tl["data"]["clips"][0]
            assert c2["volume"] == 0.8 and c2["opacity"] == 0.9 and c2["scale"] == 1.1, f"字段落地: {c2}"
            assert c2["durationMs"] == TRIM_MS, f"裁剪落地: {c2['durationMs']}"
            print(f"E4 检查器字段({', '.join(EDIT_FIELDS)} = 4 字段全部经 clip_update 落地): PASS")

            # ---------- 4. RT-1:会话变更摘要 ----------
            summary_path = proj / ".cutforge" / "session-summary.json"
            assert summary_path.is_file(), "RT-1:服务期间必须留会话变更摘要"
            sm = json.loads(summary_path.read_text(encoding="utf-8"))
            assert sm["revFrom"] == 0 and sm["revTo"] >= 5, f"rev 区间: {sm['revFrom']}→{sm['revTo']}"
            assert sm["userOpCount"] >= 5, f"user Op 数: {sm['userOpCount']}"
            assert all(op["actor"]["kind"] == "user" for op in sm["ops"]), "摘要必须只含 actor=human"
            kinds = {op["summary"] for op in sm["ops"]}
            assert any("clip_insert" in k for k in kinds), f"摘要须含插入 Op: {kinds}"
            print(f"RT-1 会话变更摘要(rev {sm['revFrom']}→{sm['revTo']},{sm['userOpCount']} 个 user Op): PASS")

            # ---------- 5+6. E5 导出 + E6-3 渲染期间并发不阻塞 ----------
            rr2 = rpc(port, token, "render_run", {"root": root_s, "backend": "cutforge"})
            assert rr2["ok"], f"render_run: {rr2}(cutforge-render 需可定位)"
            run_id = rr2["data"]["runId"]

            # E6-3:长渲染期间查询/编辑不被阻塞(逐项计时;阈值 3s 远大于正常耗时)
            latencies = {}
            for name, tool_args in [
                ("project_get", {"root": root_s}),
                ("timeline_get", {"root": root_s}),
                ("oplog_tail", {"root": root_s, "limit": 5}),
            ]:
                t0 = time.time()
                r3 = rpc(port, token, name, tool_args, timeout=5)
                latencies[name] = round(time.time() - t0, 2)
                assert r3["ok"], f"渲染期间 {name}: {r3}"
            t0 = time.time()
            r3 = rpc(port, token, "clip_update",
                     {"root": root_s, "clipId": clip["id"], "patch": {"volume": 0.7},
                      "summary": "渲染期间的编辑"}, timeout=5)
            latencies["clip_update"] = round(time.time() - t0, 2)
            assert r3["ok"], f"渲染期间编辑必须可进行: {r3}"
            for name, sec in latencies.items():
                assert sec < 3.0, f"渲染期间 {name} 耗时 {sec}s ≥3s,疑似被阻塞"
            print(f"E6-3 长渲染期间查询/编辑不阻塞{latencies}: PASS")

            # 等渲染完成 → 产物对拍
            deadline = time.time() + 180
            state_, output = "running", None
            while time.time() < deadline:
                s = rpc(port, token, "render_progress", {"root": root_s, "runId": run_id})
                state_ = s["data"]["state"]
                output = s["data"].get("output")
                if state_ != "running":
                    break
                time.sleep(1)
            assert state_ == "ok", f"渲染未成功: {state_}"
            assert output and (proj / output).is_file(), f"产物: {output}"
            rp = rpc(port, token, "render_probe", {"root": root_s})
            assert any(str(f["file"]) in output for f in rp["data"]["files"]), f"render_probe: {rp['data']}"
            if ffprobe:
                dur = json.loads(subprocess.run(
                    [ffprobe, "-v", "error", "-print_format", "json", "-show_format", str(proj / output)],
                    capture_output=True, text=True).stdout)["format"]["duration"]
                assert abs(float(dur) - TRIM_MS / 1000) <= 1.2, f"成片时长 {dur}s vs 时间线 {TRIM_MS / 1000}s"
                print(f"E5 导出成片({output},{float(dur):.2f}s ≈ {TRIM_MS / 1000:.0f}s): PASS")
            else:
                print(f"E5 导出成片({output}): PASS(本机无 ffprobe,时长对拍跳过)")
            return 0
        finally:
            serve.terminate()
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
