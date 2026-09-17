#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""回归集样本构建器(计划书 3.10 回归集要求)。

三类 videoType 各一套工程 JSON 产物(project/wordline/cutlist/notes/oplog),
字段形状严格按生产者代码考古结论构造(rs_align/rs_cut/rs_ir,2026-09-17),
替代已不存在的现网工程样本(v0.14 磁盘清理后本机无真实工程;观察项,
待下一个真实工程落地后用真实产物替换并复跑门禁)。

    python tests/regression/_build_samples.py   # 幂等,重建全部样本
"""
from __future__ import annotations

import json
from pathlib import Path

HERE = Path(__file__).parent


def write(dir_name: str, files: dict) -> None:
    d = HERE / dir_name
    d.mkdir(parents=True, exist_ok=True)
    for name, doc in files.items():
        if name.endswith(".jsonl"):
            text = "".join(json.dumps(op, ensure_ascii=False) + "\n" for op in doc)
        else:
            text = json.dumps(doc, ensure_ascii=False, indent=1) + "\n"
        (d / name).write_text(text, encoding="utf-8", newline="\n")
    print(f"built {dir_name}: {len(files)} files")


def wordline(source: str, chars_spec, gaps, sentences, *, src_ms, final_ms,
             hard_chars, coverage, conf_median, degraded=False,
             degrade_reasons=None, estimated=False, tts=False):
    chars = []
    for i, (ch, a, b) in enumerate(chars_spec):
        c = {"i": i, "ch": ch, "startMs": a, "endMs": b, "srcStartMs": a, "srcEndMs": b,
             "conf": 0.40 if estimated else 0.86}
        if estimated:
            c["estimated"] = True
        chars.append(c)
    doc = {
        "version": 1, "source": source,
        "space": "source", "fps": 30,
        "chars": chars, "gaps": gaps, "sentences": sentences,
        "speakers": [], "srcDurationMs": src_ms, "finalDurationMs": final_ms,
        "stats": {"charCount": hard_chars, "coverage": coverage, "confMedian": conf_median},
        "degraded": degraded, "degradeReasons": degrade_reasons or [],
        "charTimingEstimated": estimated,
    }
    if tts:
        doc["dub"] = {
            "similarity": 0.94,
            "rows": [{"sentence": "今天讲重点。", "estStartMs": 4200, "realStartMs": 4310,
                      "driftMs": 110, "needsReview": False}],
            "medianDriftMs": 110, "p95DriftMs": 110, "needsReview": [],
            "source": "03_assets/tts/manifest.json",
        }
    else:
        doc["asr"] = {
            "backend": "pkg",
            "capabilities": {"charTimestamps": True, "hotwords": True, "speaker": False},
            "elapsedSec": 12.4, "segmentCount": 2,
            "asrDegraded": False, "asrDegradeReasons": [],
            "hotwords": "安信德 GEO优化",
        }
        doc["calibration"] = {
            "sentences": 2, "snapped": 1, "skipped": 0,
            "medianBeforeMs": 210.0, "medianAfterMs": 12.0,
            "p95BeforeMs": 240.0, "windowMs": 500.0, "minShiftMs": 40.0,
            "shifts": [{"charRange": [4, 10], "shiftMs": -198.0}],
        }
        doc["calibrated"] = True
        doc["smooth"] = {"applied": True, "zeroWidthPunct": 2, "clampedOverlap": 1, "minWidthFixed": 0}
    return doc


def cutlist(source, cuts, keep, removed_ms, src_ms, script, from_text=None):
    doc = {
        "version": 1, "source": source,
        "detector": {"version": "cutflow-1.1", "params": {
            "silenceMinMs": 600, "tailKeepMs": 60, "retakeRatio": 0.6,
            "confRemove": 0.9, "confReview": 0.6}},
        "cuts": cuts, "keep": keep, "removedMs": removed_ms,
        "srcTotalMs": src_ms, "script": script,
    }
    if from_text:
        doc["fromText"] = from_text
    return doc


def notes_sample(items):
    return {"version": 1, "items": items}


def oplog_sample(ops):
    return ops


def main() -> None:
    # ---------- 1. talking-head ----------
    th_spec = [("大", 200, 380), ("家", 380, 560), ("好", 560, 760),
               ("，", 760, 800), ("今", 1420, 1600), ("天", 1600, 1760),
               ("讲", 1760, 1940), ("重", 1940, 2120), ("点", 2120, 2320),
               ("。", 2320, 2360)]
    write("talking-head", {
        "project.json": {
            "version": 1, "slug": "20260820-口播样板-talking-head", "fps": 30,
            "canvas": {"width": 1080, "height": 1920}, "outputs": ["9x16"],
            "joinCrossfadeMs": 120,
            "tracks": [
                {"kind": "video", "name": "主轨", "clips": [
                    {"src": "01_materials/a.mp4", "startMs": 0, "durationMs": 8400,
                     "sourceInMs": 12000, "volume": 1.0, "role": "voice"},
                    {"src": "01_materials/a.mp4", "startMs": 8400, "durationMs": 6200,
                     "sourceInMs": 21600, "volume": 1.0, "role": "voice",
                     "transition": {"type": "fade", "durMs": 300, "reason": "topic"},
                     "punchIn": {"factor": 1.4, "source": "auto"}},
                ]},
                {"kind": "audio", "clips": [
                    {"src": "03_assets/sfx/whoosh.mp3", "startMs": 8400, "durationMs": 400,
                     "role": "sfx", "volume": 0.8}]},
            ],
            "bgm": {"src": "03_assets/bgm/loop1.mp3", "gainDb": -18, "ducking": True, "loop": True},
            "markers": [{"ms": 8400, "label": "要点2"}],
            "subtitle": {"ass": "06_output/subtitles.ass",
                         "source": "02_sensed/transcript_corrected.json",
                         "style": "talkshow-bold"},
        },
        "wordline.json": wordline(
            "01_materials/a.mp4", th_spec,
            [{"after": 3, "ms": 620, "kind": "silence"}],
            [{"id": 0, "span": [0, 4], "punc": "，", "text": "大家好，"},
             {"id": 1, "span": [4, 10], "punc": "。", "text": "今天讲重点。"}],
            src_ms=15200, final_ms=14600, hard_chars=8, coverage=0.9932, conf_median=0.86),
        "cutlist.json": cutlist(
            "01_materials/a.mp4",
            [
                {"id": "c001", "inMs": 4200, "outMs": 4820, "reason": "silence", "conf": 0.93,
                 "action": "remove", "note": "",
                 "guard": {"inSilence": True, "outSilence": True, "wordClipped": False,
                           "tailKeepMs": 80, "required": ["inSilence", "outSilence", "wordClipped", "tailKeep"],
                           "ok": True, "okByReason": True},
                 "text": "大家好，[620ms 静音]今天讲重点。"},
                {"id": "c002", "inMs": 9100, "outMs": 9350, "reason": "filler", "conf": 0.72,
                 "action": "review", "note": "[rhetorical_pause_suspect]",
                 "guard": {"inSilence": True, "outSilence": False, "wordClipped": False,
                           "tailKeepMs": 45, "required": ["wordClipped", "tailKeep"],
                           "ok": False, "okByReason": True},
                 "text": "讲重点。嗯…收尾"},
            ],
            [[0, 4200], [4820, 9100], [9350, 15200]], 870, 15200,
            [{"action": "keep", "text": "大家好，(删 620ms 静音)今天讲重点。"},
             {"action": "review", "text": "'嗯'待审"}]),
        "notes.json": notes_sample([
            {"id": "n-0001",
             "anchor": {"kind": "clip", "ref": "V1-002", "tMs": 12340,
                        "span": {"startMs": 12000, "endMs": 13500}},
             "body": "这里语速太快，把这一段后面 300ms 的空白再删一点",
             "author": "user", "state": "open", "createdAt": "2026-09-17T10:00:00+08:00",
             "tags": ["节奏"]},
            {"id": "n-0002",
             "anchor": {"kind": "track", "ref": "A1", "tMs": 0},
             "body": "这条音效声音偏大",
             "author": "user", "state": "resolved", "createdAt": "2026-09-17T10:05:00+08:00",
             "resolvedBy": {"reply": "音量 0.8→0.6 已改", "opIds": ["op-8821"]}},
        ]),
        "oplog.jsonl": [
            {"opId": "op-8821", "ts": "2026-09-17T10:06:01.123+08:00",
             "actor": {"kind": "agent", "id": "cutflow-rs_cut"},
             "target": {"file": "project.json", "path": "/tracks/1/clips/0/volume"},
             "opKind": "set", "before": 0.8, "after": 0.6, "baseRev": "rev-7", "rev": 8,
             "causedBy": ["n-0002"], "summary": "音效音量 0.8→0.6"},
            {"opId": "op-8822", "ts": "2026-09-17T10:07:22.456+08:00",
             "actor": {"kind": "user", "id": "user-ui"},
             "target": {"file": "project.json", "path": "/tracks/0/clips/1/durationMs"},
             "opKind": "set", "before": 6500, "after": 6200, "baseRev": "rev-8", "rev": 9,
             "summary": "手动收短尾段 300ms"},
        ],
    })

    # ---------- 2. talking-head+animation ----------
    ta_spec = th_spec
    write("talking-head+animation", {
        "project.json": {
            "version": 1, "slug": "20260825-口播加动画-talking-head+animation", "fps": 30,
            "canvas": {"width": 1080, "height": 1920}, "outputs": ["9x16", "3x4"],
            "joinCrossfadeMs": 120,
            "tracks": [
                {"kind": "video", "name": "主轨", "clips": [
                    {"src": "01_materials/b.mp4", "startMs": 0, "durationMs": 9200,
                     "sourceInMs": 3000, "volume": 1.0, "role": "voice"},
                    {"src": "03_assets/artboard/cards/kc01.mp4", "startMs": 9200, "durationMs": 4800,
                     "volume": 0.0, "role": "ambient", "motion": {"in": "scaleIn", "inMs": 400,
                                                                  "out": "fadeOut", "outMs": 300},
                     "transition": {"durMs": 300, "reason": "topic"},
                     "freezeMs": 3850},
                ]},
                {"kind": "text", "clips": [
                    {"text": "重点一:口径统一", "startMs": 9400, "durationMs": 2400,
                     "position": {"x": 0.5, "y": 0.82}, "scale": 1.0}]},
            ],
            "bgm": {"src": "03_assets/bgm/loop2.mp3", "gainDb": -20, "ducking": True, "loop": True},
            "subtitle": {"ass": "06_output/subtitles.ass",
                         "source": "02_sensed/transcript_corrected.json",
                         "style": "talkshow-bold"},
        },
        "wordline.json": wordline(
            "01_materials/b.mp4", ta_spec,
            [{"after": 3, "ms": 620, "kind": "silence"}],
            [{"id": 0, "span": [0, 4], "punc": "，", "text": "大家好，"},
             {"id": 1, "span": [4, 10], "punc": "。", "text": "今天讲重点。"}],
            src_ms=16200, final_ms=14600, hard_chars=8, coverage=0.9951, conf_median=0.88),
        "cutlist.json": cutlist(
            "01_materials/b.mp4",
            [{"id": "c001", "inMs": 11200, "outMs": 13050, "reason": "retake", "conf": 0.97,
              "action": "remove", "note": "",
              "guard": {"inSilence": True, "outSilence": True, "wordClipped": False,
                        "tailKeepMs": 120, "required": ["inSilence", "outSilence", "wordClipped", "tailKeep"],
                        "ok": True, "okByReason": True},
              "text": "重录段整段删除"}],
            [[0, 11200], [13050, 16200]], 1850, 16200,
            [{"action": "keep", "text": "(重录段已删)"}]),
        "notes.json": notes_sample([
            {"id": "n-0001",
             "anchor": {"kind": "time", "ref": None, "tMs": 9400},
             "body": "卡片出场能否提前到 9.2s?",
             "author": "user", "state": "open", "createdAt": "2026-09-17T11:00:00+08:00",
             "tags": ["动画"]},
        ]),
        "oplog.jsonl": [
            {"opId": "op-9001", "ts": "2026-09-17T11:02:00.000+08:00",
             "actor": {"kind": "script", "id": "rs_ir --from-cards"},
             "target": {"file": "project.json", "path": "/tracks/0/clips/1/freezeMs"},
             "opKind": "set", "before": None, "after": 3850, "baseRev": "rev-3", "rev": 4,
             "summary": "冻结帧补长 3850ms(卡片 3.85s < 旁白 4.8s)"},
        ],
    })

    # ---------- 3. pure-animation ----------
    pa_spec = [(ch, 300 + i * 260, 300 + i * 260 + 220)
               for i, ch in enumerate("纯动画配音样例")]
    write("pure-animation", {
        "project.json": {
            "version": 1, "slug": "20260901-安信德GEO-pure-animation", "fps": 30,
            "canvas": {"width": 1080, "height": 1920}, "outputs": ["9x16"],
            "joinCrossfadeMs": 120,
            "tracks": [
                {"kind": "video", "name": "卡片轨", "clips": [
                    {"src": "03_assets/artboard/cards/geo01.mp4", "startMs": 0, "durationMs": 5600,
                     "volume": 0.0, "motion": {"in": "fadeIn", "inMs": 350},
                     "freezeMs": 2300},
                    {"src": "03_assets/artboard/cards/geo02.mp4", "startMs": 5600, "durationMs": 6100,
                     "volume": 0.0, "transition": {"type": "circleopen", "durMs": 500},
                     "freezeMs": 3050},
                ]},
                {"kind": "audio", "clips": [
                    {"src": "03_assets/tts/vo01.wav", "startMs": 0, "durationMs": 11700,
                     "volume": 1.0, "role": "voice"}]},
            ],
            "bgm": {"src": "03_assets/bgm/tech.mp3", "gainDb": -22, "ducking": True, "loop": True},
            "subtitle": {"ass": "06_output/subtitles.ass",
                         "source": "03_assets/tts/manifest.json",
                         "style": "tutorial-clean"},
        },
        "wordline.json": wordline(
            "03_assets/tts/manifest.json", pa_spec,
            [{"after": 3, "ms": 900, "kind": "silence"}],
            [{"id": 0, "span": [0, 4], "punc": "，", "text": "纯动画，"},
             {"id": 1, "span": [4, 8], "punc": "。", "text": "配音样例。"}],
            src_ms=11700, final_ms=11700, hard_chars=6, coverage=0.9918, conf_median=0.4,
            degraded=True,
            degrade_reasons=["TTS 路径：句级时间 + 句内均分",
                             "字级时间戳缺失：句内按均分估算(不可当字级用)"],
            estimated=True, tts=True),
        "cutlist.json": cutlist(
            "03_assets/tts/manifest.json",
            [{"id": "c001", "inMs": 6000, "outMs": 6900, "reason": "manual", "conf": 1.0,
              "action": "remove", "note": "人工:删中段空拍",
              "guard": {"inSilence": True, "outSilence": True, "wordClipped": False,
                        "tailKeepMs": 200, "required": ["inSilence", "outSilence", "wordClipped", "tailKeep"],
                        "ok": True, "okByReason": True},
              "text": "空拍 900ms"}],
            [[0, 6000], [6900, 11700]], 900, 11700,
            [{"action": "keep", "text": "(空拍已删)"}],
            from_text={"quote": "空拍", "charsSpan": [4, 6], "ms": [6000, 6900]}),
        "notes.json": notes_sample([
            {"id": "n-0001",
             "anchor": {"kind": "subtitleCard", "ref": "V1-001", "tMs": 2300},
             "body": "第二张卡的动画播完后停顿偏久,冻结帧缩短 500ms",
             "author": "agent", "state": "open", "createdAt": "2026-09-17T12:00:00+08:00",
             "tags": ["节奏", "AI建议"]},
        ]),
        "oplog.jsonl": [
            {"opId": "op-9101", "ts": "2026-09-17T12:03:00.000+08:00",
             "actor": {"kind": "agent", "id": "claude"},
             "target": {"file": "notes.json", "path": "/items/0"},
             "opKind": "insert", "before": None, "after": {"id": "n-0001"},
             "baseRev": "rev-11", "rev": 12,
             "causedBy": ["n-0001"],
             "summary": "AI 反向提问:冻结帧偏久"},
        ],
    })


if __name__ == "__main__":
    main()
