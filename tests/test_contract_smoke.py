# -*- coding: utf-8 -*-
"""CI 契约烟测:mcp-tools 契约完整、cf_validate 生成物新鲜、门禁文件在位。"""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def test_mcp_tools_contract_complete():
    doc = json.loads((ROOT / "schemas/mcp-tools.json").read_text(encoding="utf-8"))
    assert len(doc["tools"]) >= 27
    for t in doc["tools"]:
        assert t["inputSchema"] and t["outputSchema"], f"{t['name']} 缺 schema"


def test_generated_validator_fresh():
    r = subprocess.run([sys.executable, str(ROOT / "tools/schema_gen.py"), "--check"],
                       capture_output=True, text=True, encoding="utf-8", errors="replace")
    assert r.returncode == 0, "cf_validate.py 生成物过期,重跑 tools/schema_gen.py"


def test_gate_entry_exists():
    assert (ROOT / "tools/gates/gate.py").is_file()
    assert (ROOT / "docs/FLOW.md").is_file()
