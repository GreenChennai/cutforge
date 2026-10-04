"""M5 · 多端壳(跨壳等价/wasm 体积/壳纯度)。A-10 拆包。"""
from __future__ import annotations

import shutil
import subprocess
from typing import Callable

from .common import GATE_FAILED, INTERNAL, NO_ENV, OK, REPO_ROOT, CheckResult
from .m2 import _cargo
from .m3 import _cargo_test_gate


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
