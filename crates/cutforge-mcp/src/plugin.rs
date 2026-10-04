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
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

// ---------------- S-03:插件运行时强制层(声明面不再只是 warning) ----------------

/// 插件运行时违规错误(工单 V2-W1 S-03)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    /// 声明 `permissions.filesystem` 白名单之外的路径访问 → 运行时拒绝并禁用插件。
    PermissionViolation { plugin: String, path: String },
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::PermissionViolation { plugin, path } => write!(
                f,
                "PermissionViolation: 插件 {plugin} 访问声明 filesystem 白名单之外的路径 {path},已拒绝并禁用"
            ),
        }
    }
}

/// 进程级已禁用插件登记:违规即禁用(一票否决,authorize 首查)。
fn disabled_plugins() -> &'static Mutex<BTreeSet<String>> {
    static D: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(BTreeSet::new()))
}

/// 运行时违规处置:登记禁用(本进程内该插件的一切工具调用被拒)。
pub fn disable_plugin(id: &str) {
    if let Ok(mut s) = disabled_plugins().lock() {
        s.insert(id.to_string());
    }
}

/// 插件是否已被禁用(测试/宿主确认面)。
pub fn is_plugin_disabled(id: &str) -> bool {
    disabled_plugins()
        .lock()
        .ok()
        .is_some_and(|s| s.contains(id))
}

/// S-03:声明面强制——manifest 声明 `permissions.filesystem` 白名单时,path
/// (绝对)必须落在至少一个白名单条目(工程根内相对路径)之下;越界 = 运行时
/// 拒绝 + 禁用插件。白名单未声明(空)保持既有 warning 语义(兼容一版,
/// 安装确认流程见 docs/PLUGIN-SPEC.md 首启确认对话框,此处与之对齐)。
pub fn check_filesystem_access(
    manifest: &Value,
    project_root: &Path,
    path: &Path,
) -> Result<(), PluginError> {
    let declared: Vec<String> = manifest["permissions"]["filesystem"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if declared.is_empty() {
        return Ok(()); // 未声明白名单 = 兼容一版(warning 面),不作硬拒绝
    }
    let id = manifest["id"]
        .as_str()
        .unwrap_or("unknown-plugin")
        .to_string();
    let Ok(canon) = path.canonicalize() else {
        let err = PluginError::PermissionViolation {
            plugin: id.clone(),
            path: path.to_string_lossy().into_owned(),
        };
        disable_plugin(&id);
        return Err(err);
    };
    for entry in &declared {
        // 逐级前缀比对(声明的白名单目录 canonicalize 失败 = 不存在,同样拒)
        if let Ok(base_c) = project_root.join(entry).canonicalize()
            && canon.starts_with(&base_c)
        {
            return Ok(());
        }
    }
    let err = PluginError::PermissionViolation {
        plugin: id.clone(),
        path: path.to_string_lossy().into_owned(),
    };
    disable_plugin(&id);
    Err(err)
}

/// process 形态的受限启动包裹(S-03;纯 std)。Unix 用 `sh -c 'ulimit -v <内存>;
/// ulimit -t <CPU 秒>; exec "$0"'` 包裹(exec 后 ulimit 限制随进程保持);
/// Windows 侧 Job Objects 超出纯 std 纪律,原样透传并留待(V2-W1 报告注明)。
pub struct ProcessLimits {
    /// 虚拟内存上限 KB(ulimit -v)。
    pub memory_kb: u64,
    /// CPU 时间上限秒(ulimit -t)。
    pub cpu_seconds: u64,
}

/// 服务端缺省资源上限(2 GiB / 600s;宿主可经 ProcessLimits 自定)。
pub const DEFAULT_PROCESS_LIMITS: ProcessLimits = ProcessLimits {
    memory_kb: 2 * 1024 * 1024,
    cpu_seconds: 600,
};

/// 返回 (program, args)。宿主以 `Command::new(program).args(args)` 启动;
/// plugin-call(cli 侧)后续轮接入同一包裹。
pub fn process_spawn_command(entry: &str) -> (String, Vec<String>) {
    process_spawn_command_with(entry, &DEFAULT_PROCESS_LIMITS)
}

/// 带显式上限的包裹(同 [`process_spawn_command`])。
pub fn process_spawn_command_with(entry: &str, limits: &ProcessLimits) -> (String, Vec<String>) {
    if cfg!(unix) {
        (
            "sh".into(),
            vec![
                "-c".into(),
                format!(
                    "ulimit -v {}; ulimit -t {}; exec \"$0\"",
                    limits.memory_kb, limits.cpu_seconds
                ),
                entry.into(),
            ],
        )
    } else {
        // Windows:Job Objects(内存/CPU/子进程树)需 Win32 调用,超出纯 std;
        // 原样透传,限制留待(V2-W1 报告 S-03 注记)
        (entry.into(), Vec::new())
    }
}

/// 带参数面的调用裁决(S-03):在 [`authorize`] 权限矩阵之上,对路径承载参数
/// (src/out/path/dir/ass)做声明白名单运行时校验;越界即以
/// `PluginError::PermissionViolation` 拒绝并禁用插件,返回 GUARD_FAILED
/// (5.4 表内码,FORBIDDEN 语义)。plugin-call 宿主后续轮换用本入口
/// (替换裸 authorize)。
pub fn authorize_call(
    manifest: &Value,
    tool: &str,
    args: &Value,
    project_root: &Path,
) -> Result<(), (String, String)> {
    authorize(manifest, tool)?;
    for key in ["src", "out", "path", "dir", "ass"] {
        if let Some(rel) = args[key].as_str()
            && !rel.is_empty()
            && let Err(e) = check_filesystem_access(manifest, project_root, &project_root.join(rel))
        {
            return Err(("GUARD_FAILED".into(), e.to_string()));
        }
    }
    Ok(())
}

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
    // 顺序确定性由 cutforge_schema::validate 的数据键排序保证(见 engine.rs),
    // 此处不再排序——required 先于字段错误是 golden 的口径
    errs
}

/// 权限裁决:manifest 声明 vs 工具分类(查询→read / 写→write / 编排→exec)。
/// Ok = 放行;Err((code, message)) = 拒绝(5.4 表内码)。
/// S-03:已因运行时违规(声明 filesystem 外路径)被禁用的插件一票否决。
pub fn authorize(manifest: &Value, tool: &str) -> Result<(), (String, String)> {
    let id = manifest["id"].as_str().unwrap_or("?");
    if is_plugin_disabled(id) {
        return Err((
            "GUARD_FAILED".into(),
            format!(
                "FORBIDDEN: 插件 {id} 因越权访问已被禁用(PermissionViolation: 声明 filesystem 外路径);重新启用须宿主确认"
            ),
        ));
    }
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

    /// TC-SEC-020(S-03):声明 filesystem:["./assets"] 的插件读白名单外路径
    /// → PermissionViolation 运行时拒绝 + 插件禁用(此后 authorize 一票否决)。
    #[test]
    fn filesystem_outside_declaration_denied_and_disabled() {
        let dir = std::env::temp_dir().join(format!("cf-plug-fs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets").join("ok.txt"), b"x").unwrap();
        let m = json!({
            "id": "fs-guarded", "name": "受限插件", "version": "1.0.0",
            "form": "process", "entry": "p.py",
            "permissions": {"read": true, "filesystem": ["./assets"]},
        });
        // 白名单内放行
        assert!(check_filesystem_access(&m, &dir, &dir.join("assets/ok.txt")).is_ok());
        // 白名单外(~/.ssh 形态的工程外路径)→ PermissionViolation + 禁用
        let ssh = dirs_home().join(".ssh");
        let err = check_filesystem_access(&m, &dir, &ssh).unwrap_err();
        assert_eq!(
            err,
            PluginError::PermissionViolation {
                plugin: "fs-guarded".into(),
                path: ssh.to_string_lossy().into_owned(),
            }
        );
        assert!(is_plugin_disabled("fs-guarded"), "违规必须触发禁用");
        // 禁用后:authorize 一票否决(即使 read 权限为 true)
        let e = authorize(&m, "project_get").unwrap_err();
        assert_eq!(e.0, "GUARD_FAILED");
        assert!(e.1.contains("PermissionViolation"), "{}", e.1);
        // authorize_call 面:路径承载参数同样被运行时拒绝
        let e = authorize_call(
            &m,
            "project_get",
            &json!({"root": dir.to_string_lossy(), "src": "01_原始素材/secret.mp4"}),
            &dir,
        )
        .unwrap_err();
        assert_eq!(e.0, "GUARD_FAILED");
        assert!(e.1.contains("PermissionViolation"), "{}", e.1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// TC-SEC-021(S-03):确认与强制面回归——未声明 filesystem 白名单保持
    /// warning 兼容(不硬拒);process 插件启动包裹:Unix 走 sh -c ulimit/exec,
    /// Windows 原样透传(Job Objects 留待);安装确认面 = plugin_validate +
    /// process 形态 description(首启确认对话框素材)。
    #[test]
    fn enforcement_and_confirmation_surface_regression() {
        // 未声明白名单 = 兼容一版(warning,不硬拒)
        let m = manifest("process", "p.py", json!({"read": true}));
        assert!(authorize(&m, "project_get").is_ok());
        // process 形态缺 description → 建议 warning(确认对话框素材)
        let r = plugin_validate_tool(&json!({"manifest": m}));
        assert_eq!(r["code"], json!("OK"));
        assert!(
            r["data"]["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w.as_str().unwrap().contains("description")),
            "process 形态缺 description 必须有确认面 warning: {r}"
        );
        // 启动包裹形态
        let (program, args) = process_spawn_command("plugins/demo/run.py");
        if cfg!(unix) {
            assert_eq!(program, "sh");
            assert_eq!(args[0], "-c");
            assert!(
                args[1].contains("ulimit -v"),
                "内存上限必须进包裹: {}",
                args[1]
            );
            assert!(
                args[1].contains("ulimit -t"),
                "CPU 上限必须进包裹: {}",
                args[1]
            );
            assert!(
                args[1].contains("exec"),
                "必须 exec 替换 shell: {}",
                args[1]
            );
            assert_eq!(args[2], "plugins/demo/run.py");
        } else {
            assert_eq!(program, "plugins/demo/run.py");
            assert!(args.is_empty(), "Windows 透传面: Job Objects 留待");
        }
        // 显式上限透传
        let (_, args2) = process_spawn_command_with(
            "p.py",
            &ProcessLimits {
                memory_kb: 65536,
                cpu_seconds: 30,
            },
        );
        if cfg!(unix) {
            assert!(args2[1].contains("ulimit -v 65536"), "{}", args2[1]);
            assert!(args2[1].contains("ulimit -t 30"), "{}", args2[1]);
        }
    }

    /// 工程外路径探测 TC-SEC-020 用(跨平台 home;不依赖 env HOME 存在性)。
    fn dirs_home() -> std::path::PathBuf {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
    }
}
