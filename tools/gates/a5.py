"""A5 · 册五「专业深度」册级门禁(D-A2 注册制)。A-10 拆包。"""
from __future__ import annotations

from typing import Callable

from .a1 import (
    check_bench_threshold,
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
from .a4 import (
    check_e2e_editing_tools,
    check_e2e_media_perf,
    check_e2e_subtitle_editor,
)
from .common import GATE_FAILED, OK, CheckResult


# ---------------- A5 · 册五「专业深度」册级门禁(D-A2 注册制,册五收尾) ----------------
# 与 A1-A4 同风格:A4 十八项一字不动全部继承,另纳册五两份新 e2e
# (keyframes/color)+ bench 阈值(AC-5.7:基准重跑劣化 ≤20% 证据)。

def check_e2e_keyframes() -> CheckResult:
    """A5(AC-5.1): 关键帧壳侧闭环 e2e(秒表打点零 Op 松手单 Op/编辑器打点改值/
    投影 keyframes+keyframeSamples 采样对拍/求值一致性帧亮度/曲线编辑器拖锚点写回/
    时间线菱形拖移 PX_PER_MS 红线/插值预设单 Op/账目闭合)。"""
    return _a2_e2e_gate("e2e-keyframes", "tools/e2e_keyframes.py",
                        "秒表打点单 Op/投影采样对拍/曲线拖锚写回/菱形拖移/账目闭合")


def check_e2e_color() -> CheckResult:
    """A5(AC-5.2): 调色壳侧闭环 e2e(基线中性帧/色轮拖 lift 整对象写回单 Op/
    render_frame 帧色偏断言/LUT 导入应用帧压暗/示波器三画布非空/分屏割线拖动/
    grade 拷贝粘贴逐字段等价/账目闭合)。"""
    return _a2_e2e_gate("e2e-color", "tools/e2e_color.py",
                        "色轮写回单 Op/帧色偏/LUT/示波器三画布/分屏割线/账目闭合")


CHECKS_A5: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "bench-threshold": (check_bench_threshold, True),           # 册五新增(AC-5.7):基准劣化 ≤20%
    "cargo-clippy": (check_cargo_clippy, True),
    "cargo-test-workspace": (check_cargo_test_workspace, True),
    "e2e-a11y": (check_e2e_a11y, True),
    "e2e-color": (check_e2e_color, True),                       # 册五新增(AC-5.2)
    "e2e-drag-perf": (check_e2e_drag_perf, True),               # --min-fps 55;负载敏感,安静时段复跑
    "e2e-editing-tools": (check_e2e_editing_tools, True),       # A4 继承(AC-4.2)
    "e2e-events": (check_e2e_events, True),
    "e2e-hotkeys": (check_e2e_hotkeys, True),
    "e2e-keyframes": (check_e2e_keyframes, True),               # 册五新增(AC-5.1)
    "e2e-media-perf": (check_e2e_media_perf, True),             # 负载敏感,不进 CI
    "e2e-perf-budget": (check_e2e_perf_budget, True),           # 真实导出;负载敏感,不进 CI
    "e2e-perf-timeline": (check_e2e_perf_timeline, True),       # --min-fps 55;负载敏感,安静时段复跑
    "e2e-playback-survival": (check_e2e_playback_survival, True),
    "e2e-static": (check_e2e_static, True),
    "e2e-subtitle-editor": (check_e2e_subtitle_editor, True),   # A4 继承(AC-4.4)
    "e2e-ui-smoke": (check_e2e_ui_smoke, True),
    "js-line-limit": (check_js_line_limit, True),               # 行数红线(apps/web;crates 侧=A1 rust-line-limit)
    "pytest-suite": (check_pytest_suite, True),
    "shell-purity-v2": (check_shell_purity_v2, True),
    "tool-parity": (check_tool_parity, True),                   # 68 工具黄金对拍
}
