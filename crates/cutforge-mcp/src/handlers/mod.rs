// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 命令处理器注册表(A-01:拆分自 dispatch.rs 巨 match——分派体按域迁入本目录,
//! 行为逐字节零变化,tool_parity 黄金对拍为证):
//!
//! - [`CommandHandler`]:name / schema / stage / handle 四面一体的处理器契约;
//!   schema 单一真相源 = schemas/mcp-tools.json(编译期嵌入,与 OpenAPI/TS SDK
//!   生成器同源),处理器永不另写一份 schema;
//! - [`REGISTRY`]:全部 85 个工具的静态注册表——分派、覆盖校验、文档面全部由
//!   此派生;新增命令 = 新增一个 handler 文件 + 在 [`REGISTRY`] 注册一行
//!   (+ `golden.rs` 一行参数样例,TC-MCP-DISPATCH-001 强制 schema 与实现永不脱节);
//! - [`Stage`]:三种执行面(静态契约 / 免开工作区 / 工作区),决定 root 提取、
//!   锁纪律与幂等预检的归属——与旧派发表的前置检查顺序逐分支等价
//!   (旧 dispatch.rs :40-54 静态面、:60-162 渲染系/免锁面/AI 预演、:164-183 工作区面)。

mod clip_edit;
mod clip_write;
mod golden;
mod nolock;
mod notes;
mod orchestrate;
mod plan;
mod pro;
mod query;
mod render;
mod schema_check;
mod statics;
mod subtitle;
mod track;

use crate::registry::tool_def;
use cutforge_core::engine::ApplyOpts;
use cutforge_core::oplog::Actor;
use cutforge_io::Workspace;
use serde_json::Value;
use std::path::Path;

/// 工具执行面:旧派发表的前置检查顺序按面归属,行为逐分支等价。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// 静态契约面:免工程根(capability_matrix / plugin_validate)。
    Static,
    /// 免开工作区面:root 必给,但不进常驻工作区(不持排他锁、不做幂等预检):
    /// 渲染系(backend=cutforge 的 render 与全部 render_run/progress/frame 系)、
    /// 只读/派生物缓存类、AI 预演/批准面。
    NoLock,
    /// 工作区面:root 必给;只读判定 + 幂等预检 + with_resident
    /// (只读零锁/写全程锁,旧主通道语义原样)。
    Workspace,
}

/// 命令处理器契约(A-01 目标接口)。报告理想签名 `handle(args, &mut Engine)
/// -> Result<Value, EngineError>` 在本仓的协议化实现:工作区命令通道纪律
/// (护城河 #1)要求写操作经 `Workspace::apply → Op → OpLog`,而非裸 Engine;
/// 5.4 结果协议的 `ok/code` 轴即 `Result` 的错误语义载体(envelope 直接返回,
/// 错误码取值与映射零变化)。schema() 缺省自 JSON 契约派生,处理器不写第二份。
pub trait CommandHandler: Sync {
    /// 工具名(= schemas/mcp-tools.json 的 name;注册面完整性由注册表测试锁定)。
    fn name(&self) -> &'static str;

    /// 工具 inputSchema(单一真相源 = schemas/mcp-tools.json 编译期嵌入;
    /// 文档生成/OpenAPI/TS SDK 生成器同源此文件)。
    fn schema(&self) -> &'static Value {
        match tool_def(self.name()) {
            Some(def) => &def["inputSchema"],
            None => panic!("注册表完整性破坏: {} 未登记 JSON 契约", self.name()),
        }
    }

    /// 执行面(缺省工作区面;render 按 backend 动态裁定——cutforge 后端免锁,
    /// ffmpeg 编排走工作区,旧派发表同名前置检查顺序的等价表达)。
    fn stage(&self, _args: &Value) -> Stage {
        Stage::Workspace
    }

    /// 执行:入参 + 处理器上下文(工作区面携带已打开的 Workspace);
    /// 返回 5.4 结果协议 envelope。
    fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value;
}

/// 处理器上下文:root / actor / ApplyOpts 与(工作区面)已打开的 Workspace——
/// 旧 dispatch_on_ws 的参数束,字段语义逐字保留。
pub struct HandlerCtx<'a> {
    /// args["root"] 的字符串原样(root = 库根的 library_* 同此)。
    pub root_str: &'a str,
    pub ws_root: &'a Path,
    /// 归因 actor:壳手势 user / 其余通道 agent(旧派发表透传语义)。
    pub actor: Actor,
    /// 幂等/并发选项(requestId/summary/causedBy/expectRev,由 args 派生)。
    pub opts: ApplyOpts,
    ws: Option<&'a mut Workspace>,
}

impl<'a> HandlerCtx<'a> {
    pub(crate) fn rootless(actor: Actor) -> Self {
        Self {
            root_str: "",
            ws_root: Path::new(""),
            actor,
            opts: ApplyOpts::default(),
            ws: None,
        }
    }

    pub(crate) fn nolock(root_str: &'a str, ws_root: &'a Path, actor: Actor) -> Self {
        Self {
            root_str,
            ws_root,
            actor,
            opts: ApplyOpts::default(),
            ws: None,
        }
    }

    pub(crate) fn workspace(
        root_str: &'a str,
        ws_root: &'a Path,
        actor: Actor,
        opts: ApplyOpts,
        ws: &'a mut Workspace,
    ) -> Self {
        Self {
            root_str,
            ws_root,
            actor,
            opts,
            ws: Some(ws),
        }
    }

    /// 工作区面取 Workspace(仅 [`Stage::Workspace`] 携带;stage 与 handler 的
    /// 注册面一致性由 `registry_stages_are_consistent` 测试锁定)。
    pub fn ws(&mut self) -> &mut Workspace {
        self.ws
            .as_deref_mut()
            .expect("工作区面工具必须携带 Workspace(注册表 stage 一致性测试锁定)")
    }
}

/// 工具注册表(单一分派真相源;顺序 = 域分组,仅可读性无关行为)。
pub static REGISTRY: &[&dyn CommandHandler] = &[
    // ---- 静态契约面(免工程根) ----
    &statics::CapabilityMatrix,
    &statics::PluginValidate,
    // ---- 渲染系(免开工作区;E5/B6) ----
    &render::RenderRun,
    &render::RenderProgress,
    &render::RenderFrame,
    &render::PreviewZoneRender,
    &render::RenderQueue,
    // ---- 免开工作区(E6-3/B14:只读/创建/派生物缓存,不持排他锁) ----
    &nolock::ProjectNew,
    &nolock::MediaProbe,
    &nolock::MediaBrowse,
    &nolock::RenderProbe,
    &nolock::StageStatus,
    &nolock::MediaPeaks,
    &nolock::MediaThumbnail,
    &nolock::MediaThumbs,
    &nolock::MediaProxy,
    &nolock::AudioBeats,
    &nolock::LutImport,
    &nolock::ScopeData,
    &nolock::AudioLoudness,
    &nolock::EncodeProbe,
    &nolock::MulticamSync,
    &nolock::OtioImport,
    &nolock::MigrateLayout,
    &nolock::LibraryManage,
    &nolock::LibraryList,
    &nolock::LibraryRecover,
    &nolock::ExportPreflight,
    &nolock::ExportAllVariants,
    &nolock::MediaLibraryTool,
    &nolock::MediaImport,
    &nolock::ProjectPackage,
    &nolock::ProjectUnpackage,
    // ---- AI 改动预演/批准(册七 T7.5;plan 面自管工作区) ----
    &plan::PreviewPlan,
    &plan::ApplyPlan,
    // ---- 只读查询投影(E6-3:只读打开不持排他锁) ----
    &query::ProjectGet,
    &query::WordlineGet,
    &query::CutlistGet,
    &query::NotesList,
    &query::OplogTail,
    &query::ConflictList,
    &query::TimelineGet,
    &query::SessionReport,
    // ---- 写操作(全部经 Workspace 命令通道) ----
    &clip_write::ClipAdd,
    &clip_write::ClipUpdate,
    &clip_write::TransitionSet,
    &clip_write::MotionSet,
    &clip_write::SubtitleSet,
    &clip_write::SubtitleRetime,
    &clip_write::OverlayAdd,
    &clip_write::SfxAdd,
    &clip_write::CutApply,
    &clip_write::Undo,
    &clip_write::Redo,
    &clip_edit::ClipSplit,
    &clip_edit::ClipDelete,
    &clip_edit::ClipMove,
    &clip_edit::ClipDuplicate,
    &clip_edit::ClipTrim,
    &clip_edit::ClipSplitAll,
    &clip_edit::ClipGapDelete,
    &clip_edit::ClipCopy,
    &clip_edit::ClipPasteAt,
    &track::TrackAdd,
    &track::TrackUpdate,
    &track::BgmSet,
    &notes::NotesAdd,
    &notes::NotesResolve,
    &notes::NotesReject,
    &notes::NoteReply,
    &subtitle::TextAdd,
    &subtitle::SubtitleImport,
    &subtitle::SubtitleExport,
    &subtitle::SubtitleReplace,
    // ---- 专业编辑(册五 T5.4/T5.5) ----
    &pro::CompoundCreate,
    &pro::CompoundUnbind,
    &pro::MulticamCut,
    &pro::SceneDetect,
    &pro::OtioExport,
    // ---- 编排(封装 CutFlow 脚本,不重实现) ----
    &orchestrate::StageRun,
    &orchestrate::StageRebuild,
    &orchestrate::VerifyRun,
    &orchestrate::SyncCheck,
    &orchestrate::Render,
    &orchestrate::ExportJianying,
];

/// 按名取处理器(派发单一入口;线性扫描与旧 tool_def 同口径,85 项常量面)。
pub fn lookup(name: &str) -> Option<&'static dyn CommandHandler> {
    REGISTRY.iter().copied().find(|h| h.name() == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{tool_kind, tool_names};
    use std::path::PathBuf;

    /// 注册表完整性:REGISTRY 与 mcp-tools.json 工具集逐名一致(无缺无余不重)。
    /// 新增命令漏注册/漏写 JSON 契约在此即红。
    #[test]
    fn registry_matches_json_contract_exactly() {
        let mut handlers: Vec<&'static str> = REGISTRY.iter().map(|h| h.name()).collect();
        let mut contract = tool_names();
        handlers.sort_unstable();
        contract.sort_unstable();
        assert_eq!(handlers.len(), REGISTRY.len(), "处理器名不得重复注册");
        assert_eq!(
            handlers, contract,
            "REGISTRY 与 mcp-tools.json 工具集必须一致"
        );
    }

    /// stage 与锁纪律的一致性:声明免锁/静态面的工具不得是"会话变更面"(会升
    /// rev 的写操作必须走工作区面全程锁)——防"免锁面上偷偷写 IR"的锁纪律回归
    /// (旧派发表顺序的机检等价;RT-1 分类名单复用,不建平行口径)。
    #[test]
    fn registry_stages_are_consistent() {
        for h in REGISTRY {
            if h.stage(&serde_json::json!({})) == Stage::Workspace {
                continue;
            }
            assert!(
                !crate::rpc::produces_rev_mutation(h.name())
                    || crate::rpc::is_readonly_tool(h.name())
                    // apply_plan:plan 面自管工作区——写面由被批准项逐项重入
                    // 本单表的工作区工具自身承接(缺省拒绝,无越权写入);
                    // 本体只做批准面裁决与回执汇总,不直接改 IR。
                    || h.name() == "apply_plan",
                "免锁/静态工具 {} 不得计入会话变更(升 rev 写必须走工作区面)",
                h.name()
            );
        }
    }

    /// 文档面从注册表派生:每个工具可渲染出 Markdown 参考行
    /// (name/kind/description/required),85 工具全覆盖——文档与注册表永不脱节
    /// (OpenAPI/TS SDK 生成器同源 mcp-tools.json,本测试锁定 crate 内同源性)。
    #[test]
    fn docs_surface_derives_from_registry() {
        let mut md = String::from("| 工具 | 分类 | 说明 | 必填参数 |\n|---|---|---|---|\n");
        for h in REGISTRY {
            let def = tool_def(h.name()).expect("registry_matches_json_contract_exactly 已锁");
            let kind = tool_kind(h.name()).unwrap_or("unknown");
            let desc = def["description"].as_str().unwrap_or("");
            let required = def["inputSchema"]["required"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let desc = desc.replace('|', "\\|");
            md.push_str(&format!(
                "| {0} | {kind} | {desc} | {required} |\n",
                h.name()
            ));
        }
        assert_eq!(md.lines().count(), REGISTRY.len() + 2, "每工具一行表体");
        for h in REGISTRY {
            assert!(
                md.contains(&format!("| {} |", h.name())),
                "文档面缺工具 {}: {md}",
                h.name()
            );
        }
    }

    /// TC-MCP-DISPATCH-001(A-01):注册表内每个 handler 的 schema 都能校验其
    /// golden 参数样例——schema 与实现永不脱节。
    #[test]
    fn tc_mcp_dispatch_001_schema_validates_golden_args() {
        for h in REGISTRY {
            let golden = golden::golden_args(h.name());
            if let Err(e) = schema_check::validate(&golden, h.schema()) {
                panic!(
                    "TC-MCP-DISPATCH-001: {} 的 golden 参数样例未过 schema: {e}",
                    h.name()
                );
            }
        }
    }

    /// TC-MCP-DISPATCH-001 负向面:校验器真的在查——去掉首个必填参数必须变红
    /// (防校验器橡皮图章);无必填参数的工具跳过。
    #[test]
    fn schema_validator_rejects_missing_required() {
        for h in REGISTRY {
            let schema = h.schema();
            let Some(first_req) = schema["required"]
                .as_array()
                .and_then(|r| r.first().and_then(|v| v.as_str()).map(String::from))
            else {
                continue;
            };
            let mut bad = golden::golden_args(h.name());
            let removed = bad
                .as_object_mut()
                .expect("golden 参数必须是对象")
                .remove(&first_req);
            assert!(
                removed.is_some(),
                "{} golden 缺必填参数 {first_req}",
                h.name()
            );
            assert!(
                schema_check::validate(&bad, schema).is_err(),
                "{first_req} 被移除后 schema 校验必须失败: {}",
                h.name()
            );
        }
    }

    /// tool_parity 黄金覆盖清单从注册表派生:REGISTRY 每个工具都有对拍 golden
    /// (除 V2-W1 新增两工具,豁免登记在案——补录归 parity 轮,工具集口径以
    /// schemas/mcp-tools.json 为准)。
    #[test]
    fn tool_parity_golden_covers_registry() {
        // 豁免登记(registered debt):V2-W1 media_thumbs/preview_zone_render
        // 上线时未补 golden;对拍补录属 tools/bench 域(本工单文件域外),
        // 行为证据由单测 media_thumbs_batch_and_cache / preview_zone_render_param_guards 承接。
        const KNOWN_GAP: &[&str] = &["media_thumbs", "preview_zone_render"];
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/bench/golden");
        for h in REGISTRY {
            if KNOWN_GAP.contains(&h.name()) {
                continue;
            }
            assert!(
                dir.join(format!("{}.json", h.name())).is_file(),
                "tool_parity golden 缺工具 {}",
                h.name()
            );
        }
    }
}
