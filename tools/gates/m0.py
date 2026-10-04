"""M0 · 合规立项(命名/许可/仓体积/工具链/CI 骨架/probe 归置)。A-10 拆包,函数原样移入。"""
from __future__ import annotations

import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Callable

from .common import (
    CUTFLOW_REPO,
    GATE_FAILED,
    INTERNAL,
    NO_ENV,
    OK,
    PROBE_SCRIPTS,
    REPO_ROOT,
    REPO_SIZE_LIMIT_BYTES,
    TOOLCHAIN_EXPECT,
    CheckResult,
    _read,
)

# G-5/BUG-15: CORE-FILES 清单路径单点(反面测试 TC-GATE-001 通过替换本路径注入缺陷)。
CORE_FILES_PATH = REPO_ROOT / "CORE-FILES"


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

    # BUG-15/G-5(审查报告 v2): CORE-FILES 清单存在性校验——清单腐化必须显式红灯,
    # 而非静默失效。反面测试 TC-GATE-001(tools/gates/negative_tests.py)。
    if not CORE_FILES_PATH.exists():
        problems.append("CORE-FILES 不存在")
    else:
        missing_entries = [
            entry for entry in (line.strip() for line in _read(CORE_FILES_PATH).splitlines())
            if entry and not entry.startswith("#") and not (REPO_ROOT / entry).exists()
        ]
        if missing_entries:
            problems.append(f"CORE-FILES 条目不存在: {missing_entries}")

    if problems:
        return CheckResult("license", True, False, GATE_FAILED, "; ".join(problems), {"problems": problems})
    return CheckResult("license", True, True, OK,
                       "三份许可文件齐备;上游 MIT 原文逐字保留(1060B);README 无错误表述;"
                       "CORE-FILES 清单条目全部存在(存在性校验生效)",
                       {"lic_bytes": lic.stat().st_size if lic.exists() else 0,
                        "mit_bytes": mit.stat().st_size if mit.exists() else 0})


def check_repo_size() -> CheckResult:
    """M0-3: CutFlow 仓库 git clone --depth 1 体积 ≤ 300 MB(实测 clone,非工作区)。"""
    if shutil.which("git") is None:
        return CheckResult("repo-size", True, False, NO_ENV, "未找到 git", {})
    if not (CUTFLOW_REPO / ".git").exists():
        return CheckResult("repo-size", True, False, NO_ENV,
                           f"CutFlow 仓库不存在: {CUTFLOW_REPO}(可设 CUTFLOW_REPO 环境变量)", {})
    shallow = subprocess.run(["git", "-C", str(CUTFLOW_REPO), "rev-parse", "--is-shallow-repository"],
                             capture_output=True, text=True).stdout.strip() == "true"
    if shallow:
        # CI 场景:checkout 是浅克隆,无法二次 clone;改用 pack 体积近似(等价于网络传输量)
        r = subprocess.run(["git", "-C", str(CUTFLOW_REPO), "count-objects", "-v"],
                           capture_output=True, text=True, encoding="utf-8", errors="replace" )
        info = dict(line.split(": ") for line in r.stdout.strip().splitlines() if ": " in line)
        total = int(info.get("size-pack", "0")) * 1024  # KB → B
        ok = total <= REPO_SIZE_LIMIT_BYTES
        mb = total / 1024 / 1024
        return CheckResult("repo-size", True, ok, OK if ok else GATE_FAILED,
                           f"浅克隆源:pack 体积 {mb:.1f} MB(阈值 ≤300 MB)",
                           {"bytes": total, "mode": "size-pack"})
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-gate-"))
    dst = tmp / "clone"
    try:
        r = subprocess.run(
            ["git", "clone", "--depth", "1", "--no-local", "--quiet", str(CUTFLOW_REPO), str(dst)],
            capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=900,
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
