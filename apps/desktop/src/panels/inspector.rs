//! 检查器:ui-fields 驱动的字段分组(editable 数值字段 → 步进编辑提交
//! `clip_update`;其余只读展示)。字段集单一真相源在内核侧 `/ui-fields`,
//! 壳不维护字段清单——文档 v9 的新组(调色/特效/文本)自动出现为只读行。

use sable::gpui::WeakEntity;
use sable::gpui::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use sable::widgets::prelude::{SpacingTokens, h_flex, v_flex};
use sable::widgets::theme::theme;
use sable::widgets::tokens::FONT_SIZE_CAPTION;

use crate::app::DesktopApp;

/// 数值型字段与步进量(显示顺序即 ui-fields editable 分组内的顺序;
/// 不在表内的字段一律只读展示)。
const NUMERIC: &[(&str, f64)] = &[
    ("startMs", 100.0),
    ("durationMs", 100.0),
    ("sourceInMs", 100.0),
    ("freezeMs", 100.0),
    ("scale", 0.1),
    ("opacity", 0.1),
    ("volume", 0.1),
    ("rotation", 15.0),
    ("pitch", 1.0),
    ("speed", 0.0), // 0 = 乘除模式 ×1.1
];

pub struct InspectorPanel {
    app: WeakEntity<DesktopApp>,
}

impl InspectorPanel {
    pub fn new(app: &Entity<DesktopApp>, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            cx.observe(app, |_, _, cx| cx.notify()).detach();
            Self {
                app: app.downgrade(),
            }
        })
    }

    fn render_clip(&self, clip: &serde_json::Value, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme(cx).colors;
        let clip_id = clip
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?")
            .to_string();
        let src = clip
            .get("src")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();

        let mut rows = v_flex().gap(px(SpacingTokens::XS));
        rows = rows.child(
            div()
                .text_size(px(FONT_SIZE_CAPTION))
                .text_color(colors.text_primary)
                .child(clip_id.clone()),
        );
        rows = rows.child(
            div()
                .text_size(px(FONT_SIZE_CAPTION))
                .text_color(colors.text_secondary)
                .child(src)
                .truncate(),
        );

        // ui-fields editable 分组(文档形状:{editable: {组: [字段…]}, …})
        let ui_fields = {
            let app = self.app.upgrade();
            app.map(|a| a.read(cx).shared.snapshot().ui_fields)
                .unwrap_or(serde_json::Value::Null)
        };
        let groups = ui_fields
            .get("editable")
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();

        for (group, fields) in &groups {
            let Some(field_list) = fields.as_array() else {
                continue;
            };
            rows = rows.child(section_title(group, &colors));
            for field in field_list.iter().filter_map(|f| f.as_str()) {
                let current = clip.get(field).cloned().unwrap_or(serde_json::Value::Null);
                let step = NUMERIC.iter().find(|(f, _)| *f == field).map(|(_, s)| *s);
                match step {
                    Some(step) => {
                        rows = rows.child(numeric_row(
                            field, &current, step, &clip_id, &colors, &self.app,
                        ));
                    }
                    None => {
                        rows = rows.child(readonly_row(field, &current, &colors));
                    }
                }
            }
        }
        rows
    }
}

impl Render for InspectorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme(cx).colors;
        let clip = self
            .app
            .upgrade()
            .and_then(|app| app.read(cx).selected_clip());
        let body = match clip {
            Some(clip) => self.render_clip(&clip, cx).into_any_element(),
            None => div()
                .text_size(px(FONT_SIZE_CAPTION))
                .text_color(colors.text_secondary)
                .child("选中时间轴上的片段以编辑(点选片段 → 检查器;拖拽/裁剪语义在内核)")
                .into_any_element(),
        };
        v_flex().gap(px(SpacingTokens::XS)).child(body)
    }
}

fn section_title(text: &str, colors: &sable::widgets::tokens::ColorTokens) -> impl IntoElement {
    div()
        .mt(px(SpacingTokens::SM))
        .text_size(px(FONT_SIZE_CAPTION))
        .text_color(colors.text_secondary)
        .child(format!("—— {text} ——"))
}

/// 数值字段行:标签 + 当前值 + −/+ 步进(speed 用 ×/÷)。
fn numeric_row(
    field: &str,
    current: &serde_json::Value,
    step: f64,
    clip_id: &str,
    colors: &sable::widgets::tokens::ColorTokens,
    app: &WeakEntity<DesktopApp>,
) -> impl IntoElement {
    let value = current.as_f64().unwrap_or(0.0);
    let shown = if value.fract() == 0.0 && value.abs() < 1e9 {
        format!("{}", value as i64)
    } else {
        format!("{value:.2}")
    };
    let submit_patch = |new_value: f64| -> serde_json::Value {
        // 整数域字段提交整数,其余 f64(内核 ClipPatch schema 校验兜底)
        if matches!(field, "startMs" | "durationMs" | "sourceInMs" | "freezeMs") {
            serde_json::json!(new_value.round() as i64)
        } else {
            serde_json::json!(new_value)
        }
    };
    let (down_label, down_target, up_label, up_target) = if step == 0.0 {
        // speed:乘除 1.1
        ("÷1.1", value / 1.1, "×1.1", value * 1.1)
    } else {
        // 非负域字段(startMs/durationMs 等毫秒类)下限钳 0 —— 表单校验,
        // 越界/重叠仍由内核 GUARD 兜底
        let floor = if matches!(field, "startMs" | "durationMs" | "sourceInMs" | "freezeMs") {
            0.0
        } else {
            f64::NEG_INFINITY
        };
        ("−", (value - step).max(floor), "+", value + step)
    };
    let mk_btn = |id: String, label: String, new_value: f64| {
        let patch = submit_patch(new_value);
        let params = serde_json::json!({ "clipId": clip_id, "patch": { field: patch } });
        let app = app.clone();
        div()
            .id(sable::gpui::ElementId::Name(id.into()))
            .px(px(SpacingTokens::XS))
            .rounded_sm()
            .bg(colors.surface_2)
            .text_size(px(FONT_SIZE_CAPTION))
            .text_color(colors.text_primary)
            .cursor_pointer()
            .hover(|s| s.bg(colors.border_subtle))
            .child(label)
            .on_click(move |_, _, cx: &mut App| {
                if let Some(app_entity) = app.upgrade() {
                    app_entity.update(cx, |app, cx| {
                        app.submit("clip_update", params.clone(), cx);
                    });
                }
            })
    };
    h_flex()
        .gap(px(SpacingTokens::XS))
        .child(
            div()
                .w(px(88.0))
                .text_size(px(FONT_SIZE_CAPTION))
                .text_color(colors.text_secondary)
                .child(field.to_string()),
        )
        .child(
            div()
                .w(px(64.0))
                .text_size(px(FONT_SIZE_CAPTION))
                .text_color(colors.text_primary)
                .child(shown),
        )
        .child(mk_btn(
            format!("insp-{field}-down"),
            down_label.to_string(),
            down_target,
        ))
        .child(mk_btn(
            format!("insp-{field}-up"),
            up_label.to_string(),
            up_target,
        ))
}

/// 只读字段行(转场/动效/特效/调色/文本样式等整对象字段)。
fn readonly_row(
    field: &str,
    current: &serde_json::Value,
    colors: &sable::widgets::tokens::ColorTokens,
) -> impl IntoElement {
    let text = match current {
        serde_json::Value::Null => "(未设置)".to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => {
            let s = serde_json::to_string(other).unwrap_or_default();
            if s.len() > 72 {
                format!("{}…", &s[..72])
            } else {
                s
            }
        }
    };
    h_flex()
        .gap(px(SpacingTokens::XS))
        .child(
            div()
                .w(px(88.0))
                .text_size(px(FONT_SIZE_CAPTION))
                .text_color(colors.text_secondary)
                .child(field.to_string()),
        )
        .child(
            div()
                .flex_1()
                .text_size(px(FONT_SIZE_CAPTION))
                .text_color(colors.text_primary)
                .child(text)
                .truncate(),
        )
}
