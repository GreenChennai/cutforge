//! 时间轴宿主:widgets `TimelineView` 挂底部 dock;回调全部转内核意图。
//!
//! 布局(剪映对标 docs/upstream/04 T1/T2/T3):
//! - 顶部工具行(横跨全宽,图标为 SVG,见 ui/icon.rs):新建轨 / 撤销 重做 /
//!   分割 副本 删除 冻结帧 / 吸附(视觉态)· 右侧缩放 − 适配 +;
//! - 轨道头列(V/A/T 徽标 + 名称 + 锁/静音,`track_update`);
//! - 片段缩略图条:后台按素材串行抽帧(media_thumbnail,按 src+槽位缓存),
//!   喂 `TimelineView::set_clip_thumbs` 平铺渲染。
//!
//! - `on_seek` → 播放头(壳本地)+ 预览请求;
//! - `on_move_clip` → `clip_move {clipId, startMs}`(拖拽吸附由组件做,
//!   碰撞/落点裁决在内核——壳零语义);
//! - `on_select_clip` → 选中(检查器联动)。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sable::gpui::WeakEntity;
use sable::gpui::prelude::FluentBuilder as _;
use sable::gpui::{
    App, AppContext as _, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    RenderImage, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use sable::video::model::{ClipId, Timeline, TrackKind};
use sable::widgets::prelude::{SpacingTokens, h_flex, v_flex};
use sable::widgets::theme::theme;
use sable::gpui_component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use sable::widgets::timeline_view::{RULER_HEIGHT_PX, TRACK_HEIGHT_PX, TimelineView};
use sable::widgets::tokens::FONT_SIZE_CAPTION;

use crate::app::DesktopApp;
use crate::rpc::Rpc;
use crate::ui::icon::Icon;

/// 缩放档(pps;60 为 1:1 基准)。
const ZOOM_MIN: f64 = 12.0;
const ZOOM_MAX: f64 = 600.0;
/// 轨道头列宽。
const HEADER_W: f32 = 132.0;
/// 每片段抽帧槽位(素材内时长比例点)。
const THUMB_SLOTS: [f64; 4] = [0.12, 0.38, 0.62, 0.88];

/// 时间轴宿主。
pub struct TimelineHost {
    panel: Entity<TimelineView>,
    app: WeakEntity<DesktopApp>,
    /// 缩略图条服务:已请求的 (src, 槽位) 集 + 完成 resultMap(src → 帧)
    thumbs_done: HashMap<String, Vec<Arc<RenderImage>>>,
    thumbs_inflight: HashSet<(String, usize)>,
    /// 上次驱动抽帧的 clips 签名(rev 变化才扫)
    thumb_rev: u64,
}

impl TimelineHost {
    pub fn new(app: &Entity<DesktopApp>, timeline: Entity<Timeline>, cx: &mut App) -> Entity<Self> {
        let weak = app.downgrade();
        // 拖拽乐观预览用(主实体 move 给 TimelineView)
        let timeline_for_drag = timeline.clone();
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
                // 拖拽中:本地乐观预览(改视图模型 start_ms,不提交后端——
                // 逐 move 提交会引发 RPC+全量重拉,拖动一卡一卡的根因)
                .on_move_clip({
                    let timeline = timeline_for_drag;
                    move |id: ClipId, to_ms: u64, cx: &mut App| {
                        timeline.update(cx, |tl, _| {
                            for track in &mut tl.tracks {
                                for clip in &mut track.clips {
                                    if clip.id == id {
                                        clip.start_ms = to_ms;
                                    }
                                }
                            }
                        });
                    }
                })
                // 松手:一次性提交内核(乐观预览的落地帧;rev 回流后重投影对齐)
                .on_drop_clip({
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

        let host = cx.new(|cx| {
            // 根视图每次重投影 notify → 时间轴宿主同步播放头红线
            cx.observe(app, |_, _, cx| cx.notify()).detach();
            Self {
                panel,
                app: app.downgrade(),
                thumbs_done: HashMap::new(),
                thumbs_inflight: HashSet::new(),
                thumb_rev: 0,
            }
        });
        // 抽帧泵:500ms 轮询(rev 变化才扫;串行防 ffmpeg 风暴)
        let weak_host = host.downgrade();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                let _ = weak_host.update(cx, |host, cx| host.pump_thumbs(cx));
            }
        })
        .detach();
        host
    }

    /// 缩略图泵:扫描视频/图片片段 → 缺失槽位串行抽帧 → 完成集喂视图。
    fn pump_thumbs(&mut self, cx: &mut sable::gpui::Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let app_ref = app.read(cx);
        let snap = app_ref.shared.snapshot();
        if snap.rev != self.thumb_rev {
            self.thumb_rev = snap.rev;
            // 清理已不存在的 src
            let live: HashSet<&str> = snap
                .clips
                .iter()
                .filter_map(|c| c.get("src").and_then(serde_json::Value::as_str))
                .collect();
            self.thumbs_done.retain(|k, _| live.contains(k.as_str()));
        }
        // 片段 → src 的槽位需求(缺失槽位计入 inflight,收集派发清单)
        let mut wanted: Vec<(String, usize, u64)> = Vec::new();
        for clip in &snap.clips {
            let kind = clip
                .get("trackKind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("video");
            if kind == "text" {
                continue;
            }
            let Some(src) = clip.get("src").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let dur = snap
                .media
                .iter()
                .find(|m| m.path == src)
                .and_then(|m| m.duration_ms)
                .unwrap_or(4000);
            for (ix, ratio) in THUMB_SLOTS.iter().enumerate() {
                let have = self.thumbs_done.get(src).map(|v| v.len()).unwrap_or(0);
                if have > ix {
                    continue;
                }
                let key = (src.to_string(), ix);
                if self.thumbs_inflight.insert(key) {
                    wanted.push((src.to_string(), ix, (dur as f64 * ratio) as u64));
                }
            }
        }
        // 串行派发(一次一个;完成回填;cx.spawn 模式保 Send)
        if let Some((src, ix, at)) = wanted.first() {
            let (src, ix, at) = (src.clone(), *ix, *at);
            let rpc = app_ref.rpc.clone();
            cx.spawn(async move |this, cx| {
                let result = fetch_thumb_frame(&rpc, &src, at);
                let _ = this.update(cx, |host: &mut TimelineHost, cx| {
                    host.thumbs_inflight.remove(&(src.clone(), ix));
                    if let Ok(image) = result {
                        let vec = host.thumbs_done.entry(src).or_default();
                        if ix < vec.len() {
                            vec[ix] = image;
                        } else {
                            vec.push(image);
                        }
                        host.push_thumbs(cx);
                    }
                });
            })
            .detach();
        }
    }

    /// 完成集 → 视图(键 = 视图侧 ClipId 裸值)。
    fn push_thumbs(&mut self, cx: &mut sable::gpui::Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let map: HashMap<u64, Vec<Arc<RenderImage>>> = app
            .read(cx)
            .id_map
            .iter()
            .filter_map(|(vid, kernel_id)| {
                // 内核片段 id → src(从快照反查)
                let snap = app.read(cx).shared.snapshot();
                let src = snap
                    .clips
                    .iter()
                    .find(|c| {
                        c.get("id").and_then(serde_json::Value::as_str) == Some(kernel_id.as_str())
                    })
                    .and_then(|c| c.get("src").and_then(serde_json::Value::as_str))?;
                self.thumbs_done.get(src).cloned().map(|v| (*vid, v))
            })
            .collect();
        self.panel.update(cx, |panel, _| panel.set_clip_thumbs(map));
        cx.notify();
    }

    /// 缩放按钮(倍率 ×/÷ 1.3;适配 = 全长铺 700px;A-09:SVG 图标)。
    fn zoom_button(
        id: &'static str,
        icon: Icon,
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
            .hover(|s| s.bg(colors.border_subtle))
            .cursor_pointer()
            .child(icon.icon_at(14.0, colors.text_primary))
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

    /// 工具行图标按钮(剪映 T1;SVG 图标 + 可选文字标签)。
    /// `active` = 锁开态(强调底,如吸附);`enabled=false` 置灰。
    fn tool_button(
        id: &'static str,
        icon: Icon,
        label: &'static str,
        enabled: bool,
        active: bool,
        colors: &sable::widgets::tokens::ColorTokens,
    ) -> sable::gpui::Stateful<sable::gpui::Div> {
        let (bg, fg) = if active {
            (colors.accent, colors.surface_0)
        } else if enabled {
            (colors.surface_2, colors.text_primary)
        } else {
            (colors.surface_1, colors.text_secondary)
        };
        let button = div()
            .id(sable::gpui::ElementId::Name(id.into()))
            .px(px(SpacingTokens::XS + 1.0))
            .py(px(2.0))
            .rounded_sm()
            .flex()
            .items_center()
            .gap(px(3.0))
            .bg(bg)
            .child(icon.icon_at(14.0, fg))
            .text_size(px(FONT_SIZE_CAPTION + 1.0))
            .text_color(fg);
        let button = if enabled && !active {
            button
                .hover(|s| s.bg(colors.border_subtle))
                .cursor_pointer()
        } else {
            button
        };
        if label.is_empty() {
            button
        } else {
            button.child(label)
        }
    }
}

/// 拉取单个缩略图帧(media_thumbnail → PNG → RenderImage)。
fn fetch_thumb_frame(rpc: &Rpc, src: &str, at_ms: u64) -> Result<Arc<RenderImage>, String> {
    let data = rpc.call(
        "media_thumbnail",
        serde_json::json!({ "src": src, "atMs": at_ms, "width": 160 }),
        std::time::Duration::from_secs(60),
    )?;
    let file = data
        .get("file")
        .or_else(|| data.get("media"))
        .and_then(serde_json::Value::as_str)
        .ok_or("缩略图响应缺 file 路径")?;
    let bytes = std::fs::read(rpc.absolutize(file)).map_err(|e| format!("读缩略图失败:{e}"))?;
    let decoded = image::load_from_memory(&bytes).map_err(|e| format!("解码缩略图失败:{e}"))?;
    let rgba = decoded.to_rgba8();
    let buffer = image::RgbaImage::from_raw(rgba.width(), rgba.height(), rgba.to_vec())
        .ok_or("缩略图位图非法")?;
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

#[derive(Clone, Copy)]
enum Zoom {
    In,
    Out,
    Fit,
}

/// 轨道头行(kind 徽标色块 + 名称 + 锁/M)。
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
    let weak_lock = app.clone();
    let track_id = id.to_string();
    let track_id_lock = id.to_string();
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
                .child(name.to_string())
                .truncate(),
        )
        // 锁定(剪映 T2;点击 → track_update locked;A-09:字形 → SVG)
        .child(
            div()
                .id(sable::gpui::ElementId::Name(format!("lock-{id}").into()))
                .px(px(3.0))
                .rounded_sm()
                .flex()
                .items_center()
                .cursor_pointer()
                .when(locked, |s| {
                    s.bg(colors.warning)
                        .child(Icon::Lock.icon_at(10.0, colors.surface_0))
                })
                .when(!locked, |s| {
                    s.bg(colors.surface_2)
                        .hover(|s| s.bg(colors.border_subtle))
                        .child(Icon::LockOpen.icon_at(10.0, colors.text_secondary))
                })
                .on_click(move |_, _, cx: &mut App| {
                    if let Some(app) = weak_lock.upgrade() {
                        app.update(cx, |app, cx| {
                            app.submit(
                                "track_update",
                                serde_json::json!({
                                    "trackId": track_id_lock,
                                    "patch": { "locked": !locked }
                                }),
                                cx,
                            );
                        });
                    }
                }),
        )
        // 静音(仅音轨;剪映 M;A-09:字形 → SVG)
        .when(kind == TrackKind::Audio, |c| {
            let weak = weak.clone();
            let track_id = track_id.clone();
            c.child(
                div()
                    .id(sable::gpui::ElementId::Name(format!("mute-{id}").into()))
                    .px(px(3.0))
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .when(mute, |s| {
                        s.bg(colors.danger)
                            .child(Icon::VolumeOff.icon_at(10.0, colors.surface_0))
                    })
                    .when(!mute, |s| {
                        s.bg(colors.surface_2)
                            .hover(|s| s.bg(colors.border_subtle))
                            .child(Icon::Volume.icon_at(10.0, colors.text_secondary))
                    })
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
        let snap_on = app
            .as_ref()
            .map(|a| a.read(cx).snap_enabled)
            .unwrap_or(true);
        self.panel.update(cx, |panel, _| {
            panel.set_playhead(playhead);
            panel.set_selected(selected);
            panel.set_snap_enabled(snap_on);
        });
        let has_sel = selected.is_some();

        let app_weak = self.app.clone();
        let tools = h_flex()
            .w_full()
            .h(px(30.0))
            .px(px(SpacingTokens::SM))
            .gap(px(SpacingTokens::XS))
            .items_center()
            .bg(colors.surface_1)
            .border_b_1()
            .border_color(colors.border_subtle)
            // 撤销/重做(A-09:字形 → SVG 图标;禁用态见后续波)
            .child(
                Self::tool_button("tl-undo", Icon::Undo, "", true, false, &colors).on_click({
                    let weak = app_weak.clone();
                    move |_, _, cx: &mut App| {
                        if let Some(app) = weak.upgrade() {
                            app.update(cx, |app, cx| app.submit("undo", serde_json::json!({}), cx));
                        }
                    }
                }),
            )
            .child(
                Self::tool_button("tl-redo", Icon::Redo, "", true, false, &colors).on_click({
                    let weak = app_weak.clone();
                    move |_, _, cx: &mut App| {
                        if let Some(app) = weak.upgrade() {
                            app.update(cx, |app, cx| app.submit("redo", serde_json::json!({}), cx));
                        }
                    }
                }),
            )
            // 吸附开关(磁铁;active = 强调底)
            .child(
                Self::tool_button("tl-snap", Icon::Magnet, "吸附", true, snap_on, &colors)
                    .on_click({
                        let weak = app_weak.clone();
                        move |_, _, cx: &mut App| {
                            if let Some(app) = weak.upgrade() {
                                app.update(cx, |app, cx| app.toggle_snap(cx));
                            }
                        }
                    }),
            )
            .child(
                Self::tool_button("tl-split", Icon::Scissors, "分割", has_sel, false, &colors)
                    .on_click({
                        let weak = app_weak.clone();
                        move |_, _, cx: &mut App| {
                            if let Some(app) = weak.upgrade() {
                                app.update(cx, |app, cx| {
                                    let clip = app.selection.clone();
                                    if let Some(clip) = clip {
                                        app.submit(
                                            "clip_split",
                                            serde_json::json!({
                                                "clipId": clip,
                                                "tMs": app.playhead_ms
                                            }),
                                            cx,
                                        );
                                    }
                                });
                            }
                        }
                    }),
            )
            .child(
                Self::tool_button("tl-dup", Icon::Copy, "副本", has_sel, false, &colors).on_click(
                    {
                        let weak = app_weak.clone();
                        move |_, _, cx: &mut App| {
                            if let Some(app) = weak.upgrade() {
                                app.update(cx, |app, cx| {
                                    let clip = app.selection.clone();
                                    if let Some(clip) = clip {
                                        app.submit(
                                            "clip_duplicate",
                                            serde_json::json!({
                                                "clipId": clip,
                                                "startMs": app.playhead_ms
                                            }),
                                            cx,
                                        );
                                    }
                                });
                            }
                        }
                    },
                ),
            )
            .child(
                Self::tool_button("tl-del", Icon::Trash, "删除", has_sel, false, &colors).on_click(
                    {
                        let weak = app_weak.clone();
                        move |_, _, cx: &mut App| {
                            if let Some(app) = weak.upgrade() {
                                app.update(cx, |app, cx| {
                                    let clip = app.selection.clone();
                                    if let Some(clip) = clip {
                                        app.submit(
                                            "clip_delete",
                                            serde_json::json!({ "clipId": clip }),
                                            cx,
                                        );
                                    }
                                });
                            }
                        }
                    },
                ),
            )
            .child(
                Self::tool_button(
                    "tl-freeze",
                    Icon::Snowflake,
                    "冻结+0.5s",
                    has_sel,
                    false,
                    &colors,
                )
                .on_click({
                    let weak = app_weak.clone();
                    move |_, _, cx: &mut App| {
                        if let Some(app) = weak.upgrade() {
                            app.update(cx, |app, cx| {
                                let clip = app.selection.clone();
                                if let Some(clip) = clip {
                                    let cur = app
                                        .selected_clip()
                                        .and_then(|c| c.get("freezeMs").cloned())
                                        .and_then(|v| v.as_f64())
                                        .unwrap_or(0.0);
                                    app.submit(
                                        "clip_update",
                                        serde_json::json!({
                                            "clipId": clip,
                                            "patch": { "freezeMs": (cur + 500.0) as i64 }
                                        }),
                                        cx,
                                    );
                                }
                            });
                        }
                    }
                }),
            )
            // 复制/粘贴(壳侧剪贴板;与 Ctrl+C/V 同逻辑)
            .child(
                Self::tool_button("tl-copy", Icon::Copy, "复制", has_sel, false, &colors).on_click(
                    {
                        let weak = app_weak.clone();
                        move |_, _, cx: &mut App| {
                            if let Some(app) = weak.upgrade() {
                                app.update(cx, |app, cx| app.copy_selected(cx));
                            }
                        }
                    },
                ),
            )
            .child(
                Self::tool_button(
                    "tl-paste",
                    Icon::ClipboardPaste,
                    "粘贴",
                    true,
                    false,
                    &colors,
                )
                .on_click({
                    let weak = app_weak.clone();
                    move |_, _, cx: &mut App| {
                        if let Some(app) = weak.upgrade() {
                            app.update(cx, |app, cx| app.paste_at_playhead(cx));
                        }
                    }
                }),
            )
            // 全轨分割 / 关闭空隙(工程级剪辑;A-09:原乱码字形已由 SplitAll 图标替换)
            .child(
                Self::tool_button(
                    "tl-split-all",
                    Icon::SplitAll,
                    "全分割",
                    true,
                    false,
                    &colors,
                )
                .on_click({
                    let weak = app_weak.clone();
                    move |_, _, cx: &mut App| {
                        if let Some(app) = weak.upgrade() {
                            app.update(cx, |app, cx| {
                                app.submit(
                                    "clip_split_all",
                                    serde_json::json!({ "tMs": app.playhead_ms }),
                                    cx,
                                );
                            });
                        }
                    }
                }),
            )
            .child(
                Self::tool_button("tl-gap", Icon::GapClose, "关空隙", true, false, &colors)
                    .on_click({
                        let weak = app_weak.clone();
                        move |_, _, cx: &mut App| {
                            if let Some(app) = weak.upgrade() {
                                app.update(cx, |app, cx| app.close_gap_at_playhead(cx));
                            }
                        }
                    }),
            )
            .child(div().flex_1())
            // 轨道新增(从顶栏迁入;剪映 IA:轨道操作归时间线)
            .child(
                Self::tool_button("tl-add-v", Icon::AddTrack, "视频轨", true, false, &colors)
                    .on_click({
                        let weak = app_weak.clone();
                        move |_, _, cx: &mut App| {
                            if let Some(app) = weak.upgrade() {
                                app.update(cx, |app, cx| {
                                    app.submit(
                                        "track_add",
                                        serde_json::json!({ "kind": "video" }),
                                        cx,
                                    )
                                });
                            }
                        }
                    }),
            )
            .child(
                Self::tool_button("tl-add-a", Icon::AddTrack, "音频轨", true, false, &colors)
                    .on_click({
                        let weak = app_weak.clone();
                        move |_, _, cx: &mut App| {
                            if let Some(app) = weak.upgrade() {
                                app.update(cx, |app, cx| {
                                    app.submit(
                                        "track_add",
                                        serde_json::json!({ "kind": "audio" }),
                                        cx,
                                    )
                                });
                            }
                        }
                    }),
            )
            .child(
                Self::tool_button("tl-add-t", Icon::AddTrack, "字幕轨", true, false, &colors)
                    .on_click({
                        let weak = app_weak.clone();
                        move |_, _, cx: &mut App| {
                            if let Some(app) = weak.upgrade() {
                                app.update(cx, |app, cx| {
                                    app.submit(
                                        "track_add",
                                        serde_json::json!({ "kind": "text" }),
                                        cx,
                                    )
                                });
                            }
                        }
                    }),
            )
            .child(Self::zoom_button(
                "tl-out",
                Icon::ZoomOut,
                &app_weak,
                &colors,
                Zoom::Out,
            ))
            .child(Self::zoom_button(
                "tl-fit",
                Icon::ZoomFit,
                &app_weak,
                &colors,
                Zoom::Fit,
            ))
            .child(Self::zoom_button(
                "tl-in",
                Icon::ZoomIn,
                &app_weak,
                &colors,
                Zoom::In,
            ))
            .child(
                div()
                    .text_size(px(FONT_SIZE_CAPTION))
                    .text_color(colors.text_secondary)
                    .child(format!("{pps:.0} px/s")),
            );

        v_flex()
            .size_full()
            .bg(colors.surface_0)
            .child(tools)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(
                        // 轨道头列(顶部空位与标尺对齐;sable h_flex 默认
                        // items_center,会把头部列和时间轴挤到垂直中部)
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
                            }))
                            .when(tracks.is_empty(), |c| {
                                c.child(
                                    div()
                                        .p(px(SpacingTokens::SM))
                                        .text_size(px(FONT_SIZE_CAPTION))
                                        .text_color(colors.text_secondary)
                                        .child("工具栏 + 视频轨 开始"),
                                )
                            }),
                    )
                    .child(
                        // §9.6⑨:时间线右键菜单(04 缺口⑥;菜单项与工具行/键位同源命令)
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.panel.clone())
                            .context_menu(move |menu, _window, _cx| {
                                let weak = app.as_ref().map(|a| a.downgrade());
                                let mut menu = menu;
                                if let Some(weak) = weak {
                                    let item = |menu: PopupMenu,
                                                label: &'static str,
                                                tool: &'static str,
                                                args: serde_json::Value| {
                                        let weak = weak.clone();
                                        menu.item(
                                            PopupMenuItem::new(label).on_click(
                                                move |_, _, cx| {
                                                    if let Some(app) = weak.upgrade() {
                                                        app.update(cx, |app, cx| {
                                                            app.submit(
                                                                tool,
                                                                args.clone(),
                                                                cx,
                                                            )
                                                        });
                                                    }
                                                },
                                            ),
                                        )
                                    };
                                    menu = item(menu, "撤销", "undo", serde_json::json!({}));
                                    menu = item(menu, "重做", "redo", serde_json::json!({}));
                                    menu = menu.separator();
                                    menu = item(
                                        menu,
                                        "在播放头分割",
                                        "clip_split",
                                        serde_json::json!({}),
                                    );
                                    menu = item(
                                        menu,
                                        "副本",
                                        "clip_duplicate",
                                        serde_json::json!({}),
                                    );
                                    menu = item(
                                        menu,
                                        "删除选中",
                                        "clip_delete",
                                        serde_json::json!({}),
                                    );
                                    // 冻结帧=有状态命令(读当前 freezeMs 再 +500),与工具行同逻辑
                                    menu = menu.item(
                                        PopupMenuItem::new("冻结帧 +0.5s").on_click({
                                            let weak = weak.clone();
                                            move |_, _, cx| {
                                                if let Some(app) = weak.upgrade() {
                                                    app.update(cx, |app, cx| {
                                                        let clip = app.selection.clone();
                                                        if let Some(clip) = clip {
                                                            let cur = app
                                                                .selected_clip()
                                                                .and_then(|c| {
                                                                    c.get("freezeMs").cloned()
                                                                })
                                                                .and_then(|v| v.as_f64())
                                                                .unwrap_or(0.0);
                                                            app.submit(
                                                                "clip_update",
                                                                serde_json::json!({
                                                                    "clipId": clip,
                                                                    "patch": {
                                                                        "freezeMs":
                                                                            (cur + 500.0) as i64
                                                                    }
                                                                }),
                                                                cx,
                                                            );
                                                        }
                                                    });
                                                }
                                            }
                                        }),
                                    );
                                    menu = menu.separator();
                                    menu = item(
                                        menu,
                                        "关闭空隙",
                                        "clip_gap_delete",
                                        serde_json::json!({}),
                                    );
                                    menu = item(
                                        menu,
                                        "+ 视频轨",
                                        "track_add",
                                        serde_json::json!({ "kind": "video" }),
                                    );
                                    menu = item(
                                        menu,
                                        "+ 音频轨",
                                        "track_add",
                                        serde_json::json!({ "kind": "audio" }),
                                    );
                                }
                                menu
                            }),
                    ),
            )
            .child(
                // 底部信息条
                h_flex()
                    .h(px(22.0))
                    .px(px(SpacingTokens::SM))
                    .gap(px(SpacingTokens::SM))
                    .items_center()
                    .border_t_1()
                    .border_color(colors.border_subtle)
                    .bg(colors.surface_1)
                    .child(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child(format!(
                                "全长 {:.1}s · {} 轨 · 点标尺/轨道空白跳播放头 · 拖动片段移动",
                                duration as f64 / 1000.0,
                                tracks.len()
                            )),
                    ),
            )
    }
}
