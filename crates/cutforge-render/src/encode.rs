// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 最终编码(册五 T5.6;ADR-0020 色彩标签):编码器解析/质量映射/参数面/
//! bt709 标签与 ffprobe 复验。
//!
//! **设计定档**:
//! - encode 步从「原子拷贝中间产物」升级为**带 bt709 标签 remux**——ADR-0020
//!   决策 1 要求输出显式携带完整色彩标签,mp4 colr 框由输出流参数写入,
//!   `-c copy` + `-color_*` 即可携带全部标签(实测本机 build 复验可见),
//!   **零重编码、零代损失**,渲染经济与拆分前同量级;RENDERER_VERSION 随本册
//!   升版(8.0)。
//! - 显式编码选项(encoder=hw/sw 或 quality/crf/bitrate/gop/pix_fmt 任一)才走
//!   **重编码**:encoder auto/缺省语义 = 「不重编码」(确定性基线:同工程同产物
//!   是 parity 的根);hw = 试编探测后硬件重编码(失败优雅降级 libx264,WARN
//!   留痕,AC-5.6);sw = libx264 重编码。
//! - 质量预设:fast/balanced/quality → sw(crf,preset)=(28,veryfast)/(23,medium)/
//!   (18,slow);硬件按编码器各自映射(nvenc p1/p4/p7+cq、qsv veryfast/medium/slow+
//!   global_quality、amf speed/balanced/quality)。显式 crf/bitrate 覆盖预设。
//! - 探测缓存:encode_probe 工具面(mcp)缓存 `-encoders` 清单;渲染端不读缓存
//!   (试编即最可信探测,毫秒级 null 输出)。
//!
//! 本文件不启动进程的纯函数与启动探测进程的函数分区注明;探测函数可离线单测
//! (ffmpeg 缺失 → None,调用方降级)。

use crate::plan::RenderOptions;
use serde_json::{Value, json};
use std::path::Path;

/// 硬件编码器候选(试编顺序:nvenc → qsv → amf;CUDA/Intel/AMD 各归其位)。
pub const HW_CANDIDATES: [&str; 3] = ["h264_nvenc", "h264_qsv", "h264_amf"];

/// 质量预设 → (crf, x264 preset)。缺省 balanced(crf23 = 中间段同级)。
pub fn quality_settings(quality: Option<&str>) -> (u32, &'static str) {
    match quality {
        Some("fast") => (28, "veryfast"),
        Some("quality") => (18, "slow"),
        _ => (23, "medium"),
    }
}

/// 硬件编码器质量档(与 sw 档位同名的硬件映射;amf 用 -quality)。
fn hw_quality_args(encoder: &str, quality: Option<&str>, crf: Option<u32>) -> Vec<String> {
    let explicit = crf.map(|c| c.to_string());
    match encoder {
        "h264_nvenc" => {
            let p = match quality {
                Some("fast") => "p1",
                Some("quality") => "p7",
                _ => "p4",
            };
            let cq = explicit.unwrap_or_else(|| match quality {
                Some("fast") => "32".into(),
                Some("quality") => "22".into(),
                _ => "27".into(),
            });
            vec![
                "-preset".into(),
                p.into(),
                "-rc".into(),
                "vbr".into(),
                "-cq".into(),
                cq,
            ]
        }
        "h264_qsv" => {
            let p = match quality {
                Some("fast") => "veryfast",
                Some("quality") => "slow",
                _ => "medium",
            };
            let q = explicit.unwrap_or_else(|| match quality {
                Some("fast") => "32".into(),
                Some("quality") => "22".into(),
                _ => "27".into(),
            });
            vec!["-preset".into(), p.into(), "-global_quality".into(), q]
        }
        // amf:质量档 → -quality(speed/balanced/quality);码率模式为缺省,
        // crf/cq 无对应参数面(诚实映射,不硬造)
        _ => {
            let q = match quality {
                Some("fast") => "speed",
                Some("quality") => "quality",
                _ => "balanced",
            };
            vec!["-quality".into(), q.into()]
        }
    }
}

/// bt709 输出标签(ADR-0020 决策 1:恒 SDR bt709 + tv range)。
/// 双通道写入:输出旗标(-color_*)是编解码器 VUI 的常规入口;setparams 滤镜
/// 走帧属性 → 容器 nclx/VUI(实测本机 ffmpeg 2026-07-30 full build 下
/// `-color_primaries/-color_trc` 单独给不落 VUI,必须经 setparams 才复验可见)。
pub fn color_tag_args() -> [&'static str; 8] {
    [
        "-color_primaries",
        "bt709",
        "-color_trc",
        "bt709",
        "-colorspace",
        "bt709",
        "-color_range",
        "tv",
    ]
}

/// 标签的帧级通道(setparams;与 color_tag_args 同值双写,复验以 ffprobe 为准)。
pub const COLOR_TAG_SET_PARAMS: &str =
    "setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709";

/// 是否声明了重编码意图(hw/sw 或任一显式编码参数)。
pub fn needs_reencode(opts: &RenderOptions) -> bool {
    matches!(opts.encoder.as_deref(), Some("hw") | Some("sw"))
        || opts.quality.is_some()
        || opts.crf.is_some()
        || opts.bitrate_kbps.is_some()
        || opts.gop.is_some()
        || opts.pix_fmt.is_some()
}

/// 最终编码(缺省 remux+标签)命令行。返回 (参数, 编码器名;remux 为 "copy")。
/// 输入 = 合流中间产物(视频流 + aac 音频);声画流直通(混音语义已在 mix 步定形)。
pub fn final_encode_args(
    input: &Path,
    output: &Path,
    opts: &RenderOptions,
    hw: Option<&'static str>,
) -> (Vec<String>, String) {
    if hw.is_none() && !needs_reencode(opts) {
        // 缺省:remux + bt709 标签(零重编码;mp4 colr 由输出流参数写入,实测可复验)
        let mut args: Vec<String> = vec![
            "-y".into(),
            "-v".into(),
            "error".into(),
            "-i".into(),
            input.to_string_lossy().into(),
            "-c".into(),
            "copy".into(),
        ];
        args.extend(color_tag_args().iter().map(|s| s.to_string()));
        args.push(output.to_string_lossy().into());
        return (args, "copy".into());
    }
    let (crf_def, preset_def) = quality_settings(opts.quality.as_deref());
    let crf = opts.crf.unwrap_or(crf_def);
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-i".into(),
        input.to_string_lossy().into(),
    ];
    let encoder = hw.unwrap_or("libx264");
    match hw {
        Some(_) => {
            args.push("-c:v".into());
            args.push(encoder.into());
            args.extend(hw_quality_args(encoder, opts.quality.as_deref(), opts.crf));
        }
        None => {
            args.extend([
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                preset_def.into(),
            ]);
            if opts.bitrate_kbps.is_none() {
                args.extend(["-crf".into(), crf.to_string()]);
            }
        }
    }
    if let Some(kbps) = opts.bitrate_kbps {
        args.extend(["-b:v".into(), format!("{kbps}k")]);
    }
    if let Some(gop) = opts.gop {
        args.extend(["-g".into(), gop.to_string()]);
    }
    args.extend([
        "-pix_fmt".into(),
        opts.pix_fmt.clone().unwrap_or_else(|| "yuv420p".into()),
    ]);
    args.extend(["-vf".into(), COLOR_TAG_SET_PARAMS.into()]);
    args.extend(color_tag_args().iter().map(|s| s.to_string()));
    args.extend(["-c:a".into(), "copy".into()]);
    args.push(output.to_string_lossy().into());
    (args, encoder.to_string())
}

/// 硬件编码器试编探测(启动 ffmpeg;256px 0.1s null 输出,毫秒级):
/// `-encoders` 只证编译期在位,驱动/会话不可用时编不出帧——试编是最可信探测。
/// ffmpeg 不可用 → None(调用方优雅降级,不 panic)。
pub fn hw_encoder_usable(encoder: &str) -> bool {
    let out = std::process::Command::new(crate::ff_bin("ffmpeg"))
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=black:size=256x256:rate=30:duration=0.1",
            "-c:v",
            encoder,
            "-frames:v",
            "2",
            "-f",
            "null",
            "-",
        ])
        .output();
    matches!(out, Ok(o) if o.status.success())
}

/// hw 选项的解析:按候选序试编,首个可用者胜;全败 → None(调用方降级 sw)。
pub fn resolve_hw_candidate() -> Option<&'static str> {
    HW_CANDIDATES.iter().copied().find(|e| hw_encoder_usable(e))
}

/// 输出色彩标签复验(ADR-0020:标签在位纳入渲染自检)。
/// 启动 ffprobe;返回 stream 级色彩字段(缺标签/探测失败 = Err,调用方 WARN)。
pub fn verify_color_tags(output: &Path) -> Result<Value, String> {
    let out = std::process::Command::new(crate::ff_bin("ffprobe"))
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=color_primaries,color_transfer,color_space,color_range",
            "-print_format",
            "json",
            &output.to_string_lossy(),
        ])
        .output()
        .map_err(|e| format!("ffprobe 启动失败: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ffprobe 失败: {}",
            String::from_utf8_lossy(&out.stderr)
                .trim()
                .chars()
                .take(200)
                .collect::<String>()
        ));
    }
    let v: Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).map_err(|e| e.to_string())?;
    let stream = v
        .get("streams")
        .and_then(|s| s.get(0))
        .cloned()
        .ok_or("ffprobe 输出无视频流")?;
    // ffprobe 的传输特性字段名 = color_transfer(非 color_trc)
    let prim = stream["color_primaries"].as_str().unwrap_or("unknown");
    let trc = stream["color_transfer"].as_str().unwrap_or("unknown");
    let space = stream["color_space"].as_str().unwrap_or("unknown");
    let range = stream["color_range"].as_str().unwrap_or("unknown");
    let ok = prim == "bt709" && trc == "bt709" && space == "bt709";
    Ok(json!({
        "primaries": prim, "trc": trc, "space": space, "range": range,
        "inPlace": ok,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_opts() -> RenderOptions {
        RenderOptions::default()
    }

    /// 缺省参数面:remux(-c copy)+ bt709 四枚举,零重编码(ADR-0020 语义、
    /// 拆分前经济);显式编码选项才走重编码(sw medium crf23 + setparams 双写)。
    #[test]
    fn default_args_are_remux_with_bt709_tags() {
        let (args, enc) =
            final_encode_args(Path::new("/i.mp4"), Path::new("/o.mp4"), &base_opts(), None);
        assert_eq!(enc, "copy");
        let s = args.join("\u{1}");
        assert!(s.contains("-c\u{1}copy"), "{s}");
        assert!(s.contains("-color_primaries\u{1}bt709\u{1}-color_trc\u{1}bt709\u{1}-colorspace\u{1}bt709\u{1}-color_range\u{1}tv"), "{s}");
        assert!(!s.contains("-c:v"), "remux 不产编码器旗标: {s}");
        // 任一显式编码选项 → 重编码
        let mut o = base_opts();
        o.crf = Some(20);
        let (args, enc) = final_encode_args(Path::new("/i.mp4"), Path::new("/o.mp4"), &o, None);
        assert_eq!(enc, "libx264");
        let s = args.join("\u{1}");
        assert!(
            s.contains("-c:v\u{1}libx264\u{1}-preset\u{1}medium\u{1}-crf\u{1}20"),
            "{s}"
        );
        assert!(s.contains("-color_primaries\u{1}bt709\u{1}-color_trc\u{1}bt709\u{1}-colorspace\u{1}bt709\u{1}-color_range\u{1}tv"), "{s}");
        assert!(
            s.contains("setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709"),
            "{s}"
        );
        assert!(s.contains("-pix_fmt\u{1}yuv420p"), "{s}");
        assert!(s.contains("-c:a\u{1}copy"), "{s}");
    }

    /// 质量预设映射:fast/quality 两端;显式 crf 覆盖预设。
    #[test]
    fn quality_presets_map_to_crf_preset() {
        let mut o = base_opts();
        o.quality = Some("fast".into());
        let (args, _) = final_encode_args(Path::new("/i"), Path::new("/o"), &o, None);
        assert!(
            args.join("\u{1}")
                .contains("-preset\u{1}veryfast\u{1}-crf\u{1}28")
        );
        o.quality = Some("quality".into());
        let (args, _) = final_encode_args(Path::new("/i"), Path::new("/o"), &o, None);
        assert!(
            args.join("\u{1}")
                .contains("-preset\u{1}slow\u{1}-crf\u{1}18")
        );
        o.crf = Some(20);
        let (args, _) = final_encode_args(Path::new("/i"), Path::new("/o"), &o, None);
        assert!(args.join("\u{1}").contains("-crf\u{1}20"));
        // 码率模式:crf 让位
        o.bitrate_kbps = Some(6000);
        let (args, _) = final_encode_args(Path::new("/i"), Path::new("/o"), &o, None);
        let s = args.join("\u{1}");
        assert!(!s.contains("-crf"), "{s}");
        assert!(s.contains("-b:v\u{1}6000k"), "{s}");
    }

    /// 硬件参数面:nvenc/qsv/amf 各自映射 + GOP。
    #[test]
    fn hw_args_map_per_encoder() {
        let o = base_opts();
        for (enc, needle) in [
            ("h264_nvenc", "-rc\u{1}vbr\u{1}-cq\u{1}27"),
            ("h264_qsv", "-global_quality\u{1}27"),
            ("h264_amf", "-quality\u{1}balanced"),
        ] {
            let (args, name) = final_encode_args(Path::new("/i"), Path::new("/o"), &o, Some(enc));
            assert_eq!(name, enc);
            assert!(
                args.join("\u{1}").contains(needle),
                "{enc}: {}",
                args.join("\u{1}")
            );
        }
        let mut o = base_opts();
        o.gop = Some(60);
        let (args, _) = final_encode_args(Path::new("/i"), Path::new("/o"), &o, Some("h264_nvenc"));
        assert!(args.join("\u{1}").contains("-g\u{1}60"));
    }

    /// 标签参数面独立可取(8 元素 argv 片段)。
    #[test]
    fn color_tag_args_shape() {
        assert_eq!(color_tag_args().len(), 8);
        assert_eq!(color_tag_args()[1], "bt709");
    }
}
