#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册三 AC-3.6 可访问性 e2e(e2e_a11y):纯键盘编辑链路 + axe 全页扫描。

    python tools/e2e_a11y.py [--bin cutforge-mcp] [--cli cutforge-cli]

断言链:
  1. 键盘链路(全程不碰鼠标;每步断言 shortcut-gate 裁决 + 服务端 rev/OpLog):
     选择片段(Alt+←/→,会话态零 Op)→ 移动(Ctrl+→ 帧 nudged,rev+1)→
     删除(Del,rev+1)→ 撤销两条路径:toast「撤销」按钮(5s 窗,rev+1 还原)与
     Ctrl+Z(再删后键盘撤销,rev+1 还原);
  2. axe 扫描:tools/vendor/axe.min.js(axe-core 4,入库存档)注入页面全页扫描,
     断言 0 critical/serious 违规;moderate/minor 逐条打印登记不阻断;
     vendor 缺失时降级为手写检查器(role/aria-label/焦点可达三类),报告注明降级。

断言纪律(M10-R5):UI 动作只作驱动,断言以服务端状态(rev/oplog/盘面)为准。
跨平台:pathlib;二进制定位带无 .exe 回退;只依赖 playwright + stdlib(+ ffmpeg)。
退出码:0 通过 / 2 失败。
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
PX_PER_MS = 0.06
FPS = 30
MEDIA_S = 30
AXE_VENDOR = REPO / "tools" / "vendor" / "axe.min.js"

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[a11y] {msg}", flush=True)


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
            "-i", "testsrc2=size=480x270:rate=30", "-f", "lavfi",
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
    deadline = time.time() + timeout_s
    last: list[dict] = []
    while time.time() < deadline:
        last = v1_clips(port, token, root)
        if pred(last):
            return last
        time.sleep(0.15)
    raise AssertionError(f"{what}: {timeout_s}s 内盘面未收敛,末态 {last}")


def press_key(page, key: str, gate: str, what: str) -> None:
    page.evaluate("() => { const a = document.activeElement; if (a && a !== document.body && a.blur) a.blur(); }")
    page.keyboard.press(key)
    got = page.get_attribute('[data-testid="shortcut-gate"]', "data-gate")
    assert got == f"hit:{gate}", f"{what}: shortcut-gate={got!r} 期望 hit:{gate!r}"


HANDMADE_A11Y = """() => {
    const vis = (el) => !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length);
    const out = { buttons: 0, badButtons: [], noRoleDialog: [], ariaMissing: [], focusables: 0 };
    for (const b of document.querySelectorAll("button")) {
        if (!vis(b)) continue;
        out.buttons += 1;
        const named = (b.textContent || "").trim() || b.getAttribute("aria-label")
            || b.getAttribute("title") || b.getAttribute("data-tip");
        if (!named) out.badButtons.push(b.getAttribute("data-testid") || b.id || b.className);
    }
    for (const el of document.querySelectorAll("[data-testid]")) {
        const t = el.getAttribute("data-testid") || "";
        if (/^(wizard|help-dialog|settings-dialog|jy-dialog|confirm-dialog)$/.test(t) && vis(el)
            && !el.querySelector("[role='dialog']")) out.noRoleDialog.push(t);
    }
    const landmarks = [...document.querySelectorAll("header, nav, main, footer, [role]")];
    if (!landmarks.length) out.ariaMissing.push("无任何地标/role 元素");
    for (const sel of ["button", "input", "select", "textarea", "a[href]"]) {
        for (const el of document.querySelectorAll(sel)) {
            if (vis(el)) out.focusables += 1;
        }
    }
    if (!document.querySelector("input, select, button:focus, [tabindex]")) {
        out.ariaMissing.push("无原生表单控件(焦点可达面存疑)");
    }
    return out;
}"""

# 已登记违规(册三收尾 axe 首扫曾登记两笔壳侧遗留:#media-dir 无关联 label(critical)、
# .cf-tooltip 空单例缺 aria-hidden(serious);A3 收口已在 apps/web 壳侧修复)。
# 登记表现已清空 → 恢复全量阻断:任何 critical/serious 违规即 FAIL。
KNOWN_REGISTERED = set()


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
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-a11y-"))
    token = "a11y-token"
    proj = tmp / "a11y-proj"
    r = sh([str(cli), "new", str(proj), "--slug", "a11y-proj", "--json", "--fps", str(FPS),
            "--width", "1080", "--height", "1920", "--track", "video,audio"])
    assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"
    tl_rel, mat_rel = layout_rel(proj)
    (proj / mat_rel).mkdir(parents=True, exist_ok=True)
    make_media(proj / mat_rel, "a.mp4", MEDIA_S, 440)
    make_media(proj / mat_rel, "b.mp4", MEDIA_S, 660)

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
            if page.locator('[data-testid="onboard-dismiss"]').count():
                page.click('[data-testid="onboard-dismiss"]')

            # ---------- 0. 导入两素材(键盘链路的盘面基础) ----------
            page.wait_for_selector('[data-testid="media-item"]', timeout=15000)
            items = page.locator('[data-testid="media-item"]')
            items.nth(0).dblclick()
            rev1 = wait_server_rev(port, token, root, 0)
            items.nth(1).dblclick()
            rev2 = wait_server_rev(port, token, root, rev1)
            wait_clips_pred(port, token, root, lambda cs: len(cs) == 2, "导入两素材")
            # 壳面收敛(ruler 内容宽度 = ceil(60000×0.06)+40)
            page.wait_for_function(
                "() => parseFloat(document.querySelector('[data-testid=\"ruler\"]').style.width || '0')"
                " >= 3640", timeout=15000)

            # ---------- 1. 键盘链路:选择(会话态) ----------
            page.evaluate(
                "() => { const w = document.querySelector('[data-testid=\"timeline-wrap\"]'); w.scrollLeft = 0; }")
            # 双击导入后选中停在末段:Alt+← 收到首段,Alt+→ 前进,Alt+← 回首段(全程键盘)
            press_key(page, "Alt+ArrowLeft", "alt+arrowleft", "Alt+← 选到首段")
            sel1 = page.inner_text('[data-testid="sel-info"]')
            c0 = v1_clips(port, token, root)[0]["id"]
            assert c0 in sel1, f"Alt+← 应选中首段 {c0}:{sel1!r}"
            press_key(page, "Alt+ArrowRight", "alt+arrowright", "Alt+→ 选下一段")
            assert v1_clips(port, token, root)[1]["id"] in page.inner_text('[data-testid="sel-info"]')
            press_key(page, "Alt+ArrowLeft", "alt+arrowleft", "Alt+← 回选首段")
            assert server_rev(port, token, root) == rev2, "选择是会话态,零 Op"
            log(f"1. 键盘选择(Alt+←/→ 首段 {c0} ↔ 次段;会话态零 Op): PASS")

            # ---------- 2. 键盘移动(Ctrl+→ 帧 nudge,rev+1;挪末段避免与后继重叠被内核拒) ----------
            press_key(page, "Alt+ArrowRight", "alt+arrowright", "Alt+→ 选中末段")
            press_key(page, "Control+ArrowRight", "ctrl+arrowright", "Ctrl+→ 右移一帧")
            rev3 = wait_server_rev(port, token, root, rev2)
            assert oplog_count(port, token, root) == rev3, "nudge 后 OpLog 与 rev 一致"
            moved = wait_clips_pred(port, token, root,
                                    lambda cs: cs[1]["startMs"] == 30033,
                                    "Ctrl+→ 应右移一帧(30000→30033ms)")
            press_key(page, "Alt+ArrowLeft", "alt+arrowleft", "Alt+← 回选首段(删除目标)")
            log(f"2. 键盘移动(Ctrl+→ 1 帧,rev→{rev3},{moved[1]['id']} startMs→{moved[1]['startMs']}): PASS")

            # ---------- 3. 键盘删除(Del,rev+1;toast 带撤销按钮) ----------
            press_key(page, "Delete", "delete", "Del 删除选中")
            rev4 = wait_server_rev(port, token, root, rev3)
            wait_clips_pred(port, token, root, lambda cs: len(cs) == 1, "删除后剩 1 段")
            toast_btn = page.locator('[data-testid="toast-action"]')
            toast_btn.first.wait_for(state="visible", timeout=3000)  # toast 在 reproject 后弹出
            assert toast_btn.count() >= 1, "删除 toast 应带「撤销」按钮(5s 窗)"
            assert "撤销" in toast_btn.first.inner_text(), "撤销按钮文案"
            log(f"3. 键盘删除(Del,rev→{rev4};toast-action 在窗内): PASS")

            # ---------- 4. 撤销路径 A:toast 按钮(真撤销) ----------
            toast_btn.first.click()
            rev5 = wait_server_rev(port, token, root, rev4)
            assert oplog_count(port, token, root) == rev5, "撤销后 OpLog 与 rev 一致"
            restored = wait_clips_pred(port, token, root,
                                       lambda cs: len(cs) == 2 and cs[0]["startMs"] == 0,
                                       "toast 撤销应还原被删片段(回删除前位置 @0)")
            log(f"4. 撤销路径 A:toast「撤销」按钮(rev→{rev5},{c0} 还原@{restored[0]['startMs']}): PASS")

            # ---------- 5. 撤销路径 B:Ctrl+Z ----------
            press_key(page, "Alt+ArrowLeft", "alt+arrowleft", "Alt+← 重新选中首段(撤销后选中为空)")
            press_key(page, "Delete", "delete", "Del 再删")
            rev6 = wait_server_rev(port, token, root, rev5)
            wait_clips_pred(port, token, root, lambda cs: len(cs) == 1, "再删后剩 1 段")
            press_key(page, "Control+z", "ctrl+z", "Ctrl+Z 撤销删除")
            rev7 = wait_server_rev(port, token, root, rev6)
            wait_clips_pred(port, token, root,
                            lambda cs: len(cs) == 2 and cs[0]["startMs"] == 0,
                            "Ctrl+Z 应还原被删片段")
            assert oplog_count(port, token, root) == rev7, "OpLog 与 rev 一致"
            log(f"5. 撤销路径 B:Ctrl+Z(rev→{rev7},还原;全程键盘完成 编辑闭环): PASS")

            # ---------- 6. axe 全页扫描(0 critical/serious 阻断) ----------
            axe_done = False
            if AXE_VENDOR.is_file():
                page.add_script_tag(path=str(AXE_VENDOR))
                res = page.evaluate(
                    """() => axe.run(document, { resultTypes: ['violations'] })""")
                vios = res.get("violations", [])
                blocking_new = [v for v in vios
                                if v.get("impact") in ("critical", "serious")
                                and v["id"] not in KNOWN_REGISTERED]
                registered = [v for v in vios
                              if v.get("impact") in ("critical", "serious")
                              and v["id"] in KNOWN_REGISTERED]
                minor = [v for v in vios if v.get("impact") not in ("critical", "serious")]
                axe_done = True
                assert not blocking_new, (
                    "axe 扫描存在新增 critical/serious 违规(登记表外): "
                    + "; ".join(f"{v['id']}({v['impact']})×{len(v.get('nodes', []))}" for v in blocking_new))
                log(f"6. axe 扫描(vendor axe-core,全页):新增 critical/serious = 0;"
                    f"已登记违规 {len(registered)} 类(壳侧遗留,见脚本 KNOWN_REGISTERED): "
                    + (", ".join(f"{v['id']}({v['impact']})×{len(v.get('nodes', []))}" for v in registered) or "无")
                    + f";登记不阻断 {len(minor)} 类:"
                    + (", ".join(f"{v['id']}({v['impact']})×{len(v.get('nodes', []))}" for v in minor) or "无")
                    + ": PASS")
            else:  # 降级:手写检查器(role/aria-label/焦点可达三类),报告注明降级
                chk = page.evaluate(HANDMADE_A11Y)
                assert not chk["badButtons"], f"存在无可读名的按钮: {chk['badButtons'][:5]}"
                assert not chk["noRoleDialog"], f"模态缺 role=dialog: {chk['noRoleDialog']}"
                assert chk["focusables"] > 0, "无可见可聚焦控件"
                log(f"6. axe vendor 缺失,降级手写检查器(已注明):按钮 {chk['buttons']} 个全有可读名,"
                    f"焦点可达 {chk['focusables']} 个: PASS(降级)")

            assert not pageerrors, f"页面 JS 异常: {pageerrors[:5]}"
            browser.close()

        print("AC-3.6 可访问性: PASS")
        print(f"  键盘编辑闭环:选择(Alt+←/→)→移动(Ctrl+→)→删除(Del)→撤销(toast 按钮 + Ctrl+Z),"
              f"每步 rev/OpLog 断言;axe 扫描:{'vendor 注入,0 critical/serious' if axe_done else '降级手写检查器'}")
        print(f"e2e_a11y: 全部 PASS(耗时 {time.time() - t0:.1f}s)")
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
