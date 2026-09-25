# -*- coding: utf-8 -*-
"""M9-4 门禁:四桥冒烟(对真实 CutFlow 脚本跑,弱依赖:无 CutFlow 仓库时 skip,
CI python-gates job 已浅克隆 CutFlow 并设 CUTFLOW_REPO)。"""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
CUTFLOW = Path(os.environ.get("CUTFLOW_REPO", ROOT.parent / "CutFlow"))
SCRIPTS = CUTFLOW / "skills" / "cutflow" / "scripts"

pytestmark = pytest.mark.skipif(
    not (SCRIPTS / "rs_editor.py").is_file(),
    reason="CutFlow 仓库不可用(CUTFLOW_REPO 未设且无同级目录)",
)


def run_bridge(script: str, *args: str) -> dict:
    r = subprocess.run(
        [sys.executable, str(SCRIPTS / script), *args],
        capture_output=True, text=True, encoding="utf-8", errors="replace",
        env={**os.environ, "CUTFORGE_REPO": str(ROOT), "PYTHONUTF8": "1"},
    )
    assert r.returncode == 0, f"{script} 退出码 {r.returncode}: {r.stdout} {r.stderr}"
    text = r.stdout
    return json.loads(text[text.index("{"):])


def test_editor_timeline_id_fallback_on_real_ir(tmp_path):
    """真实 IR 夹具(无 clip id)→ timeline 不再输出 null,回退 id 带 idSource。"""
    proj = ROOT / "tests" / "fixtures" / "real_ir" / "project.json"
    assert proj.is_file()
    ws = tmp_path / "ws"
    # 目录契约 v2(0.5.0):工程目录中文化,cutforge 与 CutFlow 两侧同表
    # (cutforge crates/cutforge-io/src/paths.rs ↔ CutFlow rs_paths.py)。
    ir = ws / "05_时间线工程"
    ir.mkdir(parents=True)
    (ir / "project.json").write_text(proj.read_text(encoding="utf-8"), encoding="utf-8")
    out = run_bridge("rs_editor.py", "timeline", str(ws))
    rows = (out.get("data") or {}).get("clips") or []
    assert rows, f"timeline 必须有行: {out}"
    assert all(r.get("id") not in (None, "") for r in rows), "不得再输出 null id"
    assert any(r.get("idSource") == "fallback" for r in rows), "真实 IR 无 clip id,必须出现 fallback"
    assert all(r.get("track") for r in rows), "track 不得为 null"


def test_gate_probe_detects_missing_and_present():
    if "CUTFORGE_REPO" not in os.environ:
        os.environ["CUTFORGE_REPO"] = str(ROOT)
    out = run_bridge("rs_gate.py", "--probe")
    assert out["ok"] is True and "gate.py" in out["message"]


def test_oplog_bridge_protocol():
    out = run_bridge("rs_oplog.py", "--probe")
    assert out["ok"] is True


def test_notes_bridge_protocol():
    out = run_bridge("rs_notes.py", "--probe")
    assert out["ok"] is True
