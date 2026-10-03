//! 时间轴宿主:widgets `TimelineView` 挂底部 dock;回调全部转内核意图。
//!
//! - `on_seek` → 播放头(壳本地)+ 预览请求;
//! - `on_move_clip` → `clip_move {clipId, startMs}`(拖拽吸附由组件做,
//!   碰撞/落点裁决在内核——壳零语义);
//! - `on_select_clip` → 选中(检查器联动)。

use sable::gpui::WeakEntity;
use sable::gpui::{
    App, AppContext as _, Entity, IntoElement, ParentElement as _, Render, Styled as _, div,
};
use sable::video::model::{ClipId, Timeline};
use sable::widgets::timeline_view::TimelineView;

use crate::app::DesktopApp;

/// 时间轴宿主。
pub struct TimelineHost {
    panel: Entity<TimelineView>,
    app: WeakEntity<DesktopApp>,
}

impl TimelineHost {
    pub fn new(app: &Entity<DesktopApp>, timeline: Entity<Timeline>, cx: &mut App) -> Entity<Self> {
        let weak = app.downgrade();
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
                .on_move_clip({
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

        cx.new(|cx| {
            // 根视图每次重投影 notify → 时间轴宿主同步播放头红线
            cx.observe(app, |_, _, cx| cx.notify()).detach();
            Self {
                panel,
                app: app.downgrade(),
            }
        })
    }
}

impl Render for TimelineHost {
    fn render(
        &mut self,
        _window: &mut sable::gpui::Window,
        cx: &mut sable::gpui::Context<Self>,
    ) -> impl IntoElement {
        let playhead = self
            .app
            .upgrade()
            .map(|app| app.read(cx).playhead_ms)
            .unwrap_or(0);
        self.panel
            .update(cx, |panel, _| panel.set_playhead(playhead));
        div().size_full().child(self.panel.clone())
    }
}
