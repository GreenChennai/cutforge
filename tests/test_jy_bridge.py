# -*- coding: utf-8 -*-
"""阶段六 J6 门禁:export_jianying ↔ CutFlow rs_jy_draft.py 出口对拍(副文档 06 §6 判据 6)。

对拍口径(务实双层):
  1. 源级:lib.rs 的编排组把 export_jianying 落到**同一个** rs_jy_draft.py,
     scriptArgs 原样透传、不注旗标(映射真相只在 CutFlow 一处,两处不得各自漂移);
  2. 行级:对真实夹具 IR 以 export_jianying 的调用形态(cwd=工程根,
     scriptArgs=['05_ir/project.json','--name','x','--dry-run'])真跑该脚本,
     断言与 CutFlow 侧同一套编译层/门禁语义(JY_PLAN_DRY_RUN / PLAN_GATE_FAIL)。

弱依赖:无 CutFlow 仓库时 skip(CI python-gates job 已设 CUTFLOW_REPO)。
运行:python -m pytest tests/test_jy_bridge.py -q
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
CUTFLOW = Path(os.environ.get("CUTFLOW_REPO", ROOT.parent / "CutFlow"))
SCRIPTS = CUTFLOW / "skills" / "cutflow" / "scripts"
LIB_RS = ROOT / "crates" / "cutforge-mcp" / "src" / "lib.rs"

pytestmark = [
    pytest.mark.skipif(not (SCRIPTS / "rs_jy_draft.py").is_file(),
                       reason="CutFlow 仓库不可用(CUTFLOW_REPO 未设且无同级目录)"),
    pytest.mark.skipif(not LIB_RS.is_file(), reason="cutforge-mcp 源码缺失"),
]


def run_jy_draft(*args: str, cwd: Path) -> subprocess.CompletedProcess:
    """以 orchestrate 同款形态调 rs_jy_draft.py(UTF-8 + 超时,子进程纪律)。"""
    return subprocess.run(
        [sys.executable, str(SCRIPTS / "rs_jy_draft.py"), *args],
        cwd=cwd, capture_output=True, text=True, encoding="utf-8", errors="replace",
        env={**os.environ, "PYTHONUTF8": "1"}, timeout=600)


def last_json(text: str) -> dict:
    lines = [ln for ln in text.splitlines() if ln.strip().startswith("{")]
    assert lines, f"无 JSON 输出:\n{text[-2000:]}"
    return json.loads(lines[-1])


# ---------------------------------------------------------------- 源级对拍

def test_export_jianying_dispatch_maps_to_same_script():
    """lib.rs:export_jianying 与 stage_run 等同组,统一落到 rs_jy_draft.py。"""
    src = LIB_RS.read_text(encoding="utf-8")
    m = re.search(r'"stage_run"[^\n]*\n(?:.*\n){0,12}?.*_ => "rs_jy_draft\.py"', src)
    assert m, "export_jianying 编排组必须落 rs_jy_draft.py(同一脚本同一映射)"
    assert "export_jianying" in m.group(0), "export_jianying 必须在同一编排组内"


def test_export_jianying_passthrough_no_flag_injection():
    """scriptArgs 原样透传;--json 白名单仅 rs_verify(不给 rs_jy_draft 注旗标)。"""
    src = LIB_RS.read_text(encoding="utf-8")
    assert 'let supports_json = matches!(script, "rs_verify.py");' in src
    # 编排 = 子进程跑脚本 + 透传(CutFlow 码表原样回传),不重实现映射
    assert 'fn orchestrate(ws_root: &Path, script: &str, script_args: &[Value])' in src


def test_schema_description_points_to_same_script():
    """schemas/mcp-tools.json:export_jianying 的描述必须指向 rs_jy_draft.py。"""
    schema = json.loads((ROOT / "schemas" / "mcp-tools.json").read_text(encoding="utf-8"))
    tool = next(t for t in schema["tools"] if t["name"] == "export_jianying")
    assert "rs_jy_draft.py" in tool["description"]
    assert tool["kind"] == "orchestrate"


# ---------------------------------------------------------------- 行级对拍

def test_bridge_invocation_dry_run_same_semantics(tmp_path):
    """夹具 IR(cutforge real_ir)以 export_jianying 调用形态真跑 --dry-run:
    与 CutFlow 同一层编译语义 —— 门禁失败给 PLAN_GATE_FAIL 与逐条清单(退出码 4),
    成功给 JY_PLAN_DRY_RUN(退出码 0)。"""
    ws = tmp_path / "ws"
    (ws / "05_ir").mkdir(parents=True)
    (ws / "05_ir" / "project.json").write_text(
        (ROOT / "tests" / "fixtures" / "real_ir" / "project.json").read_text(encoding="utf-8"),
        encoding="utf-8")
    p = run_jy_draft("05_ir/project.json", "--name", "dev_bridge", "--dry-run", cwd=ws)
    doc = last_json(p.stdout)
    # real_ir 夹具不带真实素材 → 门禁必须拦下(结构正确 ≠ 素材在盘),这正是新门禁的语义
    assert p.returncode == 4 and doc["code"] == "PLAN_GATE_FAIL", doc
    assert any("素材不存在" in g for g in doc["data"]["gates"]), doc["data"]["gates"]
    # 映射表照印(人读排障),且是「IR 片段 → 草稿片段」形态
    assert "草稿计划映射" in p.stdout and "V1-001" in p.stdout


def test_bridge_invocation_green_path_with_real_media(tmp_path):
    """有 ffmpeg 时:真素材工程 → JY_PLAN_DRY_RUN;同一 IR 二跑计划逐键一致(纯函数)。"""
    ff = None
    for cand in ("ffmpeg", str(Path(os.environ.get("CUTFORGE_FFPROBE", "")) / "ffmpeg.exe")):
        got = None
        try:
            from shutil import which
            got = which(cand) or (cand if Path(cand).is_file() else None)
        except Exception:  # noqa: BLE001
            got = None
        if got:
            ff = got
            break
    if not ff:
        pytest.skip("本机没有 ffmpeg,无法造真素材(绿灯路径由 CI/本机具备时覆盖)")
    ws = tmp_path / "ws"
    (ws / "01_materials").mkdir(parents=True)
    (ws / "05_ir").mkdir(parents=True)
    a = ws / "01_materials" / "a.mp4"
    subprocess.run([ff, "-y", "-v", "error",
                    "-f", "lavfi", "-i", "color=c=red:s=320x240:d=4:r=30",
                    "-f", "lavfi", "-i", "sine=frequency=440:d=4",
                    "-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p",
                    "-c:a", "aac", str(a)], check=True, timeout=300)
    ir = {"version": 1, "slug": "dev-bridge", "fps": 30,
          "canvas": {"width": 1080, "height": 1920},
          "tracks": [{"kind": "video", "clips": [
              {"src": "01_materials/a.mp4", "startMs": 0, "durationMs": 4000,
               "sourceInMs": 0}]}]}
    (ws / "05_ir" / "project.json").write_text(json.dumps(ir, ensure_ascii=False),
                                               encoding="utf-8")
    plans = []
    for _ in (1, 2):
        p = run_jy_draft("05_ir/project.json", "--name", "dev_bridge", "--dry-run", cwd=ws)
        doc = last_json(p.stdout)
        assert p.returncode == 0 and doc["code"] == "JY_PLAN_DRY_RUN", doc
        plans.append(doc["data"]["plan"])
    assert plans[0] == plans[1], "同输入必得同一草稿计划(编译层纯函数口径)"
    assert plans[0]["expects"] == {"trackCount": 1, "segmentCount": 1,
                                   "mainDurationFrames": 120}
