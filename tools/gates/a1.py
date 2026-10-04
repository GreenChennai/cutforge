"""A1 · 册一「内核重构与架构加固」册级门禁(D-A2 注册制)。A-10 拆包。"""
from __future__ import annotations

import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Callable

from .common import GATE_FAILED, NO_ENV, OK, REPO_ROOT, CheckResult
from .m1 import _run_tool
from .m2 import _cargo


# ---------------- A1 · 册一「内核重构与架构加固」册级门禁 ----------------
# D-A2(册级门禁注册制):每册一个 A<n> 入口聚合本册验收,分级沿用 M0-M7 的
# 阻断/观察二元;与里程碑门禁互不干扰(CI 只跑 M0/M1,A1 本机册收官跑)。

RUST_LINE_LIMIT = 800  # AC-1.1 红线:非测试源文件 ≤800 行


def check_cargo_test_workspace() -> CheckResult:
    """A1-1(AC-1.9): cargo test --workspace --locked 全绿。
    http_hardening(T1.6/AC-1.7)与 cache_addressing(T1.5/AC-1.4)的测试均已含在
    workspace 测试面内,A1 不重复单跑。"""
    if shutil.which("cargo") is None:
        return CheckResult("cargo-test-workspace", True, False, NO_ENV, "未找到 cargo", {})
    r = _cargo(["test", "--workspace", "--locked"])
    tail = (r.stdout + r.stderr).strip().splitlines()[-3:]
    if r.returncode != 0:
        return CheckResult("cargo-test-workspace", True, False, GATE_FAILED,
                           f"cargo test --workspace 失败: {' | '.join(tail)}", {})
    return CheckResult("cargo-test-workspace", True, True, OK,
                       "cargo test --workspace --locked 全绿"
                       "(含 http_hardening / cache_addressing / protocol_conformance)", {"tail": tail})


def check_cargo_clippy() -> CheckResult:
    """A1-2(AC-1.1): clippy --workspace --all-targets -D warnings,新增告警 = 0。"""
    if shutil.which("cargo") is None:
        return CheckResult("cargo-clippy", True, False, NO_ENV, "未找到 cargo", {})
    r = _cargo(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])
    tail = (r.stdout + r.stderr).strip().splitlines()[-3:]
    if r.returncode != 0:
        return CheckResult("cargo-clippy", True, False, GATE_FAILED,
                           f"clippy 告警非零(-D warnings 升级为错误): {' | '.join(tail)}", {})
    return CheckResult("cargo-clippy", True, True, OK,
                       "clippy --workspace --all-targets 零告警", {"tail": tail})


def check_rust_line_limit() -> CheckResult:
    """A1-3(AC-1.1): crates 下非测试 .rs 全部 ≤800 行(内联实现,不依赖 shell 管道)。
    「非测试」口径:目录树含 tests/ 段的不算,与 AC-1.1 判定命令(! -path '*/tests/*')一致。"""
    root = REPO_ROOT / "crates"
    if not root.is_dir():
        return CheckResult("rust-line-limit", True, False, NO_ENV, "crates/ 不存在", {})
    offenders: list[tuple[str, int]] = []
    scanned, max_f, max_n = 0, "", 0
    for p in root.rglob("*.rs"):
        rel = p.relative_to(REPO_ROOT)
        parts = rel.parts
        if "tests" in parts or "target" in parts:
            continue
        try:
            with p.open("r", encoding="utf-8", errors="replace") as f:
                n = sum(1 for _ in f)
        except OSError:
            continue
        scanned += 1
        if n > max_n:
            max_f, max_n = "/".join(parts), n
        if n > RUST_LINE_LIMIT:
            offenders.append(("/".join(parts), n))
    if offenders:
        offenders.sort(key=lambda x: -x[1])
        head = "; ".join(f"{f}:{n}" for f, n in offenders[:6])
        return CheckResult("rust-line-limit", True, False, GATE_FAILED,
                           f"{len(offenders)} 个非测试源文件超 {RUST_LINE_LIMIT} 行红线: {head}",
                           {"offenders": offenders})
    return CheckResult("rust-line-limit", True, True, OK,
                       f"crates 非测试 .rs 共 {scanned} 个,最大 {max_n} 行({max_f})≤ {RUST_LINE_LIMIT}",
                       {"scanned": scanned, "max": max_f, "max_lines": max_n})


# G-1(审查报告 v2 §8.1): 桌面壳独立行数上限。本轮只报告不阻断——app.rs(1971 行)等
# 超限文件待 A-02 拆分波落地,挂阻断即永红;A-02 落地后翻转 blocking=True。
DESKTOP_LINE_LIMIT = 600


def check_desktop_line_limit() -> CheckResult:
    """G-1: apps/desktop 行数扫描(报告模式,观察项)。
    扫描面:apps/desktop/src/**/*.rs;豁免:ui/ 目录(token/图标/动效等纯声明模块落点,A-08);
    上限:600 行(桌面壳独立口径,与 crates 800 / apps/web js 400 分列,扫描面显式化纪律)。
    转阻断条件:A-02(app.rs 拆分四模块)落地且超限清单清零后,翻转 blocking=True。"""
    root = REPO_ROOT / "apps" / "desktop" / "src"
    if not root.is_dir():
        return CheckResult("desktop-line-limit", False, False, NO_ENV, "apps/desktop/src 不存在", {})
    offenders: list[tuple[str, int]] = []
    scanned = 0
    for p in sorted(root.rglob("*.rs")):
        rel = p.relative_to(root)
        if "ui" in rel.parts:  # ui/ 豁免(theme.rs/icon.rs/fx.rs 等落点,同 Web tokens.css 口径)
            continue
        try:
            with p.open("r", encoding="utf-8", errors="replace") as f:
                n = sum(1 for _ in f)
        except OSError:
            continue
        scanned += 1
        if n > DESKTOP_LINE_LIMIT:
            offenders.append(("/".join(rel.parts), n))
    offenders.sort(key=lambda x: -x[1])
    if offenders:
        head = "; ".join(f"{f}:{n}" for f, n in offenders[:8])
        return CheckResult("desktop-line-limit", False, False, GATE_FAILED,
                           f"报告模式(观察,不阻断): apps/desktop {scanned} 个 .rs 中 "
                           f"{len(offenders)} 个超 {DESKTOP_LINE_LIMIT} 行(待 A-02 拆分): {head}",
                           {"offenders": offenders, "limit": DESKTOP_LINE_LIMIT, "scanned": scanned})
    return CheckResult("desktop-line-limit", False, True, OK,
                       f"apps/desktop {scanned} 个 .rs 全部 ≤ {DESKTOP_LINE_LIMIT} 行",
                       {"offenders": [], "limit": DESKTOP_LINE_LIMIT, "scanned": scanned})


def _desktop_rs_hits(pattern: str, root: Path | None = None) -> list[tuple[str, int, str]]:
    """桌面壳 .rs 静态扫描器(TC-GATE-002/003 共用):返回 (相对路径, 行号, 行内容)。
    root 参数供反面测试注入临时夹具;默认扫 apps/desktop/src(豁免 ui/ 目录)。"""
    base = root if root is not None else REPO_ROOT / "apps" / "desktop" / "src"
    hits: list[tuple[str, int, str]] = []
    rx = re.compile(pattern)
    for p in sorted(base.rglob("*.rs")):
        rel = "/".join(p.relative_to(base).parts)
        if root is None and "ui" in p.relative_to(base).parts:  # ui/ 豁免(同行数扫描口径)
            continue
        try:
            text = p.read_text("utf-8", errors="replace")
        except OSError:
            continue
        for i, line in enumerate(text.splitlines(), 1):
            if rx.search(line):
                hits.append((rel, i, line.strip()[:120]))
    return hits


def check_desktop_box_leak() -> CheckResult:
    """TC-GATE-002(审查报告 v2 BUG-18): 桌面壳禁 Box::leak 静态检查(报告模式,观察项)。
    apps/desktop/src 当前存在真实命中(inspector.rs 渲染路径),如实列出不清零不粉饰;
    转阻断条件:BUG-18 修复(改 SharedString/Arc<str>/常量表)清零后,翻转 blocking=True。
    反面测试:negative_tests.py TC-GATE-002(注入含 Box::leak 的临时文件,扫描器必须命中)。"""
    hits = _desktop_rs_hits(r"Box::leak")
    if hits:
        head = "; ".join(f"{f}:{n}" for f, n, _s in hits[:8])
        return CheckResult("desktop-box-leak", False, False, GATE_FAILED,
                           f"报告模式(观察,不阻断): 发现 {len(hits)} 处 Box::leak(待 BUG-18 修复): {head}",
                           {"hits": hits})
    return CheckResult("desktop-box-leak", False, True, OK,
                       "apps/desktop/src 无 Box::leak(清零,可转阻断)", {"hits": []})


# 裸色值口径与 Web 壳 check-shell-purity R5 一致:hsl/hslA/rgb/rgba 函数与 #RRGGBB(A) 字面量。
_DESKTOP_BARE_COLOR_RX = r"\bhsla?\(|\brgba?\(|#[0-9a-fA-F]{6}\b|#[0-9a-fA-F]{8}\b"


def check_desktop_color_purity() -> CheckResult:
    """TC-GATE-003(审查报告 v2 A-08): 桌面壳裸色值扫描(报告模式,观察项)。
    扫描面:apps/desktop/src/**/*.rs(未来 ui/theme.rs 落地后随 ui/ 豁免——唯一定义点纪律);
    口径:hsl/hslA/rgb/rgba 调用与 #RRGGBB(A) 字面量(与 Web 壳纯度 R5 同源)。
    当前存在真实命中(home.rs/timeline.rs/library.rs),如实列出;
    转阻断条件:A-08 ui/theme.rs token 体系落地、命中清零后,翻转 blocking=True。
    反面测试:negative_tests.py TC-GATE-003。"""
    hits = _desktop_rs_hits(_DESKTOP_BARE_COLOR_RX)
    if hits:
        head = "; ".join(f"{f}:{n}" for f, n, _s in hits[:8])
        return CheckResult("desktop-color-purity", False, False, GATE_FAILED,
                           f"报告模式(观察,不阻断): 发现 {len(hits)} 处裸色值(待 A-08 token 化): {head}",
                           {"hits": hits})
    return CheckResult("desktop-color-purity", False, True, OK,
                       "apps/desktop/src 零裸色值(清零,可转阻断)", {"hits": []})


def check_tool_parity() -> CheckResult:
    """A1-4(AC-1.2): MCP 工具黄金响应库对拍(册一建 41;册二 A2 增 render_frame 后 42;
    册四 A4 增 clip_trim/clip_split_all/track_update/clip_gap_delete/clip_copy/clip_paste_at 后 48)。
    加法容忍:实际多出的键仅警告;值变化/键缺失 = DRIFT 阻断。
    重建快照:python tools/bench/tool_parity.py --update-golden(须随有计划的行为变更同步重建)。"""
    script = REPO_ROOT / "tools" / "bench" / "tool_parity.py"
    if not script.exists():
        return CheckResult("tool-parity", True, False, NO_ENV, f"工具不存在: {script}", {})
    rc, data = _run_tool("tools/bench/tool_parity.py", "--json")
    ok = rc == 0 and data.get("ok") is True
    return CheckResult("tool-parity", True, ok, OK if ok else GATE_FAILED,
                       data.get("message", f"tool_parity exit={rc}"),
                       {"exit": rc, "counts": data.get("data", {}).get("counts", {})})


def _py_e2e_gate(name: str, rel: str, *args: str) -> CheckResult:
    """A1 通用 python e2e 门禁:exit 0 = 通过,输出尾行进 message。"""
    script = REPO_ROOT / rel
    if not script.exists():
        return CheckResult(name, True, False, NO_ENV, f"工具不存在: {script}", {})
    r = subprocess.run([sys.executable, str(script), *args], capture_output=True, text=True,
                       encoding="utf-8", errors="replace", timeout=1800, cwd=str(REPO_ROOT))
    tail = ((r.stdout or "") + (r.stderr or "")).strip().splitlines()[-3:]
    if r.returncode != 0:
        return CheckResult(name, True, False, GATE_FAILED,
                           f"{script.name} 失败(exit={r.returncode}): {' | '.join(tail)}", {})
    return CheckResult(name, True, True, OK, f"{script.name} 断言链全过", {"tail": tail})


def check_e2e_static() -> CheckResult:
    """A1-5(AC-1.6/T1.6): 静态目录托管 e2e(穿越 100% 拒绝 + ETag/304 + 零 Rust 改动可达)。"""
    return _py_e2e_gate("e2e-static", "tools/e2e_static.py")


def check_e2e_events() -> CheckResult:
    """A1-6(AC-1.5/T1.6): 事件推送 e2e(SSE P95 ≤200ms + 长轮询降级 ≤1s + notes/cutlist 事件面)。"""
    return _py_e2e_gate("e2e-events", "tools/e2e_events.py")


def check_bench_threshold() -> CheckResult:
    """A1-7(AC-1.8/T1.8): 性能阈值 `bench.py --check`(劣化 >20% 阻断)。
    该脚本由 T1.8 并行落库;缺失时如实 SKIP(观察,不冒充通过)并注明待 T1.8;
    脚本落库后本项自动转为阻断,命令契约固定为:python tools/bench/bench.py --check。"""
    script = REPO_ROOT / "tools" / "bench" / "bench.py"
    if not script.exists():
        return CheckResult("bench-threshold", False, False, GATE_FAILED,
                           "SKIP(观察): tools/bench/bench.py 尚未落库(待 T1.8);落库后本项为真阻断",
                           {"skip": True, "reason": "待 T1.8 落库",
                            "command": "python tools/bench/bench.py --check"})
    rc, data = _run_tool("tools/bench/bench.py", "--check")
    ok = rc == 0 and data.get("ok") is not False
    return CheckResult("bench-threshold", True, ok, OK if ok else GATE_FAILED,
                       data.get("message", f"bench --check exit={rc}"),
                       {"exit": rc, **(data.get("data") or {})})


def check_pytest_suite() -> CheckResult:
    """A1-8(加固): python -m pytest tests -q 全绿。
    tests/ 面含 JY 桥源级对拍、契约烟测、四桥冒烟,此前只在 CI python-gates job 跑、
    本地 A1 未含,T1.1 拆分后的源级断言假红由此逃逸;纳为本项本地兜底(仓库根可执行)。
    CI gate.yml 的 python-gates job 照旧跑 pytest,两处口径一致;不影响 M0-M7。"""
    r = subprocess.run([sys.executable, "-m", "pytest", "tests", "-q"],
                       capture_output=True, text=True, encoding="utf-8", errors="replace",
                       timeout=900, cwd=str(REPO_ROOT))
    tail = ((r.stdout or "") + (r.stderr or "")).strip().splitlines()[-3:]
    if r.returncode != 0:
        return CheckResult("pytest-suite", True, False, GATE_FAILED,
                           f"pytest tests 失败(exit={r.returncode}): {' | '.join(tail)}", {})
    return CheckResult("pytest-suite", True, True, OK,
                       f"pytest tests 全绿({tail[-1] if tail else 'ok'})", {"tail": tail})


CHECKS_A1: dict[str, tuple[Callable[[], CheckResult], bool]] = {
    "bench-threshold": (check_bench_threshold, False),  # bench.py 未落库前为观察 SKIP(T1.8)
    "cargo-clippy": (check_cargo_clippy, True),
    "cargo-test-workspace": (check_cargo_test_workspace, True),
    "desktop-box-leak": (check_desktop_box_leak, False),    # TC-GATE-002;BUG-18 清零后转阻断
    "desktop-color-purity": (check_desktop_color_purity, False),  # TC-GATE-003;A-08 落地后转阻断
    "desktop-line-limit": (check_desktop_line_limit, False),      # G-1;A-02 落地后转阻断
    "e2e-events": (check_e2e_events, True),
    "e2e-static": (check_e2e_static, True),
    "pytest-suite": (check_pytest_suite, True),  # 本地兜底:pytest 纳入 A1(此前仅 CI 跑)
    "rust-line-limit": (check_rust_line_limit, True),
    "tool-parity": (check_tool_parity, True),
}
