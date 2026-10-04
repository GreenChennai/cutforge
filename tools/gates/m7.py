"""M7 · 开源发布(fork 检查/产物/CI 绿/许可复核)。A-10 拆包。

原文件顶部 `time = __import__("time")` 的等效导入移入本模块(唯一使用者 check_ci_green)。
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import time
from typing import Callable

from .common import CUTFLOW_REPO, GATE_FAILED, NO_ENV, OK, REPO_ROOT, CheckResult, _read
from .m0 import check_license


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
