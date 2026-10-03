//! 时间轴宿主:widgets `TimelineView` 挂底部 dock;回调全部转内核意图。
//!
//! 布局 = 左轨道头列(kind 徽标 + 名称 + mute 切换,与视图行高对齐)+
//! 右时间轴视图;底部缩放条(−/+/适配,px_per_second 是壳侧视图参数,
//! 不属时间线语义)。轨道头编辑走 `track_update`(ui-fields trackEditable)。
//!
//! - `on_seek` → 播放头(壳本地)+ 预览请求;
//! - `on_move_clip` → `clip_move {clipId, startMs}`(拖拽吸附由组件做,
//!   碰撞/落点裁决在内核——壳零语义);
//! - `on_select_clip` → 选中(检查器联动)。

use sable::gpui::WeakEntity;
use sable::gpui::prelude::FluentBuilder as _;
use sable::gpui::{
    App, AppContext as _, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use sable::video::model::{ClipId, Timeline, TrackKind};
use sable::widgets::prelude::{SpacingTokens, h_flex, v_flex};
use sable::widgets::theme::theme;
use sable::widgets::timeline_view::{RULER_HEIGHT_PX, TRACK_HEIGHT_PX, TimelineView};
use sable::widgets::tokens::FONT_SIZE_CAPTION;

use crate::app::DesktopApp;

/// 缩放档(pps;60 为 1:1 基准)。
const ZOOM_MIN: f64 = 12.0;
const ZOOM_MAX: f64 = 600.0;
/// 轨道头列宽。
const HEADER_W: f32 = 108.0;

/// 时间轴宿主。
pub struct TimelineHost {
    panel: Entity<TimelineView>,
    app: WeakEntity<DesktopApp>,
}

impl TimelineHost {
    pub fn new(app: &Entity<DesktopApp>, timeline: Entity<Timeline>, cx: &mut App) -> Entity<Self> {
        let weak = app.downgrade();
        let panel = cx.new(|_| {
            TimelineView::new(timeline)
                .on_seek({
                    let weak = weak.clone();
                    move |ms: u64, cx: &mut App| {
                        if let Some(app) = weak.upgrade() {
                            app.update(cx, |app, cx| app.set_playhead(ms, cx));
                        }
                    }
                })
                .on_move_clip({
                    let weak = weak.clone();
                    move |id: ClipId, to_ms: u64, cx: &mut App| {
                        if let Some(app) = weak.upgrade() {
                            let kernel_id = app.read(cx).id_map.get(&id.value()).cloned();
                            if let Some(clip_id) = kernel_id {
                                app.update(cx, |app, cx| {
                                    app.submit(
                                        "clip_move",
                                        serde_json::json!({ "clipId": clip_id, "startMs": to_ms }),
                                        cx,
                                    );
                                });
                            }
                        }
                    }
                })
                .on_select_clip({
                    let weak = weak.clone();
                    move |id: ClipId, cx: &mut App| {
                        if let Some(app) = weak.upgrade() {
                            app.update(cx, |app, cx| app.select_clip(id, cx));
                        }
                    }
                })
        });

        cx.new(|cx| {
            // 根视图每次重投影 notify → 时间轴宿主同步播放头红线
            cx.observe(app, |_, _, cx| cx.notify()).detach();
            Self {
                panel,
                app: app.downgrade(),
            }
        })
    }

    /// 缩放按钮(倍率 ×/÷ 1.3;适配 = 全长铺 700px)。
    fn zoom_button(
        id: &'static str,
        label: &'static str,
        app: &WeakEntity<DesktopApp>,
        colors: &sable::widgets::tokens::ColorTokens,
        mode: Zoom,
    ) -> impl IntoElement + use<> {
        let weak = app.clone();
        div()
            .id(sable::gpui::ElementId::Name(id.into()))
            .px(px(SpacingTokens::XS + 2.0))
            .py(px(2.0))
            .rounded_sm()
            .bg(colors.surface_2)
            .text_size(px(FONT_SIZE_CAPTION))
            .text_color(colors.text_primary)
            .hover(|s| s.bg(colors.border_subtle))
            .cursor_pointer()
            .child(label)
            .on_click(move |_, _, cx: &mut App| {
                if let Some(app) = weak.upgrade() {
                    app.update(cx, |app, cx| {
                        let cur = app.timeline.read(cx).px_per_second;
                        let next = match mode {
                            Zoom::In => (cur * 1.3).min(ZOOM_MAX),
                            Zoom::Out => (cur / 1.3).max(ZOOM_MIN),
                            Zoom::Fit => {
                                let dur = app.duration_ms.max(1_000) as f64 / 1000.0;
                                (700.0 / dur).clamp(ZOOM_MIN, ZOOM_MAX)
                            }
                        };
                        app.timeline.update(cx, |tl, _| tl.px_per_second = next);
                        cx.notify();
                    });
                }
            })
    }
}

#[derive(Clone, Copy)]
enum Zoom {
    In,
    Out,
    Fit,
}

/// 轨道头行(kind 徽标色块 + 名称 + 音轨 mute)。
fn track_header(
    app: &WeakEntity<DesktopApp>,
    id: &str,
    name: &str,
    kind: TrackKind,
    mute: bool,
    locked: bool,
    colors: &sable::widgets::tokens::ColorTokens,
) -> impl IntoElement + use<> {
    let (dot, kind_label) = match kind {
        TrackKind::Video => (colors.accent, "V"),
        TrackKind::Audio => (colors.success, "A"),
        TrackKind::Sticker => (colors.warning, "S"),
        TrackKind::Subtitle => (colors.text_secondary, "T"),
    };
    let weak = app.clone();
    let track_id = id.to_string();
    div()
        .w(px(HEADER_W))
        .h(px(TRACK_HEIGHT_PX))
        .mt(px(SpacingTokens::XS))
        .px(px(SpacingTokens::XS))
        .items_center()
        .gap(px(SpacingTokens::XS))
        .flex()
        .bg(colors.surface_1)
        .rounded_sm()
        .child(
            div()
                .w(px(14.0))
                .h(px(14.0))
                .rounded_sm()
                .bg(dot.opacity(0.8))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(9.0))
                .text_color(colors.surface_0)
                .child(kind_label),
        )
        .child(
            div()
                .flex_1()
                .text_size(px(FONT_SIZE_CAPTION))
                .text_color(if locked {
                    colors.text_secondary
                } else {
                    colors.text_primary
                })
                .child(format!("{}{name}", if locked { "🔒" } else { "" }))
                .truncate(),
        )
        .when(kind == TrackKind::Audio, |c| {
            let weak = weak.clone();
            let track_id = track_id.clone();
            c.child(
                div()
                    .id(sable::gpui::ElementId::Name(format!("mute-{id}").into()))
                    .px(px(3.0))
                    .rounded_sm()
                    .text_size(px(9.0))
                    .cursor_pointer()
                    .when(mute, |s| s.bg(colors.danger).text_color(colors.surface_0))
                    .when(!mute, |s| {
                        s.bg(colors.surface_2)
                            .text_color(colors.text_secondary)
                            .hover(|s| s.bg(colors.border_subtle))
                    })
                    .child("M")
                    .on_click(move |_, _, cx: &mut App| {
                        if let Some(app) = weak.upgrade() {
                            app.update(cx, |app, cx| {
                                app.submit(
                                    "track_update",
                                    serde_json::json!({
                                        "trackId": track_id,
                                        "patch": { "mute": !mute }
                                    }),
                                    cx,
                                );
                            });
                        }
                    }),
            )
        })
}

impl Render for TimelineHost {
    fn render(
        &mut self,
        _window: &mut Window,
        cx: &mut sable::gpui::Context<Self>,
    ) -> impl IntoElement {
        let colors = theme(cx).colors;
        let app = self.app.upgrade();
        let playhead = app.as_ref().map(|a| a.read(cx).playhead_ms).unwrap_or(0);
        let (tracks, pps, duration) = app
            .as_ref()
            .map(|a| {
                let a = a.read(cx);
                let snap = a.shared.snapshot();
                (
                    snap.tracks.clone(),
                    a.timeline.read(cx).px_per_second,
                    a.duration_ms,
                )
            })
            .unwrap_or((Vec::new(), 60.0, 0));
        // 选中同步(裸值反向映射:内核 id → 视图侧 u64)
        let selected = app.as_ref().and_then(|a| {
            let a = a.read(cx);
            let kernel = a.selection.clone()?;
            a.id_map
                .iter()
                .find(|(_, v)| **v == kernel)
                .map(|(vid, _)| *vid)
        });
        self.panel.update(cx, |panel, _| {
            panel.set_playhead(playhead);
            panel.set_selected(selected);
        });

        let app_weak = self.app.clone();
        v_flex()
            .size_full()
            .bg(colors.surface_0)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        // 轨道头列(顶对齐标尺高)
                        v_flex()
                            .w(px(HEADER_W))
                            .flex_shrink_0()
                            .border_r_1()
                            .border_color(colors.border_subtle)
                            .bg(colors.surface_1)
                            .child(div().h(px(RULER_HEIGHT_PX)))
                            .children(tracks.iter().map(|t| {
                                track_header(
                                    &app_weak, &t.id, &t.name, t.kind, t.mute, t.locked, &colors,
                                )
                            })),
                    )
                    .child(div().flex_1().min_w_0().child(self.panel.clone())),
            )
            .child(
                // 缩放条
                h_flex()
                    .h(px(24.0))
                    .px(px(SpacingTokens::SM))
                    .gap(px(SpacingTokens::XS))
                    .items_center()
                    .border_t_1()
                    .border_color(colors.border_subtle)
                    .bg(colors.surface_1)
                    .child(Self::zoom_button(
                        "tl-out",
                        "−",
                        &app_weak,
                        &colors,
                        Zoom::Out,
                    ))
                    .child(Self::zoom_button(
                        "tl-in",
                        "+",
                        &app_weak,
                        &colors,
                        Zoom::In,
                    ))
                    .child(Self::zoom_button(
                        "tl-fit",
                        "适配",
                        &app_weak,
                        &colors,
                        Zoom::Fit,
                    ))
                    .child(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child(format!(
                                "{pps:.0} px/s · 全长 {:.1}s",
                                duration as f64 / 1000.0
                            )),
                    ),
            )
    }
}
