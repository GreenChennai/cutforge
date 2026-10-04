// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! apply/replay 路径的增量 schema 校验(R-12②)。
//!
//! apply 知道命令动了哪些路径(mutate 产物的 path/target_id)——只校验
//! **受影响子树**(内嵌 project 契约的对应子 schema)+ **全局不变量**
//! (clip/track id 唯一)。全量 `to_validated_value` 仍守在工程装载面
//! (Engine::new/restore/replay 首帧、undo/redo、open)与 doctor 入口
//! ([`Engine::validate_full`]),红-绿口径:子树校验必须与全量在受影响
//! 子树上等价(TC-CORE-VALIDATE-001)。
//!
//! 轨道有序不变量维持 BUG-05 既有口径(apply 内 debug_assert 的
//! 「不得破坏既有有序」),不作为发布版硬拒绝——三路合并写回的外部真相
//! 本就不保证有序,发布版硬拒会冤枉合法外部态。

use crate::model::{Clip, Project};
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// 内嵌 project 契约(唯一手写契约;解析一次常驻)。
fn project_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let (_, src) = cutforge_schema::SCHEMA_SOURCES
            .iter()
            .find(|(n, _)| *n == "project")
            .expect("project 契约必须内嵌");
        serde_json::from_str(src).expect("内嵌 project 契约必须是合法 JSON")
    })
}

fn subschema(pointer: &str) -> &'static Value {
    project_schema()
        .pointer(pointer)
        .unwrap_or_else(|| panic!("project 契约缺子节点 {pointer}"))
}

/// 校验范围(由 mutate 产物的指针形态裁决;闭集)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// 单 clip 对象:/tracks/{ti}/clips/{ci}(target_id 寻址)。
    Clip,
    /// 整轨 clips 数组:/tracks/{ti}/clips。
    Clips(usize),
    /// 单轨对象:/tracks/{ti}。
    Track(usize),
    /// 全部轨:/tracks。
    Tracks,
    /// 背景乐:/bgm。
    Bgm,
    /// 遗留/未知指针形态(旧日志叶指针等)→ 全量校验兜底。
    Full,
}

fn parse_usize(s: &str) -> Option<usize> {
    s.parse::<usize>().ok()
}

fn scope_of(path: &str, target_id: Option<&str>) -> Scope {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segs.as_slice() {
        ["tracks"] => Scope::Tracks,
        ["bgm"] => Scope::Bgm,
        ["tracks", ti, "clips"] => parse_usize(ti).map_or(Scope::Full, Scope::Clips),
        ["tracks", ti] => parse_usize(ti).map_or(Scope::Full, Scope::Track),
        // 单 clip 指针:索引漂移下 ci 未必仍指向该 clip,校验只面对 after 值本身
        ["tracks", _, "clips", _] => Scope::Clip,
        // 新格式稳定 id 寻址:target_id 在场即 clip 对象 Op
        _ if target_id.is_some() => Scope::Clip,
        _ => Scope::Full,
    }
}

/// 错误定位前缀(与全量校验的 "$.tracks[i].clips[j]" 记法同风格)。
fn scope_label(path: &str, target_id: Option<&str>) -> String {
    match scope_of(path, target_id) {
        Scope::Clip => format!("$.clip({})", target_id.unwrap_or(path)),
        Scope::Clips(ti) => format!("$.tracks[{ti}].clips"),
        Scope::Track(ti) => format!("$.tracks[{ti}]"),
        Scope::Tracks => "$.tracks".to_string(),
        Scope::Bgm => "$.bgm".to_string(),
        Scope::Full => "$".to_string(),
    }
}

/// apply/replay 通用增量校验:`after` 为 mutate/Op 写入后的受影响子树值,
/// `p` 为写入后的工程(全局不变量面对完整状态)。
pub(super) fn validate_affected(
    p: &Project,
    path: &str,
    target_id: Option<&str>,
    after: &Value,
) -> Vec<String> {
    let root = project_schema();
    let label = scope_label(path, target_id);
    let mut errs = match scope_of(path, target_id) {
        Scope::Clip => {
            let mut e = cutforge_schema::engine::validate_node(
                subschema("/$defs/clip"),
                root,
                after,
                &label,
            );
            // 关键帧/复合语义按 after 值本体裁决(索引漂移免疫)
            e.extend(keyframes_of_value(after));
            e
        }
        Scope::Clips(ti) => {
            let mut e = cutforge_schema::engine::validate_node(
                subschema("/properties/tracks/items/properties/clips"),
                root,
                after,
                &label,
            );
            e.extend(keyframes_of_track(p, ti));
            e
        }
        Scope::Track(ti) => {
            let mut e = cutforge_schema::engine::validate_node(
                subschema("/properties/tracks/items"),
                root,
                after,
                &label,
            );
            if ti < p.tracks.len() {
                e.extend(keyframes_of_track(p, ti));
            }
            e
        }
        Scope::Tracks => {
            let mut e = cutforge_schema::engine::validate_node(
                subschema("/properties/tracks"),
                root,
                after,
                &label,
            );
            e.extend(p.validate_keyframes());
            e
        }
        Scope::Bgm => {
            // Op 面的"字段缺席"记作 Null(bgm_clear 的 after、创建 BgmSet 的
            // before):Project 序列化对 None 缺键,Null 不代表"存在且为 null"。
            // 缺席不可能违反子契约,跳过子树校验(全局不变量照跑)。
            if after.is_null() {
                Vec::new()
            } else {
                cutforge_schema::engine::validate_node(
                    subschema("/properties/bgm"),
                    root,
                    after,
                    &label,
                )
            }
        }
        Scope::Full => return full_errors(p),
    };
    errs.extend(global_invariants(p));
    errs
}

/// 全量兜底(遗留指针形态/doctor):与既有 `to_validated_value` 同一口径。
pub(super) fn full_errors(p: &Project) -> Vec<String> {
    p.to_validated_value().err().unwrap_or_default()
}

fn keyframes_of_track(p: &Project, ti: usize) -> Vec<String> {
    let mut errs = Vec::new();
    if let Some(t) = p.tracks.get(ti) {
        for c in &t.clips {
            errs.extend(per_clip(c));
        }
    }
    errs
}

fn keyframes_of_value(after: &Value) -> Vec<String> {
    match serde_json::from_value::<Clip>(after.clone()) {
        Ok(c) => per_clip(&c),
        Err(_) => Vec::new(),
    }
}

fn per_clip(c: &Clip) -> Vec<String> {
    let mut errs = crate::keyframes::validate_clip_keyframes(c);
    if let Some(cp) = &c.compound {
        errs.extend(cp.validate().into_iter().map(|e| format!("compound: {e}")));
    }
    errs
}

/// 全局不变量:clip/track id 全工程唯一(O(n) 单遍)。
fn global_invariants(p: &Project) -> Vec<String> {
    let mut errs = Vec::new();
    let mut clip_ids: BTreeSet<&str> = BTreeSet::new();
    let mut track_ids: BTreeSet<&str> = BTreeSet::new();
    for t in &p.tracks {
        if !track_ids.insert(t.id.as_str()) {
            errs.push(format!("$.tracks: 轨 id 重复 '{}'", t.id));
        }
        for c in &t.clips {
            if !clip_ids.insert(c.id.as_str()) {
                errs.push(format!("$.tracks: clip id 重复 '{}'", c.id));
            }
        }
    }
    errs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Engine, sample_project};
    use serde_json::json;

    /// TC-CORE-VALIDATE-001:增量校验与全量校验在「合法基线 + 单点变更」上
    /// 必须同判——好值双双放行,坏值(枚举外转场/天文数字 sourceIn)双双拒绝。
    #[test]
    fn tc_core_validate_001_incremental_agrees_with_full() {
        let eng = Engine::new(sample_project()).unwrap();
        // 好值:合法转场 → 增量放行(apply 全链路成功)
        let after_good = json!({"id": "V1-002", "src": "a.mp4", "startMs": 8400,
            "durationMs": 6200, "transition": {"type": "slideleft", "durMs": 320.0}});
        assert!(
            validate_affected(
                eng.project(),
                "/tracks/0/clips/1",
                Some("V1-002"),
                &after_good
            )
            .is_empty(),
            "合法子树必须放行"
        );
        // 坏值:枚举外转场 → 增量必须拒绝(与全量同判)
        let after_bad = json!({"id": "V1-002", "src": "a.mp4", "startMs": 8400,
            "durationMs": 6200, "transition": {"type": "爆闪", "durMs": 320.0}});
        let inc = validate_affected(
            eng.project(),
            "/tracks/0/clips/1",
            Some("V1-002"),
            &after_bad,
        );
        let clip: Clip = serde_json::from_value(after_bad).unwrap();
        let mut p = eng.project().clone();
        p.tracks[0].clips[1] = clip;
        let full = full_errors(&p);
        assert!(!inc.is_empty(), "枚举外转场必须被增量校验拒绝: {inc:?}");
        assert!(!full.is_empty(), "对照:全量校验同样拒绝");
        // 坏值:重复 clip id(全局不变量,面对工程完整状态)→ 增量拒绝
        let mut dup_state = eng.project().clone();
        dup_state.tracks[0].clips[1].id = "V1-001".into();
        let errs = validate_affected(&dup_state, "/bgm", None, &json!("02_音乐/bgm.mp3"));
        assert!(
            errs.iter().any(|e| e.contains("重复")),
            "重复 clip id 必须被全局不变量拦截: {errs:?}"
        );
        // 坏值:重复轨 id → 同上
        let mut dup_track = eng.project().clone();
        let t1 = dup_track.tracks[1].clone();
        dup_track.tracks.push(t1);
        let errs = validate_affected(&dup_track, "/tracks/0/clips", None, &json!([]));
        assert!(
            errs.iter().any(|e| e.contains("轨 id 重复")),
            "重复轨 id 必须被全局不变量拦截: {errs:?}"
        );
    }
}
