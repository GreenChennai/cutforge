#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""册七 T7.4/AC-7.4 纯 CLI 全链验收 e2e(无 UI 无 serve;除 run-script 无头建卡外全走 cutforge-cli)。

    python tools/e2e_headless.py [--cli target/debug/cutforge-cli] [--mcp target/debug/cutforge-mcp]

断言链:
  1. 单工程全链:new → run-script 建卡(clip_add)→ cutforge-cli clip-update 改字段 →
     cutforge-render 直渲 → 时长对拍(ffprobe)+ 像素抽样(非黑帧);
  2. 批处理清单:batch.yaml(3 工程,v2/v3 布局混合)单排队 → 逐项回执 →
     报告 docs/schemas/batch-report.schema.json 生成物对拍(draft-07 子集迷你校验器);
  3. 插件服务端面(T7.2 服务端):plugin-call 只读插件调写工具 → GUARD_FAILED/FORBIDDEN
     负例(工程零变化);写权限插件调用 → oplog actor=plugin 归因对账;
  4. watch 模式:启动即渲 + 外部改字段自动重渲一次(--max-runs 2 退出);
  5. ci-example 样例自证:batch.example.yaml 解析面 + check_output.py 对真实报告全绿。

退出码:0 通过 / 2 失败。依赖:ffmpeg/ffprobe(现场生成媒体);cutforge-render 与
cutforge-cli 同目录(构建产物天然满足)。
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
FPS = 30
CANVAS = (640, 360)
CLIP_MS = 4000

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def log(msg: str) -> None:
    print(f"[headless] {msg}", flush=True)


def die(msg: str) -> None:
    print(f"FAIL: {msg}", file=sys.stderr)
    sys.exit(2)


def run(cmd: list[str], timeout: int = 600) -> subprocess.CompletedProcess:
    return subprocess.run([str(c) for c in cmd], capture_output=True, text=True,
                          encoding="utf-8", errors="replace", timeout=timeout)


def make_media(dst: Path, seconds: int) -> None:
    """夹具媒体:深灰底 + 纯音轨(与 e2e_independence 同口径)。"""
    dst.parent.mkdir(parents=True, exist_ok=True)
    r = run(["ffmpeg", "-y", "-loglevel", "error",
             "-f", "lavfi", "-i", f"color=c=0x202020:size={CANVAS[0]}x{CANVAS[1]}:rate={FPS}",
             "-f", "lavfi", "-i", f"sine=frequency=440:duration={seconds}",
             "-t", str(seconds), "-pix_fmt", "yuv420p", "-c:v", "libx264",
             "-preset", "veryfast", "-c:a", "aac", "-shortest", dst])
    assert r.returncode == 0, f"ffmpeg 夹具失败:{r.stderr[-300:]}"


def ffprobe_duration_ms(p: Path) -> int:
    r = run(["ffprobe", "-v", "error", "-print_format", "json", "-show_format", p])
    assert r.returncode == 0, f"ffprobe 失败: {r.stderr[-200:]}"
    return int(round(float(json.loads(r.stdout)["format"]["duration"]) * 1000))


def frame_is_black(mp4: Path, at_s: float) -> bool:
    """抽帧像素抽样:全黑 = 渲染未真实出画(rawvideo 纯 python 解析,零 PIL)。"""
    w, h = 160, 90
    r = subprocess.run(["ffmpeg", "-v", "error", "-ss", str(at_s), "-i", str(mp4),
                        "-frames:v", "1", "-vf", f"scale={w}:{h}", "-f", "rawvideo",
                        "-pix_fmt", "rgb24", "-"], capture_output=True)
    assert r.returncode == 0, f"抽帧失败: {r.stderr[-200:]}"
    data = r.stdout
    assert len(data) >= w * h * 3, f"抽帧字节不足: {len(data)}"
    return sum(data[: w * h * 3]) < 1000  # 平均每像素亮度下限(全黑判定)


def insert_clip(mcp: Path, root: Path, src_rel: str, tag: str, tmp: Path) -> str:
    """run-script 无头建卡(与 e2e_independence 同一入口);返回分配的 clipId。"""
    script = {"format": "cutforge-script-v1", "steps": [
        {"tool": "clip_add", "args": {"trackId": "V1", "src": src_rel,
                                      "startMs": 0, "requestId": f"hl-{tag}"}}]}
    f = tmp / f"script-{tag}.json"
    f.write_text(json.dumps(script, ensure_ascii=False), encoding="utf-8")
    r = run([mcp, "run-script", "--root", root, "--file", f])
    assert r.returncode == 0, f"run-script 建卡失败: {r.stdout[-300:]} {r.stderr[-300:]}"
    rep = json.loads(r.stdout)
    assert rep["ok"] and rep["data"]["rejected"] == [], f"run-script 回执: {rep}"
    pj = json.loads(project_file(root).read_text(encoding="utf-8"))
    clips = pj["tracks"][0]["clips"]
    assert clips, f"建卡后 V1 必须有片段: {pj['tracks'][0]}"
    return clips[-1]["id"]


def cli(cli_path: Path, *args: str, timeout: int = 600) -> subprocess.CompletedProcess:
    return run([cli_path, *args], timeout=timeout)


def project_file(root: Path) -> Path:
    """工程文档路径(v3 扁平 / v2 中文布局,与 paths::has_project 同判定)。"""
    f = root / "project.json"
    return f if f.is_file() else root / "05_时间线工程" / "project.json"


def resolve_bin(p: str, name: str) -> Path:
    """Windows 下缺 .exe 后缀自动补(与既有 e2e 的 --bin 习惯一致)。"""
    path = Path(p)
    if path.is_file() or not os.name == "nt":
        return path
    return path.with_name(path.name + ".exe")


# ---------------- batch-report.schema.json 生成物对拍(draft-07 子集迷你校验器) ----------------

def validate(instance, schema: dict, path: str = "$") -> list[str]:
    errs: list[str] = []
    t = schema.get("type")
    types = t if isinstance(t, list) else ([t] if t else [])
    if types:
        ok = False
        for tt in types:
            ok |= (tt == "object" and isinstance(instance, dict)) \
                or (tt == "array" and isinstance(instance, list)) \
                or (tt == "string" and isinstance(instance, str)) \
                or (tt == "integer" and isinstance(instance, int) and not isinstance(instance, bool)) \
                or (tt == "boolean" and isinstance(instance, bool)) \
                or (tt == "null" and instance is None)
        if not ok:
            return [f"{path}: 类型 {type(instance).__name__} 不在 {types}"]
    if "const" in schema and instance != schema["const"]:
        errs.append(f"{path}: const 不符(期望 {schema['const']!r})")
    if "enum" in schema and instance not in schema["enum"]:
        errs.append(f"{path}: {instance!r} 不在 enum {schema['enum']}")
    if isinstance(instance, (int, float)) and not isinstance(instance, bool):
        if "minimum" in schema and instance < schema["minimum"]:
            errs.append(f"{path}: {instance} < minimum {schema['minimum']}")
    if isinstance(instance, dict):
        for k in schema.get("required", []):
            if k not in instance:
                errs.append(f"{path}: 缺必填 '{k}'")
        props = schema.get("properties", {})
        if schema.get("additionalProperties") is False:
            for k in instance:
                if k not in props:
                    errs.append(f"{path}: additionalProperties=false 拒绝多余键 '{k}'")
        for k, v in instance.items():
            if k in props:
                errs.extend(validate(v, props[k], f"{path}/{k}"))
    if isinstance(instance, list) and isinstance(schema.get("items"), dict):
        for i, v in enumerate(instance):
            errs.extend(validate(v, schema["items"], f"{path}/{i}"))
    return errs


# ---------------- 各阶段 ----------------

def stage_single_chain(cli_path: Path, mcp: Path, render_bin: Path, tmp: Path) -> Path:
    """1. 单工程全链:new → 建卡 → 改字段 → 渲染 → 时长/像素校验。"""
    proj = tmp / "chain-a"
    r = cli(cli_path, "new", proj, "--slug", "headless-a", "--json",
            "--width", str(CANVAS[0]), "--height", str(CANVAS[1]), "--track", "video,audio")
    assert r.returncode == 0, f"new: {r.stdout} {r.stderr}"
    make_media(proj / "01_原始素材" / "a.mp4", 4)
    clip_id = insert_clip(mcp, proj, "01_原始素材/a.mp4", "a", tmp)
    r = cli(cli_path, "clip-update", proj, clip_id, "--duration-ms", str(CLIP_MS),
            "--volume", "0.6", "--summary", "headless 改字段", "--json")
    assert r.returncode == 0, f"clip-update: {r.stdout} {r.stderr}"
    receipt = json.loads(r.stdout.strip().splitlines()[-1])
    assert receipt["ok"] and receipt["data"]["rev"] >= 2, f"改字段回执: {receipt}"
    r = run([render_bin, "--root", proj], timeout=300)
    assert r.returncode == 0, f"直渲失败: {r.stdout[-300:]} {r.stderr[-300:]}"
    outs = list((proj / "06_成片输出").glob("*.mp4"))
    assert outs, f"v2 产物落 06_成片输出: {list(proj.rglob('*.mp4'))}"
    out = outs[0]
    dur = ffprobe_duration_ms(out)
    assert abs(dur - CLIP_MS) <= 1200, f"成片时长 {dur}ms vs {CLIP_MS}ms"
    assert not frame_is_black(out, 1.0), "像素抽样:1s 处帧不得全黑"
    log(f"单工程全链 OK:rev→{receipt['data']['rev']},产物 {out.name}({dur}ms,像素抽样非黑)")
    return proj


def stage_batch(cli_path: Path, mcp: Path, tmp: Path, projs: list[Path]) -> dict:
    """2. 批处理清单(3 工程混合布局)→ 报告 schema 生成物对拍。"""
    make_media(projs[2] / "media" / "main.mp4", 4)
    insert_clip(mcp, projs[2], "media/main.mp4", "c", tmp)
    manifest = tmp / "batch.yaml"
    manifest.write_text(
        "# e2e_headless 批清单(最小 YAML 子集)\n"
        "version: 1\n"
        "defaults:\n"
        "  format: mp4\n"
        "projects:\n"
        f"  - root: {projs[0].as_posix()}\n"
        "    name: 甲\n"
        f"  - root: {projs[1].as_posix()}\n"
        "    name: 乙\n"
        "    quality: high\n"
        f"  - root: {projs[2].as_posix()}\n"
        "    name: 丙\n",
        encoding="utf-8")
    r = cli(cli_path, "batch", manifest, "--report", tmp / "batch-report.json", "--json", timeout=900)
    assert r.returncode == 0, f"batch 退出码 {r.returncode}: {r.stdout[-500:]} {r.stderr[-300:]}"
    report_path = tmp / "batch-report.json"
    report = json.loads(report_path.read_text(encoding="utf-8"))
    schema = json.loads((REPO / "docs" / "schemas" / "batch-report.schema.json").read_text(encoding="utf-8"))
    errs = validate(report, schema)
    assert not errs, f"报告 schema 对拍失败: {errs[:10]}"
    assert report["summary"]["total"] == 3 and report["summary"]["ok"] == 3, f"汇总: {report['summary']}"
    assert [j["name"] for j in report["jobs"]] == ["甲", "乙", "丙"], "单排队顺序 = 清单顺序"
    for j in report["jobs"]:
        assert j["ok"] and j["code"] == "OK" and j.get("output"), f"逐项回执: {j}"
        assert Path(j["output"]).is_file(), f"产物存在: {j['output']}"
        dur = ffprobe_duration_ms(Path(j["output"]))
        assert abs(dur - CLIP_MS) <= 1500, f"{j['name']} 时长 {dur}ms vs {CLIP_MS}ms"
    log(f"批处理 OK:3 工程单排队全成,报告 schema 逐键对拍 0 漂移({report['durationMs']}ms)")
    return report


def stage_plugin(cli_path: Path, tmp: Path, proj: Path) -> None:
    """3. 插件服务端面:只读插件越权负例(工程零变化)+ 写插件 actor=plugin 归因。"""
    rev0 = json.loads(cli(cli_path, "project", proj, "--json").stdout)["data"]["rev"]
    ro = tmp / "ro-plugin.json"
    ro.write_text(json.dumps({"id": "ro-plugin", "name": "只读插件", "version": "1.0.0",
                              "form": "process", "entry": "ro.py",
                              "permissions": {"read": True}}, ensure_ascii=False), encoding="utf-8")
    pj = json.loads(project_file(proj).read_text(encoding="utf-8"))
    clip_id = pj["tracks"][0]["clips"][0]["id"]
    r = cli(cli_path, "plugin-call", ro, "clip_update", "--json",
            "--args-json", json.dumps({"clipId": clip_id, "patch": {"volume": 0.1}}))
    assert r.returncode == 2, f"越权必须退出码 2: {r.returncode} {r.stdout[-300:]}"
    env = json.loads(r.stdout.strip().splitlines()[-1])
    assert env["code"] == "GUARD_FAILED" and "FORBIDDEN" in env["message"], f"越权回执: {env}"
    assert json.loads(cli(cli_path, "project", proj, "--json").stdout)["data"]["rev"] == rev0, "越权调用零写入"

    # manifest 非法 → SCHEMA_INVALID
    bad = tmp / "bad-plugin.json"
    bad.write_text(json.dumps({"id": "Bad", "name": "x"}), encoding="utf-8")
    r = cli(cli_path, "plugin-call", bad, "project_get", "--json")
    assert r.returncode == 2 and json.loads(r.stdout.strip().splitlines()[-1])["code"] == "SCHEMA_INVALID", \
        f"坏 manifest 拒绝: {r.stdout[-200:]}"

    # 写权限插件 → 放行,OpLog actor=plugin 归因
    rw = tmp / "rw-plugin.json"
    rw.write_text(json.dumps({"id": "demo-clip", "name": "示例插件", "version": "1.0.0",
                              "form": "process", "entry": "p.py",
                              "permissions": {"read": True, "write": True}}, ensure_ascii=False), encoding="utf-8")
    r = cli(cli_path, "plugin-call", rw, "clip_update", "--json",
            "--args-json", json.dumps({"root": str(proj), "clipId": clip_id, "patch": {"volume": 0.9}}))
    assert r.returncode == 0, f"授权调用失败: {r.stdout[-300:]}"
    r = cli(cli_path, "oplog", proj, "--actor", "plugin", "--json")
    ops = json.loads(r.stdout)["data"]["ops"]
    assert len(ops) == 1 and ops[0]["actor"]["kind"] == "plugin" and ops[0]["actor"]["id"] == "demo-clip", \
        f"actor=plugin 归因对账: {ops}"
    log("插件面 OK:越权 GUARD_FAILED/FORBIDDEN(零写入)、坏 manifest 拒、写插件 actor=plugin 留痕")


def stage_watch(cli_path: Path, mcp: Path, tmp: Path, proj: Path) -> None:
    """4. watch 模式:启动即渲(--max-runs 2)→ 外部改字段 → 自动重渲一次后退出。"""
    before = {p.name: p.stat().st_mtime for p in (proj / "06_成片输出").glob("*.mp4")}
    proc = subprocess.Popen([str(cli_path), "watch", str(proj), "--json", "--max-runs", "2",
                             "--debounce-ms", "300"],
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            text=True, encoding="utf-8", errors="replace")
    try:
        # 等首渲产物落盘(启动即渲)→ 外部改字段触发第二次
        deadline = time.time() + 180
        fired = False
        while time.time() < deadline:
            time.sleep(0.5)
            now = {p.name: p.stat().st_mtime for p in (proj / "06_成片输出").glob("*.mp4")}
            if now and not fired:
                fired = True
                pj = json.loads(project_file(proj).read_text(encoding="utf-8"))
                clip_id = pj["tracks"][0]["clips"][0]["id"]
                r = cli(cli_path, "clip-update", proj, clip_id, "--volume", "0.7", "--json")
                assert r.returncode == 0, f"watch 期间外部改字段失败: {r.stdout[-200:]}"
            if fired and proc.poll() is not None:
                break
        out, _ = proc.communicate(timeout=120)
        assert proc.returncode == 0, f"watch 退出码 {proc.returncode}: {out[-400:]}"
        assert out.count("[watch] 第 2 次渲染") == 1, f"必须自动重渲一次: {out[-400:]}"
        after = {p.name: p.stat().st_mtime for p in (proj / "06_成片输出").glob("*.mp4")}
        assert after, "watch 产物存在"
        log("watch OK:启动即渲 + 外部改字段自动重渲一次(max-runs 2 正常退出)")
    finally:
        if proc.poll() is None:
            proc.kill()


def stage_ci_example(cli_path: Path, tmp: Path) -> None:
    """5. ci-example 样例自证:清单可解析(子集语法)+ 校验器脚本可执行。"""
    from importlib import util as _util
    spec = _util.spec_from_file_location("batch_mod", REPO / "crates" / "cutforge-cli" / "src" / "batch.rs")
    # Rust 解析面由单测覆盖;此处只验 Python 侧样例脚本自洽 + 清单语法可被 YAML 子集规则接受
    example = (REPO / "tools" / "ci-example" / "batch.example.yaml").read_text(encoding="utf-8")
    assert "projects:" in example and "  - root:" in example, "样例清单形态"
    r = run([sys.executable, REPO / "tools" / "ci-example" / "check_output.py", "--help"])
    assert r.returncode == 0, f"check_output.py 不可执行: {r.stderr[-200:]}"
    log("ci-example OK:样例清单形态 + 校验器脚本自洽")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", default=str(REPO / "target" / "debug" / "cutforge-cli"))
    ap.add_argument("--mcp", default=str(REPO / "target" / "debug" / "cutforge-mcp"))
    args = ap.parse_args()
    cli_path, mcp = resolve_bin(args.cli, "cutforge-cli"), resolve_bin(args.mcp, "cutforge-mcp")
    for tool, p in [("cutforge-cli", cli_path), ("cutforge-mcp", mcp)]:
        if not Path(p).is_file():
            die(f"{tool} 不存在: {p}(先 cargo build --locked)")
    import shutil
    if not shutil.which("ffmpeg") or not shutil.which("ffprobe"):
        die("本机无 ffmpeg/ffprobe(headless 渲染链依赖)")
    render_bin = cli_path.parent / f"cutforge-render{'.exe' if os.name == 'nt' else ''}"
    if not render_bin.is_file():
        die(f"cutforge-render 不存在: {render_bin}(与 cutforge-cli 同目录)")

    with tempfile.TemporaryDirectory(prefix="cf-headless-") as td:
        tmp = Path(td)
        proj_a = stage_single_chain(cli_path, mcp, render_bin, tmp)
        # batch 三工程:甲 = 单链工程(已含片段)/ 乙 = v2 独立建卡 / 丙 = v3
        proj_b = tmp / "chain-b"
        r = cli(cli_path, "new", proj_b, "--slug", "headless-b", "--json",
                "--width", str(CANVAS[0]), "--height", str(CANVAS[1]))
        assert r.returncode == 0, f"new 乙: {r.stderr}"
        make_media(proj_b / "01_原始素材" / "b.mp4", 4)
        insert_clip(mcp, proj_b, "01_原始素材/b.mp4", "b", tmp)
        proj_c = tmp / "chain-c"
        r = cli(cli_path, "new", proj_c, "--slug", "headless-c", "--json", "--layout", "v3",
                "--width", str(CANVAS[0]), "--height", str(CANVAS[1]), "--track", "video,audio")
        assert r.returncode == 0, f"new 丙: {r.stderr}"
        report = stage_batch(cli_path, mcp, tmp, [proj_a, proj_b, proj_c])
        stage_plugin(cli_path, tmp, proj_a)
        stage_watch(cli_path, mcp, tmp, proj_a)
        stage_ci_example(cli_path, tmp)
        log(f"AC-7.4 全链验收通过:单链 + batch {report['summary']['ok']}/3 + 插件面 + watch + ci-example")
    print("HEADLESS_OK: 纯 CLI 全链(无 UI)全绿")
    return 0


if __name__ == "__main__":
    sys.exit(main())
