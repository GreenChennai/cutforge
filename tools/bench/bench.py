#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""T1.8 性能基准套件:1,000 clips / 8 轨合成工程上的 open / query / apply / undo + 30s 渲染基线。

    python tools/bench/bench.py --json          # 完整 JSON 报告(stdout)+ 落盘 docs/bench/<date>.json
    python tools/bench/bench.py --check         # 按阈值判定,退出码 0 通过 / 2 失败

测量口径(全部经 MCP serve 的 JSON-RPC 数据面,与编辑器生产链路同一路径):
  open   = project_get(只读 Workspace::open:盘面→内存 + v2 契约校验 + OpLog 重建,至可查询)
  query  = timeline_get(同上只读打开 + 时间线投影;服务端无状态,每次调用都是真实打开)
  apply  = clip_update 一笔(独占打开→apply→persist 全程;1k 片段工程整文件落盘)
  undo   = undo 一笔(独占打开→回退→persist)
  render = render_run(cutforge-render 子进程,真实 ffmpeg)30s 素材 1080×1920 成片,单独计时

判定(AC-1.8):open ≤200ms / query ≤50ms / apply ≤100ms(取中位数);render 与
docs/bench/baseline.json 比对(双方都取多轮最小值,后台负载鲁棒),劣化 >20% 判
失败;baseline 缺失时以本次实测锁定。

构建档位:默认测 **release** 构建(--debug 可改测 debug)。debug 构建逐片段开销
约 0.5ms/clip(1k 工程 open ~0.5s),是 rustc 调试档常量而非产品性能,不能作基线;
release 下逐片段开销低一个量级以上,阈值才有判据意义。

夹具:1k 工程用 `cli new` 起骨架后直接组装 project.json(schema 对齐
schemas/project.schema.json,合法性由打开时的 Rust v2 契约校验兜底);
渲染基准用独立迷你工程(V1+A1 各 1 条 30s 剪辑),素材由 ffmpeg lavfi 现场合成。
夹具与临时产物全在系统临时目录,跑完清理。只依赖 python 标准库 + cargo 产物 + ffmpeg。
"""
from __future__ import annotations

import argparse
import json
import os
import platform
import re
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DOCS_BENCH = REPO / "docs" / "bench"
FPS = 30
TRACKS_1K = [("V1", "video"), ("V2", "video"), ("V3", "video"), ("V4", "video"),
             ("A1", "audio"), ("A2", "audio"), ("A3", "audio"), ("A4", "audio")]
CLIPS_PER_TRACK = 125            # 8 × 125 = 1,000
CLIP_MS = 240                    # 125 × 240ms = 每轨 30s,全工程时间线上界 30s
RENDER_SEC = 30                  # 渲染基准素材时长
THRESHOLDS = {"open": 200.0, "query": 50.0, "apply": 100.0}   # 毫秒,判中位数
RENDER_REGRESS = 1.20            # 渲染劣化 >20% 判失败

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[bench] {msg}", file=sys.stderr)


def sh(cmd: list[str], timeout: float = 900) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True,
                          encoding="utf-8", errors="replace", timeout=timeout)


# ---------------- 二进制定位与构建 ----------------

def locate_bin(name: str, release: bool) -> Path:
    """按档位优先取二进制:release 档先找 target/release;缺文件再跨档兜底。"""
    profile_dirs = ([REPO / "target" / "release", REPO / "target" / "debug"] if release
                    else [REPO / "target" / "debug", REPO / "target" / "release"])
    cands = [d / name for d in profile_dirs]
    if name.endswith(".exe"):  # 非 Windows 退化:同名字节码
        cands += [c.with_suffix("") for c in cands]
    for cand in cands:
        if cand.is_file():
            return cand
    raise SystemExit(f"FAIL: 未找到 {name},先 cargo build")


def build_workspace(release: bool) -> str:
    """cargo build --workspace --locked(默认 release);并行子代理持锁时本调用会等待,属正常。"""
    cmd = ["cargo", "build", "--workspace", "--locked"] + (["--release"] if release else [])
    t0 = time.perf_counter()
    log(" ".join(cmd) + "(若并行任务持锁会等待)…")
    r = sh(cmd, timeout=2400)
    if r.returncode != 0:
        raise SystemExit(f"FAIL: cargo build 失败:\n{r.stderr[-2000:]}")
    tail = (r.stdout + r.stderr).strip().splitlines()[-1:] or [""]
    log(f"cargo build 完成({time.perf_counter() - t0:.1f}s){tail[0]}")
    return tail[0]


# ---------------- MCP serve 数据面(复用 e2e_from_zero 的模式) ----------------

def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def rpc(port: int, token: str, name: str, args: dict, timeout: float = 30) -> dict:
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": name, "arguments": args}}).encode()
    last_err: Exception | None = None
    for attempt in range(2):  # Windows 偶发 RST 重试一次(真宕机则第二次同样失败并抛出)
        try:
            req = urllib.request.Request(
                f"http://127.0.0.1:{port}/rpc", data=body,
                headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"})
            out = json.loads(urllib.request.urlopen(req, timeout=timeout).read())
            return json.loads(out["result"]["content"][0]["text"])
        except ConnectionResetError as e:
            last_err = e
            time.sleep(0.5)
    raise last_err  # type: ignore[misc]


class Serve:
    """cutforge-mcp serve 包装:起停 + 就绪等待。"""

    def __init__(self, bin_path: Path, root: Path, tmp: Path, tag: str):
        self.token = f"bench-{tag}"
        self.port = free_port()
        self.proc = subprocess.Popen(
            [str(bin_path), "serve", "--root", str(root), "--port", str(self.port),
             "--token", self.token, "--web", str(REPO / "apps" / "web")],
            stdout=subprocess.DEVNULL, stderr=open(tmp / f"serve-{tag}.log", "wb"))
        deadline = time.time() + 15
        self.ready = False
        while time.time() < deadline:
            try:
                urllib.request.urlopen(
                    f"http://127.0.0.1:{self.port}/session?token={self.token}", timeout=1).read()
                self.ready = True
                break
            except Exception:
                time.sleep(0.15)
        if not self.ready:
            self.stop()
            raise SystemExit(f"FAIL: serve({tag}) 未就绪")

    def call(self, name: str, args: dict, timeout: float = 30) -> dict:
        return rpc(self.port, self.token, name, args, timeout)

    def stop(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()


# ---------------- 工程夹具 ----------------

def layout_rel(proj: Path) -> tuple[str, str]:
    """(时间线工程相对目录, 素材相对目录):0.5 中文布局为准,兼容 0.4.x 英文布局。"""
    if (proj / "05_时间线工程").is_dir():
        return "05_时间线工程", "01_原始素材"
    return "05_ir", "01_materials"


def cli_new(cli: Path, proj: Path) -> None:
    r = sh([str(cli), "new", str(proj), "--slug", proj.name, "--json",
            "--fps", str(FPS), "--width", "1080", "--height", "1920", "--track", "video,audio"])
    if r.returncode != 0:
        raise SystemExit(f"FAIL: cli new 失败: {r.stdout} {r.stderr}")


def project_value_1k(materials_rel: str) -> dict:
    """1,000 clips / 8 轨:每轨 125 × 240ms 顺序相接(每轨恰 30s,时间线上界 30s)。"""
    tracks = []
    for tid, kind in TRACKS_1K:
        clips = [{
            "id": f"{tid}-{i + 1:03d}",
            "src": f"{materials_rel}/synth.mp4",
            "startMs": i * CLIP_MS,
            "durationMs": CLIP_MS,
        } for i in range(CLIPS_PER_TRACK)]
        tracks.append({"id": tid, "kind": kind, "name": tid, "clips": clips})
    return {"version": 1, "schemaVersion": "2.0.0", "slug": "bench-1k",
            "fps": FPS, "canvas": {"width": 1080, "height": 1920}, "tracks": tracks}


def schema_self_check(pj: dict) -> dict:
    """标准库版结构自检(schemas/project.schema.json 的关键约束子集);
    权威校验是打开时 Rust 侧 Project::from_value 的 v2 契约。"""
    errs: list[str] = []
    if pj.get("version") != 1:
        errs.append("version!=1")
    if pj.get("schemaVersion") != "2.0.0":
        errs.append("schemaVersion!=2.0.0")
    if pj.get("fps") not in (24, 25, 30, 50, 60):
        errs.append("fps 非法")
    if pj.get("canvas") != {"width": 1080, "height": 1920}:
        errs.append("canvas 非法")
    tracks = pj.get("tracks", [])
    if len(tracks) != 8:
        errs.append(f"tracks={len(tracks)}!=8")
    seen = set()
    total = 0
    clip_re = re.compile(r"^[VAT][0-9]+-[0-9]{3}$")
    track_re = re.compile(r"^[VAT][0-9]+$")
    for t in tracks:
        if not track_re.match(t.get("id", "")):
            errs.append(f"轨 id 非法: {t.get('id')}")
        if t.get("kind") not in ("video", "audio", "text"):
            errs.append(f"轨 kind 非法: {t.get('kind')}")
        prev_end = 0
        for c in t.get("clips", []):
            total += 1
            if not clip_re.match(c.get("id", "")):
                errs.append(f"clip id 非法: {c.get('id')}")
            if c["id"] in seen:
                errs.append(f"clip id 重复: {c['id']}")
            seen.add(c["id"])
            if c.get("startMs", -1) < 0 or c.get("durationMs", 0) <= 0:
                errs.append(f"clip 时长非法: {c['id']}")
            if c["startMs"] < prev_end:
                errs.append(f"clip 重叠: {c['id']}")
            prev_end = c["startMs"] + c["durationMs"]
    if total != 1000:
        errs.append(f"clips={total}!=1000")
    return {"ok": not errs, "errors": errs[:20], "clips": total, "tracks": len(tracks)}


def make_project_1k(cli: Path, tmp: Path) -> tuple[Path, Path, dict, float]:
    """返回 (工程根, project.json 路径, 自检结果, 构造秒数)。"""
    t0 = time.perf_counter()
    proj = tmp / "bench-1k"
    cli_new(cli, proj)
    tl_rel, mat_rel = layout_rel(proj)
    (proj / mat_rel).mkdir(parents=True, exist_ok=True)
    # 夹具素材:lavfi 合成源(30s 640×360,片段引用合法;1k 工程本身不做渲染基准)
    r = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
            "-i", "testsrc2=size=640x360:rate=30", "-f", "lavfi",
            "-i", f"sine=frequency=440:duration={RENDER_SEC}",
            "-t", str(RENDER_SEC), "-pix_fmt", "yuv420p",
            "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
            str(proj / mat_rel / "synth.mp4")], timeout=300)
    if r.returncode != 0:
        raise SystemExit(f"FAIL: ffmpeg 生成 1k 夹具素材失败: {r.stderr[-300:]}")
    pj = project_value_1k(mat_rel)
    check = schema_self_check(pj)
    if not check["ok"]:
        raise SystemExit(f"FAIL: 1k 工程自检未过: {check['errors']}")
    ppath = proj / tl_rel / "project.json"
    ppath.write_text(json.dumps(pj, ensure_ascii=False), encoding="utf-8")
    return proj, ppath, check, time.perf_counter() - t0


def make_render_fixture(cli: Path, tmp: Path) -> Path:
    """渲染基准迷你工程:V1+A1 各 1 条 30s 剪辑,素材 1080×1920 lavfi 合成。"""
    proj = tmp / "bench-render"
    cli_new(cli, proj)
    tl_rel, mat_rel = layout_rel(proj)
    (proj / mat_rel).mkdir(parents=True, exist_ok=True)
    r = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
            "-i", f"testsrc2=size=1080x1920:rate={FPS}", "-f", "lavfi",
            "-i", f"sine=frequency=440:duration={RENDER_SEC}",
            "-t", str(RENDER_SEC), "-pix_fmt", "yuv420p",
            "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
            str(proj / mat_rel / "main.mp4")], timeout=600)
    if r.returncode != 0:
        raise SystemExit(f"FAIL: ffmpeg 生成渲染素材失败: {r.stderr[-300:]}")
    pj = {"version": 1, "schemaVersion": "2.0.0", "slug": "bench-render",
          "fps": FPS, "canvas": {"width": 1080, "height": 1920},
          "tracks": [
              {"id": "V1", "kind": "video", "name": "V1",
               "clips": [{"id": "V1-001", "src": f"{mat_rel}/main.mp4",
                          "startMs": 0, "durationMs": RENDER_SEC * 1000}]},
              {"id": "A1", "kind": "audio", "name": "A1",
               "clips": [{"id": "A1-001", "src": f"{mat_rel}/main.mp4",
                          "startMs": 0, "durationMs": RENDER_SEC * 1000}]},
          ]}
    (proj / tl_rel / "project.json").write_text(json.dumps(pj, ensure_ascii=False), encoding="utf-8")
    return proj


# ---------------- 测量 ----------------

def p95(samples: list[float]) -> float:
    """最近秩 P95。"""
    s = sorted(samples)
    return s[max(0, -(-95 * len(s) // 100) - 1)]


def timed(fn, iters: int, warmups: int = 3) -> dict:
    for _ in range(warmups):
        assert fn()[0] is True, "预热调用失败"
    xs = []
    for _ in range(iters):
        t0 = time.perf_counter()
        ok, extra = fn()
        xs.append((time.perf_counter() - t0) * 1000.0)
        if not ok:
            raise SystemExit(f"FAIL: 测量调用失败: {extra}")
    return {"iters": iters, "medianMs": round(statistics.median(xs), 2),
            "p95Ms": round(p95(xs), 2),
            "minMs": round(min(xs), 2), "maxMs": round(max(xs), 2),
            "samples": [round(x, 2) for x in xs]}


def bench_open_query(serve: Serve, root: Path, iters: int) -> tuple[dict, dict, float]:
    root_s = str(root)
    # 首次打开(冷):含 watcher 守护拉起与首次盘读,仅供参考,不参与阈值判定
    t0 = time.perf_counter()
    cold = serve.call("project_get", {"root": root_s})
    cold_ms = (time.perf_counter() - t0) * 1000.0
    if not cold.get("ok"):
        raise SystemExit(f"FAIL: 冷打开失败: {cold}")
    n_clips = sum(len(t["clips"]) for t in cold["data"]["project"]["tracks"])
    op = timed(lambda: (lambda r: (r.get("ok"), r))(
        serve.call("project_get", {"root": root_s})), iters)
    qy = timed(lambda: (lambda r: (r.get("ok"), r))(
        serve.call("timeline_get", {"root": root_s})), iters)
    if n_clips != 1000:
        raise SystemExit(f"FAIL: 打开的工程 clips={n_clips}!=1000")
    return op, qy, cold_ms


def bench_apply_undo(serve: Serve, root: Path, clip_id: str, iters: int) -> tuple[dict, dict]:
    """apply:计时 clip_update(volume 0.9→0.8)+ 非计时 undo 复位;undo 反之。
    每笔 op 都真实落盘(不带 requestId,避免幂等去重造成假快)。"""
    root_s = str(root)

    def upd(summary: str) -> dict:
        return serve.call("clip_update",
                          {"root": root_s, "clipId": clip_id,
                           "patch": {"volume": 0.8}, "summary": summary})

    def do_undo() -> dict:
        return serve.call("undo", {"root": root_s})

    for _ in range(3):  # 预热 + 复位
        assert upd("bench 预热")["ok"] and do_undo()["ok"]
    ap_x, ud_x = [], []
    for i in range(iters):
        t0 = time.perf_counter()
        r = upd(f"bench apply {i}")
        ap_x.append((time.perf_counter() - t0) * 1000.0)
        if not r.get("ok"):
            raise SystemExit(f"FAIL: clip_update: {r}")
        t0 = time.perf_counter()
        r = do_undo()
        ud_x.append((time.perf_counter() - t0) * 1000.0)
        if not r.get("ok"):
            raise SystemExit(f"FAIL: undo: {r}")

    def pack(xs: list[float]) -> dict:
        return {"iters": iters, "medianMs": round(statistics.median(xs), 2),
                "p95Ms": round(p95(xs), 2),
                "minMs": round(min(xs), 2), "maxMs": round(max(xs), 2),
                "samples": [round(x, 2) for x in xs]}

    return pack(ap_x), pack(ud_x)


def bench_render(serve: Serve, root: Path, ffprobe: str | None, iters: int = 3) -> dict:
    """render_run(cutforge 后端)→ 轮询 render_progress 至完成;单独计时。
    共享开发机上单次渲染受后台负载波动大(实测 6.6s↔11.0s),故跑 iters 轮取最小值
    (min 最接近真实成本);每轮前清 render-cache,保证每轮都是完整冷渲染。"""
    root_s = str(root)
    samples, output = [], None
    for i in range(iters):
        # 内容寻址缓存会让第 2+ 轮命中 seg/compose 而失真,先清掉
        shutil.rmtree(root / ".cutforge" / "render-cache", ignore_errors=True)
        t0 = time.perf_counter()
        rr = serve.call("render_run", {"root": root_s, "backend": "cutforge"}, timeout=60)
        if not rr.get("ok"):
            raise SystemExit(f"FAIL: render_run: {rr}")
        run_id = rr["data"]["runId"]
        state, err = "queued", None
        while True:
            s = serve.call("render_progress", {"root": root_s, "runId": run_id}, timeout=10)
            state = s["data"]["state"]
            output = s["data"].get("output")
            err = s["data"].get("error")
            # 册五 T5.6 队列化:queued 为合法暂态(入队→worker ≤50ms 派发),非终态
            if state not in ("running", "queued"):
                break
            time.sleep(0.25)
        ms = (time.perf_counter() - t0) * 1000.0
        if state != "ok" or not output or not (root / output).is_file():
            raise SystemExit(f"FAIL: 渲染未成功 state={state} output={output} err={err}")
        samples.append(round(ms, 1))
    dur = None
    if ffprobe:
        r = sh([ffprobe, "-v", "error", "-print_format", "json", "-show_format",
                str(root / output)], timeout=60)
        if r.returncode == 0:
            dur = round(float(json.loads(r.stdout)["format"]["duration"]), 3)
        if dur is None or abs(dur - RENDER_SEC) > 1.5:
            raise SystemExit(f"FAIL: 成片时长 {dur}s 与 {RENDER_SEC}s 不符")
    return {"iters": iters, "medianMs": round(statistics.median(samples), 1),
            "minMs": min(samples), "p95Ms": max(samples),
            "maxMs": max(samples), "samples": samples,
            "output": output, "outputBytes": (root / output).stat().st_size,
            "outputDurationSec": dur}


# ---------------- 环境信息 ----------------

def total_mem_gb() -> str:
    if os.name != "nt":
        return "unknown"
    try:
        import ctypes

        class Stat(ctypes.Structure):
            _fields_ = [("dwLength", ctypes.c_ulong), ("dwMemoryLoad", ctypes.c_ulong),
                        ("ullTotalPhys", ctypes.c_ulonglong), ("ullAvailPhys", ctypes.c_ulonglong),
                        ("ullTotalPageFile", ctypes.c_ulonglong), ("ullAvailPageFile", ctypes.c_ulonglong),
                        ("ullTotalVirtual", ctypes.c_ulonglong), ("ullAvailVirtual", ctypes.c_ulonglong),
                        ("ullAvailExtendedVirtual", ctypes.c_ulonglong)]

        st = Stat()
        st.dwLength = ctypes.sizeof(Stat)
        ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(st))
        return f"{st.ullTotalPhys / 2**30:.1f}GB"
    except Exception:
        return "unknown"


def env_info() -> dict:
    cpu = os.environ.get("PROCESSOR_IDENTIFIER") or platform.processor() or "unknown"
    def first_line(cmd: list[str]) -> str:
        try:
            r = sh(cmd, timeout=30)
            return (r.stdout or r.stderr).strip().splitlines()[0] if (r.stdout or r.stderr).strip() else "unknown"
        except Exception:
            return "unknown"
    now = datetime.now().astimezone()
    utc_now = datetime.now(timezone.utc)
    return {
        "timestamp": now.isoformat(timespec="seconds"),
        # 报告文件名按 UTC 日期(跨午夜运行归档日稳定);timestamp 保留本地时刻
        "date": utc_now.strftime("%Y-%m-%d"),
        "os": f"{platform.system()} {platform.release()} ({platform.machine()})",
        "cpu": cpu,
        "cpuCount": os.cpu_count(),
        "memTotal": total_mem_gb(),
        "python": platform.python_version(),
        "rust": first_line(["rustc", "--version"]),
        "cargo": first_line(["cargo", "--version"]),
        "ffmpeg": first_line(["ffmpeg", "-version"]),
        "gitRev": first_line(["git", "-C", str(REPO), "rev-parse", "--short", "HEAD"]),
    }


# ---------------- 报告与判定 ----------------

def write_report(report: dict) -> Path:
    DOCS_BENCH.mkdir(parents=True, exist_ok=True)
    path = DOCS_BENCH / f"{report['env']['date']}.json"
    path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    return path


def load_baseline() -> dict | None:
    p = DOCS_BENCH / "baseline.json"
    if not p.is_file():
        return None
    return json.loads(p.read_text(encoding="utf-8"))


def save_baseline(render: dict, env: dict) -> Path:
    DOCS_BENCH.mkdir(parents=True, exist_ok=True)
    baseline = {"renderMs": render["minMs"], "renderMedianMs": render["medianMs"],
                "renderSamples": render["samples"], "statistic": "min(后台负载鲁棒)",
                "capturedAt": env["timestamp"], "buildProfile": env.get("buildProfile"),
                "fixture": f"lavfi testsrc2 1080x1920 {RENDER_SEC}s,{FPS}fps;V1+A1 各 1 剪辑;"
                           "render_run(backend=cutforge)含 cutforge-render 子进程全程",
                "env": {"cpu": env["cpu"], "os": env["os"]}}
    p = DOCS_BENCH / "baseline.json"
    p.write_text(json.dumps(baseline, ensure_ascii=False, indent=2), encoding="utf-8")
    return p


def run_bench(iters_open: int, iters_edit: int, iters_render: int, skip_build: bool, release: bool) -> dict:
    """执行全量测量,返回报告 dict。"""
    if not skip_build:
        build_workspace(release)
    else:
        log("跳过 cargo build(--skip-build)")
    mcp = locate_bin("cutforge-mcp.exe", release)
    cli = locate_bin("cutforge-cli.exe", release)
    ffprobe = shutil.which("ffprobe")
    if not shutil.which("ffmpeg"):
        raise SystemExit("FAIL: 本机无 ffmpeg")
    env = env_info()
    env["buildProfile"] = "release" if release else "debug"
    t0 = time.perf_counter()
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-bench-"))
    serve1 = serve2 = None
    try:
        proj, ppath, self_check, build_s = make_project_1k(cli, tmp)
        log(f"1k 夹具构造完成({build_s:.1f}s): {ppath}")
        serve1 = Serve(mcp, proj, tmp, "1k")
        # 打开即过 Rust v2 契约(project_get 成功 = 合法性权威校验)
        op, qy, cold_ms = bench_open_query(serve1, proj, iters_open)
        clip_id = "V1-063"  # 中位片段;apply/undo 与查询共用同一工程
        ap, ud = bench_apply_undo(serve1, proj, clip_id, iters_edit)
        serve1.stop()
        # —— 渲染基准(独立迷你工程) ——
        rproj = make_render_fixture(cli, tmp)
        serve2 = Serve(mcp, rproj, tmp, "render")
        rd = bench_render(serve2, rproj, ffprobe, iters_render)
        report = {
            "meta": {"task": "T1.8 性能基准", "fixture": "1000 clips / 8 轨(V1-V4 video, A1-A4 audio;"
                     "每轨 125×240ms 顺序相接=30s)", "fixtureBuildSec": round(build_s, 2),
                     "fixtureContractCheck": "pass(打开时 Rust Project::from_value v2 契约校验通过)",
                     "fixtureSchemaSelfCheck": self_check,
                     "elapsedSec": round(time.perf_counter() - t0, 1)},
            "env": env,
            "bench": {
                "open": {**op, "metric": "project_get(只读 Workspace::open 至可查询)",
                         "coldFirstMs": round(cold_ms, 2)},
                "query": {**qy, "metric": "timeline_get(含一次真实只读打开;服务端无状态)"},
                "apply": {**ap, "metric": f"clip_update({clip_id} volume 0.9→0.8,独占打开+整文件落盘)"},
                "undo": {**ud, "metric": "undo 一笔(独占打开+回退+落盘)"},
                "render": {**rd, "metric": f"render_run(backend=cutforge),lavfi 1080x1920 {RENDER_SEC}s 成片;"
                                        "每轮清 render-cache 冷渲染,取最小值参与判定"},
            },
        }
        return report
    finally:
        if serve1:
            serve1.stop()
        if serve2:
            serve2.stop()
        if sys.exc_info()[0] is not None:
            # 失败排查:保留现场,附 serve 日志尾
            for lg in sorted(tmp.glob("serve-*.log")):
                text = lg.read_text(encoding="utf-8", errors="replace")[-1500:]
                log(f"serve 日志尾 {lg.name}:\n{text}")
            log(f"现场保留: {tmp}")
        else:
            shutil.rmtree(tmp, ignore_errors=True)


def check_thresholds(report: dict) -> tuple[bool, list[dict]]:
    """AC-1.8 判定:open/query/apply 中位数对阈值;render 对 baseline(劣化 >20% 失败)。"""
    results = []
    for name, limit in THRESHOLDS.items():
        v = report["bench"][name]["medianMs"]
        results.append({"item": name, "value": v, "limit": limit,
                        "marginMs": round(limit - v, 2), "ok": v <= limit})
    baseline = load_baseline()
    rd = report["bench"]["render"]["minMs"]  # 判定用最小值:后台负载鲁棒(见 bench_render)
    if baseline is None:
        results.append({"item": "render", "value": rd, "limit": None, "baselineMs": None,
                        "note": "无 baseline:本次实测锁定为基线", "ok": True})
    else:
        limit = round(baseline["renderMs"] * RENDER_REGRESS, 1)
        note = None
        if baseline.get("buildProfile") != report["env"].get("buildProfile"):
            note = f"baseline 档位({baseline.get('buildProfile')})与本次({report['env'].get('buildProfile')})不同,比对仅供参考"
        results.append({"item": "render", "value": rd, "limit": limit,
                        "baselineMs": baseline["renderMs"], "statistic": "min",
                        "note": note,
                        "deltaPct": round((rd / baseline["renderMs"] - 1) * 100, 1), "ok": rd <= limit})
    return all(r["ok"] for r in results), results


def main() -> int:
    ap = argparse.ArgumentParser(description="T1.8 性能基准套件")
    ap.add_argument("--json", action="store_true", help="输出完整 JSON 报告并落盘 docs/bench/<date>.json")
    ap.add_argument("--check", action="store_true", help="测量 + 按 AC-1.8 阈值判定退出码(0/2)")
    ap.add_argument("--update-baseline", action="store_true", help="以本次 render 实测覆盖 baseline.json")
    ap.add_argument("--skip-build", action="store_true", help="跳过 cargo build(二进制已新鲜时用)")
    ap.add_argument("--debug", action="store_true", help="测 debug 构建(默认 release;debug 逐片段开销大,不作基线)")
    ap.add_argument("--iters-open", type=int, default=20, help="open/query 迭代次数(默认 20)")
    ap.add_argument("--iters-edit", type=int, default=12, help="apply/undo 迭代次数(默认 12)")
    ap.add_argument("--iters-render", type=int, default=3, help="渲染轮数(取最小值判定,默认 3)")
    args = ap.parse_args()
    if not args.json and not args.check:
        ap.print_help()
        return 2

    report = run_bench(args.iters_open, args.iters_edit, args.iters_render, args.skip_build, not args.debug)

    baseline_missing = load_baseline() is None
    if args.update_baseline or baseline_missing:
        bp = save_baseline(report["bench"]["render"], report["env"])
        log(f"render 基线已{'更新' if args.update_baseline else '锁定'}: {bp}"
            f"({report['bench']['render']['medianMs']}ms)")
    rp = write_report(report)
    log(f"报告已落盘: {rp}")

    if args.json:
        print(json.dumps(report, ensure_ascii=False, indent=2))

    if args.check:
        ok, results = check_thresholds(report)
        report["check"] = {"ok": ok,
                           "rules": {"open/query/apply": "中位数 vs 阈值",
                                     "render": "vs baseline.json,劣化 >20% 失败"},
                           "results": results}
        # check 结论并入落盘报告
        rp.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        print("== T1.8 基准判定(AC-1.8)==")
        for r in results:
            if r["item"] == "render":
                base = f"baseline={r['baselineMs']}ms" if r["baselineMs"] is not None else "baseline=本次锁定"
                med = report["bench"]["render"]["medianMs"]
                print(f"  render : min={r['value']:>8.1f}ms(中位 {med}ms)  {base}  上限={r['limit']}ms  "
                      f"Δ={r.get('deltaPct', 0):+.1f}%  {'PASS' if r['ok'] else 'FAIL'}")
            else:
                print(f"  {r['item']:<6}: {r['value']:>9.2f}ms  阈值={r['limit']:.0f}ms  "
                      f"余量={r['marginMs']:+.2f}ms  {'PASS' if r['ok'] else 'FAIL'}")
        print(f"结论: {'PASS' if ok else 'FAIL'}")
        return 0 if ok else 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
