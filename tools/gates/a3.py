"""A3 · 册三「UX 动效/键位/可访问性」册级门禁(D-A2 注册制)。A-10 拆包。"""
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
    _a2_e2e_gate,
    check_e2e_perf_timeline,
    check_e2e_playback_survival,
    check_e2e_ui_smoke,
    check_js_line_limit,
    check_shell_purity_v2,
)
from .common import GATE_FAILED, OK, CheckResult


# ---------------- A3 · 册三「UX 动效/键位/可访问性」册级门禁(D-A2 注册制,册三收尾) ----------------
# 与 A1/A2 同风格:阻断/观察分级;A1/A2/M0-M7 一字不动。legacy/ 已删(任务 A2-L2),
# 行数红线与壳纯度扫描面 = apps/web 全量。四份册三新 e2e(drag_perf/hotkeys/a11y/perf_budget)
# 均为负载或真实导出敏感项:本机册收官跑,不进 CI(同 perf_timeline 口径)。

def check_e2e_drag_perf() -> CheckResult:
    """A3(AC-3.3): 拖拽手感 e2e(--min-fps 55;ghost ≤2 帧跟手「松手才动」阴性证明/
    拖拽零 Op/松手单 Op/非法落点红态/Esc 取消含 trim 几何复位)。
    帧率阈值机器负载敏感:负载抖动时安静时段复跑(与 bench 同策);不进 CI。"""
    return _a2_e2e_gate("e2e-drag-perf", "tools/e2e_drag_perf.py",
                        "--min-fps 55;负载抖动时安静时段复跑(与 bench 同策)",
                        "--min-fps", "55")


def check_e2e_hotkeys() -> CheckResult:
    """A3(AC-3.4): 键位体系 e2e(注册表 ≥40 条遍历/抽样实按 ≥15 条含 J·K·L、I/O、B、S、
    Shift+Del、Ctrl+Z/Y、+/-、\\、?、Shift+D/重绑定冲突检测闭环/输入态与对话框屏蔽/帮助搜索)。"""
    return _a2_e2e_gate("e2e-hotkeys", "tools/e2e_hotkeys.py", "注册表遍历+抽样实按+重绑定闭环")


def check_e2e_a11y() -> CheckResult:
    """A3(AC-3.6): 可访问性 e2e(键盘编辑闭环 Alt+←/→→Ctrl+→→Del→撤销( toast 按钮/Ctrl+Z)
    全程 rev/OpLog 断言;axe-core 4(tools/vendor/axe.min.js 入库存档)全页扫描
    0 新增 critical/serious;两笔已登记壳侧违规见脚本 KNOWN_REGISTERED,serious 以下登记不阻断)。"""
    return _a2_e2e_gate("e2e-a11y", "tools/e2e_a11y.py", "键盘链+axe 全页扫描(0 新增 critical/serious)")


def check_e2e_perf_budget() -> CheckResult:
    """A3(AC-3.5): 性能预算 e2e(首屏可交互 <1s/页签切换 <100ms/导出进度节奏+平滑/
    媒体池 200 轮导航有界 ≤POOL_MAX=24);结果 JSON 落 docs/bench/perf-a3.json。
    含真实导出(分钟级渲染)与本机负载相关项:不进 CI;4h 长跑为人工项(登记),
    以 200 轮导航模拟+池有界替代。"""
    return _a2_e2e_gate("e2e-perf-budget", "tools/e2e_perf_budget.py",
                        "boot<1s/tab<100ms/导出节奏+平滑/池有界;4h 长跑=人工项")


CHECKS_A3: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "cargo-clippy": (check_cargo_clippy, True),
    "cargo-test-workspace": (check_cargo_test_workspace, True),
    "e2e-a11y": (check_e2e_a11y, True),
    "e2e-drag-perf": (check_e2e_drag_perf, True),           # --min-fps 55;负载敏感,安静时段复跑
    "e2e-events": (check_e2e_events, True),
    "e2e-hotkeys": (check_e2e_hotkeys, True),
    "e2e-perf-budget": (check_e2e_perf_budget, True),       # 真实导出;负载敏感,不进 CI
    "e2e-perf-timeline": (check_e2e_perf_timeline, True),   # --min-fps 55;负载敏感,安静时段复跑
    "e2e-playback-survival": (check_e2e_playback_survival, True),
    "e2e-static": (check_e2e_static, True),
    "e2e-ui-smoke": (check_e2e_ui_smoke, True),
    "js-line-limit": (check_js_line_limit, True),           # 行数红线;无 legacy 后扫描面=apps/web 全量
    "pytest-suite": (check_pytest_suite, True),
    "shell-purity-v2": (check_shell_purity_v2, True),       # v3 含 R5 色值;R4 legacy 豁免已收口
    "tool-parity": (check_tool_parity, True),
}
