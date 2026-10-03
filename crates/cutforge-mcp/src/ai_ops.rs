// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! AI 协作面(册七 T7.5):改动预演(preview_plan)/ 批准应用(apply_plan)/
//! 标注线程化(note_reply)/ 会话报告(session_report)。
//!
//! ## preview_plan 的 dry-run 隔离证明
//!
//! 预演**不经真工程**:先把工程复制到同卷邻位副本(cutforge_io::scratch,
//! 硬链接优先零拷贝),再把逐项 `args.root` 改写为副本根后重入
//! [`crate::dispatch::dispatch_with_actor`] 单表——与真实写通道同一实现
//! (不建第二套业务逻辑),前置/守卫/schema 校验/Op 产出全部真实发生,
//! 只是发生在副本上。逐项结果从副本 OpLog 增量提取(字段级 before/after),
//! 返回前销毁副本并逐出常驻缓存;真工程盘面逐字节未动。
//!
//! ## apply_plan 的无越权写入
//!
//! 默认拒绝(default-deny):只有 approvals 显式批准(逐项或 approveAll,
//! 显式 reject 恒胜出)的项才被逐项重入单表执行,actor 保持调用方,每项
//! Op 的 causedBy 绑定 planId(审计链);未批准项一律跳过并回执,绝不落地。

use crate::cutlist_ops::notes_op_error;
use crate::registry::{envelope, tool_def};
use cutforge_core::notes::NoteAuthor;
use cutforge_core::oplog::Actor;
use cutforge_io::Workspace;
use serde_json::{Value, json};
use std::path::Path;

/// plan 面不参与的工具:静态面/免锁探测/渲染/编排/打包目录级写——它们不经
/// 工作区命令通道(或副作用在工程外),预演语义不成立,逐项显式拒绝。
const UNPLANNABLE: &[&str] = &[
    "capability_matrix",
    "plugin_validate",
    "preview_plan",
    "apply_plan",
    "session_report",
    "note_reply",
    // 免开工作区(探测/目录级写/派生物)
    "project_new",
    "media_probe",
    "media_browse",
    "render_probe",
    "stage_status",
    "media_peaks",
    "media_thumbnail",
    "media_proxy",
    "audio_beats",
    "lut_import",
    "scope_data",
    "audio_loudness",
    "encode_probe",
    "multicam_sync",
    "otio_import",
    "migrate_layout",
    "library_manage",
    "library_list",
    "library_recover",
    "export_preflight",
    "export_all_variants",
    "media_library",
    "media_import",
    "project_package",
    "project_unpackage",
    // 渲染与编排(副作用在工程外:子进程/成片产物)
    "render",
    "render_run",
    "render_progress",
    "render_frame",
    "render_queue",
    "stage_run",
    "stage_rebuild",
    "verify_run",
    "sync_check",
    "export_jianying",
];

/// 工具是否可进 plan(预演/应用同一准入面;未知工具自然不可)。
pub(crate) fn plannable_tool(name: &str) -> bool {
    tool_def(name).is_some() && !UNPLANNABLE.contains(&name)
}

struct PlanItem {
    index: usize,
    id: Option<String>,
    tool: String,
    args: Value,
}

fn parse_plan(args: &Value) -> Result<Vec<PlanItem>, Value> {
    let Some(items) = args["plan"].as_array() else {
        return Err(envelope(
            false,
            "PRECONDITION_FAILED",
            "缺 plan(非空数组,逐项 {tool, args})",
            json!({}),
        ));
    };
    if items.is_empty() {
        return Err(envelope(
            false,
            "PRECONDITION_FAILED",
            "plan 为空(至少一项)",
            json!({}),
        ));
    }
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let Some(tool) = item["tool"].as_str() else {
            return Err(envelope(
                false,
                "PRECONDITION_FAILED",
                &format!("plan[{i}] 缺 tool"),
                json!({}),
            ));
        };
        if !plannable_tool(tool) {
            return Err(envelope(
                false,
                "PRECONDITION_FAILED",
                &format!(
                    "plan[{i}] 工具 {tool} 不参与 plan 面(渲染/编排/免锁探测/静态面/plan 自身不可预演或批量应用)"
                ),
                json!({}),
            ));
        }
        out.push(PlanItem {
            index: i,
            id: item["id"].as_str().map(String::from),
            tool: tool.to_string(),
            args: item.get("args").cloned().unwrap_or_else(|| json!({})),
        });
    }
    Ok(out)
}

/// planId 缺省生成(pid+纳秒 hash,与 runId 同风格)。
fn new_plan_id() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::process::id().hash(&mut h);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
    format!("plan-{:016x}", h.finish())
}

/// Op 的 before/after 预估瘦身:小值原样(字段级预估可直接读),大值只留摘要
/// (AI 面 token 经济;完整值经 oplog_tail 可查)。
fn slim(v: &Value) -> Value {
    let s = v.to_string();
    if s.len() > 120 {
        json!({"elided": true, "bytes": s.len()})
    } else {
        v.clone()
    }
}

/// 副本 OpLog 增量 → 逐项变更摘要(字段级 before/after 预估)。
fn changes_since(scratch: &Path, since_rev: u64) -> (u64, Vec<Value>) {
    let Ok(ws) = Workspace::open(scratch) else {
        return (since_rev, Vec::new());
    };
    let rev = ws.rev();
    let ops = match ws.engine().query(cutforge_core::engine::Query::OpLogTail {
        since_rev: Some(since_rev),
        actor_kind: None,
    }) {
        cutforge_core::engine::Answer::Ops(ops) => ops,
        _ => return (rev, Vec::new()),
    };
    let rows = ops
        .iter()
        .map(|op| {
            json!({
                "opId": op.op_id,
                "file": op.target.file,
                "path": op.target.path,
                "kind": op.op_kind,
                "summary": op.summary,
                "before": slim(&op.before),
                "after": slim(&op.after),
            })
        })
        .collect();
    (rev, rows)
}

/// T7.5-1 preview_plan(查询):批量变更集在副本工程上逐项 dry-run,
/// 返回逐项 ok/错误码/变更摘要(字段级 before/after 预估)/依赖冲突;
/// 全链预演在副本上防部分应用——真工程不落盘不产 Op。
pub(crate) fn preview_plan_tool(root: &Path, args: &Value, actor: Actor) -> Value {
    let plan = match parse_plan(args) {
        Ok(p) => p,
        Err(e) => return e,
    };
    // 副本工程(同卷邻位;调用方传 root 目录即工程根)
    let scratch = match cutforge_io::scratch::make_scratch_copy(root) {
        Ok(d) => d,
        Err(e) => {
            return envelope(
                false,
                if e.kind() == std::io::ErrorKind::NotFound {
                    "NO_CONFIG"
                } else {
                    "INTERNAL"
                },
                &format!("预演副本构建失败: {e}"),
                json!({}),
            );
        }
    };
    let scratch_s = scratch.to_string_lossy().to_string();
    let rev_from = Workspace::open(&scratch).map(|w| w.rev()).unwrap_or(0);
    let mut cursor = rev_from;
    let mut items = Vec::new();
    let (mut ok_n, mut err_n) = (0usize, 0usize);
    for item in &plan {
        // 逐项重入单一派发表:root 改写为副本根,其余参数原样
        let mut a = item.args.clone();
        if !a.is_object() {
            a = json!({});
        }
        a["root"] = json!(scratch_s);
        let env = crate::dispatch::dispatch_with_actor(&item.tool, &a, actor.clone());
        let (rev_after, changes) = changes_since(&scratch, cursor);
        let ok = env["ok"] == json!(true);
        if ok {
            ok_n += 1;
        } else {
            err_n += 1;
        }
        items.push(json!({
            "index": item.index,
            "id": item.id,
            "tool": item.tool,
            "ok": ok,
            "code": env["code"],
            "message": env["message"],
            "revBefore": cursor,
            "revAfter": rev_after,
            "opIds": env["data"]["opIds"],
            "changes": changes,
        }));
        cursor = rev_after;
    }
    cutforge_io::scratch::remove_scratch(&scratch);
    crate::resident::evict(&scratch_s);
    envelope(
        true,
        "OK",
        "预演完成(副本 dry-run,真工程未动)",
        json!({
            "dryRun": true,
            "revFrom": rev_from,
            "revTo": cursor,
            "items": items,
            "summary": {"total": plan.len(), "ok": ok_n, "err": err_n},
        }),
    )
}

/// T7.5-2 apply_plan(写):仅执行被批准项(逐项原 Op 通道,actor 保持调用方,
/// causedBy 链关联 planId);被拒/未批准项跳过并回执——默认拒绝,无越权写入。
pub(crate) fn apply_plan_tool(root_str: &str, args: &Value, actor: Actor) -> Value {
    let plan = match parse_plan(args) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let ap = args.get("approvals").cloned().unwrap_or(Value::Null);
    if !ap.is_object() {
        // approvals 是契约 required 面:整体缺席按缺参拒(防调用方误以为"缺省=全批")
        return envelope(
            false,
            "PRECONDITION_FAILED",
            "缺 approvals(批准面:{approveAll:true} 或 {approve:[…], reject:[…]};缺省=全不批,显式传入以表达意图)",
            json!({}),
        );
    }
    let approve_all = ap["approveAll"].as_bool().unwrap_or(false);
    let listed = |key: &str, item: &PlanItem| -> bool {
        ap[key].as_array().is_some_and(|list| {
            list.iter().any(|v| {
                v.as_u64().is_some_and(|n| n == item.index as u64)
                    || v.as_str().is_some_and(|s| item.id.as_deref() == Some(s))
            })
        })
    };
    let plan_id = args["planId"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(new_plan_id);
    // 应用前真实 rev(应用后对账:回执 rev 必须单调推进)
    let rev_from = Workspace::open(Path::new(root_str))
        .map(|w| w.rev())
        .unwrap_or(0);
    let mut items = Vec::new();
    let (mut applied, mut skipped, mut rejected) = (0usize, 0usize, 0usize);
    for item in &plan {
        if listed("reject", item) {
            rejected += 1;
            items.push(json!({
                "index": item.index, "id": item.id, "tool": item.tool,
                "ok": false, "skipped": true, "reason": "rejected_by_caller",
            }));
            continue;
        }
        if !approve_all && !listed("approve", item) {
            skipped += 1;
            items.push(json!({
                "index": item.index, "id": item.id, "tool": item.tool,
                "ok": false, "skipped": true, "reason": "not_approved",
            }));
            continue;
        }
        // 逐项原 Op 通道:root 强制真实工程根;causedBy 链关联 planId
        let mut a = item.args.clone();
        if !a.is_object() {
            a = json!({});
        }
        a["root"] = json!(root_str);
        let mut caused = vec![plan_id.clone()];
        if let Some(list) = a["causedBy"].as_array() {
            caused.extend(list.iter().filter_map(|v| v.as_str().map(String::from)));
        }
        a["causedBy"] = json!(caused);
        let env = crate::dispatch::dispatch_with_actor(&item.tool, &a, actor.clone());
        let ok = env["ok"] == json!(true);
        if ok {
            applied += 1;
        }
        items.push(json!({
            "index": item.index, "id": item.id, "tool": item.tool,
            "ok": ok, "code": env["code"], "message": env["message"],
            "opIds": env["data"]["opIds"], "rev": env["data"]["rev"],
        }));
    }
    let rev_to = Workspace::open(Path::new(root_str))
        .map(|w| w.rev())
        .unwrap_or(rev_from);
    envelope(
        true,
        "OK",
        "apply_plan 完成(仅批准项落地;未批准项零写入)",
        json!({
            "planId": plan_id,
            "items": items,
            "applied": applied,
            "skipped": skipped,
            "rejected": rejected,
            "revFrom": rev_from,
            "revTo": rev_to,
        }),
    )
}

/// T7.5-3 note_reply(写):同一标注多轮追加回复(线程 id = 标注 id);
/// 不改 state、不碰 resolved_by——讨论史与结案回执分离。
pub(crate) fn note_reply_tool(ws: &mut Workspace, args: &Value, actor: &Actor) -> Value {
    let (Some(note_id), Some(body)) = (args["noteId"].as_str(), args["body"].as_str()) else {
        return envelope(false, "PRECONDITION_FAILED", "缺 noteId/body", json!({}));
    };
    let author = if args["author"].as_str() == Some("agent") {
        NoteAuthor::Agent
    } else {
        NoteAuthor::User
    };
    match ws.notes_reply(note_id, author, body.into(), actor.clone()) {
        Ok((id, replies)) => envelope(
            true,
            "OK",
            "回复已追加",
            json!({"noteId": id, "replies": replies, "rev": ws.rev()}),
        ),
        Err(e) => notes_op_error(e),
    }
}

/// 摘要中的片段/轨 id 提取(手写扫描,零新依赖):形如 V1-001(片段)/ V1(轨)。
/// 判据:段含大写字母与数字;带 '-' 的两段形为片段 id(rev-1/op-1 等小写首段自然排除)。
fn extract_ids(summary: &str) -> (Vec<String>, Vec<String>) {
    let mut clips = Vec::new();
    let mut tracks = Vec::new();
    for tok in summary.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        if tok.is_empty() || !tok.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            continue;
        }
        let has_digit = tok.chars().any(|c| c.is_ascii_digit());
        if !has_digit {
            continue;
        }
        match tok.split_once('-') {
            Some((head, tail))
                if !tail.is_empty()
                    && tail.chars().all(|c| c.is_ascii_digit())
                    && head.chars().any(|c| c.is_ascii_digit()) =>
            {
                clips.push(tok.to_string())
            }
            None if tok.chars().skip(1).all(|c| c.is_ascii_digit()) => tracks.push(tok.to_string()),
            _ => {}
        }
    }
    (clips, tracks)
}

fn bump(map: &mut std::collections::BTreeMap<String, usize>, k: String) {
    *map.entry(k).or_insert(0) += 1;
}

/// T7.5-4 session_report(查询):一次会话(sinceRev 起)的改动摘要——
/// 改了哪些段/操作分布/耗时/回执率;人话 Markdown + JSON 双形态。
pub(crate) fn session_report_tool(ws: &mut Workspace, args: &Value) -> Value {
    let since = args["sinceRev"].as_u64().unwrap_or(0);
    let ops = match ws.engine().query(cutforge_core::engine::Query::OpLogTail {
        since_rev: Some(since),
        actor_kind: None,
    }) {
        cutforge_core::engine::Answer::Ops(ops) => ops,
        _ => unreachable!(),
    };
    let rev_to = ws.rev();
    let mut by_kind: std::collections::BTreeMap<String, usize> = Default::default();
    let mut by_actor: std::collections::BTreeMap<String, usize> = Default::default();
    let mut by_file: std::collections::BTreeMap<String, usize> = Default::default();
    let mut clip_hits: std::collections::BTreeMap<String, usize> = Default::default();
    let mut track_hits: std::collections::BTreeMap<String, usize> = Default::default();
    for op in &ops {
        // opKind 的 kebab-case 串(set/split/resolve-conflict…)与 actor kind 小写
        let kind_str = serde_json::to_value(op.op_kind)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "unknown".into());
        bump(&mut by_kind, kind_str);
        bump(&mut by_actor, format!("{:?}", op.actor.kind).to_lowercase());
        bump(&mut by_file, op.target.file.clone());
        let (clips, tracks) = extract_ids(&op.summary);
        for c in clips {
            bump(&mut clip_hits, c);
        }
        for t in tracks {
            bump(&mut track_hits, t);
        }
    }
    // 标注面与回执率:结案标注中绑定 opIds 的比例(4.9 纪律的量化面)
    let notes = ws.notes();
    let (mut n_open, mut n_resolved, mut n_rejected, mut n_orphan, mut n_replies, mut n_with_ops) =
        (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    for n in notes.notes() {
        match n.state {
            cutforge_core::notes::NoteState::Open => n_open += 1,
            cutforge_core::notes::NoteState::Resolved => {
                n_resolved += 1;
                if n.resolved_by.as_ref().is_some_and(|r| !r.op_ids.is_empty()) {
                    n_with_ops += 1;
                }
            }
            cutforge_core::notes::NoteState::Rejected => n_rejected += 1,
            cutforge_core::notes::NoteState::Orphan => n_orphan += 1,
        }
        n_replies += n.thread.as_ref().map(|t| t.len()).unwrap_or(0);
    }
    let receipt_rate = if n_resolved > 0 {
        n_with_ops as f64 / n_resolved as f64
    } else {
        1.0
    };
    let (started_at, ended_at, duration_ms) = match (ops.first(), ops.last()) {
        (Some(f), Some(l)) => (f.ts.clone(), l.ts.clone(), {
            let parse = |t: &str| t.parse::<chrono_like::Ts>().ok();
            match (parse(&f.ts), parse(&l.ts)) {
                (Some(a), Some(b)) => b.ms.saturating_sub(a.ms),
                _ => 0,
            }
        }),
        _ => (String::new(), String::new(), 0),
    };
    // 人话 Markdown(壳/CLI/AI 都可直接展示)
    let fmt_map = |m: &std::collections::BTreeMap<String, usize>| -> String {
        m.iter()
            .map(|(k, v)| format!("{k}×{v}"))
            .collect::<Vec<_>>()
            .join("、")
    };
    let clips_desc = if clip_hits.is_empty() {
        "无片段改动".to_string()
    } else {
        clip_hits
            .iter()
            .map(|(k, v)| format!("{k}({v} Op)"))
            .collect::<Vec<_>>()
            .join("、")
    };
    let markdown = format!(
        "## 会话改动报告(rev-{since} → rev-{rev_to})\n\n\
         - 时间窗:{started_at} → {ended_at}(耗时 {}ms)\n\
         - 改动规模:{} 个 Op;操作分布:{}\n\
         - 改动段:{clips_desc};涉及轨:{}\n\
         - 标注:结案 {n_resolved} / 在办 {n_open} / 否决 {n_rejected} / 孤儿 {n_orphan};线程回复 {n_replies} 条;结案回执率 {:.0}%\n\
         - 参与者:{}\n",
        duration_ms,
        ops.len(),
        if by_kind.is_empty() {
            "无".to_string()
        } else {
            fmt_map(&by_kind)
        },
        if track_hits.is_empty() {
            "无".to_string()
        } else {
            track_hits.keys().cloned().collect::<Vec<_>>().join("、")
        },
        receipt_rate * 100.0,
        if by_actor.is_empty() {
            "无".to_string()
        } else {
            fmt_map(&by_actor)
        },
    );
    envelope(
        true,
        "OK",
        "会话报告",
        json!({
            "sinceRev": since,
            "revTo": rev_to,
            "opCount": ops.len(),
            "startedAt": started_at,
            "endedAt": ended_at,
            "durationMs": duration_ms,
            "byKind": by_kind,
            "byActor": by_actor,
            "byFile": by_file,
            "touchedClips": clip_hits,
            "touchedTracks": track_hits,
            "notes": {
                "open": n_open, "resolved": n_resolved, "rejected": n_rejected,
                "orphan": n_orphan, "replies": n_replies,
                "receiptRate": receipt_rate,
            },
            "markdown": markdown,
        }),
    )
}

// RFC3339 毫秒解析(耗时统计用;手写零依赖——timeutil 无公开解析器)
mod chrono_like {
    pub struct Ts {
        pub ms: u64,
    }
    impl std::str::FromStr for Ts {
        type Err = ();
        fn from_str(s: &str) -> Result<Self, Self::Err> {
            // 形如 2026-10-01T21:44:31.677Z / +08:00 / -05:00;日期段的 '-' 不得当时区切
            let t = s.strip_suffix('Z').unwrap_or(s);
            let t = match t.find('+') {
                Some(i) => &t[..i],
                None => match t.char_indices().skip(10).find(|(_, c)| *c == '-') {
                    Some((i, _)) => &t[..i], // 负时区偏移只可能出现在日期(前 10 位)之后
                    None => t,
                },
            };
            let (date, time) = t.split_once('T').unwrap_or((t, ""));
            let mut dp = date.split('-');
            let y: u64 = dp.next().ok_or(())?.parse().map_err(|_| ())?;
            let mo: u64 = dp.next().ok_or(())?.parse().map_err(|_| ())?;
            let d: u64 = dp.next().ok_or(())?.parse().map_err(|_| ())?;
            let mut tp = time.split([':', '.']);
            let h: u64 = tp.next().unwrap_or("0").parse().map_err(|_| ())?;
            let mi: u64 = tp.next().unwrap_or("0").parse().map_err(|_| ())?;
            let se: u64 = tp.next().unwrap_or("0").parse().map_err(|_| ())?;
            let ms: u64 = tp.next().unwrap_or("0").parse().map_err(|_| ())?;
            // 平滑纪元日数(y-m-d → days;误差对本工具的耗时统计无关紧要)
            let days = 367 * y - (7 * (y + (mo + 9) / 12)) / 4 + (275 * mo) / 9 + d - 719_556;
            Ok(Ts {
                ms: ((days * 86_400 + h * 3_600 + mi * 60 + se) * 1_000) + ms,
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// RFC3339 三形态(Z / 正偏移 / 负偏移)解析;偏移忽略(耗时统计的同一时钟
        /// 前提下墙钟一致即够;日期 '-' 不得当时区切)。
        #[test]
        fn ts_parse_shapes_and_delta() {
            let a: Ts = "2026-10-01T21:44:31.677Z".parse().unwrap();
            let b: Ts = "2026-10-01T21:44:33.307Z".parse().unwrap();
            assert_eq!(b.ms - a.ms, 1630);
            let c: Ts = "2026-10-01T21:44:33.307+08:00".parse().unwrap();
            let d: Ts = "2026-10-01T13:44:33.307-05:00".parse().unwrap();
            assert_eq!(c.ms, b.ms, "偏移被忽略,墙钟主体一致");
            assert_ne!(c.ms, d.ms, "不同墙钟串解析不同(不做时区换算,诚实降级)");
        }
    }
}
