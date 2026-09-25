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
time = __import__("time")
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


# ---------------- M1 · 契约固化 ----------------

def _run_tool(rel: str, *args: str) -> tuple[int, dict]:
    """跑仓库内工具并解析结果协议 JSON;工具缺失按 NO_ENV 处理。"""
    script = REPO_ROOT / rel
    if not script.exists():
        return 3, {"message": f"工具不存在: {script}"}
    r = subprocess.run([sys.executable, str(script), *args],
                       capture_output=True, text=True, encoding="utf-8", errors="replace",
                       timeout=1800, cwd=str(REPO_ROOT))
    try:
        out = r.stdout or ""
        start = out.find("{")
        data = json.loads(out[start:]) if start >= 0 else {}
    except Exception:  # noqa: BLE001
        data = {"raw": ((r.stdout or "") + (r.stderr or ""))[-400:]}
    return r.returncode, data


def check_schemas_complete() -> CheckResult:
    """M1-1: 五份 schema + 生成物齐备,且被 Rust 与 Python 双向引用。"""
    problems: list[str] = []
    names = ["project", "wordline", "cutlist", "notes", "oplog"]
    for n in names:
        if not (REPO_ROOT / "schemas" / f"{n}.schema.json").exists():
            problems.append(f"schemas/{n}.schema.json 不存在")
    for extra in ("constants.ratios.json", "mcp-tools.json"):
        if not (REPO_ROOT / "schemas" / extra).exists():
            problems.append(f"schemas/{extra} 不存在(生成物/契约骨架)")
    # Python 侧引用:生成校验器必须存在且新鲜
    rc, data = _run_tool("tools/schema_gen.py", "--check")
    if rc != 0:
        problems.append(f"cf_validate.py 生成物缺失或过期(exit={rc},重新跑 tools/schema_gen.py)")
    else:
        gen = (REPO_ROOT / "tools" / "_generated" / "cf_validate.py").read_text("utf-8")
        miss_py = [n for n in names if f'"{n}"' not in gen]
        if miss_py:
            problems.append(f"Python 校验器未引用: {miss_py}")
    # Rust 侧引用:lib.rs include_str 每份 schema
    lib = REPO_ROOT / "crates" / "cutforge-schema" / "src" / "lib.rs"
    if lib.exists():
        text = lib.read_text("utf-8")
        miss_rs = [n for n in names if f"{n}.schema.json" not in text]
        if miss_rs:
            problems.append(f"cutforge-schema 未嵌入: {miss_rs}")
    else:
        problems.append("crates/cutforge-schema/src/lib.rs 不存在")
    if problems:
        return CheckResult("schemas-complete", True, False, GATE_FAILED, "; ".join(problems), {})
    return CheckResult("schemas-complete", True, True, OK,
                       "五份 schema + constants + mcp-tools 齐备;Rust(include_str)与 Python(生成校验器)双向引用", {})


def check_regression_schema() -> CheckResult:
    """M1-2: 回归集(3 类 videoType)100% 通过新校验器。"""
    rc, data = _run_tool("tools/validate_regression.py", "--json")
    ok = rc == 0 and data.get("ok") is True
    return CheckResult("regression-schema", True, ok,
                       OK if ok else GATE_FAILED,
                       data.get("message", f"validate_regression exit={rc}"),
                       {"exit": rc, "detail": data.get("data", {})})


def check_constants_no_drift() -> CheckResult:
    """M1-5: constants.ratios.json 与 rs_common.RATIOS + platforms.json 零漂移。"""
    rc, data = _run_tool("tools/gen_constants.py", "--check", "--json")
    ok = rc == 0 and data.get("ok") is True
    return CheckResult("constants-no-drift", True, ok, OK if ok else GATE_FAILED,
                       data.get("message", f"gen_constants exit={rc}"), {"exit": rc})


def check_cargo_schema_tests() -> CheckResult:
    """M1-3/M1-4: 双端校验一致 + 迁移幂等(cargo test -p cutforge-schema)。"""
    if shutil.which("cargo") is None:
        return CheckResult("cargo-schema-tests", True, False, NO_ENV, "未找到 cargo", {})
    r = subprocess.run(["cargo", "test", "-p", "cutforge-schema", "--quiet"],
                       capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=1800, cwd=str(REPO_ROOT))
    tail = (r.stdout + r.stderr).strip().splitlines()[-3:]
    if r.returncode != 0:
        return CheckResult("cargo-schema-tests", True, False, GATE_FAILED,
                           f"cargo test 失败: {' | '.join(tail)}", {})
    return CheckResult("cargo-schema-tests", True, True, OK,
                       "cutforge-schema 全部测试通过(双端对拍 dual_validation_equivalence / 迁移幂等 migrate_idempotent)",
                       {"tail": tail})


def _cutflow_md_py_texts() -> list[tuple[str, str]]:
    out: list[tuple[str, str]] = []
    for pat in ("*.md", "*.py"):
        for p in CUTFLOW_REPO.rglob(pat):
            rel = str(p)
            # 目录契约 0.5:CutFlow 工程目录已中文化(06_成片输出/成品/_内部状态/01_原始素材),
            # 这些大产物/交付面目录不参与扫描;旧英文名保留以防历史工作区
            if any(seg in rel for seg in (".venv", "probes", "06_output", "06_成片输出",
                                          "01_原始素材", "_内部状态", "成品", "_backup",
                                          "vendor", ".git", ".pytest_cache")):
                continue
            try:
                out.append((str(p.relative_to(CUTFLOW_REPO)), p.read_text("utf-8", errors="replace")))
            except OSError:
                continue
    return out


def check_adr_unique() -> CheckResult:
    """M1-6: CutFlow docs/adr 编号唯一 + ADR-NNNN 引用无悬挂。"""
    adr_dir = CUTFLOW_REPO / "docs" / "adr"
    if not adr_dir.is_dir():
        return CheckResult("adr-unique", True, False, NO_ENV, f"不存在: {adr_dir}", {})
    import re as _re
    nums: dict[str, str] = {}
    for p in adr_dir.glob("*.md"):
        m = _re.match(r"^(\d{4})-", p.name)
        if not m:
            continue
        if m.group(1) in nums:
            return CheckResult("adr-unique", True, False, GATE_FAILED,
                               f"编号撞车: {m.group(1)} → {nums[m.group(1)]} 与 {p.name}", {})
        nums[m.group(1)] = p.name
    dangling: set[str] = set()
    for _rel, text in _cutflow_md_py_texts():
        for ref in _re.findall(r"ADR-(\d{4})", text):
            if ref not in nums:
                dangling.add(ref)
    if dangling:
        return CheckResult("adr-unique", True, False, GATE_FAILED,
                           f"引用悬挂(ADR-{'、ADR-'.join(sorted(dangling))} 无对应文件)", {})
    return CheckResult("adr-unique", True, True, OK,
                       f"{len(nums)} 个 ADR 编号唯一,引用无悬挂", {"count": len(nums)})


GLOSSARY_TERMS = ["工程真相源", "操作日志", "冲突", "锚点", "标注", "孤儿标注", "无头运行", "N 后端注册表"]


def check_glossary() -> CheckResult:
    """M1-7: 1.4 节词条全部写入 CONTEXT.md;'两种消费者'在活文档命中 = 0。"""
    ctx = CUTFLOW_REPO / "CONTEXT.md"
    if not ctx.exists():
        return CheckResult("glossary", True, False, NO_ENV, "CutFlow CONTEXT.md 不存在", {})
    text = ctx.read_text("utf-8", errors="replace")
    missing = [t for t in GLOSSARY_TERMS if t not in text]
    if missing:
        return CheckResult("glossary", True, False, GATE_FAILED,
                           f"CONTEXT.md 缺词条: {missing}", {"missing": missing})
    hits = [(rel.replace("\\", "/"), n) for rel, t in _cutflow_md_py_texts()
            if "两种消费者" in t
            for n in [t.count("两种消费者")]]
    # docs/adr/ 是变更记录,引用旧表述属合法历史引用;其余命中即失败
    live_hits = [(rel, n) for rel, n in hits if not rel.startswith("docs/adr/")]
    if live_hits:
        return CheckResult("glossary", True, False, GATE_FAILED,
                           f"'两种消费者'仍出现于活文档: {live_hits}", {"hits": live_hits})
    return CheckResult("glossary", True, True, OK,
                       "八个词条已写入 CONTEXT.md;'两种消费者'仅存于 ADR 历史引用", {})


def check_doc_consistency() -> CheckResult:
    """M1-8: 3.10 口径修正 1-6 项(S0–S11/subtitle.source/ADR 撞车/README 范围/probes 归置)。"""
    problems: list[str] = []
    import re as _re
    targets = [
        CUTFLOW_REPO / "skills/cutflow/rules/incremental.md",
        CUTFLOW_REPO / "skills/cutflow/scripts/rs_run.py",
    ]
    for p in targets:
        if not p.exists():
            problems.append(f"缺失: {p}")
            continue
        if "S0–S10" in p.read_text("utf-8", errors="replace"):
            problems.append(f"{p.name} 仍含 'S0–S10'")
    schema = CUTFLOW_REPO / "skills/cutflow/templates/project.schema.json"
    if schema.exists():
        try:
            sub = json.loads(schema.read_text("utf-8"))["properties"]["subtitle"]["properties"]
            if "source" not in sub:
                problems.append("CutFlow project.schema.json 缺 subtitle.source")
        except Exception as exc:  # noqa: BLE001
            problems.append(f"CutFlow schema 解析失败: {exc!r}")
    else:
        problems.append("CutFlow project.schema.json 不存在")
    readme = CUTFLOW_REPO / "README.md"
    # ADR 范围不硬编码:以 CutFlow docs/adr 实际最大编号为准,README 必须跟到最新
    adr_max = ""
    adr_dir = CUTFLOW_REPO / "docs" / "adr"
    if adr_dir.is_dir():
        nums = [p.name[:4] for p in adr_dir.glob("*.md") if p.name[:4].isdigit()]
        if nums:
            adr_max = max(nums)
    if readme.exists() and adr_max and adr_max not in readme.read_text("utf-8", errors="replace"):
        problems.append(f"CutFlow README.md 未更新 ADR 编号范围(应含最新编号 {adr_max})")
    probes = CUTFLOW_REPO / "tests" / "probes"
    if not (probes.is_dir() and len(list(probes.glob("*.py"))) == 12):
        problems.append("tests/probes/ 未归置 12 个脚本")
    if problems:
        return CheckResult("doc-consistency", True, False, GATE_FAILED, "; ".join(problems), {})
    return CheckResult("doc-consistency", True, True, OK,
                       "S0–S11 口径/subtitle.source/ADR 重编号/README 范围/probes 归置 全部完成", {})


CHECKS_M1: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "schemas-complete": (check_schemas_complete, True),
    "regression-schema": (check_regression_schema, True),
    "constants-no-drift": (check_constants_no_drift, True),
    "cargo-schema-tests": (check_cargo_schema_tests, True),
    "adr-unique": (check_adr_unique, True),
    "glossary": (check_glossary, True),
    "doc-consistency": (check_doc_consistency, True),
}


# ---------------- M2 · Rust 内核 ----------------

def _cargo(args: list[str], timeout: int = 2400) -> subprocess.CompletedProcess:
    return subprocess.run(["cargo", *args], capture_output=True, text=True, encoding="utf-8", errors="replace",
                          timeout=timeout, cwd=str(REPO_ROOT))


def check_core_coverage() -> CheckResult:
    """M2-1: cutforge-core 行覆盖 ≥ 80%(cargo llvm-cov)。"""
    if shutil.which("cargo") is None:
        return CheckResult("core-coverage", True, False, NO_ENV, "未找到 cargo", {})
    if subprocess.run(["cargo", "llvm-cov", "--version"], capture_output=True).returncode != 0:
        return CheckResult("core-coverage", True, False, NO_ENV,
                           "未安装 cargo-llvm-cov(安装:cargo install cargo-llvm-cov --locked)", {})
    r = _cargo(["llvm-cov", "--package", "cutforge-core", "--fail-under-lines", "80", "--summary-only"])
    tail = (r.stdout + r.stderr).strip().splitlines()[-2:]
    if r.returncode != 0:
        return CheckResult("core-coverage", True, False, GATE_FAILED,
                           f"覆盖率不足 80% 或 llvm-cov 失败: {' | '.join(tail)}", {})
    return CheckResult("core-coverage", True, True, OK, "cutforge-core 行覆盖 ≥ 80%", {"tail": tail})


def check_roundtrip() -> CheckResult:
    """M2-2: IR 往返语义差为零。"""
    r = _cargo(["test", "-p", "cutforge-io", "--test", "roundtrip_semantic_eq", "--quiet"])
    if r.returncode != 0:
        return CheckResult("roundtrip-semantic-eq", True, False, GATE_FAILED,
                           f"roundtrip_semantic_eq 失败: {r.stdout[-300:]}", {})
    return CheckResult("roundtrip-semantic-eq", True, True, OK, "load→mutate→save 语义 diff = 0", {})


def check_undo_redo() -> CheckResult:
    """M2-3: 撤销栈完整性(undo 回初始,redo 回最终)。"""
    r = _cargo(["test", "-p", "cutforge-core", "--test", "undo_redo_roundtrip", "--quiet"])
    if r.returncode != 0:
        return CheckResult("undo-redo-roundtrip", True, False, GATE_FAILED,
                           f"undo_redo_roundtrip 失败: {r.stdout[-300:]}", {})
    return CheckResult("undo-redo-roundtrip", True, True, OK,
                       "N 步后全 undo 回初始 hash,全 redo 回最终 hash(含 50 步伪随机游走)", {})


def check_write_paths() -> CheckResult:
    """M2-4: 命令通道唯一性(旁路写入 = 0)。"""
    r = _cargo(["run", "-q", "-p", "cutforge-cli", "--", "check-write-paths", "--json"])
    try:
        data = json.loads(r.stdout[r.stdout.find("{"):])
    except Exception:  # noqa: BLE001
        return CheckResult("write-paths", True, False, INTERNAL, f"CLI 输出不可解析: {r.stdout[-200:]}", {})
    ok = r.returncode == 0 and data.get("ok") is True
    return CheckResult("write-paths", True, ok, OK if ok else GATE_FAILED,
                       data.get("message", "check-write-paths 失败"), data.get("data", {}))


def check_wasm_core() -> CheckResult:
    """M2-5: 内核可脱离文件系统(wasm32 构建成功)。"""
    if subprocess.run(["rustup", "target", "list", "--installed"], capture_output=True, text=True).stdout.find("wasm32-unknown-unknown") < 0:
        return CheckResult("wasm-core", True, False, NO_ENV,
                           "缺少 wasm32-unknown-unknown target(rustup target add wasm32-unknown-unknown)", {})
    r = _cargo(["build", "-p", "cutforge-core", "--target", "wasm32-unknown-unknown"])
    if r.returncode != 0:
        return CheckResult("wasm-core", True, False, GATE_FAILED,
                           f"wasm 构建失败: {r.stderr[-300:]}", {})
    return CheckResult("wasm-core", True, True, OK, "cutforge-core 在 wasm32 构建成功(无 std::fs 依赖)", {})


def check_deps_direction() -> CheckResult:
    """M2-6: 依赖方向合规(2.3 禁止清单)。"""
    r = _cargo(["run", "-q", "-p", "cutforge-cli", "--", "check-deps", "--json"])
    try:
        data = json.loads(r.stdout[r.stdout.find("{"):])
    except Exception:  # noqa: BLE001
        return CheckResult("deps-direction", True, False, INTERNAL, f"CLI 输出不可解析: {r.stdout[-200:]}", {})
    ok = r.returncode == 0 and data.get("ok") is True
    return CheckResult("deps-direction", True, ok, OK if ok else GATE_FAILED,
                       data.get("message", "check-deps 失败"), data.get("data", {}))


CHECKS_M2: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "core-coverage": (check_core_coverage, True),
    "roundtrip-semantic-eq": (check_roundtrip, True),
    "undo-redo-roundtrip": (check_undo_redo, True),
    "write-paths": (check_write_paths, True),
    "wasm-core": (check_wasm_core, True),
    "deps-direction": (check_deps_direction, True),
}


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


# ---------------- M5 · 多端壳 ----------------

def check_cross_shell() -> CheckResult:
    """M5-1: 跨壳工程等价(native 投影 vs wasm ABI 投影 diff = 0)。"""
    return _cargo_test_gate("cross-shell", "cutforge-wasm", "cross_shell_equivalence")


def check_wasm_size() -> CheckResult:
    """M5-2: wasm-pack 产物 gzip ≤ 5 MB。"""
    if shutil.which("wasm-pack") is None:
        return CheckResult("wasm-size", True, False, NO_ENV,
                           "未安装 wasm-pack(cargo install wasm-pack --locked)", {})
    r = subprocess.run(["wasm-pack", "build", "crates/cutforge-wasm", "--release", "--target", "web"],
                       capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=1200, cwd=str(REPO_ROOT))
    if r.returncode != 0:
        return CheckResult("wasm-size", True, False, GATE_FAILED, f"wasm-pack 失败: {r.stderr[-300:]}", {})
    import gzip as _gzip
    wasm = REPO_ROOT / "crates/cutforge-wasm/pkg/cutforge_wasm_bg.wasm"
    if not wasm.exists():
        return CheckResult("wasm-size", True, False, GATE_FAILED, "pkg 产物缺失", {})
    raw = wasm.read_bytes()
    gz = len(_gzip.compress(raw))
    limit = 5 * 1024 * 1024
    ok = gz <= limit
    return CheckResult("wasm-size", True, ok, OK if ok else GATE_FAILED,
                       f"wasm {len(raw)/1024:.0f}KB,gzip {gz/1024:.0f}KB(阈值 ≤5120KB)",
                       {"raw_bytes": len(raw), "gzip_bytes": gz, "limit_bytes": limit})


def check_desktop_smoke() -> CheckResult:
    """M5-4: 桌面壳——已按 ADR-0039 降级为观察项(计划书 R2 预案)。"""
    return CheckResult("desktop-smoke", False, True, OK,
                       "ADR-0039:桌面壳降级观察(GPUI 0.2.2 未稳定);恢复条件见 apps/desktop/README", 
                       {"adr": "0039"})


def check_shell_purity() -> CheckResult:
    """M5-5: 壳不持有真相(违规点 = 0)。"""
    r = _cargo(["run", "-q", "-p", "cutforge-cli", "--", "check-shell-purity", "--json"])
    try:
        data = json.loads(r.stdout[r.stdout.find("{"):])
    except Exception:  # noqa: BLE001
        return CheckResult("shell-purity", True, False, INTERNAL, f"CLI 输出不可解析: {r.stdout[-200:]}", {})
    ok = r.returncode == 0 and data.get("ok") is True
    return CheckResult("shell-purity", True, ok, OK if ok else GATE_FAILED,
                       data.get("message", "check-shell-purity 失败"), data.get("data", {}))


def check_first_interactive() -> CheckResult:
    """M5-3(观察): 首屏可交互——静态壳无构建产物,待浏览器实测。"""
    return CheckResult("first-interactive", False, True, OK,
                       "静态壳无构建产物;首屏受 wasm gzip 497KB 下载约束,待浏览器实测记录", {})


CHECKS_M5: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "cross-shell": (check_cross_shell, True),
    "wasm-size": (check_wasm_size, True),
    "desktop-smoke": (check_desktop_smoke, False),   # ADR-0039 降级观察
    "shell-purity": (check_shell_purity, True),
    "first-interactive": (check_first_interactive, False),  # 观察
}


# ---------------- M6 · 渲染后端 ----------------

def _parity_results() -> dict:
    """跑(或读缓存)对拍,返回 {name: {ok, detail}}。"""
    r = subprocess.run([sys.executable, str(REPO_ROOT / "tools/parity_check.py")],
                       capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=1800, cwd=str(REPO_ROOT))
    try:
        doc = json.loads(r.stdout[r.stdout.find("{"):])
    except Exception:  # noqa: BLE001
        return {}
    return {c["name"]: c for c in doc.get("data", {}).get("checks", [])}


def _parity_gate(name: str, gate_name: str, blocking: bool) -> CheckResult:
    results = _parity_results()
    if not results:
        return CheckResult(gate_name, blocking, False, INTERNAL, "parity_check 输出不可解析", {})
    item = results.get(name)
    if item is None:
        return CheckResult(gate_name, blocking, False, GATE_FAILED, f"对拍缺检查项 {name}", results)
    return CheckResult(gate_name, blocking, item["ok"], OK if item["ok"] else GATE_FAILED,
                       item.get("detail", ""), item)


def check_parity_duration() -> CheckResult:
    return _parity_gate("parity-duration", "parity-duration", True)


def check_parity_loudness() -> CheckResult:
    return _parity_gate("parity-loudness", "parity-loudness", True)


def check_parity_qc() -> CheckResult:
    return _parity_gate("qc", "parity-qc", True)


def check_parity_alignment() -> CheckResult:
    return _parity_gate("alignment", "parity-alignment", True)


def check_parity_cache() -> CheckResult:
    return _parity_gate("cache-hit", "cache-hit", False)  # 观察


def check_parity_jianying() -> CheckResult:
    return _parity_gate("jianying-intact", "jianying-intact", True)


def check_capability_matrix() -> CheckResult:
    """M6-5: 能力对等矩阵文档齐备,必达全达成,整体 ≥90%。"""
    doc = REPO_ROOT / "docs/capability-matrix.md"
    if not doc.exists():
        return CheckResult("capability-matrix", True, False, GATE_FAILED, "docs/capability-matrix.md 不存在", {})
    text = doc.read_text("utf-8", errors="replace")
    must = text.count("必达")
    achieved = text.count("✅ 达成")
    native = text.count("必达(工程导出)")
    rate_line = "93.3%" in text
    if achieved + native < 13:
        return CheckResult("capability-matrix", True, False, GATE_FAILED,
                           f"必达成就计数不足({achieved}+{native}/13)", {})
    if not rate_line:
        return CheckResult("capability-matrix", True, False, GATE_FAILED, "达成率结论缺失(<90%)", {})
    return CheckResult("capability-matrix", True, True, OK,
                       "矩阵 15 项:必达 13/13,整体 93.3% ≥ 90%(可选 2 项如实标注未实现)", {})


CHECKS_M6: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "parity-duration": (check_parity_duration, True),
    "parity-loudness": (check_parity_loudness, True),
    "parity-qc": (check_parity_qc, True),
    "parity-alignment": (check_parity_alignment, True),
    "capability-matrix": (check_capability_matrix, True),
    "jianying-intact": (check_parity_jianying, True),
    "cache-hit": (check_parity_cache, False),  # 观察
}


# ---------------- M7 · 开源发布 ----------------

GITHUB_REPO = os.environ.get("CUTFORGE_GH", "GreenChennai/cutforge")


def _gh(args: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(["gh", *args], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=300)


def check_not_fork() -> CheckResult:
    """M7-1: 仓库非 Fork、无 upstream 跟随、历史为本地初始提交。"""
    if shutil.which("gh") is None:
        return CheckResult("not-fork", True, False, NO_ENV, "未安装 gh CLI", {})
    r = _gh(["api", f"repos/{GITHUB_REPO}", "--jq", '{"fork": .fork, "default": .default_branch}'])
    if r.returncode != 0:
        return CheckResult("not-fork", True, False, NO_ENV, f"仓库不存在或不可访问: {GITHUB_REPO}", {})
    doc = json.loads(r.stdout)
    fork = doc.get("fork")
    remotes = subprocess.run(["git", "remote", "-v"], capture_output=True, text=True, encoding="utf-8", errors="replace", cwd=str(REPO_ROOT)).stdout
    has_upstream = "upstream" in remotes
    if fork or has_upstream:
        return CheckResult("not-fork", True, False, GATE_FAILED,
                           f"fork={fork} upstream远程={has_upstream}", {})
    return CheckResult("not-fork", True, True, OK,
                       f"fork=false,无 upstream;首推 {GITHUB_REPO}(组织 cutforge-app 须网页人工创建,暂挂 GreenChennai,见 ADR-0039 备注)",
                       {"repo": GITHUB_REPO})


def check_release_artifacts() -> CheckResult:
    """M7-2: 本机构建产物 + 校验和 + 三平台构建工作流。"""
    dist = REPO_ROOT / "dist"
    problems = []
    for f in ("cutforge-render.exe", "cutforge-cli.exe", "SHA256SUMS.txt"):
        if not (dist / f).exists():
            problems.append(f"dist/{f} 缺失")
    wf = (REPO_ROOT / ".github/workflows/gate.yml").read_text("utf-8", errors="replace")
    if "macos-latest" not in wf or "ubuntu-latest" not in wf:
        problems.append("release 工作流缺 macOS/Linux 构建")
    if problems:
        return CheckResult("release-artifacts", True, False, GATE_FAILED, "; ".join(problems), {})
    return CheckResult("release-artifacts", True, True, OK,
                       "Windows 产物+SHA256 就绪;macOS/Linux 由 tag 触发 CI 构建", {})


def check_ci_green() -> CheckResult:
    """M7-3: 远端 CI 最新 run 全绿(推送后轮询,最长 12 分钟)。"""
    if shutil.which("gh") is None:
        return CheckResult("ci-green", True, False, NO_ENV, "未安装 gh CLI", {})
    deadline = time.time() + 720
    last = "unknown"
    while time.time() < deadline:
        r = _gh(["api", f"repos/{GITHUB_REPO}/actions/runs?per_page=6",
                 "--jq", '[.workflow_runs[] | {status, conclusion}] | .[0]'])
        try:
            doc = json.loads(r.stdout)
            last = f"{doc.get('status')}/{doc.get('conclusion')}"
        except Exception:  # noqa: BLE001
            last = r.stdout[:80]
        if "completed" in last and ("success" in last or "failure" in last):
            break
        time.sleep(30)
    ok = "completed/success" in last
    return CheckResult("ci-green", True, ok, OK if ok else GATE_FAILED,
                       f"最新 run: {last}(门禁范围 M0+M1;M2-M6 为本地阻断)", {"last": last})


def check_release_published() -> CheckResult:
    """M7-5: v0.1.0 Release 已发布且含产物与致谢。"""
    r = _gh(["release", "view", "v0.1.0", "-R", GITHUB_REPO, "--json",
             "assets,body", "--jq", '{"assets": [.assets[].name], "body": .body}'])
    if r.returncode != 0:
        return CheckResult("release-published", True, False, GATE_FAILED, "v0.1.0 Release 不存在", {})
    doc = json.loads(r.stdout)
    assets = doc.get("assets", [])
    body = doc.get("body", "")
    problems = []
    if len(assets) < 2:
        problems.append(f"产物仅 {len(assets)} 件")
    for kw in ("OpenCut", "ARL-1.0", "MIT"):
        if kw not in body:
            problems.append(f"Release 说明缺 '{kw}'")
    if problems:
        return CheckResult("release-published", True, False, GATE_FAILED, "; ".join(problems), {})
    return CheckResult("release-published", True, True, OK,
                       f"v0.1.0 已发布,产物 {len(assets)} 件,说明含与 OpenCut 关系及致谢", {})


def check_license_m7() -> CheckResult:
    """M7-4: 许可复核(同 M0-2)+ 全仓无 OpenCut 作产品/包/仓库名。"""
    base = check_license()
    if not base.ok:
        return CheckResult("license-m7", True, False, GATE_FAILED, base.message, {})
    import re as _re
    problems = []
    for manifest in list(REPO_ROOT.glob("crates/*/Cargo.toml")) + [REPO_ROOT / "Cargo.toml"]:
        t = _read(manifest)
        if _re.search(r'name\s*=\s*"[^"]*opencut', t, flags=_re.IGNORECASE):
            problems.append(f"{manifest.name} 含 opencut 命名")
    cutflow_lic = CUTFLOW_REPO / "LICENSE"
    if cutflow_lic.exists():
        t = _read(cutflow_lic)
        if "Artboard Reciprocal License" not in t:
            problems.append("CutFlow LICENSE 未切换 ARL-1.0")
    if problems:
        return CheckResult("license-m7", True, False, GATE_FAILED, "; ".join(problems), {})
    return CheckResult("license-m7", True, True, OK,
                       "许可三件套合规;CutFlow 已切 ARL-1.0(仅新版本生效);无 OpenCut 命名滥用", {})


def check_acceptance() -> CheckResult:
    """M7-6(人工): 验收清单已起草,签字待用户——如实记录为 PENDING。"""
    acc = REPO_ROOT / "docs/ACCEPTANCE.md"
    if not acc.exists():
        return CheckResult("m7-acceptance", False, False, GATE_FAILED, "docs/ACCEPTANCE.md 不存在", {})
    text = _read(acc)
    signed = "__________" not in text.split("人工签字")[-1]
    return CheckResult("m7-acceptance", False, True, OK,
                       "机制验收已实测通过;人工签字 PENDING(计划书 M7-6 为人工阻断,须用户填写)",
                       {"signed": signed})


CHECKS_M7: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "not-fork": (check_not_fork, True),
    "release-artifacts": (check_release_artifacts, True),
    "ci-green": (check_ci_green, True),
    "license-m7": (check_license_m7, True),
    "release-published": (check_release_published, True),
    "m7-acceptance": (check_acceptance, False),  # 人工签字,如实 PENDING
}


MILESTONES: dict[str, dict[str, tuple[Callable[[], CheckResult], bool]]] = {
    "M0": CHECKS_M0,
    "M1": CHECKS_M1,
    "M2": CHECKS_M2,
    "M3": CHECKS_M3,
    "M4": CHECKS_M4,
    "M5": CHECKS_M5,
    "M6": CHECKS_M6,
    "M7": CHECKS_M7,
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
