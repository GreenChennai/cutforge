"""CutForge 统一门禁入口。

每个里程碑一个入口：python tools/gates/gate.py M<n> [--check NAME] [--json]
或等价的模块入口：python -m tools.gates M<n>
这是唯一的"完成判定"入口，人眼判断不作为通过依据。

包结构（A-10 拆包，纯移动）：common = 共享基建；m0…m7 / a1…a7 = 每册一个模块；
本文件聚合 MILESTONES 注册表与 run_milestone/main；tools/gates/gate.py 为兼容入口
（re-export 包的全部公开名，命令行行为与拆分前一致）。

结果协议（沿用 CutFlow 既有约定，禁止另立）：
    {"ok": bool, "code": str, "message": str, "data": object}
退出码：0=通过；2=门禁失败（结果不对）；3=前置或环境缺失；4=内部错误。
"""
from __future__ import annotations

import argparse
import json
import sys
from typing import Callable

from .common import GATE_FAILED, INTERNAL, NO_ENV, OK, CheckResult, _out
from .m0 import CHECKS_M0
from .m1 import CHECKS_M1
from .m2 import CHECKS_M2
from .m3 import CHECKS_M3
from .m4 import CHECKS_M4
from .m5 import CHECKS_M5
from .m6 import CHECKS_M6
from .m7 import CHECKS_M7
from .a1 import CHECKS_A1
from .a2 import CHECKS_A2
from .a3 import CHECKS_A3
from .a4 import CHECKS_A4
from .a5 import CHECKS_A5
from .a6 import CHECKS_A6
from .a7 import CHECKS_A7


MILESTONES: dict[str, dict[str, tuple[Callable[[], CheckResult], bool]]] = {
    "M0": CHECKS_M0,
    "M1": CHECKS_M1,
    "M2": CHECKS_M2,
    "M3": CHECKS_M3,
    "M4": CHECKS_M4,
    "M5": CHECKS_M5,
    "M6": CHECKS_M6,
    "M7": CHECKS_M7,
    "A1": CHECKS_A1,
    "A2": CHECKS_A2,
    "A3": CHECKS_A3,
    "A4": CHECKS_A4,
    "A5": CHECKS_A5,
    "A6": CHECKS_A6,
    "A7": CHECKS_A7,
}


def run_milestone(ms: str, only: str | None) -> tuple[bool, str, dict]:
    checks = MILESTONES.get(ms)
    if checks is None:
        return False, INTERNAL, {"message": f"未知里程碑: {ms}(已注册: {sorted(MILESTONES)})"}

    results: list[CheckResult] = []
    if only:
        entry = checks.get(only)
        if entry is None:
            return False, INTERNAL, {"message": f"未知检查项: {only}(可用: {sorted(checks)})"}
        results.append(entry[0]())
    else:
        for name in sorted(checks):
            results.append(checks[name][0]())

    blocking_fail = [r for r in results if r.blocking and not r.ok]
    env_missing = [r for r in results if r.code == NO_ENV]
    if env_missing:
        # 环境缺失优先上报:不得把"环境缺失"报成 2(结果不对),更不能报成 0
        msg = "环境缺失: " + "; ".join(f"{r.name}: {r.message}" for r in env_missing)
        return False, NO_ENV, {"milestone": ms, "results": [r.to_dict() for r in results], "message": msg}
    if blocking_fail:
        msg = f"{ms} 门禁失败 {len(blocking_fail)} 项: " + "; ".join(r.name for r in blocking_fail)
        return False, GATE_FAILED, {"milestone": ms, "results": [r.to_dict() for r in results], "message": msg}
    obs_fail = [r for r in results if not r.blocking and not r.ok]
    msg = f"{ms} 全部阻断项通过" + (f";观察项未达标 {len(obs_fail)} 项(已记录)" if obs_fail else "")
    return True, OK, {"milestone": ms, "results": [r.to_dict() for r in results], "message": msg}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="CutForge 统一门禁入口")
    parser.add_argument("milestone", help="里程碑,如 M0")
    parser.add_argument("--check", help="只跑指定检查项")
    parser.add_argument("--json", action="store_true", help="输出 JSON 结果协议")
    args = parser.parse_args(argv)

    try:
        ok, code, data = run_milestone(args.milestone, args.check)
    except Exception as exc:  # 未预期错误:统一 INTERNAL,不吞
        ok, code, data = False, INTERNAL, {"message": f"内部错误: {exc!r}"}

    envelope = {"ok": ok, "code": code,
                "message": data.get("message", code), "data": data}
    if args.json:
        _out(json.dumps(envelope, ensure_ascii=False, indent=2))
    else:
        mark = "PASS" if ok else "FAIL"
        _out(f"[{mark}] {args.milestone} code={code}: {envelope['message']}")
        for r in data.get("results", []):
            flag = "✓" if r["ok"] else ("!" if r["blocking"] else "-")
            _out(f"  {flag} {r['name']}{'(阻断)' if r['blocking'] else '(观察)'}: {r['message']}")
    return {OK: 0, GATE_FAILED: 2, NO_ENV: 3, INTERNAL: 4}.get(code, 4)


# 兼容 re-export(A-10):旧 gate.py 单文件时代的全部公开名在包顶层可达。
from . import a1, a2, a3, a4, a5, a6, a7, m0, m1, m2, m3, m4, m5, m6, m7  # noqa: E402

for _mod in (m0, m1, m2, m3, m4, m5, m6, m7, a1, a2, a3, a4, a5, a6, a7):
    for _name, _val in vars(_mod).items():
        if _name.startswith(("check_", "CHECKS_", "RUST_", "JS_", "HTML_", "GLOSSARY_", "GITHUB_")):
            globals().setdefault(_name, _val)
