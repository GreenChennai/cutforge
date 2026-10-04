"""M4 · MCP 与脚本(工具集/协议/沙箱/桥体检/文档工具数对拍)。A-10 拆包。"""
from __future__ import annotations

import json
import shutil
import subprocess
import sys
from typing import Callable

from .common import CUTFLOW_REPO, GATE_FAILED, INTERNAL, NO_ENV, OK, REPO_ROOT, CheckResult
from .m1 import _run_tool
from .m2 import _cargo
from .m3 import _cargo_test_gate


# ---------------- M4 · MCP 与脚本 ----------------

def check_mcp_tools() -> CheckResult:
    """M4-1: 工具集完整率(契约比对 + 双通道一致性)。"""
    r = _cargo(["run", "-q", "-p", "cutforge-mcp", "--bin", "cutforge-mcp", "--", "inspect", "--json"])
    try:
        data = json.loads(r.stdout[r.stdout.find("{"):])
    except Exception:  # noqa: BLE001
        return CheckResult("mcp-tools", True, False, INTERNAL, f"inspect 输出不可解析: {r.stdout[-200:]}", {})
    tools = data.get("data", {}).get("tools", [])
    channels = data.get("data", {}).get("channels", {})
    problems: list[str] = []
    if len(tools) < 27:
        problems.append(f"工具数 {len(tools)} < 27(计划书 5.2 全表)")
    for t in tools:
        if not (t.get("inputSchema") and t.get("outputSchema")):
            problems.append(f"{t.get('name')} 缺 inputSchema/outputSchema")
    stdio, http = channels.get("stdio", []), channels.get("embedded-http", [])
    if stdio != http:
        problems.append("stdio 与内嵌通道工具集不一致")
    # 与 schemas/mcp-tools.json 契约逐名比对
    contract = json.loads((REPO_ROOT / "schemas" / "mcp-tools.json").read_text("utf-8"))
    want = [t["name"] for t in contract["tools"]]
    got = [t["name"] for t in tools]
    if want != got:
        problems.append("注册表与 mcp-tools.json 契约不一致")
    if problems:
        return CheckResult("mcp-tools", True, False, GATE_FAILED, "; ".join(problems), {"problems": problems})
    return CheckResult("mcp-tools", True, True, OK,
                       f"{len(tools)} 工具 100% 实现,双 schema 齐备,双通道工具集一致", {"count": len(tools)})


def check_mcp_e2e_visible() -> CheckResult:
    """M4-2: AI 改一处 → 编辑器可见(≤1s,可定位字段)。"""
    return _cargo_test_gate("mcp-e2e-visible", "cutforge-mcp", "e2e_ai_edit_visible")


def check_mcp_note_loop() -> CheckResult:
    """M4-3: 标注全链路闭环(≤3s)。"""
    return _cargo_test_gate("mcp-note-loop", "cutforge-mcp", "e2e_note_loop")


def check_sandbox_escape() -> CheckResult:
    """M4-4: 沙箱无逃逸(越界读/子进程/socket 全拒,成功逃逸数=0)。"""
    return _cargo_test_gate("sandbox-escape", "cutforge-script", "sandbox_escape")


def check_protocol() -> CheckResult:
    """M4-5: 结果协议一致性(全工具 envelope + 5.4 错误码表)。"""
    return _cargo_test_gate("protocol", "cutforge-mcp", "protocol_conformance")


def check_bridge_doctor() -> CheckResult:
    """M4-6: 四个桥脚本被 rs_doctor 识别且正常;README 速查表已登记。"""
    if not (CUTFLOW_REPO / ".git").exists():
        return CheckResult("bridge-doctor", True, False, NO_ENV, f"CutFlow 仓库不存在: {CUTFLOW_REPO}", {})
    r = subprocess.run(
        [sys.executable, str(CUTFLOW_REPO / "skills/cutflow/scripts/rs_doctor.py")],
        capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=300, cwd=str(CUTFLOW_REPO),
    )
    try:
        out = json.loads(r.stdout[r.stdout.find("{"):])
    except Exception:  # noqa: BLE001
        return CheckResult("bridge-doctor", True, False, INTERNAL, f"rs_doctor 输出不可解析: {r.stdout[-200:]}", {})
    checks = out.get("data", {}).get("checks", [])
    bridges = {c["name"]: c["ok"] for c in checks if c.get("group") == "CutForge 桥"}
    expected = [f"CutForge 桥:rs_{n}.py" for n in ("editor", "notes", "oplog", "gate")]
    problems = [n for n in expected if not bridges.get(n)]
    readme = (CUTFLOW_REPO / "README.md").read_text("utf-8", errors="replace")
    registered = sum(1 for n in ("rs_editor.py", "rs_notes.py", "rs_oplog.py", "rs_gate.py") if n in readme)
    if problems:
        return CheckResult("bridge-doctor", True, False, GATE_FAILED,
                           f"桥脚本未识别或未通过: {problems}", {"bridges": bridges})
    if registered < 4:
        return CheckResult("bridge-doctor", True, False, GATE_FAILED,
                           f"README 速查表登记 {registered}/4", {"registered": registered})
    return CheckResult("bridge-doctor", True, True, OK,
                       "四个桥脚本被 doctor 识别且 --probe 全过;README 速查表登记 4/4",
                       {"bridges": bridges, "registered": registered})


def check_doc_tool_counts() -> CheckResult:
    """T3-1(副文档 05):文档工具数 ↔ schemas/mcp-tools.json 机械对拍(B7 治本)。
    README/FLOW/ACCEPTANCE/capability-matrix 的「N 工具 = A 查询 + B 写 + C 编排」
    与独立「N 工具」数字,逐个与 schema 实数比对(历史时点叙述豁免)。"""
    rc, data = _run_tool("tools/check_doc_counts.py", "--json")
    if rc != 0 and "problems" not in data:
        return CheckResult("doc-tool-counts", True, False, INTERNAL,
                           f"check_doc_counts.py 运行失败(exit={rc}): {str(data)[-200:]}", {})
    if rc != 0:
        claims = "; ".join(f"{p.get('file')}:{p.get('line')} 「{p.get('claim')}」→ {p.get('expect')}"
                           for p in data.get("problems", [])[:6])
        return CheckResult("doc-tool-counts", True, False, GATE_FAILED,
                           f"文档工具数漂移(B7): {claims}", data)
    s = data.get("schema", {})
    return CheckResult("doc-tool-counts", True, True, OK,
                       "文档工具数与 schemas/mcp-tools.json 一致:"
                       f"{s.get('total')} = {s.get('by', {}).get('query')}+"
                       f"{s.get('by', {}).get('write')}+{s.get('by', {}).get('orchestrate')}", data)


CHECKS_M4: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "mcp-tools": (check_mcp_tools, True),
    "doc-tool-counts": (check_doc_tool_counts, True),
    "mcp-e2e-visible": (check_mcp_e2e_visible, True),
    "mcp-note-loop": (check_mcp_note_loop, True),
    "sandbox-escape": (check_sandbox_escape, True),
    "protocol": (check_protocol, True),
    "bridge-doctor": (check_bridge_doctor, True),
}
