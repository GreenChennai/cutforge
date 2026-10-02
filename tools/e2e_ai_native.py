#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册七 AI 原生正式 e2e(e2e_ai_native;G2 冒烟正式化,AC-7.2/7.3/7.5 判定器)。

    python tools/e2e_ai_native.py [--bin cutforge-mcp] [--cli cutforge-cli]

断言链(四场景):
  1. 脚本页签三内置片段(AC-7.3):按标记切割(会话标记 → 逐步 clip_split)/
     批量变色(全部视频片段 grade 整对象)/ 批量转场(相邻片段 fade)各「载入 →
     运行(preview_plan 副本 dry-run)→ 结构化逐步回执」;真工程零写入;
  2. 计划批准流(AC-7.5):以 plan 提交 → 差异面板 preview 卡片(逐项 ok/字段级
     before/after)→ 逐项批准/拒绝 → apply_plan 回执(落地/跳过/拒绝 + rev 链)→
     causedBy 对账(planId 在 OpLog 显式成行,actor=user)→ 被拒项真相源零落地;
     **P0 回归断言(文件级)**:预演后真工程 rev 不变 + oplog 逐文件字节一致 +
     工程 父目录无 .cf-scratch 残留(copy_tree 硬链接穿透回归,scratch.rs);
  3. 插件九步(AC-7.2):安装 → 校验(plugin_validate 面)→ 权限确认(首启对话框)
     → 三贡献点生效(命令/菜单右键项 + 面板专属页签受控渲染)→ 越权双拦截
     (宿主镜像 GUARD_FAILED/FORBIDDEN + 服务端 plugin-call 通道第二道,工程零变化)
     → 崩溃隔离(Worker 抛错 → 自动禁用)→ 禁用(贡献点摘除)→ 卸载(注册表移除);
  4. 标注线程两轮(note_reply 不改 state)+ session_report 人话 Markdown 渲染。

断言纪律(M10-R5):UI 动作只作驱动,断言以服务端状态(rev/oplog/工程回读)为准;
toast/徽标仅作 UI 到达性辅助观察。风格与既有 e2e 同(pathlib/testid/服务端断言)。
退出码:0 通过 / 2 失败。依赖:playwright(chromium)+ ffmpeg + cutforge-cli。
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
PX_PER_MS = 0.06  # 壳显示映射红线(apps/web/js/core/model.js)
MAT_SECONDS = 20  # 素材时长(V1 两段各 20s → 批量转场 1 步)

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[ai-native] {msg}", flush=True)


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
    return subprocess.run([str(c) for c in cmd], capture_output=True, text=True,
                          encoding="utf-8", errors="replace")


def make_media(ws: Path, name: str, seconds: int) -> None:
    ws.mkdir(parents=True, exist_ok=True)
    r = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
            "-i", "testsrc2=size=640x360:rate=30", "-f", "lavfi",
            "-i", f"sine=frequency=440:duration={seconds}",
            "-t", str(seconds), "-pix_fmt", "yuv420p",
            "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest",
            str(ws / name)])
    assert r.returncode == 0 and (ws / name).is_file(), f"ffmpeg 生成素材失败:{r.stderr[-300:]}"


class Serve:
    def __init__(self, bin_path: Path, root: Path, tmp: Path, token: str):
        self.token = token
        for attempt in range(6):
            self.port = free_port()
            self.proc = subprocess.Popen(
                [str(bin_path), "serve", "--root", str(root), "--port", str(self.port),
                 "--token", token, "--web", str(REPO / "apps" / "web")],
                stdout=subprocess.DEVNULL, stderr=open(tmp / f"serve-{self.port}.log", "wb"))
            deadline = time.time() + 10
            self.ready = False
            while time.time() < deadline:
                if self.proc.poll() is not None:
                    break
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


def server_rev(port: int, token: str, root: str) -> int:
    return rpc(port, token, "project_get", {"root": root})["data"]["rev"]


def v1_clips_by_start(port: int, token: str, root: str) -> list[dict]:
    doc = rpc(port, token, "project_get", {"root": root})["data"]["project"]
    v1 = next(t for t in doc["tracks"] if t["id"] == "V1")
    return sorted(v1["clips"], key=lambda c: c["startMs"])


def oplog_file_bytes(root: Path) -> dict[str, bytes]:
    """真工程 oplog 逐文件字节(P0 文件级断言的基准面)。"""
    d = root / ".cutforge" / "oplog"
    if not d.is_dir():
        return {}
    return {p.name: p.read_bytes() for p in sorted(d.glob("*.jsonl"))}


def scratch_residue(root: Path) -> list[str]:
    """工程父目录里的 .cf-scratch 残留(预演副本清理面)。"""
    parent = root.parent
    if not parent.is_dir():
        return []
    return [p.name for p in parent.iterdir() if p.name.startswith(".cf-scratch")]


def wait_toast(page, substr: str, timeout_s: float = 10.0) -> str:
    """轮询 toast 文本(到达性辅助观察;断言仍以服务端为准)。"""
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        for sel in ('[data-testid="toast"]', '[data-testid="toast-err"]'):
            for loc in page.locator(sel).all():
                t = loc.text_content() or ""
                if substr in t:
                    return t
        page.wait_for_timeout(200)
    raise AssertionError(f"toast 在 {timeout_s}s 内未出现: {substr!r}")


def load_and_run_builtin(page, index: int, expect_steps: int, tool: str) -> None:
    """脚本页签:选中内置片段 → 载入 → 运行(预演)→ 结构化输出断言。"""
    page.select_option('[data-testid="script-lib-select"]', index=index)
    page.click('[data-testid="script-lib-load"]')
    page.wait_for_timeout(150)
    assert "已载入" in page.inner_text('[data-testid="script-status"]'), \
        f"内置片段 #{index} 载入失败:{page.inner_text('[data-testid=\"script-status\"]')}"
    page.click('[data-testid="script-run"]')
    deadline = time.time() + 30
    while time.time() < deadline:
        if "预演完成" in page.inner_text('[data-testid="script-status"]'):
            break
        page.wait_for_timeout(200)
    else:
        raise AssertionError(f"内置片段 #{index} 预演未完成:{page.inner_text('[data-testid=\"script-status\"]')}")
    items = page.locator('[data-testid="script-out-item"]')
    assert items.count() == expect_steps, \
        f"内置片段 #{index} 结构化输出应 {expect_steps} 步,实得 {items.count()}"
    first = items.nth(0).inner_text()
    assert tool in first and "rev" in first and "ok" in first, f"逐步回执缺结构化面: {first}"


def enable_plugin(page, pid: str, first_run: bool = True) -> None:
    row = page.locator(f'[data-testid="plugin-row"][data-pid="{pid}"]')
    assert row.count() == 1, f"插件 {pid} 不在已安装列表"
    row.locator('[data-testid="plugin-enable"]').click()
    if first_run:
        page.wait_for_selector('[data-testid="plugin-confirm-dialog"]', timeout=5000)
        page.click('[data-testid="plugin-confirm-ok"]')
    deadline = time.time() + 15
    while time.time() < deadline:
        if row.locator('[data-testid="plugin-state"]').get_attribute("data-state") == "running":
            return
        page.wait_for_timeout(200)
    raise AssertionError(f"插件 {pid} 启用后未达 running 态")


def install_plugin(page, files: list[Path], pid_hint: str) -> None:
    """安装三连:选文件 → 校验并安装(plugin_validate 面)→ 写入注册表。"""
    page.set_input_files('[data-testid="plugin-files"]', [str(p) for p in files])
    page.click('[data-testid="plugin-install"]')
    page.wait_for_selector('[data-testid="plugin-candidate-card"]', timeout=8000)
    page.wait_for_selector('[data-testid="plugin-valid"]', timeout=15000)
    perms = page.locator('[data-testid="plugin-perm"]').count()
    assert perms >= 5, f"校验卡权限五面列示缺失(实得 {perms} 行)"
    page.click('[data-testid="plugin-install-confirm"]')
    deadline = time.time() + 8
    while time.time() < deadline:
        if page.locator(f'[data-testid="plugin-row"][data-pid="{pid_hint}"]').count() == 1:
            return
        page.wait_for_timeout(200)
    raise AssertionError(f"插件 {pid_hint} 安装后未进列表")


def plugin_files(example: str) -> list[Path]:
    base = REPO / "apps" / "web" / "examples" / "plugins" / example
    return [base / "manifest.json", base / "main.js"]


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
        print("FAIL: 未找到 cutforge-cli(插件服务端第二道拦截依赖 cli plugin-call)", file=sys.stderr)
        return 2
    if not shutil.which("ffmpeg"):
        print("FAIL: 本机无 ffmpeg(无法现场生成素材)", file=sys.stderr)
        return 2

    from playwright.sync_api import sync_playwright

    t0 = time.time()
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-ai-native-"))
    token = "ai-native-token"
    proj = tmp / "proj"
    r = sh([str(cli), "new", str(proj), "--slug", "ai-native", "--json",
            "--fps", "30", "--width", "1080", "--height", "1920", "--track", "video,audio"])
    assert r.returncode == 0, f"cli new 失败: {r.stdout} {r.stderr}"
    mat_rel = "01_原始素材" if (proj / "05_时间线工程").is_dir() else "01_materials"
    make_media(proj / mat_rel, "main.mp4", MAT_SECONDS)
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

            # ---------- 装配:UI 导入一段 + RPC 补一段(V1 两段,批量转场的前 提) ----------
            page.wait_for_selector('[data-testid="media-item"]', timeout=15000)
            rev0 = server_rev(serve.port, token, root)
            page.dblclick('[data-testid="media-item"]')
            rev1 = wait_server_rev(serve.port, token, root, rev0)
            clips = v1_clips_by_start(serve.port, token, root)
            assert len(clips) == 1 and clips[0]["startMs"] == 0, f"导入落点: {clips}"
            r = rpc(serve.port, token, "clip_add",
                    {"root": root, "trackId": "V1", "src": f"{mat_rel}/main.mp4",
                     "startMs": MAT_SECONDS * 1000, "durationMs": MAT_SECONDS * 1000,
                     "requestId": "ai-native-clip-2"})
            assert r["ok"], f"第二段建卡失败: {r}"
            rev_setup = wait_server_rev(serve.port, token, root, rev1)
            clips = v1_clips_by_start(serve.port, token, root)
            assert [c["startMs"] for c in clips] == [0, MAT_SECONDS * 1000], f"两段就位: {clips}"
            log(f"装配:V1 两段({clips[0]['id']} / {clips[1]['id']}),rev→{rev_setup}: PASS")

            # ---------- 场景 1:脚本页签三内置片段载入 → 运行 → 结构化输出(AC-7.3) ----------
            # 会话标记两枚(5s/12s,均在首段内;M 键会话级,不落盘)——标记是时间线
            # 会话态,先在时间线页签打好,再切脚本页签载入「按标记切割」
            for ms in (5000, 12000):
                page.click('[data-testid="ruler"]', position={"x": int(ms * PX_PER_MS), "y": 10})
                page.wait_for_function(
                    "ms => Math.abs(Number(document.querySelector('[data-testid=\"playhead-ms\"]')"
                    ".textContent) - ms) <= 34", arg=ms, timeout=5000)
                page.keyboard.press("m")
                wait_toast(page, "已加标记")
            page.click('[data-testid="tab-script"]')
            page.wait_for_selector('[data-testid="script-editor"]', timeout=5000)
            rev_before_scene1 = server_rev(serve.port, token, root)
            load_and_run_builtin(page, 0, expect_steps=2, tool="clip_split")   # 按标记切割
            load_and_run_builtin(page, 1, expect_steps=2, tool="clip_update")  # 批量变色
            load_and_run_builtin(page, 2, expect_steps=1, tool="transition_set")  # 批量转场
            assert server_rev(serve.port, token, root) == rev_before_scene1, \
                "场景 1:三次预演(副本 dry-run)后真工程 rev 必须不变"
            assert not scratch_residue(proj), f"预演后副本残留: {scratch_residue(proj)}"
            log("场景 1:三内置片段载入→预演→结构化输出(2/2/1 步),真工程零写入: PASS")

            # ---------- 场景 2:计划批准流(AC-7.5)+ P0 文件级断言 ----------
            page.select_option('[data-testid="script-lib-select"]', index=1)  # 批量变色
            page.click('[data-testid="script-lib-load"]')
            page.click('[data-testid="script-submit"]')
            wait_toast(page, "已送批准流(2 项)")
            page.click('[data-testid="tab-diff"]')
            page.wait_for_selector('[data-testid="plan-draft-head"]', timeout=5000)
            assert "脚本" in page.inner_text('[data-testid="plan-draft-head"]'), "草稿头应标来源"
            # P0 基准:预演前快照(rev + oplog 逐文件字节)
            rev_pre = server_rev(serve.port, token, root)
            oplog_pre = oplog_file_bytes(proj)
            assert oplog_pre, "预演前真工程必须有 oplog 文件(P0 断言前提)"
            page.click('[data-testid="plan-preview"]')
            page.wait_for_selector('[data-testid="plan-preview-summary"]', timeout=30000)
            summary_txt = page.inner_text('[data-testid="plan-preview-summary"]')
            assert "2 项,ok 2 / 错 0" in summary_txt, f"预演汇总: {summary_txt}"
            cards = page.locator('[data-testid="plan-item"]')
            assert cards.count() == 2, f"预演卡片应 2 张,实得 {cards.count()}"
            assert "预演 ok" in cards.nth(0).inner_text(), "首卡应标预演 ok"
            assert page.locator('[data-testid="plan-change"]').count() >= 2, "应有字段级 before/after 行"
            # P0 全 rev 断言(文件级):预演后真工程 rev 不变 + oplog 字节逐一致 + 零残留
            assert server_rev(serve.port, token, root) == rev_pre, \
                f"P0:预演后真工程 rev 被推高({rev_pre} → {server_rev(serve.port, token, root)})"
            assert oplog_file_bytes(proj) == oplog_pre, "P0:预演后真工程 oplog 字节漂移(硬链接穿透)"
            assert not scratch_residue(proj), f"P0:预演后副本残留: {scratch_residue(proj)}"
            log(f"P0 文件级断言:预演后 rev 保持 {rev_pre} + oplog {len(oplog_pre)} 文件字节逐一致"
                f" + 零 .cf-scratch 残留: PASS")
            # 逐项批准/拒绝(首项批准 = 首段,次项拒绝 = 次段)
            cards.nth(0).locator('[data-testid="plan-item-approve"]').click()
            cards.nth(1).locator('[data-testid="plan-item-reject"]').click()
            assert "已批准" in cards.nth(0).locator('[data-testid="plan-item-decision"]').inner_text()
            assert "已拒绝" in cards.nth(1).locator('[data-testid="plan-item-decision"]').inner_text()
            assert "批准 1 · 拒绝 1" in page.inner_text('[data-testid="plan-approval-count"]')
            page.click('[data-testid="plan-apply"]')
            page.wait_for_selector('[data-testid="plan-apply-result"]', timeout=30000)
            result_txt = page.inner_text('[data-testid="plan-apply-result"]')
            assert "落地 1" in result_txt and "拒绝 1" in result_txt and "跳过 0" in result_txt, \
                f"apply 回执: {result_txt}"
            plan_id = page.inner_text('[data-testid="plan-apply-planid"]').strip()
            assert plan_id.startswith("plan-web-"), f"planId: {plan_id}"
            # 服务端对账:rev 恰 +1;被拒项真相源零落地;causedBy=planId 恰一 Op(actor=user)
            rev_applied = wait_server_rev(serve.port, token, root, rev_pre)
            assert rev_applied == rev_pre + 1, f"apply 后 rev 应恰 +1:{rev_pre} → {rev_applied}"
            clips = v1_clips_by_start(serve.port, token, root)
            assert (clips[0].get("grade") or {}).get("temperature") == 20, f"批准项未落地: {clips[0]}"
            assert not clips[1].get("grade"), f"被拒项竟落地(真相源污染): {clips[1]}"
            ops = rpc(serve.port, token, "oplog_tail", {"root": root, "limit": 50})["data"]["ops"]
            chained = [o for o in ops
                       if plan_id in (o.get("caused_by") or o.get("causedBy") or [])]
            assert len(chained) == 1, f"causedBy=planId 应恰一 Op: {[o['op_id'] for o in chained]}"
            assert chained[0]["actor"]["kind"] == "user", f"UI 通道归因: {chained[0]['actor']}"
            # UI 对账面:plan-result-refresh → OpLog 行内 causedBy 显式成行
            page.click('[data-testid="plan-result-refresh"]')
            page.wait_for_selector('[data-testid="diff-causedby"]', timeout=8000)
            page.wait_for_function(
                "pid => document.querySelector('[data-testid=\"diff-plan-id\"]') && "
                "document.querySelector('[data-testid=\"diff-plan-id\"]').textContent === pid",
                arg=plan_id, timeout=8000)
            log(f"场景 2:批准流逐项裁决→apply(落地 1/拒绝 1),rev→{rev_applied},"
                f"causedBy={plan_id} 对账,被拒项零落地: PASS")

            # ---------- 场景 3:插件九步(AC-7.2) ----------
            page.click('[data-testid="tab-plugins"]')
            page.wait_for_selector('[data-testid="plugin-files"]', timeout=5000)
            # ① 安装 ×3(demo-command 命令 / demo-menu 菜单 / demo-panel 面板)
            for ex, pid in (("demo-command", "demo-command"), ("demo-menu", "demo-menu"),
                            ("demo-panel", "demo-panel")):
                install_plugin(page, plugin_files(ex), pid)
            assert page.locator('[data-testid="plugin-row"]').count() == 3, "三插件应全部入列"
            log("① 安装 ×3 + ② 校验(plugin_valid 徽标 + 权限五面列示): PASS")
            # ③ 权限确认(首启对话框;三插件各一次)
            for pid in ("demo-command", "demo-menu", "demo-panel"):
                enable_plugin(page, pid, first_run=True)
            wait_toast(page, "插件已启用")
            log("③ 权限确认(首启对话框 plugin-confirm-ok ×3,启用达 running): PASS")
            # ④ 三贡献点生效:命令/菜单(右键项 ×2 + 调用回执 toast)+ 面板(专属页签受控渲染)
            page.click('[data-testid="tab-timeline"]')
            page.click('[data-testid="clip"]', button="right")
            page.wait_for_selector('[data-testid="context-menu"]', timeout=5000)
            menu_txt = page.inner_text('[data-testid="context-menu"]')
            assert "插件:统计片段数(示例·统计命令)" in menu_txt, f"命令贡献点未进右键菜单: {menu_txt}"
            assert "插件:片段信息卡(示例·片段信息菜单)" in menu_txt, f"菜单贡献点未进右键菜单: {menu_txt}"
            page.click('[data-testid="context-menu"] button:has-text("统计片段数")')
            wait_toast(page, "时间线共 2 个片段")
            page.click('[data-testid="clip"]', button="right")
            page.wait_for_selector('[data-testid="context-menu"]', timeout=5000)
            page.click('[data-testid="context-menu"] button:has-text("片段信息卡")')
            wait_toast(page, f"片段 {clips[0]['id']}")
            page.click('[data-testid="tab-plugin-demo-panel-stats"]', timeout=5000)
            page.wait_for_selector('[data-testid="plugin-panel-html-demo-panel-stats"]', timeout=15000)
            deadline = time.time() + 15
            panel_txt = ""
            while time.time() < deadline:
                panel_txt = page.inner_text('[data-testid="plugin-panel-html-demo-panel-stats"]')
                if "时间线统计" in panel_txt and "片段 2 个" in panel_txt:
                    break
                page.wait_for_timeout(500)
            else:
                raise AssertionError(f"面板贡献点未推送内容: {panel_txt!r}")
            log("④ 三贡献点生效(右键菜单 ×2 调用回执 + 面板页签受控渲染): PASS")
            # ⑤ 越权双拦截:宿主镜像(FE)+ 服务端 plugin-call 通道(BE),工程零变化
            page.click('[data-testid="tab-plugins"]')  # ④ 末停在插件面板页签,切回管理面
            page.wait_for_selector('[data-testid="plugin-files"]', timeout=5000)
            naughty = tmp / "naughty"
            naughty.mkdir()
            (naughty / "manifest.json").write_text(json.dumps({
                "id": "naughty-plugin", "name": "越权测试插件", "version": "1.0.0",
                "form": "worker", "entry": "main.js", "description": "只读权限却调写工具(双拦截负例)",
                "permissions": {"read": True, "write": False, "network": False,
                                "filesystem": [], "exec": False},
            }, ensure_ascii=False), encoding="utf-8")
            (naughty / "main.js").write_text(
                '"use strict";\n'
                "(async () => {\n"
                "  try {\n"
                '    await cutforge.call("clip_update", { clipId: "V1-001", patch: { volume: 0.1 } });\n'
                '    cutforge.toast("越权调用竟然成功(宿主拦截失效)", false);\n'
                "  } catch (e) {\n"
                '    cutforge.toast("越权被拦:" + ((e && e.envelope && e.envelope.code) || "") + " " + ((e && e.message) || e));\n'
                "  }\n"
                "  cutforge.ready();\n"
                "})();\n", encoding="utf-8")
            rev_pre_forbidden = server_rev(serve.port, token, root)
            install_plugin(page, [naughty / "manifest.json", naughty / "main.js"], "naughty-plugin")
            enable_plugin(page, "naughty-plugin", first_run=True)
            blocked = wait_toast(page, "越权被拦")
            assert "FORBIDDEN" in blocked and "GUARD_FAILED" in blocked, f"宿主拦截回执: {blocked}"
            assert server_rev(serve.port, token, root) == rev_pre_forbidden, "越权调用不得写工程"
            # 第二道:服务端 plugin-call 通道(CLI;同 manifest 同工具 → GUARD_FAILED/FORBIDDEN)
            ro_manifest = tmp / "ro-plugin.json"
            ro_manifest.write_text(json.dumps({
                "id": "ro-plugin", "name": "只读插件", "version": "1.0.0",
                "form": "process", "entry": "ro.py", "permissions": {"read": True},
            }, ensure_ascii=False), encoding="utf-8")
            r = sh([str(cli), "plugin-call", str(ro_manifest), "clip_update", "--json",
                    "--args-json", json.dumps({"root": root, "clipId": clips[0]["id"],
                                               "patch": {"volume": 0.1}})])
            assert r.returncode == 2, f"服务端越权必须退出码 2: {r.returncode} {r.stdout[-300:]}"
            env = json.loads(r.stdout.strip().splitlines()[-1])
            assert env["code"] == "GUARD_FAILED" and "FORBIDDEN" in env["message"], f"服务端拦截: {env}"
            assert server_rev(serve.port, token, root) == rev_pre_forbidden, "服务端通道越权不得写工程"
            log("⑤ 越权双拦截(宿主镜像 + 服务端 plugin-call 均 GUARD_FAILED/FORBIDDEN,"
                f"rev 保持 {rev_pre_forbidden}): PASS")
            # ⑥ 崩溃隔离:Worker 抛错 → 终止 + 自动禁用
            boom = tmp / "boom"
            boom.mkdir()
            (boom / "manifest.json").write_text(json.dumps({
                "id": "boom-plugin", "name": "崩溃测试插件", "version": "1.0.0",
                "form": "worker", "entry": "main.js", "description": "载入即抛错(崩溃隔离负例)",
                "permissions": {"read": True, "write": False, "network": False,
                                "filesystem": [], "exec": False},
            }, ensure_ascii=False), encoding="utf-8")
            (boom / "main.js").write_text(
                '"use strict";\nthrow new Error("boom: 崩溃隔离测试");\n', encoding="utf-8")
            install_plugin(page, [boom / "manifest.json", boom / "main.js"], "boom-plugin")
            boom_row = page.locator('[data-testid="plugin-row"][data-pid="boom-plugin"]')
            boom_row.locator('[data-testid="plugin-enable"]').click()
            page.wait_for_selector('[data-testid="plugin-confirm-dialog"]', timeout=5000)
            page.click('[data-testid="plugin-confirm-ok"]')
            # 崩溃路径:不等 running(必然到不了)——等崩溃 toast;徽标经 plugin-refresh
            # 观察刷新后断言(crashPlugin 内 stopPlugin 触发的重渲先于 setEnabled 落账,
            # 壳侧徽标存在一次刷新滞后——如实登记,语义面以注册表禁用为准)
            wait_toast(page, "已崩溃并被自动禁用")
            page.click('[data-testid="plugin-refresh"]')
            deadline = time.time() + 8
            state = ""
            while time.time() < deadline:
                state = boom_row.locator('[data-testid="plugin-state"]').get_attribute("data-state")
                if state == "disabled":
                    break
                page.wait_for_timeout(200)
            assert state == "disabled", f"崩溃插件必须被自动禁用(实得 {state})"
            log("⑥ 崩溃隔离(Worker 抛错 → 自动禁用 + toast,不拖垮宿主): PASS")
            # ⑦ 禁用:demo-command 停用 → 右键菜单贡献点即时摘除
            row = page.locator('[data-testid="plugin-row"][data-pid="demo-command"]')
            row.locator('[data-testid="plugin-disable"]').click()
            deadline = time.time() + 8
            while time.time() < deadline:
                if row.locator('[data-testid="plugin-state"]').get_attribute("data-state") == "disabled":
                    break
                page.wait_for_timeout(200)
            else:
                raise AssertionError("demo-command 停用未达 disabled 态")
            page.click('[data-testid="tab-timeline"]')
            page.click('[data-testid="clip"]', button="right")
            page.wait_for_selector('[data-testid="context-menu"]', timeout=5000)
            assert "统计片段数" not in page.inner_text('[data-testid="context-menu"]'), \
                "停用后命令贡献点必须从右键菜单摘除"
            page.keyboard.press("Escape")
            log("⑦ 禁用(demo-command → disabled,右键菜单贡献点即时摘除): PASS")
            # ⑧⑨ 卸载:demo-menu 移出注册表(行消失;启停生命周期与注册表面收口)
            page.click('[data-testid="tab-plugins"]')
            page.locator('[data-testid="plugin-row"][data-pid="demo-menu"]') \
                .locator('[data-testid="plugin-uninstall"]').click()
            deadline = time.time() + 8
            while time.time() < deadline:
                if page.locator('[data-testid="plugin-row"][data-pid="demo-menu"]').count() == 0:
                    break
                page.wait_for_timeout(200)
            else:
                raise AssertionError("demo-menu 卸载后仍在列表")
            assert server_rev(serve.port, token, root) == rev_pre_forbidden, \
                "插件生命周期全程(安装/启停/崩溃/卸载)不得写工程"
            log("⑧⑨ 卸载(demo-menu 注册表移除;插件面全程零工程写入): PASS")

            # ---------- 场景 4:标注线程两轮 + session_report 渲染 ----------
            page.click('[data-testid="tab-notes"]')
            page.fill('[data-testid="note-body"]', "ai-native:这段语速偏快")
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
            # 第一轮:UI 线程回复(note_reply,不改 state);行重渲竞态重试(ui_smoke 同口径)
            for _ in range(4):
                row = page.locator('[data-testid="note-row"]').filter(has_text=nid).first
                row.locator('[data-testid="note-thread-body"]').fill("第一轮:人问,能再快 5% 吗")
                row.locator('[data-testid="note-thread-send"]').click()
                try:
                    wait_toast(page, "已追加回复", timeout_s=4)
                    break
                except AssertionError:
                    continue
            else:
                raise AssertionError("线程第一轮回复未送达")
            # 第二轮:RPC agent 回复(线程双轮;author 徽标 AI)
            r = rpc(serve.port, token, "note_reply",
                    {"root": root, "noteId": nid, "body": "第二轮:AI 答,已按 5% 提速", "author": "agent"})
            assert r["ok"] and r["data"]["replies"] == 2, f"线程两轮回执: {r}"
            # 面板线程块随 notes.changed 事件自动 refresh;等两轮上帧
            deadline = time.time() + 8
            thread_items = 0
            while time.time() < deadline:
                row = page.locator('[data-testid="note-row"]').filter(has_text=nid).first
                thread_items = row.locator('[data-testid="note-thread-item"]').count()
                if thread_items >= 2:
                    break
                page.wait_for_timeout(300)
            assert thread_items >= 2, f"线程块应呈现两轮,实得 {thread_items}"
            note = next(n for n in rpc(serve.port, token, "notes_list", {"root": root})["data"]["notes"]
                        if n["id"] == nid)
            assert [t["author"] for t in note["thread"]] == ["user", "agent"], f"两轮作者序: {note['thread']}"
            assert note["state"] == "open", "线程回复不得改结案态"
            log(f"场景 4a:标注线程两轮(user+agent,线程 {nid} 两项,state 仍 open): PASS")
            # session_report 渲染(人话 Markdown;下载解锁)
            page.wait_for_selector('[data-testid="report-run"]', timeout=5000)
            page.click('[data-testid="report-run"]')
            page.wait_for_selector('[data-testid="report-markdown"]', timeout=15000)
            md_txt = page.inner_text('[data-testid="report-markdown"]')
            assert "会话改动报告" in md_txt and "改动段" in md_txt, f"报告渲染缺关键段: {md_txt[:200]}"
            assert page.locator('[data-testid="report-download"]').is_enabled(), "报告生成后下载应解锁"
            rep = rpc(serve.port, token, "session_report", {"root": root, "sinceRev": 0})
            assert rep["ok"] and rep["data"]["opCount"] > 0, f"session_report: {rep}"
            assert rep["data"]["notes"]["replies"] >= 2, f"报告线程计数: {rep['data']['notes']}"
            log(f"场景 4b:session_report 渲染(rev→{rep['data']['revTo']},"
                f"{rep['data']['opCount']} Op,人话 Markdown 上帧): PASS")

            assert not pageerrors, f"页面 JS 异常: {pageerrors[:5]}"
            browser.close()

        print(f"e2e_ai_native: 全部 PASS(耗时 {time.time() - t0:.1f}s)")
        return 0
    finally:
        serve.stop()
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except AssertionError as e:
        print(f"FAIL: {e}", file=sys.stderr)
        sys.exit(2)
