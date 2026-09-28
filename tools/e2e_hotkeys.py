#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册三 AC-3.4 键位体系 e2e(e2e_hotkeys):全表遍历 + 抽样实按 + 重绑定 + 输入态屏蔽。

    python tools/e2e_hotkeys.py [--bin cutforge-mcp] [--cli cutforge-cli]

断言链:
  1. 全表遍历:window.__cfKeymap.table()(键位注册表导出面)≥40 条绑定,字段齐备
     (id/group/label/defaultCombo/combo),id 唯一;routeCount() == 非空生效组合数;
  2. 抽样实按 ≥15 条(覆盖计划点名链路):J/K/L 倍速链(J 倒放/L×2=2x/K 暂停)、
     I/O 入出点、B 切割模式、S 分割、Shift+Del 波纹删、Ctrl+Z/Y、+/- 缩放、\\ 适应、
     ? 帮助、Shift+D 性能面板;每按断言 shortcut-gate data-gate="hit:<组合>"
     (调度器裁决证据)+ 服务端状态(rev/盘面)或可见态变化;
  3. 重绑定路径:设置(Ctrl+,)→ 捕获新键 → 冲突检测提示 → 强制绑定 → 被占绑定停用
     (comboOf 实测)→ 恢复默认(全表回默认);
  4. 输入态屏蔽:焦点在输入框时按键 gate="input" 且零 Op;对话框打开时 gate="dialog";
  5. 帮助面板搜索:全表行数 == 注册表条数;关键词过滤;无匹配显示空态。

断言纪律(M10-R5):UI 动作只作驱动,断言以服务端状态(rev/oplog/盘面)为准;
键位生效组合/帮助行数属注册表面,以 __cfKeymap 与 DOM 实测为准。
跨平台:pathlib;二进制定位带无 .exe 回退;只依赖 playwright + stdlib(+ ffmpeg)。
退出码:0 通过 / 2 失败。
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
PX_PER_MS = 0.06
FPS = 30
MEDIA_S = 30
MIN_BINDINGS = 40

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[hotkeys] {msg}", flush=True)


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


def layout_rel(proj: Path) -> tuple[str, str]:
    if (proj / "05_时间线工程").is_dir():
        return "05_时间线工程", "01_原始素材"
    return "05_ir", "01_materials"


def make_media(ws: Path, name: str, seconds: int, freq: int) -> None:
    r = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
            "-i", f"testsrc2=size=480x270:rate=30", "-f", "lavfi",
            "-i", f"sine=frequency={freq}:duration={seconds}",
            "-t", str(seconds), "-pix_fmt", "yuv420p",
            "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
            str(ws / name)])
    assert r.returncode == 0 and (ws / name).is_file(), f"ffmpeg 生成素材失败:{r.stderr[-300:]}"


def server_rev(port: int, token: str, root: str) -> int:
    return rpc(port, token, "project_get", {"root": root})["data"]["rev"]


def oplog_count(port: int, token: str, root: str) -> int:
    return rpc(port, token, "oplog_tail", {"root": root, "limit": 1})["data"]["count"]


def wait_server_rev(port: int, token: str, root: str, prev: int, timeout_s: float = 15.0) -> int:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        r = server_rev(port, token, root)
        if r > prev:
            return r
        time.sleep(0.15)
    raise AssertionError(f"服务端 rev 在 {timeout_s}s 内未从 {prev} 上涨")


def v1_clips(port: int, token: str, root: str) -> list[dict]:
    doc = rpc(port, token, "project_get", {"root": root})["data"]["project"]
    return sorted(next(t["clips"] for t in doc["tracks"] if t["id"] == "V1"),
                  key=lambda c: c["startMs"])


def wait_clips_pred(port: int, token: str, root: str, pred, what: str, timeout_s: float = 15.0) -> list[dict]:
    """等待 V1 盘面收敛到 pred(多笔 Op 的命令族——如波纹删——单点读取会撞中间态)。"""
    deadline = time.time() + timeout_s
    last: list[dict] = []
    while time.time() < deadline:
        last = v1_clips(port, token, root)
        if pred(last):
            return last
        time.sleep(0.15)
    raise AssertionError(f"{what}: {timeout_s}s 内盘面未收敛,末态 {last}")


def press_key(page, key: str, gate: str | None, what: str, raw: bool = False) -> None:
    """失焦到 body 后实按;gate 非空时断言调度器裁决。
    raw=False:断言 hit:<gate>(命令命中口径);raw=True:gate 原样比对(input/dialog 屏蔽面)。"""
    page.evaluate("() => { const a = document.activeElement; if (a && a !== document.body && a.blur) a.blur(); }")
    page.keyboard.press(key)
    if gate is not None:
        want = gate if raw else f"hit:{gate}"
        got = page.get_attribute('[data-testid="shortcut-gate"]', "data-gate")
        assert got == want, f"{what}: shortcut-gate={got!r} 期望 {want!r}"


def playhead_ms(page) -> int:
    return int(page.inner_text('[data-testid="playhead-ms"]'))


def wait_playhead(page, want: int, timeout: int = 10000) -> int:
    """等待播放头文本精确到达 want(帧步进 ±33ms 与容差带同宽,禁用容差防读到旧值)。"""
    page.wait_for_function(
        """(w) => Number(document.querySelector('[data-testid="playhead-ms"]').textContent) === w""",
        arg=want, timeout=timeout)
    return playhead_ms(page)


def measure_delta_ms(page, window_s: float = 0.6) -> int:
    a = playhead_ms(page)
    time.sleep(window_s)
    b = playhead_ms(page)
    return b - a


def ruler_click_ms(page, ms: int) -> None:
    page.click('[data-testid="ruler"]', position={"x": int(ms * PX_PER_MS), "y": 10})
    wait_playhead(page, ms)


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
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-hotkeys-"))
    token = "hotkeys-token"
    proj = tmp / "keys-proj"
    r = sh([str(cli), "new", str(proj), "--slug", "keys-proj", "--json", "--fps", str(FPS),
            "--width", "1080", "--height", "1920", "--track", "video,audio"])
    assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"
    tl_rel, mat_rel = layout_rel(proj)
    (proj / mat_rel).mkdir(parents=True, exist_ok=True)
    make_media(proj / mat_rel, "a.mp4", MEDIA_S, 440)
    make_media(proj / mat_rel, "b.mp4", MEDIA_S, 660)
    make_media(proj / mat_rel, "c.mp4", MEDIA_S, 880)

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
        sampled: list[str] = []

        with sync_playwright() as pw:
            browser = pw.chromium.launch(args=["--autoplay-policy=no-user-gesture-required"])
            page = browser.new_page(viewport={"width": 1600, "height": 1000})
            pageerrors: list[str] = []
            page.on("pageerror", lambda e: pageerrors.append(str(e)))
            page.goto(f"http://127.0.0.1:{port}/?token={token}")
            page.wait_for_function(
                "document.querySelector('[data-testid=\"rev\"]').textContent !== '-'", timeout=20000)
            rev0 = server_rev(port, token, root)
            if page.locator('[data-testid="onboard-dismiss"]').count():
                page.click('[data-testid="onboard-dismiss"]')

            # ---------- 1. 键位注册表全表遍历 ----------
            table = page.evaluate("() => window.__cfKeymap.table()")
            assert isinstance(table, list) and len(table) >= MIN_BINDINGS, \
                f"注册表绑定 {len(table or [])} 条 < {MIN_BINDINGS}"
            ids = [b.get("id") for b in table]
            assert len(set(ids)) == len(ids), "绑定 id 有重复"
            bad = [b for b in table
                   if not b.get("id") or not b.get("group") or not b.get("label")
                   or b.get("defaultCombo") is None or b.get("combo") is None]
            assert not bad, f"绑定字段缺失: {bad[:3]}"
            route_n, eff_n = page.evaluate(
                """() => [window.__cfKeymap.routeCount(),
                          window.__cfKeymap.table().filter((b) => window.__cfKeymap.comboOf(b.id)).length]""")
            assert route_n == eff_n, f"routeCount({route_n}) ≠ 非空生效组合数({eff_n})"
            log(f"1. 键位注册表:{len(table)} 条绑定(≥{MIN_BINDINGS}),字段齐备,id 唯一,"
                f"routeCount={route_n} = 非空组合数: PASS")

            # ---------- 2. 导入三素材(90s 时间线;后续实按的盘面基础) ----------
            page.wait_for_selector('[data-testid="media-item"]', timeout=15000)
            items = page.locator('[data-testid="media-item"]')
            assert items.count() >= 3, "夹具应有三个素材卡"
            items.nth(0).dblclick()
            rev1 = wait_server_rev(port, token, root, rev0)
            items.nth(1).dblclick()
            wait_server_rev(port, token, root, rev1)
            items.nth(2).dblclick()
            rev2 = wait_server_rev(port, token, root, rev1 + 1)
            deadline = time.time() + 10
            clips = v1_clips(port, token, root)
            while time.time() < deadline and len(clips) < 3:
                time.sleep(0.2)
                clips = v1_clips(port, token, root)
            assert len(clips) == 3, f"应 3 段: {clips}"
            assert [c["startMs"] for c in clips] == [0, 30000, 60000], \
                f"双击插入落点(后继追加到轨尾): {clips}"
            # 壳面收敛:ruler 内容宽度到位(= ceil(end×px)+40;90s 片段远端被虚拟化,DOM 计数不可用)
            page.wait_for_function(
                "() => parseFloat(document.querySelector('[data-testid=\"ruler\"]').style.width || '0')"
                " >= 5440", timeout=15000)
            log(f"2. 素材导入 ×3(UI 双击;追加语义,时间线 90s,rev→{rev2}): PASS")

            # ---------- 3. J/K/L 倍速链 ----------
            ruler_click_ms(page, 5000)
            press_key(page, "Space", "space", "播放")
            sampled.append("space")
            page.wait_for_function(
                "document.querySelector('[data-testid=\"play-toggle\"]').textContent.includes('暂停')",
                timeout=5000)
            d1 = measure_delta_ms(page)
            assert d1 > 300, f"空格播放后播放头未推进(Δ{d1}ms)"
            press_key(page, "l", "l", "L 正放")
            sampled.append("l")
            d2 = measure_delta_ms(page)
            assert d2 >= d1 * 1.4, f"L×2 倍速未生效(1x Δ{d1}ms → 2x Δ{d2}ms)"
            press_key(page, "k", "k", "K 暂停")
            sampled.append("k")
            page.wait_for_function(
                "document.querySelector('[data-testid=\"play-toggle\"]').textContent.includes('播放')",
                timeout=5000)
            d_pause = measure_delta_ms(page, 0.4)
            assert abs(d_pause) <= 40, f"K 后播放头应停(Δ{d_pause}ms)"
            press_key(page, "j", "j", "J 倒放")
            sampled.append("j")
            d3 = measure_delta_ms(page)
            assert d3 < -200, f"J 应倒放(Δ{d3}ms)"
            press_key(page, "k", "k", "K 暂停(收尾)")
            page.wait_for_function(
                "document.querySelector('[data-testid=\"play-toggle\"]').textContent.includes('播放')",
                timeout=5000)
            log(f"3. J/K/L 链:space Δ{d1}ms → L×2 Δ{d2}ms → K 停 → J 倒放 Δ{d3}ms → K 停: PASS")

            # ---------- 4. Home / End / 帧步进 ----------
            press_key(page, "End", "end", "End 跳结尾")
            sampled.append("end")
            assert abs(wait_playhead(page, 90000) - 90000) <= 36, "End 应到时间线末尾(90s)"
            press_key(page, "Home", "home", "Home 跳开头")
            sampled.append("home")
            assert abs(wait_playhead(page, 0) - 0) <= 36, "Home 应回 0"
            press_key(page, "ArrowRight", "arrowright", "→ 前进一帧")
            sampled.append("arrowright")
            ph = wait_playhead(page, 33)
            assert 30 <= ph <= 37, f"→ 应 +1 帧(33ms),实得 {ph}"
            press_key(page, "ArrowLeft", "arrowleft", "← 后退一帧")
            sampled.append("arrowleft")
            assert wait_playhead(page, 0) <= 3, "← 应回 0"
            log(f"4. Home/End/帧步进(→ +1 帧 = {ph}ms): PASS")

            # ---------- 5. S 分割(播放头在选中片段内) ----------
            page.evaluate(
                "() => { const w = document.querySelector('[data-testid=\"timeline-wrap\"]'); w.scrollLeft = 0; }")
            page.wait_for_timeout(200)
            ruler_click_ms(page, 2000)
            first_id = v1_clips(port, token, root)[0]["id"]
            page.locator(f'[data-testid="clip"][data-id="{first_id}"]').click(position={"x": 300, "y": 14})
            press_key(page, "s", "s", "S 分割")
            sampled.append("s")
            rev3 = wait_server_rev(port, token, root, rev2)
            clips = v1_clips(port, token, root)
            assert len(clips) == 4, f"S 分割后应 4 段: {[c['id'] for c in clips]}"
            a1 = next(c for c in clips if c["startMs"] == 0)
            a2 = next(c for c in clips if c["startMs"] == 2000)
            assert a1["durationMs"] == 2000 and a2["durationMs"] == 28000, f"分割点: {clips}"
            assert oplog_count(port, token, root) == rev3, "OpLog 与 rev 应一致"
            log(f"5. S 分割 @2000ms(rev→{rev3},4 段;a1={a1['id']}/a2={a2['id']}): PASS")

            # ---------- 6. I / O 入出点(可见 toast;会话级不落盘) ----------
            ruler_click_ms(page, 2000)
            press_key(page, "i", "i", "I 设入点")
            sampled.append("i")
            assert "入出点" in page.inner_text('[data-testid="toasts"]'), "I 后应见入出点 toast"
            assert server_rev(port, token, root) == rev3, "I/O 会话级不得落盘"
            ruler_click_ms(page, 5000)
            press_key(page, "o", "o", "O 设出点")
            sampled.append("o")
            toast_txt = page.inner_text('[data-testid="toasts"]')
            assert "2.00s" in toast_txt and "5.00s" in toast_txt, f"I/O toast 口径: {toast_txt!r}"
            assert server_rev(port, token, root) == rev3, "I/O 会话级不得落盘"
            log(f"6. I/O 入出点(toast「{toast_txt.strip().splitlines()[0]}」;零 Op): PASS")

            # ---------- 7. Shift+Del 波纹删除(后继同轨整体左移) ----------
            # 点选宽片段(避开轨头 sticky 区),再 Alt+← 把选中切到窄段 V1-001(波纹删目标)。
            # 纪律:V1-001 仅 120px 宽,整体处于轨头 sticky 覆盖带,不可点——用键盘选择链到位。
            page.evaluate(
                "() => { const w = document.querySelector('[data-testid=\"timeline-wrap\"]'); w.scrollLeft = 0; }")
            page.wait_for_timeout(300)
            page.locator(f'[data-testid="clip"][data-id="{a2["id"]}"]').click(position={"x": 300, "y": 14})
            press_key(page, "Alt+ArrowLeft", "alt+arrowleft", "Alt+← 选回窄段")
            assert a1["id"] in page.inner_text('[data-testid="sel-info"]'), \
                f"波纹删目标应是 {a1['id']}:{page.inner_text('[data-testid=\"sel-info\"]')!r}"
            page.evaluate("() => { const a = document.activeElement; if (a && a !== document.body && a.blur) a.blur(); }")
            page.keyboard.down("Shift")
            page.keyboard.press("Delete")
            page.keyboard.up("Shift")
            got = page.get_attribute('[data-testid="shortcut-gate"]', "data-gate")
            assert got == "hit:shift+delete", f"Shift+Del gate={got!r}"
            sampled.append("shift+delete")
            # 波纹删 = clip_delete + 每个后继一笔 clip_move(多笔 Op):必须等盘面收敛再断言
            wait_clips_pred(port, token, root,
                            lambda cs: len(cs) == 3 and [c["startMs"] for c in cs] == [0, 28000, 58000],
                            "波纹左移口径(被删 2000ms,三个后继整体左移)")
            rev4 = wait_server_rev(port, token, root, rev3)
            assert oplog_count(port, token, root) == rev4, "OpLog 与 rev 应一致"
            log(f"7. Shift+Del 波纹删除({a1['id']} 删,三个后继左移 2000ms,rev→{rev4}): PASS")

            # ---------- 8. Ctrl+Z / Ctrl+Y ----------
            press_key(page, "Control+z", "ctrl+z", "Ctrl+Z 撤销")
            sampled.append("ctrl+z")
            wait_clips_pred(port, token, root, lambda cs: cs[-1]["startMs"] == 60000,
                            "undo 应还原最后一笔移动(c@60000)")
            rev5 = wait_server_rev(port, token, root, rev4)
            press_key(page, "Control+y", "ctrl+y", "Ctrl+Y 重做")
            sampled.append("ctrl+y")
            wait_clips_pred(port, token, root, lambda cs: cs[-1]["startMs"] == 58000,
                            "redo 应回放移动(c@58000)")
            rev6 = wait_server_rev(port, token, root, rev5)
            log(f"8. Ctrl+Z 撤销 / Ctrl+Y 重做(rev {rev4}→{rev5}→{rev6}): PASS")

            # ---------- 9. Del 删除(非波纹) + Ctrl+Z 还原 ----------
            first_id = v1_clips(port, token, root)[0]["id"]
            page.evaluate(
                "() => { const w = document.querySelector('[data-testid=\"timeline-wrap\"]'); w.scrollLeft = 0; }")
            page.wait_for_timeout(300)
            page.locator(f'[data-testid="clip"][data-id="{first_id}"]').click(position={"x": 300, "y": 14})
            press_key(page, "Delete", "delete", "Del 删除")
            sampled.append("delete")
            wait_clips_pred(port, token, root,
                            lambda cs: len(cs) == 2 and [c["startMs"] for c in cs] == [28000, 58000],
                            "非波纹删后后继不动(剩 2 段,位置不变)")
            rev7 = wait_server_rev(port, token, root, rev6)
            press_key(page, "Control+z", "ctrl+z", "Ctrl+Z 还原删除")
            wait_clips_pred(port, token, root,
                            lambda cs: len(cs) == 3 and cs[0]["startMs"] == 0
                            and cs[1]["startMs"] == 28000 and cs[2]["startMs"] == 58000,
                            "删除应可撤销(还原后 3 段)")
            rev8 = wait_server_rev(port, token, root, rev7)
            log(f"9. Del 非波纹删 + Ctrl+Z 还原(rev→{rev8},3 段,后继位置不变): PASS")

            # ---------- 10. Alt+←/→ 片段选择链 ----------
            # 从已知选中出发(undo 可能恢复选中,不假设 null):Alt+← 收到首段 → → 下一段 → ← 回首段。
            press_key(page, "Alt+ArrowLeft", "alt+arrowleft", "Alt+← 收到首段")
            sampled.append("alt+arrowleft")
            sel0 = page.inner_text('[data-testid="sel-info"]')
            press_key(page, "Alt+ArrowRight", "alt+arrowright", "Alt+→ 选下一个")
            sampled.append("alt+arrowright")
            sel1 = page.inner_text('[data-testid="sel-info"]')
            assert sel1 != sel0 and "未选中" not in sel1, f"Alt+→ 应切换选中: {sel0!r} → {sel1!r}"
            press_key(page, "Alt+ArrowLeft", "alt+arrowleft", "Alt+← 选上一个")
            sel3 = page.inner_text('[data-testid="sel-info"]')
            assert sel3 == sel0, f"Alt+← 应回选: {sel3!r}"
            assert server_rev(port, token, root) == rev8, "选择是会话态,零 Op"
            log(f"10. Alt+←/→ 选择链({sel0.strip()} → {sel1.strip()} → 回选;零 Op): PASS")

            # ---------- 11. Tab 侧面板开合 ----------
            press_key(page, "Tab", "tab", "Tab 收侧面板")
            sampled.append("tab")
            assert page.evaluate(
                "document.querySelector('[data-testid=\"workbench\"]').classList.contains('panels-collapsed')"), \
                "Tab 后侧面板应收起"
            press_key(page, "Tab", "tab", "Tab 展开侧面板")
            assert not page.evaluate(
                "document.querySelector('[data-testid=\"workbench\"]').classList.contains('panels-collapsed')"), \
                "再按 Tab 应展开"
            log("11. Tab 侧面板开合(workbench class): PASS")

            # ---------- 12. B 切割模式 / A 选择模式 ----------
            press_key(page, "b", "b", "B 切割模式")
            sampled.append("b")
            assert page.evaluate(
                "!document.querySelector('[data-testid=\"blade-mode\"]').hidden"), "B 后切割徽标应显"
            assert page.evaluate(
                "document.querySelector('[data-testid=\"timeline-wrap\"]').classList.contains('blade-mode')"), \
                "时间线 wrap 应挂 .blade-mode"
            press_key(page, "a", "a", "A 选择模式")
            sampled.append("a")
            assert page.evaluate(
                "document.querySelector('[data-testid=\"blade-mode\"]').hidden"), "A 后徽标应隐"
            assert server_rev(port, token, root) == rev8, "模式开关会话级零 Op"
            log("12. B 切割模式 / A 选择模式(徽标+wrap class;零 Op): PASS")

            # ---------- 13. ? 帮助面板 + 搜索 ----------
            press_key(page, "?", "?", "? 帮助")
            sampled.append("?")
            page.wait_for_selector('[data-testid="help-dialog"]', timeout=5000)
            help_rows = page.locator('[data-testid="help-row"]').count()
            assert help_rows == len(table), f"帮助行 {help_rows} ≠ 注册表 {len(table)}"
            page.fill('[data-testid="help-search"]', "分割")
            page.wait_for_timeout(150)
            filtered = page.locator('[data-testid="help-row"]').count()
            assert 0 < filtered < help_rows, f"搜索「分割」应过滤(实得 {filtered}/{help_rows})"
            page.fill('[data-testid="help-search"]', "zzz不存在的键")
            page.wait_for_timeout(150)
            assert page.evaluate(
                "!document.querySelector('[data-testid=\"help-empty\"]').hidden"), "无匹配应显示空态"
            page.keyboard.press("Escape")
            assert page.locator('[data-testid="help-dialog"]').count() == 0, "Esc 应关闭帮助"
            log(f"13. ? 帮助面板(行 {help_rows} = 注册表;搜索过滤 {filtered};空态;Esc): PASS")

            # ---------- 14. Shift+D 性能面板 ----------
            press_key(page, "Shift+d", "shift+d", "Shift+D 性能面板")
            sampled.append("shift+d")
            page.wait_for_selector('[data-testid="perf-panel"]', timeout=5000)
            budgets = page.locator('[data-testid="perf-budget"]')
            assert budgets.count() == 6, f"预算行应 6 行,实得 {budgets.count()}"
            press_key(page, "Shift+d", "shift+d", "Shift+D 关性能面板")
            assert page.locator('[data-testid="perf-panel"]').count() == 0, "再按应关闭"
            log("14. Shift+D 性能面板(6 行预算表,开/关): PASS")

            # ---------- 15. 重绑定路径(Ctrl+, → 捕获 → 冲突 → 强制 → 恢复默认) ----------
            press_key(page, "Control+Comma", "ctrl+,", "Ctrl+, 设置")
            sampled.append("ctrl+,")
            page.wait_for_selector('[data-testid="settings-dialog"]', timeout=5000)
            set_rows = page.locator('[data-testid^="keybind-row-"]').count()
            assert set_rows == len(table), f"设置面板行 {set_rows} ≠ 注册表 {len(table)}"
            route_before = page.evaluate("window.__cfKeymap.routeCount()")
            page.click('[data-testid="keybind-capture-view.fit"]')
            page.wait_for_selector('[data-testid="keybind-capture-zone-view.fit"]', timeout=3000)
            page.keyboard.press("m")  # 已被 mark.marker 占用 → 冲突检测
            page.wait_for_selector('[data-testid="keybind-conflict-view.fit"]', timeout=3000)
            conflict_txt = page.inner_text('[data-testid="keybind-conflict-view.fit"]')
            assert "冲突" in conflict_txt, f"冲突提示: {conflict_txt!r}"
            assert page.evaluate("window.__cfKeymap.comboOf('view.fit')") == "\\", \
                "冲突未裁决前原组合不得变更"
            page.click('[data-testid="keybind-force-view.fit"]')
            page.wait_for_timeout(150)
            assert page.evaluate("window.__cfKeymap.comboOf('view.fit')") == "m", "强制绑定未生效"
            assert page.evaluate("window.__cfKeymap.comboOf('mark.marker')") == "", \
                "被占绑定(mark.marker)应停用"
            route_after = page.evaluate("window.__cfKeymap.routeCount()")
            # 一换一停用口径:fit(\\→m)保持占一个路由,marker(m→停用)释放一个 → 净 -1
            assert route_after == route_before - 1, \
                f"强制绑定后路由数应 -1({route_before}→{route_after};被占绑定停用即释放路由)"
            toast_txt = page.inner_text('[data-testid="toasts"]')
            assert "停用" in toast_txt, f"强制绑定 toast 应提示停用: {toast_txt!r}"
            page.click('[data-testid="keybind-reset"]')
            page.wait_for_timeout(150)
            assert page.evaluate("window.__cfKeymap.comboOf('view.fit')") == "\\", "恢复默认失败(fit)"
            assert page.evaluate("window.__cfKeymap.comboOf('mark.marker')") == "m", "恢复默认失败(marker)"
            assert page.evaluate("window.__cfKeymap.routeCount()") == route_before, "恢复默认后路由数应回原"
            page.keyboard.press("Escape")
            assert page.locator('[data-testid="settings-dialog"]').count() == 0, "Esc 应关闭设置"
            log(f"15. 重绑定:捕获 m → 冲突提示 → 强制(fit=\\→m,marker 停用,"
                f"路由数 {route_before}→{route_after})→ 恢复默认: PASS")

            # ---------- 16. 输入态屏蔽(gate=input)与对话框屏蔽(gate=dialog) ----------
            rev_guard = server_rev(port, token, root)
            page.focus('[data-testid="media-dir"]')
            page.keyboard.press("s")
            got = page.get_attribute('[data-testid="shortcut-gate"]', "data-gate")
            assert got == "input", f"输入态 gate={got!r} 期望 input"
            assert server_rev(port, token, root) == rev_guard, "输入态按键不得产生 Op"
            page.click('[data-testid="project-new"]')
            page.wait_for_selector('[data-testid="wizard"]', timeout=5000)
            # 向导首个输入框自动聚焦;失焦到 body 后再按,裁决应走对话框屏蔽(input 判定优先,
            # 焦点在向导输入框内时 gate=input 亦为正确行为,故这里显式失焦后断言 dialog 面)。
            press_key(page, "s", "dialog", "对话框内按 S", raw=True)
            rev_w = server_rev(port, token, root)
            assert rev_w == rev_guard, "对话框内按键不得产生 Op"
            page.keyboard.press("Escape")
            assert page.locator('[data-testid="wizard"]').count() == 0
            log("16. 输入态/对话框屏蔽(gate=input / gate=dialog,零 Op): PASS")

            # ---------- 17. + / - 缩放与 \ 适应(放最后:改显示映射) ----------
            ruler_w = page.evaluate(
                "() => parseFloat(document.querySelector('[data-testid=\"ruler\"]').style.width) || 0")
            press_key(page, "+", "+", "+ 放大")
            sampled.append("+")
            w_in = page.evaluate(
                "() => parseFloat(document.querySelector('[data-testid=\"ruler\"]').style.width) || 0")
            assert w_in >= ruler_w * 1.2, f"+ 应放大(ruler {ruler_w:.0f}→{w_in:.0f}px)"
            press_key(page, "-", "-", "- 缩小")
            sampled.append("-")
            w_out = page.evaluate(
                "() => parseFloat(document.querySelector('[data-testid=\"ruler\"]').style.width) || 0")
            assert abs(w_out - ruler_w) <= 2, f"- 应回原({ruler_w:.0f}→{w_out:.0f}px)"
            wrap_w = page.evaluate(
                "() => document.querySelector('[data-testid=\"timeline-wrap\"]').clientWidth")
            press_key(page, "\\", "\\", "\\ 适应窗口")
            sampled.append("\\")
            w_fit = page.evaluate(
                "() => parseFloat(document.querySelector('[data-testid=\"ruler\"]').style.width) || 0")
            # 适应口径(view-ops.fitTimeline):内容几何 = ceil(end×px)+40;时间线 88s 在
            # wrapW/MIN 之外 → 压到最小档 PX_PER_MS_MIN=0.02(超长时间线的「装进视口」分支)
            end_ms = max(c["startMs"] + c["durationMs"] for c in v1_clips(port, token, root))
            w_fit_expect = math.ceil(end_ms * 0.02) + 40
            assert abs(w_fit - w_fit_expect) <= 8, \
                f"\\ 适应后 ruler {w_fit:.0f}px 应 ≈ end({end_ms}ms)×MIN(0.02)+40 = {w_fit_expect}px"
            log(f"17. + 放大({ruler_w:.0f}→{w_in:.0f}px)/ - 回原 / \\ 适应"
                f"({w_fit:.0f} ≈ end×MIN+40 = {w_fit_expect}px): PASS")

            assert len(sampled) >= 15, f"抽样实按 {len(sampled)} 条 < 15"
            need = {"j", "k", "l", "i", "o", "b", "s", "shift+delete", "ctrl+z", "ctrl+y",
                    "+", "-", "\\", "?", "shift+d"}
            missing = need - set(sampled)
            assert not missing, f"点名链路缺采样: {missing}"

            assert not pageerrors, f"页面 JS 异常: {pageerrors[:5]}"
            browser.close()

        print("AC-3.4 键位体系: PASS")
        print(f"  注册表 {len(table)} 条(≥{MIN_BINDINGS});抽样实按 {len(sampled)} 条(≥15,含点名链路全覆盖);"
              f"每次实按均有 gate=hit:<组合> 裁决证据")
        print(f"  重绑定闭环(冲突检测→强制→停用→恢复默认);输入态/对话框屏蔽;帮助搜索")
        print(f"e2e_hotkeys: 全部 PASS(耗时 {time.time() - t0:.1f}s)")
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
