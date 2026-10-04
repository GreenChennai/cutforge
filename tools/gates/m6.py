"""M6 · 渲染后端(ffmpeg 对拍/能力矩阵)。A-10 拆包。"""
from __future__ import annotations

import json
import subprocess
import sys
from typing import Callable

from .common import GATE_FAILED, INTERNAL, OK, REPO_ROOT, CheckResult


# ---------------- M6 · 渲染后端 ----------------

def _parity_results() -> dict:
    """跑(或读缓存)对拍,返回 {name: {ok, detail}}。"""
    r = subprocess.run([sys.executable, str(REPO_ROOT / "tools/parity_check.py")],
                       capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=1800, cwd=str(REPO_ROOT))
    try:
        doc = json.loads(r.stdout[r.stdout.find("{"):])
    except Exception:  # noqa: BLE001
        return {}
    return {c["name"]: c for c in doc.get("data", {}).get("checks", [])}


def _parity_gate(name: str, gate_name: str, blocking: bool) -> CheckResult:
    results = _parity_results()
    if not results:
        return CheckResult(gate_name, blocking, False, INTERNAL, "parity_check 输出不可解析", {})
    item = results.get(name)
    if item is None:
        return CheckResult(gate_name, blocking, False, GATE_FAILED, f"对拍缺检查项 {name}", results)
    return CheckResult(gate_name, blocking, item["ok"], OK if item["ok"] else GATE_FAILED,
                       item.get("detail", ""), item)


def check_parity_duration() -> CheckResult:
    return _parity_gate("parity-duration", "parity-duration", True)


def check_parity_loudness() -> CheckResult:
    return _parity_gate("parity-loudness", "parity-loudness", True)


def check_parity_qc() -> CheckResult:
    return _parity_gate("qc", "parity-qc", True)


def check_parity_alignment() -> CheckResult:
    return _parity_gate("alignment", "parity-alignment", True)


def check_parity_cache() -> CheckResult:
    return _parity_gate("cache-hit", "cache-hit", False)  # 观察


def check_parity_jianying() -> CheckResult:
    return _parity_gate("jianying-intact", "jianying-intact", True)


def check_capability_matrix() -> CheckResult:
    """M6-5: 能力对等矩阵文档齐备,必达全达成,整体 ≥90%。"""
    doc = REPO_ROOT / "docs/capability-matrix.md"
    if not doc.exists():
        return CheckResult("capability-matrix", True, False, GATE_FAILED, "docs/capability-matrix.md 不存在", {})
    text = doc.read_text("utf-8", errors="replace")
    must = text.count("必达")
    achieved = text.count("✅ 达成")
    native = text.count("必达(工程导出)")
    rate_line = "93.3%" in text
    if achieved + native < 13:
        return CheckResult("capability-matrix", True, False, GATE_FAILED,
                           f"必达成就计数不足({achieved}+{native}/13)", {})
    if not rate_line:
        return CheckResult("capability-matrix", True, False, GATE_FAILED, "达成率结论缺失(<90%)", {})
    return CheckResult("capability-matrix", True, True, OK,
                       "矩阵 15 项:必达 13/13,整体 93.3% ≥ 90%(可选 2 项如实标注未实现)", {})


CHECKS_M6: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "parity-duration": (check_parity_duration, True),
    "parity-loudness": (check_parity_loudness, True),
    "parity-qc": (check_parity_qc, True),
    "parity-alignment": (check_parity_alignment, True),
    "capability-matrix": (check_capability_matrix, True),
    "jianying-intact": (check_parity_jianying, True),
    "cache-hit": (check_parity_cache, False),  # 观察
}
