"""M1 · 契约固化(schema 齐备/回归集/常量零漂移/跨仓 ADR 与词条)。A-10 拆包。"""
from __future__ import annotations

import json
import shutil
import subprocess
import sys
from typing import Callable

from .common import (
    CUTFLOW_REPO,
    GATE_FAILED,
    INTERNAL,
    NO_ENV,
    OK,
    REPO_ROOT,
    CheckResult,
    _read,
)


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
