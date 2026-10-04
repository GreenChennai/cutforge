//! 设置页(右 dock 第二 tab;剪映对标 I1 的分组 tab 思路):
//! - 剪辑行为:吸附开关(与时间线工具行同源);
//! - 快捷键速查(全量;与 on_key 实装一一对应);
//! - 内核/渲染依赖状态(诚实呈现,不粉饰);
//! - 候内核项如实标注(多选/关键帧/导出对话框等,见 docs/upstream/04)。

use sable::gpui::WeakEntity;
use sable::gpui::prelude::FluentBuilder as _;
use sable::gpui::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use sable::widgets::prelude::{SpacingTokens, v_flex};
use sable::widgets::property_row::{PropertyRow, section};
use sable::widgets::theme::theme;
use sable::widgets::tokens::FONT_SIZE_CAPTION;

use crate::app::DesktopApp;

/// 快捷键速查表(键位/入口 → 动作;与 DesktopApp::on_key 与预览面板实装一一对应)。
const SHORTCUTS: &[(&str, &str)] = &[
    ("空格", "播放 / 暂停(流畅=引擎 · 精确=逐帧)"),
    ("K", "暂停"),
    ("L", "正向倍速循环 1→2→4→1(≠1x 自动静音)"),
    ("Shift+L", "慢放倍速循环 1→0.5→0.25"),
    ("J", "减速方向 4→2→1→0.5→0.25(到 0.25 停)"),
    ("← / →", "步退 / 步进一帧(Shift = 1 秒)"),
    ("Home / End", "跳到开头 / 结尾"),
    ("Esc", "退出沉浸预览"),
    ("S", "在播放头分割选中片段"),
    ("T", "在播放头分割全部轨道"),
    ("D", "复制选中片段到播放头"),
    ("Ctrl+C / Ctrl+X", "复制 / 剪切选中片段"),
    ("Ctrl+V", "粘贴到播放头(源轨)"),
    ("G", "关闭播放头所在空隙"),
    ("Del / Backspace", "删除选中片段"),
    ("Ctrl+Z / Ctrl+Y", "撤销 / 重做"),
    ("+ / −", "时间轴缩放"),
    ("循环按钮", "当前片段 A→B 循环(传输条 ↻)"),
    ("静音按钮", "静音开关(传输条 🔊/🔇;无声卡提示 toast)"),
    ("画质按钮", "流畅(引擎直解码)/ 精确(逐帧 render_frame)"),
    ("截图按钮", "当前帧落 <工程>/screenshots/(预览右下)"),
    ("沉浸按钮", "收起其他面板只留预览(非系统全屏;Esc 退出)"),
    ("拖进度条", "按下拖动直接映射(零动画);松手才 seek"),
    ("拖动片段", "移动(松手提交;拖拽中本地预览)"),
    ("点标尺/轨道空白", "跳转播放头"),
    ("吸附按钮", "开关片段边缘/播放头吸附"),
];

pub struct SettingsPanel {
    app: WeakEntity<DesktopApp>,
}

impl SettingsPanel {
    pub fn new(app: &Entity<DesktopApp>, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            cx.observe(app, |_, _, cx| cx.notify()).detach();
            Self {
                app: app.downgrade(),
            }
        })
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme(cx).colors;
        let app = self.app.upgrade();
        let (snap_on, rev, kernel_status, engine_status, playback_line) = app
            .as_ref()
            .map(|a| {
                let a = a.read(cx);
                let connected = a
                    .shared
                    .connected
                    .load(std::sync::atomic::Ordering::Relaxed);
                // 引擎状态(诚实呈现:I1 降级链的可展开原因落在这里)
                let engine = match (&a.engine_note, a.engine_source.is_some()) {
                    (Some(note), _) => format!("已回落幻灯片({note})"),
                    (None, true) => "正常(引擎播放路径在场)".to_string(),
                    (None, false) => {
                        if a.quality_precise {
                            "正常(精确画质:逐帧 render_frame)".to_string()
                        } else {
                            "未启动(按空格播放时惰性创建)".to_string()
                        }
                    }
                };
                let pb = format!(
                    "画质 {} · 倍速 {}x · 循环 {} · 静音 {}",
                    if a.quality_precise {
                        "精确"
                    } else {
                        "流畅"
                    },
                    crate::app::fmt_speed(a.transport_speed),
                    if a.loop_clip { "开" } else { "关" },
                    if a.user_muted { "开" } else { "关" },
                );
                (
                    a.snap_enabled,
                    a.shared.rev.load(std::sync::atomic::Ordering::Relaxed),
                    if connected {
                        "已连接(内核子进程随壳退出)".to_string()
                    } else {
                        "未连接".to_string()
                    },
                    engine,
                    pb,
                )
            })
            .unwrap_or((
                true,
                0,
                "未连接".to_string(),
                "未启动".to_string(),
                "-".to_string(),
            ));
        let weak = self.app.clone();

        // —— 剪辑行为 ——
        let snap_row = PropertyRow::new("吸附").control(
            div()
                .id("set-snap")
                .px(px(SpacingTokens::SM))
                .py(px(3.0))
                .rounded_sm()
                .text_size(px(FONT_SIZE_CAPTION))
                .cursor_pointer()
                .when(snap_on, |s| {
                    s.bg(colors.accent).text_color(colors.surface_0)
                })
                .when(!snap_on, |s| {
                    s.bg(colors.surface_2)
                        .text_color(colors.text_secondary)
                        .hover(|s| s.bg(colors.border_subtle))
                })
                .child(if snap_on { "开启" } else { "关闭" })
                .on_click(move |_, _, cx: &mut App| {
                    if let Some(app) = weak.upgrade() {
                        app.update(cx, |app, cx| app.toggle_snap(cx));
                    }
                }),
        );
        let behavior = section(
            "剪辑行为",
            v_flex().gap(px(SpacingTokens::XS)).child(snap_row).child(
                PropertyRow::new("吸附对象").control(
                    div()
                        .text_size(px(FONT_SIZE_CAPTION))
                        .text_color(colors.text_secondary)
                        .child("片段边缘 · 工程起点 · 播放头(容差 8px)"),
                ),
            ),
        );

        // —— 内核状态 ——
        let kernel = section(
            "内核与依赖",
            v_flex()
                .gap(px(SpacingTokens::XS))
                .child(
                    PropertyRow::new("内核").control(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(if kernel_status.starts_with("已连接") {
                                colors.success
                            } else {
                                colors.danger
                            })
                            .child(kernel_status),
                    ),
                )
                .child(
                    PropertyRow::new("工程版本").control(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child(format!("rev {rev}")),
                    ),
                )
                .child(
                    PropertyRow::new("帧渲染").control(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child("render_frame(需 CUTFORGE_FFMPEG/FFPROBE/RENDER)"),
                    ),
                ),
        );

        // —— 播放(I1 M3)——
        let engine_ok = !engine_status.starts_with("已回落");
        let playback = section(
            "播放",
            v_flex()
                .gap(px(SpacingTokens::XS))
                .child(
                    PropertyRow::new("引擎状态").control(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(if engine_ok {
                                colors.success
                            } else {
                                colors.danger
                            })
                            .child(engine_status),
                    ),
                )
                .child(
                    PropertyRow::new("当前状态").control(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child(playback_line),
                    ),
                )
                .child(
                    PropertyRow::new("预览口径").control(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child("预览画质,精确画面以导出为准"),
                    ),
                ),
        );

        // —— 快捷键速查 ——
        let mut rows = v_flex().gap(px(SpacingTokens::XS));
        for (keys, action) in SHORTCUTS {
            rows = rows.child(
                PropertyRow::new(*keys).control(
                    div()
                        .text_size(px(FONT_SIZE_CAPTION))
                        .text_color(colors.text_secondary)
                        .child(*action),
                ),
            );
        }
        let shortcuts = section("快捷键", rows);

        // —— 候内核(诚实边界)——
        let pending = section(
            "待内核支持(当前不可用)",
            v_flex().gap(px(SpacingTokens::XS)).child(PropertyRow::new(
                "候后",
            ).control(
                div()
                    .text_size(px(FONT_SIZE_CAPTION))
                    .text_color(colors.text_secondary)
                    .child(
                        "多选与框选 · 关键帧 · 导出预设对话框 · 右键菜单 · 波形图\n详见 docs/upstream/04 内核缺口清单",
                    ),
            )),
        );

        v_flex()
            .id("settings-root")
            .size_full()
            .overflow_y_scroll()
            .p(px(SpacingTokens::SM))
            .gap(px(SpacingTokens::SM))
            .child(behavior)
            .child(playback)
            .child(kernel)
            .child(shortcuts)
            .child(pending)
    }
}
