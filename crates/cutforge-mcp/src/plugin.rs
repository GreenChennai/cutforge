// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 插件服务端面(册七 T7.2/ADR-0024):manifest 校验 + 权限裁决 + actor=plugin
//! 写通道。双形态(壳内 Worker / 外部进程)共用同一套 manifest 权限模型与同一
//! `dispatch` 单表——不建第二套业务逻辑;插件的一切工程写操作都走 Op 通道,
//! OpLog 以 actor=plugin(<manifest.id>)如实归因。
//!
//! 服务端权限裁决面 = manifest 声明的权限 vs 工具分类:
//!   查询类 → permissions.read / 写类 → permissions.write / 编排类 → permissions.exec;
//!   network/filesystem 为声明面(Worker 形态经宿主代理执行,process 形态按
//!   docs/PLUGIN-SPEC.md 生命周期约定审计)。越权 = GUARD_FAILED(FORBIDDEN 语义)。

use crate::registry::{envelope, tool_kind};
use cutforge_core::oplog::Actor;
use serde_json::{Value, json};

/// manifest 校验:schema 契约(cutforge-schema 单一真相源)+ 语义面
/// (entry 路径形态/形态-入口匹配/贡献点 id 去重)。返回错误清单(空 = 合法)。
pub fn validate_manifest(v: &Value) -> Vec<String> {
    let mut errs = cutforge_schema::validate("plugin-manifest", v);
    let entry = v["entry"].as_str().unwrap_or_default();
    if !entry.is_empty() {
        let path_bad = entry.starts_with('/')
            || entry.contains('\\')
            || entry.split(['/', '\\']).any(|seg| seg == "..")
            || entry.split(['/', '\\']).any(|seg| seg.contains(':'));
        if path_bad {
            errs.push(format!(
                "entry 必须是安装目录内相对路径(拒绝对象路径/盘符/..): {entry}"
            ));
        }
        match v["form"].as_str() {
            Some("worker") if !entry.ends_with(".js") && !entry.ends_with(".mjs") => {
                errs.push(format!(
                    "form=worker 的 entry 必须是 .js/.mjs 模块: {entry}"
                ));
            }
            Some("process") if !entry.contains('.') => {
                errs.push(format!(
                    "form=process 的 entry 必须带扩展名(可执行/脚本): {entry}"
                ));
            }
            _ => {}
        }
    }
    let mut dup_check = |key: &str| {
        let mut seen = std::collections::BTreeSet::new();
        if let Some(list) = v["contributes"][key].as_array() {
            for c in list {
                if let Some(id) = c["id"].as_str()
                    && !seen.insert(id.to_string())
                {
                    errs.push(format!("contributes.{key} 贡献点 id 重复: {id}"));
                }
            }
        }
    };
    dup_check("commands");
    dup_check("panels");
    // 确定性输出:workspace 引入 sable(git 依赖)后 serde_json 启用
    // preserve_order,schema 校验器按 manifest 输入序遍历键,错误列表顺序
    // 随输入漂移(golden 对拍实测)。排序让响应与 map 实现解耦。
    errs.sort();
    errs
}

/// 权限裁决:manifest 声明 vs 工具分类(查询→read / 写→write / 编排→exec)。
/// Ok = 放行;Err((code, message)) = 拒绝(5.4 表内码)。
pub fn authorize(manifest: &Value, tool: &str) -> Result<(), (String, String)> {
    let Some(kind) = tool_kind(tool) else {
        return Err(("INTERNAL".into(), format!("未知工具: {tool}")));
    };
    let (perm, zh) = match kind {
        "query" => ("read", "查询(读面)"),
        "write" => ("write", "写(工程写面;一切写走 Op 通道,actor=plugin 留痕)"),
        _ => ("exec", "编排/执行面(会拉起脚本或子进程)"),
    };
    let granted = manifest["permissions"][perm].as_bool().unwrap_or(false);
    if granted {
        return Ok(());
    }
    let id = manifest["id"].as_str().unwrap_or("?");
    Err((
        "GUARD_FAILED".into(),
        format!(
            "FORBIDDEN: 插件 {id} 未声明 permissions.{perm},拒绝调用 {kind} 类工具 {tool}({zh})"
        ),
    ))
}

/// 插件 actor(OpLog 归因;id = manifest.id,与安装目录名一致)。
pub fn plugin_actor(manifest: &Value) -> Actor {
    Actor::plugin(manifest["id"].as_str().unwrap_or("unknown-plugin"))
}

/// plugin_validate(查询):manifest JSON → 权限字段/入口/版本合法性;
/// 返回 {valid, errors, warnings}。schema 面拒 = errors;语义建议 = warnings。
pub(crate) fn plugin_validate_tool(args: &Value) -> Value {
    let Some(m) = args.get("manifest").cloned() else {
        return envelope(
            false,
            "PRECONDITION_FAILED",
            "缺 manifest(插件清单对象)",
            json!({}),
        );
    };
    let errs = validate_manifest(&m);
    let mut warnings: Vec<String> = Vec::new();
    // 建议面(不阻断):写权限插件未声明 filesystem 白名单 / process 形态未给说明
    if m["permissions"]["write"].as_bool() == Some(true)
        && m["permissions"]["filesystem"]
            .as_array()
            .is_none_or(|a| a.is_empty())
    {
        warnings.push(
            "声明了写权限但未给 permissions.filesystem 白名单(建议显式声明允许写入的工程内目录)"
                .into(),
        );
    }
    if m["form"].as_str() == Some("process")
        && m["description"]
            .as_str()
            .is_none_or(|s| s.trim().is_empty())
    {
        warnings.push("process 形态建议给 description(首次启用确认对话框展示)".into());
    }
    if errs.is_empty() {
        envelope(
            true,
            "OK",
            "manifest 合法",
            json!({"valid": true, "errors": [], "warnings": warnings}),
        )
    } else {
        envelope(
            false,
            "SCHEMA_INVALID",
            "manifest 不合法",
            json!({
                "valid": false, "errors": errs, "warnings": warnings,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(form: &str, entry: &str, perms: Value) -> Value {
        json!({
            "id": "demo-clip", "name": "示例插件", "version": "1.0.0",
            "form": form, "entry": entry, "permissions": perms,
        })
    }

    /// T7.2-9 校验负例单测(≥6 组):缺必填/坏 id/坏版本/未知形态/未知权限键/
    /// entry 越目录/形态-入口不匹配/贡献点 id 重复。
    #[test]
    fn manifest_validation_negatives() {
        let mut n = 0;
        // 1) 缺 id(schema required)
        let m = json!({"name": "x", "version": "1.0.0", "form": "process", "entry": "p.py", "permissions": {}});
        assert!(!validate_manifest(&m).is_empty());
        n += 1;
        // 2) 坏 id(大写开头)
        let mut m = manifest("process", "p.py", json!({}));
        m["id"] = json!("Demo");
        assert!(
            validate_manifest(&m)
                .iter()
                .any(|e| e.contains("id") || e.to_lowercase().contains("pattern")),
            "{m}"
        );
        n += 1;
        // 3) 坏版本(两段)
        let mut m = manifest("process", "p.py", json!({}));
        m["version"] = json!("1.0");
        assert!(!validate_manifest(&m).is_empty());
        n += 1;
        // 4) 未知形态(enum 外)
        let m = manifest("vm", "p.py", json!({}));
        assert!(!validate_manifest(&m).is_empty());
        n += 1;
        // 5) 未知权限键(additionalProperties=false)
        let m = manifest("process", "p.py", json!({"read": true, "root": true}));
        assert!(!validate_manifest(&m).is_empty());
        n += 1;
        // 6) entry 绝对路径
        let m = manifest("process", "C:/x/p.py", json!({}));
        assert!(validate_manifest(&m).iter().any(|e| e.contains("entry")));
        n += 1;
        // 7) worker 形态配 .py 入口
        let m = manifest("worker", "p.py", json!({}));
        assert!(validate_manifest(&m).iter().any(|e| e.contains("worker")));
        n += 1;
        // 8) 贡献点 id 重复
        let mut m = manifest("worker", "main.js", json!({}));
        m["contributes"] =
            json!({"commands": [{"id": "a", "title": "甲"}, {"id": "a", "title": "乙"}]});
        assert!(validate_manifest(&m).iter().any(|e| e.contains("重复")));
        n += 1;
        // 正例:全绿
        assert!(
            validate_manifest(&manifest("worker", "main.js", json!({"read": true}))).is_empty()
        );
        assert!(n >= 6, "负例 ≥6 组: {n}");
    }

    /// T7.2-10 权限裁决(读/写/编排 × 授权/越权,≥6 负例断言)。
    #[test]
    fn authorize_permission_matrix() {
        let full = json!({"id": "p1", "permissions": {"read": true, "write": true, "exec": true}});
        let ro = json!({"id": "p2", "permissions": {"read": true}});
        let none = json!({"id": "p3", "permissions": {}});
        // 授权放行
        assert!(authorize(&full, "project_get").is_ok());
        assert!(authorize(&full, "clip_update").is_ok());
        assert!(authorize(&full, "stage_run").is_ok());
        // 只读插件:查询 OK / 写与编排越权
        assert!(authorize(&ro, "timeline_get").is_ok());
        assert_eq!(authorize(&ro, "clip_update").unwrap_err().0, "GUARD_FAILED");
        assert!(
            authorize(&ro, "clip_update")
                .unwrap_err()
                .1
                .starts_with("FORBIDDEN:")
        );
        assert_eq!(authorize(&ro, "stage_run").unwrap_err().0, "GUARD_FAILED");
        // 无权限插件:查询也拒
        assert_eq!(
            authorize(&none, "notes_list").unwrap_err().0,
            "GUARD_FAILED"
        );
        // 未知工具 → INTERNAL
        assert_eq!(authorize(&full, "不存在").unwrap_err().0, "INTERNAL");
        // 编排类必须 exec(write 全有也不行)
        assert_eq!(
            authorize(
                &json!({"id": "p4", "permissions": {"read": true, "write": true}}),
                "render"
            )
            .unwrap_err()
            .0,
            "GUARD_FAILED"
        );
    }

    /// actor=plugin:plugin_actor 归因与 OpLog 写通道一致(经 apply 落 Op 后可查)。
    #[test]
    fn plugin_actor_kind() {
        let a = plugin_actor(&json!({"id": "demo-clip"}));
        assert_eq!(a.kind, cutforge_core::oplog::ActorKind::Plugin);
        assert_eq!(a.id, "demo-clip");
        // serde 往返 = oplog 落盘形态("plugin")
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["kind"], json!("plugin"));
        let back: cutforge_core::oplog::Actor = serde_json::from_value(v).unwrap();
        assert_eq!(back, a);
    }

    /// plugin_validate 工具面:缺 manifest → PRECONDITION_FAILED;非法 → SCHEMA_INVALID+valid=false。
    #[test]
    fn plugin_validate_tool_protocol() {
        let r = plugin_validate_tool(&json!({}));
        assert_eq!(r["code"], json!("PRECONDITION_FAILED"));
        let r = plugin_validate_tool(&json!({"manifest": {"id": "x"}}));
        assert_eq!(r["code"], json!("SCHEMA_INVALID"));
        assert_eq!(r["data"]["valid"], json!(false));
        let good = manifest("process", "p.py", json!({"read": true, "write": true}));
        let r = plugin_validate_tool(&json!({"manifest": good}));
        assert_eq!(r["code"], json!("OK"));
        assert_eq!(r["data"]["valid"], json!(true));
    }
}
