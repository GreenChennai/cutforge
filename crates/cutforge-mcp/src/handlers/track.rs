// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 轨道面处理器:track_add / track_update(册四 A4 T4.2)/ bgm_set(工程级背景乐)。
//! A-01 自 dispatch.rs 巨 match 逐字迁移(行为零变化)。

use crate::dispatch::{finish_apply, resolve_within_root};
use crate::handlers::{CommandHandler, HandlerCtx};
use crate::registry::envelope;
use cutforge_core::command::{BgmPatch, Command};
use serde_json::{Value, json};

/// track_add:轨道新建(kind 枚举 video/audio/text;adjust 由 json 面)。
pub(crate) struct TrackAdd;

impl CommandHandler for TrackAdd {
    fn name(&self) -> &'static str {
        "track_add"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        let ws = cx.ws();
        let Some(kind) = args["kind"].as_str() else {
            return envelope(false, "PRECONDITION_FAILED", "缺 kind", json!({}));
        };
        let kind = match kind {
            "video" => cutforge_core::model::TrackKind::Video,
            "audio" => cutforge_core::model::TrackKind::Audio,
            "text" => cutforge_core::model::TrackKind::Text,
            other => {
                return envelope(
                    false,
                    "PRECONDITION_FAILED",
                    &format!("未知 kind: {other}"),
                    json!({}),
                );
            }
        };
        let request_id = args["requestId"].as_str().map(String::from);
        finish_apply(ws.apply(Command::TrackAdd { kind, request_id }, actor, opts))
    }
}

/// track_update:轨道改名/颜色/高度/静音独奏/EQ/动态(实现集中在 edit_ops)。
pub(crate) struct TrackUpdate;

impl CommandHandler for TrackUpdate {
    fn name(&self) -> &'static str {
        "track_update"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        crate::edit_ops::track_update_tool(cx.ws(), args, &actor, opts)
    }
}

/// bgm_set:工程级背景乐(doc.bgm;不经 ClipPatch):src 为字符串=设置/合并;
/// src 显式 null=清除;src 缺省=仅调 gainDb/ducking/loop(工程尚无 bgm 时须先给 src)。
pub(crate) struct BgmSet;

impl CommandHandler for BgmSet {
    fn name(&self) -> &'static str {
        "bgm_set"
    }

    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
        let actor = cx.actor.clone();
        let opts = cx.opts.clone();
        let ws_root = cx.ws_root;
        let ws = cx.ws();
        if args.get("src").is_some_and(Value::is_null) {
            return finish_apply(ws.apply(Command::BgmClear, actor, opts));
        }
        let patch = BgmPatch {
            src: args["src"].as_str().map(String::from),
            gain_db: args["gainDb"].as_f64(),
            ducking: args["ducking"].as_bool(),
            loop_: args["loop"].as_bool(),
            // ducking 侧链参数(册五 T5.3):缺省 = 既有常量,不给即行为零变化
            duck_threshold: args["duckThreshold"].as_f64(),
            duck_ratio: args["duckRatio"].as_f64(),
            duck_attack_ms: args["duckAttackMs"].as_f64(),
            duck_release_ms: args["duckReleaseMs"].as_f64(),
        };
        if patch.is_empty() {
            return envelope(
                false,
                "PRECONDITION_FAILED",
                "bgm_set 至少给 src/gainDb/ducking/loop/duck* 之一(清除背景乐用 src:null)",
                json!({}),
            );
        }
        // 音源路径与 clip_add 同一校验(不建并行实现)
        if let Some(src) = patch.src.as_deref()
            && let Err(msg) = resolve_within_root(ws_root, src)
        {
            return envelope(
                false,
                "PRECONDITION_FAILED",
                &format!("bgm 路径不合法({src}): {msg}"),
                json!({}),
            );
        }
        finish_apply(ws.apply(Command::BgmSet { patch }, actor, opts))
    }
}
