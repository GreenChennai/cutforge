"""A7 · 册七「AI 原生与开放生态」册级门禁(D-A2 注册制)。A-10 拆包。"""
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
from .a6 import check_e2e_independence
from .common import GATE_FAILED, OK, CheckResult


# ---------------- A7 · 册七「AI 原生与开放生态」册级门禁(D-A2 注册制,册七收尾) ----------------
# 与 A1-A6 同风格:A6 二十二项一字不动全部继承,另纳册七两份新 e2e——
# e2e_ai_native(AC-7.3/7.5 判定器 + P0 文件级回归断言)与 e2e_headless(AC-7.4
# 纯 CLI 全链验收);24 项全阻断。CI web-e2e 只加 e2e_ai_native(e2e_headless 含
# watch 真实渲染与批处理,分钟级,同 perf 类"本机册收官跑"口径)。

def check_e2e_ai_native() -> CheckResult:
    """A7(AC-7.3/7.5): AI 原生 e2e——脚本页签三内置片段载入→预演→结构化输出
    (preview_plan 副本 dry-run,真工程零写入)/ 计划批准流(preview 卡片字段级
    before/after → 逐项批准/拒绝 → apply_plan 回执 → causedBy=planId 对账 →
    被拒项真相源零落地)/ 插件九步(安装→校验→权限确认→三贡献点生效→越权双拦截
    FORBIDDEN→崩溃隔离→禁用→卸载)/ 标注线程两轮 + session_report 渲染;
    P0 回归断言(文件级):预演后真工程 rev 不变 + oplog 逐文件字节一致 +
    工程父目录零 .cf-scratch 残留(scratch.rs copy_tree 硬链接穿透)。"""
    return _a2_e2e_gate("e2e-ai-native", "tools/e2e_ai_native.py",
                        "脚本三片段预演/批准流 causedBy 对账/插件九步/线程+报告;P0 文件级")


def check_e2e_headless() -> CheckResult:
    """A7(AC-7.4): 纯 CLI 全链 e2e(无 UI 无 serve)——单工程 new→run-script 建卡→
    cutforge-cli clip-update→cutforge-render 直渲→ffprobe 时长+像素抽样;批清单
    3 工程混合布局单排队 + 报告 batch-report.schema.json 生成物对拍;插件服务端面
    (只读插件越权 GUARD_FAILED/FORBIDDEN 零写入、写插件 actor=plugin OpLog 归因);
    watch 模式启动即渲 + 外部改字段自动重渲;ci-example 样例自证。"""
    return _a2_e2e_gate("e2e-headless", "tools/e2e_headless.py",
                        "单链+batch 3 工程+报告 schema+插件面+watch+ci-example")


CHECKS_A7: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "bench-threshold": (check_bench_threshold, True),
    "cargo-clippy": (check_cargo_clippy, True),
    "cargo-test-workspace": (check_cargo_test_workspace, True),
    "e2e-a11y": (check_e2e_a11y, True),
    "e2e-ai-native": (check_e2e_ai_native, True),               # 册七新增(AC-7.3/7.5 判定器)
    "e2e-color": (check_e2e_color, True),
    "e2e-drag-perf": (check_e2e_drag_perf, True),               # --min-fps 55;负载敏感,安静时段复跑
    "e2e-editing-tools": (check_e2e_editing_tools, True),
    "e2e-events": (check_e2e_events, True),
    "e2e-headless": (check_e2e_headless, True),                 # 册七新增(AC-7.4 纯 CLI 全链)
    "e2e-hotkeys": (check_e2e_hotkeys, True),
    "e2e-independence": (check_e2e_independence, True),
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
    "tool-parity": (check_tool_parity, True),                   # 83 工具黄金对拍
}
