// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! project v1→最新契约迁移器(计划书 3.3;幂等:对同一输入迁移两次,结果语义相等)。
//! 册五 T5.1:目标版本随 IR v3 升为 3.0.0;v2 工程读兼容不经本函数(缺 keyframes
//! 字段 = 无关键帧,schema enum 双版本接受,零迁移成本不强制改写)。

use serde_json::{Map, Value, json};

fn letter(kind: &str) -> char {
    match kind {
        "video" => 'V',
        "audio" => 'A',
        "text" => 'T',
        _ => 'X',
    }
}

/// 就地补齐 v3 新增契约(schemaVersion/backends/notes/track 与 clip 的稳定 id)。
/// `version` 整数保持 1 兼容旧读法(3.3 第 5 项)。缺 schemaVersion(纯 v1)补
/// "3.0.0";显式 "2.0.0" 保留不动(v2 读兼容口径,schema enum 双版本合法)。
pub fn migrate_project_v1_to_v2(doc: &Value) -> Value {
    let mut obj: Map<String, Value> = doc.as_object().cloned().expect("project 文档必须是 object");
    obj.entry("schemaVersion".to_string())
        .or_insert_with(|| json!("3.0.0"));
    obj.entry("backends".to_string())
        .or_insert_with(|| json!(["ffmpeg"]));
    obj.entry("notes".to_string())
        .or_insert_with(|| json!("notes.json"));

    if let Some(Value::Array(tracks)) = obj.get_mut("tracks") {
        for (ti, track) in tracks.iter_mut().enumerate() {
            let t = track.as_object_mut().expect("track 必须是 object");
            let tid = match t.get("id").and_then(Value::as_str) {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => {
                    let kind = t.get("kind").and_then(Value::as_str).unwrap_or("");
                    let id = format!("{}{}", letter(kind), ti + 1);
                    t.insert("id".into(), json!(id.clone()));
                    id
                }
            };
            if let Some(Value::Array(clips)) = t.get_mut("clips") {
                for (ci, clip) in clips.iter_mut().enumerate() {
                    let c = clip.as_object_mut().expect("clip 必须是 object");
                    let default_id = format!("{tid}-{:03}", ci + 1);
                    c.entry("id".to_string())
                        .or_insert_with(|| json!(default_id));
                }
            }
        }
    }
    Value::Object(obj)
}
