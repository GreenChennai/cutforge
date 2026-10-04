// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 段提取纯函数(步 2 segment;册四 A4-BE2 自 steps.rs 纯移动成模块——行数红线
//! A1-3,接口经 steps.rs `pub use` 保持 `steps::segment_*` 路径兼容)。
//!
//! 段剪辑链的**叠加顺序**(册四 T4.4/T4.9 + BE3a T4.5/T4.6 定案,链图;册五 T5.2 增 grade):
//!
//! ```text
//! [源] → crop(源像素域裁剪) → flip(h/v) → rotate(±任意角;±90=transpose 精确互换)
//!      → scale+pad+fps(画幅归一) → punchIn(中心紧构图) → grade(一级/二级调色,LUT 收尾)
//!      → fx.combo(整段特效栈)
//!      → reverse(倒放,PTS 重盖) → 变速(单段 setpts / 曲线多段 trim·setpts·fps·concat)
//!      → motion(入场/出场动画,播放域) → tpad(定格补长+转场尾帧)
//! ```
//!
//! fx 在变换后/变速前(空间域特效与倒放/变速可交换);grade 挂 fx 前(调色喂给
//! 特效,链内序与定档理由见 grade 模块注释);motion 在变速后/tpad 前
//! (入场/出场时窗按播放域计;定格克隆发生在动画完成之后)。
//!
//! 本文件**不启动任何进程**:全部函数只做输入 → ffmpeg 参数的映射,可在不装
//! ffmpeg 的环境单测断言;无新字段时参数串与拆分前的 render() 逐字一致
//! (渲染输出逐字节语义不变的底线,由 parity_matrix 实渲夹具锁定)。

use crate::plan::RenderPlan;
use cutforge_core::model::{Clip, speed_segments};
use std::path::Path;

/// 播放域恒速段(册四 T4.4):freezeMs 定格截断后的 speed_segments——
/// 定格点之后的曲线段不读源;与 plan::audio_segs_of 的音频切分同一裁剪口径
/// (投影/视频/音频三方时长一致性的公共前提)。
pub fn play_segments(clip: &Clip) -> Vec<(u64, u64, f64)> {
    let play_end = crate::plan::clip_play_ms(clip);
    speed_segments(clip)
        .into_iter()
        .filter(|(a, _, _)| *a < play_end)
        .map(|(a, b, s)| (a, b.min(play_end), s))
        .collect()
}

/// 段提取的源域读取时长(ms)= ∫speed dt 在播放域的分段积分(调用方 ceil)。
/// 无曲线无定格 = durationMs × speed,与拆分前的既有语义逐位一致。
pub fn segment_read_ms(clip: &Clip) -> f64 {
    play_segments(clip)
        .iter()
        .map(|(a, b, s)| (*b - *a) as f64 * s)
        .sum()
}

/// 段尾 tpad 总毫秒 = 定格补长(durationMs − 播放域末点)+ 转场尾帧扩展。
pub fn segment_pad_ms(clip: &Clip, tail_ms: f64) -> f64 {
    let play_end = crate::plan::clip_play_ms(clip);
    (clip.duration_ms - play_end) as f64 + tail_ms
}

/// 源窗钳制(I1 渲染缺口修):素材实际可用时长 avail 已知、且源窗(sourceIn 起
/// 按播放域分段积分)超出可用时,把播放域截到源耗尽点并返回「定格补长版」片段
/// 克隆(仅前移 freezeMs;曲线锚点原样——play_end 截断即精确钳制),缺额交给
/// 既有 tpad 末帧定格,与时间线空隙钳前段末帧同哲学。不钳则越界 -ss/-t 会让段
/// 比标称短,下游 concat/抽帧越过合成 EOF → rc=0 空产出(INTERNAL 的根)。
/// 源充足(含 1ms 容差——容器时长的帧取整亚毫秒差不得触发)→ None:键与参数
/// 逐字节不变(parity 红线)。钳制写进片段克隆 → seg 键自动分叉(素材补长后
/// 键变回,陈旧定格零复用)。
pub fn source_clamped_clip(clip: &Clip, avail_ms: u64) -> Option<Clip> {
    const TOLERANCE_MS: f64 = 1.0;
    let budget = avail_ms.saturating_sub(clip.source_in_ms.unwrap_or(0)) as f64;
    if segment_read_ms(clip) - budget <= TOLERANCE_MS {
        return None; // 源充足:参数与键零变化
    }
    let mut used = 0f64;
    let mut star = crate::plan::clip_play_ms(clip) as f64; // 兜底(理论上不可达)
    for (a, b, s) in play_segments(clip) {
        let need = (b - a) as f64 * s;
        if used + need <= budget {
            used += need;
            continue;
        }
        star = a as f64 + (budget - used).max(0.0) / s;
        break;
    }
    let t = star.floor() as u64;
    if t >= crate::plan::clip_play_ms(clip) {
        return None; // 容差兜底:截断点不在播放域内则不钳
    }
    let mut c = clip.clone();
    c.freeze_ms = Some(t);
    Some(c)
}

/// 源域变换前链(册四 A4 T4.9)。**叠加顺序**(全链图见模块注释):
/// 裁剪(crop,源像素域)→ 翻转(hflip/vflip)→ 旋转。
/// 旋转角度 mod 360 归一;±90 走 transpose(精确宽高互换,无插值无出界);
/// 180 用 hflip,vflip 组合;其余任意角度 rotate 画布内旋转——出界部分裁切、
/// 露出的底为黑色(c=black),随后经画幅归一 scale/pad 收编进画布。
pub fn transform_pre_chain(clip: &Clip) -> String {
    let mut filters: Vec<String> = Vec::new();
    if let Some(cr) = &clip.crop {
        filters.push(format!("crop=w={}:h={}:x={}:y={}", cr.w, cr.h, cr.x, cr.y));
    }
    match clip.flip.as_deref() {
        Some("h") => filters.push("hflip".into()),
        Some("v") => filters.push("vflip".into()),
        _ => {}
    }
    if let Some(kf_rot) = crate::kf_expr::kf_rotate_filter(clip) {
        // rotation 关键帧(IR v3):keyframed 逐帧覆盖静态值——表达式 rotate 直接
        // 替换静态档(±90 transpose 精确互换只属静态;连续角无此形态)
        filters.push(kf_rot);
    } else if let Some(deg) = clip.rotation {
        let e = deg.rem_euclid(360.0);
        if (e - 90.0).abs() < 1e-9 {
            filters.push("transpose=1".into());
        } else if (e - 270.0).abs() < 1e-9 {
            filters.push("transpose=2".into());
        } else if (e - 180.0).abs() < 1e-9 {
            filters.push("hflip,vflip".into());
        } else if e.abs() > 1e-9 {
            filters.push(format!("rotate={:.6}:c=black", e.to_radians()));
        }
    }
    filters.join(",")
}

/// 倒放链(册四 T4.4):`reverse` 滤镜把**整段读入内存**再倒序输出——
/// 内存 ≈ 段长 × 帧大小,长片段(长时序/高分辨率)有耗尽内存的风险
/// (schema reverse 字段描述与本注释同口径:**长素材先切短再倒**)。
/// reverse 输出 PTS 逆序,必须紧跟 `setpts=N/FRAME_RATE/TB` 重盖单调 CFR
/// 时间戳(fps 归一之后执行,帧率元数据可信),后续 setpts/trim/concat 才成立。
pub fn reverse_chain(clip: &Clip) -> &'static str {
    if clip.reverse.unwrap_or(false) {
        ",reverse,setpts=N/FRAME_RATE/TB"
    } else {
        ""
    }
}

/// 画幅归一基链(段提取与曲线分段图共用,参数逐字一致)。
fn base_filters(plan: &RenderPlan) -> String {
    let (w, h, fps) = (plan.canvas_w, plan.canvas_h, plan.fps);
    format!(
        "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,fps={fps}"
    )
}

/// punch-in 链(ADR v0.11 R3):中心裁剪 factor 倍后放大回画布(静态)。
fn punch_chain(plan: &RenderPlan, clip: &Clip) -> String {
    match &clip.punch_in {
        Some(p) => {
            let f = p.factor.clamp(1.0, 2.0);
            let w = plan.canvas_w;
            format!(
                ",crop=w=iw/{f}:h=ih/{f}:x=(iw-ow)/2:y=(ih-oh)/2,scale={w}:{}",
                plan.canvas_h
            )
        }
        None => String::new(),
    }
}

/// 段剪辑链(简单形态:速度为单恒速段,即无曲线或单点曲线):
/// 画幅归一(scale+pad+fps)→ punch-in → grade(调色,T5.2)→ fx.combo → reverse
/// → 变速 setpts → motion → 尾帧/定格 tpad。
/// 无新字段时输出与拆分前的既有链**逐字一致**(parity 夹具锁定的兼容红线)。
pub fn segment_filter(plan: &RenderPlan, clip: &Clip, tail_ms: f64) -> String {
    let fps = plan.fps;
    let mut vf = String::new();
    let pre = transform_pre_chain(clip);
    if !pre.is_empty() {
        vf.push_str(&pre);
        vf.push(',');
    }
    vf.push_str(&base_filters(plan));
    vf.push_str(&punch_chain(plan, clip));
    // grade 调色(册五 T5.2):变换后、fx 前(链图定档见 grade 模块注释)
    let (grade, _) = crate::grade::grade_chain(clip, &plan.project_dir);
    if !grade.is_empty() {
        vf.push(',');
        vf.push_str(&grade);
    }
    // fx.combo 整段特效栈(册四 T4.6):变换后/变速前(空间域,与 reverse 可交换)
    let (fx, _) = crate::catalog::fx_chain(clip, plan.canvas_w, plan.canvas_h, plan.fps);
    if !fx.is_empty() {
        vf.push(',');
        vf.push_str(&fx);
    }
    vf.push_str(reverse_chain(clip));
    let speed = play_segments(clip)
        .last()
        .map(|(_, _, s)| *s)
        .unwrap_or(1.0);
    if speed != 1.0 {
        // setpts 后重锁帧率:不同 ffmpeg 版本 -t 帧取整差异会导致时长漂移
        vf.push_str(&format!(
            ",setpts=PTS/{},fps={}",
            crate::steps::fmt_f64(speed),
            fps
        ));
    }
    // motion 入场/出场动画(册四 T4.6):变速后(播放域)、tpad 前
    let (mo_in, mo_out, _) =
        crate::catalog::motion_chains(clip, plan.canvas_w, plan.canvas_h, plan.fps);
    for chain in [mo_in, mo_out] {
        if !chain.is_empty() {
            vf.push(',');
            vf.push_str(&chain);
        }
    }
    let pad_ms = segment_pad_ms(clip, tail_ms);
    if pad_ms > 0.0 {
        // 定格补长(freezeMs)+ 转场尾帧扩展,同一个 tpad 克隆尾帧(ADR-0023/册四 T4.4)
        vf.push_str(&format!(
            ",tpad=stop_mode=clone:stop_duration={:.6}",
            pad_ms / 1000.0
        ));
    }
    vf
}

/// 曲线分段图(册四 T4.4):速度曲线为多恒速段时,把归一后的流 split 成 n 路,
/// 各路按**源域累计偏移** trim、按段均值速度 setpts(重基 + 变速一步完成)、
/// 重锁 fps,再 concat 回单流,最后 tpad 补定格/尾帧。总输出时长 = Σ段长 =
/// durationMs(投影端 endMs 与渲染端时长一致性的实现本体)。
pub fn segment_filter_complex(plan: &RenderPlan, clip: &Clip, tail_ms: f64) -> Option<String> {
    // 关键帧段图(IR v3,T5.1):任一视觉类关键帧(position/scale/rotation/opacity/
    // fx 参数)存在即走专用段图构建(kf_expr 单源编译;rotation 虽可单滤镜表达,
    // 统一入图免双路径漂移)。无关键帧 = 既有路径逐字不变(parity 红线)。
    if crate::kf_expr::has_visual_keyframes(clip) {
        return segment_filter_complex_kf(plan, clip, tail_ms);
    }
    let segs = play_segments(clip);
    if segs.len() <= 1 {
        return None;
    }
    let fps = plan.fps;
    let mut head = String::new();
    let pre = transform_pre_chain(clip);
    if !pre.is_empty() {
        head.push_str(&pre);
        head.push(',');
    }
    head.push_str(&base_filters(plan));
    head.push_str(&punch_chain(plan, clip));
    // grade 调色(册五 T5.2):归一后、split 前(作用于整段源流)
    let (grade, _) = crate::grade::grade_chain(clip, &plan.project_dir);
    if !grade.is_empty() {
        head.push(',');
        head.push_str(&grade);
    }
    // fx.combo(册四 T4.6):归一后、split 前(作用于整段源流)
    let (fx, _) = crate::catalog::fx_chain(clip, plan.canvas_w, plan.canvas_h, plan.fps);
    if !fx.is_empty() {
        head.push(',');
        head.push_str(&fx);
    }
    head.push_str(reverse_chain(clip).trim_start_matches(','));
    let n = segs.len();
    let splits: Vec<String> = (0..n).map(|i| format!("[s{i}]")).collect();
    let graph_head = format!("[0:v]{head},split={n}{}", splits.join(""));
    let mut parts = vec![graph_head];
    let mut labels: Vec<String> = Vec::with_capacity(n);
    let mut x = 0f64;
    for (i, (a, b, s)) in segs.iter().enumerate() {
        let x0 = x / 1000.0;
        x += (*b - *a) as f64 * s;
        let x1 = x / 1000.0;
        labels.push(format!("[c{i}]"));
        parts.push(format!(
            "[s{i}]trim=start={x0:.6}:end={x1:.6},setpts=(PTS-STARTPTS)/{},fps={fps}[c{i}]",
            crate::steps::fmt_f64(*s)
        ));
    }
    let pad_ms = segment_pad_ms(clip, tail_ms);
    let concat = format!("{}concat=n={n}:v=1:a=0[vcat]", labels.join(""));
    // motion(册四 T4.6):concat 后(播放域)、tpad 前
    let (mo_in, mo_out, _) =
        crate::catalog::motion_chains(clip, plan.canvas_w, plan.canvas_h, plan.fps);
    let motion: String = [mo_in, mo_out]
        .iter()
        .filter(|c| !c.is_empty())
        .map(|c| format!(",{c}"))
        .collect();
    if pad_ms > 0.0 {
        if motion.is_empty() {
            parts.push(concat);
            parts.push(format!(
                "[vcat]tpad=stop_mode=clone:stop_duration={:.6}[vout]",
                pad_ms / 1000.0
            ));
        } else {
            parts.push(format!("{concat}{motion}[vmot]"));
            parts.push(format!(
                "[vmot]tpad=stop_mode=clone:stop_duration={:.6}[vout]",
                pad_ms / 1000.0
            ));
        }
    } else if motion.is_empty() {
        // 无 pad:concat 直出为 vout(改名以免悬空标签)
        parts.push(concat.replace("[vcat]", "[vout]"));
    } else {
        // 无 pad 有动画:concat+motion 直出为 vout
        parts.push(format!("{concat}{motion}[vout]"));
    }
    Some(parts.join(";"))
}

/// 关键帧段图(IR v3,T5.1;ADR-0018 表达式路线的段内落点):
///
/// ```text
/// [源] → crop → flip → rotate(kf 表达式,源域) → scale+pad+fps(归一)
///      → punchIn → zoompan(kf scale,源域) → fx.combo(kf sendcmd/segment 分支)
///      → reverse → 变速(kf speed 已并入 speed_segments;单段 setpts / 多段图)
///      → kf 合成块(黑底 overlay:opacity geq + position 表达式;播放域)
///      → motion → tpad
/// ```
///
/// 域纪律(kf_expr 模块注释):空间变换(rotation/scale)挂变速之前 = 源域 t,
/// 播放域锚点经 play_to_source 折算;合成包络(opacity/position)挂变速之后 =
/// 播放域 t,锚点零折算——先于 reverse 会被倒放镜像,先于变速会随速度拉伸。
/// 无关键帧的 clip 不进本函数(既有路径逐字不变,parity 红线)。
pub fn segment_filter_complex_kf(plan: &RenderPlan, clip: &Clip, tail_ms: f64) -> Option<String> {
    let fps = plan.fps;
    let (w, h) = (plan.canvas_w, plan.canvas_h);
    let mut parts: Vec<String> = Vec::new();
    let mut head = String::new();
    let pre = transform_pre_chain(clip);
    if !pre.is_empty() {
        head.push_str(&pre);
        head.push(',');
    }
    head.push_str(&base_filters(plan));
    head.push_str(&punch_chain(plan, clip));
    if let Some(zp) = crate::kf_expr::kf_zoompan_filter(clip, plan) {
        head.push(',');
        head.push_str(&zp);
    }
    // grade 调色(册五 T5.2):变换后、fx 前(与简单链同位;静态逐像素,不占 kf 通道)
    let (grade, _) = crate::grade::grade_chain(clip, &plan.project_dir);
    if !grade.is_empty() {
        head.push(',');
        head.push_str(&grade);
    }
    // fx 链三形态:sendcmd 命令序列 / segment 分支重建(推迟到 head 落标签后)/ 静态既有链
    let (fx, _) = crate::catalog::fx_chain(clip, w, h, fps);
    let seg_fx = crate::kf_expr::fx_keyframe_segments(clip, w, h, fps);
    if let Some((sc, _)) = crate::kf_expr::fx_keyframe_sendcmd(clip, w, h, fps) {
        head.push(',');
        head.push_str(&sc);
    } else if !fx.is_empty() && seg_fx.is_none() {
        head.push(',');
        head.push_str(&fx);
    }
    parts.push(format!("[0:v]{head}[k0]"));
    let mut cur = "k0".to_string();
    // segment 态 fx:split → 每段 trim+重基+常量链 → concat(段界 = 关键帧锚点)
    if let Some((bounds, chains)) = seg_fx {
        let n = chains.len();
        let splits: Vec<String> = (0..n).map(|i| format!("[sf{i}]")).collect();
        parts.push(format!("[{cur}]split={n}{}", splits.join("")));
        let mut refs = String::new();
        for (i, chain) in chains.iter().enumerate() {
            let (a, b) = (bounds[i], bounds[i + 1]);
            refs.push_str(&format!("[bf{i}]"));
            parts.push(format!(
                "[sf{i}]trim=start={a:.6}:end={b:.6},setpts=PTS-STARTPTS,{chain}[bf{i}]"
            ));
        }
        parts.push(format!("{refs}concat=n={n}:v=1:a=0[kc]"));
        cur = "kc".to_string();
    }
    if clip.reverse.unwrap_or(false) {
        parts.push(format!("[{cur}]reverse,setpts=N/FRAME_RATE/TB[kr]"));
        cur = "kr".to_string();
    }
    // 变速(播放域从此开始):多段图 / 单段 setpts+fps 重锁 / 恒速直通
    let segs = play_segments(clip);
    let speed = segs.last().map(|(_, _, s)| *s).unwrap_or(1.0);
    if segs.len() > 1 {
        let n = segs.len();
        let splits: Vec<String> = (0..n).map(|i| format!("[sp{i}]")).collect();
        parts.push(format!("[{cur}]split={n}{}", splits.join("")));
        let mut refs = String::new();
        let mut x = 0f64;
        for (i, (a, b, s)) in segs.iter().enumerate() {
            let x0 = x / 1000.0;
            x += (*b - *a) as f64 * s;
            let x1 = x / 1000.0;
            refs.push_str(&format!("[sc{i}]"));
            parts.push(format!(
                "[sp{i}]trim=start={x0:.6}:end={x1:.6},setpts=(PTS-STARTPTS)/{},fps={fps}[sc{i}]",
                crate::steps::fmt_f64(*s)
            ));
        }
        parts.push(format!("{refs}concat=n={n}:v=1:a=0[vs]"));
        cur = "vs".to_string();
    } else if speed != 1.0 {
        parts.push(format!(
            "[{cur}]setpts=PTS/{},fps={fps}[vs]",
            crate::steps::fmt_f64(speed)
        ));
        cur = "vs".to_string();
    }
    // kf 合成块(opacity/position;播放域,黑底画布宿主)
    if let Some(graph) = crate::kf_expr::kf_composite_block(clip, plan, &cur, "kp") {
        parts.push(graph);
        cur = "kp".to_string();
    }
    // motion + tpad(与既有链同序:动画后定格补长);全空 → null 直通
    // (标签→标签空接非法,且 -map 固定吃 [vout])
    let (mo_in, mo_out, _) = crate::catalog::motion_chains(clip, w, h, fps);
    let motion: String = [mo_in, mo_out]
        .iter()
        .filter(|c| !c.is_empty())
        .map(|c| format!(",{c}"))
        .collect();
    let pad_ms = segment_pad_ms(clip, tail_ms);
    if pad_ms > 0.0 {
        parts.push(format!(
            "[{cur}]{motion}tpad=stop_mode=clone:stop_duration={:.6}[vout]",
            pad_ms / 1000.0
        ));
    } else if motion.is_empty() {
        parts.push(format!("[{cur}]null[vout]"));
    } else {
        parts.push(format!("[{cur}]{motion}[vout]"));
    }
    Some(parts.join(";"))
}

/// 段提取命令行(-ss/-t 均在输入侧;-t = 分段积分的源域读取时长,册四 T4.4)。
/// 纯视频,-an;音频走 mix 步。简单链走 -vf(与拆分前参数逐字一致);
/// 曲线多段走 -filter_complex([vout] 显式 map)。
pub fn segment_args(plan: &RenderPlan, clip: &Clip, tail_ms: f64, seg_out: &Path) -> Vec<String> {
    let read_ms = segment_read_ms(clip).ceil();
    let ss = clip.source_in_ms.unwrap_or(0);
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-ss".into(),
        format!("{}", ss as f64 / 1000.0),
        "-t".into(),
        format!("{}", read_ms / 1000.0),
        "-i".into(),
        plan.project_dir
            .join(clip.src.clone().unwrap_or_default())
            .to_string_lossy()
            .into(),
    ];
    match segment_filter_complex(plan, clip, tail_ms) {
        Some(fc) => {
            args.extend([
                "-filter_complex".into(),
                fc,
                "-map".into(),
                "[vout]".into(),
                "-an".into(),
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                "veryfast".into(),
                seg_out.to_string_lossy().into(),
            ]);
        }
        None => {
            args.extend([
                "-vf".into(),
                segment_filter(plan, clip, tail_ms),
                "-an".into(),
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                "veryfast".into(),
                seg_out.to_string_lossy().into(),
            ]);
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
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

    // ---- 段链:兼容红线(无新字段 = 与拆分前逐字一致)+ 册四新链序 ----

    #[test]
    fn segment_args_match_legacy_invocation() {
        let (plan, clips) = test_plan(base_clips());
        let out = PathBuf::from("/c/seg/x.mp4");
        let args = segment_args(&plan, &clips[0], 0.0, &out);
        assert_eq!(
            strv(&args),
            [
                "-y",
                "-v",
                "error",
                "-ss",
                "0.5",
                "-t",
                "2",
                "-i",
                &pj("/w", "a.mp4"),
                "-vf",
                "scale=1080:1920:force_original_aspect_ratio=decrease,pad=1080:1920:(ow-iw)/2:(oh-ih)/2,fps=30",
                "-an",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "/c/seg/x.mp4"
            ]
        );
    }

    #[test]
    fn segment_filter_speed_and_tail_compose_in_order() {
        let mut v = base_clips();
        v[0]["speed"] = json!(2.0);
        let (plan, clips) = test_plan(v);
        let vf = segment_filter(&plan, &clips[0], 500.0);
        assert_eq!(
            vf,
            "scale=1080:1920:force_original_aspect_ratio=decrease,pad=1080:1920:(ow-iw)/2:(oh-ih)/2,fps=30"
                .to_string() + ",setpts=PTS/2,fps=30,tpad=stop_mode=clone:stop_duration=0.500000"
        );
    }

    #[test]
    fn segment_filter_punch_in_clamps_factor() {
        let mut v = base_clips();
        v[0]["punchIn"] = json!({"factor": 3.0, "source": "manual"});
        let (plan, clips) = test_plan(v);
        let vf = segment_filter(&plan, &clips[0], 0.0);
        // factor 3.0 → clamp 到 2.0
        assert!(
            vf.contains(",crop=w=iw/2:h=ih/2:x=(iw-ow)/2:y=(ih-oh)/2,scale=1080:1920"),
            "{vf}"
        );
    }

    /// 册四 T4.9 变换链(顺序红线:裁剪 → 翻转 → 旋转 → 画幅归一 → punchIn →
    /// reverse → 变速 → tpad);无新字段时链与拆分前逐字一致(上方两测锁定)。
    #[test]
    fn segment_transform_chain_order_crop_flip_rotate_reverse() {
        let mut v = base_clips();
        v[0]["crop"] = json!({"x": 10, "y": 20, "w": 300, "h": 200});
        v[0]["flip"] = json!("h");
        v[0]["rotation"] = json!(90.0);
        v[0]["reverse"] = json!(true);
        v[0]["speed"] = json!(2.0);
        let (plan, clips) = test_plan(v);
        let vf = segment_filter(&plan, &clips[0], 0.0);
        assert_eq!(
            vf,
            "crop=w=300:h=200:x=10:y=20,hflip,transpose=1,".to_string()
                + "scale=1080:1920:force_original_aspect_ratio=decrease,pad=1080:1920:(ow-iw)/2:(oh-ih)/2,fps=30,"
                + "reverse,setpts=N/FRAME_RATE/TB,setpts=PTS/2,fps=30"
        );
    }

    /// 旋转角度归一:负角/超圈 mod 360;-90→transpose=2;180→hflip,vflip;
    /// 任意角 rotate(弧度,黑底);0/缺省不产滤镜。
    #[test]
    fn rotation_normalizes_and_uses_transpose_for_right_angles() {
        let mk = |deg: Value| -> Clip {
            let v = json!({"id": "V1-001", "startMs": 0, "durationMs": 1000, "rotation": deg});
            serde_json::from_value(v).unwrap()
        };
        assert_eq!(transform_pre_chain(&mk(json!(-90.0))), "transpose=2");
        assert_eq!(transform_pre_chain(&mk(json!(270.0))), "transpose=2");
        assert_eq!(
            transform_pre_chain(&mk(json!(450.0))),
            "transpose=1",
            "超圈 mod 360"
        );
        assert_eq!(transform_pre_chain(&mk(json!(180.0))), "hflip,vflip");
        assert_eq!(
            transform_pre_chain(&mk(json!(30.0))),
            "rotate=0.523599:c=black"
        );
        assert_eq!(transform_pre_chain(&mk(json!(0.0))), "");
        assert_eq!(transform_pre_chain(&mk(json!(-720.0))), "", "整圈归零");
        // vflip
        let v = json!({"id": "V1-001", "startMs": 0, "durationMs": 1000, "flip": "v"});
        assert_eq!(
            transform_pre_chain(&serde_json::from_value::<Clip>(v).unwrap()),
            "vflip"
        );
    }

    /// 册四 T4.4:曲线多段 → filter_complex 分段图(trim 源域累计偏移 + 段均值 setpts
    /// + fps 重锁 + concat + tpad);输入侧 -t = 分段积分;简单链仍走 -vf。
    #[test]
    fn speed_curve_builds_multi_branch_filter_complex() {
        let mut v = base_clips();
        v[0]["durationMs"] = json!(2000);
        v[0]["speedCurve"] = json!([
            {"atMs": 0, "speed": 1.0}, {"atMs": 1000, "speed": 1.0}, {"atMs": 2000, "speed": 3.0}
        ]);
        let (plan, clips) = test_plan(v);
        let c = &clips[0];
        // 源域读取 = 1000×1.0 + 1000×2.0 = 3000ms(段 2 均值 (1+3)/2=2)
        assert_eq!(segment_read_ms(c), 3000.0);
        let args = segment_args(&plan, c, 0.0, Path::new("/c/seg/x.mp4"));
        assert_eq!(
            strv(&args)[..7],
            ["-y", "-v", "error", "-ss", "0.5", "-t", "3"]
        );
        assert!(
            args[9] == "-filter_complex",
            "曲线多段必须走 filter_complex"
        );
        let fc = &args[10];
        assert!(fc.starts_with("[0:v]scale=1080:1920:force_original_aspect_ratio=decrease,pad=1080:1920:(ow-iw)/2:(oh-ih)/2,fps=30,split=2[s0][s1];"), "{fc}");
        assert!(
            fc.contains("[s0]trim=start=0.000000:end=1.000000,setpts=(PTS-STARTPTS)/1,fps=30[c0];"),
            "{fc}"
        );
        assert!(
            fc.contains("[s1]trim=start=1.000000:end=3.000000,setpts=(PTS-STARTPTS)/2,fps=30[c1];"),
            "{fc}"
        );
        assert!(fc.contains("[c0][c1]concat=n=2:v=1:a=0[vout]"), "{fc}");
        assert_eq!(
            &strv(&args)[11..],
            [
                "-map",
                "[vout]",
                "-an",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "/c/seg/x.mp4"
            ]
        );
        // 无曲线:走 -vf,参数与拆分前逐字一致(上方 legacy 测锁定)
        let (plan2, clips2) = test_plan(base_clips());
        let args2 = segment_args(&plan2, &clips2[0], 0.0, Path::new("/c/seg/x.mp4"));
        assert_eq!(strv(&args2)[9], "-vf", "无曲线不得走 filter_complex");
    }

    /// 册四 T4.4 组合语义:freezeMs 定格 → 源只播放到定格点(-t 缩短),
    /// tpad 一次性补足「定格余量 + 转场尾帧」;定格与曲线组合时拐点后的段不读源。
    #[test]
    fn freeze_combines_with_speed_curve_and_tail() {
        let mut v = base_clips();
        v[0]["durationMs"] = json!(3000);
        v[0]["freezeMs"] = json!(1200);
        v[0]["speedCurve"] = json!([{"atMs": 0, "speed": 1.0}, {"atMs": 2000, "speed": 2.0}]);
        let (plan, clips) = test_plan(v);
        let c = &clips[0];
        // play=[0,1200):唯一曲线段 [0,3000) 均速 1.5 → 截断 [0,1200);读源 1200×1.5=1800ms
        assert_eq!(segment_read_ms(c), 1800.0);
        // pad = (3000-1200) 定格 + 500 尾帧 = 2300ms(截断后单段 → 仍走简单 -vf 链)
        let args = segment_args(&plan, c, 500.0, Path::new("/c/seg/x.mp4"));
        assert_eq!(strv(&args)[4..7], ["0.5", "-t", "1.8"]);
        assert_eq!(strv(&args)[9], "-vf");
        let vf = &args[10];
        assert!(
            vf.ends_with("tpad=stop_mode=clone:stop_duration=2.300000"),
            "{vf}"
        );
        // 无定格:pad = 尾帧(既有语义回归)
        let (plan2, clips2) = test_plan(base_clips());
        let vf2 = segment_filter(&plan2, &clips2[0], 500.0);
        assert!(
            vf2.ends_with("tpad=stop_mode=clone:stop_duration=0.500000"),
            "{vf2}"
        );
    }

    /// 倒放短片夹具语义锁(内存警示见 reverse_chain 注释):reverse 整段缓冲,
    /// 夹具与契约描述均用短片段;reverse 先于变速(链序由 transform 测试锁定)。
    #[test]
    fn reverse_only_clip_keeps_legacy_shape_with_reverse_chain() {
        let mut v = base_clips();
        v[0]["reverse"] = json!(true);
        v[0]["speed"] = json!(2.0);
        let (plan, clips) = test_plan(v);
        let vf = segment_filter(&plan, &clips[0], 0.0);
        assert_eq!(
            vf,
            "scale=1080:1920:force_original_aspect_ratio=decrease,pad=1080:1920:(ow-iw)/2:(oh-ih)/2,fps=30"
                .to_string()
                + ",reverse,setpts=N/FRAME_RATE/TB,setpts=PTS/2,fps=30"
        );
    }

    // ---- 源窗钳制(I1 渲染缺口修) ----

    fn clip_of(v: Value) -> Clip {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn source_clamp_is_noop_when_source_is_sufficient() {
        // 源充足 → None(参数与键零变化);1ms 内的亚毫秒差同样不触发
        let c = clip_of(json!({"id": "V1-001", "startMs": 0, "durationMs": 3000, "sourceInMs": 0}));
        assert_eq!(source_clamped_clip(&c, 3000), None);
        assert_eq!(source_clamped_clip(&c, 12_000), None);
        assert_eq!(source_clamped_clip(&c, 3000), None, "整窗恰好 = 可用,不钳");
    }

    #[test]
    fn source_clamp_freezes_tail_when_source_runs_out() {
        // 素材可用 1.2s,源窗要 3s → 播放域截到 1200ms,其后定格补满
        let c = clip_of(json!({"id": "V1-001", "startMs": 0, "durationMs": 3000, "sourceInMs": 0}));
        let clamped = source_clamped_clip(&c, 1200).expect("越界必钳");
        assert_eq!(clamped.freeze_ms, Some(1200));
        assert_eq!(clamped.duration_ms, 3000, "标称时长不变(时间线不动)");
        // 钳制后源消耗 = 可用预算,tpad 缺额由 segment_pad_ms 自动补齐
        assert!((segment_read_ms(&clamped) - 1200.0).abs() < 1e-6);
        assert!((segment_pad_ms(&clamped, 0.0) - 1800.0).abs() < 1e-6);
        // 键面:钳制改变 clip JSON → seg 键必须分叉(素材补长后自动重渲)
        let (plan, _) =
            test_plan(json!([{"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 3000}]));
        let tail = 0.0;
        assert_ne!(
            crate::cache::seg_key(&plan, &c, tail),
            crate::cache::seg_key(&plan, &clamped, tail),
            "源窗钳制必须入键"
        );
    }

    #[test]
    fn source_clamp_respects_source_in_and_curves() {
        // sourceIn 500 + 素材可用 1500 → 源窗预算 1000ms,常速截到播放域 1000ms
        let c =
            clip_of(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000, "sourceInMs": 500}));
        let clamped = source_clamped_clip(&c, 1500).expect("srcIn 挤占预算必钳");
        assert_eq!(clamped.freeze_ms, Some(1000), "截断点在播放域(srcIn 之后)");
        // 曲线:锚点原样保留,freeze 前移即精确钳制(段积分见 walk 实现)
        let c2 = clip_of(json!({"id": "V1-001", "startMs": 0, "durationMs": 3000,
            "speedCurve": [{"atMs": 0, "speed": 1.0}, {"atMs": 2000, "speed": 3.0}]}));
        // [0,2000)@均值 2.0 需 4000ms 源;预算 1500 → 播放域 1500/2.0 = 750ms 处耗尽
        let clamped2 = source_clamped_clip(&c2, 1500).expect("曲线越界必钳");
        assert_eq!(
            clamped2.freeze_ms,
            Some(750),
            "区间均值常速模型:star = 预算/段速"
        );
        assert!((segment_read_ms(&clamped2) - 1500.0).abs() < 1e-6);
        // 整窗完全越界(可用 < srcIn)→ 截断点 0(调用方据此报 PRECONDITION)
        let c3 =
            clip_of(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000, "sourceInMs": 5000}));
        let clamped3 = source_clamped_clip(&c3, 1000).expect("零预算也返回钳制视图供调用方判定");
        assert_eq!(clamped3.freeze_ms, Some(0));
    }
}
