"""M3 · 双向同步与标注(合并/回放/锚点/阶段脏)。A-10 拆包。"""
from __future__ import annotations

import json
from typing import Callable

from .common import GATE_FAILED, INTERNAL, OK, CheckResult
from .m2 import _cargo


# ---------------- M3 · 双向同步与标注 ----------------

def _cargo_test_gate(name: str, package: str, test: str) -> CheckResult:
    r = _cargo(["test", "-p", package, "--test", test, "--quiet"])
    if r.returncode != 0:
        return CheckResult(name, True, False, GATE_FAILED, f"{test} 失败: {r.stdout[-300:]}", {})
    return CheckResult(name, True, True, OK, f"{test} 通过", {})


def check_sync_latency() -> CheckResult:
    """M3-1: 同步往返延迟(AI 可见 P95 ≤100ms;可感知 P95 ≤200ms)。"""
    r = _cargo(["bench", "-p", "cutforge-io", "--bench", "roundtrip", "--", "--json"])
    try:
        data = json.loads(r.stdout[r.stdout.find("{"):])
    except Exception:  # noqa: BLE001
        return CheckResult("sync-latency", True, False, INTERNAL, f"bench 输出不可解析: {r.stdout[-200:]}", {})
    ok = r.returncode == 0 and data.get("ok") is True
    return CheckResult("sync-latency", True, ok, OK if ok else GATE_FAILED,
                       data.get("message", "bench 失败"), data.get("data", {}))


def check_merge_property() -> CheckResult:
    """M3-2: 冲突零静默覆盖(N ≥ 10,000 组)。"""
    return _cargo_test_gate("merge-property", "cutforge-core", "merge_property")


def check_oplog_replay() -> CheckResult:
    """M3-3: OpLog 回放等价。"""
    return _cargo_test_gate("oplog-replay", "cutforge-core", "oplog_replay_hash_eq")


def check_merge_table() -> CheckResult:
    """M3-4: 三路合并表 9 行全覆盖。"""
    return _cargo_test_gate("merge-table", "cutforge-core", "merge_table")


def check_notes_anchor() -> CheckResult:
    """M3-5: 标注锚点重定位(100 组;无静默丢失)。"""
    return _cargo_test_gate("notes-anchor", "cutforge-core", "notes_anchor")


def check_workspace_rebuildable() -> CheckResult:
    """M3-6: .cutforge 可重建(工程内容哈希不变)。"""
    return _cargo_test_gate("workspace-rebuildable", "cutforge-io", "workspace_state_rebuildable")


def check_stage_dirty() -> CheckResult:
    """M3-7: 与阶段缓存衔接正确(project.json→S3+;subtitles.ass→仅 S8)。"""
    return _cargo_test_gate("stage-dirty", "cutforge-io", "stage_dirty_propagation")


def check_e2e_note_cli() -> CheckResult:
    """M3-8: 标注全链路(CLI 级,可复现且幂等)。"""
    return _cargo_test_gate("e2e-note-cli", "cutforge-cli", "e2e_note_cli")


CHECKS_M3: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "sync-latency": (check_sync_latency, True),
    "merge-property": (check_merge_property, True),
    "oplog-replay": (check_oplog_replay, True),
    "merge-table": (check_merge_table, True),
    "notes-anchor": (check_notes_anchor, True),
    "workspace-rebuildable": (check_workspace_rebuildable, True),
    "stage-dirty": (check_stage_dirty, True),
    "e2e-note-cli": (check_e2e_note_cli, True),
}
