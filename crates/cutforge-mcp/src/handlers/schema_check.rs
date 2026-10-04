// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 轻量 JSON-Schema 子集校验器(TC-MCP-DISPATCH-001 专用,`#[cfg(test)]`——
//! 运行时参数校验仍由引擎/处理器的 5.4 错误面承接,本模块只用于注册表测试,
//! 保证派发表重构零行为变化):
//!
//! 支持关键字 = schemas/mcp-tools.json 实际使用的全集:`type`(含并集如
//! ["string","null"])、`enum`、`required`、`properties`(dict 与
//! mcp-tools.json 册七 AI 面的 [name, schema] 对列表两种形态)、`items`、
//! `minimum`/`maximum`/`exclusiveMinimum`、`minLength`/`minItems`/`maxItems`、
//! `multipleOf`、`pattern`(已登记的两个契约正则以专测谓词实现;表外 pattern
//! 显式报错强制升级,不静默放行)、`additionalProperties: false`。

#[cfg(test)]
pub(crate) fn validate(
    instance: &serde_json::Value,
    schema: &serde_json::Value,
) -> Result<(), String> {
    use serde_json::Value;

    let at = |p: &str, e: String| format!("{p}: {e}");

    // type(字符串或并集数组)
    if let Some(ty) = schema.get("type") {
        let ok = match ty {
            Value::String(s) => type_matches(instance, s),
            Value::Array(list) => list
                .iter()
                .any(|t| t.as_str().is_some_and(|s| type_matches(instance, s))),
            _ => true,
        };
        if !ok {
            return Err(at("$", format!("type 需 {ty}, 实得 {instance}")));
        }
    }
    // enum
    if let Some(en) = schema.get("enum").and_then(Value::as_array)
        && !en.contains(instance)
    {
        return Err(at("$", format!("enum 需 {en:?}, 实得 {instance}")));
    }
    // 数值界
    if let Some(x) = instance.as_f64() {
        if let Some(b) = schema.get("minimum").and_then(Value::as_f64)
            && x < b
        {
            return Err(at("$", format!("minimum={b}, 实得 {x}")));
        }
        if let Some(b) = schema.get("maximum").and_then(Value::as_f64)
            && x > b
        {
            return Err(at("$", format!("maximum={b}, 实得 {x}")));
        }
        if let Some(b) = schema.get("exclusiveMinimum").and_then(Value::as_f64)
            && x <= b
        {
            return Err(at("$", format!("exclusiveMinimum={b}, 实得 {x}")));
        }
        if let Some(b) = schema.get("exclusiveMaximum").and_then(Value::as_f64)
            && x >= b
        {
            return Err(at("$", format!("exclusiveMaximum={b}, 实得 {x}")));
        }
        if let Some(m) = schema.get("multipleOf").and_then(Value::as_f64)
            && m > 0.0
            && (x / m - (x / m).round()).abs() > 1e-9
        {
            return Err(at("$", format!("须为 {m} 的倍数, 实得 {x}")));
        }
    }
    // 字符串面
    if let Some(s) = instance.as_str() {
        if let Some(min) = schema.get("minLength").and_then(Value::as_u64)
            && (s.chars().count() as u64) < min
        {
            return Err(at(
                "$",
                format!("minLength={min}, 实得 {}", s.chars().count()),
            ));
        }
        if let Some(pat) = schema.get("pattern").and_then(Value::as_str)
            && !pattern_matches(pat, s)
        {
            return Err(at("$", format!("pattern {pat} 不匹配: {s}")));
        }
    }
    // 数组面
    if let Some(arr) = instance.as_array() {
        if let Some(min) = schema.get("minItems").and_then(Value::as_u64)
            && (arr.len() as u64) < min
        {
            return Err(at("$", format!("minItems={min}, 实得 {}", arr.len())));
        }
        if let Some(max) = schema.get("maxItems").and_then(Value::as_u64)
            && (arr.len() as u64) > max
        {
            return Err(at("$", format!("maxItems={max}, 实得 {}", arr.len())));
        }
        if let Some(item_schema) = schema.get("items") {
            for (i, v) in arr.iter().enumerate() {
                validate(v, item_schema).map_err(|e| at(&format!("[{i}]"), e))?;
            }
        }
    }
    // 对象面:required + properties(两种形态)+ additionalProperties:false
    if let Some(obj) = instance.as_object() {
        let props = schema.get("properties");
        let pairs: Vec<(String, &Value)> = match props {
            Some(Value::Object(map)) => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
            // 册七 AI 面形态:properties = [[name, schema], …]
            Some(Value::Array(list)) => list
                .iter()
                .filter_map(|pair| {
                    let kv = pair.as_array()?;
                    Some((kv.first()?.as_str()?.to_string(), kv.get(1)?))
                })
                .collect(),
            _ => Vec::new(),
        };
        for req in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !obj.contains_key(req) {
                return Err(at("$", format!("缺必填参数 {req}")));
            }
        }
        if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
            for key in obj.keys() {
                if !pairs.iter().any(|(name, _)| name == key) {
                    return Err(at(
                        "$",
                        format!("additionalProperties=false 拒绝未知参数 {key}"),
                    ));
                }
            }
        }
        for (name, sub) in pairs {
            if let Some(v) = obj.get(&name) {
                validate(v, sub).map_err(|e| at(&format!(".{name}"), e))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
fn type_matches(v: &serde_json::Value, ty: &str) -> bool {
    match ty {
        "null" => v.is_null(),
        "boolean" => v.is_boolean(),
        "object" => v.is_object(),
        "array" => v.is_array(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64(),
        "string" => v.is_string(),
        _ => true, // 未知类型名不冒判(升级由 pattern 同策:显式登记)
    }
}

/// 契约 pattern 表(mcp-tools.json 全量两个;新 pattern 必须显式登记,报错升级)。
#[cfg(test)]
fn pattern_matches(pattern: &str, s: &str) -> bool {
    match pattern {
        // 颜色:#RGB6 / #RGBA8
        "^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$" => {
            let body = s.strip_prefix('#').unwrap_or("");
            (body.len() == 6 || body.len() == 8) && body.chars().all(|c| c.is_ascii_hexdigit())
        }
        // 关键帧可动画字段白名单
        "^(position\\.x|position\\.y|scale|rotation|opacity|volume|speed|fx\\..+\\.[^.]+)$" => {
            matches!(
                s,
                "position.x" | "position.y" | "scale" | "rotation" | "opacity" | "volume" | "speed"
            ) || {
                // fx\..+\.[^.]+:fx.<任意非空中段>.<非空尾段>
                s.starts_with("fx.")
                    && s.len() > "fx.".len()
                    && !s.ends_with('.')
                    && s["fx.".len()..].contains('.')
            }
        }
        other => panic!("schema_check 未登记的 pattern,须显式升级校验器: {other}"),
    }
}
