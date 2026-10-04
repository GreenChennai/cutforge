// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 关键帧 → ffmpeg 表达式/分段表编译器(册五 T5.1,ADR-0018 按属性分级映射表)。
//!
//! **求值器单源纪律**(ADR-0018 决策 3):本模块是**编译目标**而非第二求值真相——
//! 全部锚点值来自 cutforge-core 求值器同点输出;一致性由 parity 逐样本实渲对拍兜底。
//! 编译器是确定性纯函数(字符串进、字符串出,不触进程)。
//!
//! 时间域换算:关键帧 timeMs 是**播放域**(相对片段起点);旋转变换挂在预归一链
//! (crop→flip→rotate 位,zoompan/geq/overlay 同在变速之前)= **源域** t——
//! 播放域锚点经 speed_segments 分段积分精确折算为源域秒(段内线性,段界细分);
//! volume 表达式在混音链 atempo 之后 = 播放域局部 t,锚点按子段起点平移。
//!
//! 属性分级映射(ADR-0018 决策 1):
//! | 属性 | 通路 | 变量 |
//! |---|---|---|
//! | position.x/y | 合成层 overlay x/y 表达式(eval=frame;黑底画布宿主) | t(源域) |
//! | rotation | rotate 角度表达式(接 crop→flip→rotate 位,替代静态 rotate) | t(源域,弧度×PI/180) |
//! | scale | zoompan z 逐帧表达式(居中取景,输出锁画布) | on(帧索引,ms→帧) |
//! | opacity | geq alpha 平面表达式(rgba 域)+ 黑底 overlay 合成 | T(源域) |
//! | volume | volume 表达式(eval=frame,混音链 atempo 后) | t(播放域局部) |
//! | speed | 不经本模块:core::speed_segments 消费 speed 关键帧(B 级单机制) | — |
//! | fx.<id>.<p> | 注册表三态:sendcmd 命令序列 / segment 分支重建 / static 已拒 | t(源域) |

use crate::plan::RenderPlan;
use cutforge_core::keyframes::{FxTimeline, group_by_property};
use cutforge_core::model::{Clip, speed_segments};
use serde_json::Value;

/// clip 是否携带**视觉类**关键帧(需要段图模式的属性;volume 走混音链,speed 走
/// 分段变速,都不强制段图)。
pub fn has_visual_keyframes(clip: &Clip) -> bool {
    let Some(kfs) = &clip.keyframes else {
        return false;
    };
    kfs.iter().any(|k| {
        matches!(
            k.property.as_str(),
            "position.x" | "position.y" | "scale" | "rotation" | "opacity"
        ) || k.property.starts_with("fx.")
    })
}

// ---------------- 时间域换算(播放域 → 源域,speed_segments 分段积分) ----------------

/// 播放域毫秒 → 源域毫秒(∫speed dt 的逆):段内线性内插,段界精确。
/// 无曲线(单段)即 play×speed。
pub fn play_to_source_ms(clip: &Clip, play_ms: f64) -> f64 {
    let segs = speed_segments(clip);
    if segs.is_empty() {
        return 0.0;
    }
    let mut x = 0f64;
    for &(a, b, s) in &segs {
        let span = (b - a) as f64;
        if play_ms <= b as f64 {
            let frac = ((play_ms - a as f64) / span).clamp(0.0, 1.0);
            return x + frac * span * s;
        }
        x += span * s;
    }
    x
}

/// 关键帧锚点表(秒,值)。域判定(ADR-0018 链序纪律):
/// - `Domain::Source`(rotate/zoompan/geq 前的源域流,挂变速之前):播放域锚点经
///   speed_segments 分段积分精确折算;线性区间 2 锚点(精确),非线性缓动/贝塞尔
///   区间按求值器(单源)细分 SUBDIV 段折线逼近;speed 段边界处细分保两域一致;
/// - `Domain::Play`(合成包络:opacity/position 黑底 overlay 挂变速之后、volume
///   挂混音链 atempo 之后,t 即播放域):锚点直取 timeMs/1000,零折算。
const SUBDIV: f64 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    /// 源域(挂变速之前的滤镜 t = 源流秒)。
    Source,
    /// 播放域(挂变速之后的滤镜 t = 片段播放秒)。
    Play,
}

fn anchors(clip: &Clip, property: &str, domain: Domain) -> Option<Vec<(f64, f64)>> {
    let kfs = clip.keyframes.as_ref()?;
    let groups = group_by_property(kfs);
    let (_, group) = groups.iter().find(|(p, _)| p == property)?;
    let to_t = |play_ms: f64| -> f64 {
        match domain {
            Domain::Play => play_ms / 1000.0,
            Domain::Source => play_to_source_ms(clip, play_ms) / 1000.0,
        }
    };
    let ev = |t_play: f64| -> f64 {
        let refs: Vec<&cutforge_core::keyframes::Keyframe> = group.iter().collect();
        cutforge_core::keyframes::eval_group(
            &refs,
            |k| cutforge_core::keyframes::parse_interp(&k.interp, k.bezier),
            t_play,
        )
    };
    let speed_bounds: Vec<f64> = match domain {
        Domain::Play => Vec::new(),
        Domain::Source => {
            let segs = speed_segments(clip);
            segs.iter()
                .map(|&(a, _, _)| a as f64)
                .chain(std::iter::once(clip.duration_ms as f64))
                .collect()
        }
    };
    let mut out: Vec<(f64, f64)> = Vec::new();
    for w in group.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let (t0, t1) = (a.time_ms as f64, b.time_ms as f64);
        let interp = cutforge_core::keyframes::parse_interp(&a.interp, a.bezier);
        let steps = if matches!(interp, cutforge_core::keyframes::Interp::Bezier(_)) {
            SUBDIV as u64
        } else {
            1
        };
        for i in 0..steps {
            let ta = t0 + (t1 - t0) * (i as f64 / steps as f64);
            let tb = t0 + (t1 - t0) * ((i + 1) as f64 / steps as f64);
            out.push((to_t(ta), ev(ta)));
            // 子区间内的 speed 段边界(播放域)折点:Source 域 τ 折而 t 线性 → 细分
            for &sb in &speed_bounds {
                if sb > ta && sb < tb {
                    out.push((to_t(sb), ev(sb)));
                }
            }
            if i + 1 == steps {
                out.push((to_t(tb), ev(tb)));
            }
        }
    }
    if group.len() == 1 {
        // 单点 = 常值(端点外延两锚点)
        out.push((0.0, group[0].value));
        out.push((to_t(group[0].time_ms as f64), group[0].value));
        if domain == Domain::Play {
            out.push((f64::MAX / 4.0, group[0].value));
        }
    }
    out.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1)));
    out.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9);
    Some(out)
}

/// 锚点表 → ffmpeg 嵌套 if 表达式(var ∈ {t,on,T};区间 between+lerp,
/// 区间外端值外延)。确定性:锚点序固定,构造自内向外单次扫描。
fn emit_if_else(anchors: &[(f64, f64)], var: &str) -> String {
    let fmt = crate::steps::fmt_f64;
    if anchors.is_empty() {
        return "0".into();
    }
    if anchors.len() == 1 {
        return fmt(anchors[0].1);
    }
    let first = anchors[0].1;
    // 自最后一个区间向前包裹:内层 = 末端值(后延),外层逐区间展开
    let last_v = anchors[anchors.len() - 1].1;
    let mut expr = fmt(last_v);
    for w in anchors.windows(2).rev() {
        let ((ta, va), (tb, vb)) = (w[0], w[1]);
        let span = tb - ta;
        let lerp = if span > 1e-9 && (vb - va).abs() > 1e-12 {
            format!("{}+({})*({var}-{ta})", fmt(va), fmt((vb - va) / span))
        } else {
            fmt(va)
        };
        expr = format!(
            "if(between({var},{},{}),{},****ELSE****)",
            fmt(ta),
            fmt(tb),
            lerp
        )
        .replace("****ELSE****", &expr);
    }
    format!(
        "if(lt({var},{}),{},****FIRST****)",
        fmt(anchors[0].0),
        fmt(first)
    )
    .replace("****FIRST****", &expr)
}

// ---------------- 视觉通路(A 级表达式直译) ----------------

/// rotation 关键帧 → rotate 滤镜(角度表达式,弧度;接 crop→flip→rotate 位,
/// 替代静态 rotate:keyframed 逐帧覆盖静态值)。None = 无 rotation 关键帧。
pub fn kf_rotate_filter(clip: &Clip) -> Option<String> {
    let anchors = anchors(clip, "rotation", Domain::Source)?;
    let expr = emit_if_else(&anchors, "t");
    Some(format!("rotate='({expr})*PI/180':c=black"))
}

/// scale 关键帧 → zoompan(居中逐帧缩放;on 帧计数,ms→帧索引按 fps;输出锁画布)。
/// None = 无 scale 关键帧。
pub fn kf_zoompan_filter(clip: &Clip, plan: &RenderPlan) -> Option<String> {
    let anchors = anchors(clip, "scale", Domain::Source)?;
    // τ(源域秒)→ 帧索引:每帧代表 [n/fps,(n+1)/fps) → 锚点取帧索引 = τ×fps
    let on_anchors: Vec<(f64, f64)> = anchors
        .into_iter()
        .map(|(t, v)| ((t * plan.fps as f64 * 100.0).round() / 100.0, v))
        .collect();
    let expr = emit_if_else(&on_anchors, "on");
    let (w, h, fps) = (plan.canvas_w, plan.canvas_h, plan.fps);
    Some(format!(
        "zoompan=z='{expr}':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d=1:s={w}x{h}:fps={fps}"
    ))
}

/// 段图形态的 opacity/position 关键帧合成块(ADR-0018:geq alpha + overlay 表达式;
/// 黑底画布宿主同域合成,不另起管线通道)。**挂点 = 变速之后**(播放域:t 即
/// 片段播放秒,锚点零折算)——opacity/position 是播放域包络,先于 reverse 会
/// 被倒放镜像、先于变速会随速度拉伸,故置于 reverse/变速之后、motion 之前:
/// - opacity:format=rgba → geq 四平面(alpha 乘 alpha(t))——只挂有 opacity 关键帧的片段;
/// - position:overlay x/y 表达式(eval=frame;x = pos.x×W − W/2,0.5 = 居中);
///
/// 两者任一存在即挂黑底 overlay。返回 graph 片段:
/// `[fg]chain[fgk];color=black:...[bg];[bg][fgk]overlay=...[out]`。
pub fn kf_composite_block(
    clip: &Clip,
    plan: &RenderPlan,
    fg_label: &str,
    out_label: &str,
) -> Option<String> {
    let has_pos = has_property(clip, "position.x") || has_property(clip, "position.y");
    let has_opa = has_property(clip, "opacity");
    if !has_pos && !has_opa {
        return None;
    }
    let (w, h, fps) = (plan.canvas_w, plan.canvas_h, plan.fps);
    let mut chain = String::new();
    if has_opa {
        let anchors = anchors(clip, "opacity", Domain::Play).expect("opacity 关键帧已判存在");
        let alpha = emit_if_else(&anchors, "T");
        // alpha 平面访问器是 alpha(X,Y)(本机实测 a() 非函数),值域已 0..255——
        // 直接乘 alpha(t)∈[0,1](实测 ×255 会整型溢出产生全亮伪影);min/max 钳制
        chain.push_str(&format!(
            ",format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='max(0,min(255,alpha(X,Y)*({alpha})))'"
        ));
    }
    let xy = |prop: &str, span: u32| -> String {
        if has_property(clip, prop) {
            let anchors = anchors(clip, prop, Domain::Play).expect("position 关键帧已判存在");
            format!(
                "(({expr})*{span}-{half})",
                expr = emit_if_else(&anchors, "t"),
                half = span / 2
            )
        } else {
            "0".to_string()
        }
    };
    let x = xy("position.x", w);
    let y = xy("position.y", h);
    // 黑底宿主必须有界(color 无 d 会 infinite → overlay 永不结束)+ shortest=1。
    // fg 侧:有 opacity 修饰链 → 新链 [fg]{chain}[fgk];仅位移(无修饰)→ fg 标签
    // 直连 overlay(无滤镜不得出现 标签→标签 空接)
    let dur_s = clip.duration_ms as f64 / 1000.0;
    let chain = chain.trim_start_matches(',');
    let fg_part = if chain.is_empty() {
        format!("[bg][{fg_label}]")
    } else {
        format!("[{fg_label}]{chain}[fgk];[bg][fgk]")
    };
    Some(format!(
        "{fg_part}overlay=x='{x}':y='{y}':eval=frame:shortest=1[{out_label}];color=c=black:s={w}x{h}:r={fps}:d={}[bg]",
        crate::steps::fmt_f64(dur_s)
    ))
}

fn has_property(clip: &Clip, prop: &str) -> bool {
    clip.keyframes
        .as_ref()
        .is_some_and(|kfs| kfs.iter().any(|k| k.property == prop))
}

// ---------------- volume 通路(混音链;播放域局部 t) ----------------

/// volume 关键帧 → volume 表达式(eval=frame)体,`volume='{expr}':eval=frame`。
/// `offset_ms` = 子段在片段播放域内的起点(混音链 t 为子段局部秒,锚点平移);
/// 无 volume 关键帧 → None(调用方回落静态音量)。
pub fn kf_volume_filter(clip: &Clip, offset_ms: u64) -> Option<String> {
    let kfs = clip.keyframes.as_ref()?;
    if !kfs.iter().any(|k| k.property == "volume") {
        return None;
    }
    // 播放域锚点(秒;volume 在 atempo 之后,t 已是子段局部播放秒)→ 平移 offset
    let anchors_all = anchors(clip, "volume", Domain::Play).expect("volume 关键帧已判存在");
    let off = offset_ms as f64 / 1000.0;
    let mut shifted: Vec<(f64, f64)> = anchors_all.into_iter().map(|(t, v)| (t - off, v)).collect();
    shifted.sort_by(|x, y| x.0.total_cmp(&y.0));
    shifted.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9);
    let expr = emit_if_else(&shifted, "t");
    Some(format!("volume='{expr}':eval=frame"))
}

// ---------------- fx 通路(B 级;注册表三态) ----------------

/// fx 关键帧的 sendcmd 命令串 + 打标签后的 fx 链(ADR-0018 决策 4 sendcmd 态):
/// 逐关键帧区间生成 `[{t0} {t1}] {filter}@kf{i} {cmdOption} {value}` 命令
/// (区间内逐帧生效 = 分段常量,值 = 求值器区间中点——B 级分段逼近,单源求值);
/// 无 keyframed sendcmd 参数 → None。**只对 combo 内条目**(in/out 槽位本册仍不渲染)。
pub fn fx_keyframe_sendcmd(clip: &Clip, w: u32, h: u32, fps: u32) -> Option<(String, String)> {
    let combo = clip.fx.as_ref()?.combo.as_ref()?;
    let kfs = clip.keyframes.as_ref()?;
    let mut commands: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    let mut any = false;
    for (i, entry) in combo.iter().enumerate() {
        let Some(def) = crate::catalog::fx_list().iter().find(|f| f.id == entry.fx) else {
            continue; // 未注册由 fx_chain 既有降级路径 WARN
        };
        let mut labeled = false;
        for p in &def.params {
            let prop = format!("fx.{}.{}", def.id, p.name);
            if !kfs.iter().any(|k| k.property == prop) {
                continue;
            }
            if cutforge_core::keyframes::fx_param_timeline(&def.id, &p.name).2
                != FxTimeline::Sendcmd
            {
                continue; // segment 态由 fx_keyframe_segments 承接;static 已被校验拒
            }
            // 采样密度 100ms(播放域;B 级分段常量逼近,密度与对拍阈值同源):
            // sendcmd 裸时间点 = "t≥T 起生效",逐采样点设值,阶梯逼近曲线
            let (t_start, t_end) = {
                let pts: Vec<u64> = kfs
                    .iter()
                    .filter(|k| k.property == prop)
                    .map(|k| k.time_ms)
                    .collect();
                (
                    *pts.iter().min().unwrap_or(&0),
                    *pts.iter().max().unwrap_or(&0),
                )
            };
            let mut step = t_start;
            let mut times: Vec<u64> = Vec::new();
            while step < t_end {
                times.push(step);
                step += 100;
            }
            times.push(t_end);
            let cmd = p_cmd_option(def, &p.name).expect("sendcmd 注册必须带 cmdOption");
            for t_play in times {
                let tau = play_to_source_ms(clip, t_play as f64) / 1000.0;
                let v = cutforge_core::keyframes::eval_property(clip, &prop, t_play as f64)
                    .unwrap_or(0.0)
                    .clamp(p.min, p.max);
                commands.push(format!(
                    "{tau:.3} {}@kf{i} {} {};",
                    def.id,
                    cmd,
                    crate::steps::fmt_f64(v)
                ));
            }
            labeled = true;
            any = true;
        }
        let (text, _) = crate::catalog::fx_entry_text(def, entry, w, h, fps, &format!("c{i}"));
        let text = if labeled {
            // 滤镜串打实例标签:filter=opts → filter@kf{i}=opts(标签只加在主滤镜名上)
            match text.split_once('=') {
                Some((name, rest)) => format!("{name}@kf{i}={rest}"),
                None => format!("{text}@kf{i}"),
            }
        } else {
            text
        };
        parts.push(text);
    }
    if !any {
        return None;
    }
    // sendcmd 内联命令串(commands=;本机实测:区间形式不可用,裸时间点 + ';' 分隔)
    let cmd_text = commands.join("");
    let chain = parts.join(",");
    Some((format!("sendcmd=commands='{cmd_text}',{chain}"), chain))
}

fn p_cmd_option(def: &crate::catalog::FxDef, param: &str) -> Option<String> {
    // cmdOption 注册在 fx-catalog params(册五 T5.1);目录解析走原始 JSON
    let doc: Value = serde_json::from_str(cutforge_core::keyframes::FX_CATALOG_SOURCE).ok()?;
    doc["fx"]
        .as_array()?
        .iter()
        .find(|f| f["id"].as_str() == Some(def.id.as_str()))?["params"]
        .as_array()?
        .iter()
        .find(|p| p["name"].as_str() == Some(param))?["cmdOption"]
        .as_str()
        .map(String::from)
}

/// 源域毫秒 → 播放域毫秒(speed_segments 分段积分正变换)。
pub fn source_to_play_ms(clip: &Clip, source_ms: f64) -> f64 {
    let segs = speed_segments(clip);
    let mut acc = 0f64;
    for &(a, b, s) in &segs {
        let span = (b - a) as f64 * s;
        if acc + span >= source_ms {
            let into = (source_ms - acc) / s;
            return a as f64 + into;
        }
        acc += span;
    }
    acc.max(0.0)
}

/// fx 关键帧 segment 态(分段子段重建;ADR-0018 决策 4):返回
/// (分段边界源域秒升序去重, 每段滤镜链)——段数 = 边界数-1;segment 态
/// keyframed 参数按**段中点求值常量化**(B 级分段逼近,求值单源),其余条目/参数
/// 走既有默认裁决。无 segment 态 keyframed 参数 → None。
pub fn fx_keyframe_segments(
    clip: &Clip,
    w: u32,
    h: u32,
    fps: u32,
) -> Option<(Vec<f64>, Vec<String>)> {
    let combo = clip.fx.as_ref()?.combo.as_ref()?;
    let kfs = clip.keyframes.as_ref()?;
    let is_seg_param = |prop: &str| -> bool {
        prop.strip_prefix("fx.")
            .and_then(|rest| rest.rsplit_once('.'))
            .is_some_and(|(fx_id, param)| {
                cutforge_core::keyframes::fx_param_timeline(fx_id, param).2 == FxTimeline::Segment
            })
    };
    let seg_props: Vec<&str> = kfs
        .iter()
        .map(|k| k.property.as_str())
        .filter(|p| p.starts_with("fx.") && is_seg_param(p))
        .collect();
    if seg_props.is_empty() {
        return None;
    }
    // 分段边界 = 全部 segment 态关键帧锚点时刻(源域秒)+ 起末
    let mut bounds: Vec<f64> = vec![0.0];
    for prop in &seg_props {
        if let Some(anchors) = anchors(clip, prop, Domain::Source) {
            bounds.extend(anchors.iter().map(|(t, _)| *t));
        }
    }
    bounds.push(play_to_source_ms(clip, clip.duration_ms as f64) / 1000.0);
    bounds.sort_by(|a, b| a.total_cmp(b));
    bounds.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    // 每段:segment 态 keyframed 参数常量化为段中点值(经播放域反查求值器,单源)
    let mut chains = Vec::new();
    for wnd in bounds.windows(2) {
        let mid_tau = (wnd[0] + wnd[1]) / 2.0;
        let mid_play = source_to_play_ms(clip, mid_tau * 1000.0);
        let mut parts: Vec<String> = Vec::new();
        for (i, entry) in combo.iter().take(crate::catalog::FX_COMBO_CAP).enumerate() {
            let Some(def) = crate::catalog::fx_list().iter().find(|f| f.id == entry.fx) else {
                continue;
            };
            let mut e = entry.clone();
            if let Some(params) = &mut e.params {
                for p in &def.params {
                    let prop = format!("fx.{}.{}", def.id, p.name);
                    if seg_props.contains(&prop.as_str())
                        && let Some(v) =
                            cutforge_core::keyframes::eval_property(clip, &prop, mid_play)
                    {
                        params.insert(p.name.clone(), serde_json::json!(v.clamp(p.min, p.max)));
                    }
                }
            }
            let (text, _) = crate::catalog::fx_entry_text(def, &e, w, h, fps, &format!("sg{i}"));
            parts.push(text);
        }
        chains.push(parts.join(","));
    }
    Some((bounds, chains))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    fn clip_with(v: Value) -> Clip {
        serde_json::from_value(v).unwrap()
    }

    fn plan() -> RenderPlan {
        let project: cutforge_core::model::Project = serde_json::from_value(json!({
            "version": 1, "schemaVersion": "3.0.0", "slug": "kf", "fps": 30,
            "canvas": {"width": 320, "height": 240},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000}
            ]}]
        }))
        .unwrap();
        RenderPlan::build(&project, Path::new("/w"), None)
    }

    // ---- 时间域换算 ----

    #[test]
    fn play_source_conversion_roundtrip_constant_speed() {
        let c = clip_with(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000, "speed": 2.0}));
        assert_eq!(
            play_to_source_ms(&c, 1000.0),
            2000.0,
            "2x 速:播放 1s 消费源 2s"
        );
        assert!(
            (source_to_play_ms(&c, 2000.0) - 1000.0).abs() < 1e-9,
            "逆变换回程"
        );
        let c = clip_with(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000}));
        assert_eq!(play_to_source_ms(&c, 777.0), 777.0, "1x 恒等");
    }

    #[test]
    fn play_source_conversion_piecewise_curve() {
        // 曲线 [0,1000)@1.0 + [1000,2000]@3.0(均值段)→ 段界折算精确
        let c = clip_with(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000,
            "speedCurve": [{"atMs": 0, "speed": 1.0}, {"atMs": 1000, "speed": 1.0}, {"atMs": 2000, "speed": 3.0}]
        }));
        assert_eq!(play_to_source_ms(&c, 500.0), 500.0, "段 1 @1.0");
        // 播放 1500ms = 段 1 全部(1000ms→源 1000)+ 段 2 一半(500ms×2.0 均速 = 源 1000)
        assert_eq!(play_to_source_ms(&c, 1500.0), 2000.0);
    }

    // ---- 表达式编译(确定性 + 结构) ----

    #[test]
    fn rotate_expression_compiles_deterministic_nested_between() {
        let c = clip_with(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000,
            "keyframes": [
                {"property": "rotation", "timeMs": 0, "value": 0.0},
                {"property": "rotation", "timeMs": 1000, "value": 90.0}
            ]
        }));
        let f1 = kf_rotate_filter(&c).unwrap();
        let f2 = kf_rotate_filter(&c).unwrap();
        assert_eq!(f1, f2, "同输入确定性");
        assert!(f1.starts_with("rotate='("), "{f1}");
        assert!(f1.contains("between(t,0,1)"), "{f1}");
        assert!(f1.contains("*PI/180"), "角度转弧度: {f1}");
        assert!(
            f1.ends_with("':c=black"),
            "黑底补白与静态 rotate 同语义: {f1}"
        );
        assert!(
            f1.contains("if(between(t,0,1),0+(90)*(t-0),90)"),
            "线性区间 lerp: {f1}"
        );
        let c2 = clip_with(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000}));
        assert!(kf_rotate_filter(&c2).is_none(), "无关键帧不产滤镜");
    }

    #[test]
    fn zoompan_uses_on_frame_counter_and_locked_canvas() {
        let p = plan();
        let c = clip_with(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000,
            "keyframes": [
                {"property": "scale", "timeMs": 0, "value": 1.0},
                {"property": "scale", "timeMs": 1000, "value": 2.0}
            ]
        }));
        let f = kf_zoompan_filter(&c, &p).unwrap();
        assert!(f.starts_with("zoompan=z='"), "{f}");
        assert!(
            f.contains("between(on,0,30)"),
            "1000ms@30fps → 30 帧索引: {f}"
        );
        assert!(f.contains("s=320x240:fps=30"), "输出锁画布: {f}");
        assert!(f.contains("iw/2-(iw/zoom/2)"), "居中取景: {f}");
    }

    #[test]
    fn composite_block_opacity_geq_and_position_overlay() {
        let p = plan();
        let c = clip_with(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000,
            "keyframes": [
                {"property": "opacity", "timeMs": 0, "value": 0.0},
                {"property": "opacity", "timeMs": 1000, "value": 1.0},
                {"property": "position.x", "timeMs": 0, "value": 0.5},
                {"property": "position.x", "timeMs": 2000, "value": 0.7}
            ]
        }));
        let g = kf_composite_block(&c, &p, "fg", "out").unwrap();
        assert!(g.contains("format=rgba,geq="), "opacity 走 geq alpha: {g}");
        assert!(
            g.contains("alpha(X,Y)*("),
            "alpha 平面乘式(alpha() 访问器): {g}"
        );
        assert!(g.contains("overlay=x='"), "{g}");
        assert!(g.contains("*320-160"), "x = pos.x×W − W/2(0.5 居中): {g}");
        assert!(g.contains("eval=frame"), "{g}");
        assert!(
            g.contains("color=c=black:s=320x240:r=30:d=2"),
            "黑底画布宿主(有界): {g}"
        );
        assert!(
            g.contains("shortest=1"),
            "overlay 终止于前景(黑底宿主有界双保险): {g}"
        );
        // 只有 opacity:overlay 仍在(合成宿主),x/y 恒 0
        let c2 = clip_with(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000,
            "keyframes": [
                {"property": "opacity", "timeMs": 0, "value": 0.0},
                {"property": "opacity", "timeMs": 1000, "value": 1.0}
            ]
        }));
        let g2 = kf_composite_block(&c2, &p, "fg", "out").unwrap();
        assert!(
            g2.contains("overlay=x='0':y='0'"),
            "无位移关键帧 x/y 恒 0: {g2}"
        );
        // 都没有:None
        let c3 = clip_with(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000}));
        assert!(kf_composite_block(&c3, &p, "fg", "out").is_none());
    }

    #[test]
    fn volume_expression_shifts_to_segment_local_time() {
        let c = clip_with(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000,
            "keyframes": [
                {"property": "volume", "timeMs": 0, "value": 1.0},
                {"property": "volume", "timeMs": 1000, "value": 0.0}
            ]
        }));
        // 子段起点 500ms:锚点平移 −0.5s(淡出区间 [0,1000) → 局部 [−0.5,0.5))
        let f = kf_volume_filter(&c, 500).unwrap();
        assert!(f.starts_with("volume='"), "{f}");
        assert!(
            f.contains("between(t,-0.5,0.5)"),
            "局部时间锚点(平移后): {f}"
        );
        assert!(f.contains("eval=frame"), "{f}");
        let none = clip_with(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000}));
        assert!(kf_volume_filter(&none, 0).is_none());
    }

    // ---- fx 三态通路 ----

    #[test]
    fn fx_sendcmd_commands_labeled_and_interval_bound() {
        let c = clip_with(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000,
            "fx": {"combo": [{"fx": "fx.grain", "params": {"strength": 18}}]},
            "keyframes": [
                {"property": "fx.fx.grain.strength", "timeMs": 0, "value": 10.0},
                {"property": "fx.fx.grain.strength", "timeMs": 1000, "value": 40.0}
            ]
        }));
        let (sc, chain) = fx_keyframe_sendcmd(&c, 320, 240, 30).unwrap();
        assert!(sc.starts_with("sendcmd=commands='"), "{sc}");
        assert!(
            sc.contains(" fx.grain@kf0 alls "),
            "裸时间点命令 + 实例标签: {sc}"
        );
        assert!(sc.contains(';'), "多采样点以 ';' 分隔: {sc}");
        assert!(chain.contains("noise@kf0="), "滤镜串打标签: {chain}");
        // 端点值 = 求值器同点输出(单源):t=0 → 10;t=1s → 40;采样密度 100ms
        assert!(
            sc.starts_with("sendcmd=commands='0.000 fx.grain@kf0 alls 10;"),
            "{sc}"
        );
        assert!(sc.contains("1.000 fx.grain@kf0 alls 40;"), "末点值: {sc}");
        assert!(
            sc.contains("0.500 fx.grain@kf0 alls 25;"),
            "线性中点值(求值器同点): {sc}"
        );
    }

    #[test]
    fn fx_sendcmd_none_without_keyframed_params() {
        let c = clip_with(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000,
            "fx": {"combo": [{"fx": "fx.grain"}]},
            "keyframes": [
                {"property": "opacity", "timeMs": 0, "value": 0.0},
                {"property": "opacity", "timeMs": 1000, "value": 1.0}
            ]
        }));
        assert!(
            fx_keyframe_sendcmd(&c, 320, 240, 30).is_none(),
            "无 fx 关键帧不产 sendcmd"
        );
    }

    #[test]
    fn fx_segment_state_builds_branch_bounds() {
        let c = clip_with(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000,
            "fx": {"combo": [{"fx": "fx.mosaic", "params": {"block": 16}}]},
            "keyframes": [
                {"property": "fx.fx.mosaic.block", "timeMs": 0, "value": 8.0},
                {"property": "fx.fx.mosaic.block", "timeMs": 1000, "value": 32.0}
            ]
        }));
        let (bounds, chains) = fx_keyframe_segments(&c, 320, 240, 30).unwrap();
        assert_eq!(bounds.first(), Some(&0.0));
        assert_eq!(
            bounds.last().map(|b| (*b * 1000.0).round()),
            Some(2000.0),
            "末界 = 播放末点折算"
        );
        assert_eq!(chains.len(), bounds.len() - 1, "每段一条重建链");
        assert!(
            chains
                .iter()
                .all(|c| c.contains("mosaic") || c.contains("scale=iw")),
            "段链含 fx 重建: {chains:?}"
        );
    }

    #[test]
    fn has_visual_keyframes_scopes_correctly() {
        let vol = clip_with(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000,
            "keyframes": [
                {"property": "volume", "timeMs": 0, "value": 1.0},
                {"property": "volume", "timeMs": 1000, "value": 0.5}
            ]
        }));
        assert!(!has_visual_keyframes(&vol), "volume 走混音链,不强制段图");
        let speed = clip_with(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000,
            "keyframes": [
                {"property": "speed", "timeMs": 0, "value": 1.0},
                {"property": "speed", "timeMs": 1000, "value": 2.0}
            ]
        }));
        assert!(!has_visual_keyframes(&speed), "speed 走分段变速");
        let rot = clip_with(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000,
            "keyframes": [
                {"property": "rotation", "timeMs": 0, "value": 0.0},
                {"property": "rotation", "timeMs": 1000, "value": 45.0}
            ]
        }));
        assert!(
            has_visual_keyframes(&rot),
            "rotation 亦入段图(单路径免漂移;含 kf rotate 的 transform_pre_chain)"
        );
    }
}
