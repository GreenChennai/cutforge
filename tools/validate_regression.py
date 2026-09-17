#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""回归集校验入口(计划书 M1-2/G3-2)。

    python tools/validate_regression.py [--json]

对 tests/regression/ 下每个 videoType 样本目录:
  project.json   --migrate(v1→v2) 后校验;
  wordline.json / cutlist.json / notes.json 直接校验;
  oplog.jsonl 逐行校验。
通过率必须 100%(零豁免);任何失败退出码 2。
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT / "tools" / "_generated"))
import cf_validate  # noqa: E402 — 生成物,唯一校验实现

REG_DIR = REPO_ROOT / "tests" / "regression"


def check_one(path: Path, schema: str, migrate: bool) -> dict:
    doc = json.loads(path.read_text("utf-8"))
    if migrate:
        doc = cf_validate.migrate_project_v1_to_v2(doc)
    errors = cf_validate.validate(schema, doc)
    return {"file": str(path.relative_to(REPO_ROOT)), "schema": schema, "ok": not errors,
            "errors": errors}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="回归集 100% 校验")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args(argv)

    results: list[dict] = []
    dirs = sorted(d for d in REG_DIR.iterdir() if d.is_dir() and not d.name.startswith("_"))
    if len(dirs) < 3:
        results.append({"file": str(REG_DIR), "ok": False,
                        "errors": [f"样本目录不足 3 类,实际 {len(dirs)}"]})
    for d in dirs:
        plans = [("project.json", "project", True), ("wordline.json", "wordline", False),
                 ("cutlist.json", "cutlist", False), ("notes.json", "notes", False)]
        for fname, schema, mig in plans:
            p = d / fname
            if not p.exists():
                results.append({"file": str(p), "ok": False, "errors": ["样本缺失"]})
            else:
                results.append(check_one(p, schema, mig))
        oplog = d / "oplog.jsonl"
        if oplog.exists():
            for ln, line in enumerate(oplog.read_text("utf-8").splitlines()):
                if not line.strip():
                    continue
                errors = cf_validate.validate("oplog", json.loads(line))
                results.append({"file": f"{oplog.relative_to(REPO_ROOT)}#{ln}",
                                "schema": "oplog", "ok": not errors, "errors": errors})
        else:
            results.append({"file": str(oplog), "ok": False, "errors": ["样本缺失"]})

    fails = [r for r in results if not r["ok"]]
    total = len(results)
    env = {"ok": not fails, "code": "OK" if not fails else "SCHEMA_INVALID",
           "message": f"回归集 {total - len(fails)}/{total} 通过(要求 100%)",
           "data": {"dirs": [d.name for d in dirs], "total": total,
                    "failed": [r for r in fails]}}
    if args.json:
        print(json.dumps(env, ensure_ascii=False, indent=2))
    else:
        print(env["message"])
        for r in fails:
            print(f"  FAIL {r['file']}: {r['errors'][:5]}")
    return 0 if not fails else 2


if __name__ == "__main__":
    sys.exit(main())
