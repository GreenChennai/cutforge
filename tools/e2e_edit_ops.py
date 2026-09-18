#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""M10-1 / M10-2 门禁:浏览器自动化端到端(webapp-testing)。

    python tools/e2e_edit_ops.py [--bin target/debug/cutforge-mcp.exe]

M10-1 edit_ops_e2e:导入 CutFlow 真实工程 → 分割 → 移动 → 波纹删 → undo 全还原 → redo,
每步之后 OpLog 与盘面(rev/片段)一致。
M10-2 note_loop_ui:3 条标注 创建 → AI 执行(clip_update,caused_by 绑定)→ 回执结案,全链 ≤5s。
退出码:0 通过 / 2 失败。依赖:playwright(chromium)。
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
FIXTURE = REPO / "tests" / "fixtures" / "real_ir" / "project.json"

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")


def rpc(port: int, token: str, name: str, args: dict) -> dict:
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": name, "arguments": args}}).encode()
    req = urllib.request.Request(f"http://127.0.0.1:{port}/rpc", data=body,
                                 headers={"Content-Type": "application/json",
                                          "Authorization": f"Bearer {token}"})
    for attempt in range(4):
        try:
            out = json.loads(urllib.request.urlopen(req, timeout=10).read())
            return json.loads(out["result"]["content"][0]["text"])
        except ConnectionResetError:
            if attempt == 3:
                raise
            time.sleep(0.2 * (attempt + 1))


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def clips_of(env: dict, track_id: str) -> list[dict]:
    return next(t["clips"] for t in env["data"]["project"]["tracks"] if t["id"] == track_id)


def oplog_count(port: int, token: str, root: str) -> int:
    return rpc(port, token, "oplog_tail", {"root": root, "limit": 1})["data"]["count"]


class Driver:
    """浏览器驱动 + 确定性等待(UI 动作后轮询服务端 rev,超时吐诊断)。"""

    def __init__(self, page, port: int, token: str, root: str):
        self.page = page
        self.port = port
        self.token = token
        self.root = root

    def goto(self):
        self.page.goto(f"http://127.0.0.1:{self.port}/?token={self.token}")
        self.page.wait_for_function(
            "document.getElementById('rev').textContent !== '-'", timeout=20000)

    def get(self) -> dict:
        return rpc(self.port, self.token, "project_get", {"root": self.root})

    def rev(self) -> int:
        return int(self.page.inner_text("#rev"))

    def wait_rev(self, prev: int, timeout_s: float = 8.0):
        deadline = time.time() + timeout_s
        while time.time() < deadline:
            if self.get()["data"]["rev"] != prev:
                self.page.wait_for_timeout(250)  # 让 UI refresh 追上
                return self.get()["data"]["rev"]
            time.sleep(0.15)
        status = self.page.inner_text("#status")
        oplog = rpc(self.port, self.token, "oplog_tail", {"root": self.root, "limit": 500})
        tail = [(o.get("opId"), o.get("opKind"), o.get("target", {}).get("file"), str(o.get("summary"))[:40]) for o in oplog["data"]["ops"][-8:]]
        raise AssertionError(
            f"rev 在 {timeout_s}s 内未从 {prev} 变化;UI status={status!r};rev_now={self.get()['data']['rev']};oplog_tail={tail}")

    def click(self, sel: str):
        self.page.click(sel)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=None)
    args = ap.parse_args()
    bin_path = Path(args.bin) if args.bin else REPO / "target" / "debug" / "cutforge-mcp.exe"
    if not bin_path.is_file():
        bin_path = REPO / "target" / "release" / "cutforge-mcp.exe"
    if not bin_path.is_file():
        print("FAIL: 先 cargo build(cutforge-mcp)", file=sys.stderr)
        return 2

    from playwright.sync_api import sync_playwright

    tmp = Path(tempfile.mkdtemp(prefix="cutforge-e2e-"))
    token = "e2e-token"

    def spawn_serve(ws_dir: Path):
        for attempt in range(3):
            port = free_port()
            err_log = tmp / f"serve-{attempt}.log"
            serve = subprocess.Popen(
                [str(bin_path), "serve", "--root", str(ws_dir), "--port", str(port),
                 "--token", token, "--web", str(REPO / "apps" / "web")],
                stdout=subprocess.DEVNULL, stderr=open(err_log, "wb"))
            deadline = time.time() + 8
            ready = False
            while time.time() < deadline:
                if serve.poll() is not None:
                    break  # 进程退出(bind 失败等)→ 换端口重试
                try:
                    urllib.request.urlopen(f"http://127.0.0.1:{port}/session?token={token}", timeout=1).read()
                    ready = True
                    break
                except Exception:
                    time.sleep(0.15)
            if ready:
                return serve, port
            serve.terminate()
            time.sleep(0.3)
        raise AssertionError("serve 3 次尝试均未就绪")

    try:
        # ================= M10-1:edit_ops_e2e =================
        ws = tmp / "ws"
        (ws / "05_ir").mkdir(parents=True)
        shutil.copy(FIXTURE, ws / "05_ir" / "project.json")
        serve, port = spawn_serve(ws)
        try:
            with sync_playwright() as pw:
                browser = pw.chromium.launch()
                page = browser.new_page()
                d = Driver(page, port, token, str(ws))
                d.goto()
                rev0 = d.get()["data"]["rev"]
                base_clips = clips_of(d.get(), "V1")

                # 分割:选 V1-001,播放头 2000ms,按 S
                page.click('.clip[data-id="V1-001"]')
                page.click("#ruler", position={"x": int(2000 * 0.06), "y": 10})
                page.keyboard.press("s")
                d.wait_rev(rev0)
                rev1 = d.get()["data"]["rev"]
                c1 = clips_of(d.get(), "V1")
                assert len(c1) == len(base_clips) + 1 and any(c["id"] == "V1-003" for c in c1), f"分割: {c1}"

                # 移动:检查器 V1-003 → 8400(clip_update)
                page.click('.clip[data-id="V1-003"]')
                page.fill("#insp-start", "8400")
                page.click("#insp-apply")
                d.wait_rev(rev1)
                rev2 = d.get()["data"]["rev"]
                moved = next(c for c in clips_of(d.get(), "V1") if c["id"] == "V1-003")
                assert moved["startMs"] == 8400, f"移动: {moved}"

                # 波纹删:Shift+Delete V1-002 → V1-003 左移 4400
                page.click('.clip[data-id="V1-002"]')
                page.keyboard.press("Shift+Delete")
                d.wait_rev(rev2)
                rev3 = d.get()["data"]["rev"]
                c3 = clips_of(d.get(), "V1")
                assert not any(c["id"] == "V1-002" for c in c3), f"删除: {c3}"
                m3 = next(c for c in c3 if c["id"] == "V1-003")
                assert m3["startMs"] == 4000, f"波纹: {c3}"

                # undo 全还原(逐字段回到初始;服务端计数兜底点击竞态)
                redos_expected = 0
                while True:
                    cur = d.get()
                    if clips_of(cur, "V1") == base_clips:
                        break
                    page.click("#btn-undo")
                    redos_expected += 1
                    deadline = time.time() + 5
                    while time.time() < deadline:
                        if d.get()["data"]["rev"] > cur["data"]["rev"]:
                            break
                        page.click("#btn-undo")
                        time.sleep(0.25)
                        redos_expected += 1
                    assert redos_expected < 20, "undo 深度异常"
                rev_undo = d.get()["data"]["rev"]
                oplog = rpc(port, token, "oplog_tail", {"root": str(ws), "limit": 500})
                assert oplog["data"]["count"] == rev_undo, \
                    f"OpLog({oplog['data']['count']}) 与 rev({rev_undo}) 不一致"
                rev_file = (ws / ".cutforge" / "rev").read_text().strip()
                assert int(rev_file) == rev_undo, "盘面 rev 与视图不一致"

                # redo 等量回放(以 oplog 计数为服务端真相,点击不足则补点)
                target_ops = oplog_count(port, token, str(ws)) + redos_expected
                deadline = time.time() + 15
                while oplog_count(port, token, str(ws)) < target_ops and time.time() < deadline:
                    page.click("#btn-redo")
                    time.sleep(0.3)
                assert oplog_count(port, token, str(ws)) == target_ops, "redo 次数不足"
                v_redo = d.get()
                if clips_of(v_redo, "V1") != c3:
                    raise AssertionError(f"redo 后不一致: {clips_of(v_redo, 'V1')} vs {c3}")
                browser.close()
            print("M10-1 edit_ops_e2e: PASS")
        finally:
            serve.terminate()

        # ================= M10-2:note_loop_ui =================
        ws2 = tmp / "ws2"
        (ws2 / "05_ir").mkdir(parents=True)
        shutil.copy(FIXTURE, ws2 / "05_ir" / "project.json")
        serve2, port2 = spawn_serve(ws2)
        try:
            with sync_playwright() as pw:
                browser = pw.chromium.launch()
                page = browser.new_page()
                # 阻断 /events 长轮询:防自动刷新与本测试的行操作竞态(事件可见性
                # 已由 M9-2 门禁单独覆盖)
                page.route("**/events*", lambda route: route.abort())
                d = Driver(page, port2, token, str(ws2))
                d.goto()
                page.click('#tabs button[data-tab="notes"]')

                t0 = time.time()
                for i in range(3):
                    page.fill("#note-body", f"第{i+1}处语速偏快")
                    page.click("#note-create")
                    page.wait_for_function(
                        f"document.querySelectorAll('#notes-open .row').length >= {i+1}", timeout=8000)
                note_ids = [n["id"] for n in rpc(port2, token, "notes_list", {"root": str(ws2)})["data"]["notes"]
                            if n["state"] == "open"]
                assert len(note_ids) == 3

                # AI 执行:clip_update + caused_by 绑定 → opId
                ops_by_note = {}
                for k, nid in enumerate(note_ids):
                    r = rpc(port2, token, "clip_update",
                            {"root": str(ws2), "clipId": "V1-001",
                             "patch": {"volume": round(0.9 - 0.05 * k, 2)},
                             "causedBy": [nid], "summary": f"按 {nid} 微调音量"})
                    assert r["ok"] and r["data"]["opIds"], f"AI 执行: {r}"
                    ops_by_note[nid] = r["data"]["opIds"][0]

                # 回执结案(浏览器内逐条)
                done = 0
                while done < 3:
                    rows = page.query_selector_all("#notes-open .row")
                    if not rows:
                        page.wait_for_timeout(150)
                        continue
                    row = rows[0]
                    nid = next((x for x in note_ids if x in row.inner_text()), note_ids[0])
                    inputs = row.query_selector_all("input")
                    inputs[0].fill("已微调音量 0.9")
                    inputs[1].fill(ops_by_note[nid])
                    row.query_selector("button").click()
                    deadline = time.time() + 8
                    while time.time() < deadline:
                        cur = [n2 for n2 in rpc(port2, token, "notes_list", {"root": str(ws2)})["data"]["notes"]
                               if n2["state"] == "open"]
                        if nid not in cur:
                            break
                        time.sleep(0.15)
                    note_ids.remove(nid)
                    done += 1
                elapsed = time.time() - t0
                assert elapsed <= 5.0, f"M10-2 全链须 ≤5s,实际 {elapsed:.2f}s"
                final = rpc(port2, token, "notes_list", {"root": str(ws2)})
                resolved = [n for n in final["data"]["notes"] if n["state"] == "resolved"]
                assert len(resolved) == 3 and all(n["resolvedBy"]["opIds"] for n in resolved)
                browser.close()
            print(f"M10-2 note_loop_ui: PASS({elapsed:.2f}s ≤ 5s)")
        finally:
            serve2.terminate()
        return 0
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
