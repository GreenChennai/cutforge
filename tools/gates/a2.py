"""A2 · 册二「前端壳重写」册级门禁(D-A2 注册制,T2.6)。A-10 拆包。"""
from __future__ import annotations

import json
import subprocess
import sys
from typing import Callable

from .common import GATE_FAILED, INTERNAL, NO_ENV, OK, REPO_ROOT, CheckResult
from .a1 import (
    check_cargo_clippy,
    check_cargo_test_workspace,
    check_e2e_events,
    check_e2e_static,
    check_pytest_suite,
    check_tool_parity,
)
from .m2 import _cargo


# ---------------- A2 · 册二「前端壳重写」册级门禁(D-A2 注册制,T2.6) ----------------
# 与 A1 同风格:阻断/观察分级,聚合本册验收;与 M0-M7、A1 互不干扰(CI 只跑 M0/M1,
# A2 本机册收官跑)。裸 fetch 检查已并入 shell-purity-v2(判定器 R3),故不单列。

JS_LINE_LIMIT = 400    # AC-2.6 红线:js 单文件 ≤400 行
HTML_LINE_LIMIT = 120  # AC-2.6 红线:index.html ≤120 行


def check_js_line_limit() -> CheckResult:
    """A2(AC-2.6): 壳行数红线——apps/web 下 js 单文件 ≤400 行 + index.html ≤120 行
    (内联实现;min.* 产物不扫;legacy/ 已于册三收尾删除,扫描面=apps/web 全量,无豁免)。"""
    root = REPO_ROOT / "apps" / "web"
    if not root.is_dir():
        return CheckResult("js-line-limit", True, False, NO_ENV, "apps/web 不存在", {})
    offenders: list[tuple[str, int]] = []
    scanned = 0
    for p in root.rglob("*.js"):
        rel = p.relative_to(root)
        if p.name.startswith("min."):
            continue
        try:
            with p.open("r", encoding="utf-8", errors="replace") as f:
                n = sum(1 for _ in f)
        except OSError:
            continue
        scanned += 1
        if n > JS_LINE_LIMIT:
            offenders.append((str(rel), n))
    html = root / "index.html"
    html_n = 0
    if html.exists():
        with html.open("r", encoding="utf-8", errors="replace") as f:
            html_n = sum(1 for _ in f)
        if html_n > HTML_LINE_LIMIT:
            offenders.append(("index.html", html_n))
    if offenders:
        offenders.sort(key=lambda x: -x[1])
        head = "; ".join(f"{f}:{n}" for f, n in offenders[:6])
        return CheckResult("js-line-limit", True, False, GATE_FAILED,
                           f"{len(offenders)} 个文件超红线(js≤{JS_LINE_LIMIT}/html≤{HTML_LINE_LIMIT}): {head}",
                           {"offenders": offenders})
    return CheckResult("js-line-limit", True, True, OK,
                       f"apps/web js {scanned} 文件(max ≤{JS_LINE_LIMIT})+ index.html({html_n}/{HTML_LINE_LIMIT} 行)"
                       " 全部在红线内(全量扫描,无 legacy 豁免)",
                       {"scanned": scanned, "index_html": html_n})


def check_shell_purity_v2() -> CheckResult:
    """A2(T2.6/D-B3/ADR-0013): 壳纯度 v2——语义禁令 + 投影只读 + 禁裸 fetch(全量 js 面)。
    判定器:cutforge-cli check-shell-purity(v2,与 M5-5 同一实现);裸 fetch 并入本项(R3),
    A2 不另单列 fetch 检查;R4 legacy 豁免已随 legacy/ 删除收口(册三,A2-L2 了断)。"""
    r = _cargo(["run", "-q", "-p", "cutforge-cli", "--", "check-shell-purity", "--json"])
    try:
        data = json.loads(r.stdout[r.stdout.find("{"):])
    except Exception:  # noqa: BLE001
        return CheckResult("shell-purity-v2", True, False, INTERNAL, f"CLI 输出不可解析: {r.stdout[-200:]}", {})
    ok = r.returncode == 0 and data.get("ok") is True
    return CheckResult("shell-purity-v2", True, ok, OK if ok else GATE_FAILED,
                       data.get("message", "check-shell-purity v2 失败"), data.get("data", {}))


def _a2_e2e_gate(name: str, rel: str, note: str, *args: str) -> CheckResult:
    """A2 新 e2e 门禁:脚本已落库 → 真跑(阻断);未落库 → SKIP(观察,不冒充通过),
    注明待并行代理落地、主控终验统一跑。命令契约:python <script> [args],exit 0 = 过。"""
    script = REPO_ROOT / rel
    if not script.exists():
        return CheckResult(name, False, False, GATE_FAILED,
                           f"SKIP(观察): {rel} 尚未落库(待并行代理落地,主控终验统一跑)",
                           {"skip": True, "reason": note})
    r = subprocess.run([sys.executable, str(script), *args], capture_output=True, text=True,
                       encoding="utf-8", errors="replace", timeout=1800, cwd=str(REPO_ROOT))
    tail = ((r.stdout or "") + (r.stderr or "")).strip().splitlines()[-3:]
    if r.returncode != 0:
        return CheckResult(name, True, False, GATE_FAILED,
                           f"{script.name} 失败(exit={r.returncode}): {' | '.join(tail)}", {})
    return CheckResult(name, True, True, OK, f"{script.name} 断言链全过", {"tail": tail})


def check_e2e_ui_smoke() -> CheckResult:
    """A2(AC-2.7/T2.7): UI 冒烟 e2e(data-testid 驱动;断言以服务端状态 rev/盘面为准)。"""
    return _a2_e2e_gate("e2e-ui-smoke", "tools/e2e_ui_smoke.py", "T2.7 并行落地")


def check_e2e_playback_survival() -> CheckResult:
    """A2(T2.4): 播放存活 e2e(播放中改工程不销毁媒体元素/事件不丢/mediaStore 增量)。"""
    return _a2_e2e_gate("e2e-playback-survival", "tools/e2e_playback_survival.py", "T2.4 并行落地")


def check_e2e_perf_timeline() -> CheckResult:
    """A2(AC-2.8/T2.9): 时间线性能 e2e(--min-fps 55)。
    机器负载敏感:负载抖动时于安静时段复跑(与 bench 同策);不进 CI(共享 runner 抖动假红)。"""
    return _a2_e2e_gate("e2e-perf-timeline", "tools/e2e_perf_timeline.py",
                        "--min-fps 55;负载抖动时安静时段复跑(与 bench 同策)",
                        "--min-fps", "55")


CHECKS_A2: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "cargo-clippy": (check_cargo_clippy, True),
    "cargo-test-workspace": (check_cargo_test_workspace, True),
    "e2e-events": (check_e2e_events, True),
    "e2e-perf-timeline": (check_e2e_perf_timeline, True),   # 未落库前 SKIP 观察(T2.9 并行)
    "e2e-playback-survival": (check_e2e_playback_survival, True),  # 未落库前 SKIP 观察(T2.4 并行)
    "e2e-static": (check_e2e_static, True),
    "e2e-ui-smoke": (check_e2e_ui_smoke, True),             # 未落库前 SKIP 观察(T2.7 并行)
    "js-line-limit": (check_js_line_limit, True),
    # legacy-reminder 观察项已随 legacy/ 删除移除(册三收尾,A2-L2 了断:R4 豁免同步收口)
    "pytest-suite": (check_pytest_suite, True),
    "shell-purity-v2": (check_shell_purity_v2, True),       # 含裸 fetch(R3),不单列
    "tool-parity": (check_tool_parity, True),
}
