#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""CutForge 统一门禁入口。

每个里程碑一个入口：python tools/gates/gate.py M<n> [--check NAME] [--json]
这是唯一的"完成判定"入口，人眼判断不作为通过依据。

结果协议（沿用 CutFlow 既有约定，禁止另立）：
    {"ok": bool, "code": str, "message": str, "data": object}
退出码：0=通过；2=门禁失败（结果不对）；3=前置或环境缺失；4=内部错误。
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Callable

REPO_ROOT = Path(__file__).resolve().parents[2]
CUTFLOW_REPO = Path(os.environ.get("CUTFLOW_REPO", r"E:\平日资料\GitHub\CutFlow"))

REPO_SIZE_LIMIT_BYTES = 300 * 1024 * 1024  # M0-3: clone ≤ 300 MB
TOOLCHAIN_EXPECT = {"moon": "2.3.3", "bun": "1.3.11", "rust": "1.97.0"}

# M0-6: 计划书点名的 12 个 probe 脚本
PROBE_SCRIPTS = [
    "inspect_run.py", "patch_fx3.py", "probe_asr.py", "probe_baseline.py",
    "probe_cards.py", "probe_d3b.py", "probe_fix3.py", "probe_rows.py",
    "probe_sapi.py", "probe_ts.py", "run_diag.py", "sync_check.py",
]

OK = "OK"
GATE_FAILED = "GATE_FAILED"
NO_ENV = "NO_ENV"
INTERNAL = "INTERNAL"


def _out(text: str) -> None:
    """Windows 控制台 cp936 兜底：宁可替换字符也不抛 UnicodeEncodeError。"""
    stream = sys.stdout
    try:
        stream.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[union-attr]
    except Exception:
        pass
    stream.write(text + "\n")


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8", errors="replace")


class CheckResult:
    def __init__(self, name: str, blocking: bool, ok: bool, code: str, message: str, data: dict) -> None:
        self.name = name
        self.blocking = blocking
        self.ok = ok
        self.code = code
        self.message = message
        self.data = data

    def to_dict(self) -> dict:
        return {
            "name": self.name, "blocking": self.blocking, "ok": self.ok,
            "code": self.code, "message": self.message, "data": self.data,
        }


def check_naming() -> CheckResult:
    """M0-1: docs/NAMING.md 存在且含全量实测存档与落位表。"""
    p = REPO_ROOT / "docs" / "NAMING.md"
    if not p.exists():
        return CheckResult("naming", True, False, GATE_FAILED, "docs/NAMING.md 不存在", {})
    text = _read(p)
    required = [
        "cutforge-core", "cutforge-mcp", "cutforge-schema",   # crates.io 4 项(含 cutforge 本名)
        "@cutforge/editor", "@cutforge-app",                  # npm 2 项
        "cutforge-app", "cutforgehq",                         # GitHub 2 项(另含被占的 cutforge)
        "FREE", "TAKEN", "落位", "保留",
    ]
    missing = [k for k in required if k not in text]
    if missing:
        return CheckResult("naming", True, False, GATE_FAILED,
                           f"NAMING.md 缺少关键内容: {missing}", {"missing": missing})
    return CheckResult("naming", True, True, OK, "NAMING.md 存在,含 4+2+2 项实测存档与落位决策(保留 CutForge)",
                       {"file": str(p)})


def check_license() -> CheckResult:
    """M0-2: 三份许可文件齐备与合规 + README 无"停更/archived"错误表述。"""
    problems: list[str] = []
    lic = REPO_ROOT / "LICENSE"
    if not lic.exists():
        problems.append("LICENSE 不存在")
    else:
        t = _read(lic)
        if "Artboard Reciprocal License" not in t:
            problems.append("LICENSE 缺 'Artboard Reciprocal License'")
        if "Version 1.0" not in t:
            problems.append("LICENSE 缺 'Version 1.0'")
        if "GreenChennai" not in t:
            problems.append("LICENSE 缺版权人 GreenChennai")

    mit = REPO_ROOT / "LICENSE-OPENCUT.MIT"
    if not mit.exists():
        problems.append("LICENSE-OPENCUT.MIT 不存在")
    else:
        raw = mit.read_bytes()
        t = _read(mit)
        # 上游原文为短式 MIT(无 "MIT License" 标题行),故以功能性文本+逐字体积判定
        if "Copyright" not in t or "Permission is hereby granted" not in t:
            problems.append("LICENSE-OPENCUT.MIT 缺 MIT 原文要素(Copyright/Permission)")
        if len(raw) != 1060:
            problems.append(f"LICENSE-OPENCUT.MIT 字节数 {len(raw)} != 上游原文 1060(须逐字复制)")

    notice = REPO_ROOT / "NOTICE.md"
    if not notice.exists():
        problems.append("NOTICE.md 不存在")
    else:
        t = _read(notice)
        for kw in ("https://github.com/OpenCut-app/OpenCut",
                   "https://github.com/OpenCut-app/opencut-classic",
                   "逐字复制", "不得也不会被撤回"):
            if kw not in t:
                problems.append(f"NOTICE.md 缺关键内容: {kw}")

    readme = REPO_ROOT / "README.md"
    bad_hits = 0
    if readme.exists():
        bad_hits = len(re.findall(r"停更|archived", _read(readme), flags=re.IGNORECASE))
        if bad_hits:
            problems.append(f"README.md 出现 '停更|archived' 错误表述 {bad_hits} 次(红线 3)")
    else:
        problems.append("README.md 不存在")

    if problems:
        return CheckResult("license", True, False, GATE_FAILED, "; ".join(problems), {"problems": problems})
    return CheckResult("license", True, True, OK,
                       "三份许可文件齐备;上游 MIT 原文逐字保留(1060B);README 无错误表述",
                       {"lic_bytes": lic.stat().st_size if lic.exists() else 0,
                        "mit_bytes": mit.stat().st_size if mit.exists() else 0})


def check_repo_size() -> CheckResult:
    """M0-3: CutFlow 仓库 git clone --depth 1 体积 ≤ 300 MB(实测 clone,非工作区)。"""
    if shutil.which("git") is None:
        return CheckResult("repo-size", True, False, NO_ENV, "未找到 git", {})
    if not (CUTFLOW_REPO / ".git").exists():
        return CheckResult("repo-size", True, False, NO_ENV,
                           f"CutFlow 仓库不存在: {CUTFLOW_REPO}(可设 CUTFLOW_REPO 环境变量)", {})
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-gate-"))
    dst = tmp / "clone"
    try:
        r = subprocess.run(
            ["git", "clone", "--depth", "1", "--no-local", "--quiet", str(CUTFLOW_REPO), str(dst)],
            capture_output=True, text=True, timeout=900,
        )
        if r.returncode != 0:
            return CheckResult("repo-size", True, False, INTERNAL,
                               f"clone 失败: {r.stderr.strip()[:300]}", {})
        total = 0
        for root, _dirs, files in os.walk(dst):
            for f in files:
                fp = Path(root) / f
                try:
                    total += fp.lstat().st_size
                except OSError:
                    pass
        ok = total <= REPO_SIZE_LIMIT_BYTES
        mb = total / 1024 / 1024
        return CheckResult(
            "repo-size", True, ok, OK if ok else GATE_FAILED,
            f"clone --depth 1 体积 {mb:.1f} MB(阈值 ≤300 MB)" + ("" if ok else " —— 超限"),
            {"bytes": total, "limit_bytes": REPO_SIZE_LIMIT_BYTES, "repo": str(CUTFLOW_REPO)},
        )
    except subprocess.TimeoutExpired:
        return CheckResult("repo-size", True, False, INTERNAL, "clone 超时(900s)", {})
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def check_toolchain() -> CheckResult:
    """M0-4: 工具链 pin 与上游一致;Cargo.toml edition=2024, resolver=3。"""
    problems: list[str] = []
    proto = REPO_ROOT / ".prototools"
    if not proto.exists():
        problems.append(".prototools 不存在")
    else:
        text = _read(proto)
        found: dict[str, str] = {}
        for line in text.splitlines():
            m = re.match(r"\s*(moon|bun|rust)\s*=\s*\"([^\"]+)\"", line)
            if m:
                found[m.group(1)] = m.group(2)
        for k, want in TOOLCHAIN_EXPECT.items():
            got = found.get(k)
            if got != want:
                problems.append(f".prototools {k}={got!r} 期望 {want!r}")

    cargo = REPO_ROOT / "Cargo.toml"
    if not cargo.exists():
        problems.append("Cargo.toml 不存在")
    else:
        t = _read(cargo)
        if not re.search(r'edition\s*=\s*"2024"', t):
            problems.append("Cargo.toml 缺 edition = \"2024\"")
        if not re.search(r'resolver\s*=\s*"3"', t):
            problems.append("Cargo.toml 缺 resolver = \"3\"")

    if problems:
        return CheckResult("toolchain", True, False, GATE_FAILED, "; ".join(problems), {"problems": problems})
    return CheckResult("toolchain", True, True, OK,
                       f"rust={TOOLCHAIN_EXPECT['rust']} moon={TOOLCHAIN_EXPECT['moon']} "
                       f"bun={TOOLCHAIN_EXPECT['bun']}; edition=2024; resolver=3", {})


def check_ci() -> CheckResult:
    """M0-5(部分): gate.yml 存在且结构有效;CI 远端全绿须推送后人工确认(观察)。"""
    p = REPO_ROOT / ".github" / "workflows" / "gate.yml"
    if not p.exists():
        return CheckResult("ci", True, False, GATE_FAILED, ".github/workflows/gate.yml 不存在", {})
    t = _read(p)
    missing = [kw for kw in ("name: gate", "jobs:", "gate.py") if kw not in t]
    if missing:
        return CheckResult("ci", True, False, GATE_FAILED, f"gate.yml 缺要素: {missing}", {"missing": missing})
    return CheckResult(
        "ci", True, True, OK,
        "gate.yml 存在且含门禁入口;『CI 远端全绿』需推送后确认(本地无法验证)",
        {"note": "ci-green 远端验证项,推送后在 GitHub Actions 页面确认"},
    )


def check_tests_layout() -> CheckResult:
    """M0-6(观察): CutFlow tests/probes/ 归置 12 个 probe 脚本并标注不参与门禁。"""
    probes = CUTFLOW_REPO / "tests" / "probes"
    if not (CUTFLOW_REPO / ".git").exists():
        return CheckResult("tests-layout", False, False, NO_ENV,
                           f"CutFlow 仓库不存在: {CUTFLOW_REPO}", {})
    if not probes.exists():
        return CheckResult("tests-layout", False, False, GATE_FAILED,
                           f"{probes} 不存在(12 个 probe 脚本未归置)", {})
    names = {p.name for p in probes.glob("*.py")}
    missing = [s for s in PROBE_SCRIPTS if s not in names]
    readme = CUTFLOW_REPO / "tests" / "README.md"
    marked = readme.exists() and "不参与门禁" in _read(readme)
    if missing or not marked:
        return CheckResult("tests-layout", False, False, GATE_FAILED,
                           f"缺失脚本: {missing}; tests/README.md 标注: {marked}",
                           {"missing": missing, "readme_marked": marked})
    return CheckResult("tests-layout", False, True, OK,
                       "12 个 probe 脚本已归入 tests/probes/ 且 README 标注'调试用,不参与门禁'",
                       {"count": len(PROBE_SCRIPTS)})


CHECKS_M0: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "naming": (check_naming, True),
    "license": (check_license, True),
    "repo-size": (check_repo_size, True),
    "toolchain": (check_toolchain, True),
    "ci": (check_ci, True),
    "tests-layout": (check_tests_layout, False),  # 观察
}

MILESTONES: dict[str, dict[str, tuple[Callable[[], CheckResult], bool]]] = {
    "M0": CHECKS_M0,
    # M1+ 在对应里程碑落地时注册
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


if __name__ == "__main__":
    sys.exit(main())
