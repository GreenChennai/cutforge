#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""CutForge 统一门禁入口(兼容壳)。

实现已按 A-10 拆分为 tools/gates/ 包(m0…m7、a1…a7 + common + __init__ + __main__),
拆分为纯移动、行为零变化;本文件保留为兼容入口,re-export 包的全部公开名:

    python tools/gates/gate.py M<n> [--check NAME] [--json]
    python -m tools.gates M<n>            # 等价模块入口

结果协议与退出码不变:0=通过;2=门禁失败;3=前置或环境缺失;4=内部错误。

门禁工程纪律(§8.2,全文见 CONTRIBUTING.md「门禁工程纪律」):
1. 宣称即证据——结论必须附可复现命令 + 退出码 + 关键输出;
2. 门禁要有反面测试——python tools/gates/negative_tests.py(TC-GATE-001~004 + TC-DESK-ICON-002);
3. 门禁扫描面显式化——crates/800、apps/desktop/600、apps/web js/400 分列,登记在 CONTRIBUTING.md。
"""
from __future__ import annotations

import sys
from pathlib import Path

# 以脚本方式直跑(python tools/gates/gate.py)时,sys.path[0]=tools/gates,
# 需把仓库根加入才能 import tools.gates(命名空间包)。
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from tools.gates import *  # noqa: F401,F403
from tools.gates import main

if __name__ == "__main__":
    sys.exit(main())
