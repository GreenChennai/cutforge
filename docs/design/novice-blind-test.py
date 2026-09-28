#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""新手盲测脚本(T3.7 / AC-3.7;一次性工具,归档于 docs/design/ 附带)。

    python docs/design/novice-blind-test.py [--bin cutforge-mcp] [--cli cutforge-cli]

「盲」的纪律:三条流程的每一步只用「界面上看得见的东西」驱动——
可见按钮文本 / 可见列表项文本 / 悬停提示(title)/ 下拉选项文本。
禁用 data-testid 与任何源码知识;找不到下一步动作即记卡点(blocker)。

流程:
  ① 导入 3 个素材 → 剪切(右键分割)→ 导出成片;
  ② 给片段加转场(检查器转场分组 → 叠化 → 应用);
  ③ 加 BGM(素材行「BGM」按钮 → 应用 BGM)。

断言纪律(与 e2e 同口径):UI 只作驱动,以服务端状态(rev/盘面)为准。
退出码:0 三流程全过 / 2 有卡点。卡点→修复对照见 docs/design/novice-audit.md。
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

REPO = Path(__file__).resolve().parents[2]

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

BLOCKERS: list[str] = []


def log(msg: str) -> None:
    print(f"[blind] {msg}", flush=True)


def blocker(step: str, detail: str) -> None:
    BLOCKERS.append(f"{step}: {detail}")
    log(f"卡点: {step} — {detail}")


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def rpc(port: int, token: str, name: str, args: dict, timeout: float = 30) -> dict:
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
            time.sleep(0.4)
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


class Serve:
    def __init__(self, bin_path: Path, root: Path, tmp: Path, token: str):
        self.token = token
        self.port = free_port()
        self.proc = subprocess.Popen(
            [str(bin_path), "serve", "--root", str(root), "--port", str(self.port),
             "--token", token, "--web", str(REPO / "apps" / "web")],
            stdout=subprocess.DEVNULL, stderr=open(tmp / "serve.log", "wb"))
        deadline = time.time() + 10
        self.ready = False
        while time.time() < deadline:
            try:
                urllib.request.urlopen(
                    f"http://127.0.0.1:{self.port}/session?token={token}", timeout=1).read()
                self.ready = True
                break
            except Exception:
                time.sleep(0.15)
        if not self.ready:
            raise AssertionError("serve 未就绪")

    def stop(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()


def make_media(ws: Path, name: str, seconds: int, freq: int = 440) -> None:
    r = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
            "-i", f"testsrc2=size=320x180:rate=30", "-f", "lavfi",
            f"-i", f"sine=frequency={freq}:duration={seconds}",
            "-t", str(seconds), "-pix_fmt", "yuv420p",
            "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
            str(ws / name)])
    assert r.returncode == 0, f"ffmpeg 造素材失败:{r.stderr[-300:]}"


def wait_rev_change(port: int, token: str, root: str, prev: int, timeout_s: float = 20.0) -> int:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        r = rpc(port, token, "project_get", {"root": root})["data"]["rev"]
        if r != prev:
            return r
        time.sleep(0.15)
    raise AssertionError(f"rev 在 {timeout_s}s 内未从 {prev} 变化")


def layout_rel(proj: Path) -> tuple[str, str]:
    if (proj / "05_时间线工程").is_dir():
        return "05_时间线工程", "01_原始素材"
    return "05_ir", "01_materials"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=None)
    ap.add_argument("--cli", default=None)
    args = ap.parse_args()
    mcp = locate_bin(args.bin, ("cutforge-mcp",))
    cli = locate_bin(args.cli, ("cutforge-cli",))
    if not mcp or not cli:
        print("FAIL: 先 cargo build(cutforge-mcp / cutforge-cli)", file=sys.stderr)
        return 2
    if not shutil.which("ffmpeg"):
        print("FAIL: 本机无 ffmpeg", file=sys.stderr)
        return 2

    from playwright.sync_api import sync_playwright

    tmp = Path(tempfile.mkdtemp(prefix="cutforge-blind-"))
    token = "blind-token"
    proj = tmp / "blind-proj"
    r = sh([str(cli), "new", str(proj), "--slug", "blind-proj", "--json",
            "--fps", "30", "--width", "1080", "--height", "1920", "--track", "video,audio"])
    assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"
    tl_rel, mat_rel = layout_rel(proj)
    mat = proj / mat_rel
    mat.mkdir(parents=True, exist_ok=True)
    make_media(mat, "开场.mp4", 8, 440)
    make_media(mat, "采访.mp4", 8, 550)
    make_media(mat, "空镜.mp4", 8, 660)
    make_media(mat, "配乐.mp3" if False else "配乐.wav", 6, 330)
    root = str(proj)

    serve = Serve(mcp, proj, tmp, token)
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

            # ============ 流程① 导入 3 个素材 → 剪切 → 导出 ============
            log("—— 流程①:导入 3 素材 → 剪切 → 导出 ——")
            # 新手等列表加载完(面板显示「加载中…」时人会等;纯文本可见性等待)
            try:
                page.wait_for_selector("text=开场.mp4", timeout=20000)
            except Exception:
                blocker("①导入", "素材列表 20s 内没有显示出文件(加载不可见或不结束)")
                raise
            # 新手第一步:看到「素材面板」标题和文件列表;双击文件名(提示文案说双击插入)
            for name in ("开场.mp4", "采访.mp4", "空镜.mp4"):
                item = page.get_by_text(name, exact=False).first
                if item.count() == 0:
                    blocker("①导入", f"素材列表里看不到「{name}」")
                    continue
                prev = rpc(serve.port, token, "project_get", {"root": root})["data"]["rev"]
                item.dblclick()
                try:
                    wait_rev_change(serve.port, token, root, prev)
                except AssertionError:
                    blocker("①导入", f"双击「{name}」后工程无变化(toasts={page.inner_text('[data-testid=\"toasts\"]')[-160:]}!)")
                    raise
            clips = rpc(serve.port, token, "timeline_get", {"root": root})["data"]["clips"]
            assert len(clips) == 3, f"导入后应有 3 个片段: {len(clips)}"
            log("① 导入 3 素材(双击文件名,提示文案指引): PASS")

            # 剪切:右键中间片段 → 可见菜单项「分割」
            rev_a = rpc(serve.port, token, "project_get", {"root": root})["data"]["rev"]
            clip_el = page.locator('[data-testid="clip"]').nth(1)
            clip_el.click(button="right")
            try:
                page.wait_for_selector('[data-testid="context-menu"]', timeout=4000)
            except Exception:
                blocker("①剪切", "右键片段没有出现菜单")
                raise
            split_btn = page.get_by_role("menuitem").filter(has_text="分割")
            if split_btn.count() == 0:
                blocker("①剪切", "菜单里没有「分割」这样的可见词")
                raise AssertionError("无分割菜单项")
            split_btn.first.click()
            rev_b = wait_rev_change(serve.port, token, root, rev_a)
            clips = rpc(serve.port, token, "timeline_get", {"root": root})["data"]["clips"]
            assert len(clips) == 4, f"分割后应 4 段: {len(clips)}"
            log(f"① 剪切(右键 →「分割」): PASS(rev→{rev_b})")

            # 导出:可见按钮「导出成片」→ 进度出现「完成」
            rev_c = rpc(serve.port, token, "project_get", {"root": root})["data"]["rev"]
            export_btn = page.get_by_role("button", name=re.compile("导出成片"))
            if export_btn.count() == 0:
                blocker("①导出", "界面上找不到「导出成片」按钮")
                raise AssertionError("无导出按钮")
            export_btn.click()
            page.wait_for_function(
                "() => document.querySelector('[data-testid=\"export-progress\"]')?.textContent.includes('完成')",
                timeout=120000)
            files = rpc(serve.port, token, "render_probe", {"root": root})
            assert files["ok"] and files["data"]["files"], "导出后产物清单为空"
            log("① 导出成片(按钮→进度「完成」): PASS")

            # ============ 流程② 加转场 ============
            log("—— 流程②:给片段加转场 ——")
            rev_d = rpc(serve.port, token, "project_get", {"root": root})["data"]["rev"]
            # 新手:选中一个片段 → 检查器找「转场」相关字样
            page.locator('[data-testid="clip"]').first.click()
            insp = page.locator('[data-testid="inspector"]')
            # 点「转场」分组字样展开(若本已展开、一点反而收起,新手会再点一次)
            grp = insp.get_by_text("转场", exact=False).first
            if grp.count() == 0:
                blocker("②转场", "检查器里没有「转场」字样(字段不可发现)")
                raise AssertionError("无转场分组")
            sel = insp.locator("select").filter(
                has=page.locator("option", has_text="叠化")).first
            if sel.count() == 0:
                blocker("②转场", "检查器里没有转场下拉(找不到「叠化」选项)")
                raise AssertionError("无转场下拉")
            for _ in range(3):
                if sel.is_visible():
                    break
                grp.click()
                page.wait_for_timeout(250)
            if not sel.is_visible():
                blocker("②转场", "点「转场」分组后下拉仍不可见(展开状态不可发现)")
                raise AssertionError("转场下拉不可见")
            sel.select_option(label="叠化")
            apply_btn = insp.get_by_role("button", name=re.compile("应用"))
            apply_btn.click()
            rev_e = wait_rev_change(serve.port, token, root, rev_d)
            doc = rpc(serve.port, token, "project_get", {"root": root})["data"]["project"]
            trans = [c.get("transition", {}).get("type") for t in doc["tracks"] for c in t["clips"]]
            assert "fade" in trans, f"转场未落地: {trans}"
            log(f"② 加转场(检查器「转场」→叠化→应用): PASS(rev→{rev_e})")

            # ============ 流程③ 加 BGM ============
            log("—— 流程③:加 BGM ——")
            rev_f = rpc(serve.port, token, "project_get", {"root": root})["data"]["rev"]
            bgm_btn = page.get_by_role("button", name="BGM", exact=True)
            if bgm_btn.count() == 0:
                blocker("③BGM", "素材行上没有「BGM」按钮")
                raise AssertionError("无 BGM 按钮")
            bgm_btn.first.click()  # 音频行「BGM」= 一键设为背景乐(title 说明)
            wait_rev_change(serve.port, token, root, rev_f)
            doc = rpc(serve.port, token, "project_get", {"root": root})["data"]["project"]
            assert doc.get("bgm", {}).get("src"), f"BGM 未落地: {doc.get('bgm')}"
            log("③ 加 BGM(素材行「BGM」按钮): PASS")

            assert not pageerrors, f"页面 JS 异常: {pageerrors[:5]}"
            browser.close()

        if BLOCKERS:
            print(f"\n盲测完成但有 {len(BLOCKERS)} 个卡点:")
            for b in BLOCKERS:
                print(f"  - {b}")
            return 2
        print("\n盲测三流程全部通过(零卡点)。")
        return 0
    finally:
        serve.stop()
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except AssertionError as e:
        print(f"FAIL: {e}", file=sys.stderr)
        if BLOCKERS:
            for b in BLOCKERS:
                print(f"  - 卡点: {b}", file=sys.stderr)
        sys.exit(2)
