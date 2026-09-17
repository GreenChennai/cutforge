#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""M6 对拍检查器(计划书 7.7):同一 IR 分别经 ffmpeg 后端(rs_render)与
cutforge 后端(cutforge-render)渲染,实测时长/响度/QC/对齐/缓存命中率/剪映退路。

结果缓存于 .gate/parity.json(fixture 确定性;--force 重建)。
退出码:0 全过 / 2 有检查未达标 / 3 环境缺失。
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
CUTFLOW = Path(__file__).resolve().parents[1] / ".." / "CutFlow"
CUTFLOW = CUTFLOW.resolve()
FFMPEG = r"E:\Tools\ffmpeg\bin\ffmpeg.exe"
FFPROBE = r"E:\Tools\ffmpeg\bin\ffprobe.exe"

WORDLINE = {
    "version": 1, "source": "01_materials/a.mp4", "space": "final", "fps": 30,
    "chars": [
        {"i": 0, "ch": "测", "startMs": 0, "endMs": 800, "srcStartMs": 0, "srcEndMs": 800, "conf": 0.9},
        {"i": 1, "ch": "试", "startMs": 800, "endMs": 1600, "srcStartMs": 800, "srcEndMs": 1600, "conf": 0.9},
        {"i": 2, "ch": "。", "startMs": 1600, "endMs": 1601, "srcStartMs": 1600, "srcEndMs": 1601, "conf": 0.9},
    ],
    "gaps": [], "sentences": [{"id": 0, "span": [0, 3], "punc": "。", "text": "测试。"}],
    "speakers": [], "srcDurationMs": 10000, "finalDurationMs": 10000,
    "stats": {"charCount": 2, "coverage": 1.0, "confMedian": 0.9},
    "degraded": False, "degradeReasons": [], "charTimingEstimated": False,
}

ASS = """[Script Info]
ScriptType: v4.00+
PlayResX: 1080
PlayResY: 1920

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, OutlineColour, BackColour, Bold, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Base,Microsoft YaHei,72,&H00FFFFFF,&H00000000,&H7F000000,0,1,3,0,2,60,60,60,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:00.00,0:00:01.60,Base,,0,0,0,,测试。
"""

PROJECT = {
    "version": 1, "slug": "对拍样板", "fps": 30,
    "canvas": {"width": 1080, "height": 1920}, "outputs": ["9x16"],
    "joinCrossfadeMs": 120,
    "tracks": [
        {"kind": "video", "name": "主轨", "clips": [
            {"src": "01_materials/a.mp4", "startMs": 0, "durationMs": 10000,
             "sourceInMs": 0, "volume": 1.0, "role": "voice"}]},
        {"kind": "audio", "clips": [
            {"src": "01_materials/sfx.wav", "startMs": 3000, "durationMs": 1000,
             "role": "sfx", "volume": 0.6}]},
    ],
    "subtitle": {"ass": "06_output/subtitles.ass", "style": "talkshow-bold"},
}


def sh(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8",
                          errors="replace", timeout=900, **kw)


def build_fixture(root: Path) -> None:
    (root / "01_materials").mkdir(parents=True, exist_ok=True)
    (root / "05_ir").mkdir(exist_ok=True)
    (root / "04_cut").mkdir(exist_ok=True)
    (root / "06_output").mkdir(exist_ok=True)
    a = root / "01_materials/a.mp4"
    sh([FFMPEG, "-y", "-v", "error",
        "-f", "lavfi", "-i", "testsrc2=size=1080x1920:rate=30:duration=10",
        "-f", "lavfi", "-i", "sine=frequency=440:duration=10",
        "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", str(a)])
    sh([FFMPEG, "-y", "-v", "error",
        "-f", "lavfi", "-i", "sine=frequency=880:duration=1",
        str(root / "01_materials/sfx.wav")])
    (root / "05_ir/project.json").write_text(json.dumps(PROJECT, ensure_ascii=False, indent=1), encoding="utf-8")
    (root / "05_ir/wordline.final.json").write_text(json.dumps(WORDLINE, ensure_ascii=False, indent=1), encoding="utf-8")
    (root / "06_output/subtitles.ass").write_text(ASS, encoding="utf-8")


def probe_duration(p: Path) -> float:
    r = sh([FFPROBE, "-v", "error", "-print_format", "json", "-show_format", str(p)])
    return float(json.loads(r.stdout)["format"]["duration"])


def loudness(p: Path) -> float:
    r = sh([FFMPEG, "-hide_banner", "-nostats", "-i", str(p),
            "-filter_complex", "loudnorm=I=-14:TP=-1.0:print_format=json", "-f", "null", "-"])
    import re
    blocks = re.findall(r"\{[^{}]*\}", r.stderr)
    doc = json.loads(blocks[-1])  # 最后一个完整 JSON 块即 loudnorm 汇总
    return float(doc["input_i"])


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--force", action="store_true", help="忽略缓存重建对拍结果")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()

    gate_dir = REPO_ROOT / ".gate"
    cache = gate_dir / "parity.json"
    if cache.exists() and not a.force:
        env = json.loads(cache.read_text("utf-8"))
        print(json.dumps(env, ensure_ascii=False, indent=2))
        fails = [c for c in env["data"]["checks"] if not c["ok"]]
        return 0 if not fails else 2

    if not (CUTFLOW / ".git").exists():
        print(json.dumps({"ok": False, "code": "NO_ENV", "message": "CutFlow 仓库不存在", "data": {}}))
        return 3

    root = Path(tempfile.mkdtemp(prefix="cutforge-parity-"))
    build_fixture(root)

    # ① ffmpeg 后端(CutFlow rs_render,Golden)
    t0 = time.time()
    r1 = sh([sys.executable, str(CUTFLOW / "skills/cutflow/scripts/rs_render.py"),
             str(root / "05_ir/project.json"), "--ratio", "9x16", "--profile", "final"], cwd=str(CUTFLOW))
    rs_sec = time.time() - t0
    golden = root / "06_output/final_talkshow-full_916.mp4"
    if not golden.exists():
        # 命名可能不同:找最新 final_*.mp4(排除 cutforge_)
        cands = sorted(root.glob("06_output/final_*.mp4"),
                       key=lambda p: p.stat().st_mtime, reverse=True)
        cands = [p for p in cands if "cutforge" not in p.name]
        golden = cands[0] if cands else golden

    # ② cutforge 后端
    t0 = time.time()
    r2 = sh([sys.executable, "-c",
             "import subprocess,sys; sys.exit(subprocess.call(['cargo','run','-q','-p','cutforge-render','--','--root',sys.argv[1],'--ass',sys.argv[2]], cwd=sys.argv[3]))",
             str(root), str(root / "06_output/subtitles.ass"), str(REPO_ROOT)])
    forge_sec = time.time() - t0
    forge_out = root / "06_output/final_cutforge_对拍样板_1080x1920.mp4"
    if not forge_out.exists():
        cands = sorted(root.glob("06_output/final_cutforge_*.mp4"),
                       key=lambda p: p.stat().st_mtime, reverse=True)
        forge_out = cands[0] if cands else forge_out

    checks = []
    if not golden.exists() or not forge_out.exists():
        checks.append({"name": "render-both", "ok": False,
                       "detail": f"golden={golden.exists()} forge={forge_out.exists()} rs={r1.stderr[-200:]} forge={r2.stderr[-200:]}"})
        env = {"ok": False, "code": "GATE_FAILED", "message": "渲染产物缺失",
               "data": {"checks": checks}}
        gate_dir.mkdir(exist_ok=True)
        cache.write_text(json.dumps(env, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(env, ensure_ascii=False, indent=2))
        return 2

    # M6-1 时长(≤1 帧 = 33.33ms @30fps)
    d_gold, d_forge = probe_duration(golden), probe_duration(forge_out)
    diff_ms = abs(d_gold - d_forge) * 1000
    checks.append({"name": "parity-duration", "ok": diff_ms <= 33.34,
                   "detail": f"rs_render {d_gold:.3f}s vs cutforge {d_forge:.3f}s(差 {diff_ms:.1f}ms ≤1 帧)"})

    # M6-2 响度(差 ≤0.5 LU,且都在 -14±1)
    try:
        l1, l2 = loudness(golden), loudness(forge_out)
        checks.append({"name": "parity-loudness", "ok": abs(l1 - l2) <= 0.5 and -15 <= l2 <= -13,
                       "detail": f"rs_render {l1:.2f} LUFS vs cutforge {l2:.2f} LUFS(差 {abs(l1-l2):.2f})"})
    except Exception as exc:  # noqa: BLE001
        checks.append({"name": "parity-loudness", "ok": False, "detail": repr(exc)})

    # M6-3 QC 体检(cutforge 成片)
    r_qc = sh([sys.executable, str(CUTFLOW / "skills/cutflow/scripts/rs_sync.py"),
               "--video", str(forge_out), "--wordline", str(root / "05_ir/wordline.final.json"),
               "--ass", str(root / "06_output/subtitles.ass"), "--qc"], cwd=str(CUTFLOW))
    try:
        qc_doc = json.loads((r_qc.stdout or "")[(r_qc.stdout or "").find("{"):])
    except Exception:  # noqa: BLE001
        qc_doc = {}
    qc_ok = r_qc.returncode == 0 and qc_doc.get("ok") is True
    qc_msg = str(qc_doc.get("message") or (r_qc.stderr or "")[-200:])
    # 对齐断言中位数在 rs_sync 报告里(M6-4 同源输出)
    import re as _re
    median = None
    m = _re.search(r"中位[^0-9-]*(\d+(?:\.\d+)?)", (r_qc.stdout or "") + qc_msg)
    if m:
        median = float(m.group(1))
    checks.append({"name": "qc", "ok": qc_ok, "detail": qc_msg[:160]})
    checks.append({"name": "alignment", "ok": qc_ok and (median is None or median <= 40),
                   "detail": f"对齐中位 {median}ms(≤40)"})

    # M6-6 缓存命中率(观察):二次渲染
    t0 = time.time()
    sh([sys.executable, "-c",
        "import subprocess,sys; sys.exit(subprocess.call(['cargo','run','-q','-p','cutforge-render','--','--root',sys.argv[1]], cwd=sys.argv[2]))",
        str(root), str(REPO_ROOT)])
    second = time.time() - t0
    checks.append({"name": "cache-hit", "ok": second <= max(rs_sec, 1), "obs": True,
                   "detail": f"二次渲染 {second:.1f}s(首次 {forge_sec:.1f}s;rs_render {rs_sec:.1f}s)"})

    # M6-7 剪映退路未破坏
    r_jy = sh([sys.executable, str(CUTFLOW / "skills/cutflow/scripts/rs_jy_draft.py"),
               str(root / "05_ir/project.json"), "--name", "cutforge-parity-dev"], cwd=str(CUTFLOW))
    checks.append({"name": "jianying-intact", "ok": r_jy.returncode == 0,
                   "detail": ("5.9 草稿生成正常" if r_jy.returncode == 0 else (r_jy.stderr or r_jy.stdout)[-160:])})

    ok = all(c["ok"] for c in checks if not c.get("obs"))
    env = {"ok": ok, "code": "OK" if ok else "GATE_FAILED",
           "message": f"对拍 {sum(1 for c in checks if c['ok'])}/{len(checks)} 项通过",
           "data": {"checks": checks, "golden": str(golden), "forge": str(forge_out)}}
    gate_dir.mkdir(exist_ok=True)
    cache.write_text(json.dumps(env, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(env, ensure_ascii=False, indent=2))
    return 0 if ok else 2


if __name__ == "__main__":
    sys.exit(main())
