//! 预览监视器:`render_frame` 单帧精确预览(内核 T2.4 工具,同步出帧,
//! 帧缓存键含工作区指纹)。播放头变化 → 请求槽位 → 后台任务渲染 →
//! PNG 解码 RGBA → gpui image 上屏(pixels/player_view 同款桥)。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sable::gpui::{
    App, AppContext as _, Context, Corners, Entity, IntoElement, ParentElement as _, Render,
    RenderImage, Styled as _, Window, canvas, div, px,
};
use sable::widgets::prelude::{SpacingTokens, h_flex, v_flex};
use sable::widgets::theme::theme;
use sable::widgets::tokens::FONT_SIZE_CAPTION;

use crate::rpc::{Rpc, render_timeout};
use crate::state::{PreviewFrame, Shared};

pub struct PreviewPanel {
    shared: Arc<Shared>,
    rpc: Arc<Rpc>,
    image: Option<(u64, Arc<RenderImage>)>,
    /// 正在渲染的 tMs(幂等:同 t 请求不重复出帧)
    rendering: Option<u64>,
}

impl PreviewPanel {
    pub fn new(shared: Arc<Shared>, rpc: Arc<Rpc>, cx: &mut App) -> Entity<Self> {
        let panel = cx.new(|_| Self {
            shared,
            rpc,
            image: None,
            rendering: None,
        });
        let weak = panel.downgrade();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let _ = weak.update(cx, |panel, cx| panel.pump(cx));
            }
        })
        .detach();
        panel
    }

    /// 泵(500ms):取预览请求 → 后台出帧;取完成帧 → 上屏位图。
    fn pump(&mut self, cx: &mut Context<Self>) {
        // 完成帧搬运(shared.preview 由后台任务写入)
        if let Ok(mut guard) = self.shared.preview.lock()
            && let Some(frame) = guard.take()
            && let Some(image) = png_rgba_to_render_image(&frame.rgba, frame.width, frame.height)
        {
            self.image = Some((frame.t_ms, image));
            self.rendering = None;
            cx.notify();
        }
        // 新请求
        let want = self.shared.preview_request.lock().ok().and_then(|r| *r);
        let Some(t_ms) = want else { return };
        if self.rendering == Some(t_ms) || self.image.as_ref().is_some_and(|(t, _)| *t == t_ms) {
            return;
        }
        self.rendering = Some(t_ms);
        let rpc = self.rpc.clone();
        let shared = self.shared.clone();
        cx.background_executor()
            .spawn(async move {
                match render_frame_at(&rpc, t_ms) {
                    Ok(frame) => {
                        *shared.preview.lock().unwrap() = Some(frame);
                    }
                    Err(e) => shared.set_error(format!("render_frame 失败:{e}")),
                }
            })
            .detach();
        cx.notify();
    }
}

impl Render for PreviewPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme(cx).colors;
        let caption = match (&self.image, self.rendering) {
            (Some((t, _)), _) => {
                format!("精确预览 @ {:.2}s(render_frame,单帧)", *t as f64 / 1000.0)
            }
            (None, Some(t)) => format!("出帧中… {:.2}s", t as f64 / 1000.0),
            (None, None) => "预览:◀▶ 或点时间轴移动播放头(单帧渲染)".to_string(),
        };
        let image = self.image.clone();
        v_flex()
            .size_full()
            .bg(colors.surface_0)
            .child(
                h_flex()
                    .px(px(SpacingTokens::SM))
                    .py(px(SpacingTokens::XS))
                    .child(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child(caption),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .child(match image {
                        Some((_t, image)) => frame_view(image).into_any_element(),
                        None => div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child("(尚无预览帧)")
                            .into_any_element(),
                    }),
            )
    }
}

/// 位图铺满元素(canvas paint 阶段 paint_image;story/player_view 同款)。
fn frame_view(image: Arc<RenderImage>) -> impl IntoElement + use<> {
    div().relative().size_full().child(
        canvas(
            move |_, _, _| {},
            move |bounds, _, window, _| {
                let _ = window.paint_image(bounds, Corners::all(px(0.0)), image.clone(), 0, false);
            },
        )
        .size_full(),
    )
}

/// 后台出帧:render_frame → 响应里找 PNG 路径 → 读文件 → RGBA。
fn render_frame_at(rpc: &Rpc, t_ms: u64) -> Result<PreviewFrame, String> {
    let data = rpc.call(
        "render_frame",
        serde_json::json!({ "tMs": t_ms }),
        render_timeout("render_frame"),
    )?;
    let path = find_png_path(&data).ok_or("响应中未找到 PNG 路径")?;
    let bytes = std::fs::read(PathBuf::from(&path)).map_err(|e| format!("读帧文件失败:{e}"))?;
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
