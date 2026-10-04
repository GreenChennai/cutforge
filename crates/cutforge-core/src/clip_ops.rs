// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 片段级深度操作(审查报告 v2 BUG-01/02/04 收口):切分/合并的**语义**收进
//! 模型层,engine 只做定位与 Op 面组装(§10「split_at/merge_with 收进 model」)。
//!
//! 时间换算**单一真相源** = [`speed_segments`] 分段积分:
//! - [`source_read_ms`]/[`source_read_ms_upto`]:整段/前缀源读时长(∫speed dt);
//! - [`Clip::split_at`]:时间字段、sourceIn 换算(含 speed/speedCurve/reverse)、
//!   speedCurve 与关键帧 rebase、fade/motion/transition/freeze/text 归属;
//! - [`Clip::merge_with`]:语义连续性谓词(同源/同速/内容窗相接)+ split 的精确逆。
//!
//! 归属规则(切分):入场侧(fade.in/motion.in)只留左,出场侧(fade.out/motion.out)
//! 只留右,transition 只留左,片尾定格 freeze_ms 只留右,文本只留左——防切点处
//! 双重淡入/双重转场(BUG-02)。engine 不再做任何私有坐标平移。

use crate::keyframes::Keyframe;
use crate::model::{Clip, Fade, Motion, SpeedPoint};

/// 时间线恒速段(册四 A4 T4.4 的**单一真相源**,册五 T5.1 扩 speed 关键帧入口):
/// `(start_ms, end_ms, mean_speed)` 三元组,`mean_speed` 为该段渲染用的常速
/// (区间两端点速度的算术平均 = 线性插值 speed 函数在区间上的积分均值)。
///
/// 口径(投影与渲染共用本函数,红线 = 两边时长严格一致):
/// - **speed 关键帧优先**(IR v3):clip.keyframes 含 speed 属性时,先经
///   [`crate::keyframes::speed_keyframes_to_curve`] 合成为曲线(非线性缓动区间
///   按求值器细分逼近),此后与本条 speedCurve 口径完全一致(与 speedCurve 互斥
///   由校验器保证,此处不再判);
/// - 无 speedCurve → 单段 `(0, duration_ms, speed.unwrap_or(1.0))`,与既有线性 speed 完全同形;
/// - 有 speedCurve → 点按 atMs 升序(防御性排序),首点速度前延到 0、末点速度后延到
///   durationMs(端点常速外延);相邻点之间 speed 函数线性插值,渲染按区间
///   **均值常速**执行(每段一个 setpts,总时长与总源消耗都是分段积分的精确值);
/// - 单点曲线 ≡ 常速;atMs 超出 [0,durationMs] 的点被钳到边界后并段。
pub fn speed_segments(clip: &Clip) -> Vec<(u64, u64, f64)> {
    let dur = clip.duration_ms;
    if dur == 0 {
        return Vec::new();
    }
    // speed 关键帧(IR v3):合成曲线后与 speedCurve 同路径(B 级分段常速逼近)
    let speed_kf_curve = clip.keyframes.as_ref().and_then(|kfs| {
        let pts: Vec<crate::keyframes::Keyframe> = kfs
            .iter()
            .filter(|k| k.property == "speed")
            .cloned()
            .collect();
        if pts.is_empty() {
            None
        } else {
            Some(crate::keyframes::speed_keyframes_to_curve(&pts))
        }
    });
    let points = speed_kf_curve.as_ref().or(clip.speed_curve.as_ref());
    let Some(points) = points else {
        return vec![(0, dur, clip.speed.unwrap_or(1.0))];
    };
    // 防御性归一:排序(乱序输入),钳到 [0, dur](越界点钳边)
    let mut pts: Vec<(u64, f64)> = points.iter().map(|p| (p.at_ms.min(dur), p.speed)).collect();
    pts.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    if pts.is_empty() {
        return vec![(0, dur, clip.speed.unwrap_or(1.0))];
    }
    // 端点外延成完整覆盖 [0, dur] 的节点序列:0→首点速度,dur→末点速度
    let mut nodes: Vec<(u64, f64)> = Vec::with_capacity(pts.len() + 2);
    if pts[0].0 > 0 {
        nodes.push((0, pts[0].1));
    }
    nodes.extend(pts);
    if nodes.last().map(|(t, _)| *t).unwrap_or(0) < dur {
        let s = nodes.last().map(|(_, s)| *s).unwrap_or(1.0);
        nodes.push((dur, s));
    }
    // 相邻节点成段;区间速度 = 线性插值 → 常速渲染取区间均值(积分精确)
    let mut segs: Vec<(u64, u64, f64)> = Vec::new();
    for w in nodes.windows(2) {
        let (a, sa) = w[0];
        let (b, sb) = w[1];
        if b <= a {
            continue; // 零长段(重复 atMs)跳过
        }
        let mean = (sa + sb) / 2.0;
        match segs.last_mut() {
            // 均值相等的相邻段并段(等速点不产生多余 setpts)
            Some(last) if (last.2 - mean).abs() < f64::EPSILON => last.1 = b,
            _ => segs.push((a, b, mean)),
        }
    }
    segs
}

/// 片段的源域读取时长(ms,f64;调用方决定取整)= ∫ speed dt 的分段积分。
/// 无曲线 = durationMs × speed(与既有语义逐位一致);freezeMs 定格在调用方裁剪。
pub fn source_read_ms(clip: &Clip) -> f64 {
    speed_segments(clip)
        .iter()
        .map(|(a, b, s)| (*b - *a) as f64 * s)
        .sum()
}

/// 片段在播放域前缀 [0, local_ms) 内消耗的源时长(ms)= ∫₀^local speed dt。
/// 切分 sourceIn 换算的唯一入口(BUG-01 收口):engine 不得再做 1:1 平移。
pub fn source_read_ms_upto(clip: &Clip, local_ms: u64) -> f64 {
    let local_ms = local_ms.min(clip.duration_ms);
    speed_segments(clip)
        .iter()
        .map(|&(a, b, s)| (b.min(local_ms) - a) as f64 * s)
        .sum()
}

/// 曲线在播放域 t 的速度值(节点线性插值 + 端点常速外延;归一口径与
/// [`speed_segments`] 一致:排序、钳边、同刻并点)。切点补点用。
fn curve_speed_at(pts: &[SpeedPoint], dur: u64, t: u64) -> f64 {
    if pts.is_empty() {
        return 1.0;
    }
    let mut nodes: Vec<(u64, f64)> = pts.iter().map(|p| (p.at_ms.min(dur), p.speed)).collect();
    nodes.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    nodes.dedup_by(|a, b| a.0 == b.0);
    let last = nodes.last().expect("非空已守卫");
    if t <= nodes[0].0 {
        return nodes[0].1;
    }
    if t >= last.0 {
        return last.1;
    }
    for w in nodes.windows(2) {
        let (t0, v0) = w[0];
        let (t1, v1) = w[1];
        if t >= t0 && t <= t1 && t1 > t0 {
            let k = (t - t0) as f64 / (t1 - t0) as f64;
            return v0 + (v1 - v0) * k;
        }
    }
    last.1
}

/// 切分失败:t 不在片段内部(t ≤ start 或 t ≥ end)。语义与 engine 的
/// `Reject::SplitOutside` 一致,由调用方映射。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitError {
    pub clip_id: String,
    pub t_ms: u64,
}

impl Clip {
    /// 切分(BUG-01/02 深模块化):返回 (left, right)。
    /// right.id 由调用方按目标轨重新分配(split_at 不识轨,保持原 id 占位);
    /// 时间字段、sourceIn 换算、曲线/关键帧 rebase、效果归属全部在此裁决。
    pub fn split_at(&self, t_ms: u64) -> Result<(Clip, Clip), SplitError> {
        let end = self.start_ms + self.duration_ms;
        if t_ms <= self.start_ms || t_ms >= end {
            return Err(SplitError {
                clip_id: self.id.clone(),
                t_ms,
            });
        }
        let offset = t_ms - self.start_ms;
        let mut left = self.clone();
        let mut right = self.clone();
        left.duration_ms = offset;
        right.start_ms = t_ms;
        right.duration_ms = self.duration_ms - offset;

        // 1. sourceIn 换算(BUG-01;单一真相源 = 分段积分,含 speed/curve)
        match self.source_in_ms {
            Some(si) if self.reverse == Some(true) => {
                // 倒放("reverse 先于变速"):窗口 [si, si+D) 自尾向头播放。
                // 右段持窗口头 [si, si+D-R(o)) 入点不变;左段持窗口尾
                // [si+D-R(o), si+D) 入点前移——两段各自倒放拼接 = 原倒放窗口。
                let head = source_read_ms(self) - source_read_ms_upto(self, offset);
                right.source_in_ms = Some(si);
                left.source_in_ms = Some(si + head.round() as u64);
            }
            Some(si) => {
                right.source_in_ms = Some(si + source_read_ms_upto(self, offset).round() as u64);
            }
            // 无素材坐标知识(文本/图片等):两侧均不臆造
            None => {}
        }

        // 2. speedCurve rebase:左右各持本域曲线;切点补插值点(曲线在该处的
        //    精确值),保证两段的分段积分与原曲线逐段一致(SPLIT-003 解析解对拍)
        if let Some(pts) = &self.speed_curve {
            let cut = curve_speed_at(pts, self.duration_ms, offset);
            let mut lp: Vec<SpeedPoint> =
                pts.iter().filter(|p| p.at_ms <= offset).copied().collect();
            if lp.last().map(|p| p.at_ms) != Some(offset) {
                lp.push(SpeedPoint {
                    at_ms: offset,
                    speed: cut,
                });
            }
            let mut rp: Vec<SpeedPoint> = vec![SpeedPoint {
                at_ms: 0,
                speed: cut,
            }];
            rp.extend(
                pts.iter()
                    .filter(|p| p.at_ms > offset)
                    .map(|&p| SpeedPoint {
                        at_ms: p.at_ms - offset,
                        speed: p.speed,
                    }),
            );
            left.speed_curve = Some(lp);
            right.speed_curve = Some(rp);
        }

        // 3. 关键帧 rebase(BUG-02):左留 t<offset,右 rebase 减 offset 并丢负帧;
        //    帧时间以片段局部播放域存储,右段起点已变为 t_ms
        if let Some(kfs) = &self.keyframes {
            let lk: Vec<Keyframe> = kfs.iter().filter(|k| k.time_ms < offset).cloned().collect();
            let rk: Vec<Keyframe> = kfs
                .iter()
                .filter(|k| k.time_ms >= offset)
                .map(|k| {
                    let mut k = k.clone();
                    k.time_ms -= offset;
                    k
                })
                .collect();
            left.keyframes = if lk.is_empty() { None } else { Some(lk) };
            right.keyframes = if rk.is_empty() { None } else { Some(rk) };
        }

        // 4. 效果归属(BUG-02):fade_in/motion.in 只留左,fade_out/motion.out 只留
        //    右,transition 只留左,片尾定格只留右,文本只留左
        left.fade = self.fade.and_then(|f| {
            norm_fade(Fade {
                in_ms: f.in_ms,
                out_ms: 0.0,
            })
        });
        right.fade = self.fade.and_then(|f| {
            norm_fade(Fade {
                in_ms: 0.0,
                out_ms: f.out_ms,
            })
        });
        left.motion = motion_side(self.motion.clone(), true);
        right.motion = motion_side(self.motion.clone(), false);
        right.transition = None; // transition_in 只留左(防切点双重转场)
        left.freeze_ms = None; // 片尾定格归属右段
        right.text = None; // 文本归属左段(既有语义)
        Ok((left, right))
    }

    /// 合并 right 入 self(BUG-04):语义连续性谓词 + [`Clip::split_at`] 的精确逆。
    /// 失败返回带机读标记的错误串(MergeDifferentSource/MergeDifferentRate/
    /// MergeNotContiguous),engine 映射为 InvariantViolation。
    /// 连续性口径:同 src;同 speed/reverse(speedCurve 允许两侧不同——切分后
    /// 左右各持本域曲线,由内容窗积分校验兜底);sourceIn 按「播放域→源域」
    /// 积分相接(±1ms;任一侧无 sourceIn 视为无素材坐标知识,跳过该项)。
    pub fn merge_with(mut self, right: &Clip) -> Result<Clip, String> {
        if self.src != right.src {
            return Err(format!(
                "clip_merge 拒绝(MergeDifferentSource):{} 与 {} 源不同({:?} vs {:?}),不相干内容不得焊成一段",
                self.id, right.id, self.src, right.src
            ));
        }
        if self.speed != right.speed || self.reverse != right.reverse {
            return Err(format!(
                "clip_merge 拒绝(MergeDifferentRate):{} 与 {} speed/reverse 不同,变速拼接请用 speed 关键帧",
                self.id, right.id
            ));
        }
        let left_dur = self.duration_ms;
        if let (Some(lsi), Some(rsi)) = (self.source_in_ms, right.source_in_ms) {
            let expect = if self.reverse == Some(true) {
                lsi as f64 - source_read_ms(right) // 倒放:右窗头 = 左窗头 - 右窗长
            } else {
                lsi as f64 + source_read_ms(&self) // 顺放:右入点 = 左入点 + 左读长
            };
            if (rsi as f64 - expect).abs() > 1.0 {
                return Err(format!(
                    "clip_merge 拒绝(MergeNotContiguous):{} 与 {} 内容窗不连续(right sourceIn {rsi} ≠ 期望 {expect:.0})",
                    self.id, right.id
                ));
            }
        }
        // —— 以下为 split 的逆:字段并回 ——
        self.duration_ms = right.start_ms + right.duration_ms - self.start_ms;
        // 关键帧:right 帧整体平移 left.duration_ms 后并回(书写序仍严格递增)
        match (self.keyframes.as_mut(), right.keyframes.as_ref()) {
            (Some(l), Some(r)) => l.extend(r.iter().map(|k| {
                let mut k = k.clone();
                k.time_ms += left_dur;
                k
            })),
            (None, Some(r)) => {
                self.keyframes = Some(
                    r.iter()
                        .map(|k| {
                            let mut k = k.clone();
                            k.time_ms += left_dur;
                            k
                        })
                        .collect(),
                );
            }
            _ => {}
        }
        // speedCurve 并回(切点合成点去重;仅左侧有曲线时保持左曲线——既有合并语义)
        match (self.speed_curve.as_mut(), right.speed_curve.as_ref()) {
            (Some(l), Some(r)) => {
                l.extend(r.iter().map(|p| SpeedPoint {
                    at_ms: p.at_ms + left_dur,
                    speed: p.speed,
                }));
                l.sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then(a.speed.total_cmp(&b.speed)));
                l.dedup_by(|a, b| a.at_ms == b.at_ms);
            }
            (None, Some(r)) => {
                self.speed_curve = Some(
                    r.iter()
                        .map(|p| SpeedPoint {
                            at_ms: p.at_ms + left_dur,
                            speed: p.speed,
                        })
                        .collect(),
                );
            }
            _ => {}
        }
        self.fade = merge_fade(self.fade, right.fade);
        self.motion = merge_motion(self.motion, right.motion.clone());
        self.transition = self.transition.take().or_else(|| right.transition.clone());
        self.freeze_ms = self.freeze_ms.or(right.freeze_ms);
        if self.text.is_none() {
            self.text = right.text.clone();
        }
        if self.reverse == Some(true) {
            // 倒放:合并窗口头 = 右段入点(左段入点是切分时前移过的窗口尾)
            self.source_in_ms = right.source_in_ms.or(self.source_in_ms);
        }
        Ok(self)
    }
}

/// 全零 fade 归一为 None(切分后左右各自只携带有效侧,byte 面干净)。
fn norm_fade(f: Fade) -> Option<Fade> {
    if f.in_ms == 0.0 && f.out_ms == 0.0 {
        None
    } else {
        Some(f)
    }
}

/// fade 并回:入场取左、出场取右(split 的精确逆;两侧全无 → None)。
fn merge_fade(l: Option<Fade>, r: Option<Fade>) -> Option<Fade> {
    if l.is_none() && r.is_none() {
        return None;
    }
    Some(Fade {
        in_ms: l.map(|f| f.in_ms).unwrap_or(0.0),
        out_ms: r.map(|f| f.out_ms).unwrap_or(0.0),
    })
}

/// motion 侧别裁剪:left=true 留入场字段(in/inMs/inFx),false 留出场字段。
fn motion_side(m: Option<Motion>, left: bool) -> Option<Motion> {
    let mut m = m?;
    if left {
        m.out = None;
        m.out_ms = None;
        m.out_fx = None;
        if m.in_.is_none() && m.in_ms.is_none() && m.in_fx.is_none() {
            None
        } else {
            Some(m)
        }
    } else {
        m.in_ = None;
        m.in_ms = None;
        m.in_fx = None;
        if m.out.is_none() && m.out_ms.is_none() && m.out_fx.is_none() {
            None
        } else {
            Some(m)
        }
    }
}

/// motion 并回:入场取左、出场取右(split 的精确逆)。
fn merge_motion(l: Option<Motion>, r: Option<Motion>) -> Option<Motion> {
    let mut m = Motion::default();
    if let Some(l) = l {
        m.in_ = l.in_;
        m.in_ms = l.in_ms;
        m.in_fx = l.in_fx;
    }
    if let Some(r) = r {
        m.out = r.out;
        m.out_ms = r.out_ms;
        m.out_fx = r.out_fx;
    }
    if m.in_.is_none()
        && m.in_ms.is_none()
        && m.in_fx.is_none()
        && m.out.is_none()
        && m.out_ms.is_none()
        && m.out_fx.is_none()
    {
        None
    } else {
        Some(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn clip_of(v: serde_json::Value) -> Clip {
        serde_json::from_value(v).unwrap()
    }

    /// TC-CORE-SPLIT-010/011/012 的模型级细化:split_at 直接断言 rebase/归属。
    #[test]
    fn split_at_keyframes_fade_transition_model_level() {
        let c = clip_of(json!({
            "id": "V1-001", "src": "a.mp4", "startMs": 1000, "durationMs": 4000,
            "sourceInMs": 8000, "text": "字幕",
            "fade": {"inMs": 300.0, "outMs": 500.0},
            "transition": {"type": "fade"},
            "motion": {"in": "zoomIn", "out": "fadeOut"},
            "keyframes": [
                {"property": "opacity", "timeMs": 1000, "value": 0.0},
                {"property": "opacity", "timeMs": 3000, "value": 1.0}
            ]
        }));
        let (l, r) = c.split_at(3000).unwrap();
        // 关键帧:左留 @1000,右 rebase @3000→@1000(右段局部域,offset=切点-start=2000)
        assert_eq!(l.keyframes.as_ref().unwrap()[0].time_ms, 1000);
        assert_eq!(r.keyframes.as_ref().unwrap()[0].time_ms, 1000);
        assert_eq!(r.keyframes.as_ref().unwrap()[0].value, 1.0);
        // fade/motion/transition/freeze/text 归属
        assert_eq!(
            l.fade,
            Some(Fade {
                in_ms: 300.0,
                out_ms: 0.0
            })
        );
        assert_eq!(
            r.fade,
            Some(Fade {
                in_ms: 0.0,
                out_ms: 500.0
            })
        );
        assert!(
            l.motion.as_ref().unwrap().out.is_none() && l.motion.as_ref().unwrap().in_.is_some()
        );
        assert!(
            r.motion.as_ref().unwrap().in_.is_none() && r.motion.as_ref().unwrap().out.is_some()
        );
        assert!(l.transition.is_some() && r.transition.is_none());
        assert_eq!(l.text.as_deref(), Some("字幕"));
        assert!(r.text.is_none());
        // 时间字段与 sourceIn(1:1 speed:右段 = 8000+2000)
        assert_eq!((l.start_ms, l.duration_ms), (1000, 2000));
        assert_eq!((r.start_ms, r.duration_ms), (3000, 2000));
        assert_eq!(l.source_in_ms, Some(8000));
        assert_eq!(r.source_in_ms, Some(10000));
    }

    /// 越界切分拒绝(t 在片段外)。
    #[test]
    fn split_at_outside_rejected() {
        let c = clip_of(json!({"id": "V1-001", "startMs": 1000, "durationMs": 2000}));
        assert_eq!(c.split_at(1000).unwrap_err().t_ms, 1000, "t == start 拒绝");
        assert_eq!(c.split_at(3000).unwrap_err().t_ms, 3000, "t == end 拒绝");
        assert!(c.split_at(500).is_err());
        assert!(c.split_at(2000).is_ok(), "片段内部合法");
    }

    /// 无 sourceIn(文本/图片类)切分不臆造素材坐标。
    #[test]
    fn split_at_without_source_in_keeps_none() {
        let c = clip_of(json!({
            "id": "T1-001", "startMs": 0, "durationMs": 2000, "text": "hi"
        }));
        let (l, r) = c.split_at(1000).unwrap();
        assert_eq!(l.source_in_ms, None);
        assert_eq!(r.source_in_ms, None);
    }
}
