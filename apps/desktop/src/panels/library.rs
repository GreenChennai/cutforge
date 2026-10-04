//! 媒体库:media_browse 清单(壳侧滤缓存目录)→ 缩略图卡片网格
//! (media_thumbnail 抽帧,串行拉取防 ffmpeg 风暴),双击卡片 =
//! clip_add 落到播放头(轨道按素材类型路由,首个同类未锁轨)。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use sable::gpui::prelude::FluentBuilder as _;
use sable::gpui::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, Render, RenderImage, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, Window, div, img, px,
};
use sable::widgets::prelude::{SpacingTokens, h_flex, v_flex};
use sable::widgets::theme::theme;
use sable::widgets::tokens::FONT_SIZE_CAPTION;

use sable::gpui_component::input::{Input, InputEvent, InputState};

use crate::app::DesktopApp;
use crate::rpc::Rpc;
use crate::state::Shared;
use crate::ui::icon::Icon;
use crate::ui::theme as cf_theme;

/// 卡片缩略图区尺寸(px)。
const THUMB_H: f32 = 74.0;

pub struct LibraryPanel {
    shared: Arc<Shared>,
    rpc: Arc<Rpc>,
    app: Entity<DesktopApp>,
    /// 搜索框(壳侧文件名过滤,零语义)
    search: Option<Entity<InputState>>,
    query: String,
    /// 分类过滤(None=全部;"video"/"audio"/"image")
    kind_filter: Option<String>,
    /// 已取到的缩略图(path → 位图)
    thumbs: std::collections::HashMap<String, Arc<RenderImage>>,
    /// 待取缩略图的 path 队列(串行消费)
    queue: Vec<String>,
    /// 在途请求的 path(完成前不再派新)
    inflight: Option<String>,
    /// 上次建队列时的 media path 集(变化才重建)
    queued_for: Vec<String>,
}

impl LibraryPanel {
    pub fn new(
        shared: Arc<Shared>,
        rpc: Arc<Rpc>,
        app: &Entity<DesktopApp>,
        window: &mut sable::gpui::Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索文件名"));
        let panel = cx.new(|_| Self {
            shared,
            rpc,
            app: app.clone(),
            search: Some(search.clone()),
            query: String::new(),
            kind_filter: None,
            thumbs: Default::default(),
            queue: Vec::new(),
            inflight: None,
            queued_for: Vec::new(),
        });
        // 搜索变化 → 本地过滤(subscribe 挂在 panel 实体上下文)
        let search_for_sub = search.clone();
        let _ = search_for_sub;
        panel.update(cx, |_, cx| {
            cx.subscribe(
                &search,
                move |this: &mut LibraryPanel, _state, event, cx| {
                    if let InputEvent::Change = event
                        && let Some(editor) = this.search.clone()
                    {
                        this.query = editor.read(cx).value().to_string();
                        cx.notify();
                    }
                },
            )
            .detach();
        });
        let weak = panel.downgrade();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(400))
                    .await;
                let _ = weak.update(cx, |panel, cx| panel.pump(cx));
            }
        })
        .detach();
        panel
    }

    /// 泵:媒体清单变化 → 重建缩略图队列;空闲 → 派下一张(串行)。
    fn pump(&mut self, cx: &mut Context<Self>) {
        let media = self.shared.snapshot().media;
        let paths: Vec<String> = media.iter().map(|m| m.path.clone()).collect();
        if self.queued_for != paths {
            self.queued_for = paths.clone();
            let have: HashSet<&String> = self.thumbs.keys().collect();
            self.queue = paths
                .iter()
                .filter(|p| !have.contains(p) && self.inflight.as_ref() != Some(*p))
                .cloned()
                .collect();
        }
        if self.inflight.is_some() {
            return;
        }
        let Some(path) = self.queue.pop() else {
            return;
        };
        self.inflight = Some(path.clone());
        let rpc = self.rpc.clone();
        cx.spawn(async move |this, cx| {
            let result = fetch_thumbnail(&rpc, &path.clone());
            let _ = this.update(cx, |panel, cx| {
                panel.inflight = None;
                if let Ok((path, image)) = result {
                    panel.thumbs.insert(path, image);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}

/// 拉取缩略图(media_thumbnail → PNG → RenderImage)。
fn fetch_thumbnail(rpc: &Rpc, src: &str) -> Result<(String, Arc<RenderImage>), String> {
    let data = rpc.call(
        "media_thumbnail",
        serde_json::json!({ "src": src, "width": 320 }),
        Duration::from_secs(60),
    )?;
    let file = data
        .get("file")
        .or_else(|| data.get("media"))
        .and_then(serde_json::Value::as_str)
        .ok_or("缩略图响应缺 file 路径")?;
    // 内核给工程内相对路径,须挂工程根再读
    let bytes = std::fs::read(rpc.absolutize(file)).map_err(|e| format!("读缩略图失败:{e}"))?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("解码缩略图失败:{e}"))?;
    let rgba = img.to_rgba8();
    let image =
        png_rgba_to_render_image(&rgba, rgba.width(), rgba.height()).ok_or("缩略图位图非法")?;
    Ok((src.to_string(), image))
}

/// RGBA8 → RenderImage(与 preview 同式)。
fn png_rgba_to_render_image(rgba: &[u8], width: u32, height: u32) -> Option<Arc<RenderImage>> {
    let buffer = image::RgbaImage::from_raw(width, height, rgba.to_vec())?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// 素材类型 → 占位图标(A-09:文本字形 → SVG)。
fn kind_icon(kind: &str) -> Icon {
    match kind {
        "audio" => Icon::Audio,
        "image" => Icon::Image,
        _ => Icon::Video,
    }
}

impl Render for LibraryPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme(cx).colors;
        let snapshot = self.shared.snapshot();
        let added: HashSet<&str> = snapshot
            .clips
            .iter()
            .filter_map(|c| c.get("src").and_then(serde_json::Value::as_str))
            .collect();
        let q = self.query.trim().to_lowercase();
        let kind = self.kind_filter.clone();
        let media: Vec<_> = snapshot
            .media
            .iter()
            .filter(|m| {
                (q.is_empty() || m.name.to_lowercase().contains(&q))
                    && kind.as_deref().is_none_or(|k| m.kind == k)
            })
            .cloned()
            .collect();
        let app_root = self.app.clone();
        let panel_weak = cx.entity().downgrade();

        let mut grid = div()
            .flex()
            .flex_wrap()
            .gap(px(SpacingTokens::SM))
            .content_start();
        if media.is_empty() {
            grid = grid.child(
                div()
                    .text_size(px(FONT_SIZE_CAPTION))
                    .text_color(colors.text_secondary)
                    .child("工程目录无媒体文件(放入 03_assets/ 等目录后会自动列出)"),
            );
        }
        for entry in &media {
            let thumb = self.thumbs.get(&entry.path).cloned();
            let icon = kind_icon(&entry.kind);
            let duration = entry
                .duration_ms
                .map(|d| {
                    let s = d / 1000;
                    format!("{:>2}:{:02}", s / 60, s % 60)
                })
                .unwrap_or_default();
            let path = entry.path.clone();
            let app_root = app_root.clone();
            grid = grid.child(
                div()
                    .id(sable::gpui::ElementId::Name(
                        format!("media-{}", entry.path).into(),
                    ))
                    .w(px(132.0))
                    .rounded_sm()
                    .overflow_hidden()
                    .bg(colors.surface_1)
                    .border_1()
                    .border_color(colors.border_subtle)
                    .cursor_pointer()
                    .hover(|s| s.border_color(colors.accent))
                    .on_click(move |ev, _, cx: &mut App| {
                        // 双击 = 插入时间线(播放头处,同类首轨)
                        if ev.click_count() >= 2 {
                            let path = path.clone();
                            app_root.update(cx, |app, cx| app.insert_media(&path, cx));
                        }
                    })
                    .child(
                        div()
                            .relative()
                            .h(px(THUMB_H))
                            .w_full()
                            .bg(colors.surface_0)
                            .child(match thumb {
                                Some(image) => thumb_view(image).into_any_element(),
                                None => div()
                                    .size_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    // A-09:占位字形 → SVG 图标
                                    .child(icon.icon_at(18.0, colors.text_secondary))
                                    .into_any_element(),
                            })
                            .when(added.contains(entry.path.as_str()), |c| {
                                c.child(
                                    div()
                                        .absolute()
                                        .left(px(0.0))
                                        .top(px(0.0))
                                        .px(px(4.0))
                                        .py(px(1.0))
                                        .rounded_br_sm()
                                        .bg(colors.success.opacity(0.9))
                                        .text_size(px(9.0))
                                        .text_color(colors.surface_0)
                                        .child("已添加"),
                                )
                            })
                            .when(!duration.is_empty(), |c| {
                                c.child(
                                    div()
                                        .absolute()
                                        .right(px(2.0))
                                        .bottom(px(2.0))
                                        .px(px(3.0))
                                        .rounded_sm()
                                        // A-08:时长角标遮罩 → 语义层 MASK(web --cf-mask 同值)
                                        .bg(cf_theme::h(cf_theme::MASK))
                                        .text_size(px(9.0))
                                        .text_color(colors.text_primary)
                                        .child(duration),
                                )
                            }),
                    )
                    .child(
                        div()
                            .px(px(SpacingTokens::XS))
                            .py(px(2.0))
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_primary)
                            .child(entry.name.clone())
                            .truncate(),
                    ),
            );
        }

        v_flex()
            .size_full()
            .child(
                v_flex()
                    .px(px(SpacingTokens::SM))
                    .pt(px(SpacingTokens::XS))
                    .pb(px(SpacingTokens::XS))
                    .gap(px(SpacingTokens::XS))
                    .child(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child(format!("素材 {} 项 · 双击插入到播放头", media.len())),
                    )
                    .child(
                        // 分类 chips(剪映素材库分组入口;壳侧零语义过滤)
                        h_flex().gap(px(2.0)).children(
                            [
                                ("全部", None),
                                ("视频", Some("video")),
                                ("音频", Some("audio")),
                                ("图片", Some("image")),
                            ]
                            .iter()
                            .map(|(label, k)| {
                                let active = self.kind_filter == k.map(str::to_string);
                                let kf = k.map(str::to_string);
                                let panel_weak = panel_weak.clone();
                                div()
                                    .id(sable::gpui::ElementId::Name(
                                        format!("kind-{label}").into(),
                                    ))
                                    .px(px(SpacingTokens::XS + 1.0))
                                    .py(px(1.0))
                                    .rounded_sm()
                                    .text_size(px(FONT_SIZE_CAPTION))
                                    .cursor_pointer()
                                    .when(active, |s| {
                                        s.bg(colors.accent).text_color(colors.surface_0)
                                    })
                                    .when(!active, |s| {
                                        s.bg(colors.surface_2)
                                            .text_color(colors.text_secondary)
                                            .hover(|s| s.bg(colors.border_subtle))
                                    })
                                    .child(*label)
                                    .on_click(move |_, _, cx: &mut App| {
                                        if let Some(panel) = panel_weak.upgrade() {
                                            panel.update(cx, |p, cx| {
                                                p.kind_filter = kf.clone();
                                                cx.notify();
                                            });
                                        }
                                    })
                            }),
                        ),
                    )
                    .when_some(self.search.clone(), |c, search| {
                        c.child(
                            div()
                                .w_full()
                                .text_size(px(FONT_SIZE_CAPTION))
                                .child(Input::new(&search)),
                        )
                    }),
            )
            .child(
                div()
                    .id("library-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(SpacingTokens::SM))
                    .pb(px(SpacingTokens::SM))
                    .child(grid),
            )
    }
}

/// 缩略图铺满卡片(img 元素 Cover 裁剪)。
fn thumb_view(image: Arc<RenderImage>) -> impl IntoElement + use<> {
    img(image).size_full().object_fit(ObjectFit::Cover)
}
