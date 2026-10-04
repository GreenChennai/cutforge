// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 字幕工作流纯函数层(册四 A4 T4.7):SRT/ASS ↔ 文本片段的解析与导出。
//!
//! **SRT 往返零丢失**(AC-4.4):`srt_parse → srt_format` 对规范形输入 byte 级
//! 幂等(parse(format(parse(x))) == parse(x),单测锁定);时间精度 = 毫秒
//! (SRT 原生精度,零舍入损失);文本保留多行(clip.text 内含 \n,ASS 生成端
//! 转 \N)。ASS 导入只取 Dialogue 的时窗与纯文本(剥 override 标签,\N → \n)——
//! 样式属 textStyle/huazi 契约,不随导入伪造。
//!
//! 解析纪律(与全仓一致):非法块跳过不中断(逐块容错),零合法块返回错误由
//! 调用方翻译协议码;确定性(同输入同产出,无环境读取)。

/// 解析出的单条字幕(与文本片段的字段映射:atMs/durationMs/text)。
#[derive(Debug, Clone, PartialEq)]
pub struct SubLine {
    pub at_ms: u64,
    pub duration_ms: u64,
    pub text: String,
}

/// SRT 时间戳 `HH:MM:SS,mmm`(兼容 `.` 分隔)→ ms;非法 → None。
pub fn srt_time_parse(s: &str) -> Option<u64> {
    let s = s.trim();
    let (hms, msm) = match s.split_once(',') {
        Some((a, b)) => (a, b),
        None => s.split_once('.')?,
    };
    let msm = if msm.len() == 3 {
        msm
    } else {
        msm.split('.').nth(1)?
    };
    let parts: Vec<&str> = hms.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: u64 = parts[0].trim().parse().ok()?;
    let m: u64 = parts[1].trim().parse().ok()?;
    let sec: u64 = parts[2].trim().parse().ok()?;
    let ms: u64 = msm.parse().ok()?;
    Some(h * 3_600_000 + m * 60_000 + sec * 1000 + ms)
}

/// ms → SRT 时间戳 `HH:MM:SS,mmm`(确定性与溢出安全)。
pub fn srt_time_format(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms % 3_600_000 / 60_000,
        ms % 60_000 / 1000,
        ms % 1000
    )
}

/// SRT 解析:块间空行分隔;序号行可选;时间行 `-->`;正文多行(保留换行)。
/// 非法块跳过(逐块容错);零合法块 → None(调用方报协议错)。
pub fn srt_parse(input: &str) -> Option<Vec<SubLine>> {
    let mut out: Vec<SubLine> = Vec::new();
    for block in input.replace("\r\n", "\n").split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.is_empty() {
            continue;
        }
        // 时间行定位(序号行可选):第一处含 "-->" 的行
        let Some(ti) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let mut seg = lines[ti].split("-->");
        let (Some(a), Some(b)) = (seg.next(), seg.next()) else {
            continue;
        };
        let (Some(at), Some(end)) = (srt_time_parse(a), srt_time_parse(b)) else {
            continue;
        };
        if end <= at {
            continue;
        }
        let text = lines[ti + 1..].join("\n");
        if text.trim().is_empty() {
            continue;
        }
        out.push(SubLine {
            at_ms: at,
            duration_ms: end - at,
            text,
        });
    }
    if out.is_empty() { None } else { Some(out) }
}

/// SRT 导出:序号从 1 连续;块间空行;时间戳毫秒精度零损失。
pub fn srt_format(lines: &[SubLine]) -> String {
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            srt_time_format(l.at_ms),
            srt_time_format(l.at_ms + l.duration_ms),
            l.text
        ));
    }
    out
}

/// 文本轨片段 → SRT(按 atMs 升序、序号重排;字幕导出的工具面入口)。
pub fn srt_export_clips(clips: &[ExportClip]) -> String {
    let mut sorted: Vec<&ExportClip> = clips.iter().collect();
    sorted.sort_by_key(|c| (c.at_ms, c.id.clone()));
    let lines: Vec<SubLine> = sorted
        .iter()
        .map(|c| SubLine {
            at_ms: c.at_ms,
            duration_ms: c.duration_ms,
            text: c.text.clone(),
        })
        .collect();
    srt_format(&lines)
}

/// ASS 时间戳 `H:MM:SS.cc` → ms(厘秒 ×10;非法 → None)。
pub fn ass_time_parse(s: &str) -> Option<u64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: u64 = parts[0].trim().parse().ok()?;
    let m: u64 = parts[1].trim().parse().ok()?;
    let (sec, cs) = parts[2].trim().split_once('.')?;
    if cs.len() != 2 {
        return None;
    }
    Some(
        h * 3_600_000 + m * 60_000 + sec.parse::<u64>().ok()? * 1000 + cs.parse::<u64>().ok()? * 10,
    )
}

/// ASS Dialogue 文本 → 纯文本:剥 {...} override 标签,\N/\n → 换行。
pub fn ass_text_plain(body: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for ch in body.chars() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.replace("\\N", "\n")
        .replace("\\n", "\n")
        .trim_matches('\n')
        .to_string()
}

/// ASS 导入:取 Dialogue 行(时窗 + 纯文本);时窗非法/正文空的行跳过;
/// 零合法行 → None。
pub fn ass_parse(input: &str) -> Option<Vec<SubLine>> {
    let mut out: Vec<SubLine> = Vec::new();
    for line in input.lines() {
        let Some(rest) = line.strip_prefix("Dialogue:") else {
            continue;
        };
        let fields: Vec<&str> = rest.splitn(10, ',').collect();
        if fields.len() < 10 {
            continue;
        }
        let (Some(at), Some(end)) = (ass_time_parse(fields[1]), ass_time_parse(fields[2])) else {
            continue;
        };
        if end <= at {
            continue;
        }
        let text = ass_text_plain(fields[9]);
        if text.trim().is_empty() {
            continue;
        }
        out.push(SubLine {
            at_ms: at,
            duration_ms: end - at,
            text,
        });
    }
    if out.is_empty() { None } else { Some(out) }
}

/// 文本片段视图(调用方从工程文本轨投影;按 atMs 升序导出)。
#[derive(Debug, Clone, PartialEq)]
pub struct ExportClip {
    pub id: String,
    pub at_ms: u64,
    pub duration_ms: u64,
    pub text: String,
}

/// ASS 导出(文本轨 → ASS 文档;样式恒 Default 基线——导出的样式契约面在
/// textStyle 字段,不回填花字/卡拉OK标签;正文换行 → \N)。
pub fn ass_export(clips: &[ExportClip], canvas_w: u32, canvas_h: u32) -> String {
    let mut styles = String::new();
    let mut events = String::new();
    let mut sorted: Vec<&ExportClip> = clips.iter().collect();
    sorted.sort_by_key(|c| (c.at_ms, c.id.clone()));
    for c in &sorted {
        let id = &c.id;
        let mv = canvas_h as u64 / 10;
        styles.push_str(&format!(
            "Style: CF_{id},sans-serif,64,&H00FFFFFF,&H00FFFFFF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,40,40,{mv},1\n"
        ));
        events.push_str(&format!(
            "Dialogue: 0,{},{},CF_{id},,0,0,0,,{}\n",
            crate::textass::ass_time(c.at_ms),
            crate::textass::ass_time(c.at_ms + c.duration_ms),
            c.text
                .replace('\r', "")
                .replace('\n', "\\N")
                .replace('{', "｛")
                .replace('}', "｝"),
        ));
    }
    format!(
        "[Script Info]\n; exported by cutforge (deterministic text ASS)\nScriptType: v4.00+\nPlayResX: {w}\nPlayResY: {h}\nWrapStyle: 2\nScaledBorderAndShadow: yes\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n{styles}\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n{events}",
        w = canvas_w,
        h = canvas_h,
    )
}

/// 自动识别格式:WEBVTT 头 → VTT;SRT(含 `-->` 与 `,mmm` 形时间)次之;
/// 否则按 ASS Dialogue。
pub fn parse_auto(input: &str) -> Option<Vec<SubLine>> {
    if input.trim_start().starts_with("WEBVTT") {
        return vtt_parse(input);
    }
    if input.contains("-->") {
        srt_parse(input)
            .or_else(|| vtt_parse(input))
            .or_else(|| ass_parse(input))
    } else {
        ass_parse(input)
            .or_else(|| srt_parse(input))
            .or_else(|| vtt_parse(input))
    }
}

// ---------------- WebVTT(册五 T5.5;SRT↔VTT 差异:逗号→点 + WEBVTT 头) ----------------

/// VTT 时间戳 `HH:MM:SS.mmm`(兼容短形 `MM:SS.mmm`;非法 → None)。
pub fn vtt_time_parse(s: &str) -> Option<u64> {
    let s = s.trim();
    // 剥离 cue settings(时间后跟的空白+非时间 token 由调用方切;此处容忍纯时间)
    let parts: Vec<&str> = s.split(':').collect();
    let (h, m, sec) = match parts.len() {
        3 => (parts[0], parts[1], parts[2]),
        2 => ("0", parts[0], parts[1]),
        _ => return None,
    };
    let h: u64 = h.trim().parse().ok()?;
    let m: u64 = m.trim().parse().ok()?;
    let (sec, msm) = sec.trim().split_once('.')?;
    if msm.len() != 3 {
        return None;
    }
    Some(h * 3_600_000 + m * 60_000 + sec.parse::<u64>().ok()? * 1000 + msm.parse::<u64>().ok()?)
}

/// ms → VTT 时间戳 `HH:MM:SS.mmm`(点分隔;确定性与溢出安全)。
pub fn vtt_time_format(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms % 3_600_000 / 60_000,
        ms % 60_000 / 1000,
        ms % 1000
    )
}

/// VTT 解析:首行 WEBVTT 头(必需);NOTE/STYLE/REGION 块跳过;cue 正文多行保留。
/// 非法 cue 跳过(逐块容错);零合法 cue → None(调用方报协议错)。
pub fn vtt_parse(input: &str) -> Option<Vec<SubLine>> {
    let input = input.replace("\r\n", "\n");
    if !input.trim_start().starts_with("WEBVTT") {
        return None;
    }
    let mut out: Vec<SubLine> = Vec::new();
    for block in input.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.is_empty() {
            continue;
        }
        let first = lines[0].trim();
        if first.starts_with("WEBVTT")
            || first.starts_with("NOTE")
            || first.starts_with("STYLE")
            || first.starts_with("REGION")
        {
            continue;
        }
        // 时间行定位(cue id 行可选):第一处含 "-->" 的行
        let Some(ti) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let mut seg = lines[ti].split("-->");
        let (Some(a), Some(b)) = (seg.next(), seg.next()) else {
            continue;
        };
        // cue settings(终点时间后的空白+token)剥除
        let b_time: String = b
            .trim()
            .chars()
            .take_while(|c| !c.is_whitespace())
            .collect();
        let (Some(at), Some(end)) = (vtt_time_parse(a), vtt_time_parse(&b_time)) else {
            continue;
        };
        if end <= at {
            continue;
        }
        let text = lines[ti + 1..].join("\n");
        if text.trim().is_empty() {
            continue;
        }
        out.push(SubLine {
            at_ms: at,
            duration_ms: end - at,
            text,
        });
    }
    if out.is_empty() { None } else { Some(out) }
}

/// VTT 导出:WEBVTT 头 + cue(时序毫秒精度零损失;无序号——VTT 惯例,cue id 缺省)。
pub fn vtt_format(lines: &[SubLine]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for l in lines {
        out.push_str(&format!(
            "{} --> {}\n{}\n\n",
            vtt_time_format(l.at_ms),
            vtt_time_format(l.at_ms + l.duration_ms),
            l.text
        ));
    }
    out
}

/// 文本轨片段 → VTT(按 atMs 升序;字幕导出的工具面入口)。
pub fn vtt_export_clips(clips: &[ExportClip]) -> String {
    let mut sorted: Vec<&ExportClip> = clips.iter().collect();
    sorted.sort_by_key(|c| (c.at_ms, c.id.clone()));
    let lines: Vec<SubLine> = sorted
        .iter()
        .map(|c| SubLine {
            at_ms: c.at_ms,
            duration_ms: c.duration_ms,
            text: c.text.clone(),
        })
        .collect();
    vtt_format(&lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANONICAL_SRT: &str = "1\n00:00:00,500 --> 00:00:02,500\n你好世界\n\n2\n00:00:03,000 --> 00:00:04,250\n第二行字幕\n跨行文本\n\n3\n00:00:05,000 --> 00:00:06,000\n带标点,字幕。\n\n";

    #[test]
    fn srt_time_roundtrip() {
        assert_eq!(srt_time_parse("00:00:00,500"), Some(500));
        assert_eq!(srt_time_parse("01:02:03,456"), Some(3_723_456));
        assert_eq!(srt_time_parse("0:00:01.500"), Some(1500), "点分隔兼容");
        assert_eq!(srt_time_parse("垃圾"), None);
        assert_eq!(srt_time_parse("00:00:00"), None);
        assert_eq!(srt_time_format(0), "00:00:00,000");
        assert_eq!(srt_time_format(3_723_456), "01:02:03,456");
        assert_eq!(srt_time_format(86_399_999), "23:59:59,999");
    }

    /// SRT 往返零丢失(byte 级):parse → format 对规范形输入逐字节还原;
    /// 再 parse 语义相等(幂等不动点)。
    #[test]
    fn srt_roundtrip_zero_loss() {
        let lines = srt_parse(CANONICAL_SRT).expect("夹具必须可解析");
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            SubLine {
                at_ms: 500,
                duration_ms: 2000,
                text: "你好世界".into()
            }
        );
        assert_eq!(lines[1].text, "第二行字幕\n跨行文本", "多行保留");
        assert_eq!(lines[2].text, "带标点,字幕。");
        let back = srt_format(&lines);
        assert_eq!(back, CANONICAL_SRT, "byte 级往返零丢失");
        let lines2 = srt_parse(&back).unwrap();
        assert_eq!(lines, lines2, "再解析语义相等(幂等不动点)");
    }

    /// 容错:序号行缺失/非时间块跳过/零合法块 None。
    #[test]
    fn srt_parse_tolerates_and_rejects() {
        let loose = "00:00:01,000 --> 00:00:02,000\n无序号行\n\n这不是字幕块\n\n10\n00:00:02,000 --> 00:00:01,000\n时间倒置跳过\n\n";
        let lines = srt_parse(loose).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "无序号行");
        assert!(srt_parse("完全没有时间行").is_none());
        assert!(srt_parse("").is_none());
    }

    #[test]
    fn ass_time_and_dialogue_parse() {
        assert_eq!(ass_time_parse("0:00:00.50"), Some(500));
        assert_eq!(ass_time_parse("1:02:03.46"), Some(3_723_460));
        assert_eq!(ass_time_parse("垃圾"), None);
        let ass = "[Script Info]\nScriptType: v4.00+\n\n[V4+ Styles]\nFormat: Name\nStyle: Default,Arial,60\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.50,0:00:02.50,Default,,0,0,0,,帧上{\\bord8}字幕\nDialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,两\\N行\n";
        let lines = ass_parse(ass).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            SubLine {
                at_ms: 500,
                duration_ms: 2000,
                text: "帧上字幕".into()
            },
            "override 标签剥除"
        );
        assert_eq!(lines[1].text, "两\n行", "\\N 转换行");
        assert!(ass_parse("无 Dialogue").is_none());
    }

    /// ASS 导入 → 导出 → 再导入:时窗与文本语义零丢失(AC-4.4 的 ASS 支路)。
    #[test]
    fn ass_import_export_semantic_roundtrip() {
        let src = "Dialogue: 0,0:00:00.50,0:00:02.50,Default,,0,0,0,,你好{\\i1}世界\nDialogue: 0,0:00:03.00,0:00:04.25,OP,,0,0,0,,第二句\n";
        let lines = ass_parse(src).unwrap();
        let clips: Vec<ExportClip> = lines
            .iter()
            .enumerate()
            .map(|(i, l)| ExportClip {
                id: format!("T1-{i:03}"),
                at_ms: l.at_ms,
                duration_ms: l.duration_ms,
                text: l.text.clone(),
            })
            .collect();
        let exported = ass_export(&clips, 1080, 1920);
        let back = ass_parse(&exported).unwrap();
        assert_eq!(back, lines, "ASS 语义往返零丢失");
        assert!(
            exported.contains("Dialogue: 0,0:00:00.50,0:00:02.50,CF_T1-000,,0,0,0,,你好世界"),
            "{exported}"
        );
    }

    /// SRT 导出确定性 + 按时序重排 + 空轨导出空串(调用方对空轨报 PRECONDITION)。
    #[test]
    fn srt_export_deterministic_and_sorted() {
        let clips = vec![
            ExportClip {
                id: "T1-002".into(),
                at_ms: 3000,
                duration_ms: 1000,
                text: "后".into(),
            },
            ExportClip {
                id: "T1-001".into(),
                at_ms: 500,
                duration_ms: 2000,
                text: "前".into(),
            },
        ];
        let a = srt_export_clips(&clips);
        let b = srt_export_clips(&clips);
        assert_eq!(a, b, "同输入同产物(确定性)");
        assert!(
            a.starts_with("1\n00:00:00,500 --> 00:00:02,500\n前"),
            "导出按 atMs 升序: {a}"
        );
        assert_eq!(srt_export_clips(&[]), "");
    }

    #[test]
    fn parse_auto_detects_format() {
        assert_eq!(
            parse_auto(CANONICAL_SRT),
            srt_parse(CANONICAL_SRT),
            "含 --> 识别为 SRT"
        );
        let ass = "Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,甲\n";
        assert_eq!(parse_auto(ass), ass_parse(ass), "无 --> 识别为 ASS");
        assert!(parse_auto("两者皆非").is_none());
    }

    // ---- 册五 T5.5:WebVTT(SRT→VTT 时间格式差异:逗号→点,WEBVTT 头) ----

    const CANONICAL_VTT: &str = "WEBVTT\n\n00:00:00.500 --> 00:00:02.500\n你好世界\n\n00:00:03.000 --> 00:00:04.250\n第二行字幕\n跨行文本\n\n";

    #[test]
    fn vtt_time_roundtrip() {
        assert_eq!(vtt_time_parse("00:00:00.500"), Some(500));
        assert_eq!(vtt_time_parse("01:02:03.456"), Some(3_723_456));
        assert_eq!(vtt_time_parse("01:02.500"), Some(62_500), "短形 MM:SS.mmm");
        assert_eq!(vtt_time_parse("垃圾"), None);
        assert_eq!(vtt_time_format(0), "00:00:00.000");
        assert_eq!(vtt_time_format(3_723_456), "01:02:03.456");
    }

    /// VTT 往返零丢失(本仓规范形 byte 级)+ 语义幂等;cue settings/NOTE 块容错。
    #[test]
    fn vtt_roundtrip_zero_loss() {
        let lines = vtt_parse(CANONICAL_VTT).expect("夹具必须可解析");
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            SubLine {
                at_ms: 500,
                duration_ms: 2000,
                text: "你好世界".into()
            }
        );
        assert_eq!(lines[1].text, "第二行字幕\n跨行文本", "多行保留");
        assert_eq!(vtt_format(&lines), CANONICAL_VTT, "byte 级往返零丢失");
        assert_eq!(
            vtt_parse(&vtt_format(&lines)).unwrap(),
            lines,
            "再解析语义相等"
        );
        // 容错:cue id 行 / cue settings / NOTE 块 / 零合法块
        let loose = "WEBVTT\n\nNOTE 这是一个注释块\n跨行注释\n\ncue-1\n00:00:01.000 --> 00:00:02.000 position:50%\n带设置\n\n00:00:03.000 --> 00:00:02.000\n倒置跳过\n\n";
        let lines = vtt_parse(loose).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "带设置");
        assert!(vtt_parse("没有 WEBVTT 头\n\n00:00:01.000 --> 00:00:02.000\nx").is_none());
        assert!(vtt_parse("WEBVTT\n\n").is_none());
        // 与 SRT 的格式差异锁:VTT 用点,SRT 用逗号(同毫秒内容互相转换等值)
        let srt_lines = srt_parse(CANONICAL_SRT).unwrap();
        let as_vtt = vtt_format(&srt_lines);
        assert!(
            as_vtt.starts_with("WEBVTT\n\n00:00:00.500 -->"),
            "VTT 头 + 点分隔: {as_vtt}"
        );
        assert_eq!(
            vtt_parse(&as_vtt).unwrap(),
            srt_lines,
            "SRT 内容经 VTT 格式化后解析等值"
        );
    }

    /// VTT 导出确定性 + 按时序重排;parse_auto 识别 WEBVTT 头优先。
    #[test]
    fn vtt_export_deterministic_and_auto_detected() {
        let clips = vec![
            ExportClip {
                id: "T1-002".into(),
                at_ms: 3000,
                duration_ms: 1000,
                text: "后".into(),
            },
            ExportClip {
                id: "T1-001".into(),
                at_ms: 500,
                duration_ms: 2000,
                text: "前".into(),
            },
        ];
        let a = vtt_export_clips(&clips);
        let b = vtt_export_clips(&clips);
        assert_eq!(a, b, "同输入同产物(确定性)");
        assert!(
            a.starts_with("WEBVTT\n\n00:00:00.500 --> 00:00:02.500\n前"),
            "{a}"
        );
        assert_eq!(parse_auto(&a), vtt_parse(&a), "WEBVTT 头识别优先");
    }
}
