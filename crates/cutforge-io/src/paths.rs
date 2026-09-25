// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 阶段目录契约唯一真相源(多风格迭代 M1:目录中文化,目录契约 v2)。
//!
//! 与 CutFlow 侧 `rs_paths.py`(ADR-0046)同构:同一张「逻辑键 → 目录名」映射
//! 两仓各持一份实现,以各自测试锁定零漂移;任何目录改名必须两侧同步。
//! 约定:
//! - 文件名全部 ASCII 不变(project.json / wordline.json / cutlist.json …);
//! - `03_创作素材/artboard` 子目录保留英文(与 artboard 技能自身约定一致);
//! - 工程根的 `notes.json` 与 `.cutforge/` 不属于阶段目录,不在本表(0.4.x 起不变);
//! - `成品/` 为 0.5.0 新增交付区,NEVER_CLEAN——任何清理/重建不得触碰。
//!
//! 0.4.x 的英文目录名保留为 `LEGACY_*`:旧工程**兼容读写、原地保留**,
//! 不做自动迁移(避免在 CutFlow 并行升级期间挪动正在使用的工程);
//! 新建工程一律产新布局。

/// 制作简报(逻辑键 `brief`)。
pub const BRIEF: &str = "00_制作简报";
/// 原始素材(逻辑键 `materials`;只读语义不变)。
pub const MATERIALS: &str = "01_原始素材";
/// 转写与校对(逻辑键 `sensed`)。
pub const SENSED: &str = "02_转写与校对";
/// 创作素材(逻辑键 `assets`)。
pub const ASSETS: &str = "03_创作素材";
/// 创作素材下的动画卡目录:保留英文(与 artboard 技能自身约定一致,改名收益低、对拍成本高)。
pub const ARTBOARD: &str = "artboard";
/// 粗剪决策(逻辑键 `cut`)。
pub const CUT: &str = "04_粗剪决策";
/// 时间线工程(逻辑键 `timeline`;project.json/wordline.json 所在)。
pub const TIMELINE: &str = "05_时间线工程";
/// 成片输出(逻辑键 `output`)。
pub const OUTPUT: &str = "06_成片输出";
/// 内部状态(逻辑键 `state`;备份、阶段记账;不属于交付面)。
pub const STATE: &str = "_内部状态";
/// 交付区(逻辑键 `deliver`;0.5.0 新增;NEVER_CLEAN)。
pub const DELIVER: &str = "成品";

// ---- 工程真相源相对路径(阶段目录 × ASCII 文件名;拼接形态的唯一登记处) ----
/// 工程真相源(IR)。
pub const PROJECT_REL: &str = "05_时间线工程/project.json";
/// 全片时间真相源。
pub const WORDLINE_REL: &str = "05_时间线工程/wordline.json";
/// 粗剪决策单。
pub const CUTLIST_REL: &str = "04_粗剪决策/cutlist.json";
/// 粗剪决策单(已应用回执)。
pub const CUTLIST_APPLIED_REL: &str = "04_粗剪决策/cutlist.applied.json";
/// 标注(工程根,不属于阶段目录;0.4.x 起不变)。
pub const NOTES_REL: &str = "notes.json";

// ---- 0.4.x 旧布局(英文目录)——兼容读写,不自动迁移 ----
pub const LEGACY_BRIEF: &str = "00_brief";
pub const LEGACY_MATERIALS: &str = "01_materials";
pub const LEGACY_SENSED: &str = "02_sensed";
pub const LEGACY_ASSETS: &str = "03_assets";
pub const LEGACY_CUT: &str = "04_cut";
pub const LEGACY_TIMELINE: &str = "05_ir";
pub const LEGACY_OUTPUT: &str = "06_output";
pub const LEGACY_STATE: &str = "_state";

pub const LEGACY_PROJECT_REL: &str = "05_ir/project.json";
pub const LEGACY_WORDLINE_REL: &str = "05_ir/wordline.json";
pub const LEGACY_CUTLIST_REL: &str = "04_cut/cutlist.json";
pub const LEGACY_CUTLIST_APPLIED_REL: &str = "04_cut/cutlist.applied.json";

/// 非工程真相源文件 → 工程内相对路径(新布局;file_states 初始化与落盘的依据)。
pub const FILE_TRUTHS_NEW: [(&str, &str); 3] = [
    ("wordline.json", WORDLINE_REL),
    ("cutlist.json", CUTLIST_REL),
    ("cutlist.applied.json", CUTLIST_APPLIED_REL),
];
/// 同上,旧布局(0.4.x 工程;打开时按盘面择一,整个生命周期保持一致)。
pub const FILE_TRUTHS_LEGACY: [(&str, &str); 3] = [
    ("wordline.json", LEGACY_WORDLINE_REL),
    ("cutlist.json", LEGACY_CUTLIST_REL),
    ("cutlist.applied.json", LEGACY_CUTLIST_APPLIED_REL),
];

/// 布局判定:旧布局工程 = 旧 project.json 在盘且新 project.json 不在。
/// 新旧并存(理论上不该发生)时以新为准。
pub fn is_legacy_layout(root: &Path) -> bool {
    root.join(LEGACY_PROJECT_REL).is_file() && !root.join(PROJECT_REL).is_file()
}

/// 新旧双布局解析现存文件:优先新;新缺失且旧在 → 旧;都不在 → 新(错误信息指向契约位)。
pub fn resolve_rel(root: &Path, new_rel: &str, legacy_rel: &str) -> PathBuf {
    let new = root.join(new_rel);
    if new.exists() {
        return new;
    }
    let old = root.join(legacy_rel);
    if old.exists() { old } else { new }
}

/// 新旧双布局解析现存目录(语义同 [`resolve_rel`])。
pub fn resolve_dir(root: &Path, new_dir: &str, legacy_dir: &str) -> PathBuf {
    resolve_rel(root, new_dir, legacy_dir)
}

/// project.json 的磁盘位置(新旧布局皆认)。
pub fn project_path(root: &Path) -> PathBuf {
    resolve_rel(root, PROJECT_REL, LEGACY_PROJECT_REL)
}

/// 该目录是否为可打开的工程(新旧布局皆认)。
pub fn has_project(root: &Path) -> bool {
    root.join(PROJECT_REL).is_file() || root.join(LEGACY_PROJECT_REL).is_file()
}

/// 当前盘面生效的 project.json 相对路径(空目录/新工程 → 新常量;旧布局 → 旧常量)。
/// 供 /session 等需要把相对路径透传给外部脚本(CutFlow rs_render 等)的面使用。
pub fn project_rel_on_disk(root: &Path) -> &'static str {
    if is_legacy_layout(root) { LEGACY_PROJECT_REL } else { PROJECT_REL }
}

use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{atomic, fsutil};

    /// 零漂移守卫:拼接形态的相对路径必须以对应阶段目录常量开头
    /// (防「改了目录名忘了改拼接串」或反之——两处只能同时错,不会单边漂)。
    #[test]
    fn rel_paths_match_stage_dirs() {
        for (dir, rel) in [
            (TIMELINE, PROJECT_REL),
            (TIMELINE, WORDLINE_REL),
            (CUT, CUTLIST_REL),
            (CUT, CUTLIST_APPLIED_REL),
            (LEGACY_TIMELINE, LEGACY_PROJECT_REL),
            (LEGACY_TIMELINE, LEGACY_WORDLINE_REL),
            (LEGACY_CUT, LEGACY_CUTLIST_REL),
            (LEGACY_CUT, LEGACY_CUTLIST_APPLIED_REL),
        ] {
            assert!(rel.starts_with(&format!("{dir}/")), "{rel} 必须落在 {dir} 下");
        }
    }

    /// 契约面快照(目录契约 v2;与 CutFlow rs_paths.STAGE_DIRS 同构,改任一侧必须同步)。
    #[test]
    fn contract_v2_dirs() {
        assert_eq!(BRIEF, "00_制作简报");
        assert_eq!(MATERIALS, "01_原始素材");
        assert_eq!(SENSED, "02_转写与校对");
        assert_eq!(ASSETS, "03_创作素材");
        assert_eq!(ARTBOARD, "artboard", "artboard 子目录保留英文");
        assert_eq!(CUT, "04_粗剪决策");
        assert_eq!(TIMELINE, "05_时间线工程");
        assert_eq!(OUTPUT, "06_成片输出");
        assert_eq!(STATE, "_内部状态");
        assert_eq!(DELIVER, "成品");
        assert_eq!(NOTES_REL, "notes.json", "工程根 notes.json 不动");
    }

    #[test]
    fn layout_detection_and_resolve() {
        let root = fsutil::temp_dir("paths-layout");
        // 空目录 → 新布局(新建工程的默认形态)
        assert!(!is_legacy_layout(&root));
        assert!(!has_project(&root));
        assert_eq!(project_path(&root), root.join(PROJECT_REL));
        assert_eq!(resolve_dir(&root, OUTPUT, LEGACY_OUTPUT), root.join(OUTPUT));

        // 旧布局工程(0.4.x)
        fsutil::ensure(&root.join(LEGACY_TIMELINE)).unwrap();
        atomic::atomic_write(&root.join(LEGACY_PROJECT_REL), b"{}").unwrap();
        assert!(is_legacy_layout(&root));
        assert!(has_project(&root));
        assert_eq!(project_path(&root), root.join(LEGACY_PROJECT_REL));
        assert_eq!(project_rel_on_disk(&root), LEGACY_PROJECT_REL);
        // 目录解析只在旧目录真实在盘时才回退旧目录
        assert_eq!(resolve_dir(&root, OUTPUT, LEGACY_OUTPUT), root.join(OUTPUT));
        fsutil::ensure(&root.join(LEGACY_OUTPUT)).unwrap();
        assert_eq!(resolve_dir(&root, OUTPUT, LEGACY_OUTPUT), root.join(LEGACY_OUTPUT));

        // 新旧并存(理论上不该发生)时以新为准
        fsutil::ensure(&root.join(TIMELINE)).unwrap();
        atomic::atomic_write(&root.join(PROJECT_REL), b"{}").unwrap();
        assert!(!is_legacy_layout(&root));
        assert_eq!(project_path(&root), root.join(PROJECT_REL));
        fsutil::cleanup(&root);
    }
}
