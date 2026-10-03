//! 检查器:ui-fields 驱动的分组字段(单一真相源在内核 `/ui-fields`)。
//!
//! - 数值字段 → sable NumberField(拖拽/滚轮/键入全交互;实体池稳定复用,
//!   选片变化才重建,避免每帧重建丢编辑态);
//! - 文本字段(text)→ gpui-component TextInput(IME 全支持,Enter 提交
//!   clip_update);
//! - 转场/动效 → 目录快捷按钮 + 时长数值框(整对象 patch);
//! - 开关类(reverse/flip/denoise)→ 分段按钮;
//! - 其余对象字段 → 只读摘要行。
//!
//! 一切编辑以 `clip_update {clipId, patch}` 提交内核;越界/重叠由内核
//! GUARD 兜底(壳零语义)。

use std::collections::HashMap;

use sable::gpui::WeakEntity;
use sable::gpui::prelude::FluentBuilder as _;
use sable::gpui::{
    App, AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use sable::gpui_component::input::{Input, InputEvent, InputState};
use sable::widgets::number_field::NumberField;
use sable::widgets::prelude::{SpacingTokens, h_flex, v_flex};
use sable::widgets::property_row::{PropertyRow, section};
use sable::widgets::theme::theme;
use sable::widgets::tokens::{ColorTokens, FONT_SIZE_CAPTION};

use crate::app::DesktopApp;
use crate::state::Snapshot;

/// 数值字段域表(步长/范围/单位;不在表内的数值字段走缺省档)。
const NUMERIC: &[(&str, f64, f64, f64, &str)] = &[
    // 字段, min, max, step, unit
    ("startMs", 0.0, 3.6e7, 100.0, "ms"),
    ("durationMs", 1.0, 3.6e7, 100.0, "ms"),
    ("sourceInMs", 0.0, 3.6e7, 100.0, "ms"),
    ("freezeMs", 0.0, 60_000.0, 100.0, "ms"),
    ("scale", 0.05, 10.0, 0.1, "×"),
    ("opacity", 0.0, 1.0, 0.1, ""),
    ("volume", 0.0, 2.0, 0.1, ""),
    ("rotation", -360.0, 360.0, 15.0, "°"),
    ("pitch", 0.5, 2.0, 0.1, "×"),
    ("speed", 0.1, 10.0, 0.1, "×"),
];

/// 字段中文名(返回 String:切断对分组表借用的生命周期传播)。
fn field_label(field: &str) -> String {
    match field {
        "startMs" => "起点".to_string(),
        "durationMs" => "时长".to_string(),
        "sourceInMs" => "素材入点".to_string(),
        "freezeMs" => "冻结帧".to_string(),
        "scale" => "缩放".to_string(),
        "opacity" => "不透明度".to_string(),
        "rotation" => "旋转".to_string(),
        "volume" => "音量".to_string(),
        "pitch" => "音调".to_string(),
        "speed" => "速度".to_string(),
        "denoise" => "降噪".to_string(),
        "reverse" => "倒放".to_string(),
        "flip" => "翻转".to_string(),
        "text" => "文本".to_string(),
        "transition" => "转场".to_string(),
        "motion" => "动效".to_string(),
        "speedCurve" => "变速曲线".to_string(),
        "crop" => "裁剪".to_string(),
        "textStyle" => "文本样式".to_string(),
        "huazi" => "花字".to_string(),
        "fx" => "特效".to_string(),
        "grade" => "调色".to_string(),
        other => other.to_string(),
    }
}

/// 数值字段的空值缺省显示(null → 语义缺省)。
fn numeric_default(field: &str) -> f64 {
    match field {
        "scale" | "speed" | "pitch" | "opacity" => 1.0,
        _ => 0.0,
    }
}

pub struct InspectorPanel {
    app: WeakEntity<DesktopApp>,
    /// NumberField 实体池(键 = 字段名;选片变化才重建,保住编辑态)
    fields: HashMap<String, Entity<NumberField>>,
    /// 池所属 clip id
    pool_clip: Option<String>,
    /// 文本编辑器(文本片段才有;选片变化重建)
    text_editor: Option<Entity<InputState>>,
    /// 文本编辑器提交监听挂载标记(避免重复 subscribe)
    text_clip: Option<String>,
}

impl InspectorPanel {
    pub fn new(app: &Entity<DesktopApp>, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            cx.observe(app, |_, _, cx| cx.notify()).detach();
            Self {
                app: app.downgrade(),
                fields: Default::default(),
                pool_clip: None,
                text_editor: None,
                text_clip: None,
            }
        })
    }

    /// 取或建数值字段实体(池稳定;Binding 闭包捕获 clip id)。
    fn numeric_field(
        &mut self,
        field: &'static str,
        clip_id: String,
        app: &WeakEntity<DesktopApp>,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> Entity<NumberField> {
        if let Some(entity) = self.fields.get(field) {
            return entity.clone();
        }
        let get_app = app.clone();
        let set_app = app.clone();
        let (min, max, step, unit) = NUMERIC
            .iter()
            .find(|(f, ..)| *f == field)
            .map(|(_, min, max, step, unit)| (*min, *max, *step, *unit))
            .unwrap_or((0.0, 1e9, 1.0, ""));
        let is_ms = field.ends_with("Ms");
        let binding = sable::widgets::binding::Binding::new(
            move |cx: &App| {
                get_app
                    .upgrade()
                    .and_then(|a| a.read(cx).selected_clip())
                    .and_then(|clip| clip.get(field).and_then(serde_json::Value::as_f64))
                    .unwrap_or_else(|| numeric_default(field))
            },
            move |value: f64, cx: &mut App| {
                if let Some(app) = set_app.upgrade() {
                    let patch = serde_json::json!({ field: if is_ms {
                        serde_json::json!(value.round() as i64)
                    } else {
                        serde_json::json!(value)
                    }});
                    app.update(cx, |app, cx| {
                        app.submit(
                            "clip_update",
                            serde_json::json!({ "clipId": clip_id, "patch": patch }),
                            cx,
                        );
                    });
                }
            },
        );
        let entity = cx.new(|_| {
            NumberField::new(binding)
                .range(min, max)
                .step(step)
                .unit(unit)
                .element_id(sable::gpui::ElementId::Name(
                    format!("insp-{field}-{ix}").into(),
                ))
        });
        self.fields.insert(field.to_string(), entity.clone());
        entity
    }

    /// 取或建嵌套对象数值字段(transition.durMs / motion.inMs 等):get 读
    /// `clip[obj][val]` 缺省 `default`,set 读当前对象 merge 后整对象提交
    /// (TransitionPatch/MotionPatch 按字段合并,壳零语义只搬运)。
    #[allow(clippy::too_many_arguments)]
    fn object_numeric_field(
        &mut self,
        key: &str,
        obj_field: &'static str,
        val_field: &'static str,
        default: f64,
        min: f64,
        max: f64,
        step: f64,
        unit: &'static str,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> Entity<NumberField> {
        if let Some(entity) = self.fields.get(key) {
            return entity.clone();
        }
        let get_app = self.app.clone();
        let set_app = self.app.clone();
        let key_static: &'static str = Box::leak(key.to_string().into_boxed_str());
        let binding = sable::widgets::binding::Binding::new(
            move |cx: &App| {
                get_app
                    .upgrade()
                    .and_then(|a| a.read(cx).selected_clip())
                    .and_then(|clip| {
                        clip.get(obj_field)
                            .and_then(|o| o.get(val_field))
                            .and_then(serde_json::Value::as_f64)
                    })
                    .unwrap_or(default)
            },
            move |value: f64, cx: &mut App| {
                if let Some(app) = set_app.upgrade() {
                    app.update(cx, |app, cx| {
                        // 读当前对象,merge 单字段后整对象提交(patch 语义)
                        let mut obj = app
                            .selected_clip()
                            .and_then(|clip| clip.get(obj_field).cloned())
                            .and_then(|v| v.as_object().cloned())
                            .unwrap_or_default();
                        obj.insert(
                            val_field.to_string(),
                            serde_json::json!(value.round() as i64),
                        );
                        app.submit(
                            "clip_update",
                            serde_json::json!({
                                "clipId": app.selection.clone(),
                                "patch": { obj_field: serde_json::Value::Object(obj) }
                            }),
                            cx,
                        );
                    });
                }
            },
        );
        let entity = cx.new(|_| {
            NumberField::new(binding)
                .range(min, max)
                .step(step)
                .unit(unit)
                .element_id(sable::gpui::ElementId::Name(
                    format!("insp-{key_static}-{ix}").into(),
                ))
        });
        self.fields.insert(key.to_string(), entity.clone());
        entity
    }

    /// 取或建文本编辑器(选片变化即重建并灌入当前文本)。
    fn text_field(
        &mut self,
        clip_id: &str,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let need_rebuild = self.text_clip.as_deref() != Some(clip_id);
        if need_rebuild {
            let initial = text.to_string();
            let state = cx.new(|cx| {
                let mut state = InputState::new(window, cx).placeholder("字幕文本(Enter 提交)");
                state.set_value(initial, window, cx);
                state
            });
            // Enter = 提交 clip_update
            let app = self.app.clone();
            let clip = clip_id.to_string();
            cx.subscribe(&state, move |this, _state, event, cx| {
                if let InputEvent::PressEnter { .. } = event
                    && let Some(app) = app.upgrade()
                {
                    let value = this
                        .text_editor
                        .as_ref()
                        .map(|e| e.read(cx).value().to_string());
                    if let Some(text) = value
                        && !text.is_empty()
                    {
                        app.update(cx, |app, cx| {
                            app.submit(
                                "clip_update",
                                serde_json::json!({
                                    "clipId": clip,
                                    "patch": { "text": text }
                                }),
                                cx,
                            );
                        });
                    }
                }
            })
            .detach();
            self.text_editor = Some(state);
            self.text_clip = Some(clip_id.to_string());
        }
        self.text_editor.clone().expect("text_editor 刚建必有")
    }
}

/// 快照便捷读(Weak 升级失败 → Default)。
fn snapshot_of(app: &WeakEntity<DesktopApp>, cx: &App) -> Snapshot {
    app.upgrade()
        .map(|a| a.read(cx).shared.snapshot())
        .unwrap_or_default()
}

impl Render for InspectorPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme(cx).colors;
        let app = self.app.clone();
        let clip = self.app.upgrade().and_then(|a| a.read(cx).selected_clip());

        let Some(clip) = clip else {
            self.pool_clip = None;
            self.text_editor = None;
            self.text_clip = None;
            return v_flex()
                .size_full()
                .p(px(SpacingTokens::SM))
                .child(
                    div()
                        .text_size(px(FONT_SIZE_CAPTION))
                        .text_color(colors.text_secondary)
                        .child("点击时间轴上的片段以编辑属性"),
                )
                .into_any_element();
        };

        let clip_id = clip
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?")
            .to_string();
        let track_kind = clip
            .get("trackKind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("video")
            .to_string();
        let src = clip
            .get("src")
            .and_then(serde_json::Value::as_str)
            .map(|s| s.rsplit(['/', '\\']).next().unwrap_or(s).to_string())
            .or_else(|| {
                clip.get("text")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();

        // 选片变化 → 清池
        if self.pool_clip.as_deref() != Some(clip_id.as_str()) {
            self.fields.clear();
            self.text_editor = None;
            self.text_clip = None;
            self.pool_clip = Some(clip_id.clone());
        }

        let snap = snapshot_of(&self.app, cx);
        let app_weak = self.app.clone();
        let playhead = app_weak
            .upgrade()
            .map(|a| a.read(cx).playhead_ms)
            .unwrap_or(0);

        // —— 头部:类型徽标 + id + 素材名 ——
        let header = v_flex()
            .gap(px(2.0))
            .child(
                h_flex()
                    .gap(px(SpacingTokens::XS))
                    .child(
                        div()
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded_sm()
                            .bg(colors.accent_muted)
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_primary)
                            .child(kind_name(&track_kind)),
                    )
                    .child(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child(clip_id.clone()),
                    ),
            )
            .child(
                div()
                    .text_size(px(FONT_SIZE_CAPTION + 1.0))
                    .text_color(colors.text_primary)
                    .child(if src.is_empty() { clip_id.clone() } else { src })
                    .truncate(),
            );

        // —— 片段操作行 ——
        let actions = h_flex()
            .gap(px(SpacingTokens::XS))
            .child(action_button(
                "insp-split",
                "✂ 分割@播放头",
                &colors,
                app.clone(),
                {
                    let clip = clip_id.clone();
                    move |_cx: &mut Context<DesktopApp>| {
                        serde_json::json!({ "clipId": clip, "tMs": playhead })
                    }
                },
                "clip_split",
            ))
            .child(action_button(
                "insp-dup",
                "⧉ 副本@播放头",
                &colors,
                app.clone(),
                {
                    let clip = clip_id.clone();
                    move |_: &mut Context<DesktopApp>| {
                        serde_json::json!({ "clipId": clip, "startMs": playhead })
                    }
                },
                "clip_duplicate",
            ))
            .child(action_button(
                "insp-del",
                "✕ 删除",
                &colors,
                app.clone(),
                {
                    let clip = clip_id.clone();
                    move |_: &mut Context<DesktopApp>| serde_json::json!({ "clipId": clip })
                },
                "clip_delete",
            ));

        // —— ui-fields 分组 ——
        let mut rows = v_flex().gap(px(SpacingTokens::XS));
        let mut field_ix = 0usize;
        let groups = snap
            .ui_fields
            .get("editable")
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();

        for (group, fields) in &groups {
            let Some(field_list) = fields.as_array() else {
                continue;
            };
            let mut group_rows = v_flex().gap(px(SpacingTokens::XS));
            for field in field_list.iter().filter_map(|f| f.as_str()) {
                let current = clip.get(field).cloned().unwrap_or(serde_json::Value::Null);
                match field {
                    // 文本片段的文本输入(视频/音频片段没有 text 字段,整组跳过)
                    "text" => {
                        let Some(text) = current.as_str().map(str::to_string) else {
                            continue;
                        };
                        let editor = self.text_field(&clip_id, &text, window, cx);
                        group_rows = group_rows.child(
                            PropertyRow::new(field_label(field)).control(
                                div()
                                    .w_full()
                                    .max_w(px(220.0))
                                    .text_size(px(FONT_SIZE_CAPTION))
                                    .child(Input::new(&editor)),
                            ),
                        );
                    }
                    // 文本样式/花字只对文本片段有意义(text 为 null/缺失都跳过)
                    "textStyle" | "huazi"
                        if clip.get("text").and_then(serde_json::Value::as_str).is_none() =>
                    {
                        continue;
                    }
                    // 转场:类型按钮 + 时长
                    "transition" => {
                        group_rows = group_rows
                            .child(transition_rows(&app, &clip, &snap, &clip_id, &colors));
                        group_rows = group_rows.child(PropertyRow::new("时长").control(
                            self.object_numeric_field(
                                "transition.durMs",
                                "transition",
                                "durMs",
                                500.0,
                                100.0,
                                5000.0,
                                100.0,
                                "ms",
                                field_ix,
                                cx,
                            ),
                        ));
                        field_ix += 1;
                    }
                    // 动效:入/出场目录 + 时长
                    "motion" => {
                        group_rows =
                            group_rows.child(motion_rows(&app, &clip, &snap, &clip_id, &colors));
                        group_rows = group_rows.child(PropertyRow::new("入场时长").control(
                            self.object_numeric_field(
                                "motion.inMs",
                                "motion",
                                "inMs",
                                400.0,
                                0.0,
                                5000.0,
                                100.0,
                                "ms",
                                field_ix,
                                cx,
                            ),
                        ));
                        field_ix += 1;
                        group_rows = group_rows.child(PropertyRow::new("出场时长").control(
                            self.object_numeric_field(
                                "motion.outMs",
                                "motion",
                                "outMs",
                                400.0,
                                0.0,
                                5000.0,
                                100.0,
                                "ms",
                                field_ix,
                                cx,
                            ),
                        ));
                        field_ix += 1;
                    }
                    // 倒放开关
                    "reverse" => {
                        let on = current.as_bool().unwrap_or(false);
                        group_rows =
                            group_rows.child(PropertyRow::new(field_label(field)).control(
                                toggle_button(&app, &clip_id, "reverse", on, "倒放", &colors),
                            ));
                    }
                    // 翻转三态:无/H/V/HV
                    "flip" => {
                        group_rows = group_rows.child(
                            PropertyRow::new(field_label(field)).control(flip_buttons(
                                &app,
                                &clip_id,
                                current.as_str().unwrap_or("none"),
                                &colors,
                            )),
                        );
                    }
                    // 降噪三态
                    "denoise" => {
                        group_rows = group_rows.child(
                            PropertyRow::new(field_label(field)).control(denoise_buttons(
                                &app,
                                &clip_id,
                                current.as_str().unwrap_or("none"),
                                &colors,
                            )),
                        );
                    }
                    other => {
                        let numeric = NUMERIC.iter().find(|(f, ..)| *f == other);
                        if let Some(&(f, _, _, _, _)) = numeric {
                            let entity =
                                self.numeric_field(f, clip_id.clone(), &app_weak, field_ix, cx);
                            field_ix += 1;
                            group_rows = group_rows
                                .child(PropertyRow::new(field_label(other)).control(entity));
                        } else {
                            // 整对象/未知字段 → 只读摘要
                            group_rows = group_rows.child(
                                PropertyRow::new(field_label(other)).control(
                                    div()
                                        .max_w(px(220.0))
                                        .text_size(px(FONT_SIZE_CAPTION))
                                        .text_color(colors.text_secondary)
                                        .child(summarize(&current))
                                        .truncate(),
                                ),
                            );
                        }
                    }
                }
            }
            if !field_list.is_empty() {
                rows = rows.child(section(group.clone(), group_rows));
            }
        }

        v_flex()
            .id("inspector-root")
            .size_full()
            .overflow_y_scroll()
            .p(px(SpacingTokens::SM))
            .gap(px(SpacingTokens::SM))
            .child(header)
            .child(actions)
            .child(rows)
            .into_any_element()
    }
}

/// 片段类型中文名。
fn kind_name(kind: &str) -> &'static str {
    match kind {
        "audio" => "音频",
        "text" => "文本",
        "image" => "图片",
        _ => "视频",
    }
}

/// 只读值摘要。
fn summarize(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "(未设置)".to_string(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(o) => {
            let parts: Vec<String> = o
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| {
                    let v = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => serde_json::to_string(other).unwrap_or_default(),
                    };
                    format!("{k}={v}")
                })
                .collect();
            if parts.is_empty() {
                "(未设置)".to_string()
            } else {
                parts.join(" ")
            }
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// 片段操作按钮(点击 → weak 升级 → params 闭包现算 → submit)。
fn action_button(
    id: &'static str,
    label: &'static str,
    colors: &ColorTokens,
    app: WeakEntity<DesktopApp>,
    params: impl Fn(&mut sable::gpui::Context<DesktopApp>) -> serde_json::Value + 'static,
    tool: &'static str,
) -> sable::gpui::AnyElement {
    div()
        .id(sable::gpui::ElementId::Name(id.into()))
        .px(px(SpacingTokens::XS + 2.0))
        .py(px(3.0))
        .rounded_sm()
        .bg(colors.surface_2)
        .text_size(px(FONT_SIZE_CAPTION))
        .text_color(colors.text_primary)
        .hover(|s| s.bg(colors.border_subtle))
        .cursor_pointer()
        .child(label.to_string())
        .on_click(move |_: &ClickEvent, _, cx: &mut App| {
            if let Some(app) = app.upgrade() {
                app.update(cx, |app, cx| {
                    let params = params(cx);
                    app.submit(tool, params, cx);
                });
            }
        })
        .into_any_element()
}

/// 开关按钮(当前态高亮 accent)。
fn toggle_button(
    app: &WeakEntity<DesktopApp>,
    clip_id: &str,
    field: &'static str,
    on: bool,
    label_on: &str,
    colors: &sable::widgets::tokens::ColorTokens,
) -> impl IntoElement + use<> {
    let weak = app.clone();
    let clip = clip_id.to_string();
    div()
        .id(sable::gpui::ElementId::Name(
            format!("tgl-{clip_id}-{field}").into(),
        ))
        .px(px(SpacingTokens::XS + 2.0))
        .py(px(3.0))
        .rounded_sm()
        .text_size(px(FONT_SIZE_CAPTION))
        .cursor_pointer()
        .when(on, |s| s.bg(colors.accent).text_color(colors.surface_0))
        .when(!on, |s| {
            s.bg(colors.surface_2)
                .text_color(colors.text_secondary)
                .hover(|s| s.bg(colors.border_subtle))
        })
        .child(format!("{label_on} {}", if on { "开" } else { "关" }))
        .on_click(move |_, _, cx: &mut App| {
            if let Some(app) = weak.upgrade() {
                let patch = serde_json::json!({ field: !on });
                app.update(cx, |app, cx| {
                    app.submit(
                        "clip_update",
                        serde_json::json!({ "clipId": clip, "patch": patch }),
                        cx,
                    );
                });
            }
        })
}

/// 翻转三态段选(无/H/V/HV)。
fn flip_buttons(
    app: &WeakEntity<DesktopApp>,
    clip_id: &str,
    current: &str,
    colors: &sable::widgets::tokens::ColorTokens,
) -> impl IntoElement + use<> {
    let options: &[(&str, &str)] = &[("none", "无"), ("h", "水平"), ("v", "垂直"), ("hv", "双向")];
    segmented(app, clip_id, "flip", current, options, colors)
}

/// 降噪三态段选。
fn denoise_buttons(
    app: &WeakEntity<DesktopApp>,
    clip_id: &str,
    current: &str,
    colors: &sable::widgets::tokens::ColorTokens,
) -> impl IntoElement + use<> {
    let options: &[(&str, &str)] = &[("none", "关"), ("low", "低"), ("high", "高")];
    segmented(app, clip_id, "denoise", current, options, colors)
}

/// 通用段选(整对象字符串字段;点击提交 patch {field: value})。
fn segmented(
    app: &WeakEntity<DesktopApp>,
    clip_id: &str,
    field: &'static str,
    current: &str,
    options: &[(&str, &str)],
    colors: &sable::widgets::tokens::ColorTokens,
) -> impl IntoElement + use<> {
    let mut row = h_flex().gap(px(2.0));
    for (value, label) in options {
        let weak = app.clone();
        let clip = clip_id.to_string();
        let value = value.to_string();
        let active = value == current;
        row = row.child(
            div()
                .id(sable::gpui::ElementId::Name(
                    format!("seg-{clip_id}-{field}-{value}").into(),
                ))
                .px(px(SpacingTokens::XS + 1.0))
                .py(px(2.0))
                .rounded_sm()
                .text_size(px(FONT_SIZE_CAPTION))
                .cursor_pointer()
                .when(active, |s| s.bg(colors.accent).text_color(colors.surface_0))
                .when(!active, |s| {
                    s.bg(colors.surface_2)
                        .text_color(colors.text_secondary)
                        .hover(|s| s.bg(colors.border_subtle))
                })
                .child(label.to_string())
                .on_click(move |_, _, cx: &mut App| {
                    if let Some(app) = weak.upgrade() {
                        let patch = serde_json::json!({ field: value });
                        app.update(cx, |app, cx| {
                            app.submit(
                                "clip_update",
                                serde_json::json!({ "clipId": clip, "patch": patch }),
                                cx,
                            );
                        });
                    }
                }),
        );
    }
    row
}

/// 转场行:当前类型 + 快捷类型按钮。
fn transition_rows(
    app: &WeakEntity<DesktopApp>,
    clip: &serde_json::Value,
    snap: &Snapshot,
    clip_id: &str,
    colors: &sable::widgets::tokens::ColorTokens,
) -> impl IntoElement + use<> {
    let current = clip.get("transition");
    let cur_type = current
        .and_then(|t| t.get("type"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("(无)")
        .to_string();
    let cur_dur = current
        .and_then(|t| t.get("durMs"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(500.0);

    // 快捷目录:catalogs.transition 里取 基础+图形 分类的前 6 项 + 无
    let mut options: Vec<(String, String)> = vec![("none".into(), "无".into())];
    if let Some(list) = snap
        .catalogs
        .get("transition")
        .and_then(|t| t.get("transitions"))
        .and_then(serde_json::Value::as_array)
    {
        for t in list
            .iter()
            .filter(|t| t.get("category").and_then(serde_json::Value::as_str) != Some("滑动"))
            .take(7)
        {
            if let (Some(id), Some(name)) = (
                t.get("id").and_then(serde_json::Value::as_str),
                t.get("name").and_then(serde_json::Value::as_str),
            ) {
                options.push((id.to_string(), name.to_string()));
            }
        }
    }

    let mut rows = v_flex().gap(px(SpacingTokens::XS));
    rows = rows.child(
        PropertyRow::new("类型").control(
            div()
                .text_size(px(FONT_SIZE_CAPTION))
                .text_color(colors.text_secondary)
                .child(format!("{cur_type} · {cur_dur:.0}ms")),
        ),
    );
    let mut picker = h_flex().gap(px(2.0)).flex_wrap();
    for (value, label) in options {
        let weak = app.clone();
        let clip = clip_id.to_string();
        let active = value == cur_type;
        picker = picker.child(
            div()
                .id(sable::gpui::ElementId::Name(
                    format!("trans-{clip_id}-{value}").into(),
                ))
                .px(px(SpacingTokens::XS + 1.0))
                .py(px(2.0))
                .rounded_sm()
                .text_size(px(FONT_SIZE_CAPTION))
                .cursor_pointer()
                .when(active, |s| s.bg(colors.accent).text_color(colors.surface_0))
                .when(!active, |s| {
                    s.bg(colors.surface_2)
                        .text_color(colors.text_secondary)
                        .hover(|s| s.bg(colors.border_subtle))
                })
                .child(label.clone())
                .on_click(move |_, _, cx: &mut App| {
                    if let Some(app) = weak.upgrade() {
                        let patch = if value == "none" {
                            serde_json::json!({ "transition": { "type": "none" } })
                        } else {
                            serde_json::json!({ "transition": { "type": value, "durMs": cur_dur as i64 } })
                        };
                        app.update(cx, |app, cx| {
                            app.submit(
                                "clip_update",
                                serde_json::json!({ "clipId": clip, "patch": patch }),
                                cx,
                            );
                        });
                    }
                }),
        );
    }
    rows = rows.child(PropertyRow::new("切换").control(picker));
    section("转场", rows)
}

/// 动效行:入/出场快捷按钮(目录中文名)。
fn motion_rows(
    app: &WeakEntity<DesktopApp>,
    clip: &serde_json::Value,
    snap: &Snapshot,
    clip_id: &str,
    colors: &sable::widgets::tokens::ColorTokens,
) -> impl IntoElement + use<> {
    let motion = clip.get("motion");
    let cur_in = motion
        .and_then(|m| m.get("in"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none")
        .to_string();
    let cur_out = motion
        .and_then(|m| m.get("out"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none")
        .to_string();

    let catalog = |dir: &str| -> Vec<(String, String)> {
        let mut out = vec![("none".to_string(), "无".to_string())];
        if let Some(list) = snap
            .catalogs
            .get("fx")
            .and_then(|f| f.get("motion"))
            .and_then(|m| m.get(dir))
            .and_then(serde_json::Value::as_array)
        {
            for m in list {
                if let (Some(id), Some(name)) = (
                    m.get("id").and_then(serde_json::Value::as_str),
                    m.get("name").and_then(serde_json::Value::as_str),
                ) {
                    out.push((id.to_string(), name.to_string()));
                }
            }
        }
        out
    };

    let mk = |dir: &'static str, label: String, current: String| -> sable::gpui::AnyElement {
        let mut picker = h_flex().gap(px(2.0)).flex_wrap();
        for (value, name) in catalog(dir) {
            let weak = app.clone();
            let clip = clip_id.to_string();
            let cur = current.to_string();
            let active = value == *current;
            picker = picker.child(
                div()
                    .id(sable::gpui::ElementId::Name(
                        format!("motion-{clip_id}-{dir}-{value}").into(),
                    ))
                    .px(px(SpacingTokens::XS + 1.0))
                    .py(px(2.0))
                    .rounded_sm()
                    .text_size(px(FONT_SIZE_CAPTION))
                    .cursor_pointer()
                    .when(active, |s| s.bg(colors.accent).text_color(colors.surface_0))
                    .when(!active, |s| {
                        s.bg(colors.surface_2)
                            .text_color(colors.text_secondary)
                            .hover(|s| s.bg(colors.border_subtle))
                    })
                    .child(name)
                    .on_click(move |_, _, cx: &mut App| {
                        if let Some(app) = weak.upgrade()
                            && value != cur
                        {
                            let mut obj = serde_json::Map::new();
                            let (in_v, out_v) = if dir == "in" {
                                (value.clone(), cur.clone())
                            } else {
                                (cur.clone(), value.clone())
                            };
                            if in_v != "none" {
                                obj.insert("in".into(), serde_json::json!(in_v));
                            }
                            if out_v != "none" {
                                obj.insert("out".into(), serde_json::json!(out_v));
                            }
                            app.update(cx, |app, cx| {
                                app.submit(
                                    "clip_update",
                                    serde_json::json!({
                                        "clipId": clip,
                                        "patch": { "motion": obj }
                                    }),
                                    cx,
                                );
                            });
                        }
                    }),
            );
        }
        PropertyRow::new(label).control(picker).into_any_element()
    };

    let mut rows = v_flex().gap(px(SpacingTokens::XS));
    rows = rows.child(mk("in", "入场".to_string(), cur_in.clone()));
    rows = rows.child(mk("out", "出场".to_string(), cur_out));
    section("动效", rows)
}
