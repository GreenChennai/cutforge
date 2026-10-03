//! 预览监视器:`render_frame` 单帧精确预览(内核 T2.4 工具,同步出帧,
//! 帧缓存键含工作区指纹)。播放头变化/工程 rev 变化 → 请求槽位 → 后台
//! 渲染 → PNG 解码 RGBA → gpui image 上屏(pixels/player_view 同款桥)。
//! 底部传输控制条(⏮◀▶⏸▶⏭ + 时间码)回调 DesktopApp;取景为信箱式
//! 等比缩放(canvas paint 阶段按帧宽高比算目的矩形)。

use std::sync::Arc;
use std::time::Duration;

use sable::gpui::WeakEntity;
use sable::gpui::{
    App, AppContext as _, Context, Corners, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, RenderImage, StatefulInteractiveElement as _, Styled as _, Window,
    canvas, div, px,
};
use sable::widgets::prelude::{SpacingTokens, h_flex, v_flex};
use sable::widgets::theme::theme;
use sable::widgets::tokens::FONT_SIZE_CAPTION;

use crate::app::DesktopApp;
use crate::rpc::{Rpc, render_timeout};
use crate::state::{PreviewFrame, Shared};

/// 渲染卡死自愈上限(超过即放弃该帧;render_timeout 300s 是内核上限,
/// 壳侧 60s 还没回包基本=挂了)。
const RENDER_STUCK_SECS: u64 = 60;

pub struct PreviewPanel {
    shared: Arc<Shared>,
    rpc: Arc<Rpc>,
    app: WeakEntity<DesktopApp>,
    image: Option<PreviewShot>,
    /// 正在渲染的 tMs(单飞;更新的请求挂 pending)
    rendering: Option<(u64, std::time::Instant)>,
    /// 渲染中被更新的请求(完成后立刻补一发出帧)
    pending: Option<u64>,
}

/// 一帧已解码的预览(时间点 + 原始宽高 + GPU 位图)。
#[derive(Clone)]
struct PreviewShot {
    t_ms: u64,
    dims: (u32, u32),
    image: Arc<RenderImage>,
}

impl PreviewPanel {
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

    /// 泵(120ms):完成帧搬运 + 请求分派(单飞 + pending 折叠)。
    fn pump(&mut self, cx: &mut Context<Self>) {
        // 渲染结果搬运(shared.preview_result 由后台任务写入)
        if let Ok(mut guard) = self.shared.preview_result.lock()
            && let Some(result) = guard.take()
        {
            match result {
                Ok(frame) => {
                    if let Some(image) =
                        png_rgba_to_render_image(&frame.rgba, frame.width, frame.height)
                    {
                        self.image = Some(PreviewShot {
                            t_ms: frame.t_ms,
                            dims: (frame.width, frame.height),
                            image,
                        });
                    }
                }
                Err(e) => self.shared.set_error(format!("render_frame 失败:{e}")),
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
    }

    fn transport_button(
        id: &'static str,
        glyph: &'static str,
        colors: &sable::widgets::tokens::ColorTokens,
        app: &WeakEntity<DesktopApp>,
    ) -> impl IntoElement + use<> {
        let weak = app.clone();
        div()
            .id(id)
            .w(px(32.0))
            .h(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .bg(colors.surface_2)
            .text_size(px(11.0))
            .text_color(colors.text_primary)
            .hover(|s| s.bg(colors.border_subtle))
            .cursor_pointer()
            .child(glyph)
            .on_click(move |_, _, cx: &mut App| {
                if let Some(app) = weak.upgrade() {
                    app.update(cx, |app, cx| match id {
                        "tp-home" => app.set_playhead(0, cx),
                        "tp-end" => app.set_playhead(app.duration_ms, cx),
                        "tp-play" => app.toggle_play(cx),
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
            })
    }
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
        let caption = match (&self.image, self.rendering) {
            (Some(shot), _) => format!(
                "{} / {} · 单帧精确预览",
                fmt_timecode(shot.t_ms),
                fmt_timecode(total)
            ),
            (None, Some((t, _))) => format!("出帧中… {}", fmt_timecode(t)),
            (None, None) => "待出帧(移动播放头)".to_string(),
        };
        let image = self.image.clone();
        let weak = self.app.clone();

        v_flex()
            .size_full()
            .bg(colors.surface_0)
            // 取景区
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .bg(colors.surface_0)
                    .child(match image {
                        Some(shot) => frame_view(shot.image, Some(shot.dims)).into_any_element(),
                        None => div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child("(尚无预览帧)")
                            .into_any_element(),
                    }),
            )
            // 传输控制条
            .child(
                h_flex()
                    .w_full()
                    .h(px(36.0))
                    .px(px(SpacingTokens::SM))
                    .gap(px(SpacingTokens::XS))
                    .items_center()
                    .border_t_1()
                    .border_color(colors.border_subtle)
                    .bg(colors.surface_1)
                    .child(Self::transport_button("tp-home", "|◀", &colors, &weak))
                    .child(Self::transport_button("tp-prev", "◀", &colors, &weak))
                    .child(Self::transport_button(
                        "tp-play",
                        if playing { "❚❚" } else { "▶" },
                        &colors,
                        &weak,
                    ))
                    .child(Self::transport_button("tp-next", "▶", &colors, &weak))
                    .child(Self::transport_button("tp-end", "▶|", &colors, &weak))
                    .child(
                        div()
                            .ml(px(SpacingTokens::SM))
                            .text_size(px(FONT_SIZE_CAPTION + 1.0))
                            .text_color(colors.text_primary)
                            .child(format!(
                                "{} / {}",
                                fmt_timecode(playhead),
                                fmt_timecode(total)
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child(caption)
                            .truncate(),
                    ),
            )
    }
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
                // 等比缩放 + 4% 留边
                let scale = f32::min(
                    f32::from(bounds.size.width) / fw as f32,
                    f32::from(bounds.size.height) / fh as f32,
                ) * 0.98;
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

/// 宽容扫描响应:任意层级下以 .png 结尾的字符串值。
fn find_png_path(value: &serde_json::Value) -> Option<String> {
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
