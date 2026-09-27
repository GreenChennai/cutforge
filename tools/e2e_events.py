#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""T1.6/AC-1.5 门禁:事件推送升级端到端(SSE 为主,长轮询降级兼容;纯 stdlib 探针)。

    python tools/e2e_events.py [--bin target/debug/cutforge-mcp.exe]

断言链:
  1. SSE 建立:GET /events(Accept: text/event-stream)→ 200 + text/event-stream;
  2. AC-1.5 主判据:外部进程改 project.json → SSE 收到 `workspace.changed`,
     P95 ≤ 200ms(20 轮实测);
  3. 长轮询降级(A1-R2:兼容旧壳,册二完成后移除):同场景 P95 ≤ 1s,
     且负载老字段一个不少(ok/code/event/seq);
  4. 事件面扩展:外部新建 notes.json → `notes.changed`;外部改 cutlist.json →
     `cutlist.changed`(SSE)。
退出码:0 通过 / 2 失败。依赖:仅 Python 标准库 + 已构建的 cutforge-mcp。
"""
from __future__ import annotations

import argparse
import json
import math
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
WEB = REPO / "apps" / "web"
FIXTURE = REPO / "tests" / "fixtures" / "real_ir" / "project.json"
# P95 预算(AC-1.5 原文):SSE ≤200ms;长轮询降级 ≤1s
SSE_BUDGET = 0.2
LP_BUDGET = 1.0
ROUNDS = 20  # P95 取第 19 位,容忍 1 次抖动

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def p95(xs: list[float]) -> float:
    ys = sorted(xs)
    return ys[max(0, math.ceil(0.95 * len(ys)) - 1)]


def external_modify_project(ws: Path, marker: str) -> float:
    """外部进程语义的 project.json 改动(写完才起算延迟);marker 变长保证
    len+mtime 双信号任一变化必被 watcher 捕捉。"""
    pj = ws / "05_ir" / "project.json"
    doc = json.loads(pj.read_text(encoding="utf-8"))
    doc["_e2eEventsProbe"] = marker + "x" * (len(marker) % 7)
    pj.write_text(json.dumps(doc, ensure_ascii=False), encoding="utf-8")
    return time.perf_counter()


class SseProbe:
    """SSE 探针:原始 socket 逐帧解析 id:/event:/data:(注释行 : 开头忽略)。"""

    def __init__(self, port: int, token: str):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=15)
        req = (f"GET /events?token={token} HTTP/1.1\r\nHost: 127.0.0.1\r\n"
               f"Accept: text/event-stream\r\n"
               f"Authorization: Bearer {token}\r\n"
               f"Connection: keep-alive\r\n\r\n")
        self.sock.sendall(req.encode("latin-1"))
        head = b""
        while b"\r\n\r\n" not in head:
            head += self.sock.recv(4096)
        first = head.decode("latin-1").split("\r\n")[0]
        assert " 200 " in first and "text/event-stream" in head.decode("latin-1"), \
            f"SSE 建立应 200 + text/event-stream,实得 {first}"
        self.buf = head.split(b"\r\n\r\n", 1)[1]

    def wait_event(self, name: str, timeout: float) -> tuple[float, float, dict]:
        """阻塞到收到指定事件名;返回 (绝对接收时刻, 事件内 seq, data)。"""
        deadline = time.perf_counter() + timeout
        while time.perf_counter() < deadline:
            self.sock.settimeout(max(0.05, deadline - time.perf_counter()))
            try:
                chunk = self.sock.recv(4096)
            except (socket.timeout, TimeoutError):
                break
            if not chunk:
                break
            self.buf += chunk
            while b"\n\n" in self.buf:
                frame, self.buf = self.buf.split(b"\n\n", 1)
                ev, seq, data = None, 0, {}
                for line in frame.decode("utf-8", "replace").split("\n"):
                    if line.startswith(":"):
                        continue
                    if line.startswith("event:"):
                        ev = line[6:].strip()
                    elif line.startswith("id:"):
                        seq = int(line[3:].strip() or 0)
                    elif line.startswith("data:"):
                        try:
                            data = json.loads(line[5:].strip())
                        except json.JSONDecodeError:
                            pass
                if ev == name:
                    return time.perf_counter(), (data.get("seq") or seq), data
        raise AssertionError(f"SSE {timeout:.1f}s 内未收到 {name}")

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass


def longpoll(port: int, token: str, since: int | None) -> dict:
    """旧壳语义的长轮询:Bearer 头 + since;服务端 ≤900ms 内有新事件即回。"""
    q = f"/events?token={token}" + (f"&since={since}" if since is not None else "")
    req = urllib.request.Request(f"http://127.0.0.1:{port}{q}",
                                 headers={"Authorization": f"Bearer {token}"})
    return json.loads(urllib.request.urlopen(req, timeout=5).read())


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=None)
    args = ap.parse_args()
    bin_path = Path(args.bin) if args.bin else REPO / "target" / "debug" / "cutforge-mcp.exe"
    if not bin_path.is_file():
        bin_path = REPO / "target" / "debug" / "cutforge-mcp"
    if not bin_path.is_file():
        bin_path = REPO / "target" / "release" / "cutforge-mcp.exe"
    if not bin_path.is_file():
        bin_path = REPO / "target" / "release" / "cutforge-mcp"
    if not bin_path.is_file():
        print("FAIL: 先 cargo build -p cutforge-mcp", file=sys.stderr)
        return 2

    token = "events-e2e-token"
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-events-e2e-"))
    ws = tmp / "ws"
    (ws / "05_ir").mkdir(parents=True)
    shutil.copy(FIXTURE, ws / "05_ir" / "project.json")
    port = free_port()
    serve = subprocess.Popen(
        [str(bin_path), "serve", "--root", str(ws), "--port", str(port),
         "--token", token, "--web", str(WEB)],
        stdout=subprocess.DEVNULL, stderr=open(tmp / "serve.log", "wb"))
    probe: SseProbe | None = None
    try:
        # 就绪等待 + 长轮询降级面基线(老字段一个不少)
        deadline = time.time() + 10
        base = None
        while time.time() < deadline:
            try:
                base = longpoll(port, token, None)
                break
            except Exception:
                time.sleep(0.15)
        assert base is not None, "serve 未就绪"
        assert base["event"] in ("workspace.changed", "none"), f"长轮询基线异常:{base}"
        for k in ("ok", "code", "event", "seq"):
            assert k in base, f"长轮询负载缺老字段 {k}:{base}"
        assert base["ok"] is True and base["code"] == "OK"
        lp_seq = base["seq"]
        print(f"长轮询降级面(负载 ok/code/event/seq 齐全,seq={lp_seq}): PASS")

        # SSE 建立
        probe = SseProbe(port, token)
        print("SSE 建立(200 + text/event-stream): PASS")

        # AC-1.5 主判据:外部改 project.json → SSE workspace.changed,P95 ≤ 200ms
        lats: list[float] = []
        for i in range(ROUNDS):
            time.sleep(0.25)
            t_mod = external_modify_project(ws, f"r{i:03d}")
            recv_at, seq, data = probe.wait_event("workspace.changed", timeout=5)
            assert seq > 0 and data.get("event") == "workspace.changed"
            lats.append(recv_at - t_mod)
        p95_sse = p95(lats)
        assert p95_sse <= SSE_BUDGET, \
            f"SSE workspace.changed P95={p95_sse * 1000:.0f}ms 超 {SSE_BUDGET * 1000:.0f}ms 预算({lats})"
        print(f"AC-1.5 SSE workspace.changed P95={p95_sse * 1000:.0f}ms"
              f"(20 轮,中位 {sorted(lats)[len(lats) // 2] * 1000:.0f}ms,预算 ≤200ms): PASS")

        # 事件面扩展:notes.json / cutlist.json 外部改动 → notes.changed / cutlist.changed
        (ws / "notes.json").write_text('[{"id":"n1","body":"e2e 探针"}]', encoding="utf-8")
        t_mod = time.perf_counter()
        recv_at, _, _ = probe.wait_event("notes.changed", timeout=5)
        print(f"事件面扩展 notes.changed({(recv_at - t_mod) * 1000:.0f}ms): PASS")
        (ws / "04_cut").mkdir(exist_ok=True)
        (ws / "04_cut" / "cutlist.json").write_text('{"items":[]}', encoding="utf-8")
        t_mod = time.perf_counter()
        recv_at, _, _ = probe.wait_event("cutlist.changed", timeout=5)
        print(f"事件面扩展 cutlist.changed({(recv_at - t_mod) * 1000:.0f}ms): PASS")

        # 长轮询降级路径同场景 P95 ≤ 1s(A1-R2:兼容旧壳,册二完成后移除)。
        # 先排干陈旧事件(SSE 阶段的 project.json 改动同样推高守护线程 seq,
        # 不排干会让计时轮"白捡"旧 seq 造成假快),再进入计时轮。
        for _ in range(64):
            v = longpoll(port, token, lp_seq)
            if v.get("event") != "workspace.changed" or v.get("seq", 0) <= lp_seq:
                break
            lp_seq = v["seq"]
        lats_lp: list[float] = []
        for i in range(ROUNDS):
            time.sleep(0.25)
            t_mod = external_modify_project(ws, f"lp{i:03d}")
            hit = False
            while time.perf_counter() - t_mod < 5:
                v = longpoll(port, token, lp_seq)
                if v.get("event") == "workspace.changed" and v.get("seq", 0) > lp_seq:
                    lats_lp.append(time.perf_counter() - t_mod)
                    lp_seq = v["seq"]
                    hit = True
                    break
            assert hit, f"长轮询 5s 内未收到 workspace.changed(第 {i} 轮)"
        p95_lp = p95(lats_lp)
        assert p95_lp <= LP_BUDGET, \
            f"长轮询 workspace.changed P95={p95_lp * 1000:.0f}ms 超 {LP_BUDGET * 1000:.0f}ms 预算({lats_lp})"
        print(f"降级路径 长轮询 workspace.changed P95={p95_lp * 1000:.0f}ms"
              f"(20 轮,预算 ≤1000ms): PASS")

        print("AC-1.5 事件推送 e2e: 全部 PASS")
        return 0
    finally:
        if probe:
            probe.close()
        serve.terminate()
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except AssertionError as e:
        print(f"FAIL: {e}", file=sys.stderr)
        sys.exit(2)
