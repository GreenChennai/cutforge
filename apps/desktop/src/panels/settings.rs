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
use crate::app::shortcut_rows;
use crate::ui::fx;

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
        let weak_motion = weak.clone();

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

        // —— 外观与动效(§9.7:reduced-motion 总控;fx 全局闸桥接 sable 库侧)——
        let reduced = fx::reduced_motion();
        let motion = section(
            "外观与动效",
            v_flex().gap(px(SpacingTokens::XS)).child(
                PropertyRow::new("减弱动态效果").control(
                    div()
                        .id("set-reduced-motion")
                        .px(px(SpacingTokens::SM))
                        .py(px(3.0))
                        .rounded_sm()
                        .text_size(px(FONT_SIZE_CAPTION))
                        .cursor_pointer()
                        .when(reduced, |s| {
                            s.bg(colors.accent).text_color(colors.surface_0)
                        })
                        .when(!reduced, |s| {
                            s.bg(colors.surface_2)
                                .text_color(colors.text_secondary)
                                .hover(|s| s.bg(colors.border_subtle))
                        })
                        .child(if reduced { "已减弱" } else { "正常" })
                        .on_click(move |_, _, cx: &mut App| {
                            fx::set_reduced_motion(!fx::reduced_motion());
                            if let Some(app) = weak_motion.upgrade() {
                                app.update(cx, |app, cx| {
                                    app.status = format!(
                                        "动效 {}",
                                        if fx::reduced_motion() {
                                            "已减弱"
                                        } else {
                                            "正常"
                                        }
                                    );
                                    cx.notify();
                                });
                            }
                        }),
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

        // —— 快捷键速查(BUG-22:从命令注册表生成,与 on_key 分发同源;按组分节)——
        let rows = shortcut_rows();
        let mut shortcut_sections = Vec::new();
        for group in ["播放", "编辑", "视图"] {
            let mut list = v_flex().gap(px(SpacingTokens::XS));
            let mut any = false;
            for (g, keys, label) in &rows {
                if *g != group {
                    continue;
                }
                any = true;
                list = list.child(
                    PropertyRow::new(keys.join(" / ")).control(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child(*label),
                    ),
                );
            }
            if any {
                shortcut_sections.push(section(format!("快捷键 · {group}"), list));
            }
        }

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
            .child(motion)
            .child(playback)
            .child(kernel)
            .children(shortcut_sections)
            .child(pending)
    }
}
