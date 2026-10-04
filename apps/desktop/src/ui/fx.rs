//! 动效基建(审查报告 v2 §9.7):时长梯度 token + easing 三族 +
//! reduced-motion 全局闸。**本模块是壳内唯一允许出现动效毫秒数的地方**
//! (静态门禁 TC-GATE-004 `desktop-magic-ms` 扫 `Animation::new` 散写,
//! `ui/` 豁免——与裸色值唯一定义点同口径)。
//!
//! # 纪律(照搬 Web 已验收的 60 项微交互纪律,docs/design/micro-interactions.md)
//!
//! 1. 动效**只用于状态变化**(渲染期由状态代际换动画 id 重启,如 preview
//!    zone 角标的 epoch);
//! 2. **拖拽/指针跟随零动画**——指针即真相(scrub/框选/trim 直映射);
//! 3. 一律 transform/opacity 合成器路径(gpui 动画只回调样式,天然满足;
//!    禁止对布局属性做补间);
//! 4. 同屏并发入场 ≤3;循环动画仅豁免"导出不确定进度/断连呼吸点"两处。
//!
//! # reduced-motion 总控
//!
//! 设置页开关 → [`set_reduced_motion`] → 桥接 sable
//! `anim::set_reduced_motion`(库侧一切 `value_at` 直通终值)+ 本模块
//! [`duration`] 收敛(与 Web `motion.css` 同口径:全量压到近 0 直切)。
//!
//! # gpui 接法(gpui 0.2.2)
//!
//! `Stateful::with_animation(id, Animation, move |el, delta| …)`:
//! [`animation`] 把时长 token 与 easing 三族装配成 gpui [`Animation`];
//! 每帧回调对元素施加 opacity/transform refinement。sable `anim` 的
//! `Spring`/`Animated` 属物理层(拖拽惯性),需要时直接用,不经本模块补间。

// 本目录为 token/图标/动效**基建层**:部分槽位与 helper 由第 4 波(逐面板
// 美化,报告 §9.6)按需消费;门禁 clippy -D warnings 下的 dead_code 豁免
// 仅限本目录(与裸色值/字形/散写 ms 的"唯一定义点"豁免同口径)。
#![allow(dead_code)]
use std::time::Duration;

use sable::gpui::{Animation, ElementId};

/// 按压/悬停/把手显现(hover 80ms 提亮一档)。
pub const FX_PRESS: Duration = Duration::from_millis(80);
/// 面板/浮层/分组展开/toast/画质切换。
pub const FX_PANEL: Duration = Duration::from_millis(160);
/// 视图切换/模态弹入/吸附脉冲。
pub const FX_VIEW: Duration = Duration::from_millis(240);

/// reduced-motion 下压到的"直切"时长(gpui Animation 不接受 0 时长,取 1ms)。
const REDUCED_DURATION: Duration = Duration::from_millis(1);

/// 当前是否"减弱动态效果"(设置页总控;桥接 sable 库侧闸)。
pub fn reduced_motion() -> bool {
    sable::widgets::anim::reduced_motion()
}

/// 设置"减弱动态效果"(设置页开关;幂等)。真时库侧 value_at 直通终值,
/// [`duration`] 一律收敛 [`REDUCED_DURATION`]。
pub fn set_reduced_motion(on: bool) {
    sable::widgets::anim::set_reduced_motion(on);
}

/// 实际生效时长(reduced-motion 时收敛为直切)。
pub fn duration(base: Duration) -> Duration {
    if reduced_motion() {
        REDUCED_DURATION
    } else {
        base
    }
}

// ---------------------------------------------------------------------------
// easing 三族(§9.7:入场 ease-out-quart / 位移 ease-in-out / 吸附 spring)
// ---------------------------------------------------------------------------

/// 入场族 ease-out-quart:快进缓出(对应 web `--cf-ease-enter` 家族)。
pub fn ease_out_quart(delta: f32) -> f32 {
    let d = delta.clamp(0.0, 1.0);
    1.0 - (1.0 - d).powi(4)
}

/// 位移族 ease-in-out(三次):状态切换的标准曲线(对应 web `--cf-ease`)。
pub fn ease_in_out(delta: f32) -> f32 {
    let d = delta.clamp(0.0, 1.0);
    if d < 0.5 {
        4.0 * d * d * d
    } else {
        let v = -2.0 * d + 2.0;
        1.0 - v * v * v / 2.0
    }
}

/// 吸附族 spring(轻回弹):吸附脉冲/对齐落点,峰值过冲 ≈8% 后落定
/// (easeOutBack;c1=1.70158 的 1/4 档,克制不廉价)。
pub fn spring_snap(delta: f32) -> f32 {
    const C1: f32 = 0.425_395; // 1.70158 / 4
    const C3: f32 = C1 + 1.0;
    let d = delta.clamp(0.0, 1.0);
    let v = d - 1.0;
    1.0 + C3 * v * v * v + C1 * v * v
}

// ---------------------------------------------------------------------------
// gpui 装配
// ---------------------------------------------------------------------------

/// 装配 gpui [`Animation`]:时长 token(经 reduced-motion 闸)+ easing 函数。
///
/// 用法(与 preview.rs zone 角标同式):
/// `el.with_animation(id, fx::animation(fx::FX_PANEL, fx::ease_out_quart),
///   move |el, delta| el.opacity(delta))`
pub fn animation(base: Duration, easing: fn(f32) -> f32) -> Animation {
    Animation::new(duration(base)).with_easing(easing)
}

/// 动画元素 id(状态代际换 id 即重启动画——"动效只用于状态变化"的落地式)。
pub fn epoch_id(prefix: &'static str, epoch: usize) -> ElementId {
    ElementId::named_usize(prefix, epoch)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 时长梯度与 §9.7 表一致(80/160/240)。
    #[test]
    fn duration_tokens() {
        assert_eq!(FX_PRESS, Duration::from_millis(80));
        assert_eq!(FX_PANEL, Duration::from_millis(160));
        assert_eq!(FX_VIEW, Duration::from_millis(240));
    }

    /// easing 三族端点:f(0)=0、f(1)=1(位移族中点对称)。
    #[test]
    fn easing_endpoints() {
        for f in [ease_out_quart as fn(f32) -> f32, ease_in_out, spring_snap] {
            assert_eq!(f(0.0), 0.0);
            assert!((f(1.0) - 1.0).abs() < 1e-6, "f(1) 应精确落定 1");
            assert!(f(-0.5) >= 0.0 && f(1.5) <= 1.2, "越界输入须钳制");
        }
        assert!((ease_in_out(0.5) - 0.5).abs() < 1e-6, "in-out 中点对称");
    }

    /// spring 族有轻过冲(>1)且不过度(≤1.1);入场族单调不减且 ≤1。
    #[test]
    fn easing_character() {
        let spring_peak = (0..=100)
            .map(|i| spring_snap(i as f32 / 100.0))
            .fold(f32::MIN, f32::max);
        assert!(spring_peak > 1.0, "spring 应有过冲");
        assert!(
            spring_peak <= 1.1,
            "spring 过冲克制(≤1.1),实际 {spring_peak}"
        );
        let mut prev = -1.0_f32;
        for i in 0..=100 {
            let v = ease_out_quart(i as f32 / 100.0);
            assert!(v >= prev, "ease_out_quart 应单调不减");
            assert!(v <= 1.0);
            prev = v;
        }
    }

    /// reduced-motion 闸:duration 收敛直切(1ms),开闭幂等可还原。
    #[test]
    fn reduced_motion_gate() {
        let saved = reduced_motion();
        set_reduced_motion(true);
        assert!(reduced_motion());
        assert_eq!(duration(FX_VIEW), Duration::from_millis(1));
        set_reduced_motion(false);
        assert!(!reduced_motion());
        assert_eq!(duration(FX_VIEW), FX_VIEW);
        set_reduced_motion(saved); // 还原进程态,不污染并行测试
    }

    /// animation() 装配:正常态用 token 时长;reduced 态收敛直切。
    #[test]
    fn animation_assembly() {
        let saved = reduced_motion();
        set_reduced_motion(false);
        assert_eq!(animation(FX_PANEL, ease_out_quart).duration, FX_PANEL);
        set_reduced_motion(true);
        assert_eq!(
            animation(FX_PANEL, ease_out_quart).duration,
            REDUCED_DURATION
        );
        set_reduced_motion(saved);
    }
}
