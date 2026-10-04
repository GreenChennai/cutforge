"""A6 · 册六「独立化」册级门禁(D-A2 注册制)。A-10 拆包。"""
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
from .a5 import check_e2e_color, check_e2e_keyframes
from .common import GATE_FAILED, OK, CheckResult


# ---------------- A6 · 册六「独立化」册级门禁(D-A2 注册制,册六收尾) ----------------
# 与 A1-A5 同风格:A5 二十一项一字不动全部继承,另纳册六 e2e_independence
# (AC-6.3 判定器:环境隔离 + 四大件内置工具链 + serve 进程子树零 python 子进程;
# CI 主场 = ubuntu 无 CUTFLOW_REPO 天然隔离,进 web-e2e)。

def check_e2e_independence() -> CheckResult:
    """A6(AC-6.3): 独立运行 e2e——CUTFLOW_* 全剥离的环境隔离起 serve;字幕(含花字)/
    卡点/重构图(anchorY 承载防丢)/成片导出四大件全程内置工具链;渲染全程对 serve
    进程子树采样断言零 python 子进程(剪映导出豁免,ADR-0023——本脚本不调用它);
    产物时长/像素抽样(纯绿字幕上帧);纯 CLI 面(无 UI:run-script 建卡→clip-update
    改字段→cutforge-render 直渲→ffprobe 校验,AC-7.4 预演)。"""
    return _a2_e2e_gate("e2e-independence", "tools/e2e_independence.py",
                        "环境隔离+四大件+零 python 进程树断言+产物校验+纯 CLI 面")


CHECKS_A6: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "bench-threshold": (check_bench_threshold, True),
    "cargo-clippy": (check_cargo_clippy, True),
    "cargo-test-workspace": (check_cargo_test_workspace, True),
    "e2e-a11y": (check_e2e_a11y, True),
    "e2e-color": (check_e2e_color, True),
    "e2e-drag-perf": (check_e2e_drag_perf, True),               # --min-fps 55;负载敏感,安静时段复跑
    "e2e-editing-tools": (check_e2e_editing_tools, True),
    "e2e-events": (check_e2e_events, True),
    "e2e-hotkeys": (check_e2e_hotkeys, True),
    "e2e-independence": (check_e2e_independence, True),         # 册六新增(AC-6.3 判定器)
    "e2e-keyframes": (check_e2e_keyframes, True),
    "e2e-media-perf": (check_e2e_media_perf, True),             # 负载敏感,不进 CI
    "e2e-perf-budget": (check_e2e_perf_budget, True),           # 真实导出;负载敏感,不进 CI
    "e2e-perf-timeline": (check_e2e_perf_timeline, True),       # --min-fps 55;负载敏感,安静时段复跑
    "e2e-playback-survival": (check_e2e_playback_survival, True),
    "e2e-static": (check_e2e_static, True),
    "e2e-subtitle-editor": (check_e2e_subtitle_editor, True),
    "e2e-ui-smoke": (check_e2e_ui_smoke, True),
    "js-line-limit": (check_js_line_limit, True),               # 行数红线(apps/web;crates 侧=A1 rust-line-limit)
    "pytest-suite": (check_pytest_suite, True),
    "shell-purity-v2": (check_shell_purity_v2, True),
    "tool-parity": (check_tool_parity, True),                   # 76 工具黄金对拍
}
