#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""T1.1/AC-1.2 门禁:MCP 工具黄金响应库(golden 响应对拍;册一建 41,册二 A2 增
render_frame 后 42,册四 A4 增六个时间线编辑工具后 48,册四 A4-BE3b 增文本/字幕/媒体
八工具后 56,册五 A5 增专业编辑/互操作七工具后 68,册六 A6 增布局迁移/工程库/崩溃恢复四工具后 72,数量口径以 schemas/mcp-tools.json 为准)。

    python tools/bench/tool_parity.py                  # 对比模式:重跑采集,与 golden 逐字段对拍
    python tools/bench/tool_parity.py --update-golden  # 采集模式:重建 tools/bench/golden/*.json
    python tools/bench/tool_parity.py --json           # 结果协议 JSON(供 gate.py 消费)

机制:
  1. 启动 serve(与 tools/e2e_*.py 同一启动/鉴权/端口模式,Bearer token + /rpc);
  2. 构造固定夹具工程(project_new + 固定序列的 clip/track/notes/cutlist 操作,
     21 个写工具全部有真实调用);编排类工具的 CutFlow 依赖用**桩脚本**替代
     (CUTFLOW_REPO 指向临时桩目录)——本对拍锁定的是 cutforge-mcp 的派发/编排行为,
     不是 CutFlow 脚本内容,桩化后跨机/CI 确定性成立;
  3. 逐工具发 /rpc,归一化响应后与黄金快照对比。

归一化(剥离易变字段,采集与对比同规则):
  - 分隔符统一(第一步):字符串里所有反斜杠(含 JSON-in-JSON 的 \\ 转义连写)
    坍缩为单正斜杠;能整体 json.loads 的"JSON-in-JSON"行(渲染进度行、编排 stdout)
    先解析、递归归一字段值、再 sort_keys 重序列化 —— Windows 录制与 ubuntu 实测自此同形;
  - rev / rev-N 串 → <REV>;ts/createdAt 等 RFC3339 → <TS>;runId → <RUN_ID>;
  - 临时目录 / 仓库根 / token → <TMP> / <REPO> / <TOKEN>;残余绝对路径 → <ABS>;
  - render_frame 的帧缓存路径/键(A2):工作区指纹入键,run 间必变 →
    `render-cache/frame/<16hex>.<ext>` 文件名整体 → frame/<FRAME><ext>,
    恰为 16 位十六进制的 "key" 字段值 → <FRAME_KEY>(缓存键正确性由 Rust 单测锁定,
    对拍只锁协议形状);
  - 媒体探测值(ffprobe 口径):durationMs 就近取整到 100ms、bytes → <BYTES>
    (这两者反映外部工具链产物,不是 cutforge-mcp 行为);
  - audio_beats 启发式面(onset-energy;CI 实证跨解码器漂移):confidence 量化到
    0.2 步进档(0.42/0.33 同归 0.4)、onsets 时间戳 100ms 网格化 + 去重 —— 启发式
    检测值对 ffmpeg 解码舍入敏感(Windows/ubuntu 样本微差让个别 onset 跨过自适应
    阈值),与 media_probe 的 durationMs 100ms 量化同策。
  - 册五 A5 启发式面(scene_detect 帧差分 / multicam_sync 包络互相关;同 audio_beats
    "检测值反映外部工具链产物"口径):cuts[].tMs 500ms 网格化(吸收 ±1 抽帧的
    剪切点漂移)、cuts[].confidence 走 0.2 档量化、frames/cutCount 走计数容差
    (≤1,WARN)、durationsMs 逐项 100ms 量化(AAC 解码样本数跨 build 微差);
    multicam_sync 的 offsetMs 用**同源字节复制**夹具(take2 = take1 复制)锁死
    确定性零偏移,不进容差。

对比语义(**加法容忍**,适配后续有计划的加法演进):
  - golden 有的键:实际必须有且值相等,否则 DRIFT(阻断);
  - 实际多出的键:仅警告(WARN),不失败;
  - 数组:长度与逐元素(按下标)严格一致;
  - 唯一宽口径(仅启发式检测面:audio_beats 的 onsets/onsetCount、scene_detect 的
    cuts/cutCount/frames):网格化后计数差 ≤2(a5 cuts/frames ≤1)且首个网格值相等
    网格值相等、onsetCount 计数差 ≤2 → WARN 不 DRIFT;超差仍 DRIFT
    (不弱化其他工具的严格度)。

退出码:0 = 68/68 PASS;2 = 有 DRIFT/FAIL;3 = 环境缺失(ffmpeg)。
依赖:Python 标准库 + 已构建的 cutforge-mcp(+ 同目录 cutforge-render)+ ffmpeg。
"""
from __future__ import annotations

import argparse
import json
import math
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
GOLDEN_DIR = Path(__file__).resolve().parent / "golden"
TOKEN = "parity-golden-token"
TIMEOUT_RPC = 30

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

# ---------------- 结果协议 ----------------


class Report:
    """逐工具 PASS/WARN/DRIFT/FAIL 清单 + 汇总;--json 输出结果协议 envelope。"""

    def __init__(self) -> None:
        self.rows: list[dict] = []
        self.missing: list[str] = []  # 序列中注册但调用阶段异常的工具

    def add(self, tool: str, status: str, detail: str, extras: list[str] | None = None) -> None:
        self.rows.append({"tool": tool, "status": status, "detail": detail,
                          "extras": extras or []})

    def ok(self) -> bool:
        return not self.missing and all(r["status"] in ("PASS", "WARN") for r in self.rows)

    def counts(self) -> dict:
        c = {"pass": 0, "warn": 0, "drift": 0, "fail": 0}
        for r in self.rows:
            c[{"PASS": "pass", "WARN": "warn", "DRIFT": "drift", "FAIL": "fail"}[r["status"]]] += 1
        return c

    def summary_line(self, mode: str, n_tools: int) -> str:
        c = self.counts()
        msg = (f"tool_parity {mode}完成(覆盖 {n_tools} 工具): "
               f"{c['pass']} PASS / {c['warn']} WARN / {c['drift']} DRIFT / {c['fail']} FAIL")
        bad = [r["tool"] for r in self.rows if r["status"] in ("DRIFT", "FAIL")]
        if bad:
            msg += f";漂移工具: {bad}"
        if self.missing:
            msg += f";调用异常: {self.missing}"
        if not bad and not self.missing:
            msg += f";{n_tools} 工具黄金对拍零漂移"
        return msg

    def envelope(self, mode: str, n_tools: int) -> dict:
        return {"ok": self.ok(), "code": "OK" if self.ok() else "GATE_FAILED",
                "message": self.summary_line(mode, n_tools),
                "data": {"tools": self.rows, "missing": self.missing, "counts": self.counts()}}


# ---------------- RPC 客户端(与 tools/e2e_edit_ops.py 同模式) ----------------


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def rpc(port: int, token: str, name: str, args: dict) -> dict:
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": name, "arguments": args}}).encode()
    req = urllib.request.Request(f"http://127.0.0.1:{port}/rpc", data=body,
                                 headers={"Content-Type": "application/json",
                                          "Authorization": f"Bearer {token}"})
    for attempt in range(4):
        try:
            out = json.loads(urllib.request.urlopen(req, timeout=TIMEOUT_RPC).read())
            return json.loads(out["result"]["content"][0]["text"])
        except ConnectionResetError:
            if attempt == 3:
                raise
            time.sleep(0.2 * (attempt + 1))


# ---------------- 归一化 ----------------

TS_KEYS = {"ts", "createdAt", "resolvedAt", "rejectedAt", "generatedAt",
           "modifiedAt", "updatedAt", "startedAt", "finishedAt", "savedAt"}
NUMERIC_TS_KEYS = {"modifiedAtMs"}  # 册六 A6:工程库卡片 mtime(epoch ms;run 间必变)
ELAPSED_KEYS = {"elapsedMs", "elapsedSec", "tookMs", "wallMs", "elapsedUs"}
RE_ISO_TS = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?")
RE_REV = re.compile(r"(?<![A-Za-z0-9-])rev-\d+(?![A-Za-z0-9-])")
RE_WIN_PATH = re.compile(r"(?i)\b[a-z]:[\\/](?:[^\s\"'\\/:*?<>|]+[\\/])*[^\s\"'<>|]*")
RE_POSIX_PATH = re.compile(r"(?<![\w\">])/(?:tmp|home|Users|usr|var)/[^\s\"']*" )
RE_BS_RUN = re.compile(r"\\+")  # 反斜杠串(含 JSON-in-JSON 里 \\ 转义出的连写形态)
# A2 render_frame:帧缓存文件名内嵌工作区指纹键(run 间必变)→ 占位;扩展名保留
RE_FRAME_FILE = re.compile(r"render-cache/frame/[0-9a-f]{16}(\.png|\.jpe?g)")
RE_FRAME_KEY = re.compile(r"[0-9a-f]{16}")
# A4-BE3b:媒体派生物缓存文件名内嵌内容键(mtime 入键,run 间必变)→ 占位
RE_MEDIA_CACHE_FILE = re.compile(
    r"\.cutforge/((?:peaks-cache|thumb-cache|proxy|scope-cache)/)[0-9a-f]{16}\.(json|png|mp4)")
# 册六 A6:工程库回收站条目名内嵌时间戳(delete 落点 .trash/<ts>-<名>,run 间必变)→ 占位
RE_TRASH_ENTRY = re.compile(r"\.trash/[0-9]+-")
# 册五 A5:响度测量键(audio_loudness;probe_mode 下 0.5LU 量化)+ 硬件探测键
LUFS_KEYS = {"inputI", "inputTp", "inputLra", "inputThresh", "target", "deviation"}
HW_KEYS = {"nvenc", "qsv", "amf"}


def _fmt_half(f: float) -> str:
    """0.5LU 量化 + 去尾零(与 Rust 侧 fmt_num 同风格;跨机 golden 稳定)。"""
    q = round(f * 2) / 2
    t = f"{q:.6f}".rstrip("0").rstrip(".")
    return t if t not in ("", "-0") else "0"


# 册五:audio_beats(onset-energy 启发式)跨平台容差口径 —— CI(ubuntu)实证:Windows
# 录制的 golden 与实测可差 1 个 onset、confidence 相差 0.1 级(ffmpeg 解码样本微差让
# 个别 onset 跨过自适应阈值)。启发式检测值反映解码产物而非 cutforge-mcp 行为,与
# media_probe 的 durationMs 100ms 量化同策,在归一化/对比两层吸收:
ONSET_GRID_MS = 100  # onsets 时间戳 100ms 网格(与 durationMs 量化同格)
ONSET_COUNT_TOL = 2  # onset 计数容忍差(网格化后 golden/actual 允许 ±2)
CONF_BIN = 0.2       # confidence 量化档(见 _conf_bin:0.1 档无法并档,取 0.2 步进)

# 册五 A5:scene_detect(帧差分启发式)跨平台容差口径 —— 抽帧输出帧数跨 ffmpeg
# build 可差 ±1,剪切点时间戳随之漂移一个采样帧(5fps 下 200ms)。与 audio_beats
# 同策:网格化(500ms,吸收 ±1 采样帧)+ 计数容差。
CUT_GRID_MS = 500    # cuts[].tMs 网格(粗于采样帧 200ms,吸收 ±1 帧漂移)
CUT_COUNT_TOL = 1    # cuts/cutCount/frames 计数容忍差(±1 抽帧)


def _grid(values: list, grid_ms: int) -> list[int] | None:
    """数值时间戳 → 网格就近取整 + 保序去重(采集与对比同规则)。

    非数值列表返回 None,交回严格对比(能力面若变更形态不静默放宽)。
    """
    out: list[int] = []
    for v in values:
        if isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v):
            return None
        g = int(round(v / grid_ms) * grid_ms)
        if not out or out[-1] != g:
            out.append(g)
    return out


def _onset_grid(values: list) -> list[int] | None:
    """onsets 时间戳 → 100ms 网格就近取整 + 保序去重(采集与对比同规则)。

    非数值列表返回 None,交回严格对比(onset 能力面若变更形态不静默放宽)。
    """
    return _grid(values, ONSET_GRID_MS)


def _conf_bin(f: float) -> float:
    """confidence 档位量化:floor(x/0.2 + 0.5) * 0.2,使 CI 实证的 0.42/0.33 同归 0.4。

    注:若按 0.1 档最近舍入,0.42→0.4、0.33→0.3 仍分两档,吸收不了该漂移;
    0.2 步进档才满足"0.42/0.33 同归 0.4"的并档口径。IEEE 双精度下两端同式同结果。
    """
    return math.floor(f / CONF_BIN + 0.5) * CONF_BIN


def _fwd(p: str) -> str:
    """与归一化同规则的路径展平:反斜杠串(含 canonicalize 的 \\\\?\\ 前缀)归一为单正斜杠。"""
    return RE_BS_RUN.sub("/", p)


class Normalizer:
    """采集与对比共用同一规则;tmp/repo/token 上下文在构造时注入。

    跨平台口径(golden 在 Windows 录制,parity 也在 ubuntu CI 跑,两者必须同形):
      1. 分隔符统一先于一切占位替换 —— 字符串里所有反斜杠(含 JSON 转义出的
         \\\\ 连写形态)一律坍缩为单正斜杠,Windows 路径与 POSIX 路径自此同形;
      2. "JSON-in-JSON"字符串(渲染进度行、编排 stdout 行)先按原文 json.loads、
         递归归一化字段值、再 sort_keys 重序列化 —— 转义形态交给解析器还原,
         避免字符串里的 \\\\ 形态躲过 tmp 占位替换。
    """

    def __init__(self, tmp: Path, token: str) -> None:
        subs: list[tuple[str, str]] = []
        # 占位替换发生在分隔符统一之后,基准路径只需正斜杠形态;
        # canonicalize 的 \\\\?\\ 前缀按同规则展平成 /?/ 一并收录
        for base in {_fwd(str(tmp)), _fwd("\\\\?\\" + str(tmp))}:
            if base:
                subs.append((base, "<TMP>"))
        subs.append((_fwd(str(REPO)), "<REPO>"))
        subs.append((token, "<TOKEN>"))
        # 长路径优先替换,避免前缀互相吞
        self.subs = sorted(subs, key=lambda kv: -len(kv[0]))

    def _s(self, s: str, probe_mode: bool = False) -> str:
        # 0b) JSON-Lines 串(render 工具的 stdout 字段:多行进度事件拼接):
        #     逐行尽力解析——JSON 对象行递归归一(elapsedMs 等),非 JSON 行
        #     (RENDER_OK 等)走标量归一路径,按行重拼接
        nl = chr(10)
        if nl in s:
            lines = s.split(nl)
            out_lines = []
            hit = 0
            for l in lines:
                obj = None
                if l.strip():
                    try:
                        cand = json.loads(l)
                        if isinstance(cand, dict):
                            obj = cand
                    except ValueError:
                        obj = None
                if obj is not None:
                    out_lines.append(json.dumps(self(obj, probe_mode), ensure_ascii=False, sort_keys=True))
                    hit += 1
                else:
                    out_lines.append(self(l, probe_mode))
            if hit:
                return nl.join(out_lines)
        if s.lstrip()[:1] in ("{", "["):
            try:
                obj = json.loads(s)
            except ValueError:
                obj = None
            if isinstance(obj, (dict, list)):
                return json.dumps(self(obj, probe_mode), ensure_ascii=False, sort_keys=True)
        # 1) 分隔符统一:反斜杠串坍缩为单正斜杠(先于占位替换,两边自此同形)
        s = RE_BS_RUN.sub("/", s)
        # 2) 占位符替换 + 残余绝对路径/时间戳/修订号
        for a, b in self.subs:
            if a in s:
                s = s.replace(a, b)
        s = RE_ISO_TS.sub("<TS>", s)
        s = RE_REV.sub("<REV>", s)
        s = RE_WIN_PATH.sub("<ABS>", s)
        s = RE_POSIX_PATH.sub("<ABS>", s)
        # 3) render_frame 帧文件名占位(内嵌工作区指纹键,run 间必变;扩展名保留)
        s = RE_FRAME_FILE.sub(lambda m: f"render-cache/frame/<FRAME>{m.group(1)}", s)
        # 4) 媒体派生物缓存文件名占位(peaks/thumb/proxy 内容键含 mtime,run 间必变;目录与扩展名保留)
        s = RE_MEDIA_CACHE_FILE.sub(lambda m: f".cutforge/{m.group(1)}<MEDIA_CACHE>.{m.group(2)}", s)
        # 5) 回收站条目时间戳占位(册六 library delete 落点,run 间必变;目录结构保留)
        s = RE_TRASH_ENTRY.sub(".trash/<TRASH>-", s)
        return s

    def __call__(self, v, probe_mode: bool = False):
        if isinstance(v, dict):
            out = {}
            for k, val in v.items():
                if k == "rev":
                    out[k] = "<REV>"
                elif k == "runId":
                    out[k] = "<RUN_ID>"
                elif k in TS_KEYS:
                    out[k] = "<TS>"
                elif k in NUMERIC_TS_KEYS:
                    out[k] = "<TS>"  # 数值时间戳(工程库卡片 mtime)同占位口径
                elif k in ELAPSED_KEYS:
                    out[k] = "<ELAPSED>"
                elif k == "key" and isinstance(val, str) and RE_FRAME_KEY.fullmatch(val):
                    out[k] = "<FRAME_KEY>"  # render_frame 帧缓存键(工作区指纹入键,run 间必变)
                elif k == "bytes" and probe_mode:
                    out[k] = "<BYTES>"
                elif k == "durationMs" and probe_mode and isinstance(val, (int, float)):
                    out[k] = int(round(val / 100.0) * 100)  # ffprobe 口径:就近 100ms
                elif k == "confidence" and probe_mode and isinstance(val, (int, float)) \
                        and not isinstance(val, bool) and math.isfinite(val):
                    # audio_beats 启发式置信度跨解码器漂移(0.42 vs 0.33)→ 档位量化
                    out[k] = _conf_bin(val)
                elif k == "onsets" and probe_mode and isinstance(val, list):
                    # audio_beats onset 时间戳:100ms 网格化 + 去重(跨解码器 ±1 onset
                    # 漂移的吸收口径;仅 audio_beats 有此键,probe_mode 限定不外溢)
                    gridded = _onset_grid(val)
                    out[k] = gridded if gridded is not None else val
                elif k == "cuts" and probe_mode and isinstance(val, list):
                    # scene_detect 剪切点:[{tMs,confidence}] → tMs 500ms 网格化
                    # (吸收 ±1 抽帧漂移);confidence 走下方统一档位量化
                    if all(isinstance(c, dict) and "tMs" in c for c in val):
                        out[k] = [dict(c, tMs=int(round(c["tMs"] / CUT_GRID_MS) * CUT_GRID_MS))
                                  for c in val]
                    else:
                        out[k] = self(val, probe_mode)
                elif k == "durationsMs" and probe_mode and isinstance(val, list):
                    # multicam_sync 各角度 PCM 时长(ms):AAC 解码样本数跨 build 微差
                    # → 逐项 100ms 量化(与 durationMs 同格)
                    if all(isinstance(x, (int, float)) and not isinstance(x, bool) for x in val):
                        out[k] = [int(round(x / 100.0) * 100) for x in val]
                    else:
                        out[k] = self(val, probe_mode)
                elif k in LUFS_KEYS and probe_mode and isinstance(val, str):
                    # 响度测量值(跨 ffmpeg build 有 0.x LU 漂移)→ 0.5LU 量化
                    try:
                        f = float(val)
                        out[k] = _fmt_half(f) if math.isfinite(f) else val
                    except ValueError:
                        out[k] = val  # "-inf" 等原样
                elif k in HW_KEYS and probe_mode and isinstance(val, dict):
                    out[k] = "<HW_PROBE>"  # 硬件在位/可用随机器与驱动变化 → 占位
                else:
                    out[k] = self(val, probe_mode)
            return out
        if isinstance(v, list):
            return [self(x, probe_mode) for x in v]
        if isinstance(v, str):
            return self._s(v, probe_mode)
        return v


# ---------------- 加法容忍对比 ----------------


def _tolerant_pair(k: str, g, a, path: str, extras: list[str]) -> bool:
    """audio_beats 启发式面的唯一宽口径(严格对比前尝试;返回 True = 已按容差记 WARN)。

    CI 实证:启发式 onset 面对解码舍入敏感,Windows/ubuntu 可差 1 个 onset、
    confidence ±0.1 级 —— confidence/onsets 形态已在归一化层量化/网格化,这里对
    "计数"放宽:onsets 网格化后计数差 ≤ ONSET_COUNT_TOL 且 onsetCount 计数差
    ≤ ONSET_COUNT_TOL → WARN(不要求首个 onset 相等:ubuntu 实证缺失的可能是
    第一个 onset,首元素相等前置会把容差整条落回严格对比);超差返回 False
    落回严格对比报 DRIFT(其他工具不受影响)。
    """
    if k == "onsets" and isinstance(g, list) and isinstance(a, list):
        gg, aa = _onset_grid(g), _onset_grid(a)
        if gg is None or aa is None:
            return False
        d = abs(len(gg) - len(aa))
        if d <= ONSET_COUNT_TOL:
            extras.append(f"{path}: 启发式容差 golden={len(gg)} actual={len(aa)} 个 onset"
                          f"(计数差 {d} ≤ {ONSET_COUNT_TOL})")
            return True
        return False  # 超差 → 严格对比,按数组长度/逐元素报 DRIFT
    if k == "onsetCount" and isinstance(g, (int, float)) and isinstance(a, (int, float)) \
            and not isinstance(g, bool) and not isinstance(a, bool):
        if abs(g - a) <= ONSET_COUNT_TOL:
            extras.append(f"{path}: 启发式容差计数差 golden={g} actual={a}"
                          f"(≤ {ONSET_COUNT_TOL})")
            return True
    # scene_detect 帧差分启发式面(册五 A5;同 audio_beats 口径):cuts 数组在归一化层
    # 已 500ms 网格化,这里对计数(±1 抽帧)与首个剪切点网格值放宽 → WARN;超差落回
    # 严格对比报 DRIFT。
    if k == "cuts" and isinstance(g, list) and isinstance(a, list):
        gg = [int(round(c.get("tMs", 0) / CUT_GRID_MS) * CUT_GRID_MS) for c in g
              if isinstance(c, dict)] if g else []
        aa = [int(round(c.get("tMs", 0) / CUT_GRID_MS) * CUT_GRID_MS) for c in a
              if isinstance(c, dict)] if a else []
        d = abs(len(gg) - len(aa))
        if g != a and len(g) == len(gg) and len(a) == len(aa) and d <= CUT_COUNT_TOL and gg[:1] == aa[:1]:
            extras.append(f"{path}: 启发式容差 golden={len(gg)} actual={len(aa)} 个剪切点"
                          f"(计数差 {d} ≤ {CUT_COUNT_TOL} 且首个剪切点网格值相等)")
            return True
        return False  # 超差/形态不符 → 严格对比,按数组长度/逐元素报 DRIFT
    if k in ("cutCount", "frames") and isinstance(g, (int, float)) and isinstance(a, (int, float)) \
            and not isinstance(g, bool) and not isinstance(a, bool):
        if g != a and abs(g - a) <= CUT_COUNT_TOL:
            extras.append(f"{path}: 启发式容差计数差 golden={g} actual={a}"
                          f"(≤ {CUT_COUNT_TOL};抽帧输出帧数跨 build ±1)")
            return True
    return False  # 超差/不适用 → 严格对比


def compare(golden, actual, path: str, diffs: list[str], extras: list[str]) -> None:
    """golden 有的键必须存在且相等;实际多出的键只记警告。数组按下标全等。"""
    if isinstance(golden, dict):
        if not isinstance(actual, dict):
            diffs.append(f"{path}: 类型不一致 golden=dict actual={type(actual).__name__}")
            return
        for k, gv in golden.items():
            if k not in actual:
                diffs.append(f"{path}.{k}: 键缺失(实际响应没有 golden 的键)")
            elif _tolerant_pair(k, gv, actual[k], f"{path}.{k}", extras):
                pass  # audio_beats 启发式容差命中,已记 WARN
            else:
                compare(gv, actual[k], f"{path}.{k}", diffs, extras)
        for k in actual:
            if k not in golden:
                extras.append(f"{path}.{k}: 加法键(容忍,仅警告)")
        return
    if isinstance(golden, list):
        if not isinstance(actual, list):
            diffs.append(f"{path}: 类型不一致 golden=list actual={type(actual).__name__}")
            return
        if len(golden) != len(actual):
            diffs.append(f"{path}: 数组长度 golden={len(golden)} actual={len(actual)}")
            return
        for i, (g, a) in enumerate(zip(golden, actual)):
            compare(g, a, f"{path}[{i}]", diffs, extras)
        return
    if golden != actual:
        diffs.append(f"{path}: golden={golden!r} actual={actual!r}")


# ---------------- CutFlow 桩(编排类工具的确定性替身) ----------------

STUB_COMMON = '''#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""tool_parity stub: echo argv to prove orchestrate passthrough."""
import json, sys
print(json.dumps({"argv": sys.argv[1:], "stub": True}, sort_keys=True))
'''

# rs_run.py --status 要返回 data.stages(stage_status 的 rs_run 同口径解析路径)
STUB_RS_RUN = '''#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""tool_parity stub: --status returns fixed stage map; else echo argv."""
import json, sys
if "--status" in sys.argv[1:]:
    print(json.dumps({"data": {"stages": {
        "S0": "missing", "S2": "missing", "S3": "missing", "S5": "missing",
        "S6": "missing", "S8": "missing", "S10": "missing"}}, "ok": True,
        "stub": True}, sort_keys=True))
else:
    print(json.dumps({"argv": sys.argv[1:], "stub": True}, sort_keys=True))
'''

# rs_verify.py 是 --json 白名单脚本:stdout 会被 orchestrate 整体 parse 成响应 envelope
STUB_RS_VERIFY = '''#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""tool_parity stub: rs_verify --json whitelist path (stdout IS the envelope)."""
import json, sys
print(json.dumps({"code": "VERIFY_STATUS", "data": {"argv": sys.argv[1:],
    "verdict": "pass"}, "ok": True, "stub": True}, sort_keys=True))
'''


def write_cutflow_stub(tmp: Path, main_ws: Path) -> Path:
    """建桩 CutFlow 仓库:skills/cutflow/scripts/ 五脚本 + 工程内 rebuild.py。

    serve 以 CUTFLOW_REPO=桩目录 启动;orchestrate 的脚本定位、py 启动器、
    scriptArgs 序列化、--json 白名单、stdout 透传全部走真实代码路径。
    """
    scripts = tmp / "cutflow-stub" / "skills" / "cutflow" / "scripts"
    scripts.mkdir(parents=True)
    (scripts / "rs_run.py").write_text(STUB_RS_RUN, encoding="utf-8")
    (scripts / "rs_verify.py").write_text(STUB_RS_VERIFY, encoding="utf-8")
    for name in ("rs_sync.py", "rs_render.py", "rs_jy_draft.py"):
        (scripts / name).write_text(STUB_COMMON, encoding="utf-8")
    # stage_rebuild 的脚本解析顺序:CutFlow scripts/ 无 rebuild.py → 回退工程目录内
    (main_ws / "rebuild.py").write_text(STUB_COMMON, encoding="utf-8")
    return tmp / "cutflow-stub"


# ---------------- 夹具与调用序列 ----------------


def find_bin(arg: str | None) -> Path:
    """与 e2e 脚本同一二进制定位顺序(--bin → debug.exe → debug → release.exe → release)。"""
    if arg:
        p = Path(arg)
        if p.is_file():
            return p
    for cand in (REPO / "target" / "debug" / "cutforge-mcp.exe",
                 REPO / "target" / "debug" / "cutforge-mcp",
                 REPO / "target" / "release" / "cutforge-mcp.exe",
                 REPO / "target" / "release" / "cutforge-mcp"):
        if cand.is_file():
            return cand
    raise ParityError("FAIL: 先 cargo build -p cutforge-mcp", 2)


class ParityError(Exception):
    """带期望退出码的失败(2=结果不对,3=环境缺失)。"""

    def __init__(self, msg: str, code: int) -> None:
        super().__init__(msg)
        self.code = code


def make_media(ws: Path) -> None:
    """现场生成确定性媒体夹具(testsrc2 画面 + 正弦;同 e2e 口径;
    durationMs/bytes 的跨 ffmpeg 差异由归一化吸收)。"""
    src_dir = ws / "01_原始素材"
    src_dir.mkdir(parents=True, exist_ok=True)
    jobs = [
        (["-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30",
          "-f", "lavfi", "-i", "sine=frequency=440:duration=10",
          "-t", "10", "-pix_fmt", "yuv420p",
          "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest"],
         src_dir / "take1.mp4"),
        (["-f", "lavfi", "-i", "sine=frequency=220:duration=5",
          "-c:a", "libmp3lame", "-b:a", "64k"],
         src_dir / "bgm.mp3"),
        (["-f", "lavfi", "-i", "sine=frequency=880:duration=1",
          "-c:a", "libmp3lame", "-b:a", "64k"],
         src_dir / "sfx.mp3"),
        # 册五 T5.2:平色帧(scope_data 三类数据跨机确定性;4x3x0xNN 域无渐变)
        (["-f", "lavfi", "-i", "color=c=0x4080C0:size=64x48",
          "-frames:v", "1"],
         src_dir / "frame.png"),
        # 册五 T5.4:硬切夹具(2s 红 + 2s 蓝 CFR 拼接;scene_detect 帧差分在 2s
        # 边界必然触发,红/蓝灰度差 ≈47 级远超自适应阈值)。带**静音轨**(anullsrc):
        # 混音图按片段数引 [N:a],纯视频源会让 mix pass A 报 Stream specifier ':a'
        # 失效(既有限制:plan 以 clip 为粒度建音频事件,无音轨探测,登记遗留)——
        # 夹具侧带静音轨绕开,不给对拍引入与工具行为无关的渲染失败。
        (["-f", "lavfi", "-i", "color=c=red:size=320x180:rate=30:duration=2",
          "-f", "lavfi", "-i", "color=c=blue:size=320x180:rate=30:duration=2",
          "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo",
          "-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0[v];[2:a]atrim=0:4[a]",
          "-map", "[v]", "-map", "[a]", "-pix_fmt", "yuv420p",
          "-c:v", "libx264", "-preset", "veryfast", "-c:a", "aac", "-shortest"],
         src_dir / "hardcut.mp4"),
    ]
    for extra, out in jobs:
        r = subprocess.run(["ffmpeg", "-y", "-loglevel", "error", *extra, str(out)],
                           capture_output=True, text=True)
        if r.returncode != 0 or not out.is_file():
            raise ParityError(f"FAIL: ffmpeg 生成夹具失败({out.name}): {r.stderr[-200:]}", 2)
    # 册五 T5.4:multicam_sync 双素材夹具 —— take2 = take1 字节级复制(同源素材),
    # 互相关零偏移恒成立(offsetMs=0,confidence=1.0),跨 ffmpeg build 零漂移;
    # 检测偏移值的跨平台量化口径不做(检测质量不进 golden,确定性进)。
    shutil.copyfile(src_dir / "take1.mp4", src_dir / "take2.mp4")
    # 册五 T5.2:3D .cube 夹具(4³ 主格式;LUT_3D_SIZE 4 + 64 数据行)
    rows = []
    for b in range(4):
        for g in range(4):
            for r_ in range(4):
                fr, fg, fb = r_ / 3, g / 3, b / 3
                rows.append(f"{min(1, fr * 1.5):.6f} {fg:.6f} {fb:.6f}")
    (src_dir / "test.cube").write_text(
        'TITLE "parity lut"\nLUT_3D_SIZE 4\n' + "\n".join(rows) + "\n", encoding="utf-8")


def write_subtitle_fixture(ws: Path) -> None:
    """SRT 导入夹具(规范形;导入→导出 byte 级对拍的种子)。"""
    srt = "1\n00:00:04,000 --> 00:00:05,000\n导入第一句\n\n2\n00:00:05,000 --> 00:00:06,000\n导入第二句\n\n"
    (ws / "05_时间线工程" / "subs.srt").write_text(srt, encoding="utf-8")


def write_truth_fixtures(ws: Path) -> None:
    """wordline.json / cutlist.json 最小合法夹具(过各自 schema;cutlist 另过 finalize 重算)。"""
    (ws / "05_时间线工程" / "wordline.json").write_text(json.dumps({
        "version": 1, "source": "parity-fixture", "space": "final", "fps": 30,
        "chars": [{"i": 0, "ch": "测", "startMs": 0, "endMs": 400,
                   "srcStartMs": 0, "srcEndMs": 400, "conf": 0.99}],
        "gaps": [], "sentences": [{"id": "s1", "span": [0, 1], "punc": "", "text": "测"}],
        "speakers": [], "srcDurationMs": 10000, "finalDurationMs": 8400,
        "stats": {"charCount": 1, "coverage": 1.0, "confMedian": 0.99},
        "degraded": False, "degradeReasons": [], "charTimingEstimated": False,
    }, ensure_ascii=False, indent=1), encoding="utf-8")
    cut_dir = ws / "04_粗剪决策"
    cut_dir.mkdir(parents=True, exist_ok=True)
    (cut_dir / "cutlist.json").write_text(json.dumps({
        "version": 1, "source": "parity-fixture",
        "detector": {"version": "parity", "params": {}},
        "cuts": [
            {"id": "c001", "inMs": 1000, "outMs": 2000, "reason": "silence",
             "conf": 0.9, "action": "keep", "note": "", "guard": {}, "text": ""},
            {"id": "c002", "inMs": 5000, "outMs": 6500, "reason": "silence",
             "conf": 0.9, "action": "keep", "note": "", "guard": {}, "text": ""},
            {"id": "c003", "inMs": 8000, "outMs": 8500, "reason": "silence",
             "conf": 0.9, "action": "keep", "note": "", "guard": {}, "text": ""},
        ],
        "keep": [[0, 10000]],
        "removedMs": 0, "srcTotalMs": 10000, "script": [],
    }, ensure_ascii=False, indent=1), encoding="utf-8")


def build_sequence() -> list[tuple[str, dict, bool]]:
    """固定调用序列:(工具名, 参数, probe_mode)。全部工具覆盖、全部写工具真实调用
    (册四 A4 起 27 写工具;数量以 schemas/mcp-tools.json 为准)。

    参数里的 {MAIN}/{NEW} 在发 rpc 前替换为真实临时路径;@...@ 为动态接线占位
    (notes_resolve 的 opIds 取自 clip_update(causedBy) 回执;render_progress 的
    runId 取自 render_run 回执)。顺序即状态:写工具按无重叠时间线推进
    (引擎 enforce_no_overlap),undo/redo 紧跟 clip_delete 保证回执有实义。
    probe_mode=True 的响应走媒体探测归一化(durationMs 100ms 量化、bytes 占位)。
    """
    return [
        # -- 阶段 A:免开工作区(空目录新工程 + 静态矩阵) --
        ("project_new", {"root": "{NEW}", "slug": "parity-new", "fps": 30,
                         "canvasW": 1080, "canvasH": 1920, "tracks": ["video", "audio"]}, False),
        ("capability_matrix", {}, False),
        # -- 阶段 B:pristine 查询 --
        ("project_get", {"root": "{MAIN}"}, False),
        ("timeline_get", {"root": "{MAIN}"}, False),
        ("notes_list", {"root": "{MAIN}"}, False),
        ("stage_status", {"root": "{MAIN}"}, False),
        ("oplog_tail", {"root": "{MAIN}", "limit": 5}, False),
        ("conflict_list", {"root": "{MAIN}"}, False),
        ("render_probe", {"root": "{MAIN}"}, True),
        ("media_browse", {"root": "{MAIN}", "dir": ""}, True),
        ("media_probe", {"root": "{MAIN}", "src": "01_原始素材/take1.mp4"}, True),
        ("wordline_get", {"root": "{MAIN}"}, False),
        ("cutlist_get", {"root": "{MAIN}"}, False),
        # -- 阶段 C:写序列(21 写工具全覆盖) --
        ("track_add", {"root": "{MAIN}", "kind": "text", "requestId": "parity-track-1"}, False),
        ("clip_add", {"root": "{MAIN}", "trackId": "V1",
                      "src": "01_原始素材/take1.mp4", "startMs": 0, "durationMs": 4000,
                      "volume": 1.0, "requestId": "parity-add-1"}, False),
        ("clip_add", {"root": "{MAIN}", "trackId": "V1",
                      "src": "01_原始素材/take1.mp4", "startMs": 4000, "durationMs": 6000,
                      "sourceInMs": 1000, "requestId": "parity-add-2"}, False),
        ("clip_split", {"root": "{MAIN}", "clipId": "V1-001", "tMs": 2000}, False),
        ("clip_update", {"root": "{MAIN}", "clipId": "V1-001",
                         "patch": {"volume": 0.9}, "summary": "对拍:微调音量"}, False),
        ("clip_move", {"root": "{MAIN}", "clipId": "V1-003", "startMs": 12000,
                       "requestId": "parity-move-1"}, False),
        ("clip_duplicate", {"root": "{MAIN}", "clipId": "V1-001", "startMs": 20000,
                            "requestId": "parity-dup-1"}, False),
        ("clip_delete", {"root": "{MAIN}", "clipId": "V1-004"}, False),
        ("undo", {"root": "{MAIN}"}, False),
        ("redo", {"root": "{MAIN}"}, False),
        ("transition_set", {"root": "{MAIN}", "clipId": "V1-002", "type": "fade",
                            "durMs": 300, "reason": "topic", "summary": "对拍:转场"}, False),
        ("motion_set", {"root": "{MAIN}", "clipId": "V1-001", "in": "zoomIn",
                        "inMs": 280, "out": "fadeOut", "outMs": 260}, False),
        ("subtitle_set", {"root": "{MAIN}", "clipId": "V1-001", "text": "第一句字幕"}, False),
        ("subtitle_retime", {"root": "{MAIN}", "clipId": "V1-001",
                             "startMs": 0, "durationMs": 1800}, False),
        ("overlay_add", {"root": "{MAIN}", "trackId": "T1",
                         "element": {"src": "01_原始素材/take1.mp4", "atMs": 1000,
                                     "durationMs": 2000,
                                     "overlay": {"x": 84, "y": 240, "w": 400, "h": 200,
                                                 "opacity": 0.5}},
                         "requestId": "parity-overlay-1"}, False),
        ("sfx_add", {"root": "{MAIN}", "tMs": 500, "src": "01_原始素材/sfx.mp3",
                     "volume": 0.8, "requestId": "parity-sfx-1"}, False),
        ("bgm_set", {"root": "{MAIN}", "src": "01_原始素材/bgm.mp3",
                     "gainDb": -12, "ducking": True, "loop": True,
                     "summary": "对拍:背景乐"}, False),
        ("cut_apply", {"root": "{MAIN}",
                       "patch": {"script": [{"action": "review", "text": "对拍补丁旁白"}]}}, False),
        ("notes_add", {"root": "{MAIN}", "anchor": {"kind": "clip", "ref": "V1-001"},
                       "body": "这里语速偏快", "author": "agent", "tags": ["parity"],
                       "requestId": "parity-note-1"}, False),
        ("clip_update", {"root": "{MAIN}", "clipId": "V1-001", "patch": {"volume": 0.85},
                         "causedBy": ["n-0001"], "summary": "按 n-0001 微调音量"}, False),
        ("notes_resolve", {"root": "{MAIN}", "noteId": "n-0001", "reply": "已微调音量 0.85",
                           "opIds": ["@OP_OF_CAUSED_UPDATE@"]}, False),
        ("notes_add", {"root": "{MAIN}", "anchor": {"kind": "time", "tMs": 500},
                       "body": "开头音效偏响", "author": "user"}, False),
        ("notes_reject", {"root": "{MAIN}", "noteId": "n-0002", "reason": "风格即如此"}, False),
        # -- 阶段 C2:时间线编辑全工具(册四 A4 T4.2;27 写工具收口) --
        # 此时 V1:V1-001[0,1800) V1-002[4000,10000) V1-003[12000,14000)
        # (V1-004 已被 redo 二次删除,不复存在;roll 一例落点无贴合邻居,
        #  记录 GUARD_FAILED 守护拒绝面——正向语义由引擎单测与协议测试覆盖)
        # gap_delete 闭的是 [1800,4000) 共 2200ms 间隙:后继整体左移 2200。
        ("track_update", {"root": "{MAIN}", "trackId": "T1",
                          "patch": {"name": "字幕轨", "color": "#22CC88", "heightPx": 120}}, False),
        ("track_update", {"root": "{MAIN}", "trackId": "A1",
                          "patch": {"mute": True, "solo": False}}, False),
        ("clip_trim", {"root": "{MAIN}", "clipId": "V1-002", "mode": "trim",
                       "edge": "out", "deltaMs": -1000}, False),
        ("clip_gap_delete", {"root": "{MAIN}", "trackId": "V1", "tMs": 1900}, False),
        ("clip_trim", {"root": "{MAIN}", "clipId": "V1-003", "mode": "roll",
                       "edge": "out", "deltaMs": 400}, False),
        ("clip_trim", {"root": "{MAIN}", "clipId": "V1-003", "mode": "slip", "deltaMs": 250}, False),
        ("clip_trim", {"root": "{MAIN}", "clipId": "V1-003", "mode": "slide", "deltaMs": 600}, False),
        ("clip_split_all", {"root": "{MAIN}", "tMs": 5000}, False),
        ("clip_copy", {"root": "{MAIN}", "clipId": "V1-003"}, False),
        ("clip_paste_at", {"root": "{MAIN}", "trackId": "V1", "startMs": 15000,
                           "requestId": "parity-paste-1"}, False),
        # -- 阶段 C3:文本/字幕/媒体工具(册四 A4-BE3b;56 写/编排/查询面) --
        ("clip_update", {"root": "{MAIN}", "clipId": "V1-001",
                         "patch": {"textStyle": {"fontSize": 72, "color": "#FFCC00", "align": "topCenter"},
                                    "huazi": {"template": "hz.pop"},
                                    "denoise": "mid", "pitch": -4}}, False),
        ("text_add", {"root": "{MAIN}", "text": "新文本", "atMs": 0, "durationMs": 900,
                      "textStyle": {"fontSize": 64, "color": "#FFFFFF"},
                      "requestId": "parity-text-1"}, False),
        ("subtitle_import", {"root": "{MAIN}", "src": "05_时间线工程/subs.srt",
                             "requestId": "parity-imp-1"}, False),
        ("subtitle_replace", {"root": "{MAIN}", "find": "第一句", "replace": "改一句"}, False),
        ("subtitle_export", {"root": "{MAIN}", "format": "srt", "trackId": "T1"}, False),
        ("subtitle_export", {"root": "{MAIN}", "format": "ass"}, False),
        ("media_peaks", {"root": "{MAIN}", "src": "01_原始素材/bgm.mp3", "level": "coarse"}, True),
        ("media_peaks", {"root": "{MAIN}", "src": "01_原始素材/bgm.mp3", "level": "coarse"}, True),
        ("media_thumbnail", {"root": "{MAIN}", "src": "01_原始素材/take1.mp4", "atMs": 500}, True),
        ("media_proxy", {"root": "{MAIN}", "src": "01_原始素材/take1.mp4"}, True),
        ("media_proxy", {"root": "{MAIN}", "src": "01_原始素材/take1.mp4", "generate": False}, True),
        ("audio_beats", {"root": "{MAIN}", "src": "01_原始素材/bgm.mp3", "sensitivity": 0.5}, True),
        # -- 阶段 C4:册五 T5.2/T5.3/T5.6(调色 LUT/示波器/响度计/编码探测 + 新 IR 面) --
        ("lut_import", {"root": "{MAIN}", "src": "01_原始素材/test.cube"}, False),
        ("clip_update", {"root": "{MAIN}", "clipId": "V1-001",
                         "patch": {"grade": {"lift": [0.2, 0.0, -0.1], "saturation": 1.2,
                                             "curves": {"master": [[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]]},
                                             "lut": ".cutforge/luts/test.cube"}}},
         False),
        ("track_update", {"root": "{MAIN}", "trackId": "A1",
                          "patch": {"eq": [{"type": "peaking", "freq": 200, "gain": -6, "q": 1.0},
                                           {"type": "highshelf", "freq": 8000, "gain": 3}],
                                    "dyn": {"thresholdDb": -24, "ratio": 4}}}, False),
        ("bgm_set", {"root": "{MAIN}", "duckThreshold": 0.05, "duckRatio": 6,
                     "duckAttackMs": 40, "duckReleaseMs": 300}, False),
        ("scope_data", {"root": "{MAIN}", "src": "01_原始素材/frame.png"}, False),
        ("audio_loudness", {"root": "{MAIN}", "src": "01_原始素材/bgm.mp3", "target": -14}, True),
        ("encode_probe", {"root": "{MAIN}", "trial": False}, True),
        # -- 阶段 C5:册五 T5.4/T5.5 专业编辑与互操作(新七工具;68 收口) --
        # multicam_sync 双素材(take2 = take1 字节级复制 → 零偏移确定性);
        # probe_mode:confidence 档位量化 + durationsMs 100ms 量化(AAC 样本数微差)
        ("multicam_sync", {"root": "{MAIN}",
                           "angles": ["01_原始素材/take1.mp4", "01_原始素材/take2.mp4"]}, True),
        # multicam_cut:切换序列展开(单 Op;V1 此时空闲 [20000,24000))
        ("multicam_cut", {"root": "{MAIN}", "trackId": "V1", "startMs": 20000,
                          "durationMs": 4000,
                          "angles": [{"src": "01_原始素材/take1.mp4", "offsetMs": 0},
                                     {"src": "01_原始素材/take2.mp4", "offsetMs": 250}],
                          "switches": [{"tMs": 0, "angle": 0}, {"tMs": 1500, "angle": 1}],
                          "requestId": "parity-mc-1"}, False),
        # scene_detect:硬切夹具(2s 红 + 2s 蓝;5fps 抽帧)——先纯检测,
        # 再 autoSplit 真切段(单 Op;切点 ~2000ms 落在 V2 的 [0,4000) 片段内部)
        ("track_add", {"root": "{MAIN}", "kind": "video", "requestId": "parity-track-2"}, False),
        ("clip_add", {"root": "{MAIN}", "trackId": "V2", "src": "01_原始素材/hardcut.mp4",
                      "startMs": 0, "durationMs": 4000, "requestId": "parity-add-hc"}, False),
        ("scene_detect", {"root": "{MAIN}", "src": "01_原始素材/hardcut.mp4",
                          "sampleFps": 5}, True),
        ("scene_detect", {"root": "{MAIN}", "src": "01_原始素材/hardcut.mp4",
                          "sampleFps": 5, "autoSplit": {"trackId": "V2"}}, True),
        # compound_create → compound_unbind 轻量组合(V2 两段切后首尾相接;
        # 真实产物断言见 product_assert:innerClips=2 / unbound=2)
        ("compound_create", {"root": "{MAIN}", "clipIds": ["V2-001", "V2-002"],
                             "toTrack": "V2", "startMs": 0,
                             "requestId": "parity-compound-1"}, False),
        ("compound_unbind", {"root": "{MAIN}", "clipId": "@COMPOUND_ID@"}, False),
        # otio_export → otio_import → otio_export 往返(真实产物断言:语义 diff=0)
        ("otio_export", {"root": "{MAIN}", "format": "otio"}, False),
        ("otio_import", {"root": "{NEW2}", "src": "@OTIO_OUT@"}, False),
        ("otio_export", {"root": "{NEW2}", "format": "otio",
                         "out": "06_成片输出/roundtrip.otio"}, False),
        # -- 阶段 D:写后查询(投影面) --
        ("project_get", {"root": "{MAIN}"}, False),
        ("timeline_get", {"root": "{MAIN}"}, False),
        ("notes_list", {"root": "{MAIN}"}, False),
        ("oplog_tail", {"root": "{MAIN}", "limit": 4}, False),
        ("stage_status", {"root": "{MAIN}"}, False),
        ("conflict_list", {"root": "{MAIN}"}, False),
        # -- 阶段 E:渲染(真实 cutforge-render 子进程;异步进度轮询) --
        ("render_run", {"root": "{MAIN}"}, False),
        ("render_progress", {"root": "{MAIN}", "runId": "@RUN_ID@"}, False),
        ("render", {"root": "{MAIN}", "backend": "cutforge"}, False),
        # -- 阶段 E2:T2.4 单帧精确预览(整片渲后 video 链全命中,只做抽帧) --
        ("render_frame", {"root": "{MAIN}", "atMs": 1500}, False),
        # -- 阶段 E3:册五 T5.6 渲染队列(整片渲已完成 → 终态快照确定) --
        ("render_queue", {"root": "{MAIN}", "action": "list"}, False),
        # -- 阶段 F:编排(CUTFLOW_REPO 桩;锁派发/参数透传/stdout 透传行为) --
        ("render", {"root": "{MAIN}", "backend": "ffmpeg"}, False),
        ("export_jianying", {"root": "{MAIN}", "name": "parity-成片"}, False),
        ("stage_run", {"root": "{MAIN}", "stage": "S3",
                       "scriptArgs": ["--only", "S3"]}, False),
        ("stage_rebuild", {"root": "{MAIN}", "dir": "05_时间线工程",
                           "scriptArgs": ["05_时间线工程"]}, False),
        ("verify_run", {"root": "{MAIN}", "level": "L1",
                        "scriptArgs": ["--level", "L1"]}, False),
        ("sync_check", {"root": "{MAIN}",
                        "video": "06_成片输出/final_cutforge_parity-main_1080x1920.mp4",
                        "qc": True, "scriptArgs": ["--video", "06_成片输出/final.mp4"]}, False),
        # -- 阶段 G:册六 T6.1(布局迁移/工程库/崩溃恢复;72 收口) --
        # migrate:V2 工程(带素材)→ v3 一次性,再跑幂等 NOOP;冲突拒绝面一并入 golden
        ("project_new", {"root": "{MIG}", "slug": "parity-mig", "fps": 30,
                         "tracks": ["video"]}, False),
        ("migrate_layout", {"root": "{MIG}", "to": "v3"}, False),
        ("migrate_layout", {"root": "{MIG}", "to": "v3"}, False),
        ("migrate_layout", {"root": "{MIG}", "to": "v2"}, False),
        # library:七操作 + 搜索(卡片 mtime/路径走归一化;durationMs 为 IR 派生,确定性)
        ("library_manage", {"root": "{LIB}", "action": "new", "name": "parity-a",
                            "slug": "库甲", "fps": 30}, False),
        ("library_manage", {"root": "{LIB}", "action": "new", "name": "parity-a"}, False),
        ("library_manage", {"root": "{LIB}", "action": "copy", "name": "parity-a",
                            "to": "parity-b"}, False),
        ("library_list", {"root": "{LIB}"}, False),
        ("library_list", {"root": "{LIB}", "query": "parity-b"}, False),
        ("library_manage", {"root": "{LIB}", "action": "rename", "name": "parity-b",
                            "to": "parity-c"}, False),
        ("library_manage", {"root": "{LIB}", "action": "archive", "name": "parity-c"}, False),
        ("library_list", {"root": "{LIB}", "includeArchived": True}, False),
        ("library_manage", {"root": "{LIB}", "action": "unarchive", "name": "parity-c"}, False),
        ("library_manage", {"root": "{LIB}", "action": "delete", "name": "parity-c"}, False),
        ("library_manage", {"root": "{LIB}", "action": "delete", "name": "无此工程"}, False),
        # recover:清单为空(无残留锁,确定性);recover 缺 name 拒绝面
        ("library_recover", {"root": "{LIB}", "action": "list"}, False),
        ("library_recover", {"root": "{LIB}", "action": "recover"}, False),
    ]


def resolve_placeholders(args: dict, ctx: dict, main_ws: Path, new_ws: Path,
                         import_ws: Path, mig_ws: Path, lib_root: Path) -> dict:
    """深递归替换:{MAIN}/{NEW}/{NEW2}/{MIG}/{LIB} 路径标记与 @...@ 动态接线占位(含列表内元素)。"""
    def walk(v):
        if isinstance(v, str):
            v = (v.replace("{MAIN}", str(main_ws)).replace("{NEW}", str(new_ws))
                  .replace("{NEW2}", str(import_ws)).replace("{MIG}", str(mig_ws))
                  .replace("{LIB}", str(lib_root))
                  .replace("@OP_OF_CAUSED_UPDATE@", ctx.get("caused_update_op", ""))
                  .replace("@RUN_ID@", ctx.get("run_id", ""))
                  .replace("@COMPOUND_ID@", ctx.get("compound_id", ""))
                  .replace("@OTIO_OUT@", ctx.get("otio_out", "")))
            if "@" in v and "@" in ctx_marker_scan(v):
                raise ParityError(f"FAIL: 动态接线占位未解析(序列/回执接线有误): {v!r}", 2)
            return v
        if isinstance(v, dict):
            return {k: walk(x) for k, x in v.items()}
        if isinstance(v, list):
            return [walk(x) for x in v]
        return v
    return walk(args)


def ctx_marker_scan(v: str) -> str:
    """剩余 @X@ 形态标记检测(正常值里不应再出现 @ 包围的标记)。"""
    import re as _re
    m = _re.search(r"@[A-Z_]+@", v)
    return m.group(0) if m else ""


def _rt_ms(v: dict | None) -> int:
    """RationalTime.value(秒)→ 毫秒(与 Rust 侧 rt_ms 同式:×1000 四舍五入)。"""
    if not isinstance(v, dict):
        return 0
    return int(round(float(v.get("value", 0)) * 1000))


def otio_semantic_rows(doc: dict) -> list:
    """OTIO 文档 → 语义行 [轨序, kind, (src, 轨位ms, durms, sourceInms), …]。

    clip 名/id 不进比较 —— 导入侧 id 重新确定性分配,语义等价即可(与 Rust 侧
    pro_ops_tools_full_chain 往返投影等价同口径,多一道产物级实证)。轨位由
    Gap/Clip 游标累计重建(Transition 不占轨位,与导出写序一致)。
    """
    rows: list = []
    for tr in doc.get("tracks", {}).get("children", []):
        if tr.get("OTIO_SCHEMA") != "Track.1":
            continue
        items: list = []
        cursor = 0
        for ch in tr.get("children", []):
            schema = ch.get("OTIO_SCHEMA")
            sr = ch.get("source_range") or {}
            if schema == "Gap.1":
                cursor += _rt_ms(sr.get("duration"))
            elif schema == "Clip.1":
                dur = _rt_ms(sr.get("duration"))
                src = ch.get("media_reference", {}).get("target_url", "")
                items.append((src, cursor, dur, _rt_ms(sr.get("start_time"))))
                cursor += dur
            elif schema == "Stack.1":
                items.append(("compound", cursor, json.dumps(ch).count("Clip.1"), 0))
        rows.append((tr.get("kind"), tr.get("name"), items))
    return rows


def product_assert(name: str, args: dict, resp: dict, ctx: dict) -> None:
    """新七工具轻量组合的**真实产物断言**(采集/对比两模式都跑;失败 = ParityError(2)
    阻断,不进 golden 也不降级 WARN)——响应值进 golden 之外,组合行为必须有实证。"""
    if not isinstance(resp, dict) or not resp.get("ok"):
        return  # 失败响应由 golden 逐字段对拍把关(本断言只锁成功面)
    data = resp.get("data", {})
    if name == "multicam_sync":
        offs = [a.get("offsetMs") for a in data.get("angles", [])]
        if offs != [0, 0]:
            raise ParityError(f"FAIL: 同源复制素材互相关偏移必须为 [0,0]: {offs}", 2)
        if data.get("confidence") != 1.0:
            raise ParityError(f"FAIL: 同源素材置信度必须为 1.0: {data.get('confidence')}", 2)
    elif name == "multicam_cut":
        if data.get("segments") != 2 or data.get("angles") != 2:
            raise ParityError(f"FAIL: 多机位展开必须 2 段 2 角度: {data}", 2)
    elif name == "scene_detect" and "autoSplit" in args:
        if (data.get("cutCount") or 0) < 1 or not data.get("autoSplit", {}).get("points"):
            raise ParityError(f"FAIL: 硬切夹具必须检出 ≥1 剪切点且完成自动切段: {data}", 2)
    elif name == "compound_create":
        if data.get("innerClips") != 2 or data.get("durationMs") != 4000:
            raise ParityError(f"FAIL: 打包产物必须 2 子片段/4000ms: {data}", 2)
    elif name == "compound_unbind":
        if data.get("unbound") != 2:
            raise ParityError(f"FAIL: 解包必须还原 2 子片段: {data}", 2)
    elif name == "otio_export" and "roundtrip.otio" in str(args.get("out", "")):
        if "otio_first_doc" not in ctx:
            raise ParityError("FAIL: 往返断言缺第一次导出上下文", 2)
        second_doc = json.loads(Path(args["root"], args["out"]).read_text(encoding="utf-8"))
        if otio_semantic_rows(ctx["otio_first_doc"]) != otio_semantic_rows(second_doc):
            raise ParityError("FAIL: OTIO 往返语义 diff ≠ 0(导出→导入→再导出必须等价)", 2)


# ---------------- serve 生命周期 ----------------


def spawn_serve(bin_path: Path, ws: Path, tmp: Path, stub: Path):
    """3 次端口尝试起 serve;CUTFLOW_REPO 指向桩目录(编排确定性)。"""
    env = dict(os.environ)
    env["CUTFLOW_REPO"] = str(stub)
    for attempt in range(3):
        port = free_port()
        serve = subprocess.Popen(
            [str(bin_path), "serve", "--root", str(ws), "--port", str(port),
             "--token", TOKEN, "--web", str(REPO / "apps" / "web")],
            stdout=subprocess.DEVNULL, stderr=open(tmp / f"serve-{attempt}.log", "wb"),
            env=env)
        deadline = time.time() + 10
        ready = False
        while time.time() < deadline:
            if serve.poll() is not None:
                break  # 进程退出(bind 失败等)→ 换端口重试
            try:
                urllib.request.urlopen(
                    f"http://127.0.0.1:{port}/session?token={TOKEN}", timeout=1).read()
                ready = True
                break
            except Exception:
                time.sleep(0.15)
        if ready:
            return serve, port
        serve.terminate()
        time.sleep(0.3)
    log = tmp / "serve-2.log"
    tail = log.read_text("utf-8", errors="replace")[-400:] if log.exists() else ""
    raise ParityError(f"FAIL: serve 3 次尝试均未就绪;日志尾部: {tail}", 2)


def wait_render_done(port: int, run_id: str, timeout: float = 300) -> dict:
    """轮询真实渲染任务直到终态(采集与对比都等到 ok/fail,保证进度快照确定)。"""
    deadline = time.time() + timeout
    last: dict = {}
    while time.time() < deadline:
        last = rpc(port, TOKEN, "render_progress", {"root": "", "runId": run_id})
        if last.get("data", {}).get("state") in ("ok", "fail"):
            return last
        time.sleep(0.5)
    raise ParityError(
        f"FAIL: 渲染 {timeout:.0f}s 未到终态: {json.dumps(last, ensure_ascii=False)[:300]}", 2)


# ---------------- 主流程 ----------------


def run(update: bool, bin_arg: str | None) -> tuple[int, Report, str, int]:
    """返回 (退出码, 报告, 模式描述, 覆盖工具数)。"""
    if shutil.which("ffmpeg") is None:
        raise ParityError("NO_ENV: 未找到 ffmpeg(媒体夹具无法生成;安装后重跑)", 3)
    bin_path = find_bin(bin_arg)
    GOLDEN_DIR.mkdir(parents=True, exist_ok=True)
    report = Report()
    seq = build_sequence()
    mode = "采集模式(重建 golden)" if update else "对比模式"

    tmp = Path(tempfile.mkdtemp(prefix="cutforge-tool-parity-"))
    serve = None
    try:
        main_ws = tmp / "proot"
        (main_ws / "05_时间线工程").mkdir(parents=True)
        # serve preflight 要求工程文件先在位:主工作区预置与 scaffold 同模板的空工程
        # (V1/A1 空轨;后续全部状态经 MCP 写工具产生,不旁路)
        (main_ws / "05_时间线工程" / "project.json").write_text(json.dumps({
            "version": 1, "schemaVersion": "2.0.0", "slug": "parity-main",
            "fps": 30, "canvas": {"width": 1080, "height": 1920},
            "backends": ["ffmpeg", "cutforge"], "notes": "notes.json",
            "tracks": [
                {"id": "V1", "kind": "video", "clips": []},
                {"id": "A1", "kind": "audio", "clips": []},
            ],
        }, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")

        stub = write_cutflow_stub(tmp, main_ws)
        make_media(main_ws)
        write_truth_fixtures(main_ws)
        write_subtitle_fixture(main_ws)

        serve, port = spawn_serve(bin_path, main_ws, tmp, stub)
        norm = Normalizer(tmp, TOKEN)
        ctx: dict = {}

        collected: dict[str, list[dict]] = {}
        for name, raw_args, probe_mode in seq:
            args = resolve_placeholders(raw_args, ctx, main_ws, tmp / "wsnew", tmp / "wsimport",
                                        tmp / "wsmig", tmp / "lib")
            try:
                resp = rpc(port, TOKEN, name, args)
            except Exception as exc:  # noqa: BLE001
                report.add(name, "FAIL", f"/rpc 调用异常: {exc!r}")
                continue
            # 动态接线:回执喂给后续调用(render_progress 的 runId、notes_resolve 的 opIds、
            # compound_unbind 的壳 id、otio_import 的产物路径)
            if name == "render_run" and resp.get("ok"):
                ctx["run_id"] = resp["data"]["runId"]
                done = wait_render_done(port, ctx["run_id"])
                if done.get("data", {}).get("state") != "ok":
                    report.add(name, "FAIL",
                               f"真实渲染未成功: {done.get('data', {}).get('error')}")
                    continue
            if name == "clip_update" and args.get("causedBy") == ["n-0001"] and resp.get("ok"):
                ctx["caused_update_op"] = resp["data"]["opIds"][0]
            if name == "compound_create" and resp.get("ok"):
                ctx["compound_id"] = resp["data"]["clipId"]
            if name == "otio_export" and resp.get("ok"):
                if "roundtrip.otio" in str(args.get("out", "")):
                    pass  # 往返第二跳:产物断言在 product_assert 里做语义 diff
                else:
                    ctx["otio_out"] = str(main_ws / resp["data"]["out"])
                    ctx["otio_first_doc"] = json.loads(
                        (main_ws / resp["data"]["out"]).read_text(encoding="utf-8"))
            # 新七工具轻量组合的真实产物断言(采集/对比同跑;失败 → ParityError)
            product_assert(name, args, resp, ctx)
            collected.setdefault(name, []).append({
                "args": norm(args),  # 参数仅存档不参与对比(动态接线值已确定)
                "response": norm(resp, probe_mode),
            })

        # ---------------- 对比 / 建库 ----------------
        covered = list(collected.keys())
        for name in covered:
            calls = collected[name]
            gpath = GOLDEN_DIR / f"{name}.json"
            if update:
                gpath.write_text(json.dumps({"tool": name, "calls": calls},
                                            ensure_ascii=False, indent=1) + "\n",
                                 encoding="utf-8")
                report.add(name, "PASS", f"golden 已重建({len(calls)} 次调用)")
                continue
            if not gpath.exists():
                report.add(name, "FAIL", f"golden 缺失: {gpath.name}")
                continue
            golden = json.loads(gpath.read_text(encoding="utf-8"))
            gcalls = golden.get("calls", [])
            if len(gcalls) != len(calls):
                report.add(name, "DRIFT",
                           f"调用次数不一致 golden={len(gcalls)} actual={len(calls)}")
                continue
            alldiff: list[str] = []
            allextra: list[str] = []
            for i, (g, a) in enumerate(zip(gcalls, calls)):
                diffs: list[str] = []
                extras: list[str] = []
                compare(g.get("response"), a["response"], f"call{i}", diffs, extras)
                alldiff += diffs
                allextra += extras
            if alldiff:
                head = "; ".join(alldiff[:4]) + (f"(共 {len(alldiff)} 处)" if len(alldiff) > 4 else "")
                report.add(name, "DRIFT", head, allextra[:8])
            elif allextra:
                report.add(name, "WARN", f"容忍项 {len(allextra)} 条(加法键/启发式容差,仅警告)",
                           allextra[:8])
            else:
                report.add(name, "PASS", f"{len(calls)} 次调用逐字段一致")

        report.missing = sorted({n for n, _, _ in seq} - set(covered))
        code = 0 if report.ok() else 2
        return code, report, mode, len(covered)
    finally:
        if serve is not None:
            serve.terminate()
        shutil.rmtree(tmp, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser(description="MCP 工具黄金响应库对拍(AC-1.2;工具数以 schemas/mcp-tools.json 为准)")
    ap.add_argument("--update-golden", action="store_true", help="重建 tools/bench/golden/*.json")
    ap.add_argument("--bin", default=None, help="cutforge-mcp 二进制路径(默认 target/debug)")
    ap.add_argument("--json", action="store_true", help="只输出结果协议 envelope JSON")
    args = ap.parse_args()
    try:
        code, report, mode, n_tools = run(args.update_golden, args.bin)
    except ParityError as e:
        if args.json:
            print(json.dumps({"ok": False, "code": "NO_ENV" if e.code == 3 else "GATE_FAILED",
                              "message": str(e), "data": {}}, ensure_ascii=False))
        else:
            print(str(e), file=sys.stderr)
        return e.code
    env = report.envelope(mode, n_tools)
    if args.json:
        print(json.dumps(env, ensure_ascii=False))
    else:
        print(f"tool_parity: {mode}")
        for r in report.rows:
            print(f"  {r['tool']}: {r['status']} — {r['detail']}")
            for e in r["extras"]:
                print(f"      {e}")
        for m in report.missing:
            print(f"  {m}: MISSING — 调用阶段异常,无响应入库")
        print(env["message"])
    return code


if __name__ == "__main__":
    sys.exit(main())
