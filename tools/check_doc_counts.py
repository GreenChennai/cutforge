#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""文档工具数 ↔ schemas/mcp-tools.json 机械对拍(B7 治本;副文档 05 T3-1)。

    python tools/check_doc_counts.py [--json]

schema 是唯一真相源(tools 数组按 kind 分类计数,38 = 13 查询 + 18 写 + 7 编排)。
「当前口径」文档(README / docs/FLOW / docs/ACCEPTANCE / docs/capability-matrix)中
出现的工具数声明,正则抽取后逐个与实数比对,文档数字漂移即失败:

  · 全形态:  「N 工具 = A 查询 + B 写 + C 编排」—— 四个数字全部比对;
              兼容「工具数 N(A 查询+B 写+C 编排)」括号形;
  · 独立形:  「N 工具」「工具数 N」—— 须等于总数,除非处于**历史语境**
              (同一行内邻近「时点 / 当时」,或前置「新增」,如
              「28 工具为 M4 时点」「阶段二新增 4 工具后为 38」——历史叙述不算现口径);
  · 豁免面:  V2-PROGRESS / ITERATION-PLAN-v2.0 是只增不改的历史台账(34→38 的
              演进记录在案属合法),整文件不扫;改「现口径」请改 README/FLOW/ACCEPTANCE。

退出码:0 一致 / 1 漂移(逐条列出 文件:行 → 文档数字 vs schema 实数)。
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

REPO = Path(__file__).resolve().parents[1]
SCHEMA = REPO / "schemas" / "mcp-tools.json"

# 「当前口径」文档面;历史台账(V2-PROGRESS/ITERATION-PLAN)整文件豁免
SCAN_FILES = [
    REPO / "README.md",
    REPO / "docs" / "FLOW.md",
    REPO / "docs" / "ACCEPTANCE.md",
    REPO / "docs" / "capability-matrix.md",
]
EXEMPT = {"docs/V2-PROGRESS.md", "docs/ITERATION-PLAN-v2.0.md"}

FULL_FORMS = [
    # N 工具 = A 查询 + B 写 + C 编排(容忍全角与 ** 加粗)
    re.compile(r"(\d+)\s*工具\s*[=＝:：]\s*\*{0,2}(\d+)\*{0,2}\s*查询\s*[+＋]\s*"
               r"\*{0,2}(\d+)\*{0,2}\s*写\s*[+＋]\s*\*{0,2}(\d+)\*{0,2}\s*编排"),
    # 工具数 N(A 查询+B 写+C 编排)
    re.compile(r"工具数\s*(\d+)\s*[（(]\s*(\d+)\s*查询\s*[+＋]\s*(\d+)\s*写\s*[+＋]\s*(\d+)\s*编排\s*[）)]"),
]
BARE_FORM = re.compile(r"(\d+)\s*工具|工具数\s*(\d+)")
HIST_AFTER = re.compile(r"时点|当时")
HIST_BEFORE = re.compile(r"新增\s*$")


def schema_counts() -> dict:
    doc = json.loads(SCHEMA.read_text(encoding="utf-8"))
    tools = doc.get("tools", [])
    by: dict[str, int] = {}
    for t in tools:
        k = str(t.get("kind", "?"))
        by[k] = by.get(k, 0) + 1
    return {"total": len(tools), "by": by, "doc": str(doc.get("_doc", ""))}


def _historical(line: str, start: int, end: int) -> bool:
    """历史语境判定:数字之后 20 字内出现「时点/当时」,或紧邻前文是「新增」。"""
    after = line[end:end + 20]
    before = line[max(0, start - 12):start]
    return bool(HIST_AFTER.search(after) or HIST_BEFORE.search(before))


def scan_docs(real: dict) -> list[dict]:
    """返回漂移清单:[{file, line, claim, expect}]。"""
    problems: list[dict] = []
    total, by = real["total"], real["by"]
    expect_full = (total, by.get("query", 0), by.get("write", 0), by.get("orchestrate", 0))
    for path in SCAN_FILES:
        if not path.is_file():
            problems.append({"file": str(path.relative_to(REPO)), "line": 0,
                             "claim": "文件缺失", "expect": "当前口径文档必须在"})
            continue
        rel = path.relative_to(REPO).as_posix()
        if rel in EXEMPT:
            continue
        for no, line in enumerate(path.read_text("utf-8", errors="replace").splitlines(), 1):
            consumed: list[tuple[int, int]] = []
            for pat in FULL_FORMS:
                for m in pat.finditer(line):
                    consumed.append((m.start(), m.end()))
                    got = tuple(int(x) for x in m.groups())
                    if got != expect_full:
                        problems.append({"file": rel, "line": no,
                                         "claim": m.group(0).strip(),
                                         "expect": f"{expect_full[0]} 工具 = {expect_full[1]} 查询 + "
                                                   f"{expect_full[2]} 写 + {expect_full[3]} 编排"})
            for m in BARE_FORM.finditer(line):
                if any(s <= m.start() < e for s, e in consumed):
                    continue
                n = int(m.group(1) or m.group(2))
                if n != total and not _historical(line, m.start(), m.end()):
                    problems.append({"file": rel, "line": no, "claim": m.group(0).strip(),
                                     "expect": f"{total} 工具(schema 实数;历史叙述请写明时点)"})
    return problems


def main() -> int:
    ap = argparse.ArgumentParser(description="文档工具数 ↔ schema 对拍(B7 治本)")
    ap.add_argument("--json", action="store_true", help="机器可读输出(门禁透传用)")
    a = ap.parse_args()

    real = schema_counts()
    # schema 自证:_doc 头的口径声明必须与 tools 数组一致(schema 内部也不许漂)
    self_problems: list[str] = []
    m = FULL_FORMS[0].search(real["doc"])
    if m is None:
        self_problems.append("schemas/mcp-tools.json 的 _doc 缺「N 工具 = A 查询 + B 写 + C 编排」口径声明")
    elif tuple(int(x) for x in m.groups()) != (
            real["total"], real["by"].get("query", 0), real["by"].get("write", 0),
            real["by"].get("orchestrate", 0)):
        self_problems.append(f"_doc 口径({m.group(0).strip()})与 tools 数组实数不一致")

    problems = scan_docs(real)
    for s in self_problems:
        problems.insert(0, {"file": "schemas/mcp-tools.json", "line": 0,
                            "claim": s, "expect": "_doc 与 tools 数组一致"})

    summary = {"schema": {"total": real["total"], "by": real["by"]},
               "scanned": [str(p.relative_to(REPO)) for p in SCAN_FILES if p.is_file()],
               "problems": problems, "ok": not problems}
    if a.json:
        print(json.dumps(summary, ensure_ascii=False))
    else:
        for p in problems:
            print(f"[DRIFT] {p['file']}:{p['line']}  文档写「{p['claim']}」  期望「{p['expect']}」")
        if not problems:
            print(f"[OK] 文档工具数与 schema 一致:{real['total']} 工具 = "
                  f"{real['by'].get('query', 0)} 查询 + {real['by'].get('write', 0)} 写 + "
                  f"{real['by'].get('orchestrate', 0)} 编排;扫描 {len(summary['scanned'])} 个文件")
    return 0 if not problems else 1


if __name__ == "__main__":
    sys.exit(main())
