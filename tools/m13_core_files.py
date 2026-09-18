#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""M13:ARL-CORE 头补齐 + CORE-FILES 清单 + 版本号。"""
from pathlib import Path

HEADER = "// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)\n"
count = 0
for rs in sorted(Path("crates").glob("*/src/**/*.rs")):
    text = rs.read_text(encoding="utf-8")
    if "ARL-CORE" in text:
        continue
    rs.write_text(HEADER + text, encoding="utf-8", newline="\n")
    count += 1
print("headers added:", count)

BS = chr(92)
files = []
for rs in sorted(Path("crates").glob("*/src/**/*.rs")):
    files.append(str(rs).replace(BS, "/"))
for j in sorted(Path("schemas").glob("*.json")):
    p = str(j).replace(BS, "/")
    if p not in files:
        files.append(p)
for j in ["schemas/mcp-tools.json", "docs/capability-matrix.json"]:
    if Path(j).is_file() and j not in files:
        files.append(j)
Path("CORE-FILES").write_text(
    "# CutForge 核心文件清单(ARL-1.0 LICENSE 1.3 ③ 口径;随版本更新调整,不追溯既往版本)\n"
    "# 覆盖:契约(schema)、内核(core)、IO/合并、MCP 协议、渲染、脚本宿主、wasm、CLI。\n"
    + "\n".join(files) + "\n",
    encoding="utf-8", newline="\n")
print("CORE-FILES entries:", len(files))

ct = Path("Cargo.toml")
s = ct.read_text(encoding="utf-8")
s = s.replace('version = "0.1.0"', 'version = "0.2.0"')
ct.write_text(s, encoding="utf-8", newline="\n")
print("version bumped")
