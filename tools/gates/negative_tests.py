#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""门禁反面测试(工程纪律 §8.2-2:每条门禁配一个"注入缺陷应红灯"的负例)。

用例清单(审查报告 v2 §4 BUG-15 / §8 / §9.4 A-08 / §9.7):
    TC-GATE-001 (BUG-15): 向 CORE-FILES 注入不存在路径 → gate M0/license 反例非零退出(exit=2);
                          真实清单 → 通过(exit=0)。红-绿双向证明存在性校验生效。
    TC-GATE-002 (BUG-18): 桌面壳 Box::leak 扫描器对注入缺陷必须命中;并报告当前树真实命中。
    TC-GATE-003 (A-08):  桌面壳裸色值扫描器对注入缺陷必须命中;并报告当前树真实命中。
    TC-GATE-004 (§9.7):  桌面壳动效散写 ms 扫描器(Animation::new 直写 from_millis)
                          对注入缺陷必须命中;附当前树真实命中。
    TC-DESK-ICON-002 (A-09): 字形字面量扫描器对注入字形必须命中;并报告当前树真实命中。

运行:python tools/gates/negative_tests.py [--json]
退出码:0=全部反面测试通过(门禁能红,即门禁健在);2=有反面测试失败(门禁腐化)。
结果协议与门禁一致:{"ok": bool, "code": str, "message": str, "data": object}。
"""
from __future__ import annotations

import argparse
import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from tools.gates import m0, run_milestone  # noqa: E402
from tools.gates.a1 import (  # noqa: E402
    _DESKTOP_BARE_COLOR_RX,
    _DESKTOP_GLYPH_RX,
    _DESKTOP_MAGIC_MS_RX,
    _desktop_rs_hits,
)

OK = "OK"
GATE_FAILED = "GATE_FAILED"


def _out(text: str) -> None:
    stream = sys.stdout
    try:
        stream.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[union-attr]
    except Exception:
        pass
    stream.write(text + "\n")


def tc_gate_001() -> tuple[bool, str, dict]:
    """注入不存在路径的反面清单 → M0/license 必须红(经 main() 的退出码映射 = 2 非零)。"""
    with tempfile.TemporaryDirectory(prefix="tc-gate-001-") as td:
        fake = Path(td) / "CORE-FILES"
        fake.write_text(
            "# TC-GATE-001 反面夹具:首条为不存在的路径\n"
            "crates/cutforge-core/src/engine.rs\n"
            "crates/cutforge-core/src/lib.rs\n",
            encoding="utf-8",
        )
        saved = m0.CORE_FILES_PATH
        m0.CORE_FILES_PATH = fake
        try:
            r = m0.check_license()
            # 走完整 main() 拿真实退出码(经 run_milestone → 退出码映射);
            # 内层 gate 的 stdout 就地吸收,避免污染本脚本的 JSON 协议输出
            import io
            from contextlib import redirect_stdout
            from tools.gates import main as gate_main
            buf = io.StringIO()
            with redirect_stdout(buf):
                injected_exit = gate_main(["M0", "--check", "license", "--json"])
            injected_tail = buf.getvalue().strip().splitlines()[-1] if buf.getvalue().strip() else ""
        finally:
            m0.CORE_FILES_PATH = saved
        if r.ok:
            return False, "TC-GATE-001 失败: 注入不存在路径后 check_license 仍 ok(存在性校验失效)", {}
        if "engine.rs" not in r.message:
            return False, f"TC-GATE-001 失败: 红灯信息未点名注入条目: {r.message!r}", {}
        if injected_exit == 0:
            return False, f"TC-GATE-001 失败: 注入后 gate 退出码为 0(应非零): {injected_exit}", {}
    # 绿向:真实清单必须通过
    ok, code, _data = run_milestone("M0", "license")
    if not ok:
        return False, f"TC-GATE-001 失败: 真实清单 M0/license 应通过,得到 code={code}", {}
    return True, (f"TC-GATE-001 通过: 注入不存在路径 → check_license 红 + gate 退出码 {injected_exit}(非零);"
                  f"真实清单 → exit 0(CORE-FILES 存在性校验红绿双向生效)"), {
        "injected_exit": injected_exit,
        "injected_gate_line": injected_tail,
    }


def tc_gate_002() -> tuple[bool, str, dict]:
    """Box::leak 扫描器:注入缺陷的临时目录必须命中;附当前树真实命中报告(报告模式)。"""
    with tempfile.TemporaryDirectory(prefix="tc-gate-002-") as td:
        bad = Path(td) / "fixture.rs"
        bad.write_text("fn f() { let k: &'static str = Box::leak(String::from(\"x\").into_boxed_str()); }\n",
                       encoding="utf-8")
        Path(td, "clean.rs").write_text("fn g() { let s = String::from(\"ok\"); }\n", encoding="utf-8")
        hits = _desktop_rs_hits(r"Box::leak", root=Path(td))
        if [(f, n) for f, n, _s in hits] != [("fixture.rs", 1)]:
            return False, f"TC-GATE-002 失败: 注入 Box::leak 未被精确命中: {hits}", {}
    real = _desktop_rs_hits(r"Box::leak")
    return True, (f"TC-GATE-002 通过: 扫描器对注入缺陷精确命中;当前树真实命中 {len(real)} 处"
                  f"(报告模式列出,待 BUG-18 修复)"), {"seed_hit": True,
                                                  "real_hits": [f"{f}:{n}" for f, n, _s in real]}


def tc_gate_003() -> tuple[bool, str, dict]:
    """裸色值扫描器:注入 hsla 字面量的临时目录必须命中;附当前树真实命中报告(报告模式)。"""
    with tempfile.TemporaryDirectory(prefix="tc-gate-003-") as td:
        bad = Path(td) / "fixture.rs"
        bad.write_text("fn f() { bg(hsla(0.58, 0.55, 0.35, 1.0)) }\n", encoding="utf-8")
        Path(td, "clean.rs").write_text("fn g() { let n = 0u32; }\n", encoding="utf-8")
        hits = _desktop_rs_hits(_DESKTOP_BARE_COLOR_RX, root=Path(td))
        if [(f, n) for f, n, _s in hits] != [("fixture.rs", 1)]:
            return False, f"TC-GATE-003 失败: 注入裸色值未被精确命中: {hits}", {}
    real = _desktop_rs_hits(_DESKTOP_BARE_COLOR_RX)
    return True, (f"TC-GATE-003 通过: 扫描器对注入缺陷精确命中;当前树真实命中 {len(real)} 处"
                  f"(报告模式列出,待 A-08 token 化)"), {"seed_hit": True,
                                                  "real_hits": [f"{f}:{n}" for f, n, _s in real]}


def tc_gate_004() -> tuple[bool, str, dict]:
    """散写动效 ms 扫描器:注入 Animation::new 直写 from_millis 的临时目录必须命中;
    附当前树真实命中报告(报告模式)。"""
    with tempfile.TemporaryDirectory(prefix="tc-gate-004-") as td:
        bad = Path(td) / "fixture.rs"
        bad.write_text(
            "fn f() { el.with_animation(id, Animation::new(Duration::from_millis(200)), |e, d| e) }\n",
            encoding="utf-8",
        )
        Path(td, "clean.rs").write_text(
            "fn g() { let t = Duration::from_millis(100); }\n", encoding="utf-8"
        )
        hits = _desktop_rs_hits(_DESKTOP_MAGIC_MS_RX.pattern, root=Path(td))
        if [(f, n) for f, n, _s in hits] != [("fixture.rs", 1)]:
            return False, f"TC-GATE-004 失败: 注入散写 ms 未被精确命中: {hits}", {}
    real = _desktop_rs_hits(_DESKTOP_MAGIC_MS_RX.pattern)
    return True, (f"TC-GATE-004 通过: 扫描器对注入缺陷精确命中;当前树真实命中 {len(real)} 处"
                  f"(报告模式;动效时长应引用 ui/fx.rs token)"), {"seed_hit": True,
                                                          "real_hits": [f"{f}:{n}" for f, n, _s in real]}


def tc_desk_icon_002() -> tuple[bool, str, dict]:
    """字形字面量扫描器(TC-DESK-ICON-002):注入字形(✂)的临时目录必须命中;
    ×(乘号单位)不在字形集——注入 × 的行必须不命中;附当前树真实命中。"""
    with tempfile.TemporaryDirectory(prefix="tc-desk-icon-002-") as td:
        bad = Path(td) / "fixture.rs"
        bad.write_text('fn f() { btn.child("✂ 分割") }\n', encoding="utf-8")
        Path(td, "unit.rs").write_text('fn g() { label("2.0×") }\n', encoding="utf-8")
        hits = _desktop_rs_hits(_DESKTOP_GLYPH_RX.pattern, root=Path(td))
        if [(f, n) for f, n, _s in hits] != [("fixture.rs", 1)]:
            return False, f"TC-DESK-ICON-002 失败: 注入字形未被精确命中(×误报/漏报): {hits}", {}
    real = _desktop_rs_hits(_DESKTOP_GLYPH_RX.pattern)
    return True, (f"TC-DESK-ICON-002 通过: 字形扫描器对注入精确命中且 × 单位不误报;"
                  f"当前树真实命中 {len(real)} 处"), {"seed_hit": True,
                                            "real_hits": [f"{f}:{n}" for f, n, _s in real]}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="CutForge 门禁反面测试(TC-GATE-001~004 + TC-DESK-ICON-002)")
    parser.add_argument("--json", action="store_true", help="输出 JSON 结果协议")
    args = parser.parse_args(argv)

    results: list[dict] = []
    for name, fn in (
        ("TC-GATE-001", tc_gate_001),
        ("TC-GATE-002", tc_gate_002),
        ("TC-GATE-003", tc_gate_003),
        ("TC-GATE-004", tc_gate_004),
        ("TC-DESK-ICON-002", tc_desk_icon_002),
    ):
        try:
            ok, msg, data = fn()
        except Exception as exc:  # noqa: BLE001
            ok, msg, data = False, f"{name} 异常: {exc!r}", {}
        results.append({"name": name, "ok": ok, "message": msg, "data": data})

    all_ok = all(r["ok"] for r in results)
    envelope = {"ok": all_ok, "code": OK if all_ok else GATE_FAILED,
                "message": ("5 条反面测试全部通过(被测门禁均能正确红灯)"
                            if all_ok else
                            "反面测试失败(被测门禁腐化): " + "; ".join(r["name"] for r in results if not r["ok"])),
                "data": {"results": results}}
    if args.json:
        _out(json.dumps(envelope, ensure_ascii=False, indent=2))
    else:
        _out(f"[{'PASS' if all_ok else 'FAIL'}] negative-tests: {envelope['message']}")
        for r in results:
            _out(f"  {'✓' if r['ok'] else '✗'} {r['name']}: {r['message']}")
    return 0 if all_ok else 2


if __name__ == "__main__":
    sys.exit(main())
