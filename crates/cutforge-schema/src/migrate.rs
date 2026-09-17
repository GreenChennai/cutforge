//! project v1→v2 迁移器(计划书 3.3;幂等:对同一输入迁移两次,结果语义相等)。

use serde_json::{json, Map, Value};

fn letter(kind: &str) -> char {
    match kind {
        "video" => 'V',
        "audio" => 'A',
        "text" => 'T',
        _ => 'X',
    }
}

/// 就地补齐 v2 新增字段(schemaVersion/backends/notes/track 与 clip 的稳定 id)。
/// `version` 整数保持 1 兼容旧读法(3.3 第 5 项)。
pub fn migrate_project_v1_to_v2(doc: &Value) -> Value {
    let mut obj: Map<String, Value> = doc.as_object().cloned().expect("project 文档必须是 object");
    obj.entry("schemaVersion".to_string()).or_insert_with(|| json!("2.0.0"));
    obj.entry("backends".to_string()).or_insert_with(|| json!(["ffmpeg"]));
    obj.entry("notes".to_string()).or_insert_with(|| json!("notes.json"));

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
                    c.entry("id".to_string()).or_insert_with(|| json!(default_id));
                }
            }
        }
    }
    Value::Object(obj)
}
