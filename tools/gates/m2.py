"""M2 · Rust 内核(覆盖率/往返/撤销/写路径唯一/wasm/依赖方向)。A-10 拆包。"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
from typing import Callable

from .common import GATE_FAILED, INTERNAL, NO_ENV, OK, REPO_ROOT, CheckResult


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


def check_changelog() -> CheckResult:
    """G-4(审查报告 v2 §8.1): CHANGELOG 纪律——警告级(观察项,不阻断)。
    简化口径:diff 触及 crates/*/src/** 或 apps/*/src/**(公开行为面的保守代理)而未改
    CHANGELOG.md → 警告。基准:env CUTFORGE_CHANGELOG_BASE(CI 传 PR base sha;
    缺省 diff 工作树 = 本地未提交改动口径;push 无基准自然跳过)。
    升级为红线条件:CHANGELOG.md Unreleased 段约定跑满一个完整迭代、CI 警告无新增误报后,
    把本项 blocking 翻转为 True(见 CONTRIBUTING.md 门禁扫描面登记)。"""
    if shutil.which("git") is None:
        return CheckResult("changelog", False, True, OK, "SKIP(观察): 未找到 git,无法取 diff", {})
    base = os.environ.get("CUTFORGE_CHANGELOG_BASE", "").strip()
    git_args = ["diff", "--name-only", base] if base else ["diff", "--name-only", "HEAD"]
    r = subprocess.run(["git", *git_args], capture_output=True, text=True,
                       encoding="utf-8", errors="replace", cwd=str(REPO_ROOT))
    if r.returncode != 0:
        return CheckResult("changelog", False, True, OK,
                           f"SKIP(观察): diff 基准不可用({'base=' + base if base else 'HEAD'}),不拦", {})
    files = [ln.strip().replace("\\", "/") for ln in r.stdout.splitlines() if ln.strip()]
    if not files:
        return CheckResult("changelog", False, True, OK,
                           f"无待检 diff({'base=' + base if base else '工作树 vs HEAD 干净'}),通过", {})
    code_touched = sorted(f for f in files
                          if f.startswith(("crates/", "apps/")) and "/src/" in f)
    changelog_touched = any(f.lower() == "changelog.md" for f in files)
    data = {"base": base or "HEAD", "changed": len(files), "code_files": code_touched[:20],
            "changelog_touched": changelog_touched}
    if code_touched and not changelog_touched:
        return CheckResult("changelog", False, False, GATE_FAILED,
                           f"警告级(观察,不阻断): 触及 {len(code_touched)} 个内核/壳源文件"
                           f"(crates|apps */src/**)而未改 CHANGELOG.md——公开行为面变更须登记"
                           f"(升级红线条件见 CONTRIBUTING.md 门禁扫描面登记)", data)
    return CheckResult("changelog", False, True, OK,
                       "CHANGELOG 纪律通过(源码变更已伴随 CHANGELOG,或本次无内核/壳源码变更)", data)


CHECKS_M2: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "changelog": (check_changelog, False),  # G-4 警告级;跑满一轮迭代后评估转红线
    "core-coverage": (check_core_coverage, True),
    "roundtrip-semantic-eq": (check_roundtrip, True),
    "undo-redo-roundtrip": (check_undo_redo, True),
    "write-paths": (check_write_paths, True),
    "wasm-core": (check_wasm_core, True),
    "deps-direction": (check_deps_direction, True),
}
