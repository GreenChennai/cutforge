//! draft-07 子集校验引擎(与 tools/schema_gen.py 生成的 Python 引擎逐语义对齐)。
//!
//! 支持:type(string|array)/const/enum/required/properties/additionalProperties(false)/
//! items/$ref(#/$defs/...)/minimum/maximum/exclusiveMinimum/minLength/maxLength/
//! pattern/minItems/maxItems,以及两个自定义跨字段断言:
//! `x-removeRequiresGuardOk`(cutlist,SKILL Hard Rule 3 进契约)与
//! `x-keepCoversTimeline`(keep 区间有序不重叠且覆盖 [0,srcTotalMs])。
//!
//! 第三方依赖 regex 的引入依据 ADR-0034(pattern 断言需要;契约层最小依赖集:
//! serde + serde_json + regex)。

use serde_json::Value;

fn resolve<'a>(schema: &'a Value, root: &'a Value) -> &'a Value {
    let mut cur = schema;
    while let Some(r) = cur.get("$ref").and_then(Value::as_str) {
        assert!(r.starts_with("#/"), "不支持的外部引用: {r}");
        let mut node = root;
        for part in r[2..].split('/') {
            node = node.get(part.replace("~1", "/").replace("~0", "~").as_str()).unwrap_or_else(|| panic!("悬垂 $ref: {r}"));
        }
        cur = node;
    }
    cur
}

fn type_matches(types: &[&str], data: &Value) -> bool {
    let m = |t: &str| match t {
        "integer" => data.is_i64() || data.is_u64(),
        "number" => data.is_number(),
        "string" => data.is_string(),
        "boolean" => data.is_boolean(),
        "object" => data.is_object(),
        "array" => data.is_array(),
        "null" => data.is_null(),
        _ => false,
    };
    types.iter().any(|t| m(t))
}

fn num_of(v: &Value) -> Option<f64> {
    v.as_f64()
}

pub fn validate_node(schema: &Value, root: &Value, data: &Value, path: &str) -> Vec<String> {
    let mut errors: Vec<String> = Vec::new();
    let schema = resolve(schema, root);

    if let Some(c) = schema.get("const") {
        if c != data {
            errors.push(format!("{path}: const 期望 {c} 实际 {data}"));
            return errors;
        }
    }
    if let Some(en) = schema.get("enum").and_then(Value::as_array) {
        if !en.contains(data) {
            errors.push(format!("{path}: enum {en:?} 不含 {data}"));
            return errors;
        }
    }
    if let Some(t) = schema.get("type") {
        let ok = match t {
            Value::String(s) => type_matches(&[s.as_str()], data),
            Value::Array(a) => {
                let names: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
                type_matches(&names, data)
            }
            _ => true,
        };
        if !ok {
            errors.push(format!("{path}: type 期望 {t} 实际 {}", type_name(data)));
            return errors;
        }
    }

    for (key, op) in [("minimum", std::cmp::Ordering::Less), ("maximum", std::cmp::Ordering::Greater)] {
        if let (Some(limit), Some(d)) = (schema.get(key).and_then(num_of), num_of(data)) {
            if d.partial_cmp(&limit) == Some(op) {
                errors.push(format!("{path}: {key} {limit} 实际 {d}"));
            }
        }
    }
    if let (Some(limit), Some(d)) = (schema.get("exclusiveMinimum").and_then(num_of), num_of(data)) {
        if d <= limit {
            errors.push(format!("{path}: exclusiveMinimum {limit} 实际 {d}"));
        }
    }

    if let Some(s) = data.as_str() {
        if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
            if (s.chars().count() as u64) < min {
                errors.push(format!("{path}: minLength {min} 实际长度 {}", s.chars().count()));
            }
        }
        if let Some(max) = schema.get("maxLength").and_then(Value::as_u64) {
            if (s.chars().count() as u64) > max {
                errors.push(format!("{path}: maxLength {max} 实际长度 {}", s.chars().count()));
            }
        }
        if let Some(pat) = schema.get("pattern").and_then(Value::as_str) {
            let re = regex::Regex::new(pat).expect("schema pattern 必须合法");
            if !re.is_match(s) {
                errors.push(format!("{path}: pattern {pat:?} 不匹配 {s:?}"));
            }
        }
    }

    if let Some(arr) = data.as_array() {
        if let Some(min) = schema.get("minItems").and_then(Value::as_u64) {
            if (arr.len() as u64) < min {
                errors.push(format!("{path}: minItems {min} 实际 {}", arr.len()));
            }
        }
        if let Some(max) = schema.get("maxItems").and_then(Value::as_u64) {
            if (arr.len() as u64) > max {
                errors.push(format!("{path}: maxItems {max} 实际 {}", arr.len()));
            }
        }
        if let Some(item) = schema.get("items").filter(|v| v.is_object()) {
            for (i, el) in arr.iter().enumerate() {
                errors.extend(validate_node(item, root, el, &format!("{path}[{i}]")));
            }
        }
    }

    if let Some(obj) = data.as_object() {
        if let Some(req) = schema.get("required").and_then(Value::as_array) {
            for r in req.iter().filter_map(Value::as_str) {
                if !obj.contains_key(r) {
                    errors.push(format!("{path}: 缺必填键 '{r}'"));
                }
            }
        }
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (k, v) in obj {
                match props.get(k) {
                    Some(ps) => errors.extend(validate_node(ps, root, v, &format!("{path}.{k}"))),
                    None => {
                        if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                            errors.push(format!("{path}: additionalProperties=false 拒绝多余键 '{k}'"));
                        }
                    }
                }
            }
        }
    }

    // ---- 自定义跨字段断言(与 Python 引擎一致) ----
    if schema.get("x-removeRequiresGuardOk") == Some(&Value::Bool(true)) {
        if let Some(obj) = data.as_object() {
            if obj.get("action").and_then(Value::as_str) == Some("remove") {
                let guard = obj.get("guard");
                match guard.and_then(Value::as_object) {
                    None => errors.push(format!("{path}: action=remove 但 guard 为空(guard_passed 视为未过)")),
                    Some(g) if g.is_empty() => {
                        errors.push(format!("{path}: action=remove 但 guard 为空(guard_passed 视为未过)"))
                    }
                    Some(g) => {
                        let ok = g.get("okByReason").or_else(|| g.get("ok"));
                        if ok != Some(&Value::Bool(true)) {
                            errors.push(format!("{path}: action=remove 但 guard 判定 ok={ok:?}"));
                        }
                        if g.get("wordClipped") == Some(&Value::Bool(true)) {
                            errors.push(format!("{path}: action=remove 但 wordClipped=true(任何 reason 下硬失败)"));
                        }
                    }
                }
            }
        }
    }
    if schema.get("x-keepCoversTimeline") == Some(&Value::Bool(true)) {
        if let Some(obj) = data.as_object() {
            if let (Some(keep), Some(total)) =
                (obj.get("keep").and_then(Value::as_array), obj.get("srcTotalMs").and_then(Value::as_i64))
            {
                let mut cur: i64 = 0;
                let mut ok = true;
                for seg in keep {
                    let pair = seg.as_array().map(|a| {
                        (a.len() == 2, a.first().and_then(Value::as_i64), a.get(1).and_then(Value::as_i64))
                    });
                    match pair {
                        Some((true, Some(a), Some(b))) if a >= cur && b >= a => cur = b,
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    errors.push(format!("{path}: keep 区间无序/重叠/非法"));
                } else if cur != total {
                    errors.push(format!("{path}: keep 覆盖到 {cur} ≠ srcTotalMs {total}"));
                }
            }
        }
    }
    errors
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
