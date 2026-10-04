"""CutForge 统一门禁 · 共享基建(常量/结果协议/IO 兜底)。

A-10 拆包:自 gate.py 原样移入,行为零变化。REPO_ROOT 口径不变
(tools/gates/<file> 的 parents[2] = 仓库根)。
"""
from __future__ import annotations

import os
import sys
from pathlib import Path



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
