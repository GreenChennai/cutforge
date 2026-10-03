//! 媒体库:media_browse 只读清单(缩略图/拖拽导入候 C-FE4;当前双击导入
//! 语义在 Web 壳已有,桌面壳先立清单与信息面)。

use std::sync::Arc;

use sable::gpui::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window, div, px,
};
use sable::widgets::prelude::{SpacingTokens, v_flex};
use sable::widgets::theme::theme;
use sable::widgets::tokens::FONT_SIZE_CAPTION;

use crate::rpc::Rpc;
use crate::state::Shared;

pub struct LibraryPanel {
    shared: Arc<Shared>,
}

impl LibraryPanel {
    pub fn new(shared: Arc<Shared>, rpc: Arc<Rpc>, cx: &mut App) -> Entity<Self> {
        // 一次性拉媒体清单(内核就绪后 1.5s;失败静默——面板留说明)
        let rpc_for_media = rpc.clone();
        let shared_for_media = shared.clone();
        cx.background_executor()
            .spawn(async move {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                if let Ok(data) = rpc_for_media.call(
                    "media_browse",
                    serde_json::json!({}),
                    std::time::Duration::from_secs(20),
                ) {
                    let entries = data
                        .get("entries")
                        .and_then(serde_json::Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    shared_for_media.inner.lock().unwrap().media = entries;
                    shared_for_media
                        .dirty
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
            })
            .detach();
        cx.new(|_| Self { shared })
    }
}

impl Render for LibraryPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme(cx).colors;
        let media = self.shared.snapshot().media;
        let mut list = v_flex().gap(px(SpacingTokens::XS));
        if media.is_empty() {
            list = list.child(
                div()
                    .text_size(px(FONT_SIZE_CAPTION))
                    .text_color(colors.text_secondary)
                    .child("素材清单为空或加载中(导入走 Web 壳/CLI,拖拽导入候 C-FE4)"),
            );
        }
        for (i, entry) in media.iter().enumerate().take(200) {
            let name = entry
                .get("name")
                .or_else(|| entry.get("path"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("(未知素材)")
                .to_string();
            list = list.child(
                div()
                    .id(sable::gpui::ElementId::Name(format!("media-{i}").into()))
                    .px(px(SpacingTokens::SM))
                    .py(px(2.0))
                    .rounded_sm()
                    .text_size(px(FONT_SIZE_CAPTION))
                    .text_color(colors.text_primary)
                    .hover(|s| s.bg(colors.surface_2))
                    .child(name)
                    .truncate(),
            );
        }
        v_flex()
            .size_full()
            .p(px(SpacingTokens::SM))
            .gap(px(SpacingTokens::XS))
            .child(list)
    }
}
