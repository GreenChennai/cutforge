#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册六 T6.5/AC-6.3 独立运行验收 e2e:四大件编辑链**全程零 python 子进程**(进程树断言)。

    python tools/e2e_independence.py [--bin cutforge-mcp] [--cli cutforge-cli]

断言链(计划书 06 册六 AC-6.3 + AC-7.4 预演;判定器即计划书点名的 tools/e2e_independence.py):
  1. 环境隔离:CUTFLOW_* 全部剥离后起 serve(模拟"无外部管线仓库/配置"的独立机器);
  2. 四大件(全部经 CutForge 内置工具,字幕含花字/卡点/重构图/成片导出):
     字幕 subtitle_import(SRT→文本轨卡)+ 花字 text_add(huazi 模板)+ 卡点 audio_beats
     (onset-energy 启发式)+ 重构图 clip_update patch.reframe(anchorY 契约承载,
     渲染端零消费为 ADR-0023/能力矩阵 #50 登记口径,此处断言字段防丢);
  3. AC-6.3 核心判定:渲染全程对 serve 进程子树做进程树采样(psutil 优先,缺则
     Windows wmic / POSIX /proc 兜底),断言**零 python 子进程**——
     剪映草稿导出豁免(ADR-0023:断言范围=四大件编辑链;export_jianying 为随包
     Python 脚本,用户显式调用面,本脚本不调用它);
  4. 产物校验:成片 ffprobe 时长对拍 + 像素抽样(纯绿字幕字样上帧 = 非黑 + 烧录实证);
  5. 纯 CLI 面(AC-7.4 预演,无 UI 无 serve):run-script 无头建卡 → cutforge-cli
     clip-update 改字段 → cutforge-render 直渲 → ffprobe 校验(v3 布局,产物落 exports/)。
退出码:0 通过 / 2 失败。依赖:ffmpeg(现场生成媒体);psutil 可缺(自动兜底采样)。
"""
from __future__ import annotations

import argparse
import csv
import io
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from collections import Counter
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
FPS = 30
CLIP_MS = 6000          # V1-001 裁剪目标(0 → 6000)
CANVAS = (640, 360)
SUB_COLOR = "#00FF00"   # 纯绿字幕(底为深灰,帧上绿色只能来自烧录字样)

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[independence] {msg}", flush=True)


def rpc(port: int, token: str, name: str, args: dict, timeout: float = 20) -> dict:
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": name, "arguments": args}}).encode()
    req = urllib.request.Request(f"http://127.0.0.1:{port}/rpc", data=body,
                                 headers={"Content-Type": "application/json",
                                          "Authorization": f"Bearer {token}"})
    out = json.loads(urllib.request.urlopen(req, timeout=timeout).read())
    return json.loads(out["result"]["content"][0]["text"])


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def is_python_proc(name: str) -> bool:
    """python 进程判定(大小写/扩展名不敏感:python/python3/pythonw/py)。"""
    n = name.lower().strip()
    if n.endswith(".exe"):
        n = n[:-4]
    return n == "py" or n.startswith("python")


# ---------------- 进程树采样(psutil 优先;Windows wmic / POSIX /proc 兜底) ----------------

def descendants_psutil(root_pid: int) -> dict[int, str]:
    import psutil  # 本机已装;CI 无则走 _proc/_wmic 兜底
    parent = psutil.Process(root_pid)
    return {c.pid: c.name() for c in parent.children(recursive=True)}


def descendants_wmic(root_pid: int) -> dict[int, str]:
    out = subprocess.run(["wmic", "process", "get", "ProcessId,ParentProcessId,Name", "/format:csv"],
                         capture_output=True, text=True, encoding="utf-8", errors="replace").stdout
    rows = list(csv.DictReader(io.StringIO(out)))
    names, children = {}, {}
    for r in rows:
        try:
            pid, ppid = int(r["ProcessId"]), int(r["ParentProcessId"])
        except (KeyError, TypeError, ValueError):
            continue
        names[pid] = r.get("Name") or ""
        children.setdefault(ppid, []).append(pid)
    seen: dict[int, str] = {}
    stack = list(children.get(root_pid, []))
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen[pid] = names.get(pid, "")
        stack.extend(children.get(pid, []))
    return seen


def descendants_procfs(root_pid: int) -> dict[int, str]:
    names, children = {}, {}
    for d in os.listdir("/proc"):
        if not d.isdigit():
            continue
        try:
            with open(f"/proc/{d}/stat", "rb") as f:
                stat = f.read()
            name = stat[stat.index(b"(") + 1:stat.rindex(b")")].decode("utf-8", "replace")
            ppid = int(stat[stat.rindex(b")") + 2:].split()[1])
        except (OSError, ValueError, IndexError):
            continue
        names[int(d)] = name
        children.setdefault(ppid, []).append(int(d))
    seen: dict[int, str] = {}
    stack = list(children.get(root_pid, []))
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen[pid] = names.get(pid, "")
        stack.extend(children.get(pid, []))
    return seen


class TreeSampler(threading.Thread):
    """serve 进程子树周期采样;python 违例与子进程名单(证据)双记录。"""

    def __init__(self, root_pid: int, interval: float = 0.4):
        super().__init__(daemon=True)
        self.root_pid = root_pid
        self.interval = interval
        self.cycles = 0
        self.proc_observations = 0
        self.names: Counter[str] = Counter()
        self.violations: list[str] = []
        self.errors = 0
        self._stop = threading.Event()

    def _snapshot(self) -> dict[int, str]:
        for fn in (descendants_psutil, descendants_wmic if os.name == "nt" else descendants_procfs,
                   descendants_wmic if os.name != "nt" else descendants_procfs):
            try:
                snap = fn(self.root_pid)
                if snap is not None:
                    return snap
            except ImportError:
                continue
            except Exception:  # noqa: BLE001 — 采样失败计数,不中断断言面
                self.errors += 1
                return {}
        return {}

    def run(self) -> None:
        while not self._stop.is_set():
            snap = self._snapshot()
            self.cycles += 1
            for _pid, name in snap.items():
                self.proc_observations += 1
                self.names[name] += 1
                if is_python_proc(name):
                    self.violations.append(f"pid={_pid} name={name}")
            self._stop.wait(self.interval)

    def stop(self) -> None:
        self._stop.set()


def make_media(dst: Path, seconds: int, freq: int, kind: str = "gray") -> None:
    """夹具媒体:深灰底(绿字上帧可判定)+ 纯音轨;beat 形加 2Hz 音量开关(卡点用)。"""
    vf = "color=c=0x202020" if kind == "gray" else "color=c=0x202020"
    cmd = ["ffmpeg", "-y", "-loglevel", "error",
           "-f", "lavfi", "-i", f"{vf}:size=640x360:rate={FPS}"]
    if kind == "beat":
        cmd += ["-f", "lavfi", "-i", f"sine=frequency={freq}:duration={seconds}",
                "-af", f"volume='if(lt(mod(t,0.5),0.08),1,0.02)':eval=frame"]
    else:
        cmd += ["-f", "lavfi", "-i", f"sine=frequency={freq}:duration={seconds}"]
    cmd += ["-t", str(seconds), "-pix_fmt", "yuv420p", "-c:v", "libx264",
            "-preset", "veryfast", "-c:a", "aac", "-shortest", str(dst)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    assert r.returncode == 0, f"ffmpeg 夹具失败:{r.stderr[-300:]}"


def frame_pixels(mp4: Path, at_s: float) -> list[tuple[int, int, int]]:
    """抽一帧 RGB24 原始像素(纯 python 解析,零 PIL 依赖)。"""
    w, h = 320, 180
    r = subprocess.run(["ffmpeg", "-v", "error", "-ss", str(at_s), "-i", str(mp4),
                        "-frames:v", "1", "-vf", f"scale={w}:{h}", "-f", "rawvideo",
                        "-pix_fmt", "rgb24", "-"],
                       capture_output=True)
    assert r.returncode == 0 and len(r.stdout) >= w * h * 3, f"抽帧失败: {r.stderr[-200:]}"
    raw = r.stdout[:w * h * 3]
    return [(raw[i], raw[i + 1], raw[i + 2]) for i in range(0, len(raw), 3)]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=None)
    ap.add_argument("--cli", default=None)
    args = ap.parse_args()
    bin_path = Path(args.bin) if args.bin else next(
        (REPO / "target" / prof / f"cutforge-mcp{ext}" for prof in ("debug", "release")
         for ext in (".exe", "") if (REPO / "target" / prof / f"cutforge-mcp{ext}").is_file()), None)
    cli_path = Path(args.cli) if args.cli else next(
        (REPO / "target" / prof / f"cutforge-cli{ext}" for prof in ("debug", "release")
         for ext in (".exe", "") if (REPO / "target" / prof / f"cutforge-cli{ext}").is_file()), None)
    render_path = next(
        (REPO / "target" / prof / f"cutforge-render{ext}" for prof in ("debug", "release")
         for ext in (".exe", "") if (REPO / "target" / prof / f"cutforge-render{ext}").is_file()), None)
    if not (bin_path and bin_path.is_file()):
        print("FAIL: 先 cargo build(cutforge-mcp)", file=sys.stderr)
        return 2
    if not (cli_path and cli_path.is_file() and render_path and render_path.is_file()):
        print("FAIL: 先 cargo build(cutforge-cli + cutforge-render;纯 CLI 面需要)", file=sys.stderr)
        return 2
    ffprobe = shutil.which("ffprobe")

    tmp = Path(tempfile.mkdtemp(prefix="cutforge-independence-"))
    token = "independence-token"
    # 环境隔离:剥离全部 CUTFLOW_*(外部管线仓库/配置的唯二入口)再起 serve,
    # 并保持 serve 子进程 env 干净(断言的是"独立机器"语义,不是本机 shell 卫生)
    isolated_env = {k: v for k, v in os.environ.items() if not k.upper().startswith("CUTFLOW_")}
    dropped = sorted(set(os.environ) - set(isolated_env))
    log(f"环境隔离:剥离 {len(dropped)} 个外部管线变量 {dropped}(CUTFLOW_*)")

    proj = tmp / "indep"
    mat = proj / "01_原始素材"
    try:
        # ---------- 夹具:工程(v2,含文本轨)+ 素材 + SRT ----------
        r = subprocess.run(
            [str(cli_path), "new", str(proj), "--slug", "indep", "--json", "--fps", str(FPS),
             "--width", str(CANVAS[0]), "--height", str(CANVAS[1]), "--track", "video,audio,text"],
            capture_output=True, text=True, encoding="utf-8", errors="replace")
        assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"
        tl_rel = "05_时间线工程" if (proj / "05_时间线工程").is_dir() else "05_ir"
        mat.mkdir(parents=True, exist_ok=True)
        make_media(mat / "main.mp4", 24, 440)
        make_media(mat / "beat.mp4", 8, 880, kind="beat")
        (mat / "subs.srt").write_text(
            "1\n00:00:01,000 --> 00:00:03,000\n独立验收第一句\n\n"
            "2\n00:00:03,500 --> 00:00:05,500\n第二句绿字\n",
            encoding="utf-8")
        # 重构图锚点在夹具期入盘(契约字段;编辑面 ClipPatch 未承接为登记口径,
        # AC-6.3 断言 = 四大件工具链写操作**不丢**该字段——承载防丢)
        pj_path = proj / tl_rel / "project.json"
        pj = json.loads(pj_path.read_text(encoding="utf-8"))
        pj["tracks"][0]["clips"].append(
            {"id": "V1-901", "src": "01_原始素材/seed.mp4", "startMs": 30000, "durationMs": 1000,
             "reframe": {"anchorY": 0.35}})
        pj_path.write_text(json.dumps(pj, ensure_ascii=False), encoding="utf-8")
        make_media(mat / "seed.mp4", 2, 440)
        log("夹具:工程 + main.mp4/beat.mp4/subs.srt + reframe 种子片段就绪")

        # ---------- 起 serve(隔离 env)----------
        port = free_port()
        serve = subprocess.Popen(
            [str(bin_path), "serve", "--root", str(proj), "--port", str(port),
             "--token", token, "--web", str(REPO / "apps" / "web")],
            stdout=subprocess.DEVNULL, stderr=open(tmp / "serve.log", "wb"), env=isolated_env)
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
            root_s = str(proj)

            # ---------- 进程树采样开跑(覆盖四大件 + 渲染全程) ----------
            sampler = TreeSampler(serve.pid)
            sampler.start()

            # ---------- 2-0. 导入主素材(V1;裁到 6s 作为成片窗) ----------
            add = rpc(port, token, "clip_add",
                      {"root": root_s, "trackId": "V1", "src": "01_原始素材/main.mp4",
                       "startMs": 0, "requestId": "indep-1"})
            assert add["ok"], f"clip_add: {add}"
            tl = rpc(port, token, "timeline_get", {"root": root_s})["data"]
            v1_new = next(c for c in tl["clips"] if c["track"] == "V1" and c["id"] != "V1-901")
            tr = rpc(port, token, "clip_update",
                     {"root": root_s, "clipId": v1_new["id"],
                      "patch": {"durationMs": CLIP_MS}, "summary": "独立性验收:主素材裁 6s"})
            assert tr["ok"], f"clip_update 裁时长: {tr}"
            log(f"主素材 clip_add + 裁 {CLIP_MS}ms(rev→{tr['data']['rev']}): PASS")

            # ---------- 2a. 字幕(SRT → 文本轨) ----------
            imp = rpc(port, token, "subtitle_import",
                      {"root": root_s, "src": "01_原始素材/subs.srt",
                       "textStyle": {"fontSize": 72, "color": SUB_COLOR, "outlineWidth": 0, "shadow": 0}})
            assert imp["ok"], f"subtitle_import: {imp}"
            tl = rpc(port, token, "timeline_get", {"root": root_s})["data"]
            sub_clips = [c for c in tl["clips"] if c.get("text")]
            assert len(sub_clips) >= 2, f"字幕卡必须落文本轨: {[c.get('id') for c in tl['clips']]}"
            log(f"字幕 subtitle_import(SRT 2 行 → 文本轨 {len(sub_clips)} 卡): PASS")

            # ---------- 2b. 花字(huazi 模板经 text_add;卡放 0.1–0.7s 避开字幕轨重叠 CF-004) ----------
            hz = rpc(port, token, "text_add",
                     {"root": root_s, "text": "花字标题", "atMs": 100, "durationMs": 600,
                      "huazi": {"template": "hz.neon"}})
            assert hz["ok"], f"text_add 花字: {hz}"
            tl = rpc(port, token, "timeline_get", {"root": root_s})["data"]
            hz_clip = next(c for c in tl["clips"] if c.get("text") == "花字标题")
            assert hz_clip.get("huazi", {}).get("template") == "hz.neon", f"花字未挂载: {hz_clip}"
            log("花字 text_add(hz.neon 模板,投影 huazi.template): PASS")

            # ---------- 2c. 卡点(audio_beats 启发式) ----------
            bt = rpc(port, token, "audio_beats",
                     {"root": root_s, "src": "01_原始素材/beat.mp4", "sensitivity": 0.5})
            assert bt["ok"], f"audio_beats: {bt}"
            assert bt["data"]["onsetCount"] >= 1, \
                f"脉冲音轨必须检出 onset(启发式口径): {bt['data']['onsetCount']}"
            log(f"卡点 audio_beats(onset-energy,onsetCount={bt['data']['onsetCount']},"
                f"bpm={bt['data']['bpm']},engine={bt['data']['engine']}): PASS")

            # ---------- 2d. 重构图(契约字段承载防丢;渲染零消费为登记口径) ----------
            # clip_update 写链全跑完后,回读工程真相源:reframe.anchorY 必须原样在
            # (工具链写操作不丢契约字段 = #50「渲染端零消费」之外的承载面承诺)
            tl = rpc(port, token, "timeline_get", {"root": root_s})["data"]
            v1 = next(c for c in tl["clips"] if c["track"] == "V1" and c["id"] != "V1-901")
            rf = rpc(port, token, "clip_update",
                     {"root": root_s, "clipId": v1["id"], "patch": {"volume": 0.9},
                      "summary": "独立性验收:V1 字段写链"})
            assert rf["ok"], f"V1 写链: {rf}"
            pj_now = json.loads((proj / tl_rel / "project.json").read_text(encoding="utf-8"))
            seed = next(c for t in pj_now["tracks"] for c in t["clips"] if c["id"] == "V1-901")
            assert seed.get("reframe", {}).get("anchorY") == 0.35, \
                f"契约字段被写链丢弃: {seed}"
            log("重构图 reframe.anchorY=0.35 承载防丢(工具链写链后仍在真相源): PASS"
                "(渲染端零消费为能力矩阵 #50 登记口径;编辑面 ClipPatch 未承接为只读登记)")
            # 种子片段功成身退(删除走内置工具链;成片窗收敛回 6s)
            dl = rpc(port, token, "clip_delete",
                     {"root": root_s, "clipId": "V1-901", "summary": "独立性验收:种子片段移除"})
            assert dl["ok"], f"clip_delete 种子: {dl}"

            # ---------- 导出成片(渲染全程在采样窗内) ----------
            rr = rpc(port, token, "render_run", {"root": root_s, "backend": "cutforge"})
            assert rr["ok"], f"render_run: {rr}"
            run_id = rr["data"]["runId"]
            state, output = "queued", None
            deadline = time.time() + 180
            while time.time() < deadline:
                s = rpc(port, token, "render_progress", {"root": root_s, "runId": run_id})
                state, output = s["data"]["state"], s["data"].get("output")
                if state not in ("running", "queued"):
                    break
                time.sleep(0.8)
            assert state == "ok", f"渲染未成功: {state}"
            out_file = proj / output
            assert out_file.is_file(), f"产物: {output}"

            # ---------- 3. AC-6.3 核心判定:采样收口,零 python 子进程 ----------
            time.sleep(0.8)   # 收尾一拍,覆盖渲染子进程退出瞬间
            sampler.stop()
            sampler.join(timeout=3)
            assert sampler.cycles >= 5 and sampler.proc_observations >= 1, \
                f"采样面失效(cycles={sampler.cycles}, obs={sampler.proc_observations}, err={sampler.errors})"
            assert not sampler.violations, \
                f"AC-6.3 失败:serve 子树出现 python 进程 {sampler.violations}"
            child_names = sorted(sampler.names)
            assert all(not is_python_proc(n) for n in child_names), child_names
            log(f"AC-6.3 进程树断言:采样 {sampler.cycles} 轮 / {sampler.proc_observations} 进程次,"
                f"子树进程名单 {child_names},python 违例 0: PASS"
                "(剪映导出豁免未触发——本脚本不调用 export_jianying,ADR-0023)")

            # ---------- 4. 产物校验:时长 + 像素抽样 ----------
            if ffprobe:
                dur = json.loads(subprocess.run(
                    [ffprobe, "-v", "error", "-print_format", "json", "-show_format", str(out_file)],
                    capture_output=True, text=True).stdout)["format"]["duration"]
                assert abs(float(dur) - CLIP_MS / 1000) <= 1.2, \
                    f"成片时长 {dur}s vs 时间线 {CLIP_MS / 1000}s"
                log(f"产物时长 ffprobe({float(dur):.2f}s ≈ {CLIP_MS / 1000:.0f}s): PASS")
            px = frame_pixels(out_file, 2.0)   # 字幕窗 1–3s 内
            mean_luma = sum(0.299 * r + 0.587 * g + 0.114 * b for r, g, b in px) / len(px)
            greens = sum(1 for r, g, b in px if g > 150 and r < 120 and b < 120)
            assert mean_luma > 5, f"成片帧全黑(mean luma={mean_luma:.1f})"
            assert greens >= 20, f"纯绿字幕字样必须烧录上帧(green={greens},底为深灰不可伪造)"
            log(f"产物像素抽样(t=2.0s:mean luma={mean_luma:.1f},纯绿字样 {greens} 像素): PASS")
        finally:
            serve.terminate()
            try:
                serve.wait(timeout=10)
            except subprocess.TimeoutExpired:
                serve.kill()

        # ---------- 5. 纯 CLI 面(AC-7.4 预演):无 UI 无 serve 改字段→渲染→校验 ----------
        proj2 = tmp / "cli-only"
        r = subprocess.run(
            [str(cli_path), "new", str(proj2), "--slug", "clifc", "--json", "--layout", "v3",
             "--width", str(CANVAS[0]), "--height", str(CANVAS[1]), "--track", "video,audio"],
            capture_output=True, text=True, encoding="utf-8", errors="replace")
        assert r.returncode == 0, f"cli new v3: {r.stdout} {r.stderr}"
        assert (proj2 / "media").is_dir() and (proj2 / "project.json").is_file(), "v3 扁平布局树"
        make_media(proj2 / "media" / "main.mp4", 10, 440)
        script = {"format": "cutforge-script-v1", "steps": [
            {"tool": "clip_add", "args": {"trackId": "V1", "src": "media/main.mp4",
                                          "startMs": 0, "requestId": "indep-cli-1"}}]}
        script_file = tmp / "cli-edit.json"
        script_file.write_text(json.dumps(script), encoding="utf-8")
        r = subprocess.run([str(bin_path), "run-script", "--root", str(proj2), "--file", str(script_file)],
                           capture_output=True, text=True, encoding="utf-8", errors="replace")
        rep = json.loads(r.stdout)
        assert rep["ok"] and rep["data"]["rejected"] == [] and rep["data"]["dispatched"] == 1, \
            f"run-script 无头建卡: {rep}"
        pj = json.loads((proj2 / "project.json").read_text(encoding="utf-8"))
        clip_id = pj["tracks"][0]["clips"][0]["id"]
        r = subprocess.run(
            [str(cli_path), "clip-update", str(proj2), clip_id,
             "--volume", "0.5", "--duration-ms", "4000", "--summary", "纯 CLI 面改字段", "--json"],
            capture_output=True, text=True, encoding="utf-8", errors="replace")
        assert r.returncode == 0, f"clip-update: {r.stdout} {r.stderr}"
        receipt = json.loads(r.stdout.strip().splitlines()[-1])
        assert receipt["ok"] and receipt["data"]["rev"] >= 2, f"改字段回执: {receipt}"
        r = subprocess.run([str(render_path), "--root", str(proj2)],
                           capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=300)
        assert r.returncode == 0, f"cutforge-render 直渲: {r.stdout[-300:]} {r.stderr[-300:]}"
        outs = list((proj2 / "exports").glob("*.mp4"))
        assert outs, f"v3 工程产物必须落 exports/: {list(proj2.rglob('*.mp4'))}"
        if ffprobe:
            dur = json.loads(subprocess.run(
                [ffprobe, "-v", "error", "-print_format", "json", "-show_format", str(outs[0])],
                capture_output=True, text=True).stdout)["format"]["duration"]
            assert abs(float(dur) - 4.0) <= 1.2, f"CLI 面成片时长 {dur}s vs 4s"
        log(f"纯 CLI 面(v3 布局):run-script 建卡 → clip-update 改字段(rev→{receipt['data']['rev']}) "
            f"→ cutforge-render 直渲 → exports/{outs[0].name} ffprobe 校验: PASS(AC-7.4 预演)")

        print("e2e_independence: 断言链全过(AC-6.3 零 python 子进程 + 四大件内置 + 产物校验 + 纯 CLI 面)")
        return 0
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
