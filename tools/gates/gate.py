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


# ---------------- M1 · 契约固化 ----------------

def _run_tool(rel: str, *args: str) -> tuple[int, dict]:
    """跑仓库内工具并解析结果协议 JSON;工具缺失按 NO_ENV 处理。"""
    script = REPO_ROOT / rel
    if not script.exists():
        return 3, {"message": f"工具不存在: {script}"}
    r = subprocess.run([sys.executable, str(script), *args],
                       capture_output=True, text=True, timeout=1800, cwd=str(REPO_ROOT))
    try:
        start = r.stdout.find("{")
        data = json.loads(r.stdout[start:]) if start >= 0 else {}
    except Exception:  # noqa: BLE001
        data = {"raw": (r.stdout + r.stderr)[-400:]}
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
                       capture_output=True, text=True, timeout=1800, cwd=str(REPO_ROOT))
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
            if any(seg in rel for seg in (".venv", "probes", "06_output", "_backup",
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
    if readme.exists() and "0001–0038" not in readme.read_text("utf-8", errors="replace"):
        problems.append("CutFlow README.md 未更新 ADR 编号范围(0001–0038)")
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
    return subprocess.run(["cargo", *args], capture_output=True, text=True,
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


MILESTONES: dict[str, dict[str, tuple[Callable[[], CheckResult], bool]]] = {
    "M0": CHECKS_M0,
    "M1": CHECKS_M1,
    "M2": CHECKS_M2,
    "M3": CHECKS_M3,
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
