#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""转场/特效缩略帧生成器(册四 T4.5/T4.6;计划书 4.C:剪映式缩略预览,抽帧生成)。

    python tools/gen_transition_thumbs.py                    # 全量:转场 58 + 特效 11
    python tools/gen_transition_thumbs.py --kind transition  # 仅转场
    python tools/gen_transition_thumbs.py --kind fx          # 仅特效
    python tools/gen_transition_thumbs.py --only fade,dissolve --out <dir>

机制(不依赖渲染服务,直接 ffmpeg):
  - 转场:两路色卡(testsrc2 左青右暖 双半幅)各 1.2s,xfade duration=0.8,在
    转场中点抽帧 → 240x135 JPEG(质量 70),文件名 = 目录 id;
  - 特效:testsrc2 2s 挂 fx.combo 模板(参数取目录默认;{W} 等上下文本地代入,
    {L} 标签 fx0),在 t=1.0 抽帧 → 同规格 JPEG,文件名 = fxId;
  - 附 index.json(id → 文件/分类/中文名),壳缩略预览网格按需取用(FE 接线)。

产物体积口径:单帧 JPEG ≈ 6-10KB,全量 69 张 ≈ 0.5MB(≤5MB 预算);重跑覆盖。
真相源 = schemas/transition-catalog.json + schemas/fx-catalog.json(本脚本只消费,
不内嵌目录副本)。ffmpeg 定位:PATH 或 CUTFORGE_FFMPEG。
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
OUT_DEFAULT = REPO / "docs" / "design" / "transitions"
W, H = 240, 135  # 16:9 缩略规格
FMT = ["-vf", f"scale={W}:{H}", "-q:v", "7"]

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")


def ffmpeg_bin() -> str:
    import os
    v = os.environ.get("CUTFORGE_FFMPEG")
    return v if v else "ffmpeg"


def sh(cmd: list[str]) -> None:
    r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg 失败: {r.stderr[-400:]}")


def sample_pair(tmp: Path) -> tuple[Path, Path]:
    """两路样片:左半青色 + 右半暖色的移动 testsrc2(转场语义可读:两侧内容不同)。"""
    a, b = tmp / "a.mp4", tmp / "b.mp4"
    sh([ffmpeg_bin(), "-y", "-v", "error",
        "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=1.2",
        "-vf", "hue=h=120", "-c:v", "libx264", "-preset", "veryfast", str(a)])
    sh([ffmpeg_bin(), "-y", "-v", "error",
        "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=1.2",
        "-vf", "hue=h=-60", "-c:v", "libx264", "-preset", "veryfast", str(b)])
    return a, b


def gen_transition(out: Path, tmp: Path, tid: str, a: Path, b: Path) -> None:
    """xfade duration=0.8 offset=0.4 → 转场窗 [0.4,1.2),中点 0.8 抽帧。"""
    # -vf 与 -filter_complex 互斥:缩放并入图;输出侧 -ss 0.8 = 转场窗中点抽帧
    sh([ffmpeg_bin(), "-y", "-v", "error", "-i", str(a), "-i", str(b),
        "-filter_complex", f"[0:v][1:v]xfade=transition={tid}:duration=0.8:offset=0.4,scale={W}:{H}[v]",
        "-map", "[v]", "-ss", "0.8", "-frames:v", "1", "-q:v", "7", str(out)])


def substitute(template: str, params: dict, label: str) -> str:
    ctx = {"W": W, "H": H, "FPS": 30, "D": "0.5", "ST": "0.5", "N": "15", "L": label}
    text = template.replace("{W}", str(W)).replace("{H}", str(H)).replace("{FPS}", "30")
    text = text.replace("{D}", "0.5").replace("{ST}", "0.5").replace("{N}", "15").replace("{L}", label)
    for name, spec in params.items():
        ctx[name] = str(spec.get("default", 0))
        text = text.replace("{" + name + "}", ctx[name])
    return text


def gen_fx(out: Path, tmp: Path, fx_id: str, entry: dict) -> None:
    src = tmp / "fxsrc.mp4"
    if not src.exists():
        sh([ffmpeg_bin(), "-y", "-v", "error",
            "-f", "lavfi", "-i", "testsrc2=size=240x135:rate=30:duration=2", "-c:v",
            "libx264", "-preset", "veryfast", str(src)])
    chain = substitute(entry["filter"], {p["name"]: p for p in entry.get("params", [])}, "fx0")
    sh([ffmpeg_bin(), "-y", "-v", "error", "-i", str(src),
        "-vf", chain, "-ss", "1.0", "-frames:v", "1", *FMT, str(out)])


def main() -> int:
    ap = argparse.ArgumentParser(description="转场/特效缩略帧生成器(册四 T4.5/T4.6)")
    ap.add_argument("--kind", choices=["transition", "fx", "all"], default="all")
    ap.add_argument("--out", type=Path, default=OUT_DEFAULT)
    ap.add_argument("--only", type=str, default="", help="逗号分隔 id 过滤")
    a = ap.parse_args()
    only = {x.strip() for x in a.only.split(",") if x.strip()}

    tr_doc = json.loads((REPO / "schemas" / "transition-catalog.json").read_text("utf-8"))
    fx_doc = json.loads((REPO / "schemas" / "fx-catalog.json").read_text("utf-8"))
    a.out.mkdir(parents=True, exist_ok=True)
    index: dict[str, dict] = {}
    total_bytes = 0

    with tempfile.TemporaryDirectory(prefix="cf-thumbs-") as td:
        tmp = Path(td)
        if a.kind in ("transition", "all"):
            pa, pb = sample_pair(tmp)
            for t in tr_doc["transitions"]:
                tid = t["id"]
                if only and tid not in only:
                    continue
                f = a.out / f"{tid}.jpg"
                gen_transition(f, tmp, tid, pa, pb)
                index[tid] = {"file": f.name, "kind": "transition", "category": t["category"], "name": t["name"]}
                total_bytes += f.stat().st_size
        if a.kind in ("fx", "all"):
            for e in fx_doc["fx"]:
                fid = e["id"]
                if only and fid not in only:
                    continue
                f = a.out / f"{fid}.jpg"
                gen_fx(f, tmp, fid, e)
                index[fid] = {"file": f.name, "kind": "fx", "category": e["category"], "name": e["name"]}
                total_bytes += f.stat().st_size

    (a.out / "index.json").write_text(
        json.dumps({"version": 1, "spec": f"{W}x{H} jpeg", "items": index}, ensure_ascii=False, indent=1),
        encoding="utf-8")
    print(f"[OK] 生成 {len(index)} 张缩略帧 → {a.out}(共 {total_bytes / 1024:.0f} KB,预算 ≤5MB)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
