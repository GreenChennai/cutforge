#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册三 AC-3.2 微交互录屏存档工具(docs/design/record-captures.py;可重录,非门禁)。

    python docs/design/record-captures.py [--bin cutforge-mcp] [--cli cutforge-cli]
                                          [--out docs/design/recordings] [--only 02-drag]

用 Playwright 逐条录制 docs/design/micro-interactions.md 清单的关键微交互,
产出 docs/design/recordings/*.webm(命名对齐清单项;单文件 ≤2MB,超限降分辨率/时长)。
每条一个独立 browser context(record_video_dir),驱动完成后 close 落盘再重命名。
体例与 tools/e2e_*.py 一致:pathlib / 二进制定位无 .exe 回退 / playwright+stdlib(+ ffmpeg 夹具)。
录屏仅供人审与文档回链,断言以 e2e 为准(本脚本不作为门禁)。
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

REPO = Path(__file__).resolve().parents[2]
PX_PER_MS = 0.06
FPS = 30
MEDIA_S = 30
VIEWPORT = {"width": 960, "height": 600}
MAX_BYTES = 2 * 1024 * 1024

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[record] {msg}", flush=True)


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


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
    assert r.returncode == 0, f"ffmpeg 夹具失败:{r.stderr[-200:]}"


def write_base_project(proj: Path, tl_rel: str, mat_rel: str) -> None:
    """基准盘面:V1 两段(a@0 / b@30s)+ 空音轨 A1(非法落点演示用)。"""
    pj = {"version": 1, "schemaVersion": "2.0.0", "slug": "record-caps", "fps": FPS,
          "canvas": {"width": 1080, "height": 1920},
          "tracks": [
              {"id": "V1", "kind": "video", "name": "V1",
               "clips": [{"id": "V1-001", "src": f"{mat_rel}/a.mp4", "startMs": 0, "durationMs": 30000},
                         {"id": "V1-002", "src": f"{mat_rel}/b.mp4", "startMs": 30000, "durationMs": 30000}]},
              {"id": "A1", "kind": "audio", "name": "A1", "clips": []}]}
    (proj / tl_rel / "project.json").write_text(json.dumps(pj, ensure_ascii=False), encoding="utf-8")


def clip_center(page, clip_id: str, start_ms: int):
    """滚动到片段附近再取中心(窄视口下远端片段被虚拟化,须先入窗)。"""
    page.evaluate(
        """(x) => { const w = document.querySelector('[data-testid="timeline-wrap"]');
                    w.scrollLeft = Math.max(0, x - 260); }""", start_ms * PX_PER_MS)
    page.wait_for_timeout(350)
    box = page.locator(f'[data-testid="clip"][data-id="{clip_id}"]').bounding_box()
    assert box, f"片段 {clip_id} 无布局盒"
    return box["x"] + box["width"] / 2, box["y"] + box["height"] / 2


def reset_state(page) -> None:
    """每条录制开头:滚回原点、收引导条(统一画面基线)。"""
    page.evaluate(
        "() => { const w = document.querySelector('[data-testid=\"timeline-wrap\"]');"
        "        if (w) w.scrollLeft = 0; }")
    bar = page.locator('[data-testid="onboard-dismiss"]')
    if bar.count():
        bar.first.click()
    page.wait_for_timeout(150)


# ---------------- 各条录制剧本(对应 micro-interactions.md 清单项) ----------------

def cap_01_onboarding(page):
    """首启引导条(T3.7;清单二·新手路径):显示 → 「知道了」消失。"""
    page.wait_for_selector('[data-testid="onboard-bar"]', timeout=10000)
    page.wait_for_timeout(1600)
    page.click('[data-testid="onboard-dismiss"]')
    page.wait_for_timeout(700)


def ruler_click_at(page, ms: float, scroll_left: float) -> None:
    """原生 mouse 点击标尺(窄视口 + 滚动偏移下,locator 点击会被 wrap 判定拦截)。"""
    wrap = page.locator('[data-testid="timeline-wrap"]').bounding_box()
    x = wrap["x"] + (ms * PX_PER_MS - scroll_left)
    page.mouse.click(x, wrap["y"] + 8)


def cap_02_drag_ghost_snap(page):
    """拖拽 ghost 跟手 + 吸附脉冲 + 时间码气泡(清单 1/2/4/5)→ 松手提交;尾段 Esc 取消(清单 7)。"""
    reset_state(page)
    sl = max(0.0, 30000 * PX_PER_MS - 260)
    page.evaluate(
        """(x) => { const w = document.querySelector('[data-testid="timeline-wrap"]');
                    w.scrollLeft = x; }""", sl)
    page.wait_for_timeout(300)
    ruler_click_at(page, 31000, sl)
    page.wait_for_timeout(500)
    cx, cy = clip_center(page, "V1-002", 30000)
    page.mouse.move(cx, cy)
    page.mouse.down()
    for i in range(1, 16):
        page.mouse.move(cx + i * 14, cy)
        page.wait_for_timeout(45)
    page.mouse.up()
    page.wait_for_timeout(900)
    # 边缘卷入(清单 14):拖到视口右缘 60px 带内,时间线自动滚动
    cx, cy = clip_center(page, "V1-002", 33500)
    page.mouse.move(cx, cy)
    page.mouse.down()
    for i in range(1, 10):
        page.mouse.move(cx + 300 + i * 12, cy)
        page.wait_for_timeout(60)
    page.keyboard.press("Escape")
    page.wait_for_timeout(500)
    # Esc 取消(清单 7):拖拽中 Esc → 零 Op、ghost 消失、几何复位
    cx, cy = clip_center(page, "V1-002", 33500)
    page.mouse.move(cx, cy)
    page.mouse.down()
    for i in range(1, 8):
        page.mouse.move(cx - i * 16, cy)
        page.wait_for_timeout(45)
    page.keyboard.press("Escape")
    page.wait_for_timeout(700)


def cap_03_drop_invalid_red(page):
    """非法落点红态 + 目标轨高亮(清单 3):video 拖到 A1(红)→ 拖回(正常)→ Esc。"""
    reset_state(page)
    cx, cy = clip_center(page, "V1-002", 51000)
    page.mouse.move(cx, cy)
    page.mouse.down()
    page.mouse.move(cx + 30, cy)
    lane = page.locator('[data-testid="track-lane-A1"]').bounding_box()
    page.mouse.move(lane["x"] + 300, lane["y"] + lane["height"] / 2, steps=12)
    page.wait_for_timeout(900)
    page.mouse.move(cx + 60, cy, steps=12)
    page.wait_for_timeout(700)
    page.keyboard.press("Escape")
    page.wait_for_timeout(600)


def cap_04_trim_bubble(page):
    """trim 实时时长气泡(清单 5/6):右缘内拖(时长气泡)→ Esc 复位;左缘同样一瞥。"""
    reset_state(page)
    loc = page.locator('[data-testid="clip"][data-id="V1-001"]')
    loc.hover()
    page.wait_for_timeout(350)
    edge = loc.locator(".edge-r").bounding_box()
    page.mouse.move(edge["x"] + edge["width"] / 2, edge["y"] + edge["height"] / 2)
    page.mouse.down()
    for i in range(1, 9):
        page.mouse.move(edge["x"] - i * 12, edge["y"] + 8)
        page.wait_for_timeout(55)
    page.keyboard.press("Escape")
    page.wait_for_timeout(500)
    edge_l = loc.locator(".edge-l").bounding_box()
    page.mouse.move(edge_l["x"] + edge_l["width"] / 2, edge_l["y"] + 8)
    page.mouse.down()
    for i in range(1, 7):
        page.mouse.move(edge_l["x"] + i * 10, edge_l["y"] + 8)
        page.wait_for_timeout(55)
    page.keyboard.press("Escape")
    page.wait_for_timeout(600)


def cap_05_clip_hover_select(page):
    """clip hover 边缘把手浮现 + 选中光晕(清单 8/9)。"""
    reset_state(page)
    loc = page.locator('[data-testid="clip"][data-id="V1-001"]')
    box = loc.bounding_box()
    page.mouse.move(box["x"] + 420, box["y"] + box["height"] / 2)
    page.wait_for_timeout(700)
    page.mouse.click(box["x"] + 420, box["y"] + box["height"] / 2)
    page.wait_for_timeout(900)


def cap_06_playhead_seek_ruler(page):
    """播放头离散 seek 平滑 + 标尺悬停气泡 + 按下即跳 scrub(清单 10/11/12)。"""
    reset_state(page)
    page.mouse.move(int(12000 * PX_PER_MS) + 8, 8)
    page.wait_for_timeout(900)
    page.mouse.down()
    for ms in (14000, 17000, 21000, 26000):
        page.mouse.move(int(ms * PX_PER_MS) + 8, 8, steps=6)
        page.wait_for_timeout(130)
    page.mouse.up()
    page.wait_for_timeout(700)


def cap_07_marquee_multiselect(page):
    """框选多选(清单 13):A1 空白拉框越过 V1 片段 → 高亮;Shift 复选再框。"""
    reset_state(page)
    lane = page.locator('[data-testid="track-lane-A1"]').bounding_box()
    page.mouse.move(lane["x"] + 40, lane["y"] + lane["height"] / 2)
    page.mouse.down()
    page.mouse.move(lane["x"] + 900, lane["y"] - 40, steps=14)
    page.wait_for_timeout(600)
    page.mouse.up()
    page.wait_for_timeout(700)
    page.mouse.move(lane["x"] + 940, lane["y"] + lane["height"] / 2, steps=4)
    page.keyboard.down("Shift")
    page.mouse.down()
    page.mouse.move(lane["x"] + 60, lane["y"] - 40, steps=10)
    page.wait_for_timeout(400)
    page.mouse.up()
    page.keyboard.up("Shift")
    page.wait_for_timeout(700)


def cap_08_zoom_wheel_fit(page):
    """Ctrl+滚轮缩放(视口中心锚定)+ \\ 适应窗口(清单 15)。"""
    reset_state(page)
    page.locator('[data-testid="timeline-wrap"]').hover()
    for _ in range(4):
        page.keyboard.down("Control")
        page.mouse.wheel(0, -240)
        page.keyboard.up("Control")
        page.wait_for_timeout(260)
    page.wait_for_timeout(400)
    page.evaluate("() => { const a = document.activeElement; if (a && a.blur) a.blur(); }")
    page.keyboard.press("\\")
    page.wait_for_timeout(800)


def cap_09_tab_crossfade(page):
    """页签切换入场交叉淡入(清单 16)。"""
    reset_state(page)
    for tab in ("tab-notes", "tab-diff", "tab-conflicts", "tab-timeline"):
        page.click(f'[data-testid="{tab}"]')
        page.wait_for_timeout(650)


def cap_10_inspector_group(page):
    """检查器分组展开/折叠 160ms(清单 17)。"""
    reset_state(page)
    page.locator('[data-testid="clip"][data-id="V1-001"]').click(position={"x": 300, "y": 14})
    page.wait_for_timeout(500)
    groups = page.locator('[data-testid^="insp-group-"] legend, [data-testid^="insp-group-"] .insp-legend')
    n = groups.count()
    if n:
        for i in range(min(2, n)):
            groups.nth(i).click()
            page.wait_for_timeout(450)
        for i in range(min(2, n)):
            groups.nth(i).click()
            page.wait_for_timeout(450)
    page.wait_for_timeout(400)


def cap_11_media_hover_wave(page):
    """素材卡 hover 提亮 + 插入后波形入场淡入(清单 18/19)。"""
    reset_state(page)
    item = page.locator('[data-testid="media-item"]').first
    item.hover()
    page.wait_for_timeout(800)
    item.dblclick()
    page.wait_for_timeout(1400)


def cap_12_button_tooltip(page):
    """按钮 hover/press 双态 + 工具提示浮现(清单 20/25;data-tip 载体:导出剪映草稿按钮)。"""
    reset_state(page)
    btn = page.locator('[data-testid="export-jianying"]')
    btn.hover()
    page.wait_for_timeout(1100)
    page.mouse.down()
    page.wait_for_timeout(200)
    page.mouse.up()
    page.wait_for_timeout(700)
    page.keyboard.press("Escape")
    page.wait_for_timeout(500)


def cap_13_dialog_zoom(page):
    """模态缩放入场 + 遮罩渐显(清单 22):? 帮助面板 → Esc;Ctrl+, 设置 → Esc。"""
    reset_state(page)
    page.evaluate("() => { const a = document.activeElement; if (a && a.blur) a.blur(); }")
    page.keyboard.press("?")
    page.wait_for_selector('[data-testid="help-dialog"]', timeout=4000)
    page.wait_for_timeout(1100)
    page.keyboard.press("Escape")
    page.wait_for_timeout(500)
    page.keyboard.press("Control+Comma")
    page.wait_for_timeout(1100)
    page.keyboard.press("Escape")
    page.wait_for_timeout(600)


def cap_14_context_menu(page):
    """右键菜单入场 + hover 态(清单 24;含分割按键提示)。"""
    reset_state(page)
    page.locator('[data-testid="clip"][data-id="V1-001"]').click(position={"x": 300, "y": 14})
    page.wait_for_timeout(300)
    page.locator('[data-testid="clip"][data-id="V1-001"]').click(button="right", position={"x": 300, "y": 14})
    page.wait_for_selector('[data-testid="context-menu"]', timeout=4000)
    page.wait_for_timeout(700)
    items = page.locator('[data-testid="context-menu"] button')
    if items.count() > 1:
        items.nth(1).hover()
        page.wait_for_timeout(600)
    page.keyboard.press("Escape")
    page.wait_for_timeout(600)


def cap_15_toast_undo(page):
    """toast 滑入 + 自动淡出 + 撤销按钮(清单 21;T3.7 action toast)。"""
    reset_state(page)
    page.locator('[data-testid="clip"][data-id="V1-001"]').click(position={"x": 300, "y": 14})
    page.wait_for_timeout(300)
    page.evaluate("() => { const a = document.activeElement; if (a && a.blur) a.blur(); }")
    page.keyboard.press("Delete")
    page.wait_for_timeout(1500)
    act = page.locator('[data-testid="toast-action"]')
    if act.count():
        act.first.click(force=True)  # toast 带滑入动画,跳过 Playwright 稳定态等待(5s TTL 竞速)
    page.wait_for_timeout(1000)


def cap_16_export_progress(page):
    """导出进度条平滑(清单 23;不确定进度循环豁免登记)。"""
    reset_state(page)
    page.click('[data-testid="export-run"]')
    page.wait_for_timeout(5200)
    deadline = time.time() + 120
    while time.time() < deadline:
        if "完成" in page.inner_text('[data-testid="export-progress"] .exp-text'):
            break
        page.wait_for_timeout(300)
    page.wait_for_timeout(900)


def cap_17_rev_dot_pulse(page):
    """保存中→已保存脉冲点(清单 26):检查器应用一笔 → #rev-dot 脉冲。"""
    reset_state(page)
    page.locator('[data-testid="clip"][data-id="V1-001"]').click(position={"x": 300, "y": 14})
    page.wait_for_timeout(500)
    page.fill('[data-testid="field-opacity"]', "0.92")
    page.click('[data-testid="insp-apply"]')
    page.wait_for_timeout(1800)


def cap_18_conn_banner(page):
    """断连横幅滑入 + 呼吸点 + 恢复收起(清单 27):拦截 /rpc → ⟳ → 横幅;放行 → 收起。"""
    reset_state(page)
    page.route("**/rpc", lambda route: route.abort())
    page.click('[data-testid="media-refresh"]')
    page.wait_for_selector('[data-testid="banner-conn"]:not([hidden])', timeout=15000)
    page.wait_for_timeout(2200)
    page.unroute("**/rpc")
    page.click('[data-testid="media-refresh"]')
    page.wait_for_timeout(2500)


def cap_19_perf_panel(page):
    """性能面板(Shift+D;预算表逐行可视)。"""
    reset_state(page)
    page.evaluate("() => { const a = document.activeElement; if (a && a.blur) a.blur(); }")
    page.keyboard.press("Shift+D")
    page.wait_for_timeout(2600)
    page.keyboard.press("Shift+D")
    page.wait_for_timeout(600)


CAPTURES = [
    ("01-first-run-onboarding.webm", cap_01_onboarding,
     "新手引导条(T3.7)"),
    ("02-drag-ghost-snap.webm", cap_02_drag_ghost_snap,
     "清单 1/2/4/5/7:ghost 跟手·落点阴影·吸附脉冲·时间码气泡·Esc 取消"),
    ("03-drop-invalid-red.webm", cap_03_drop_invalid_red,
     "清单 3:目标轨高亮/非法落点红态"),
    ("04-trim-bubble.webm", cap_04_trim_bubble,
     "清单 5/6:trim 实时时长气泡·相邻碰撞约束"),
    ("05-clip-hover-select.webm", cap_05_clip_hover_select,
     "清单 8/9:边缘把手浮现·选中光晕"),
    ("06-playhead-seek-ruler.webm", cap_06_playhead_seek_ruler,
     "清单 10/11/12:seek 平滑·标尺悬停气泡·按下即跳 scrub"),
    ("07-marquee-multiselect.webm", cap_07_marquee_multiselect,
     "清单 13:框选多选(Shift 加选)"),
    ("08-zoom-wheel-fit.webm", cap_08_zoom_wheel_fit,
     "清单 15:Ctrl+滚轮缩放·\\ 适应窗口"),
    ("09-tab-crossfade.webm", cap_09_tab_crossfade,
     "清单 16:页签切换交叉淡入"),
    ("10-inspector-group.webm", cap_10_inspector_group,
     "清单 17:检查器分组展开/折叠"),
    ("11-media-hover-wave.webm", cap_11_media_hover_wave,
     "清单 18/19:素材卡 hover 提亮·波形入场淡入"),
    ("12-button-tooltip.webm", cap_12_button_tooltip,
     "清单 20/25:按钮 hover/press 双态·工具提示浮现"),
    ("13-dialog-zoom.webm", cap_13_dialog_zoom,
     "清单 22:模态缩放入场·遮罩渐显"),
    ("14-context-menu.webm", cap_14_context_menu,
     "清单 24:右键菜单入场+hover 态"),
    ("15-toast-undo.webm", cap_15_toast_undo,
     "清单 21:toast 滑入·撤销按钮"),
    ("16-export-progress.webm", cap_16_export_progress,
     "清单 23:导出进度条平滑(不确定进度循环)"),
    ("17-rev-dot-pulse.webm", cap_17_rev_dot_pulse,
     "清单 26:rev 翻牌脉冲点"),
    ("18-conn-banner.webm", cap_18_conn_banner,
     "清单 27:断连横幅滑入·呼吸点·恢复收起"),
    ("19-perf-panel.webm", cap_19_perf_panel,
     "性能面板(T3.5;预算表逐行可视)"),
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=None)
    ap.add_argument("--cli", default=None)
    ap.add_argument("--out", default=str(REPO / "docs" / "design" / "recordings"))
    ap.add_argument("--only", default=None, help="只录文件名含该子串的条目")
    args = ap.parse_args()
    mcp = locate_bin(args.bin, ("cutforge-mcp",))
    cli = locate_bin(args.cli, ("cutforge-cli",))
    if not mcp or not cli:
        print("FAIL: 先 cargo build(cutforge-mcp + cutforge-cli)", file=sys.stderr)
        return 2
    if not shutil.which("ffmpeg"):
        print("FAIL: 本机无 ffmpeg", file=sys.stderr)
        return 2

    from playwright.sync_api import sync_playwright

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-record-"))
    token = "record-token"
    proj = tmp / "record-caps"
    r = sh([str(cli), "new", str(proj), "--slug", "record-caps", "--json", "--fps", str(FPS),
            "--width", "1080", "--height", "1920", "--track", "video,audio"])
    assert r.returncode == 0, f"cli new 失败: {r.stderr[:200]}"
    tl_rel, mat_rel = layout_rel(proj)
    (proj / mat_rel).mkdir(parents=True, exist_ok=True)
    make_media(proj / mat_rel, "a.mp4", MEDIA_S, 440)
    make_media(proj / mat_rel, "b.mp4", MEDIA_S, 660)
    write_base_project(proj, tl_rel, mat_rel)

    port = free_port()
    serve = subprocess.Popen(
        [str(mcp), "serve", "--root", str(proj), "--port", str(port),
         "--token", token, "--web", str(REPO / "apps" / "web")],
        stdout=subprocess.DEVNULL, stderr=open(tmp / "serve.log", "wb"))
    written: list[tuple[str, int]] = []
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
        base = f"http://127.0.0.1:{port}/?token={token}"

        with sync_playwright() as pw:
            browser = pw.chromium.launch(args=["--autoplay-policy=no-user-gesture-required"])
            for name, fn, _desc in CAPTURES:
                if args.only and args.only not in name:
                    continue
                write_base_project(proj, tl_rel, mat_rel)  # 每条回到基准盘面
                vdir = tmp / "videos" / name
                vdir.mkdir(parents=True, exist_ok=True)
                ctx = browser.new_context(
                    viewport=VIEWPORT, record_video_dir=str(vdir),
                    record_video_size=VIEWPORT)
                page = ctx.new_page()
                # 非引导条录制:预写「已读」偏好,统一画面基线(01 号除外)
                if not name.startswith("01-"):
                    page.add_init_script(
                        'localStorage.setItem("cutforge.prefs.v1", JSON.stringify({onboardDismissed:true}));')
                page.goto(base)
                page.wait_for_function(
                    "document.querySelector('[data-testid=\"rev\"]').textContent !== '-'", timeout=20000)
                try:
                    fn(page)
                    page.wait_for_timeout(300)
                finally:
                    video = page.video
                    ctx.close()
                files = list(vdir.glob("*.webm"))
                assert files, f"{name}: 未产出录屏"
                src = files[0]
                dst = out_dir / name
                shutil.move(str(src), str(dst))
                size = dst.stat().st_size
                written.append((name, size))
                mark = "OK " if size <= MAX_BYTES else "BIG"
                log(f"{mark} {name}: {size / 1024:.0f} KB")
            browser.close()

    finally:
        serve.terminate()
        total = sum(n for _, n in written)
        log(f"共 {len(written)} 条,合计 {total / 1024 / 1024:.2f} MB → {out_dir}")
        if total > 10 * 1024 * 1024:
            log("总增量 >10MB:请降分辨率/时长后重录(record_video_size/剧本等待)")
        shutil.rmtree(tmp, ignore_errors=True)

    overs = [(n, s) for n, s in written if s > MAX_BYTES]
    if overs:
        print(f"FAIL: {len(overs)} 条超 2MB: {overs}", file=sys.stderr)
        return 2
    print(f"record-captures: {len(written)} 条录屏落盘,全部 ≤2MB")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except AssertionError as e:
        print(f"FAIL: {e}", file=sys.stderr)
        sys.exit(2)
