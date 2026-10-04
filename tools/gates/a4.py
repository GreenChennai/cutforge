"""A4 · 册四「核心工具与媒体管线」册级门禁(D-A2 注册制)。A-10 拆包。

注:原文件中 A5 函数(check_e2e_keyframes/check_e2e_color)插在 A4 函数与 CHECKS_A4 之间;
拆包时按归属分入 a5.py,CHECKS_A4 的注册项与取值一字不变。
"""
from __future__ import annotations

from typing import Callable

from .a1 import (
    check_cargo_clippy,
    check_cargo_test_workspace,
    check_e2e_events,
    check_e2e_static,
    check_pytest_suite,
    check_tool_parity,
)
from .a2 import (
    check_e2e_perf_timeline,
    check_e2e_playback_survival,
    check_e2e_ui_smoke,
    check_js_line_limit,
    check_shell_purity_v2,
)
from .a3 import (
    check_e2e_a11y,
    check_e2e_drag_perf,
    check_e2e_hotkeys,
    check_e2e_perf_budget,
)
from .common import GATE_FAILED, OK, CheckResult


# ---------------- A4 · 册四「核心工具与媒体管线」册级门禁(D-A2 注册制,册四收尾) ----------------
# 与 A1/A2/A3 同风格:A3 十五项一字不动全部继承,另纳册四三份新 e2e
# (editing_tools/subtitle_editor/media_perf);media_perf 为负载敏感项(千素材滚动帧率),
# 不进 CI,本机册收官安静时段跑(同 perf_timeline/drag_perf 口径)。

def check_e2e_editing_tools() -> CheckResult:
    """A4(AC-4.2): 时间线编辑全工具 e2e(四件套手势各一次=恰一 Op(oplog 计数差值)/
    clip_split_all/clip_gap_delete/clip_paste_at 交互/四件套各实渲一次时长语义逐差对拍/
    锁定轨拒编辑零 Op)。"""
    return _a2_e2e_gate("e2e-editing-tools", "tools/e2e_editing_tools.py",
                        "四件套单 Op/split_all/gap_delete/paste/实渲对拍/锁定拒编辑")


def check_e2e_subtitle_editor() -> CheckResult:
    """A4(AC-4.4): 字幕编辑器全流程 e2e(SRT 导入导出 byte 级往返/批量替换单 Op+幂等/
    新建文本→画布拖位置→render_frame 帧上文字位置/卡拉OK \\kf 主色前沿推进/花字模板上帧
    +patch.huazi={} 显式清除/投影逐 clip 含 fx 键)。"""
    return _a2_e2e_gate("e2e-subtitle-editor", "tools/e2e_subtitle_editor.py",
                        "SRT 往返/替换/帧证位置/卡拉OK/花字+清除/fx 投影")


def check_e2e_media_perf() -> CheckResult:
    """A4(AC-4.5/4.6): 媒体池千素材性能 e2e(--min-fps 55;1000 文件目录/BROWSE_CAP 500
    如实截断/缩略图懒加载视口外零请求/代理开关在位)+ AC-4.6 听觉对比存档
    (降噪/变调前后四支 WAV 落 docs/design/audio-samples/ ≤3MB;机器听觉对比=人工项)。
    帧率阈值机器负载敏感:负载抖动时安静时段复跑(与 bench 同策);不进 CI。"""
    return _a2_e2e_gate("e2e-media-perf", "tools/e2e_media_perf.py",
                        "--min-fps 55;千素材懒加载+滚动帧率;负载抖动时安静时段复跑(与 bench 同策)",
                        "--min-fps", "55")


CHECKS_A4: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "cargo-clippy": (check_cargo_clippy, True),
    "cargo-test-workspace": (check_cargo_test_workspace, True),
    "e2e-a11y": (check_e2e_a11y, True),
    "e2e-drag-perf": (check_e2e_drag_perf, True),               # --min-fps 55;负载敏感,安静时段复跑
    "e2e-editing-tools": (check_e2e_editing_tools, True),       # 册四新增(AC-4.2)
    "e2e-events": (check_e2e_events, True),
    "e2e-hotkeys": (check_e2e_hotkeys, True),
    "e2e-media-perf": (check_e2e_media_perf, True),             # 册四新增(AC-4.5/4.6);负载敏感,不进 CI
    "e2e-perf-budget": (check_e2e_perf_budget, True),           # 真实导出;负载敏感,不进 CI
    "e2e-perf-timeline": (check_e2e_perf_timeline, True),       # --min-fps 55;负载敏感,安静时段复跑
    "e2e-playback-survival": (check_e2e_playback_survival, True),
    "e2e-static": (check_e2e_static, True),
    "e2e-subtitle-editor": (check_e2e_subtitle_editor, True),   # 册四新增(AC-4.4)
    "e2e-ui-smoke": (check_e2e_ui_smoke, True),
    "js-line-limit": (check_js_line_limit, True),               # 行数红线(apps/web;crates 侧=A1 rust-line-limit)
    "pytest-suite": (check_pytest_suite, True),
    "shell-purity-v2": (check_shell_purity_v2, True),
    "tool-parity": (check_tool_parity, True),                   # 56 工具黄金对拍
}
