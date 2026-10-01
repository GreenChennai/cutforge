// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 批处理清单(册七 T7.4):`cutforge-cli batch <manifest.json|yaml>`——
//! 多工程渲染单排队,报告 JSON schema 固化(docs/schemas/batch-report.schema.json)。
//!
//! 清单格式:纯 JSON,或**手写最小 YAML 子集**(零依赖纪律,ADR-0009 同源):
//! 2 空格缩进;映射 `key: value`(冒号后必须有空格,值含冒号的 Windows 路径安全);
//! 列表 `- ` 项;标量 = 裸串/引号串/整数/true/false;`#` 注释;不支持锚点/多行/流式集合。
//! 工程条目键与 MCP render/render_run 参数同名(root/ass/format/quality/crf/…),
//! 渲染参数装配复用 cutforge_mcp::build_render_extra 单一实现。

use crate::{emit, Args};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// batch 子命令入口。退出码:0 = 全部成功;2 = 有失败/清单非法;3 = 渲染器缺失。
pub fn run(a: &Args) -> i32 {
    let Some(manifest_path) = a.positional.first() else {
        return emit(a.json, false, "PRECONDITION_FAILED",
            "用法: batch <manifest.json|yaml> [--report out.json] [--stop-on-fail]", json!({}));
    };
    let text = match std::fs::read_to_string(manifest_path) {
        Ok(t) => t,
        Err(e) => return emit(a.json, false, "NO_CONFIG", &format!("清单不可读: {manifest_path}({e})"), json!({})),
    };
    let manifest = match parse_manifest(&text) {
        Ok(v) => v,
        Err(e) => return emit(a.json, false, "SCHEMA_INVALID", &e, json!({})),
    };
    let Some(projects) = manifest["projects"].as_array().filter(|p| !p.is_empty()) else {
        return emit(a.json, false, "PRECONDITION_FAILED", "清单缺 projects(非空数组,逐项 {root, …渲染参数})", json!({}));
    };
    // 渲染器缺失 = 环境问题,退出码 3(诚实上报,不冒充失败)
    if cutforge_mcp::resolve_render_bin().is_none() {
        return emit(a.json, false, "DEP_MISSING",
            "未找到 cutforge-render:与 cutforge-cli 同目录放置、加入 PATH,或设 CUTFORGE_RENDER", json!({}));
    }
    let report_out = a.flags.get("report").cloned()
        .or_else(|| manifest["report"].as_str().map(String::from));
    let stop_on_fail = a.flags.contains_key("stop-on-fail") || manifest["stopOnFail"].as_bool() == Some(true);
    let defaults = manifest.get("defaults").cloned().unwrap_or_else(|| json!({}));

    let started_at = cutforge_core::timeutil::now_rfc3339();
    let t0 = std::time::Instant::now();
    let mut jobs: Vec<Value> = Vec::new();
    let (mut ok_n, mut fail_n) = (0usize, 0usize);
    for (i, entry) in projects.iter().enumerate() {
        let Some(root) = entry["root"].as_str() else {
            fail_n += 1;
            jobs.push(json!({"index": i, "project": "", "name": "", "ok": false,
                "code": "PRECONDITION_FAILED", "error": "条目缺 root", "durationMs": 0}));
            if stop_on_fail { break; }
            continue;
        };
        let name = entry["name"].as_str()
            .map(String::from)
            .or_else(|| Path::new(root).file_name().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default();
        // 合并顺序:defaults → 条目覆盖(与编辑器工程级/片段级合并同风格)
        let mut args = if defaults.is_object() { defaults.clone() } else { json!({}) };
        if let Some(obj) = entry.as_object() {
            for (k, v) in obj {
                args[k] = v.clone();
            }
        }
        args["root"] = json!(root);
        let t_job = std::time::Instant::now();
        let (ok, code, output, error) = render_one(Path::new(root), &args);
        let dur = t_job.elapsed().as_millis() as u64;
        if ok { ok_n += 1; } else { fail_n += 1; }
        let mut job = json!({
            "index": i, "project": root, "name": name, "ok": ok, "code": code, "durationMs": dur,
        });
        if let Some(o) = &output { job["output"] = json!(o); }
        if let Some(e) = &error { job["error"] = json!(e); }
        jobs.push(job.clone());
        let mark = if ok { "PASS" } else { "FAIL" };
        println!("[{mark}] #{i} {name}: {code} ({}ms){}", dur,
            output.as_deref().map(|o| format!(" → {o}")).unwrap_or_default());
        if !ok && stop_on_fail { break; }
    }
    let report = json!({
        "kind": "cutforge-batch-report",
        "version": 1,
        "startedAt": started_at,
        "finishedAt": cutforge_core::timeutil::now_rfc3339(),
        "durationMs": t0.elapsed().as_millis() as u64,
        "manifest": manifest_path,
        "renderBin": cutforge_mcp::resolve_render_bin().map(|p| p.to_string_lossy().into_owned()),
        "jobs": jobs,
        "summary": {"total": projects.len(), "ok": ok_n, "fail": fail_n},
    });
    if let Some(out) = &report_out {
        let mut buf = serde_json::to_vec_pretty(&report).unwrap_or_default();
        buf.push(b'\n');
        let out_path = PathBuf::from(out);
        if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty())
            && let Err(e) = std::fs::create_dir_all(parent) {
                return emit(a.json, false, "INTERNAL", &format!("报告目录不可建: {}({e})", parent.display()), json!({}));
            }
        if let Err(e) = cutforge_io::atomic::atomic_write(&PathBuf::from(out), &buf) {
            return emit(a.json, false, "INTERNAL", &format!("报告写盘失败: {out}({e})"), json!({}));
        }
    }
    let code = if fail_n == 0 { "OK" } else { "INTERNAL" };
    let mut data = report.clone();
    if let Some(out) = &report_out {
        data["report"] = json!(out);
    }
    emit(a.json, fail_n == 0, code,
        &format!("批处理完成:{ok_n}/{} 成功", projects.len()), data)
}

/// 单工程渲染(同步;与 MCP render backend=cutforge 同一实现)。
fn render_one(root: &Path, args: &Value) -> (bool, String, Option<String>, Option<String>) {
    let ass = args["ass"].as_str().filter(|s| !s.is_empty() && root.join(s).is_file());
    let extra = cutforge_mcp::build_render_extra(args, true);
    let env = cutforge_mcp::render_cutforge_sync(root, ass, false, &extra);
    let ok = env["ok"] == json!(true);
    (
        ok,
        env["code"].as_str().unwrap_or("INTERNAL").to_string(),
        env["data"]["output"].as_str().map(String::from),
        if ok { None } else { Some(env["message"].as_str().unwrap_or_default().to_string()) },
    )
}

/// 清单解析:JSON 优先,失败回退最小 YAML 子集(形态见模块头注释)。
pub fn parse_manifest(text: &str) -> Result<Value, String> {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        return if v.is_object() { Ok(v) } else { Err("清单顶层必须是对象".into()) };
    }
    let lines: Vec<(usize, &str)> = text
        .lines()
        .filter_map(|l| {
            let no_comment = l.split_once('#').map(|(h, _)| h).unwrap_or(l);
            let t = no_comment.trim_end();
            let indent = t.len() - t.trim_start().len();
            let body = t.trim_start();
            if body.is_empty() { None } else { Some((indent, body)) }
        })
        .collect();
    let (v, consumed) = parse_block(&lines, 0, lines.first().map_or(0, |(i, _)| *i))?;
    if consumed != lines.len() {
        return Err(format!("YAML 子集解析未收口:第 {} 行附近(缩进层级不齐?)", lines[consumed].0));
    }
    Ok(v)
}

/// 递归下降解析一个缩进块,返回 (值, 已消费行数)。
fn parse_block(lines: &[(usize, &str)], start: usize, indent: usize) -> Result<(Value, usize), String> {
    if start >= lines.len() {
        return Err("YAML 子集:意外结尾".into());
    }
    if lines[start].1.starts_with("- ") || lines[start].1 == "-" {
        let mut items = Vec::new();
        let mut i = start;
        while i < lines.len() && lines[i].0 == indent && (lines[i].1.starts_with("- ") || lines[i].1 == "-") {
            let rest = lines[i].1.strip_prefix("- ").unwrap_or("").trim();
            if rest.is_empty() {
                // "- " 后跟嵌套块
                let (v, used) = parse_block(lines, i + 1, indent + 2)?;
                items.push(v);
                i = used;
                continue;
            }
            if rest.contains(": ") || rest.ends_with(':') {
                // "- key: value" 行内映射:其后续同级键在 indent+2
                let mut inline = vec![(indent + 2, rest)];
                let mut j = i + 1;
                while j < lines.len() && lines[j].0 > indent {
                    inline.push((lines[j].0, lines[j].1));
                    j += 1;
                }
                let (v, used) = parse_block(&inline, 0, indent + 2)?;
                items.push(v);
                let _ = used;
                i = j;
                continue;
            }
            items.push(parse_scalar(rest));
            i += 1;
        }
        return Ok((Value::Array(items), i));
    }
    // 映射
    let mut map = serde_json::Map::new();
    let mut i = start;
    while i < lines.len() && lines[i].0 == indent {
        let line = lines[i].1;
        let (key, rest) = split_kv(line)?;
        if rest.is_empty() {
            // 嵌套块:子缩进必须更大
            if i + 1 < lines.len() && lines[i + 1].0 > indent {
                let (v, used) = parse_block(lines, i + 1, lines[i + 1].0)?;
                map.insert(key, v);
                i = used;
            } else {
                map.insert(key, Value::Null);
                i += 1;
            }
        } else {
            map.insert(key, parse_scalar(rest));
            i += 1;
        }
    }
    Ok((Value::Object(map), i))
}

/// `key: value` 切分(冒号后必须有空格或行尾;键不带引号)。
fn split_kv(line: &str) -> Result<(String, &str), String> {
    let idx = line.find(": ")
        .or_else(|| if line.ends_with(':') { Some(line.len() - 1) } else { None });
    let Some(idx) = idx else {
        return Err(format!("YAML 子集:映射行必须是 `key: value`(冒号后空格): {line}"));
    };
    let key = line[..idx].trim().to_string();
    if key.is_empty() || key.contains(':') {
        return Err(format!("YAML 子集:键非法: {line}"));
    }
    Ok((key, line[idx + 1..].trim()))
}

/// 标量:引号串 / 整数 / true / false / 其余按裸串。
fn parse_scalar(s: &str) -> Value {
    let t = s.trim();
    if (t.starts_with('"') && t.ends_with('"') && t.len() >= 2)
        || (t.starts_with('\'') && t.ends_with('\'') && t.len() >= 2)
    {
        return json!(&t[1..t.len() - 1]);
    }
    if let Ok(n) = t.parse::<u64>() {
        return json!(n);
    }
    match t {
        "true" => return json!(true),
        "false" => return json!(false),
        "null" | "~" => return Value::Null,
        _ => {}
    }
    json!(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 最小 YAML 子集:映射/嵌套/列表/引号/注释/整数布尔;JSON 优先通道。
    #[test]
    fn manifest_yaml_subset_and_json() {
        let y = r#"
# 批处理清单(注释)
version: 1
concurrency: 1
report: "out/batch.json"
defaults:
  format: mp4
  quality: high
projects:
  - root: D:/素材/工程甲
    name: 甲
    format: gif
    outMs: 30000
  - root: 'D:/素材/工程乙:备份'   # 值含冒号
"#;
        let v = parse_manifest(y).unwrap();
        assert_eq!(v["version"], json!(1));
        assert_eq!(v["defaults"]["format"], json!("mp4"));
        assert_eq!(v["report"], json!("out/batch.json"));
        let projects = v["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0]["root"], json!("D:/素材/工程甲"));
        assert_eq!(projects[0]["format"], json!("gif"));
        assert_eq!(projects[0]["outMs"], json!(30000));
        assert_eq!(projects[1]["root"], json!("D:/素材/工程乙:备份"));
        // JSON 通道等价
        let j = parse_manifest(r#"{"projects": [{"root": "X"}], "version": 1}"#).unwrap();
        assert_eq!(j["projects"][0]["root"], json!("X"));
        // 非法面:缩进不齐 / 映射行缺空格
        assert!(parse_manifest("a: b\n  c: d\n").is_err(), "缩进不齐必须拒绝");
    }
}
