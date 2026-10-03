//! 桌面壳面板:媒体库 / 预览 / 检查器 / 时间轴宿主。

pub mod inspector;
pub mod library;
pub mod preview;
pub mod timeline;

use sable::gpui::{IntoElement, ParentElement as _, Styled as _, div, px};
use sable::widgets::prelude::SpacingTokens;
use sable::widgets::tokens::ColorTokens;
use sable::widgets::tokens::FONT_SIZE_CAPTION;

/// 面板小节标题(各面板共用)。
#[allow(dead_code)] // 各面板共用小节标题(C-FE4 波次接入)
pub fn section_label(text: &str, colors: &ColorTokens) -> impl IntoElement + use<> {
    div()
        .px(px(SpacingTokens::SM))
        .py(px(SpacingTokens::XS))
        .text_size(px(FONT_SIZE_CAPTION))
        .text_color(colors.text_secondary)
        .child(text.to_string())
}
