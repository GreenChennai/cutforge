//! 预览监视器:双通道出帧(I1,docs/tickets/I1-S2)——
//! - **流畅(引擎)**:播放泵(16ms)写 `shared.engine_frame` → 本面板上屏;
//! - **精确(幻灯片)**:`render_frame` 单帧精确预览(内核 T2.4 工具,同步出帧,
//!   帧缓存键含工作区指纹)。播放头变化/工程 rev 变化 → 请求槽位 → 后台
//!   渲染 → PNG 解码 RGBA → gpui image 上屏(pixels/player_view 同款桥)。
//!
//! 视觉/交互(NLE 惯例):
//! - 取景区 = 纯黑画布 + 细边框,帧等比居中(canvas paint 阶段算目的矩形);
//! - 右下小按钮组(画质/截图/沉浸)+ zone「预览渲染中」角标(160ms 淡入 /
//!   240ms 淡出,动效只出现在状态变化);
//! - 传输条 = 左时间码(当前大字/总长小字)· 居中 ⏮◀▶/⏸▶⏭ + 循环/静音
//!   (播放态高亮)· 右倍速挡 + 出帧状态徽标;
//! - 进度条 = 传输条上方 4px 通栏,按下拖动(拖拽零动画直接映射),
//!   **松手才 seek**(引擎 seek = 重启流,约百 ms)。

use std::sync::Arc;
use std::time::Duration;

use sable::gpui::WeakEntity;
use sable::gpui::prelude::FluentBuilder as _;
use sable::gpui::{
    Animation, AnimationExt as _, App, AppContext as _, Context, Corners, ElementId, Entity,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, RenderImage,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, px,
};
use sable::widgets::prelude::{SpacingTokens, h_flex, v_flex};
use sable::widgets::theme::theme;
use sable::widgets::tokens::{FONT_SIZE_CAPTION, FONT_SIZE_HEADING};

use crate::app::DesktopApp;
use crate::rpc::{Rpc, render_timeout};
use crate::state::{PreviewFrame, Shared};

/// 渲染卡死自愈上限(超过即放弃该帧;render_timeout 300s 是内核上限,
/// 壳侧 60s 还没回包基本=挂了)。
const RENDER_STUCK_SECS: u64 = 60;
/// 传输按钮尺寸。
const TP_BTN: f32 = 30.0;

pub struct PreviewPanel {
    shared: Arc<Shared>,
    rpc: Arc<Rpc>,
    app: WeakEntity<DesktopApp>,
    image: Option<PreviewShot>,
    /// 正在渲染的 tMs(单飞;更新的请求挂 pending)
    rendering: Option<(u64, std::time::Instant)>,
    /// 渲染中被更新的请求(完成后立刻补一发出帧)
    pending: Option<u64>,
    /// 进度条 track 区 bounds(prepaint 回写;拖拽 seek 换算基准)
    progress_bounds: std::rc::Rc<std::cell::Cell<sable::gpui::Bounds<sable::gpui::Pixels>>>,
    /// 已上屏引擎帧序号(去重;播放泵与 120ms 泵都会触发 pump)
    last_engine_seq: Option<u64>,
    /// zone 角标当前态(与 app.zone_loading 对账)
    badge_shown: bool,
    /// 角标动效代际(状态变化即 +1,换动画元素 id 重启淡入/淡出)
    badge_epoch: usize,
}

/// 一帧已解码的预览(时间点 + 原始宽高 + GPU 位图)。
#[derive(Clone)]
struct PreviewShot {
    t_ms: u64,
    dims: (u32, u32),
    image: Arc<RenderImage>,
}

impl PreviewPanel {
    /// 上屏一帧并**驱逐上一帧的图集纹理**:gpui 0.2.2 的 RetainAllImageCache
    /// 只进不出,逐帧新建 RenderImage 不驱逐 = 每帧泄漏一个帧缓冲
    /// (实测 30fps 下 ~90MB/s,75s 涨到 7GB)。保活当前帧 + 待驱逐的上一帧。
    fn install_image(&mut self, cx: &mut Context<Self>, shot: PreviewShot) {
        if let Some(prev) = self.image.take() {
            cx.drop_image(prev.image, None);
        }
        self.image = Some(shot);
    }
    pub fn new(
        shared: Arc<Shared>,
        rpc: Arc<Rpc>,
        app: &Entity<DesktopApp>,
        cx: &mut App,
    ) -> Entity<Self> {
        let panel = cx.new(|_| Self {
            shared,
            rpc,
            app: app.downgrade(),
            image: None,
            rendering: None,
            pending: None,
            progress_bounds: Default::default(),
            last_engine_seq: None,
            badge_shown: false,
            badge_epoch: 0,
        });
        let weak = panel.downgrade();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let _ = weak.update(cx, |panel, cx| panel.pump(cx));
            }
        })
        .detach();
        panel
    }

    /// 引擎路径在场(DesktopApp 判定;帧由播放泵供给)。
    fn engine_active(&self, cx: &Context<Self>) -> bool {
        self.app
            .upgrade()
            .is_some_and(|a| a.read(cx).engine_active())
    }

    /// 泵(120ms 独立循环入口;此刻 DesktopApp 不在更新中,可回读)。
    pub(crate) fn pump(&mut self, cx: &mut Context<Self>) {
        let engine_on = self.engine_active(cx);
        let zone_loading = self
            .app
            .upgrade()
            .is_some_and(|a| a.read(cx).zone_loading.is_some());
        self.pump_once(cx, engine_on, zone_loading);
    }

    /// 引擎泵入口:宿主(DesktopApp)**正在更新中**调用——gpui 实体借用规则
    /// 禁止此刻回读 DesktopApp(实测回读即 panic),状态由宿主算好传入。
    pub(crate) fn pump_from_host(&mut self, cx: &mut Context<Self>, zone_loading: bool) {
        self.pump_once(cx, true, zone_loading);
    }

    fn pump_once(&mut self, cx: &mut Context<Self>, engine_on: bool, zone_loading: bool) {
        // 引擎帧优先:有新帧直接上屏(序号去重);锁作用域内先取 owned,
        // 出作用域再安装(install_image 需可变借 self,与锁卫兵借用不相交)
        let incoming = if let Ok(mut g) = self.shared.engine_frame.lock()
            && let Some(f) = g.take()
            && self.last_engine_seq != Some(f.seq)
        {
            self.last_engine_seq = Some(f.seq);
            png_rgba_to_render_image(&f.rgba, f.width, f.height).map(|image| PreviewShot {
                t_ms: f.t_ms,
                dims: (f.width, f.height),
                image,
            })
        } else {
            None
        };
        if let Some(shot) = incoming {
            self.install_image(cx, shot);
            cx.notify();
        }
        // zone 角标状态变化 → 换代际重启淡入/淡出(动效只出现在状态变化)
        if zone_loading != self.badge_shown {
            self.badge_shown = zone_loading;
            self.badge_epoch += 1;
            cx.notify();
        }
        // 完成帧搬运(幻灯片路径;引擎在场时帧源是播放泵,跳过)
        if !engine_on {
            // 锁作用域只取走 result;install_image 需可变借 self,须出卫兵作用域
            let result = if let Ok(mut guard) = self.shared.preview_result.lock() {
                guard.take()
            } else {
                None
            };
            if let Some(result) = result {
                let shot = match result {
                    Ok(frame) => png_rgba_to_render_image(&frame.rgba, frame.width, frame.height)
                        .map(|image| PreviewShot {
                            t_ms: frame.t_ms,
                            dims: (frame.width, frame.height),
                            image,
                        }),
                    Err(e) => {
                        self.shared.set_error(format!("render_frame 失败:{e}"));
                        None
                    }
                };
                if let Some(shot) = shot {
                    self.install_image(cx, shot);
                }
                self.rendering = None;
                cx.notify();
            }
            // 卡死自愈
            if let Some((_, started)) = self.rendering
                && started.elapsed() > Duration::from_secs(RENDER_STUCK_SECS)
            {
                self.rendering = None;
            }
            // 请求分派
            let want = self
                .shared
                .preview_request
                .lock()
                .ok()
                .and_then(|mut r| r.take())
                .or(self.pending);
            if let Some(t_ms) = want {
                let have = self.image.as_ref().map(|s| s.t_ms);
                if self.rendering.is_none() && have != Some(t_ms) {
                    self.rendering = Some((t_ms, std::time::Instant::now()));
                    self.pending = None;
                    let rpc = self.rpc.clone();
                    let shared = self.shared.clone();
                    cx.background_executor()
                        .spawn(async move {
                            let result = render_frame_at(&rpc, t_ms);
                            *shared.preview_result.lock().unwrap() = Some(result);
                        })
                        .detach();
                    cx.notify();
                } else if self.rendering.map(|(t, _)| t) != Some(t_ms) {
                    self.pending = Some(t_ms);
                }
            }
        } else {
            // 引擎在场:挂起的出帧请求保留意图,弃源后幻灯片接管补一帧
            let want = self
                .shared
                .preview_request
                .lock()
                .ok()
                .and_then(|mut r| r.take());
            if let Some(t_ms) = want {
                self.pending = Some(t_ms);
            }
            self.rendering = None;
        }
    }

    /// 传输按钮(播放键放大居中;`primary` = accent 底)。
    fn transport_button(
        id: &'static str,
        glyph: &'static str,
        primary: bool,
        big: bool,
        colors: &sable::widgets::tokens::ColorTokens,
        app: &WeakEntity<DesktopApp>,
    ) -> sable::gpui::AnyElement {
        let weak = app.clone();
        let size = if big { TP_BTN + 6.0 } else { TP_BTN };
        let font = if big { 13.0 } else { 11.0 };
        let btn = div()
            .id(id)
            .w(px(size))
            .h(px(size))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .text_size(px(font))
            .on_click(move |_, window, cx: &mut App| {
                if let Some(app) = weak.upgrade() {
                    app.update(cx, |app, cx| match id {
                        "tp-home" => app.set_playhead(0, cx),
                        "tp-end" => app.set_playhead(app.duration_ms, cx),
                        "tp-play" => app.toggle_play(cx),
                        "tp-loop" => app.toggle_loop(cx),
                        "tp-mute" => app.toggle_mute(window, cx),
                        "tp-prev" => {
                            let fps = app.shared.snapshot().fps();
                            let cur = app.playhead_ms;
                            app.set_playhead(cur.saturating_sub(frame_ms(fps)), cx);
                        }
                        _ => {
                            let fps = app.shared.snapshot().fps();
                            let cur = app.playhead_ms;
                            app.set_playhead(cur.saturating_add(frame_ms(fps)), cx);
                        }
                    });
                }
            });
        if primary {
            btn.bg(colors.accent)
                .text_color(colors.surface_0)
                .hover(|s| s.bg(colors.text_secondary))
                .cursor_pointer()
                .child(glyph)
                .into_any_element()
        } else {
            btn.bg(colors.surface_2)
                .text_color(colors.text_primary)
                .hover(|s| s.bg(colors.border_subtle))
                .cursor_pointer()
                .child(glyph)
                .into_any_element()
        }
    }
}

/// 预览右下小按钮(画质/截图/沉浸;状态变化即直切,无补间)。
fn corner_btn(
    id: &'static str,
    label: String,
    active: bool,
    weak: &WeakEntity<DesktopApp>,
    handle: impl Fn(&mut DesktopApp, &mut Context<DesktopApp>) + 'static,
    colors: &sable::widgets::tokens::ColorTokens,
) -> sable::gpui::AnyElement {
    let weak = weak.clone();
    div()
        .id(ElementId::Name(id.into()))
        .px(px(SpacingTokens::XS + 2.0))
        .py(px(2.0))
        .rounded(px(4.0))
        .text_size(px(FONT_SIZE_CAPTION))
        .cursor_pointer()
        .when(active, |s| s.bg(colors.accent).text_color(colors.surface_0))
        .when(!active, |s| {
            s.bg(colors.surface_1)
                .text_color(colors.text_secondary)
                .hover(|s| s.bg(colors.border_subtle))
        })
        .child(label)
        .on_click(move |_, _, cx: &mut App| {
            if let Some(app) = weak.upgrade() {
                app.update(cx, |app, cx| handle(app, cx));
            }
        })
        .into_any_element()
}

/// 进度条窗口 x 坐标 → 工程时刻 ms(track bounds 换算比例;total=0 忽略)。
fn progress_ms(
    weak: &WeakEntity<DesktopApp>,
    bounds_slot: &std::rc::Rc<std::cell::Cell<sable::gpui::Bounds<sable::gpui::Pixels>>>,
    x: sable::gpui::Pixels,
    cx: &App,
) -> Option<u64> {
    let app = weak.upgrade()?;
    let total = app.read(cx).duration_ms;
    let bounds = bounds_slot.get();
    let width = f32::from(bounds.size.width);
    if total == 0 || width <= 0.0 {
        return None;
    }
    let ratio = ((f32::from(x) - f32::from(bounds.origin.x)) / width).clamp(0.0, 1.0);
    Some((ratio * total as f32) as u64)
}

/// 单帧时长(ms;fps 防御 ≤0)。
fn frame_ms(fps: f64) -> u64 {
    if fps <= 0.0 {
        33
    } else {
        (1000.0 / fps).round().max(1.0) as u64
    }
}

impl Render for PreviewPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme(cx).colors;
        let app = self.app.upgrade();
        let (playhead, total, playing) = app
            .as_ref()
            .map(|a| {
                let a = a.read(cx);
                (a.playhead_ms, a.duration_ms, a.playing)
            })
            .unwrap_or((0, 0, false));
        let fps = app
            .as_ref()
            .map(|a| a.read(cx).shared.snapshot().fps())
            .unwrap_or(30.0);
        let engine_on = self.engine_active(cx);
        let zone_loading = self.badge_shown;
        let (quality_precise, speed_eff, muted_eff, loop_on, immersive) = app
            .as_ref()
            .map(|a| {
                let a = a.read(cx);
                (
                    a.quality_precise,
                    a.engine_speed_effective(),
                    a.transport_params().1,
                    a.loop_clip,
                    a.immersive,
                )
            })
            .unwrap_or((false, 1.0, false, false, false));
        let frame_state = if zone_loading {
            "预览渲染中…".to_string()
        } else if engine_on {
            format!("流畅 {}x", crate::app::fmt_speed(speed_eff))
        } else {
            match (&self.image, self.rendering) {
                (Some(_), Some((t, _))) => format!("出帧中 {t}ms"),
                (Some(_), None) => {
                    if quality_precise {
                        "单帧精确".to_string()
                    } else {
                        "幻灯片".to_string()
                    }
                }
                (None, Some((t, _))) => format!("出帧中… {}", fmt_timecode(t)),
                (None, None) => "待出帧".to_string(),
            }
        };
        let image = self.image.clone();
        let weak = self.app.clone();
        let progress = if total > 0 {
            (playhead as f32 / total as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // —— 右下小按钮组(04 差距表 P2:画质/截图/沉浸)——
        let corner_group = h_flex()
            .gap(px(SpacingTokens::XS))
            .child(corner_btn(
                "pv-quality",
                if quality_precise {
                    "画质·精确".to_string()
                } else {
                    "画质·流畅".to_string()
                },
                false,
                &weak,
                |app, cx| app.toggle_quality(cx),
                &colors,
            ))
            .child(corner_btn(
                "pv-screenshot",
                "截图".to_string(),
                false,
                &weak,
                |app, cx| app.screenshot(cx),
                &colors,
            ))
            .child(corner_btn(
                "pv-immersive",
                if immersive {
                    "退出沉浸".to_string()
                } else {
                    "沉浸".to_string()
                },
                immersive,
                &weak,
                |app, cx| app.toggle_immersive(cx),
                &colors,
            ));

        // —— zone 角标(状态变化驱动:160ms 淡入 / 240ms 淡出)——
        let badge = (self.badge_epoch > 0).then(|| {
            let shown = self.badge_shown;
            let dur = if shown { 160 } else { 240 };
            div()
                .px(px(SpacingTokens::XS + 2.0))
                .py(px(2.0))
                .rounded_sm()
                .bg(sable::gpui::black().opacity(0.72))
                .border_1()
                .border_color(colors.border_subtle)
                .text_size(px(FONT_SIZE_CAPTION))
                .text_color(colors.text_primary)
                .child("预览渲染中…")
                .with_animation(
                    ElementId::named_usize("zone-badge", self.badge_epoch),
                    Animation::new(Duration::from_millis(dur)),
                    move |el, delta| el.opacity(if shown { delta } else { 1.0 - delta }),
                )
        });

        v_flex()
            .size_full()
            .bg(colors.surface_0)
            // 松手在任何位置都收尾拖拽(capture 相不要求 hover;拖拽零动画)
            .capture_any_mouse_up({
                let weak = weak.clone();
                move |_, _, cx: &mut App| {
                    if let Some(app) = weak.upgrade()
                        && app.read(cx).scrubbing()
                    {
                        let ms = app.read(cx).playhead_ms;
                        app.update(cx, |app, cx| app.end_scrub(ms, cx));
                    }
                }
            })
            // 拖动进度条:按住移动只更新显示(松手才 seek)
            .on_mouse_move({
                let weak = weak.clone();
                let bounds_slot = self.progress_bounds.clone();
                move |ev: &sable::gpui::MouseMoveEvent, _, cx: &mut App| {
                    let Some(app) = weak.upgrade() else {
                        return;
                    };
                    if !app.read(cx).scrubbing() {
                        return;
                    }
                    if ev.pressed_button == Some(MouseButton::Left)
                        && let Some(ms) = progress_ms(&weak, &bounds_slot, ev.position.x, cx)
                    {
                        app.update(cx, |app, cx| app.scrub_to(ms, cx));
                    }
                }
            })
            // —— 取景区(纯黑画布 + 细边框;右下按钮组与角标浮层)——
            .child(
                div().flex_1().min_h_0().p(px(SpacingTokens::SM)).child(
                    div()
                        .relative()
                        .size_full()
                        .bg(sable::gpui::black())
                        .border_1()
                        .border_color(colors.border_subtle)
                        .rounded(px(4.0))
                        .overflow_hidden()
                        .items_center()
                        .justify_center()
                        .child(match image {
                            Some(shot) => {
                                frame_view(shot.image, Some(shot.dims)).into_any_element()
                            }
                            None => v_flex()
                                .gap(px(SpacingTokens::XS))
                                .items_center()
                                .child(
                                    div()
                                        .text_size(px(22.0))
                                        .text_color(colors.surface_2)
                                        .child("▸"),
                                )
                                .child(
                                    div()
                                        .text_size(px(FONT_SIZE_CAPTION))
                                        .text_color(colors.text_secondary)
                                        .child("移动播放头出帧"),
                                )
                                .into_any_element(),
                        })
                        // 角标(按钮组上方,右下)
                        .children(badge.map(|b| {
                            div()
                                .absolute()
                                .bottom(px(34.0))
                                .right(px(SpacingTokens::SM))
                                .child(b)
                        }))
                        // 右下小按钮组
                        .child(
                            div()
                                .absolute()
                                .bottom(px(SpacingTokens::XS))
                                .right(px(SpacingTokens::SM))
                                .child(corner_group),
                        ),
                ),
            )
            // —— 进度条(按下拖动直接映射,松手才 seek;bounds 由 canvas prepaint 回写)——
            .child(
                div()
                    .id("preview-progress")
                    .w_full()
                    .h(px(10.0))
                    .px(px(SpacingTokens::SM))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, {
                        let weak = weak.clone();
                        let bounds_slot = self.progress_bounds.clone();
                        move |ev: &sable::gpui::MouseDownEvent, _, cx: &mut App| {
                            if let Some(ms) = progress_ms(&weak, &bounds_slot, ev.position.x, cx)
                                && let Some(app) = weak.upgrade()
                            {
                                app.update(cx, |app, cx| {
                                    app.begin_scrub(cx);
                                    app.scrub_to(ms, cx);
                                });
                            }
                        }
                    })
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h_full()
                            .flex()
                            .items_center()
                            .child(
                                canvas(
                                    {
                                        let bounds_slot = self.progress_bounds.clone();
                                        move |bounds: sable::gpui::Bounds<sable::gpui::Pixels>,
                                              _w: &mut Window,
                                              _cx: &mut App| {
                                            bounds_slot.set(bounds);
                                        }
                                    },
                                    |_b: sable::gpui::Bounds<sable::gpui::Pixels>,
                                     _s: (),
                                     _w: &mut Window,
                                     _cx: &mut App| {},
                                )
                                .size_full(),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .w_full()
                                    .h(px(4.0))
                                    .rounded_full()
                                    .bg(colors.surface_2)
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .h_full()
                                            .rounded_full()
                                            .bg(colors.accent)
                                            .w(sable::gpui::relative(progress)),
                                    ),
                            )
                            .child(
                                // 播放头小把手
                                div()
                                    .absolute()
                                    .left(sable::gpui::relative(progress))
                                    .top(px(1.0))
                                    .w(px(8.0))
                                    .h(px(8.0))
                                    .rounded_full()
                                    .bg(colors.text_primary)
                                    .ml(px(-4.0)),
                            ),
                    ),
            )
            // —— 传输控制条 ——
            .child(
                h_flex()
                    .w_full()
                    .h(px(48.0))
                    .px(px(SpacingTokens::SM))
                    .pb(px(SpacingTokens::SM))
                    .gap(px(SpacingTokens::XS))
                    .items_center()
                    .bg(colors.surface_0)
                    // 左:时间码(当前大字 / 总长小字)
                    .child(
                        h_flex()
                            .w(px(170.0))
                            .flex_shrink_0()
                            .gap(px(SpacingTokens::XS))
                            .items_baseline()
                            .child(
                                div()
                                    .text_size(px(FONT_SIZE_HEADING + 2.0))
                                    .text_color(colors.text_primary)
                                    .child(fmt_frame_timecode(playhead, fps)),
                            )
                            .child(
                                div()
                                    .text_size(px(FONT_SIZE_CAPTION))
                                    .text_color(colors.text_secondary)
                                    .child(format!("/ {}", fmt_timecode(total))),
                            ),
                    )
                    // 中:传输按钮组(含循环/静音)
                    .child(
                        h_flex()
                            .flex_1()
                            .justify_center()
                            .gap(px(SpacingTokens::SM))
                            .child(Self::transport_button(
                                "tp-home", "|◀", false, false, &colors, &weak,
                            ))
                            .child(Self::transport_button(
                                "tp-prev", "◀", false, false, &colors, &weak,
                            ))
                            .child(Self::transport_button(
                                "tp-play",
                                if playing { "❚❚" } else { "▶" },
                                true,
                                true,
                                &colors,
                                &weak,
                            ))
                            .child(Self::transport_button(
                                "tp-next", "▶", false, false, &colors, &weak,
                            ))
                            .child(Self::transport_button(
                                "tp-end", "▶|", false, false, &colors, &weak,
                            ))
                            .child(Self::transport_button(
                                "tp-loop", "↻", loop_on, false, &colors, &weak,
                            ))
                            .child(Self::transport_button(
                                "tp-mute",
                                if muted_eff { "🔇" } else { "🔊" },
                                muted_eff,
                                false,
                                &colors,
                                &weak,
                            )),
                    )
                    // 右:倍速挡 + 出帧状态徽标
                    .child(
                        h_flex()
                            .w(px(220.0))
                            .flex_shrink_0()
                            .justify_end()
                            .gap(px(SpacingTokens::XS))
                            .child(
                                div()
                                    .px(px(SpacingTokens::XS + 2.0))
                                    .py(px(2.0))
                                    .rounded_sm()
                                    .bg(colors.surface_1)
                                    .border_1()
                                    .border_color(if (speed_eff - 1.0).abs() > 1e-3 {
                                        colors.accent
                                    } else {
                                        colors.border_subtle
                                    })
                                    .text_size(px(FONT_SIZE_CAPTION))
                                    .text_color(if (speed_eff - 1.0).abs() > 1e-3 {
                                        colors.accent
                                    } else {
                                        colors.text_secondary
                                    })
                                    .child(format!("{}x", crate::app::fmt_speed(speed_eff))),
                            )
                            .child(
                                div()
                                    .px(px(SpacingTokens::XS + 2.0))
                                    .py(px(2.0))
                                    .rounded_sm()
                                    .bg(colors.surface_1)
                                    .border_1()
                                    .border_color(colors.border_subtle)
                                    .text_size(px(FONT_SIZE_CAPTION))
                                    .text_color(colors.text_secondary)
                                    .child(frame_state),
                            ),
                    ),
            )
    }
}

/// 帧级时间码 HH:MM:SS:FF(剪映 P1;fps 防御 ≤0 回落 30)。
fn fmt_frame_timecode(ms: u64, fps: f64) -> String {
    let fps = if fps <= 0.0 { 30.0 } else { fps };
    let total_sec = ms / 1000;
    let ff = ((ms % 1000) as f64 / 1000.0 * fps).floor() as u64;
    let (h, rem) = (total_sec / 3600, total_sec % 3600);
    let (m, sec) = (rem / 60, rem % 60);
    format!("{h:02}:{m:02}:{sec:02}:{ff:02}")
}

/// 时间码 m:ss.t(与 sable fmt_timecode 同式;壳内独立避免 pub 依赖漂移)。
fn fmt_timecode(ms: u64) -> String {
    let total_tenths = ms / 100;
    let tenths = total_tenths % 10;
    let total_seconds = total_tenths / 10;
    let (m, s) = (total_seconds / 60, total_seconds % 60);
    format!("{m}:{s:02}.{tenths}")
}

/// 信箱式取景:canvas paint 阶段按帧宽高比算居中目的矩形再 paint_image。
fn frame_view(image: Arc<RenderImage>, dims: Option<(u32, u32)>) -> impl IntoElement + use<> {
    div().relative().size_full().child(
        canvas(
            move |_, _, _| {},
            move |bounds, _, window, _| {
                let Some((fw, fh)) = dims else { return };
                if fw == 0 || fh == 0 || bounds.size.width <= px(0.0) {
                    return;
                }
                // 等比缩放铺满(取景区已留白,内部不再加边)
                let scale = f32::min(
                    f32::from(bounds.size.width) / fw as f32,
                    f32::from(bounds.size.height) / fh as f32,
                );
                let dw = fw as f32 * scale;
                let dh = fh as f32 * scale;
                let x = f32::from(bounds.origin.x) + (f32::from(bounds.size.width) - dw) / 2.0;
                let y = f32::from(bounds.origin.y) + (f32::from(bounds.size.height) - dh) / 2.0;
                let dest = sable::gpui::Bounds {
                    origin: sable::gpui::point(px(x), px(y)),
                    size: sable::gpui::size(px(dw), px(dh)),
                };
                let _ = window.paint_image(dest, Corners::all(px(0.0)), image.clone(), 0, false);
            },
        )
        .size_full(),
    )
}

/// 后台出帧:render_frame(atMs) → 响应里找 PNG 路径 → 读文件 → RGBA。
fn render_frame_at(rpc: &Rpc, t_ms: u64) -> Result<PreviewFrame, String> {
    let data = rpc.call(
        "render_frame",
        serde_json::json!({ "atMs": t_ms }),
        render_timeout("render_frame"),
    )?;
    let path = find_png_path(&data).ok_or("响应中未找到 PNG 路径")?;
    let bytes = std::fs::read(rpc.absolutize(&path)).map_err(|e| format!("读帧文件失败:{e}"))?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("解码帧失败:{e}"))?;
    let rgba = img.to_rgba8();
    Ok(PreviewFrame {
        t_ms,
        rgba: rgba.to_vec(),
        width: rgba.width(),
        height: rgba.height(),
    })
}

/// 宽容扫描响应:任意层级下以 .png 结尾的字符串值(截图按钮复用)。
pub(crate) fn find_png_path(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => s.ends_with(".png").then(|| s.clone()),
        serde_json::Value::Array(a) => a.iter().find_map(find_png_path),
        serde_json::Value::Object(o) => o.values().find_map(find_png_path),
        _ => None,
    }
}

/// RGBA8(直通 alpha)→ gpui RenderImage 裸帧(image crate 豁免同 sable-canvas "png")。
pub fn png_rgba_to_render_image(rgba: &[u8], width: u32, height: u32) -> Option<Arc<RenderImage>> {
    let buffer = image::RgbaImage::from_raw(width, height, rgba.to_vec())?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}
