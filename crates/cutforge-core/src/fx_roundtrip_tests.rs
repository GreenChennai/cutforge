// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
// model 字段 roundtrip 测试(册四 T4.6 fx 字段;自 model.rs 纯移动拆分——行数
// 红线 A1-3;断言内容零改动,sample()/导入自持)。

use crate::model::*;
use serde_json::{json, Value};

fn sample() -> Value {
    json!({
        "version": 1, "schemaVersion": "2.0.0",
        "slug": "测试工程", "fps": 30,
        "canvas": {"width": 1080, "height": 1920},
        "backends": ["ffmpeg", "cutforge"],
        "tracks": [
            {"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 8400,
                 "sourceInMs": 12000, "role": "voice"},
                {"id": "V1-002", "src": "a.mp4", "startMs": 8400, "durationMs": 6200,
                 "transition": {"type": "fade", "durMs": 300, "reason": "topic"}}
            ]},
            {"id": "A1", "kind": "audio", "clips": [
                {"id": "A1-001", "src": "sfx.mp3", "startMs": 8400, "durationMs": 400,
                 "role": "sfx", "volume": 0.8}
            ]}
        ]
    })
}

    /// 承接——"不加载 = 必丢"教训,roundtrip 证明读写不丢(参数键序确定性)。
    #[test]
    fn clip_fx_field_roundtrip_no_loss() {
        let mut v = sample();
        v["tracks"][0]["clips"][0]["fx"] = json!({
            "in": {"fx": "mo.fadeIn"},
            "combo": [
                {"fx": "fx.mono"},
                {"fx": "fx.grain", "params": {"strength": 24}}
            ]
        });
        let p = Project::from_value(&v).expect("带 fx 字段的工程必须通过 v2 校验");
        let c = &p.tracks[0].clips[0];
        let fx = c.fx.as_ref().unwrap();
        assert_eq!(fx.in_.as_ref().unwrap().fx, "mo.fadeIn");
        assert_eq!(fx.combo.as_ref().unwrap().len(), 2);
        assert_eq!(fx.combo.as_ref().unwrap()[0].fx, "fx.mono");
        assert_eq!(
            fx.combo.as_ref().unwrap()[1].params.as_ref().unwrap()["strength"],
            json!(24)
        );
        // 序列化回 Value:逐键在位(写不丢)
        let back = p.to_validated_value().unwrap();
        let f0 = &back["tracks"][0]["clips"][0]["fx"];
        assert_eq!(f0["combo"][1]["params"]["strength"], json!(24));
        assert_eq!(f0["in"]["fx"], json!("mo.fadeIn"));
        // 再读入:语义相等(serde 往返无静默丢弃)
        let p2 = Project::from_value(&back).unwrap();
        assert_eq!(p, p2);
        // 旧工程(无 fx 字段)照常读写:缺省 None,不臆造落盘
        let old = Project::from_value(&sample()).unwrap();
        assert!(old.tracks[0].clips[0].fx.is_none());
        let back_old = old.to_validated_value().unwrap();
        assert!(back_old["tracks"][0]["clips"][0].get("fx").is_none(), "缺省字段不得臆造");
        // 契约边界:combo 上限 3,第 4 条在 schema 层拒
        let mut bad = sample();
        bad["tracks"][0]["clips"][0]["fx"] = json!({"combo": [
            {"fx": "fx.mono"}, {"fx": "fx.blur"}, {"fx": "fx.grain"}, {"fx": "fx.vignette"}
        ]});
        assert!(Project::from_value(&bad).is_err(), "combo 超 3 条必须 SCHEMA_INVALID");
    }

