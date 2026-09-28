#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册二 T2.7 新壳 UI 冒烟门禁(e2e_ui_smoke;AC-2.3/2.5/2.6 内嵌于此)。

    python tools/e2e_ui_smoke.py [--bin cutforge-mcp] [--cli cutforge-cli]

断言链:
  全 tab 遍历 → 素材导入(UI 双击)→ 选中 → 检查器改字段(ui-fields 驱动)→
  undo/redo → BGM 设置 → 右键菜单分割 → 向导模态(Esc/焦点陷阱/焦点归还)→
  轨头眼睛开关(ephemeral 不落盘)→ 标注回路(创建→AI 执行 caused_by→回执结案);
  AC-2.3 内嵌统计:MutationObserver 计一次 clip move 的相关 DOM 变更(<50 次;
  旧壳 innerHTML 全量重建 ≈ 数千);
  AC-2.5 内嵌断言:window.__cutforgeSelfTest() 清空→重投影→逐字段 diff=0;
  AC-2.6 三场景:
    ① 超时:拦截 /rpc 写命令不响应 → 壳按 10s AbortController 中止(requestfailed
       证据)且未落账,解除拦截后可恢复写入;
    ② 401:坏 token 启动 → token 横幅且壳停摆;服务换 token 重启后数据面 rpc →
       401 分流 toast(#status);
    ③ 断线重连:SSE 灌坏 MIME 强制 EventSource CLOSED → 壳降级长轮询(pollOnce),
       外部改 project.json 仍驱动重投影(标尺宽度增长为事件到达信号)并恢复可写。

断言纪律(M10-R5):UI 动作只作驱动,断言以服务端状态(rev/oplog/工程回读)为准;
选中态/ephemeral 等纯会话态才以 UI 文本辅助观察。
退出码:0 通过 / 2 失败。依赖:playwright(chromium)+ ffmpeg + cutforge-cli。
"""
from __future__ import annotations

import argparse
import json
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
PX_PER_MS = 0.06          # 壳显示映射红线(apps/web/js/core/model.js)
MUTATION_BUDGET = 50      # AC-2.3:一次 clip move 相关 DOM 变更上限
RPC_TIMEOUT_MS = 10000    # 与 apps/web/js/core/api.js DEFAULT_TIMEOUT_MS 对齐
MAT_CLIP_MS = 20000       # 冒烟素材片段时长

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[ui-smoke] {msg}", flush=True)


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def rpc(port: int, token: str, name: str, args: dict, timeout: float = 10) -> dict:
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
    return subprocess.run(cmd, capture_output=True, text=True,
                          encoding="utf-8", errors="replace")


def cli_new(cli: Path, proj: Path) -> None:
    r = sh([str(cli), "new", str(proj), "--slug", proj.name, "--json",
            "--fps", "30", "--width", "1080", "--height", "1920", "--track", "video,audio"])
    assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"


def layout_rel(proj: Path) -> tuple[str, str]:
    """(时间线工程相对目录, 素材相对目录):0.5 中文布局为准,兼容 0.4.x 英文布局。"""
    if (proj / "05_时间线工程").is_dir():
        return "05_时间线工程", "01_原始素材"
    return "05_ir", "01_materials"


def make_media(ws: Path, name: str, seconds: int, size: str = "640x360") -> None:
    r = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
            "-i", f"testsrc2=size={size}:rate=30", "-f", "lavfi",
            f"-i", f"sine=frequency=440:duration={seconds}",
            "-t", str(seconds), "-pix_fmt", "yuv420p",
            "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
            str(ws / name)])
    assert r.returncode == 0 and (ws / name).is_file(), f"ffmpeg 生成素材失败:{r.stderr[-300:]}"


class Serve:
    def __init__(self, bin_path: Path, root: Path, tmp: Path, token: str, port: int | None = None,
                 want_port: int | None = None):
        self.token = token
        # want_port:token 轮换场景要求旧页面可达,必须复用同一端口
        for attempt in range(6):
            self.port = want_port if want_port is not None else free_port()
            self.proc = subprocess.Popen(
                [str(bin_path), "serve", "--root", str(root), "--port", str(self.port),
                 "--token", token, "--web", str(REPO / "apps" / "web")],
                stdout=subprocess.DEVNULL, stderr=open(tmp / f"serve-{self.port}.log", "wb"))
            deadline = time.time() + 10
            self.ready = False
            while time.time() < deadline:
                if self.proc.poll() is not None:
                    break  # bind 失败 → 换端口/重试
                try:
                    urllib.request.urlopen(
                        f"http://127.0.0.1:{self.port}/session?token={token}", timeout=1).read()
                    self.ready = True
                    break
                except Exception:
                    time.sleep(0.15)
            if self.ready:
                return
            self.stop()
            if want_port is None:
                continue
            time.sleep(0.5)
        raise AssertionError("serve 多次尝试均未就绪")

    def stop(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()


def wait_server_rev(port: int, token: str, root: str, prev: int, timeout_s: float = 15.0) -> int:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        r = rpc(port, token, "project_get", {"root": root})["data"]["rev"]
        if r != prev:
            return r
        time.sleep(0.15)
    raise AssertionError(f"服务端 rev 在 {timeout_s}s 内未从 {prev} 变化")


def oplog_count(port: int, token: str, root: str) -> int:
    return rpc(port, token, "oplog_tail", {"root": root, "limit": 1})["data"]["count"]


def assert_ledger(port: int, token: str, root: str, tag: str) -> None:
    """OpLog 与 rev 一致(A1 口径;只在 clip 写窗口调用,notes 面不在承诺内)。"""
    rev = rpc(port, token, "project_get", {"root": root})["data"]["rev"]
    cnt = oplog_count(port, token, root)
    assert cnt == rev, f"{tag}: OpLog({cnt}) 与 rev({rev}) 不一致"


def v1_clips(port: int, token: str, root: str) -> list[dict]:
    doc = rpc(port, token, "project_get", {"root": root})["data"]["project"]
    return next(t["clips"] for t in doc["tracks"] if t["id"] == "V1")


def ui_rev_eq(page, val: int, timeout: int = 10000) -> None:
    page.wait_for_function(
        "v => document.querySelector('[data-testid=\"rev\"]').textContent === String(v)",
        arg=val, timeout=timeout)


def ui_edit_until_rev(page, port_: int, token_: str, root_: str, field_testid: str,
                      value: str, apply_testid: str, prev: int, attempts: int = 3,
                      window_s: float = 4.0) -> None:
    """UI 填字段 → 点应用 → 等服务端 rev 变化;失败重试。

    重试兜底的竞态:bgm 面板在每笔投影到达时以工程值回填输入框(fillFrom),
    若上一笔 op 的 reproject 尚未到达,fill 的草稿会被清空 → 本笔点击无效果。"""
    for _ in range(attempts):
        page.fill(f'[data-testid="{field_testid}"]', value)
        page.click(f'[data-testid="{apply_testid}"]')
        deadline = time.time() + window_s
        while time.time() < deadline:
            if rpc(port_, token_, "project_get", {"root": root_})["data"]["rev"] != prev:
                return
            time.sleep(0.15)
    raise AssertionError(f"UI 编辑 {field_testid}={value} 在 {attempts} 次尝试后未落账")


def ruler_width(page) -> float:
    return page.evaluate(
        "() => parseFloat(document.querySelector('[data-testid=\"ruler\"]').style.width) || 0")


def external_append_clip(ws: Path, tl_rel: str, mat_rel: str, clip_id: str,
                         start_ms: int, dur_ms: int, volume: float | None = None) -> None:
    """外部进程语义:直接改 project.json 触发 watcher(同 e2e_events 探针口径)。"""
    pj = ws / tl_rel / "project.json"
    doc = json.loads(pj.read_text(encoding="utf-8"))
    v1 = next(t for t in doc["tracks"] if t["id"] == "V1")
    if volume is not None and v1["clips"]:
        v1["clips"][0]["volume"] = volume
    v1["clips"].append({"id": clip_id, "src": f"{mat_rel}/main.mp4",
                        "startMs": start_ms, "durationMs": dur_ms})
    pj.write_text(json.dumps(doc, ensure_ascii=False), encoding="utf-8")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=None)
    ap.add_argument("--cli", default=None)
    args = ap.parse_args()
    mcp = locate_bin(args.bin, ("cutforge-mcp",))
    cli = locate_bin(args.cli, ("cutforge-cli",))
    if not mcp:
        print("FAIL: 先 cargo build(cutforge-mcp)", file=sys.stderr)
        return 2
    if not cli:
        print("FAIL: 未找到 cutforge-cli(向导/工程夹具依赖 cli new)", file=sys.stderr)
        return 2
    if not shutil.which("ffmpeg"):
        print("FAIL: 本机无 ffmpeg(无法现场生成素材)", file=sys.stderr)
        return 2

    from playwright.sync_api import sync_playwright

    t0 = time.time()
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-ui-smoke-"))
    token = "ui-smoke-token"
    proj = tmp / "smoke-proj"
    cli_new(cli, proj)
    tl_rel, mat_rel = layout_rel(proj)
    (proj / mat_rel).mkdir(parents=True, exist_ok=True)
    make_media(proj / mat_rel, "main.mp4", 20)
    root = str(proj)

    serve = Serve(mcp, proj, tmp, token)
    serve2: Serve | None = None
    pageerrors: list[str] = []
    try:
        with sync_playwright() as pw:
            browser = pw.chromium.launch(args=["--autoplay-policy=no-user-gesture-required"])
            page = browser.new_page(viewport={"width": 1600, "height": 1000})
            page.on("pageerror", lambda e: pageerrors.append(str(e)))
            page.goto(f"http://127.0.0.1:{serve.port}/?token={token}")
            page.wait_for_function(
                "document.querySelector('[data-testid=\"rev\"]').textContent !== '-'", timeout=20000)
            rev0 = rpc(serve.port, token, "project_get", {"root": root})["data"]["rev"]
            assert rev0 == 0, f"空工程初始 rev 应为 0,实得 {rev0}"
            log("装配:新壳就绪(rev 翻牌,127+ testid 面): PASS")

            # ---------- 1. 全 tab 遍历 ----------
            for tab_testid, panel_dom_id in [("tab-notes", "tab-notes"), ("tab-diff", "tab-diff"),
                                             ("tab-conflicts", "tab-conflicts")]:
                page.click(f'[data-testid="{tab_testid}"]')
                page.wait_for_timeout(250)  # 面板自刷新(notes_list/oplog_tail/conflict_list)
                assert page.evaluate(f"document.getElementById('{panel_dom_id}').classList.contains('active')"), \
                    f"{tab_testid} 点击后面板未激活"
                assert page.evaluate(
                    f"document.querySelector('[data-testid=\"{tab_testid}\"]').classList.contains('active')")
            page.click('[data-testid="tab-timeline"]')
            assert page.evaluate("document.getElementById('tab-timeline').classList.contains('active')")
            log("1. 全 tab 遍历(timeline/notes/diff/conflicts 激活态): PASS")

            # ---------- 2. 素材导入(UI 双击 → clip_add) ----------
            page.wait_for_selector('[data-testid="media-item"]', timeout=15000)
            page.dblclick('[data-testid="media-item"]')
            rev1 = wait_server_rev(serve.port, token, root, rev0)
            tl = rpc(serve.port, token, "timeline_get", {"root": root})["data"]["clips"]
            assert len(tl) == 1 and tl[0]["track"] == "V1" and tl[0]["startMs"] == 0, f"导入落点: {tl}"
            assert abs(tl[0]["durationMs"] - MAT_CLIP_MS) <= 800, f"导入时长: {tl[0]}"
            ui_rev_eq(page, rev1)
            assert page.locator('[data-testid="clip"]').count() == 1, "时间线应出现 1 个片段节点"
            assert_ledger(serve.port, token, root, "导入")
            log(f"2. 素材导入(UI 双击 → clip_add,rev→{rev1},V1@0): PASS")

            # ---------- 3. 选中 + 检查器回填(ui-fields 驱动) ----------
            clip_id = tl[0]["id"]
            page.click('[data-testid="clip"]')  # 无位移点击 = 仅选中
            assert clip_id in page.inner_text('[data-testid="sel-info"]'), \
                f"sel-info 未含选中 id:{page.inner_text('[data-testid=\"sel-info\"]')}"
            assert page.input_value('[data-testid="insp-start"]') == str(tl[0]["startMs"]), \
                "检查器 startMs 草稿应与投影一致"
            log(f"3. 选中 {clip_id}(检查器草稿=投影): PASS")

            # ---------- 4. 检查器改字段(apply → clip_update) ----------
            ui_edit_until_rev(page, serve.port, token, root, "field-opacity", "0.9", "insp-apply", rev1)
            rev2 = rpc(serve.port, token, "project_get", {"root": root})["data"]["rev"]
            cur = next(c for c in v1_clips(serve.port, token, root) if c["id"] == clip_id)
            assert cur.get("opacity") == 0.9, f"opacity 未落地: {cur}"
            ui_rev_eq(page, rev2)
            assert_ledger(serve.port, token, root, "字段")
            log(f"4. 检查器 opacity 0.9(ui-fields 驱动,rev→{rev2}): PASS")

            # ---------- 5. undo / redo ----------
            page.click('[data-testid="undo"]')
            rev3 = wait_server_rev(serve.port, token, root, rev2)
            cur = next(c for c in v1_clips(serve.port, token, root) if c["id"] == clip_id)
            assert cur.get("opacity") != 0.9, f"undo 未还原 opacity: {cur}"
            page.click('[data-testid="redo"]')
            rev4 = wait_server_rev(serve.port, token, root, rev3)
            cur = next(c for c in v1_clips(serve.port, token, root) if c["id"] == clip_id)
            assert cur.get("opacity") == 0.9, f"redo 未回放 opacity: {cur}"
            ui_rev_eq(page, rev4)  # redo 的 reproject 收敛后再碰面板草稿
            assert_ledger(serve.port, token, root, "undo/redo")
            log(f"5. undo 还原 / redo 回放(rev {rev2}→{rev3}→{rev4}): PASS")

            # ---------- 6. BGM 设置(bgm_set 可撤销) ----------
            ui_edit_until_rev(page, serve.port, token, root, "bgm-src", f"{mat_rel}/main.mp4",
                              "bgm-apply", rev4)
            rev5 = rpc(serve.port, token, "project_get", {"root": root})["data"]["rev"]
            bgm = rpc(serve.port, token, "project_get", {"root": root})["data"]["project"].get("bgm")
            assert bgm and bgm.get("src") == f"{mat_rel}/main.mp4", f"BGM 未落地: {bgm}"
            assert_ledger(serve.port, token, root, "BGM")
            log(f"6. BGM 设置(bgm_set,rev→{rev5}): PASS")

            # ---------- 7. 标尺 seek + 右键菜单分割 ----------
            page.click('[data-testid="ruler"]', position={"x": int(2000 * PX_PER_MS), "y": 10})
            # 播放头文本在 rAF tick 里刷新,等收敛(帧磁吸 60 帧 = 2000ms)
            page.wait_for_function(
                "() => Math.abs(Number(document.querySelector('[data-testid=\"playhead-ms\"]').textContent) - 2000) <= 34",
                timeout=5000)
            ph = int(page.inner_text('[data-testid="playhead-ms"]'))
            assert abs(ph - 2000) <= 34, f"标尺 seek 播放头 {ph} ≠ 2000(PX_PER_MS 红线)"
            page.click('[data-testid="clip"]', button="right")
            page.wait_for_selector('[data-testid="context-menu"]', timeout=5000)
            page.click('[data-testid="context-menu"] button:has-text("分割")')
            rev6 = wait_server_rev(serve.port, token, root, rev5)
            clips_v1 = v1_clips(serve.port, token, root)
            assert len(clips_v1) == 2, f"分割后 V1 应 2 段: {[c['id'] for c in clips_v1]}"
            assert sorted(c["startMs"] for c in clips_v1) == [0, 2000], f"分割点: {clips_v1}"
            assert page.locator('[data-testid="context-menu"]').count() == 0, "菜单点击后未关闭"
            assert_ledger(serve.port, token, root, "分割")
            log(f"7. 右键菜单分割 @2000ms(rev→{rev6}): PASS")

            # ---------- 8. 向导模态(Esc / 焦点陷阱 / 焦点归还) ----------
            page.click('[data-testid="project-new"]')
            page.wait_for_selector('[data-testid="wizard"]', timeout=5000)
            assert page.evaluate(
                "document.querySelector('[data-testid=\"wizard\"] [role=\"dialog\"]') !== null"), \
                "向导卡片缺 role=dialog"
            for _ in range(6):
                page.keyboard.press("Tab")
                assert page.evaluate(
                    "document.querySelector('[data-testid=\"wizard\"]').contains(document.activeElement)"), \
                    "焦点陷阱失效:Tab 把焦点送出了对话框"
            page.keyboard.press("Escape")
            assert page.locator('[data-testid="wizard"]').count() == 0, "Esc 未关闭向导"
            assert page.evaluate("document.activeElement && document.activeElement.id") == "btn-project-new", \
                "焦点未归还触发按钮"
            rev_w = rpc(serve.port, token, "project_get", {"root": root})["data"]["rev"]
            assert rev_w == rev6, "取消向导不得产生服务端变更"
            log("8. 向导模态(role=dialog/焦点陷阱/Esc/焦点归还/零副作用): PASS")

            # ---------- 9. 轨头眼睛开关(ephemeral:不落盘/不撤销) ----------
            ledger0, revb0 = oplog_count(serve.port, token, root), rev6
            page.click('[data-testid="track-visibility-V1"]')
            assert page.get_attribute('[data-testid="track-visibility-V1"]', "aria-pressed") == "false"
            assert page.evaluate(
                "document.querySelector('[data-testid=\"track-lane-V1\"]').classList.contains('hidden-by-user')")
            assert oplog_count(serve.port, token, root) == ledger0, "眼睛开关不得产生 Op"
            assert rpc(serve.port, token, "project_get", {"root": root})["data"]["rev"] == revb0
            page.click('[data-testid="track-visibility-V1"]')
            assert page.get_attribute('[data-testid="track-visibility-V1"]', "aria-pressed") == "true"
            log("9. 轨头眼睛开关(ephemeral 视图隐藏,oplog/rev 零变化): PASS")

            # ---------- 10. 标注回路(创建 → AI 执行 caused_by → 回执结案) ----------
            page.click('[data-testid="tab-notes"]')
            page.fill('[data-testid="note-body"]', "smoke:这里语速偏快")
            page.click('[data-testid="note-create"]')
            deadline = time.time() + 8
            notes_open = []
            while time.time() < deadline:
                notes_open = [n for n in rpc(serve.port, token, "notes_list", {"root": root})["data"]["notes"]
                              if n["state"] == "open"]
                if notes_open:
                    break
                time.sleep(0.15)
            assert notes_open, "标注创建后服务端仍无 open 态"
            nid = notes_open[0]["id"]
            r = rpc(serve.port, token, "clip_update",
                    {"root": root, "clipId": clip_id, "patch": {"volume": 0.88},
                     "causedBy": [nid], "summary": f"按 {nid} 微调音量"})
            assert r["ok"] and r["data"]["opIds"], f"AI 执行: {r}"
            op_id = r["data"]["opIds"][0]
            page.wait_for_selector('[data-testid="note-row"]', timeout=8000)
            # 行重渲染竞态:notes_add 的 notes.changed 事件会触发第二次 refresh(),
            # fill 与点击之间行可能被重建(新行输入框为空 → 服务端 EmptyReply)。
            # 与 e2e_edit_ops M10-2 同口径:重查行→填→点→查服务端,失败重试。
            resolved = None
            for _ in range(4):
                row = page.locator('[data-testid="note-row"]').filter(has_text=nid).first
                row.locator('[data-testid="note-reply"]').fill("smoke 已改")
                row.locator('[data-testid="note-opids"]').fill(op_id)
                row.locator('[data-testid="note-resolve"]').click()
                deadline = time.time() + 3
                while time.time() < deadline:
                    found = [n for n in rpc(serve.port, token, "notes_list", {"root": root})["data"]["notes"]
                             if n["id"] == nid and n["state"] == "resolved"]
                    if found:
                        resolved = found[0]
                        break
                    time.sleep(0.15)
                if resolved:
                    break
            assert resolved and resolved.get("resolvedBy", {}).get("opIds") == [op_id], \
                (f"结案未绑定 opId: {resolved};服务端全量:"
                 f"{[(n['id'], n['state'], n.get('resolvedBy')) for n in rpc(serve.port, token, 'notes_list', {'root': root})['data']['notes']]};"
                 f"UI status={page.inner_text('[data-testid=\"status\"]')!r}")
            log(f"10. 标注回路(创建 {nid} → caused_by {op_id} → 回执结案): PASS")

            # ---------- 11. AC-2.3:MutationObserver 计一次 clip move ----------
            page.click('[data-testid="tab-timeline"]')
            clips_v1 = sorted(v1_clips(serve.port, token, root), key=lambda c: c["startMs"])
            move_id = clips_v1[-1]["id"]  # 末段右移无重叠顾虑
            page.click(f'[data-testid="clip"][data-id="{move_id}"]')
            page.wait_for_timeout(900)  # 预览循环休眠(QUIET_MS=600)后再开观察窗
            page.evaluate("""() => {
                window.__mutLog = [];
                const inTracks = (n) => {
                    const el = n && n.nodeType === 1 ? n : n && n.parentElement;
                    return !!(el && el.closest && el.closest('[data-testid="timeline-tracks"]'));
                };
                const mo = new MutationObserver((recs) => {
                    for (const r of recs) window.__mutLog.push({t: inTracks(r.target), y: r.type});
                });
                mo.observe(document.body, {childList: true, subtree: true, attributes: true, characterData: true});
                window.__mutMo = mo;
            }""")
            page.fill('[data-testid="insp-start"]', "5000")
            page.click('[data-testid="insp-apply"]')
            rev7 = wait_server_rev(serve.port, token, root, rev6)
            moved = next(c for c in v1_clips(serve.port, token, root) if c["id"] == move_id)
            assert moved["startMs"] == 5000, f"clip move 未落地: {moved}"
            ui_rev_eq(page, rev7)
            page.wait_for_timeout(800)  # 收敛余量(含 toast 生命期头)
            mut_log = page.evaluate("() => { const l = window.__mutLog; window.__mutMo.disconnect(); return l; }")
            scoped = [m for m in mut_log if m["t"]]
            kinds: dict[str, int] = {}
            for m in scoped:
                kinds[m["y"]] = kinds.get(m["y"], 0) + 1
            assert len(scoped) < MUTATION_BUDGET, \
                f"AC-2.3 失败:clip move 相关 DOM 变更 {len(scoped)} ≥ {MUTATION_BUDGET}({kinds})"
            assert_ledger(serve.port, token, root, "move")
            log(f"11. AC-2.3 clip move 增量渲染: 相关 DOM 变更 {len(scoped)} < {MUTATION_BUDGET}"
                f"(全页 {len(mut_log)},{kinds}): PASS")

            # ---------- 12. AC-2.5:__cutforgeSelfTest 重建铁律 ----------
            selftest = page.evaluate("window.__cutforgeSelfTest()")
            assert selftest and selftest.get("ok") is True, \
                f"AC-2.5 失败:重建 diff≠0(before={len(selftest.get('before',''))}B after={len(selftest.get('after',''))}B)"
            ui_rev_eq(page, rev7)
            log(f"12. AC-2.5 selfTest 重建 ok=true(快照 {len(selftest['before'])}B 逐字段相等): PASS")

            # ---------- 13. AC-2.6③ 断线重连:SSE 毒化 → 长轮询接管 ----------
            # 服务端 SSE 流寿命 30min,已有连接等不到自然重连;改为:先挂毒化路由
            # (SSE 请求灌坏 MIME → EventSource 判致命 CLOSED),再 reload 让壳以
            # 「SSE 必死」环境重新装配 → 降级长轮询(pollOnce)接管。
            lp_seen: list[float] = []

            def on_request(req):
                if re.search(r"/events(\?.*)?$", req.url) and \
                        "text/event-stream" not in (req.headers.get("accept") or ""):
                    lp_seen.append(time.time())

            page.on("request", on_request)

            def sse_poison(route):
                if "text/event-stream" in (route.request.headers.get("accept") or ""):
                    route.fulfill(status=200, content_type="text/plain", body="not-an-event-stream")
                else:
                    route.continue_()

            page.route(re.compile(r"/events(\?.*)?$"), sse_poison)
            page.reload()
            page.wait_for_function(
                "document.querySelector('[data-testid=\"rev\"]').textContent !== '-'", timeout=20000)
            page.wait_for_timeout(2000)
            assert lp_seen, "AC-2.6③:SSE 毒化后壳未转入长轮询(pollOnce 无请求)"
            # 恢复选中(reload 清会话态;显式选 move_id 供后续检查器写),再以外部改动验证长轮询仍驱动重投影
            page.click(f'[data-testid="clip"][data-id="{move_id}"]')
            width_before = ruler_width(page)
            external_append_clip(proj, tl_rel, mat_rel, "V1-901", 60000, 10000, volume=0.66)
            deadline = time.time() + 10
            width_after = width_before
            while time.time() < deadline:
                width_after = ruler_width(page)
                if width_after >= 70000 * PX_PER_MS - 20:
                    break
                time.sleep(0.15)
            assert width_after >= 70000 * PX_PER_MS - 20, \
                f"长轮询场景下外部改动未驱动重投影(标尺 {width_before}→{width_after})"
            # 恢复:壳仍可写(当前选中片段,检查器草稿随选中面)
            ui_edit_until_rev(page, serve.port, token, root, "field-opacity", "0.85", "insp-apply", rev7)
            rev8 = rpc(serve.port, token, "project_get", {"root": root})["data"]["rev"]
            cur = next(c for c in v1_clips(serve.port, token, root) if c["id"] == move_id)
            assert cur.get("opacity") == 0.85, f"恢复写失败: {cur}"
            page.unroute(re.compile(r"/events(\?.*)?$"))
            page.remove_listener("request", on_request)
            log(f"13. AC-2.6③ SSE 毒化→长轮询接管({len(lp_seen)} 次 pollOnce),"
                f"外部改动驱动重投影(标尺 {width_before:.0f}→{width_after:.0f}px),恢复可写(rev→{rev8}): PASS")

            # ---------- 14. AC-2.6① 超时:拦截 rpc 不响应 → 10s 中止 + 恢复 ----------
            # 证据面:Playwright 路由拦截发生在真实网络之前,页面 abort 拦截态请求
            # 不产生 requestfailed;改为包装 window.fetch 直接记录 Abort 异常与耗时,
            # 以「中止时刻 ≈10s + 服务端未落账 + 解除后可恢复」三点定案。
            rev9 = rpc(serve.port, token, "project_get", {"root": root})["data"]["rev"]
            page.evaluate("""() => {
                const orig = window.fetch.bind(window);
                window.__fetchLog = [];
                window.fetch = async (...args) => {
                    const t0 = performance.now();
                    try {
                        const r = await orig(...args);
                        window.__fetchLog.push({ms: performance.now() - t0, err: null});
                        return r;
                    } catch (e) {
                        window.__fetchLog.push({ms: performance.now() - t0, err: String(e && e.name || e)});
                        throw e;
                    }
                };
            }""")
            hung_routes: list = []

            def hang_route(route):
                hung_routes.append(route)  # 悬挂:永不放行,等壳的 AbortController 到点

            page.route(re.compile(r"/rpc$"), hang_route)
            t_hang = time.time()
            page.fill('[data-testid="field-opacity"]', "0.8")
            page.click('[data-testid="insp-apply"]')
            deadline = t_hang + RPC_TIMEOUT_MS / 1000 + 8
            abort_entry = None
            while time.time() < deadline:
                entries = page.evaluate("window.__fetchLog")
                # api.js 以 ctrl.abort("timeout") 中止:reject 值即字符串 "timeout"
                # (非 AbortError 异常),两条口径都认。
                abort_entry = next((e for e in entries if e["err"] and
                                    ("Abort" in e["err"] or "timeout" in e["err"])), None)
                if abort_entry:
                    break
                time.sleep(0.2)
            assert abort_entry, \
                f"AC-2.6①:壳未在预算内中止被悬挂的 rpc(fetchLog={page.evaluate('window.__fetchLog')})"
            dt = abort_entry["ms"] / 1000
            assert 9.0 <= dt <= 13.0, \
                f"AC-2.6①:中止时刻 {dt:.2f}s 不符合 10s 超时口径(err={abort_entry['err']})"
            rev_still = rpc(serve.port, token, "project_get", {"root": root})["data"]["rev"]
            assert rev_still == rev9, f"超时失败命令不得落账: {rev9} → {rev_still}"
            for hr in hung_routes:
                try:
                    hr.abort()  # 收尾:完成拦截,不留悬挂句柄
                except Exception:
                    pass
            page.unroute(re.compile(r"/rpc$"))
            ui_edit_until_rev(page, serve.port, token, root, "field-opacity", "0.8", "insp-apply", rev9)
            rev10 = rpc(serve.port, token, "project_get", {"root": root})["data"]["rev"]
            cur = next(c for c in v1_clips(serve.port, token, root) if c["id"] == move_id)
            assert cur.get("opacity") == 0.8, f"超时后恢复写失败: {cur}"
            log(f"14. AC-2.6① rpc 悬挂 {dt:.1f}s 后壳按 10s 超时中止(fetch Abort),"
                f"未落账(rev 保持 {rev9}),解除后恢复(rev→{rev10}): PASS")

            # ---------- 15. AC-2.6② 401 分流 ----------
            # (a) 坏 token 启动:横幅 + 壳停摆
            page2 = browser.new_page(viewport={"width": 1200, "height": 800})
            page2.goto(f"http://127.0.0.1:{serve.port}/?token=wrong-token")
            page2.wait_for_selector('[data-testid="banner-token"]:not([hidden])', timeout=8000)
            banner_txt = page2.inner_text('[data-testid="banner-token"]')
            assert banner_txt.strip(), "token 横幅为空"
            assert page2.inner_text('[data-testid="rev"]').strip() == "-", "坏 token 下投影不应到达"
            page2.close()
            # (b) 服务换 token 重启(同端口)后,旧页面数据面 rpc → 401 分流 toast
            serve.stop()
            serve2 = Serve(mcp, proj, tmp, "rotated-token", want_port=serve.port)
            page.click('[data-testid="media-refresh"]')
            page.wait_for_function(
                "document.querySelector('[data-testid=\"status\"]').textContent.includes('鉴权失败')",
                timeout=10000)
            healthy = rpc(serve2.port, "rotated-token", "project_get", {"root": root})
            assert healthy["ok"], "换 token 后服务端应照常服务"
            log("15. AC-2.6② 坏 token 启动横幅 + 换 token 后数据面 401 分流 toast: PASS")

            assert not pageerrors, f"页面 JS 异常: {pageerrors[:5]}"
            browser.close()

        print(f"e2e_ui_smoke: 全部 PASS(耗时 {time.time() - t0:.1f}s)")
        return 0
    finally:
        serve.stop()
        if serve2 is not None:
            serve2.stop()
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except AssertionError as e:
        print(f"FAIL: {e}", file=sys.stderr)
        sys.exit(2)
