#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""批处理产物校验器(CI 自动出片样例;给用户抄的最小工作流,零第三方依赖)。

    python tools/ci-example/check_output.py <batch-report.json> \
        [--expect-duration-ms N] [--tolerance-ms 800] [--sample-at-ms 1000]

读 cutforge-cli batch 的报告(docs/schemas/batch-report.schema.json),对每个成功 job:
  1. 产物存在;
  2. 时长校验:ffprobe 实测时长 vs --expect-duration-ms(±容差);报告未给期望值时跳过;
  3. 像素抽样:抽样时刻抽一帧,断言非全黑(渲染链路真实出画的下限证明)。

退出码:0 全过 / 2 有失败 / 3 依赖缺失(ffmpeg/ffprobe)。
"""
from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")


def ffprobe_duration_ms(video: Path) -> int:
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration",
         "-of", "default=noprint_wrappers=1:nokey=1", str(video)],
        capture_output=True, text=True, check=True)
    return int(round(float(out.stdout.strip()) * 1000))


def frame_not_black(video: Path, at_ms: int) -> bool:
    """抽一帧,断言平均亮度非零(全黑 = 渲染链路未真实出画)。"""
    with tempfile.TemporaryDirectory() as td:
        frame = Path(td) / "f.png"
        subprocess.run(
            ["ffmpeg", "-y", "-loglevel", "error", "-ss", f"{at_ms/1000:.3f}",
             "-i", str(video), "-frames:v", "1", str(frame)],
            capture_output=True, check=True)
        # 统计平均亮度(signalstats YAVG)
        out = subprocess.run(
            ["ffmpeg", "-hide_banner", "-i", str(frame), "-vf", "signalstats,metadata=print",
             "-f", "null", "-"],
            capture_output=True, text=True)
        for line in (out.stderr or "").splitlines():
            if "YAVG" in line:
                return float(line.split("YAVG=")[1].split()[0]) > 1.0
    return False


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("report", help="batch-report.json")
    ap.add_argument("--expect-duration-ms", type=int, default=None)
    ap.add_argument("--tolerance-ms", type=int, default=800)
    ap.add_argument("--sample-at-ms", type=int, default=1000)
    args = ap.parse_args()

    if not shutil.which("ffprobe") or not shutil.which("ffmpeg"):
        print("FAIL: ffmpeg/ffprobe 不在 PATH", file=sys.stderr)
        return 3
    report = json.loads(Path(args.report).read_text(encoding="utf-8"))
    jobs = [j for j in report.get("jobs", []) if j.get("ok")]
    if not jobs:
        print("FAIL: 报告内无成功 job,无产物可校验")
        return 2
    failures = 0
    for job in jobs:
        output = job.get("output")
        name = job.get("name") or job.get("project")
        if not output or not Path(output).is_file():
            print(f"FAIL {name}: 产物缺失 {output}")
            failures += 1
            continue
        dur = ffprobe_duration_ms(Path(output))
        line = f"PASS {name}: {output}({dur}ms)"
        if args.expect_duration_ms is not None:
            drift = abs(dur - args.expect_duration_ms)
            if drift > args.tolerance_ms:
                print(f"FAIL {name}: 时长 {dur}ms 偏离期望 {args.expect_duration_ms}ms"
                      f"(±{args.tolerance_ms}ms)")
                failures += 1
                continue
            line += f" 时长±{drift}ms 达标"
        if not frame_not_black(Path(output), args.sample_at_ms):
            print(f"FAIL {name}: 抽样帧全黑(@{args.sample_at_ms}ms)")
            failures += 1
            continue
        line += f" 像素抽样(@{args.sample_at_ms}ms)非黑帧"
        print(line)
    if failures:
        print(f"FAIL: {failures} 项未过")
        return 2
    print(f"OK: {len(jobs)} 个产物全部过校验(时长/像素抽样)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
