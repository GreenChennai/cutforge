// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 渲染步骤纯函数(T1.4):每步 = 纯函数(生成 ffmpeg 参数)+ lib.rs 执行器调用。
//! 本文件**不启动任何进程**:全部函数只做输入 → 命令行字符串的映射,
//! 可在不装 ffmpeg 的环境单测断言;参数串与拆分前的 render() 逐字一致
//! (渲染输出逐字节语义不变的底线,由 parity_matrix 实渲夹具锁定)。
//! 册四 A4-BE2:段提取链(步 2 segment)纯移动至 [`crate::segment`];此处
//! `pub use` 保持 `steps::segment_*` 接口路径不变。

use crate::plan::{OverlaySeg, RenderPlan};
use cutforge_core::model::{Clip, Transition};
use std::path::{Path, PathBuf};

// ---- 步 2 segment(实现在 segment.rs;路径兼容再导出,册四 A4-BE2) ----
pub use crate::segment::{
    play_segments, reverse_chain, segment_args, segment_filter, segment_filter_complex,
    segment_pad_ms, segment_read_ms, transform_pre_chain,
};

/// clip i 的出向转场时长(ms;转场字段在 clip i 上表示 i-1→i 转场,i=0 无意义);
/// type=cut/none 或 durMs<=0 → 硬切(None)。
pub fn transition_out_ms(clip: &Clip) -> Option<(String, f64)> {
    if clip.start_ms == 0 && clip.transition.is_none() {
        return None;
    }
    let t: &Transition = clip.transition.as_ref()?;
    match t.type_.as_deref() {
        Some("cut") | Some("none") => return None,
        _ => {}
    }
    let dur = t.dur_ms.unwrap_or(0.0);
    if dur <= 0.0 {
        return None;
    }
    let kind = t.type_.clone().unwrap_or_else(|| "fade".into());
    Some((kind, dur))
}

/// 段 i 的尾帧扩展毫秒(ADR-0023):段 i 延长 D_{i+1} 供 seg_i/seg_{i+1} 间
/// xfade 消费,保证整链零时间漂移;该值入段缓存键(漏掉 → 改转场陈旧复用
/// tpad)。D 取**有效转场时长**(册四 BE3a 钳两侧,与 compose/acrossfade 同源)。
pub fn segment_tail_ms(video_clips: &[Clip], i: usize) -> f64 {
    if i + 1 < video_clips.len() {
        crate::catalog::effective_transition_ms(video_clips, i + 1)
    } else {
        0.0
    }
}

// ---------------- 步 1 probe ----------------

/// probe 目标:全部存在的素材源(容错:图片等无时长素材合法存在;真坏了在 segment 步炸)。
/// 覆盖三路来源:主时间线视频段 / 音频事件 / overlay 叠加层。
pub fn probe_paths(plan: &RenderPlan) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for c in &plan.video_clips {
        if let Some(src) = &c.src {
            out.push(plan.project_dir.join(src));
        }
    }
    for s in &plan.audio_segs {
        out.push(s.src.clone());
    }
    for o in &plan.overlay_segs {
        out.push(o.src.clone());
    }
    out.retain(|p| p.is_file());
    out
}

// ---------------- 步 3 compose(xfade 链 / concat 退化) ----------------

/// xfade 链启用判定(I1 修收紧):链内**每个**边界都必须「时间线相接(前段
/// 末端 = 后段起点)+ 有效转场 > 0」。ADR-0023 的尾帧/offset 对齐口径只在全
/// 转场链自洽(seg_i 的尾帧恰被边界 i+1 的 xfade 消费,[x_k] 实际长度 = 名义
/// 累计);存在 d=0 边界时该边界生成 duration=0 的 xfade,ffmpeg 直接丢弃第二
/// 输入(cf-demo 实测:三段工程合成片只有前两段,第三段整体消失,抽帧越过
/// 合成 EOF → 空产出 INTERNAL)。不满足即整链退化 concat(-c copy 零重编码;
/// 空隙折叠为前段末帧定格,与 timeline_to_compose_ms 的 seg_durs 口径同源;
/// 已声明转场被降级硬切,由 compose 步 WARN 留痕)。
pub fn is_xfade_chain(video_clips: &[Clip]) -> bool {
    video_clips.len() > 1
        && (1..video_clips.len()).all(|i| {
            crate::catalog::effective_transition_ms(video_clips, i) > 0.0
                && video_clips[i - 1].start_ms + video_clips[i - 1].duration_ms
                    == video_clips[i].start_ms
        })
}

/// xfade 链命令行:offset_k = **前序名义时长累计**(ADR-0023:尾帧只进转场重叠
/// 不前移 offset——旧实现把尾帧计入累计导致 xfade 坍缩截断,BE3a 实测修复)。
/// 返回 (参数, WARN 列表)——转场名经目录直通,未注册降级 fade 留痕(T4.5)。
pub fn compose_xfade_args(
    video_clips: &[Clip],
    seg_files: &[PathBuf],
    composed_out: &Path,
) -> (Vec<String>, Vec<String>) {
    let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];
    for f in seg_files {
        args.extend(["-i".into(), f.to_string_lossy().into()]);
    }
    let mut filters: Vec<String> = Vec::new();
    let mut warns: Vec<String> = Vec::new();
    let mut acc_nominal_ms = 0f64;
    let mut cur_label = "[0:v]".to_string();
    for i in 1..video_clips.len() {
        let dur = crate::catalog::effective_transition_ms(video_clips, i);
        acc_nominal_ms += video_clips[i - 1].duration_ms as f64;
        let offset = acc_nominal_ms / 1000.0;
        let (kind, warn) = crate::catalog::resolve_transition(&video_clips[i]);
        if let Some(w) = warn {
            warns.push(format!("clip {} 转场: {w}", video_clips[i].id));
        }
        let out_label = format!("[x{i}]");
        filters.push(format!(
            "{cur_label}[{i}:v]xfade=transition={kind}:duration={:.6}:offset={:.6}{out_label}",
            dur / 1000.0,
            offset
        ));
        cur_label = out_label;
    }
    let last_label = format!("[x{}]", video_clips.len() - 1);
    args.extend([
        "-filter_complex".into(),
        filters.join(";"),
        "-map".into(),
        last_label,
    ]);
    args.extend([
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        composed_out.to_string_lossy().into(),
    ]);
    (args, warns)
}

/// concat 清单内容(路径统一正斜杠;demuxer 对引号内反斜杠敏感)。
pub fn concat_list_content(seg_files: &[PathBuf]) -> String {
    let mut list = String::new();
    for f in seg_files {
        list.push_str(&format!(
            "file '{}'\n",
            f.to_string_lossy().replace('\\', "/")
        ));
    }
    list
}

/// concat 合成命令行(-c copy 不重编码)。
pub fn compose_concat_args(concat_list: &Path, composed_out: &Path) -> Vec<String> {
    [
        "-y",
        "-v",
        "error",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        &concat_list.to_string_lossy(),
        "-c",
        "copy",
        &composed_out.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

// ---------------- 步 4 overlay ----------------

/// overlay 合成命令行:绝对像素 + opacity(colorchannelmixer)+ 时间窗 enable。
pub fn overlay_args(
    overlay_segs: &[OverlaySeg],
    composed_in: &Path,
    overlaid_out: &Path,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-i".into(),
        composed_in.to_string_lossy().into(),
    ];
    for ov in overlay_segs {
        args.extend(["-i".into(), ov.src.to_string_lossy().into()]);
    }
    let mut filters: Vec<String> = Vec::new();
    let mut cur = "[0:v]".to_string();
    for (i, ov) in overlay_segs.iter().enumerate() {
        let scale_chain = format!("scale={}:{}", ov.spec.w, ov.spec.h);
        let chain = if (ov.spec.opacity - 1.0).abs() < f64::EPSILON {
            scale_chain
        } else {
            format!(
                "{scale_chain},format=rgba,colorchannelmixer=aa={:.4}",
                ov.spec.opacity
            )
        };
        // 层输入 = i+1(基片恒输入 0,叠加源从 1 起;A5-BE3 修复:旧实现 [i:v]
        // 把基片自身缩放叠加——红底红 logo 不可见的潜伏缺陷,compound 夹具 lime 检出)
        let inp = i + 1;
        filters.push(format!(
            "[{inp}:v]{chain}[l{i}];{cur}[l{i}]overlay=x={}:y={}:enable='between(t,{:.3},{:.3})'[o{}]",
            ov.spec.x, ov.spec.y,
            ov.start_ms as f64 / 1000.0,
            (ov.start_ms + ov.duration_ms) as f64 / 1000.0,
            i
        ));
        cur = format!("[o{i}]");
    }
    args.extend([
        "-filter_complex".into(),
        filters.join(";"),
        "-map".into(),
        cur,
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        overlaid_out.to_string_lossy().into(),
    ]);
    args
}

// ---------------- 步 4.5 adjust(册五 T5.4 调整层) ----------------

/// 调整层时间窗处理命令行(纯函数,可离线单测):每片段
/// `trim 抽窗 → setpts 归零 → fx+grade 链作用于窗内流 → overlay enable 贴回`;
/// 不依赖滤镜级 enable(任意链可窗内生效);空链片段跳过;编码与 overlay 同款。
pub fn adjust_args(
    adjust_clips: &[Clip],
    base_video: &Path,
    adjusted_out: &Path,
    project_dir: &Path,
    w: u32,
    h: u32,
    fps: u32,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-i".into(),
        base_video.to_string_lossy().into(),
    ];
    let mut filters: Vec<String> = Vec::new();
    let mut cur = "[0:v]".to_string();
    let mut n = 0usize;
    for c in adjust_clips {
        let (fx, _) = crate::catalog::fx_chain(c, w, h, fps);
        let (grade, _) = crate::grade::grade_chain(c, project_dir);
        let mut chain = String::new();
        for part in [grade, fx] {
            if !part.is_empty() {
                if !chain.is_empty() {
                    chain.push(',');
                }
                chain.push_str(&part);
            }
        }
        if chain.is_empty() {
            continue; // 无 fx/grade 声明 = 无处理(时间窗纯占位)
        }
        let s = c.start_ms as f64 / 1000.0;
        let e = (c.start_ms + c.duration_ms) as f64 / 1000.0;
        let win_in = format!("[w{n}in]");
        let win_out = format!("[w{n}]");
        let merged_out = format!("[a{n}]");
        filters.push(format!(
            "{cur}trim=start={s:.3}:end={e:.3},setpts=PTS-STARTPTS{win_in}"
        ));
        filters.push(format!("{win_in}{chain}{win_out}"));
        filters.push(format!(
            "{cur}{win_out}overlay=enable='between(t,{s:.3},{e:.3})'{merged_out}"
        ));
        cur = merged_out;
        n += 1;
    }
    if filters.is_empty() {
        // 全部片段无链:透传拷贝(不产滤镜图)
        args.extend([
            "-c".into(),
            "copy".into(),
            adjusted_out.to_string_lossy().into(),
        ]);
        return args;
    }
    args.extend([
        "-filter_complex".into(),
        filters.join(";"),
        "-map".into(),
        cur,
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        adjusted_out.to_string_lossy().into(),
    ]);
    args
}

// ---------------- 步 5 mix(画幅无关,共享缓存) ----------------

/// mix pass A 命令行:主时间线有视频转场时走 acrossfade 链(册四 T4.5,M11-R1;
/// 见 [`crate::across`] 模块注释),否则既有逐段落点路径(参数逐字一致,parity 红线)。
/// 册五 T5.3:有轨道处理声明(plan.track_proc 非空)时按轨组建流,per-track
/// EQ/动态链在组建流 amix 之后、进总线之前插入;无声明 = 既有图零变化。
pub fn mix_pass_a_args(plan: &RenderPlan, mixed_raw_out: &Path) -> Vec<String> {
    // 全音轨可用 = 既有行为(包装仅供既有调用面/单测兼容)
    let all = vec![true; plan.audio_segs.len()];
    mix_pass_a_args_with(plan, mixed_raw_out, &all, true)
}

/// 同 [`mix_pass_a_args`],逐事件素材的音轨可用性显式给定(I1 渲染缺口修):
/// `segs_has` 与 audio_segs 同序。素材纯视频(无音频流)的段不开 `-i`——
/// `[N:a]` 对其匹配零个流,整图 Invalid argument(纯视频时间线是最常见形态)——
/// 改用 anullsrc 静音占位(adelay 落点与段长照旧):占位的混音贡献 = 恒等静音,
/// 且时间域贡献必须保留(amix duration=longest 需要占位流撑住段落在时间线上的
/// 位置,剔除会让成片音轨短于视频轨)。amix 输入数恒 = 段数。BGM 素材无音轨 →
/// 按无 BGM 处理(静音背景乐 = 无)。全部段无音轨 → 图照常(全占位 = 等价静音)。
pub fn mix_pass_a_args_with(
    plan: &RenderPlan,
    mixed_raw_out: &Path,
    segs_has: &[bool],
    bgm_has: bool,
) -> Vec<String> {
    if crate::across::chain_active(plan) {
        return crate::across::mix_pass_a_chain_args_with(plan, mixed_raw_out, segs_has, bgm_has);
    }
    let live_bgm = plan.bgm.as_ref().filter(|_| bgm_has);
    let audio_segs = &plan.audio_segs;
    let total_ms = plan.total_ms;
    let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];
    let mut filters: Vec<String> = Vec::new();
    if audio_segs.is_empty() && live_bgm.is_none() {
        args.extend([
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "anullsrc=r=48000:cl=stereo".into(),
        ]);
    }
    let mut input_idx = 0usize;
    let mut event_refs: Vec<String> = Vec::new();
    for (i, seg) in audio_segs.iter().enumerate() {
        let read_ms = (seg.duration_ms as f64 * seg.speed).ceil();
        if segs_has.get(i).copied().unwrap_or(true) {
            // 素材有音轨:实输入(-ss/-t 读源),[N:a] 链(N = 实输入序)
            args.extend([
                "-ss".into(),
                format!("{}", seg.source_in_ms as f64 / 1000.0),
                "-t".into(),
                format!("{}", read_ms / 1000.0),
                "-i".into(),
                seg.src.to_string_lossy().into(),
            ]);
            let chain_body = crate::across::event_body(seg);
            filters.push(format!(
                "[{input_idx}:a]{chain_body},adelay={}:all=1[a{input_idx}]",
                seg.start_ms
            ));
            event_refs.push(format!("[a{input_idx}]"));
            input_idx += 1;
        } else {
            // 素材纯视频(无音频流):静音占位(不开 -i,anullsrc 源滤镜),
            // adelay 落点与段长照旧——时间域贡献必须保留(剔除会让 amix 变短,
            // 成片音轨短于视频轨);EQ/组流按同一 refs 平行数组消费,零特判
            filters.push(format!(
                "anullsrc=r=48000:cl=stereo,atrim=0:{:.6},aformat=sample_rates=48000:channel_layouts=stereo,adelay={}:all=1[as{i}]",
                seg.duration_ms as f64 / 1000.0,
                seg.start_ms
            ));
            event_refs.push(format!("[as{i}]"));
        }
    }
    if !audio_segs.is_empty() {
        if plan.track_proc.is_empty() {
            // 既有路径:逐事件标签直接进总线 amix(全可用时参数逐字一致)
            filters.push(format!(
                "{}amix=inputs={}:duration=longest:normalize=0[bus]",
                event_refs.join(""),
                audio_segs.len()
            ));
        } else {
            // 轨道组建流(册五 T5.3):有处理声明的轨 amix 建流 → per-track 链 → 单标签
            let all_segs: Vec<&crate::plan::AudioSeg> = audio_segs.iter().collect();
            let (group_parts, labels) =
                crate::across::grouped_bus_labels(plan, &all_segs, &event_refs);
            filters.extend(group_parts);
            filters.push(format!(
                "{}amix=inputs={}:duration=longest:normalize=0[bus]",
                labels.join(""),
                labels.len()
            ));
        }
    }
    // 总线([bus] 恒存在:bgm-only 工程用 anullsrc 占位)
    if audio_segs.is_empty() && live_bgm.is_some() {
        args.extend([
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "anullsrc=r=48000:cl=stereo".into(),
        ]);
        filters.push(format!("[{input_idx}:a]anull[bus]"));
        input_idx += 1;
    }
    // BGM(循环铺满 + gain + ducking 侧链,参数化 T5.3)
    if let Some(bgm) = live_bgm {
        let bgm_path = plan.project_dir.join(&bgm.src);
        args.extend([
            "-stream_loop".into(),
            "-1".into(),
            "-t".into(),
            format!("{}", total_ms as f64 / 1000.0),
            "-i".into(),
            bgm_path.to_string_lossy().into(),
        ]);
        let bgm_idx = input_idx;
        filters.push(format!(
            "[{bgm_idx}:a]aformat=sample_rates=48000:channel_layouts=stereo,volume={:.1}dB[bgmg]",
            bgm.gain_db
        ));
        if bgm.ducking && !audio_segs.is_empty() {
            // [bus] 需被 sidechain(key)与 amix 各消费一次 → asplit 分流
            // (本地 ffmpeg 容忍重复 label,CI 严格报 Invalid stream specifier)
            filters.push("[bus]asplit=2[busA][busB]".into());
            filters.push(format!(
                "[bgmg][busA]{}[bgmc]",
                crate::across::ducking_filter(bgm)
            ));
            filters.push("[busB][bgmc]amix=inputs=2:duration=first:normalize=0[mixout]".into());
        } else {
            filters.push("[bus][bgmg]amix=inputs=2:duration=first:normalize=0[mixout]".into());
        }
        args.extend([
            "-filter_complex".into(),
            filters.join(";"),
            "-map".into(),
            "[mixout]".into(),
        ]);
    } else if !audio_segs.is_empty() {
        args.extend([
            "-filter_complex".into(),
            filters.join(";"),
            "-map".into(),
            "[bus]".into(),
        ]);
    }
    if live_bgm.is_none() && audio_segs.is_empty() {
        // anullsrc 路径:输入即静音源,无 filter_complex
    }
    args.extend([
        "-t".into(),
        format!("{}", total_ms as f64 / 1000.0),
        "-c:a".into(),
        "aac".into(),
        mixed_raw_out.to_string_lossy().into(),
    ]);
    args
}

// ---- loudnorm 测量/双 pass(实现在 across.rs,册五 T5.3 纯移动——行数红线
// A1-3;`pub use` 保持 `steps::mix_*` 路径兼容) ----
pub use crate::across::{
    mix_linear_filter, mix_linear_filter_t, mix_measure_args, mix_measure_args_t,
    mix_measured_is_loud, mix_pass_b_args, mix_pass_b_silent_args,
};

// ---------------- 步 6 subtitle(最后叠) ----------------

/// 字幕烧录命令行(cwd = 缓存根;ASS 以相对路径喂给 subtitles 滤镜,规避
/// Windows 盘符冒号在滤镜参数里的转义问题)。ass_rels = 烧录序列(册四 T4.7:
/// 外部字幕 + 文本轨生成 ASS 串联——多条 subtitles 滤镜链式应用,免解析合并
/// 外部文件;单条时参数与拆分前逐字一致,parity 红线)。
pub fn subtitle_burn_args(
    video_in: &Path,
    mixed_in: &Path,
    ass_rels: &[String],
    subbed_out: &Path,
) -> Vec<String> {
    let vf = ass_rels
        .iter()
        .map(|r| format!("subtitles={r}"))
        .collect::<Vec<_>>()
        .join(",");
    [
        "-y",
        "-v",
        "error",
        "-i",
        &video_in.to_string_lossy(),
        "-i",
        &mixed_in.to_string_lossy(),
        "-vf",
        &vf,
        "-map",
        "0:v",
        "-map",
        "1:a",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-c:a",
        "copy",
        &subbed_out.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// 无字幕合流命令行(零重编码转封装)。
pub fn subtitle_mux_args(video_in: &Path, mixed_in: &Path, subbed_out: &Path) -> Vec<String> {
    [
        "-y",
        "-v",
        "error",
        "-i",
        &video_in.to_string_lossy(),
        "-i",
        &mixed_in.to_string_lossy(),
        "-map",
        "0:v",
        "-map",
        "1:a",
        "-c:v",
        "copy",
        "-c:a",
        "copy",
        &subbed_out.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

// ---------------- 步 7 encode ----------------

/// 最终成片输出路径:06_成片输出/final_cutforge_{slug}_{W}x{H}.mp4(文件名消毒;
/// 路径与格式与拆分前一致,不缓存)。
pub fn encode_output_path(plan: &RenderPlan) -> PathBuf {
    let canvas = format!("{}x{}", plan.canvas_w, plan.canvas_h);
    plan.out_dir.join(format!(
        "final_cutforge_{}_{}.mp4",
        sanitize_slug(&plan.slug),
        canvas
    ))
}

// ---------------- 共享小工具 ----------------

/// 输出文件名消毒:仅替换文件系统敌对字符,保留 CJK/字母数字/-_
pub fn sanitize_slug(slug: &str) -> String {
    slug.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect()
}

/// atempo 链(单实例只支持 0.5–2.0;speed ∈ [0.25,4] 用链式分解)
pub fn atempo_chain(speed: f64) -> Vec<String> {
    let mut out = Vec::new();
    let mut s = speed;
    while s > 2.0 {
        out.push("atempo=2.0".into());
        s /= 2.0;
    }
    while s < 0.5 {
        out.push("atempo=0.5".into());
        s /= 0.5;
    }
    out.push(format!("atempo={s:.6}"));
    out
}

/// f64 → 紧凑字符串(去尾零;空串回落 "0")。
pub fn fmt_f64(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s.is_empty() { "0".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cutforge_core::model::Overlay;
    use serde_json::{Value, json};
    use std::path::PathBuf;

    /// 测试计划:fake 路径(纯函数不触 IO)。
    fn test_plan(clips: Value) -> (RenderPlan, Vec<Clip>) {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "demo", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": clips}]
        }))
        .unwrap();
        let plan = RenderPlan::build(&project, Path::new("/w"), None);
        (plan, project.tracks[0].clips.clone())
    }

    fn base_clips() -> Value {
        json!([
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000,
             "sourceInMs": 500, "role": "voice", "volume": 1.0},
            {"id": "V1-002", "src": "a.mp4", "startMs": 2000, "durationMs": 2000, "role": "voice"}
        ])
    }

    fn strv(v: &[String]) -> Vec<&str> {
        v.iter().map(|s| s.as_str()).collect()
    }

    /// 平台一致的路径期望值(join 的分隔符随平台变化)。
    fn pj(base: &str, rel: &str) -> String {
        Path::new(base).join(rel).to_string_lossy().into_owned()
    }

    /// 全参数拼成单串做子串断言(filter_complex 的滤镜以 ';' 连在同一参数内)。
    fn joined(args: &[String]) -> String {
        args.join("\u{1}")
    }

    #[test]
    fn tail_ms_comes_from_next_clip_transition() {
        let mut v = base_clips();
        v[1]["transition"] = json!({"type": "fade", "durMs": 500, "reason": "topic"});
        let (_, clips) = test_plan(v);
        assert_eq!(
            segment_tail_ms(&clips, 0),
            500.0,
            "段 0 的尾帧由 clip 1 的入向转场决定"
        );
        assert_eq!(segment_tail_ms(&clips, 1), 0.0, "末段无尾帧");
    }

    #[test]
    fn transition_cut_and_zero_dur_are_hard_cut() {
        let mut v = base_clips();
        v[1]["transition"] = json!({"type": "cut", "durMs": 500});
        let (_, clips) = test_plan(v.clone());
        assert_eq!(transition_out_ms(&clips[1]), None);
        v[1]["transition"] = json!({"type": "fade", "durMs": 0});
        let (_, clips) = test_plan(v);
        assert_eq!(transition_out_ms(&clips[1]), None);
    }

    // ---- 步 3:compose ----

    #[test]
    fn compose_concat_args_when_no_transitions() {
        let (_, clips) = test_plan(base_clips());
        assert!(!is_xfade_chain(&clips));
        let segs = vec![PathBuf::from("/c/seg/1.mp4"), PathBuf::from("/c/seg/2.mp4")];
        let list = concat_list_content(&segs);
        assert_eq!(list, "file '/c/seg/1.mp4'\nfile '/c/seg/2.mp4'\n");
        let args = compose_concat_args(Path::new("/c/tmp/c.txt"), Path::new("/c/compose/o.mp4"));
        assert_eq!(
            strv(&args),
            [
                "-y",
                "-v",
                "error",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
                "/c/tmp/c.txt",
                "-c",
                "copy",
                "/c/compose/o.mp4"
            ]
        );
    }

    /// I1 修:xfade 链只在「全边界相接 + 全边界有转场」时启用——任一边界
    /// 空隙(不相接)或无有效转场(d=0,该边界 duration=0 的 xfade 会被
    /// ffmpeg 丢段)都整链退化 concat。合法全转场链不受影响。
    #[test]
    fn xfade_chain_requires_connected_transitional_boundaries() {
        // 相接 + 转场 → 链
        let full = json!([
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000},
            {"id": "V1-002", "src": "a.mp4", "startMs": 2000, "durationMs": 2000,
             "transition": {"type": "fade", "durMs": 500}}
        ]);
        let (_, clips) = test_plan(full);
        assert!(is_xfade_chain(&clips));
        // 空隙边界(前段末端 ≠ 后段起点)+ 转场 → 退化(concat 折叠空隙为末帧定格)
        let gapped = json!([
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000},
            {"id": "V1-002", "src": "a.mp4", "startMs": 3000, "durationMs": 2000,
             "transition": {"type": "fade", "durMs": 500}}
        ]);
        let (_, clips) = test_plan(gapped);
        assert!(
            !is_xfade_chain(&clips),
            "空隙边界不得进 xfade 链(offset 对齐失配丢段)"
        );
        // 相接但边界无转场(d=0)→ 退化(duration=0 的 xfade 丢第二输入)
        let partial = json!([
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000},
            {"id": "V1-002", "src": "a.mp4", "startMs": 2000, "durationMs": 2000},
            {"id": "V1-003", "src": "a.mp4", "startMs": 4000, "durationMs": 2000,
             "transition": {"type": "fade", "durMs": 500}}
        ]);
        let (_, clips) = test_plan(partial);
        assert!(!is_xfade_chain(&clips), "d=0 边界(1→2)不得进 xfade 链");
        // 单段/硬切照旧
        let (_, single) = test_plan(json!([
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000}
        ]));
        assert!(!is_xfade_chain(&single));
        let (_, cut) = test_plan(json!([
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000},
            {"id": "V1-002", "src": "a.mp4", "startMs": 2000, "durationMs": 2000,
             "transition": {"type": "cut", "durMs": 500}}
        ]));
        assert!(!is_xfade_chain(&cut), "显式硬切 = d=0,退化 concat");
    }

    #[test]
    fn compose_xfade_offsets_accumulate_segment_durations() {
        let mut v = base_clips();
        v[1]["transition"] = json!({"type": "fade", "durMs": 500, "reason": "topic"});
        let (_, clips) = test_plan(v);
        assert!(is_xfade_chain(&clips));
        let segs = vec![PathBuf::from("/c/seg/1.mp4"), PathBuf::from("/c/seg/2.mp4")];
        let (args, warns) = compose_xfade_args(&clips, &segs, Path::new("/c/compose/o.mp4"));
        assert!(warns.is_empty());
        let s = strv(&args);
        assert_eq!(
            s[8], "[0:v][1:v]xfade=transition=fade:duration=0.500000:offset=2.000000[x1]",
            "offset = 前序**名义**时长累计(ADR-0023;尾帧只进重叠不前移 offset)"
        );
        assert_eq!(
            &s[..7],
            [
                "-y",
                "-v",
                "error",
                "-i",
                "/c/seg/1.mp4",
                "-i",
                "/c/seg/2.mp4"
            ]
        );
        assert_eq!(
            &s[9..],
            [
                "-map",
                "[x1]",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "/c/compose/o.mp4"
            ]
        );
    }

    /// 册四 BE3a 零漂移红线:两段各 2s + 500ms 转场,xfade 链总输出 = Σdur = 4.0s
    /// (offset=名义时长口径;旧实现 offset 含尾帧会把后段截没——夹具以**视频流**
    /// 帧数锁定,容器时长会被音频 mux 撑大而说谎)。
    #[test]
    fn compose_xfade_two_boundary_chain_offsets_use_nominal_durations() {
        let mut v = base_clips();
        v[1]["durationMs"] = json!(1500);
        v[1]["startMs"] = json!(2000);
        v[1]["transition"] = json!({"type": "fade", "durMs": 500});
        v.as_array_mut().unwrap().push(
            json!({"id": "V1-003", "src": "a.mp4", "startMs": 3500, "durationMs": 1000,
             "role": "voice", "transition": {"type": "fade", "durMs": 400}}),
        );
        let (_, clips) = test_plan(v);
        let segs: Vec<PathBuf> = (1..=3)
            .map(|i| PathBuf::from(format!("/c/seg/{i}.mp4")))
            .collect();
        let (args, warns) = compose_xfade_args(&clips, &segs, Path::new("/c/o.mp4"));
        assert!(warns.is_empty());
        let s = args.join("\u{1}");
        assert!(
            s.contains("xfade=transition=fade:duration=0.500000:offset=2.000000[x1]"),
            "{s}"
        );
        assert!(
            s.contains("[x1][2:v]xfade=transition=fade:duration=0.400000:offset=3.500000[x2]"),
            "{s}"
        );
    }

    /// 目录直通:fx=tr.circleclose 覆写 type;未注册 id 进 WARN 列表(降级 fade)。
    #[test]
    fn compose_xfade_resolves_catalog_ids_with_warnings() {
        let mut v = base_clips();
        v[1]["transition"] = json!({"type": "fade", "fx": "tr.circleclose", "durMs": 500});
        let (_, clips) = test_plan(v.clone());
        let segs = vec![PathBuf::from("/c/seg/1.mp4"), PathBuf::from("/c/seg/2.mp4")];
        let (args, warns) = compose_xfade_args(&clips, &segs, Path::new("/c/o.mp4"));
        assert!(args.join("\u{1}").contains("transition=circleclose"));
        assert!(warns.iter().any(|w| w.contains("以 fx 为准")), "{warns:?}");
        v[1]["transition"] = json!({"type": "fade", "fx": "tr.幽灵", "durMs": 500});
        let (_, clips) = test_plan(v);
        let (args, warns) = compose_xfade_args(&clips, &segs, Path::new("/c/o.mp4"));
        assert!(
            args.join("\u{1}").contains("transition=fade"),
            "未注册降级 fade"
        );
        assert!(warns.iter().any(|w| w.contains("未注册")), "{warns:?}");
    }

    // ---- 步 4:overlay ----

    #[test]
    fn overlay_args_chain_scale_window_and_opacity() {
        let (plan, _) = test_plan(base_clips());
        let ovs = vec![OverlaySeg {
            src: PathBuf::from("/w/logo.png"),
            start_ms: 0,
            duration_ms: 2000,
            spec: Overlay {
                x: 40,
                y: 40,
                w: 60,
                h: 60,
                opacity: 0.5,
            },
        }];
        let args = overlay_args(
            &ovs,
            &plan.cache_dir.join("c.mp4"),
            Path::new("/c/overlay/o.mp4"),
        );
        let s = strv(&args);
        assert_eq!(
            &s[..7],
            [
                "-y",
                "-v",
                "error",
                "-i",
                &pj(&plan.cache_dir.to_string_lossy(), "c.mp4"),
                "-i",
                "/w/logo.png"
            ]
        );
        assert_eq!(
            s[8],
            "[1:v]scale=60:60,format=rgba,colorchannelmixer=aa=0.5000[l0];\
[0:v][l0]overlay=x=40:y=40:enable='between(t,0.000,2.000)'[o0]"
        );
        assert_eq!(
            &s[9..],
            [
                "-map",
                "[o0]",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "/c/overlay/o.mp4"
            ]
        );
    }

    #[test]
    fn overlay_opacity_one_skips_alpha_chain() {
        let ovs = vec![OverlaySeg {
            src: PathBuf::from("/w/logo.png"),
            start_ms: 0,
            duration_ms: 1000,
            spec: Overlay {
                x: 1,
                y: 2,
                w: 3,
                h: 4,
                opacity: 1.0,
            },
        }];
        let args = overlay_args(&ovs, Path::new("/c.mp4"), Path::new("/o.mp4"));
        assert!(
            args[8].starts_with("[1:v]scale=3:4[l0];"),
            "opacity=1 不引入 rgba 链;层输入 = 1(基片恒 0)"
        );
    }

    // ---- 步 5:mix ----

    #[test]
    fn mix_pass_a_voice_segment_chain_has_crop_fades_and_delay() {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "m", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [
                {"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000,
                     "sourceInMs": 0, "role": "voice", "volume": 1.0, "speed": 1.5,
                     "fade": {"inMs": 800, "outMs": 400}}
                ]},
                {"id": "A1", "kind": "audio", "clips": []}
            ]
        }))
        .unwrap();
        let plan = RenderPlan::build(&project, Path::new("/w"), None);
        let args = mix_pass_a_args(&plan, Path::new("/c/mix/r.m4a"));
        // read_ms = 2000×1.5 = 3000 → 输入侧 -ss 0 / -t 3
        assert_eq!(&strv(&args)[3..8], ["-ss", "0", "-t", "3", "-i"]);
        assert_eq!(args[8], pj("/w", "a.mp4"));
        let j = joined(&args);
        assert!(
            j.contains(
                "[0:a]aformat=sample_rates=48000:channel_layouts=stereo,atempo=1.500000,volume=1.0000,\
afade=t=in:st=0:d=0.800,afade=t=out:st=1.600:d=0.400,adelay=0:all=1[a0]"
            ),
            "单段链(变速/音量/淡入淡出/落点)不符: {j}"
        );
        assert!(j.contains("amix=inputs=1:duration=longest:normalize=0[bus]"));
        assert!(j.contains("-map"));
        assert!(j.contains("[bus]"));
        assert_eq!(args.last().unwrap(), "/c/mix/r.m4a");
    }

    #[test]
    fn mix_pass_a_bgm_only_uses_anullsrc_bus_and_ducking_splits_bus() {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "m", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "bgm": {"src": "bgm.mp3", "gainDb": -6, "ducking": true, "loop": true},
            "tracks": [
                {"id": "V1", "kind": "video", "clips": []},
                {"id": "A1", "kind": "audio", "clips": []}
            ]
        }))
        .unwrap();
        let plan = RenderPlan::build(&project, Path::new("/w"), None);
        let args = mix_pass_a_args(&plan, Path::new("/c/mix/r.m4a"));
        let j = joined(&args);
        assert!(j.contains("anullsrc=r=48000:cl=stereo"));
        assert!(j.contains("[0:a]anull[bus]"));
        assert!(j.contains("volume=-6.0dB[bgmg]"));
        // bgm-only:ducking 分支需要人声,缺省走直混
        assert!(j.contains("[bus][bgmg]amix=inputs=2:duration=first:normalize=0[mixout]"));
        assert!(j.contains("-stream_loop"));
        assert!(!j.contains("sidechaincompress"), "无人声时无侧链");
    }

    #[test]
    fn mix_pass_a_ducking_with_voice_splits_bus() {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "m", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "bgm": {"src": "bgm.mp3", "gainDb": -6, "ducking": true, "loop": true},
            "tracks": [
                {"id": "V1", "kind": "video", "clips": []},
                {"id": "A1", "kind": "audio", "clips": [
                    {"id": "A1-001", "src": "v.mp3", "startMs": 0, "durationMs": 2000, "volume": 1.0}
                ]}
            ]
        }))
        .unwrap();
        let plan = RenderPlan::build(&project, Path::new("/w"), None);
        let args = mix_pass_a_args(&plan, Path::new("/c/mix/r.m4a"));
        let j = joined(&args);
        assert!(j.contains("[bus]asplit=2[busA][busB]"));
        assert!(j.contains(
            "[bgmg][busA]sidechaincompress=threshold=0.03:ratio=8:attack=80:release=500[bgmc]"
        ));
        assert!(j.contains("[busB][bgmc]amix=inputs=2:duration=first:normalize=0[mixout]"));
    }

    // ---- 音轨可用性(I1 缺口修:纯视频素材的段无音频流,[N:a] 零匹配炸图) ----

    fn mix_two_voice_plan() -> RenderPlan {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "m", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000, "role": "voice"},
                {"id": "V1-002", "src": "b.mp4", "startMs": 2000, "durationMs": 2000, "role": "voice"}
            ]}]
        }))
        .unwrap();
        RenderPlan::build(&project, Path::new("/w"), None)
    }

    #[test]
    fn mix_placeholders_events_whose_source_has_no_audio_stream() {
        let plan = mix_two_voice_plan();
        // 段 0 素材有音轨、段 1 素材纯视频 → 只开 a.mp4 一个输入;段 1 用静音占位
        // (adelay 落点/段长照旧,amix 输入数恒 = 段数——占位撑住时间域)
        let args = mix_pass_a_args_with(&plan, Path::new("/c/mix/r.m4a"), &[true, false], true);
        let j = joined(&args);
        assert_eq!(
            j.matches("\u{1}-i\u{1}").count(),
            1,
            "无音轨段不开输入: {j}"
        );
        assert!(
            j.contains("a.mp4") && !j.contains("b.mp4"),
            "无音轨素材不得进命令行: {j}"
        );
        assert!(j.contains("[0:a]"), "有音轨段用实输入序 0: {j}");
        assert!(!j.contains("[1:a]"), "不得出现零匹配流说明符: {j}");
        assert!(
            j.contains(
                "anullsrc=r=48000:cl=stereo,atrim=0:2.000000,aformat=sample_rates=48000:channel_layouts=stereo,adelay=2000:all=1[as1]"
            ),
            "无音轨段 = 静音占位(落点/段长照旧): {j}"
        );
        assert!(
            j.contains("[a0][as1]amix=inputs=2:duration=longest:normalize=0[bus]"),
            "{j}"
        );
        // 对照:全可用 = 既有图(两输入两链,与包装版逐字一致)
        let all = mix_pass_a_args(&plan, Path::new("/c/mix/r.m4a"));
        let ja = joined(&all);
        assert!(ja.contains("[0:a]") && ja.contains("[1:a]"));
        assert!(ja.contains("amix=inputs=2:duration=longest:normalize=0[bus]"));
    }

    #[test]
    fn mix_all_sourceless_graph_stays_silent_with_full_timespan() {
        let plan = mix_two_voice_plan();
        // 全部段素材无音轨 → 图照常(全占位),amix 输入数确定,产物等价静音
        let args = mix_pass_a_args_with(&plan, Path::new("/c/mix/r.m4a"), &[false, false], false);
        let j = joined(&args);
        assert!(
            j.contains("filter_complex") && j.contains("amix=inputs=2"),
            "全占位图照常: {j}"
        );
        assert_eq!(j.matches("\u{1}-i\u{1}").count(), 0, "零实输入: {j}");
        assert!(
            j.contains("[as0][as1]amix=inputs=2:duration=longest:normalize=0[bus]"),
            "{j}"
        );
        // 对照:有 BGM 工程在 bgm_has=true 时走既有 [mixout] 形态(ducking 需要
        // bus,占位图照常提供)
        let bgm_plan: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "m", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "bgm": {"src": "bgm.mp3", "gainDb": -6, "ducking": true, "loop": true},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000, "role": "voice"},
                {"id": "V1-002", "src": "b.mp4", "startMs": 2000, "durationMs": 2000, "role": "voice"}
            ]}]
        }))
        .unwrap();
        let with_bgm = mix_pass_a_args_with(
            &RenderPlan::build(&bgm_plan, Path::new("/w"), None),
            Path::new("/c/mix/r.m4a"),
            &[false, false],
            true,
        );
        let jw = joined(&with_bgm);
        assert!(
            jw.contains("bgm.mp3") && jw.contains("[mixout]"),
            "全占位 + 可用 BGM: {jw}"
        );
    }

    #[test]
    fn mix_bgm_without_source_audio_is_dropped() {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "m", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "bgm": {"src": "bgm.mp3", "gainDb": -6, "ducking": true, "loop": true},
            "tracks": [{"id": "A1", "kind": "audio", "clips": [
                {"id": "A1-001", "src": "v.mp3", "startMs": 0, "durationMs": 2000, "volume": 1.0}
            ]}]
        }))
        .unwrap();
        let plan = RenderPlan::build(&project, Path::new("/w"), None);
        // BGM 素材无音轨 → 按无 BGM 处理(静音背景乐 = 无);事件段照常
        let args = mix_pass_a_args_with(&plan, Path::new("/c/mix/r.m4a"), &[true], false);
        let j = joined(&args);
        assert!(!j.contains("bgm.mp3"), "无音轨 BGM 不得进命令行: {j}");
        assert!(
            !j.contains("sidechaincompress"),
            "无 BGM 即无 ducking 侧链: {j}"
        );
        assert!(j.contains("-map\u{1}[bus]"), "{j}");
        // 对照:bgm 有音轨 = 既有形态
        let with = mix_pass_a_args_with(&plan, Path::new("/c/mix/r.m4a"), &[true], true);
        assert!(joined(&with).contains("bgm.mp3"));
    }

    #[test]
    fn mix_silent_project_is_anullsrc_passthrough() {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "m", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": []}]
        }))
        .unwrap();
        let plan = RenderPlan::build(&project, Path::new("/w"), None);
        let args = mix_pass_a_args(&plan, Path::new("/c/mix/r.m4a"));
        assert!(
            !args
                .iter()
                .any(|a| a.contains("filter_complex") || a.contains("amix")),
            "无 filter_complex"
        );
        assert!(args.iter().any(|a| a.contains("anullsrc")));
    }

    #[test]
    fn atempo_chain_decomposes_extreme_speeds() {
        assert_eq!(atempo_chain(1.0), vec!["atempo=1.000000"]);
        assert_eq!(atempo_chain(4.0), vec!["atempo=2.0", "atempo=2.000000"]);
        assert_eq!(atempo_chain(0.25), vec!["atempo=0.5", "atempo=0.500000"]);
        assert_eq!(atempo_chain(3.0), vec!["atempo=2.0", "atempo=1.500000"]);
    }

    // ---- 步 6/7:subtitle + encode ----

    #[test]
    fn subtitle_burn_and_mux_args_are_backward_compatible() {
        let burn = subtitle_burn_args(
            Path::new("/c/sub/v.mp4"),
            Path::new("/c/mix/m.m4a"),
            &["tmp/ass-k.ass".into()],
            Path::new("/c/sub/o.mp4"),
        );
        assert_eq!(
            strv(&burn),
            [
                "-y",
                "-v",
                "error",
                "-i",
                "/c/sub/v.mp4",
                "-i",
                "/c/mix/m.m4a",
                "-vf",
                "subtitles=tmp/ass-k.ass",
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-c:a",
                "copy",
                "/c/sub/o.mp4"
            ]
        );
        // 册四 T4.7:外部字幕 + 文本轨生成 ASS 串联(链式 subtitles 滤镜)
        let both = subtitle_burn_args(
            Path::new("/c/sub/v.mp4"),
            Path::new("/c/mix/m.m4a"),
            &["tmp/ass-user.ass".into(), "tmp/ass-text.ass".into()],
            Path::new("/c/sub/o.mp4"),
        );
        assert_eq!(
            both[8], "subtitles=tmp/ass-user.ass,subtitles=tmp/ass-text.ass",
            "{:?}",
            both[8]
        );
        let mux = subtitle_mux_args(
            Path::new("/c/sub/v.mp4"),
            Path::new("/c/mix/m.m4a"),
            Path::new("/c/sub/o.mp4"),
        );
        assert_eq!(
            strv(&mux),
            [
                "-y",
                "-v",
                "error",
                "-i",
                "/c/sub/v.mp4",
                "-i",
                "/c/mix/m.m4a",
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-c:v",
                "copy",
                "-c:a",
                "copy",
                "/c/sub/o.mp4"
            ]
        );
    }

    #[test]
    fn encode_output_path_sanitizes_hostile_slug() {
        let (plan, _) = test_plan(base_clips());
        assert_eq!(
            encode_output_path(&plan),
            Path::new("/w")
                .join("06_成片输出")
                .join("final_cutforge_demo_1080x1920.mp4")
        );
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "bad*slug:<>", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": []}]
        }))
        .unwrap();
        let plan = RenderPlan::build(&project, Path::new("/w"), None);
        assert_eq!(
            encode_output_path(&plan)
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "final_cutforge_bad_slug____1080x1920.mp4"
        );
    }

    #[test]
    fn fmt_f64_trims_trailing_zeros() {
        assert_eq!(fmt_f64(2.0), "2");
        assert_eq!(fmt_f64(1.5), "1.5");
        assert_eq!(fmt_f64(0.0), "0");
    }

    /// overlay 输入索引锁(A5-BE3 修复):层输入恒 i+1(基片输入 0);
    /// 旧实现 [i:v] 把基片自身当叠加源(红底红 logo 不可见的潜伏缺陷)。
    #[test]
    fn overlay_args_layer_inputs_are_offset_by_one() {
        let mk = |i: usize| OverlaySeg {
            src: PathBuf::from(format!("logo{i}.png")),
            start_ms: (i * 1000) as u64,
            duration_ms: 500,
            spec: cutforge_core::model::Overlay {
                x: 4,
                y: 5,
                w: 60,
                h: 60,
                opacity: 1.0,
            },
        };
        let args = overlay_args(&[mk(0), mk(1)], Path::new("base.mp4"), Path::new("out.mp4"));
        let fc = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(
            fc.contains("[1:v]scale=60:60[l0];"),
            "第一层必须吃输入 1: {fc}"
        );
        assert!(
            fc.contains("[2:v]scale=60:60[l1];"),
            "第二层必须吃输入 2: {fc}"
        );
        assert!(fc.contains("[o0][l1]overlay"), "层链串联: {fc}");
        assert!(fc.contains(";[0:v][l0]overlay="), "基片恒输入 0: {fc}");
    }

    /// 册五 T5.4:调整层时间窗命令行——trim 抽窗/setpts 归零/链作用/overlay enable
    /// 贴回四段式;grade 前置 fx(调色喂特效,与段链序一致);空链片段跳过。
    #[test]
    fn adjust_args_window_pipeline_shape() {
        let mk = |v: serde_json::Value| -> Clip { serde_json::from_value(v).unwrap() };
        let clips = vec![
            mk(json!({"id": "X1-001", "startMs": 500, "durationMs": 1000,
                      "fx": {"combo": [{"fx": "fx.blur"}]}})),
            mk(json!({"id": "X1-002", "startMs": 2000, "durationMs": 500,
                      "grade": {"saturation": 1.5}})),
            mk(json!({"id": "X1-003", "startMs": 3000, "durationMs": 500})),
        ];
        let args = adjust_args(
            &clips,
            Path::new("base.mp4"),
            Path::new("out.mp4"),
            Path::new("/w"),
            320,
            240,
            30,
        );
        let fc = args
            .iter()
            .find(|a| a == &&"-filter_complex".to_string())
            .map(|_| ())
            .is_some()
            .then(|| args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1].clone())
            .unwrap();
        assert!(
            fc.contains("trim=start=0.500:end=1.500,setpts=PTS-STARTPTS"),
            "{fc}"
        );
        assert!(
            fc.contains("overlay=enable='between(t,0.500,1.500)'"),
            "{fc}"
        );
        assert!(fc.contains("trim=start=2.000:end=2.500"), "{fc}");
        // 饱和度链(eq)在窗内流上;X1-003 无链不产窗
        assert!(fc.contains("eq="), "grade 链必须在窗内: {fc}");
        assert!(!fc.contains("trim=start=3.000"), "空链片段不产窗: {fc}");
        // 全空链 → 透传拷贝
        let empty = vec![mk(json!({"id": "X1-001", "startMs": 0, "durationMs": 500}))];
        let args = adjust_args(
            &empty,
            Path::new("base.mp4"),
            Path::new("out.mp4"),
            Path::new("/w"),
            320,
            240,
            30,
        );
        assert!(
            args.contains(&"-c".to_string()) && args.contains(&"copy".to_string()),
            "全空链透传: {args:?}"
        );
    }
}
