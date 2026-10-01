// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 互操作单测(册五 T5.5;拆分自 interop.rs——行数红线 A1-3,纯移动;
//! `#[path]` 内联为 interop::tests,断言路径与语义零变化)。

use super::*;
use serde_json::json;

fn project(v: Value) -> Project {
        serde_json::from_value(v).unwrap()
    }

    /// 子集全覆盖工程:视频轨(含间隙 + fade 转场 + 复合片段)+ 音频轨 + 标记。
    fn subset_project() -> Project {
        project(json!({
            "version": 1, "schemaVersion": "3.0.0", "slug": "otio-往返", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "markers": [{"ms": 500, "label": "开场"}],
            "tracks": [
                {"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000,
                     "sourceInMs": 100},
                    {"id": "V1-002", "src": "b.mp4", "startMs": 3000, "durationMs": 1500,
                     "transition": {"type": "fade", "durMs": 300}}
                ]},
                {"id": "A1", "kind": "audio", "clips": [
                    {"id": "A1-001", "src": "sfx.mp3", "startMs": 500, "durationMs": 400}
                ]}
            ]
        }))
    }

    /// 往返语义等价(AC-5.5):出→入→再出,两份 OTIO 语义 diff=0;
    /// 工程投影等价(时间/媒体/结构逐键,不含 id/轨名这类句柄)。
    #[test]
    fn otio_roundtrip_semantic_eq() {
        let p = subset_project();
        let (otio1, w1) = otio_export(&p);
        assert!(w1.is_empty(), "子集内工程导出零 WARN: {w1:?}");
        let (p2, w2) = otio_import(&otio1).expect("子集内导入必须成功");
        assert!(w2.is_empty(), "子集内导入零 WARN: {w2:?}");
        let (otio2, w3) = otio_export(&p2);
        assert!(w3.is_empty(), "再导出零 WARN: {w3:?}");
        assert!(otio_semantic_eq(&otio1, &otio2), "出→入→再出语义 diff≠0:\n{otio1}\nvs\n{otio2}");
        // 工程投影等价:轨型/片段 (src, startMs, durationMs, sourceInMs, transition) 逐键
        type ClipRow = (Option<String>, u64, u64, Option<u64>, Option<String>);
        fn proj_tracks(p: &Project) -> Vec<(char, Vec<ClipRow>)> {
            p.tracks.iter().map(|t| (t.kind.letter(), t.clips.iter().map(|c| (
                c.src.clone(), c.start_ms, c.duration_ms,
                Some(c.source_in_ms.unwrap_or(0)), // OTIO 侧 source_range.start 恒显式,0 ≡ 缺席
                c.transition.as_ref().and_then(|t| t.type_.clone()),
            )).collect())).collect()
        }
        assert_eq!(proj_tracks(&p), proj_tracks(&p2), "工程投影等价");
        // 标记往返
        assert_eq!(p2.markers.as_ref().map(|m| m.len()), Some(1));
        assert_eq!(p2.markers.as_ref().unwrap()[0].ms, 500);
    }

    /// 复合片段 ↔ 嵌套 Stack 往返:结构(src/时窗/内层转场)保真;id 不参与语义。
    #[test]
    fn otio_compound_stack_roundtrip() {
        let p = project(json!({
            "version": 1, "schemaVersion": "3.0.0", "slug": "cpd", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": [{
                "id": "V1-001", "startMs": 0, "durationMs": 2000,
                "compound": {"clips": [
                    {"id": "V1-001", "src": "red.mp4", "startMs": 0, "durationMs": 1000},
                    {"id": "V1-002", "src": "blue.mp4", "startMs": 1000, "durationMs": 1000,
                     "transition": {"type": "wipeleft", "durMs": 400}}
                ]}
            }]}]
        }));
        let (otio1, _) = otio_export(&p);
        let (p2, w) = otio_import(&otio1).expect("复合导入必须成功");
        assert!(w.is_empty(), "{w:?}");
        let (otio2, _) = otio_export(&p2);
        assert!(otio_semantic_eq(&otio1, &otio2), "复合往返语义 diff≠0:\n{otio1}\nvs\n{otio2}");
        let shell = &p2.tracks[0].clips[0];
        let cp = shell.compound.as_ref().expect("复合字段必须还原");
        assert_eq!(cp.clips.len(), 2);
        assert_eq!(cp.clips[0].src.as_deref(), Some("red.mp4"));
        assert_eq!(cp.clips[1].transition.as_ref().unwrap().type_.as_deref(), Some("wipeleft"));
        assert_eq!(cp.duration_ms(), 2000);
    }

    /// 子集外诚实留痕:未知轨型/未知 schema/未知键导入 WARN,不静默丢;结构非法 Err。
    #[test]
    fn otio_out_of_subset_warns_and_rejects() {
        let p = subset_project();
        let (mut otio, _) = otio_export(&p);
        // 注入子集外:未知轨型 + 效果栈键 + 未知 schema 条目
        otio["tracks"]["children"][0]["children"][0]["effects"] = json!([{"OTIO_SCHEMA": "LinearTimeWarp.1"}]);
        otio["tracks"]["children"].as_array_mut().unwrap().push(json!({
            "OTIO_SCHEMA": "Track.1", "name": "T1", "kind": "Text", "children": []
        }));
        let (_p2, warns) = otio_import(&otio).unwrap();
        assert!(warns.iter().any(|w| w.contains("LinearTimeWarp") || w.contains("effects")), "{warns:?}");
        assert!(warns.iter().any(|w| w.contains("kind=Some(\"Text\")") || w.contains("整轨跳过")), "{warns:?}");
        // 结构非法:根不是 Timeline → Err
        assert!(otio_import(&json!({"OTIO_SCHEMA": "Stack.1"})).is_err());
    }

    /// EDL:头部生成器与版本;C/D 事件;转场映射 D + 帧数;复合内层注释;
    /// 非视频轨头注释声明略过(AC-5.5 人工可读判定面)。
    #[test]
    fn edl_export_shape() {
        let p = subset_project(); // V1-002 带 fade 300ms → D 事件(9 帧 @30fps)
        let edl = edl_export(&p);
        assert!(edl.starts_with("TITLE: otio-往返\n"), "{edl}");
        assert!(edl.contains("generated by cutforge"), "头部须写生成器与版本");
        assert!(edl.contains("FCM: NON-DROP FRAME"));
        assert!(edl.contains("SKIPPED TRACK: A1"), "音频轨略过须留痕");
        let d_line = edl.lines().find(|l| l.contains("  D    ")).expect("fade 必须映射 D 事件");
        assert!(d_line.contains("009"), "300ms@30fps = 9 帧: {d_line}");
        assert!(edl.contains("* FROM CLIP NAME: b.mp4"));
        let c_line = edl.lines().find(|l| l.contains("  C        ")).expect("硬切映射 C 事件");
        assert!(c_line.contains("00:00:00:03"), "V1-001 srcIn 100ms → 3 帧: {c_line}");
        // 30fps 时间码换算:3000ms → 00:00:03:00
        assert_eq!(tc(3000, 30), "00:00:03:00");
        assert_eq!(tc(0, 25), "00:00:00:00");
    }
