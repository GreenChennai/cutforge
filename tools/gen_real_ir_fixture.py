#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""M8-2 门禁夹具生成器:调 CutFlow rs_ir.py build 生成**真实** IR(顶层含 _meta)。

    python tools/gen_real_ir_fixture.py [--check]

CutFlow 仓库定位:CUTFLOW_REPO 环境变量 → cutforge 仓库同级目录的 CutFlow。
夹具落盘 tests/fixtures/real_ir/project.json;--check 校验夹具在位且含 _meta。
退出码:0 成功 / 2 失败。
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

REPO_ROOT = Path(__file__).resolve().parents[1]
OUT_DIR = REPO_ROOT / "tests" / "fixtures" / "real_ir"
OUT_PATH = OUT_DIR / "project.json"


def find_cutflow() -> Path:
    env = os.environ.get("CUTFLOW_REPO")
    if env:
        return Path(env)
    sibling = REPO_ROOT.parent / "CutFlow"
    if (sibling / "skills/cutflow/scripts/rs_ir.py").is_file():
        return sibling
    return sibling


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()

    if args.check:
        if not OUT_PATH.is_file():
            print(f"FAIL: 夹具缺失 {OUT_PATH}", file=sys.stderr)
            return 2
        doc = json.loads(OUT_PATH.read_text(encoding="utf-8"))
        if "_meta" not in doc:
            print("FAIL: 夹具不含顶层 _meta(不是 CutFlow 真实 IR)", file=sys.stderr)
            return 2
        print("OK: 夹具在位且含 _meta")
        return 0

    cutflow = find_cutflow()
    rs_ir = cutflow / "skills/cutflow/scripts/rs_ir.py"
    if not rs_ir.is_file():
        print(f"FAIL: 找不到 rs_ir.py({rs_ir});设 CUTFLOW_REPO 指向 CutFlow 仓库", file=sys.stderr)
        return 2

    # 最小 applied cutlist(keep 两段、真切换 gap≥1s → 产出带 transition 的真实 IR)
    cutlist = {
        "version": 1,
        "source": "01_materials/take1.mp4",
        "detector": {"version": "fixture", "params": {"mode": "fixture"}},
        "cuts": [
            {"id": "c001", "inMs": 4000, "outMs": 5200, "reason": "silence", "conf": 0.95,
             "action": "remove", "note": "fixture", "guard": {"ok": True}, "text": "…"},
        ],
        "keep": [[0, 4000], [5200, 9600]],
        "removedMs": 1200,
        "srcTotalMs": 9600,
        "script": [{"action": "remove", "text": "~~…~~"}],
    }
    with tempfile.TemporaryDirectory() as td:
        tdp = Path(td)
        (tdp / "cutlist.json").write_text(json.dumps(cutlist, ensure_ascii=False, indent=1), encoding="utf-8")
        r = subprocess.run(
            [sys.executable, str(rs_ir), "build", "--from-cutlist", str(tdp / "cutlist.json"),
             "--slug", "real-ir-fixture", "--ratio", "9x16", "--out", str(OUT_PATH)],
            capture_output=True, text=True, encoding="utf-8", errors="replace",
        )
        if r.returncode != 0:
            print(f"FAIL: rs_ir build 退出码 {r.returncode}\n{r.stdout}\n{r.stderr}", file=sys.stderr)
            return 2
    doc = json.loads(OUT_PATH.read_text(encoding="utf-8"))
    assert "_meta" in doc, "rs_ir 产物必须含 _meta"
    print(f"OK: 真实 IR 夹具已生成 {OUT_PATH}(keepSegments={doc['_meta'].get('keepSegments')})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
