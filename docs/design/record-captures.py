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
VIEWPORT_NEW = {"width": 720, "height": 450}   # 册五补录段(总量红线;时长压缩后仍超即降此)
MAX_BYTES = 2 * 1024 * 1024
# 册五补录段前缀(40-46 微交互 + E-FE2 面板演示;低分辨率录制)
NEW_CAP_PREFIXES = ("40-", "41-", "42-", "43-", "44-", "45-", "46-",
                    "06-mixer", "07-mixer", "08-compound", "09-multicam",
                    "10-scene", "11-encode", "12-queue", "13-otio",
                    "50-", "51-", "52-", "53-", "54-", "55-",  # 册六(F4):工程库/迁移/模板/导出矩阵/preflight/素材库
                    "56-", "57-", "58-", "59-", "60-")  # 册七(F4):脚本页签/批准流/线程+报告/插件安装/贡献点+越权
# 册七补录段:录完即 VP9 CRF46 重编码压总量(目录 ≤8MB 红线;时长逐支不变)
RECAP_PREFIXES = ("56-", "57-", "58-", "59-", "60-")

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


def rpc(port: int, token: str, name: str, args: dict, timeout: float = 60) -> dict:
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": name, "arguments": args}}).encode()
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/rpc", data=body,
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"})
    out = json.loads(urllib.request.urlopen(req, timeout=timeout).read())
    return json.loads(out["result"]["content"][0]["text"])


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


def make_hardcut(ws: Path, name: str = "cut.mp4") -> None:
    """2s 红 + 2s 蓝硬切夹具(带静音轨;scene_detect 必然检出 ~2s 剪切点)。"""
    r = sh(["ffmpeg", "-y", "-loglevel", "error",
            "-f", "lavfi", "-i", "color=c=red:size=480x270:rate=30:duration=2",
            "-f", "lavfi", "-i", "color=c=blue:size=480x270:rate=30:duration=2",
            "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo",
            "-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0[v];[2:a]atrim=0:4[a]",
            "-map", "[v]", "-map", "[a]", "-pix_fmt", "yuv420p",
            "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac",
            str(ws / name)])
    assert r.returncode == 0 and (ws / name).is_file(), f"ffmpeg 硬切夹具失败:{r.stderr[-200:]}"


def write_compound_fixture(proj: Path, tl_rel: str, mat_rel: str) -> None:
    """08-compound 用:基准盘面 + V1-003 复合壳(两子片段内联子时间线)。"""
    write_base_project(proj, tl_rel, mat_rel)
    pj = json.loads((proj / tl_rel / "project.json").read_text(encoding="utf-8"))
    pj["tracks"][0]["clips"].append({
        "id": "V1-003", "src": f"{mat_rel}/a.mp4", "startMs": 60000, "durationMs": 4000,
        "compound": {"clips": [
            {"id": "V1-101", "src": f"{mat_rel}/a.mp4", "startMs": 0, "durationMs": 2500},
            {"id": "V1-102", "src": f"{mat_rel}/b.mp4", "startMs": 2500, "durationMs": 1500},
        ]},
    })
    (proj / tl_rel / "project.json").write_text(json.dumps(pj, ensure_ascii=False), encoding="utf-8")


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


# ---------------- 册五补录:微交互 #40-46(E-FE1 关键帧/调色面) ----------------

def expand_group(page, name: str) -> None:
    """检查器折叠组展开(重试至内容可见)。"""
    grp = page.locator(f'[data-testid="insp-group-{name}"]')
    for _ in range(3):
        if "collapsed" not in (grp.get_attribute("class") or ""):
            return
        grp.locator("legend").click()
        page.wait_for_timeout(250)
    assert "collapsed" not in (grp.get_attribute("class") or ""), f"检查器组「{name}」无法展开"


def select_clip(page, clip_id: str = "V1-001") -> None:
    page.click(f'[data-testid="clip"][data-id="{clip_id}"]')
    page.wait_for_timeout(350)


def cap_40_kf_watch(page):
    """#40 秒表开/关双态:开启 accent 描边 + aria-pressed,开启即打点(零动画)。"""
    reset_state(page)
    select_clip(page)
    expand_group(page, "画面")
    btn = page.locator('[data-testid="kw-toggle-opacity"]')
    btn.scroll_into_view_if_needed()
    page.wait_for_timeout(400)
    page.click('[data-testid="kw-toggle-opacity"]')
    page.wait_for_timeout(600)
    assert btn.get_attribute("aria-pressed") == "true", "秒表开启态应 aria-pressed=true"
    page.wait_for_timeout(500)


def cap_41_kf_watch_confirm(page):
    """#41 秒表关闭确认弹窗(复用 confirm-dialog;末属性整组清空文案如实呈现)。"""
    reset_state(page)
    select_clip(page)
    expand_group(page, "画面")
    page.locator('[data-testid="kw-toggle-opacity"]').scroll_into_view_if_needed()
    page.click('[data-testid="kw-toggle-opacity"]')
    page.wait_for_timeout(700)
    page.click('[data-testid="kw-toggle-opacity"]')
    page.wait_for_selector('[data-testid="confirm-dialog"]', timeout=6000)
    page.wait_for_timeout(700)
    page.click('[data-testid="confirm-ok"]')
    page.wait_for_timeout(600)


def cap_42_kf_diamond_drag(page):
    """#42 关键帧菱形拖拽跟手(时间码气泡;松手单 Op;末段 Esc 取消)。"""
    reset_state(page)
    select_clip(page)
    expand_group(page, "画面")
    page.click('[data-testid="kw-toggle-opacity"]')
    page.wait_for_timeout(500)
    page.click('[data-testid="ruler"]', position={"x": 240, "y": 10})
    page.wait_for_timeout(400)
    page.click('[data-testid="kw-add"]')   # 第二帧走「+ 打点」(秒表二连击=关闭确认,非打点)
    page.wait_for_timeout(500)
    dia = page.locator('[data-testid="kw-diamond"][data-time-ms="4000"]')
    dia.first.scroll_into_view_if_needed()
    box = dia.first.bounding_box()
    cx, cy = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
    page.mouse.move(cx, cy)
    page.mouse.down()
    for i in range(1, 8):
        page.mouse.move(cx + i * 9, cy)
        page.wait_for_timeout(55)
    page.keyboard.press("Escape")   # 拖拽中取消:零 Op 复位
    page.wait_for_timeout(600)
    page.mouse.move(cx, cy)
    page.mouse.down()
    for i in range(1, 8):
        page.mouse.move(cx + i * 9, cy)
        page.wait_for_timeout(55)
    page.mouse.up()                 # 松手提交:单 Op
    page.wait_for_timeout(600)


def cap_43_kf_curve_drag(page):
    """#43 曲线画布锚点拖拽(拖拽期直线示意 + 采样点云降淡;松手投影刷新真曲线)。"""
    reset_state(page)
    select_clip(page)
    expand_group(page, "画面")
    page.click('[data-testid="kw-toggle-opacity"]')
    page.wait_for_timeout(500)
    page.click('[data-testid="ruler"]', position={"x": 240, "y": 10})
    page.wait_for_timeout(400)
    page.click('[data-testid="kw-add"]')
    page.wait_for_timeout(700)
    cv = page.locator('[data-testid="kf-curve-canvas"]')
    cv.scroll_into_view_if_needed()
    box = cv.bounding_box()
    ax = box["x"] + (10 + (4000 / 12000) * 320) / 340 * box["width"]
    ay = box["y"] + (160 - 0.5 * 150) / 170 * box["height"]
    page.mouse.move(ax, ay)
    page.mouse.down()
    for i in range(1, 7):
        page.mouse.move(ax, ay - i * 9)
        page.wait_for_timeout(60)
    page.mouse.up()
    page.wait_for_timeout(700)
    ax2 = box["x"] + (10 + (4000 / 12000) * 320) / 340 * box["width"]
    ay2 = box["y"] + (160 - 0.9333 * 150) / 170 * box["height"]
    page.mouse.move(ax2, ay2)
    page.mouse.down()
    for i in range(1, 6):
        page.mouse.move(ax2 + i * 8, ay2 + i * 6)
        page.wait_for_timeout(55)
    page.keyboard.press("Escape")
    page.wait_for_timeout(700)


def cap_44_grade_wheel(page):
    """#44 色轮指针拖拽跟手(角度=色相 半径=强度;盘面光谱数据面)。"""
    reset_state(page)
    select_clip(page)
    expand_group(page, "调色")
    wh = page.locator('[data-testid="grade-wheel-lift"]')
    wh.scroll_into_view_if_needed()
    box = wh.bounding_box()
    cx, cy = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
    page.mouse.move(cx + 8, cy)
    page.mouse.down()
    for i in range(1, 13):
        page.mouse.move(cx + 8 + i * 3, cy - i * 2)
        page.wait_for_timeout(55)
    page.mouse.up()
    page.wait_for_timeout(600)
    page.click('[data-testid="grade-wheel-lift-reset"]')
    page.wait_for_timeout(500)


def cap_45_scopes_status(page):
    """#45 示波器采样状态文本态(采样中…→已采样@ms;数据域标注精确帧含调色)。"""
    reset_state(page)
    select_clip(page)
    page.click('[data-testid="scopes-toggle"]')
    page.wait_for_timeout(400)
    page.click('[data-testid="scopes-sample"]')
    page.wait_for_function(
        """() => document.querySelector('[data-testid="scopes-status"]').textContent.startsWith('已采样')""",
        timeout=60000)
    page.wait_for_timeout(800)


def cap_46_compare_divider(page):
    """#46 分屏割线拖拽跟手(clip-path inset 直写;双击复位 50%)。"""
    reset_state(page)
    select_clip(page)
    page.click('[data-testid="compare-toggle"]')
    page.wait_for_function(
        """() => { const i = document.querySelector('[data-testid="compare-img-base"]');
                   return i && !!i.getAttribute('src'); }""", timeout=120000)
    dv = page.locator('[data-testid="compare-divider"]')
    dv.scroll_into_view_if_needed()
    box = dv.bounding_box()
    cx, cy = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
    page.mouse.move(cx, cy)
    page.mouse.down()
    for i in range(1, 11):
        page.mouse.move(cx + i * 12, cy)
        page.wait_for_timeout(50)
    page.mouse.up()
    page.wait_for_timeout(500)
    dv.dblclick()
    page.wait_for_timeout(500)


# ---------------- 册五补录:E-FE2 面板演示(混音/复合/多机位/场景/编码/队列/OTIO) ----------------

def cap_e2_06_mixer_eq(page):
    """混音台 EQ:轨道条 + EQ 折叠组 → 加段 → 参数 → 应用(track_update 单 Op)。"""
    reset_state(page)
    page.click('[data-testid="tab-mixer"]')
    page.wait_for_selector('[data-testid="mixer-tracks"]', timeout=8000)
    page.wait_for_timeout(500)
    eq = page.locator('[data-testid="mix-eq-A1"]')
    eq.scroll_into_view_if_needed()
    eq.locator("legend").click()
    page.wait_for_timeout(400)
    page.click('[data-testid="mix-eq-add-A1"]')
    page.wait_for_timeout(400)
    page.fill('[data-testid="mix-eq-freq-A1"]', "8000")
    page.fill('[data-testid="mix-eq-gain-A1"]', "3")
    page.wait_for_timeout(300)
    page.click('[data-testid="mix-eq-apply-A1"]')
    page.wait_for_timeout(800)


def cap_e2_07_mixer_bus_loudness(page):
    """响度单:目标 I:TP → 取素材 → 测量 → 偏差徽标(ok/warn 如实)。"""
    reset_state(page)
    page.click('[data-testid="tab-mixer"]')
    page.wait_for_selector('[data-testid="mix-bus"]', timeout=8000)
    box = page.locator('[data-testid="mix-bus"]')
    box.scroll_into_view_if_needed()
    page.fill('[data-testid="mix-target"]', "-14")
    page.fill('[data-testid="mix-bus-src"]', "01_原始素材/a.mp4")
    page.wait_for_timeout(300)
    page.click('[data-testid="mix-bus-measure"]')
    page.wait_for_function(
        """() => { const n = document.querySelector('[data-testid="mix-bus-nums"]');
                   return n && n.textContent.trim() && !n.textContent.includes("—"); }""",
        timeout=60000)
    page.wait_for_timeout(900)


def cap_e2_08_compound(page):
    """复合片段:双击复合壳 → 说明卡(子片段概要)→ 解包还原(单 Op)。"""
    reset_state(page)
    page.evaluate(
        """() => { const w = document.querySelector('[data-testid="timeline-wrap"]');
                    w.scrollLeft = 60000 * 0.06 - 260; }""")
    page.wait_for_timeout(400)
    page.dblclick('[data-testid="clip"][data-id="V1-003"]')
    page.wait_for_selector('[data-testid="cpd-summary"]', timeout=8000)
    page.wait_for_timeout(800)
    page.click('[data-testid="cpd-unbind"]')
    page.wait_for_timeout(1000)


def cap_e2_09_multicam(page):
    """多机位:同步集手输 ×2 → 同步分析(置信度/偏移如实)→ 打切换点 → 生成序列。"""
    reset_state(page)
    page.click('[data-testid="tab-multicam"]')
    page.wait_for_selector('[data-testid="mc-angles"]', timeout=8000)
    mc = page.locator('[data-testid="mc-angles"]')
    mc.scroll_into_view_if_needed()
    for src in ("01_原始素材/a.mp4", "01_原始素材/a2.mp4"):
        page.fill('[data-testid="mc-angle-src"]', src)
        page.click('[data-testid="mc-angle-add"]')
        page.wait_for_timeout(350)
    page.click('[data-testid="mc-sync"]')
    page.wait_for_function(
        """() => { const o = document.querySelector('[data-testid="mc-sync-out"]');
                   return o && o.textContent.includes("offset"); }""", timeout=120000)
    page.wait_for_timeout(500)
    # 切换点打点需播放头定位:页签互斥(tab-multicam 隐藏时间线),故来回切换
    def seek_and_switch(ms_px: int, angle: str) -> None:
        page.click('[data-testid="tab-timeline"]')
        page.wait_for_timeout(250)
        page.click('[data-testid="ruler"]', position={"x": ms_px, "y": 10})
        page.wait_for_timeout(300)
        page.click('[data-testid="tab-multicam"]')
        page.wait_for_timeout(250)
        if angle is not None:
            page.select_option('[data-testid="mc-switch-angle"]', angle)
        page.click('[data-testid="mc-switch-add"]')
        page.wait_for_timeout(300)
    seek_and_switch(180, "0")
    seek_and_switch(360, "1")
    cut = page.locator('[data-testid="mc-cut"]')
    cut.scroll_into_view_if_needed()
    page.select_option('[data-testid="mc-track"]', "V1")
    page.fill('[data-testid="mc-duration"]', "6000")
    page.click('[data-testid="mc-cut"]')
    page.wait_for_timeout(900)
    page.click('[data-testid="tab-timeline"]')   # 回时间线展示生成序列
    page.wait_for_timeout(400)


def cap_e2_10_scene(page):
    """场景检测:硬切夹具检测(切点列表)→ 勾选自动切段 → 单 Op 落轨。"""
    reset_state(page)
    page.click('[data-testid="tab-multicam"]')
    st = page.locator('[data-testid="scene-tool"]')
    st.scroll_into_view_if_needed()
    page.wait_for_timeout(300)
    page.fill('[data-testid="scene-src"]', "01_原始素材/cut.mp4")
    page.click('[data-testid="scene-run"]')
    # 壳侧渲染友好摘要(scene-summary「N 剪切点」),不含原始键名 cutCount —— 等摘要行出现
    page.wait_for_selector('[data-testid="scene-summary"]', timeout=120000)
    page.wait_for_timeout(600)
    # toggleField 的 testid 挂在 checkbox 本体(非包装层)
    page.click('[data-testid="scene-auto"]')
    page.wait_for_timeout(300)
    page.select_option('[data-testid="scene-track"]', "V1")
    page.click('[data-testid="scene-run"]')
    page.wait_for_timeout(1100)


def cap_e2_11_encode_hw(page):
    """编码探测:encode_probe 毫秒级清单(硬件在位/试编可用性如实)。"""
    reset_state(page)
    enc = page.locator('[data-testid="export-encode"]')
    enc.scroll_into_view_if_needed()
    page.wait_for_timeout(300)
    page.click('[data-testid="export-probe"]')
    page.wait_for_function(
        """() => { const o = document.querySelector('[data-testid="export-probe-out"]');
                   return o && !o.textContent.includes("未探测"); }""", timeout=60000)
    page.wait_for_timeout(900)


def make_cap_e2_12(port: int, token: str, root: str):
    """队列生命周期录制(需要 serve 端口/令牌/工程根 → 工厂闭包):
    入队(queued/running)→ 暂停 → 恢复 → 取消(canceled)。"""
    def run(page) -> None:
        reset_state(page)
        rpc(port, token, "render_run", {"root": root}, timeout=30)
        page.click('[data-testid="tab-queue"]')
        page.wait_for_timeout(600)
        page.click('[data-testid="queue-refresh"]')
        page.wait_for_selector('[data-testid="queue-row"]', timeout=15000)
        page.wait_for_timeout(800)
        pause = page.locator('[data-testid="queue-pause"]').first
        if pause.is_disabled():
            page.wait_for_timeout(2500)
        if not pause.is_disabled():
            pause.click()
            page.wait_for_timeout(900)
            resume = page.locator('[data-testid="queue-resume"]').first
            if not resume.is_disabled():
                resume.click()
                page.wait_for_timeout(600)
        cancel = page.locator('[data-testid="queue-cancel"]').first
        if not cancel.is_disabled():
            cancel.click()
            page.wait_for_timeout(900)
        page.click('[data-testid="queue-refresh"]')
        page.wait_for_timeout(600)
    return run


def cap_e2_13_otio(page):
    """OTIO 往返:导出(派生物落 06_成片输出)→ 导入对话框(预填路径)→ 新工程。"""
    reset_state(page)
    ot = page.locator('[data-testid="export-otio"]')
    ot.scroll_into_view_if_needed()
    page.select_option('[data-testid="export-otio-format"]', "otio")
    page.click('[data-testid="export-otio-run"]')
    page.wait_for_timeout(800)
    page.click('[data-testid="export-otio-import"]')
    page.wait_for_selector('[data-testid="otio-run"]', timeout=8000)
    page.wait_for_timeout(300)
    page.click('[data-testid="otio-run"]')
    page.wait_for_function(
        """() => { const o = document.querySelector('[data-testid="otio-progress"]');
                   return o && o.textContent.includes("完成"); }""", timeout=60000)
    page.wait_for_timeout(600)
    page.click('[data-testid="otio-cancel"]')
    page.wait_for_timeout(300)

def write_missing_fixture(proj: Path, tl_rel: str, mat_rel: str) -> None:
    """54-preflight 用:基准盘面 + V1-003 指向不存在的素材(preflight missingAssets 实证)。"""
    write_base_project(proj, tl_rel, mat_rel)
    pj = json.loads((proj / tl_rel / "project.json").read_text(encoding="utf-8"))
    pj["tracks"][0]["clips"].append({
        "id": "V1-003", "src": f"{mat_rel}/gone.mp4", "startMs": 60000, "durationMs": 4000,
    })
    (proj / tl_rel / "project.json").write_text(json.dumps(pj, ensure_ascii=False), encoding="utf-8")


def make_media_lib(lib_root: Path) -> None:
    """55-media_lib 用:素材库根(bgm/sfx 两支;media_library 扫描合并幂等入库)。"""
    lib_root.mkdir(parents=True, exist_ok=True)
    for name, freq, seconds in (("bgm-light.mp3", 660, 6), ("sfx-pop.mp3", 990, 1)):
        r = sh(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi",
                "-i", f"sine=frequency={freq}:duration={seconds}",
                "-c:a", "libmp3lame", "-b:a", "64k", str(lib_root / name)])
        assert r.returncode == 0, f"素材库夹具失败:{r.stderr[-200:]}"


def menu_click(page, label: str) -> None:
    """「工程 ▾」菜单点按(菜单复用 context-menu 载体,按可见文本定位)。"""
    page.click('[data-testid="project-menu"]')
    item = page.locator('[data-testid="context-menu"] button', has_text=label).first
    item.wait_for(state="visible", timeout=4000)
    item.click()
    page.wait_for_timeout(250)


def card_btn(page, card_name: str, tid: str):
    return (page.locator('[data-testid="lib-card"]', has_text=card_name).first
            .locator(f'[data-testid="{tid}"]'))


# ---------------- 册六补录:T6.1 工程库 / 迁移 / 模板 / T6.3 导出矩阵 / preflight / 素材库 ----------------

def cap_50_library_ops(page):
    """工程库视图七操作(50):菜单进库 → 搜索 → 重命名/复制/归档/恢复/删除/打开命令复制。
    「库内新建」子面缺录(登记):library-ops.js:166 把字段对象 `name` 而非 `name.root`
    传进 h(),对话框 appendChild 非节点打不开(apps/web 本窗口禁碰,A6-L 人工修复后补录)。"""
    menu_click(page, "工程库…")
    page.wait_for_selector('[data-testid="lib-grid"]', timeout=8000)
    page.locator('[data-testid="lib-refresh"]').click()
    page.wait_for_selector('[data-testid="lib-card"]', timeout=8000)
    page.wait_for_timeout(600)
    # 搜索命中过滤(服务端 query)
    page.fill('[data-testid="lib-search"]', "演示")
    page.wait_for_timeout(700)
    page.fill('[data-testid="lib-search"]', "")
    page.wait_for_timeout(500)
    # 重命名 → 复制 → 归档 → 归档区徽标 → 恢复 → 删除(.trash 捞回提示在确认框文案)
    card_btn(page, "演示副本", "lib-rename").click()
    page.wait_for_selector('[data-testid="lib-rename-to"]', timeout=4000)
    page.fill('[data-testid="lib-rename-to"]', "演示副本二号")
    page.click('[data-testid="lib-rename-run"]')
    page.wait_for_timeout(900)
    card_btn(page, "演示副本二号", "lib-copy").click()
    page.wait_for_selector('[data-testid="lib-copy-to"]', timeout=4000)
    page.fill('[data-testid="lib-copy-to"]', "演示副本三号")
    page.click('[data-testid="lib-copy-run"]')
    page.wait_for_timeout(900)
    card_btn(page, "演示副本三号", "lib-archive").click()
    page.wait_for_timeout(900)
    page.click('[data-testid="lib-archived"]')   # 并入归档区:归档徽标可见
    page.wait_for_timeout(800)
    card_btn(page, "演示副本三号", "lib-unarchive").click()
    page.wait_for_timeout(900)
    card_btn(page, "演示副本二号", "lib-delete").click()
    page.wait_for_selector('[data-testid="lib-delete-run"]', timeout=4000)
    page.wait_for_timeout(600)
    page.click('[data-testid="lib-delete-run"]')
    page.wait_for_timeout(900)
    # 打开 = serve 命令一键复制(诚实口径:一个 serve 一个工程;当前工程点开只给 toast)
    card_btn(page, "record-caps", "lib-open").click()
    page.wait_for_timeout(800)
    card_btn(page, "演示副本三号", "lib-open").click()
    page.wait_for_selector('[data-testid="lib-open-cmd"]', timeout=4000)
    page.wait_for_timeout(800)
    page.click('[data-testid="lib-open-close"]')
    page.wait_for_timeout(400)


def cap_51_migrate_v3(page):
    """迁移 v3(51):v2 工程迁移提示条 → 确认框 → 迁移后页面重载 → 工程菜单只读布局项。"""
    banner = page.locator('[data-testid="banner-migrate"]')
    try:
        banner.wait_for(state="visible", timeout=6000)
        page.click('[data-testid="migrate-banner-run"]')
    except Exception:  # 提示条被偏好拦时走常驻入口(诚实降级路径)
        menu_click(page, "迁移到 v3")
    page.wait_for_selector('[data-testid="migrate-run"]', timeout=6000)
    page.wait_for_timeout(800)
    page.click('[data-testid="migrate-run"]')
    page.wait_for_function(
        "document.querySelector('[data-testid=\"rev\"]').textContent !== '-'", timeout=20000)
    page.wait_for_timeout(900)
    # 迁移后:工程菜单的迁移入口应已禁用(已是 v3,只读布局信息项)
    menu_click(page, "工程库…")   # 借菜单出现展示 v3 文案后收掉
    page.keyboard.press("Escape")
    page.wait_for_timeout(500)


def cap_52_wizard_template(page):
    """向导三模板(52):模板切换预填画幅/帧率/轨道 → 布局 v3 选择 → Esc 关闭(不创建)。"""
    page.click('[data-testid="project-new"]')
    page.wait_for_selector('[data-testid="wiz-template"]', timeout=6000)
    for tpl in ("duo", "social", "vo"):
        page.select_option('[data-testid="wiz-template"]', tpl)
        page.wait_for_timeout(550)   # 选即预填:画幅/帧率/轨道联动(手改即脱离)
    page.select_option('[data-testid="wiz-layout"]', "v3")
    page.wait_for_timeout(700)
    page.keyboard.press("Escape")
    page.wait_for_timeout(500)


def cap_53_export_matrix(page):
    """导出矩阵面板(53):七格式说明行联动 → 三档位 → 区域窗口 → 仅视频。"""
    box = page.locator('[data-testid="export-matrix"]')
    box.scroll_into_view_if_needed()
    for fmt in ("gif", "m4a", "png-seq", ""):   # "" = mp4-h264 缺省出口(选项面无该值,见 FORMATS)
        page.select_option('[data-testid="export-format"]', fmt)
        page.wait_for_timeout(450)   # 说明行随格式切换(gif=12fps/m4a=仅音频/png-seq=序列帧)
    page.select_option('[data-testid="export-preset"]', "square")
    page.select_option('[data-testid="export-quality-tier"]', "720p")
    page.select_option('[data-testid="export-bitrate-tier"]', "high")
    page.wait_for_timeout(500)
    page.fill('[data-testid="export-in-ms"]', "1000")
    page.fill('[data-testid="export-out-ms"]', "4000")
    page.click('[data-testid="export-range-fill"]')   # 无入出点时的诚实 toast;再手填窗口
    page.wait_for_timeout(700)
    page.check('[data-testid="export-video-only"]')
    page.wait_for_timeout(500)
    page.uncheck('[data-testid="export-video-only"]')
    page.wait_for_timeout(500)


def cap_54_preflight_gate(page):
    """preflight 检查门(54):「导出成片」触发导出前自动门 → 缺失素材 warn 行 →
    裁决「先不导出」。(裁决按钮 export-pf-cancel/anyway 只在自动门路径渲染——
    runPreflight(autoResolve) 有问题项才挂;手动「检查」按钮路径无裁决面。)"""
    page.click('[data-testid="export-run"]')   # cutforge 后端导出 = preflight 自动门
    page.wait_for_selector('[data-testid="export-preflight"]:not([hidden])', timeout=30000)
    page.wait_for_selector('[data-testid="export-pf-item"]', timeout=30000)
    page.wait_for_timeout(900)
    # 缺失素材(gone.mp4)= 问题项 → 裁决门:先不导出(诚实演示;仍要导出按钮同面)
    page.click('[data-testid="export-pf-cancel"]')
    page.wait_for_timeout(700)


def cap_55_media_library(page):
    """素材库页签(55):库根扫描入库 → 类型 chips/标签/搜索 → 标签整组替换 → 一键导入。"""
    page.click('[data-testid="media-tab-library"]')
    page.wait_for_selector('[data-testid="media-lib-root"]', timeout=6000)
    page.fill('[data-testid="media-lib-root"]', str(LIB_ROOT))
    page.click('[data-testid="media-lib-refresh"]')
    page.wait_for_selector('[data-testid="media-lib-item"]', timeout=15000)
    page.wait_for_timeout(700)
    page.fill('[data-testid="media-lib-tag"]', "音乐")
    page.wait_for_timeout(600)
    page.fill('[data-testid="media-lib-tag"]', "")
    page.wait_for_timeout(400)
    item = page.locator('[data-testid="media-lib-item"]').first
    item.locator('[data-testid="media-lib-tag-edit"]').click()
    page.wait_for_selector('[data-testid="media-lib-tags-input"]', timeout=4000)
    page.fill('[data-testid="media-lib-tags-input"]', "音乐,轻快")
    page.click('[data-testid="media-lib-tags-run"]')
    page.wait_for_timeout(900)
    item.locator('[data-testid="media-lib-import"]').first.click()
    page.wait_for_timeout(1100)
    page.click('[data-testid="media-tab-project"]')
    page.wait_for_timeout(500)


# ---------------- 册七补录(F4):脚本页签 / 批准流 / 线程+报告 / 插件 ----------------

def _plugin_files(example: str) -> list[str]:
    base = REPO / "apps" / "web" / "examples" / "plugins" / example
    return [str(base / "manifest.json"), str(base / "main.js")]


def _install_example_plugin(page, example: str) -> None:
    """安装三连(校验卡 → 写入注册表;候选卡停留一拍供画面可读)。"""
    page.set_input_files('[data-testid="plugin-files"]', _plugin_files(example))
    page.click('[data-testid="plugin-install"]')
    page.wait_for_selector('[data-testid="plugin-candidate-card"]', timeout=10000)
    page.wait_for_timeout(700)
    page.click('[data-testid="plugin-install-confirm"]')
    page.wait_for_selector('[data-testid="plugin-row"]', timeout=6000)


def _enable_first_plugin(page) -> None:
    """首启确认 → 启用(确认对话框停留一拍;权限五面入画)。"""
    page.locator('[data-testid="plugin-row"]').first.locator('[data-testid="plugin-enable"]').click()
    page.wait_for_selector('[data-testid="plugin-confirm-dialog"]', timeout=5000)
    page.wait_for_timeout(700)
    page.click('[data-testid="plugin-confirm-ok"]')
    page.wait_for_timeout(1200)


def cap_56_script_run(page):
    """脚本页签运行(56):批量变色内置片段载入 → 运行(preview_plan 预演)→ 结构化输出。"""
    reset_state(page)
    page.click('[data-testid="tab-script"]')
    page.select_option('[data-testid="script-lib-select"]', index=1)
    page.click('[data-testid="script-lib-load"]')
    page.wait_for_timeout(700)
    page.click('[data-testid="script-run"]')
    page.wait_for_selector('[data-testid="script-out-item"]', timeout=20000)
    page.wait_for_timeout(1500)


def cap_57_plan_flow(page):
    """plan 批准流(57):以 plan 提交 → 差异面板预演卡 → 逐项批准/拒绝 → apply 回执。"""
    reset_state(page)
    page.click('[data-testid="tab-script"]')
    page.select_option('[data-testid="script-lib-select"]', index=1)
    page.click('[data-testid="script-lib-load"]')
    page.wait_for_timeout(300)
    page.click('[data-testid="script-submit"]')
    page.wait_for_timeout(600)
    page.click('[data-testid="tab-diff"]')
    page.wait_for_selector('[data-testid="plan-draft-head"]', timeout=5000)
    page.click('[data-testid="plan-preview"]')
    page.wait_for_selector('[data-testid="plan-preview-summary"]', timeout=20000)
    page.wait_for_timeout(800)
    items = page.locator('[data-testid="plan-item"]')
    items.nth(0).locator('[data-testid="plan-item-approve"]').click()
    page.wait_for_timeout(300)
    items.nth(1).locator('[data-testid="plan-item-reject"]').click()
    page.wait_for_timeout(500)
    page.click('[data-testid="plan-apply"]')
    page.wait_for_selector('[data-testid="plan-apply-result"]', timeout=20000)
    page.wait_for_timeout(1600)


def cap_58_note_thread_report(page):
    """标注线程 + 会话报告(58):创建标注 → 线程两轮回复(note_reply)→ session_report 渲染。"""
    reset_state(page)
    page.click('[data-testid="tab-notes"]')
    page.fill('[data-testid="note-body"]', "录屏:这段语速偏快")
    page.click('[data-testid="note-create"]')
    page.wait_for_selector('[data-testid="note-row"]', timeout=8000)
    for text in ("人问:能再快 5% 吗?", "AI 答:已按 5% 提速"):
        row = page.locator('[data-testid="note-row"]').first  # 行随 refresh 重建,逐轮重取
        row.locator('[data-testid="note-thread-body"]').fill(text)
        row.locator('[data-testid="note-thread-send"]').click()
        page.wait_for_timeout(1000)
    page.locator('[data-testid="report-run"]').click()
    page.wait_for_selector('[data-testid="report-markdown"]', timeout=15000)
    page.wait_for_timeout(1400)


def cap_59_plugin_install_confirm(page):
    """插件安装确认(59):选文件 → plugin_validate 校验卡 → 写入注册表 → 首启权限确认 → 运行。"""
    reset_state(page)
    page.click('[data-testid="tab-plugins"]')
    _install_example_plugin(page, "demo-panel")
    page.wait_for_timeout(400)
    _enable_first_plugin(page)
    page.wait_for_timeout(800)


def cap_60_plugin_contribs_forbidden(page):
    """贡献点 + 越权拦截(60):命令贡献点进右键菜单并调用 → 只读插件越权被 FORBIDDEN 拦截。"""
    reset_state(page)
    page.click('[data-testid="tab-plugins"]')
    _install_example_plugin(page, "demo-command")
    _enable_first_plugin(page)
    page.wait_for_timeout(500)
    page.click('[data-testid="tab-timeline"]')
    page.wait_for_timeout(300)
    page.click('[data-testid="clip"]', button="right")
    page.wait_for_selector('[data-testid="context-menu"]', timeout=5000)
    page.wait_for_timeout(600)
    page.click('[data-testid="context-menu"] button:has-text("统计片段数")')
    page.wait_for_timeout(1200)
    page.click('[data-testid="tab-plugins"]')
    naughty = NAUGHTY_DIR
    page.set_input_files('[data-testid="plugin-files"]',
                         [str(naughty / "manifest.json"), str(naughty / "main.js")])
    page.click('[data-testid="plugin-install"]')
    page.wait_for_selector('[data-testid="plugin-candidate-card"]', timeout=10000)
    page.wait_for_timeout(600)
    page.click('[data-testid="plugin-install-confirm"]')
    page.wait_for_timeout(400)
    row = page.locator('[data-testid="plugin-row"][data-pid="naughty-plugin"]')
    row.locator('[data-testid="plugin-enable"]').click()
    page.wait_for_selector('[data-testid="plugin-confirm-dialog"]', timeout=5000)
    page.click('[data-testid="plugin-confirm-ok"]')
    page.wait_for_timeout(1800)  # 越权 toast(GUARD_FAILED/FORBIDDEN)入画


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
    # ---- 册五补录:微交互 #40-46 ----
    ("40-kf-watch.webm", cap_40_kf_watch,
     "清单 40:秒表开/关双态(accent 描边 + aria-pressed;开启即打点)"),
    ("41-kf-watch-confirm.webm", cap_41_kf_watch_confirm,
     "清单 41:秒表关闭确认弹窗(confirm-dialog 复用;清空通道文案如实)"),
    ("42-kf-diamond-drag.webm", cap_42_kf_diamond_drag,
     "清单 42:关键帧菱形拖拽跟手(时间码气泡;Esc 取消 + 松手单 Op)"),
    ("43-kf-curve-drag.webm", cap_43_kf_curve_drag,
     "清单 43:曲线画布锚点拖拽(直线示意;松手投影刷新真曲线)"),
    ("44-grade-wheel.webm", cap_44_grade_wheel,
     "清单 44:色轮指针拖拽跟手(角度=色相 半径=强度)"),
    ("45-scopes-status.webm", cap_45_scopes_status,
     "清单 45:示波器采样状态文本态(采样中→已采样@ms;数据域标注)"),
    ("46-compare-divider.webm", cap_46_compare_divider,
     "清单 46:分屏割线拖拽跟手(clip-path 直写;双击复位 50%)"),
    # ---- 册五补录:E-FE2 面板演示 ----
    ("06-mixer-eq.webm", cap_e2_06_mixer_eq,
     "E-FE2:混音台轨道 EQ(track_update patch.eq 单 Op)"),
    ("07-mixer-bus-loudness.webm", cap_e2_07_mixer_bus_loudness,
     "E-FE2:响度单(audio_loudness 测量 + 偏差徽标如实)"),
    ("08-compound.webm", cap_e2_08_compound,
     "E-FE2:复合片段说明卡 + 解包(compound_unbind 单 Op;盘面走覆盖夹具)"),
    ("09-multicam.webm", cap_e2_09_multicam,
     "E-FE2:多机位同步分析 → 切换点 → 生成序列(multicam_cut 单 Op)"),
    ("10-scene.webm", cap_e2_10_scene,
     "E-FE2:场景检测(scene_detect 硬切夹具;可选自动切段单 Op)"),
    ("11-encode-hw.webm", cap_e2_11_encode_hw,
     "E-FE2:编码探测(encode_probe;AMF/NVENC 在位如实)"),
    ("12-queue-lifecycle.webm", None,
     "E-FE2:渲染队列生命周期(入队→暂停→恢复→取消;工厂闭包注册)"),
    ("13-otio.webm", cap_e2_13_otio,
     "E-FE2:OTIO 导出→导入(新工程;往返最小子集)"),
    # ---- 册六补录:T6.1 工程库/迁移/模板 + T6.3 导出矩阵/preflight + T6.2 素材库 ----
    # 51-migrate 排最后:迁移会改共享夹具布局(v2→v3),不回头改写其他录制基准面
    ("50-library-ops.webm", cap_50_library_ops,
     "T6.1 工程库七操作(新建/搜索/重命名/复制/归档/恢复/删除/打开命令复制)"),
    ("51-migrate-v3.webm", cap_51_migrate_v3,
     "T6.1 布局迁移 v3(提示条→确认框→重载;工程菜单常驻入口)"),
    ("52-wizard-template.webm", cap_52_wizard_template,
     "T6.1 向导三模板(口播/双机位/方形选即预填;布局 v3 选择)"),
    ("53-export-matrix.webm", cap_53_export_matrix,
     "T6.3 导出矩阵(七格式说明行/三档位/区域窗口/仅视频)"),
    ("54-preflight-gate.webm", cap_54_preflight_gate,
     "T6.3 导出前检查门(缺失素材 warn 行→裁决「先不导出」)"),
    ("55-media-library.webm", cap_55_media_library,
     "T6.2 素材库(库根扫描/chips/标签整组替换/一键导入)"),
    # ---- 册七补录(F4):脚本页签/批准流/线程+报告/插件安装/贡献点+越权 ----
    ("56-script-run.webm", cap_56_script_run,
     "T7.3 脚本页签运行(内置片段载入 → preview_plan 预演 → 结构化逐步回执)"),
    ("57-plan-approve-flow.webm", cap_57_plan_flow,
     "T7.5 计划批准流(以 plan 提交 → 预演卡 → 逐项批准/拒绝 → apply 回执+planId)"),
    ("58-note-thread-report.webm", cap_58_note_thread_report,
     "T7.5 标注线程两轮(note_reply)+ session_report 人话报告渲染"),
    ("59-plugin-install-confirm.webm", cap_59_plugin_install_confirm,
     "T7.2 插件安装确认(plugin_validate 校验卡 → 注册表 → 首启权限确认 → 运行)"),
    ("60-plugin-contribs-forbidden.webm", cap_60_plugin_contribs_forbidden,
     "T7.2 贡献点生效(命令进右键菜单)+ 越权双拦截(只读插件调写工具 FORBIDDEN)"),
]


def reencode_vp9(src: Path, crf: int = 46) -> None:
    """册七补录段压总量:VP9 CRF46 重编码(时长逐支不变;失败/未变小则保留原录像)。"""
    tmp_out = src.with_name(src.stem + ".reenc.webm")
    r = sh(["ffmpeg", "-y", "-loglevel", "error", "-i", str(src),
            "-c:v", "libvpx-vp9", "-crf", str(crf), "-b:v", "0",
            "-cpu-used", "8", "-row-mt", "1", "-an", str(tmp_out)])
    if r.returncode == 0 and tmp_out.is_file() and tmp_out.stat().st_size < src.stat().st_size:
        tmp_out.replace(src)
    else:
        if tmp_out.exists():
            tmp_out.unlink()


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
    make_media(proj / mat_rel, "a2.mp4", 6, 440)   # 同源短素材(09 多机位同步演示;秒级互相关)
    make_hardcut(proj / mat_rel)
    write_base_project(proj, tl_rel, mat_rel)
    global LIB_ROOT
    LIB_ROOT = tmp / "medialib"                    # 55 素材库根(扫描合并幂等入库)
    make_media_lib(LIB_ROOT)
    # 60 越权插件夹具(只读权限却调写工具;宿主镜像 GUARD_FAILED/FORBIDDEN 拦截演示)
    global NAUGHTY_DIR
    NAUGHTY_DIR = tmp / "naughty-plugin"
    NAUGHTY_DIR.mkdir(parents=True)
    (NAUGHTY_DIR / "manifest.json").write_text(json.dumps({
        "id": "naughty-plugin", "name": "越权演示插件", "version": "1.0.0",
        "form": "worker", "entry": "main.js", "description": "只读权限却调写工具(拦截演示)",
        "permissions": {"read": True, "write": False, "network": False,
                        "filesystem": [], "exec": False},
    }, ensure_ascii=False), encoding="utf-8")
    (NAUGHTY_DIR / "main.js").write_text(
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
    # 50 工程库演示卡(「库内新建」对话框壳侧 bug 登记期,卡面由 CLI 造,见 cap_50 docstring)
    rc_lib = sh([str(cli), "library", "new", "演示副本", "--library", str(tmp), "--json",
                 "--layout", "v2", "--slug", "demo-copy", "--fps", "30",
                 "--width", "1080", "--height", "1920"])
    assert rc_lib.returncode == 0, f"library new 夹具失败: {rc_lib.stderr[:200]}"

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
            # 12 号队列生命周期需要 serve 端口/令牌/工程根 → 此处工厂注册
            caps = [(n, f, d) for n, f, d in CAPTURES]
            caps = [(n, make_cap_e2_12(port, token, str(proj)) if f is None else f, d)
                    for n, f, d in caps]
            for name, fn, _desc in caps:
                if args.only and args.only not in name:
                    continue
                # 每条回到基准盘面(08-compound 走复合壳覆盖夹具;54-preflight 走缺失素材夹具)
                if name.startswith("08-compound"):
                    write_compound_fixture(proj, tl_rel, mat_rel)
                elif name.startswith("54-preflight"):
                    write_missing_fixture(proj, tl_rel, mat_rel)
                else:
                    write_base_project(proj, tl_rel, mat_rel)
                vdir = tmp / "videos" / name
                vdir.mkdir(parents=True, exist_ok=True)
                size = VIEWPORT_NEW if name.startswith(NEW_CAP_PREFIXES) else VIEWPORT
                ctx = browser.new_context(
                    viewport=size, record_video_dir=str(vdir),
                    record_video_size=size)
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
                if name.startswith(RECAP_PREFIXES):
                    reencode_vp9(dst)  # 册七补录段:VP9 CRF46 压总量(时长不变)
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
