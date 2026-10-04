//! 键位命令注册表(BUG-22 单一真相;A-02 命令面的数据件):
//! 命令 id / 标题 / 快捷键 / 分组收口一张表——`on_key` 查表分发、
//! 设置页速查表从本表生成(消灭双维护)、TC-DESK-KEY-002 冲突检测在此。
//!
//! id 语义与 Web 壳 `js/ui/keymap.js` 注册表同源(play.* / edit.* / clip.* /
//! view.*);桌面与 Web 的具体组合差异(如 S/T 分割、沉浸 Esc)为桌面侧
//! 实装现状,跨壳逐条平价断言留第 3 波(TC-DESK-KEY-001 口径)。

/// 绑定入口类型:键盘组合(参与分发)/ 手势与按钮(仅速查表展示)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// 键盘组合;`shift_any = true` 表示不区分 Shift 档(对齐原 on_key 的 `_` 档)
    Key {
        key: &'static str,
        ctrl: bool,
        shift: bool,
        shift_any: bool,
    },
    /// 按钮/手势入口(传输条、拖拽等;无键盘绑定,不参与分发)
    Gesture,
}

/// 注册表一行 = 一条命令的展示与(可选)键盘绑定。
/// 同一 id 可有多行组合(如重做 = Ctrl+Shift+Z 与 Ctrl+Y),展示时合并。
pub(crate) struct Binding {
    /// 命令 id(与 Web 键位表同源语义;前缀 play./edit./clip./view./capture.)
    pub id: &'static str,
    /// 分组(播放/编辑/视图;设置页展示用)
    pub group: &'static str,
    /// 标题(设置页速查表文案)
    pub label: &'static str,
    /// 展示键位("Ctrl+Shift+Z";Gesture 行为按钮/手势名)
    pub keys: &'static str,
    pub kind: Kind,
}

#[allow(clippy::too_many_arguments)] // 注册表行的静态数据面,8 参即绑定语义全量
const fn key(
    id: &'static str,
    group: &'static str,
    label: &'static str,
    keys: &'static str,
    k: &'static str,
    ctrl: bool,
    shift: bool,
    shift_any: bool,
) -> Binding {
    Binding {
        id,
        group,
        label,
        keys,
        kind: Kind::Key {
            key: k,
            ctrl,
            shift,
            shift_any,
        },
    }
}

const fn gesture(
    id: &'static str,
    group: &'static str,
    label: &'static str,
    keys: &'static str,
) -> Binding {
    Binding {
        id,
        group,
        label,
        keys,
        kind: Kind::Gesture,
    }
}

/// 键位命令注册表(顺序即设置页速查表展示序)。
/// 行为对照原 on_key 内联 match,除上/下键按工单改跨轨遍历外零变化。
#[rustfmt::skip]
pub(crate) static REGISTRY: &[Binding] = &[
    // ---- 播放 ----
    key("play.toggle", "播放", "播放 / 暂停(流畅=引擎 · 精确=逐帧)", "空格", " ", false, false, false),
    key("play.k",      "播放", "暂停",                                "K",      "k", false, false, false),
    key("play.l",      "播放", "正向倍速循环 1→2→4→1(≠1x 自动静音)",   "L",      "l", false, false, false),
    key("play.slow",   "播放", "慢放倍速循环 1→0.5→0.25",             "Shift+L","l", false, true,  false),
    key("play.j",      "播放", "减速方向 4→2→1→0.5→0.25(到 0.25 停)",  "J",      "j", false, false, false),
    key("play.frameBack", "播放", "步退一帧",  "←",        "left",  false, false, false),
    key("play.frameFwd",  "播放", "步进一帧",  "→",        "right", false, false, false),
    key("play.secBack",   "播放", "步退 1 秒", "Shift+←",  "left",  false, true,  false),
    key("play.secFwd",    "播放", "步进 1 秒", "Shift+→",  "right", false, true,  false),
    key("play.home",   "播放", "跳到开头", "Home", "home", false, false, true),
    key("play.end",    "播放", "跳到结尾", "End",  "end",  false, false, true),
    // ---- 编辑 ----
    key("clip.siblingPrev", "编辑", "上一个片段(跨轨遍历)", "↑", "up",   false, false, false),
    key("clip.siblingNext", "编辑", "下一个片段(跨轨遍历)", "↓", "down", false, false, false),
    key("clip.split",     "编辑", "在播放头分割选中片段",   "S", "s", false, false, false),
    key("clip.splitAll",  "编辑", "在播放头分割全部轨道",   "T", "t", false, false, false),
    key("clip.duplicate", "编辑", "复制选中片段到播放头",   "D", "d", false, false, false),
    key("edit.copy",      "编辑", "复制选中片段",           "Ctrl+C", "c", true, false, false),
    key("edit.cut",       "编辑", "剪切选中片段",           "Ctrl+X", "x", true, false, false),
    key("edit.paste",     "编辑", "粘贴到播放头(源轨)",     "Ctrl+V", "v", true, false, false),
    key("edit.closeGap",  "编辑", "关闭播放头所在空隙",     "G", "g", false, false, false),
    key("edit.delete",    "编辑", "删除选中片段",           "Del",       "delete",    false, false, true),
    key("edit.deleteBksp","编辑", "删除选中片段(退格)",     "Backspace", "backspace", false, false, true),
    key("edit.undo",      "编辑", "撤销",                   "Ctrl+Z", "z", true, false, false),
    key("edit.redo",      "编辑", "重做",                   "Ctrl+Shift+Z", "z", true, true,  false),
    key("edit.redo",      "编辑", "重做",                   "Ctrl+Y",       "y", true, false, true),
    // ---- 视图 ----
    key("view.immersiveExit", "视图", "退出沉浸预览", "Esc", "escape", false, false, false),
    key("view.zoomIn",  "视图", "时间轴放大", "+ / =", "=", false, false, true),
    key("view.zoomIn",  "视图", "时间轴放大", "+ / =", "+",  false, false, true),
    key("view.zoomOut", "视图", "时间轴缩小", "−",      "-", false, false, true),
    // ---- 按钮/手势入口(速查展示;实装在面板与传输条) ----
    gesture("loop.toggle",       "播放", "当前片段 A→B 循环(传输条循环键)",                 "循环按钮"),
    gesture("mute.toggle",       "播放", "静音开关(传输条音量/静音键;无声卡提示 toast)",      "静音按钮"),
    gesture("quality.toggle",    "播放", "流畅(引擎直解码)/ 精确(逐帧 render_frame)",   "画质按钮"),
    gesture("capture.screenshot","播放", "当前帧落 <工程>/screenshots/(预览右下)",       "截图按钮"),
    gesture("view.immersive",    "视图", "收起其他面板只留预览(非系统全屏;Esc 退出)",    "沉浸按钮"),
    gesture("scrub.drag",        "播放", "按下拖动直接映射(零动画);松手才 seek",         "拖进度条"),
    gesture("clip.drag",         "编辑", "移动(松手提交;拖拽中本地预览)",                "拖动片段"),
    gesture("playhead.click",    "播放", "跳转播放头",                                    "点标尺/轨道空白"),
    gesture("snap.toggle",       "编辑", "开关片段边缘/播放头吸附",                       "吸附按钮"),
];

/// 键盘组合解析 → 注册表行(精确匹配 key+ctrl;shift_any 行兜底任意 Shift 档)。
pub(crate) fn resolve(k: &str, ctrl: bool, shift: bool) -> Option<&'static Binding> {
    REGISTRY.iter().find(|b| match b.kind {
        Kind::Key {
            key,
            ctrl: c,
            shift: s,
            shift_any,
        } => key == k && c == ctrl && (shift_any || s == shift),
        Kind::Gesture => false,
    })
}

/// TC-DESK-KEY-002:冲突绑定检测——注册表内不得有两组 Key 行抢占同一组合。
/// 返回与候选组合冲突的既有命令 id。本轮由单测守护(注册表零冲突 +
/// 合成冲突必命中);重绑定设置 UI 第 3 波接线,故非测试构建暂无调用方。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn conflict_of(cand: Kind) -> Option<&'static str> {
    let Kind::Key {
        key: ck,
        ctrl: cc,
        shift: cs,
        shift_any: csa,
    } = cand
    else {
        return None;
    };
    REGISTRY.iter().find_map(|b| match b.kind {
        Kind::Key {
            key,
            ctrl,
            shift,
            shift_any,
        } => (key == ck && ctrl == cc && (shift_any || csa || shift == cs)).then_some(b.id),
        Kind::Gesture => None,
    })
}

/// 设置页速查表数据(从注册表生成;同 id 多组合合并为一条,键位串接)。
/// 返回 (分组, 键位组合列表, 标题) 三元组,顺序 = 注册表序。
pub(crate) fn shortcut_rows() -> Vec<(&'static str, Vec<&'static str>, &'static str)> {
    // 先按 id 归并(仅相邻行;注册表内同 id 行连续),再投影为展示三元组
    let mut merged: Vec<(&'static str, &'static str, Vec<&'static str>, &'static str)> = Vec::new();
    for b in REGISTRY {
        match merged.last_mut() {
            Some(last) if last.0 == b.id => last.2.push(b.keys),
            _ => merged.push((b.id, b.group, vec![b.keys], b.label)),
        }
    }
    merged
        .into_iter()
        .map(|(_, group, keys, label)| (group, keys, label))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_ids_unique_keys_present() {
        // 同 id 多组合合法(重做/放大);同 id + 同组合才是真重复
        for (i, a) in REGISTRY.iter().enumerate() {
            for b in REGISTRY.iter().skip(i + 1) {
                let dup = a.id == b.id && a.kind == b.kind;
                let shown = format!("{}/{}", a.id, a.keys);
                assert!(!dup, "注册表重复行:{shown}");
            }
        }
        assert!(
            REGISTRY
                .iter()
                .all(|b| !b.label.is_empty() && !b.keys.is_empty())
        );
        // 命令面覆盖度:独立命令 id ≥ 25(播放/编辑/视图三组)
        let mut ids: Vec<&str> = REGISTRY.iter().map(|b| b.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert!(ids.len() >= 25, "命令数不足:{ids:?}");
    }

    #[test]
    fn registry_no_conflicts() {
        let keys: Vec<(Kind, &str)> = REGISTRY
            .iter()
            .filter(|b| matches!(b.kind, Kind::Key { .. }))
            .map(|b| (b.kind, b.id))
            .collect();
        for i in 0..keys.len() {
            for j in i + 1..keys.len() {
                let Kind::Key {
                    key: k1,
                    ctrl: c1,
                    shift: s1,
                    shift_any: a1,
                } = keys[i].0
                else {
                    unreachable!()
                };
                let Kind::Key {
                    key: k2,
                    ctrl: c2,
                    shift: s2,
                    shift_any: a2,
                } = keys[j].0
                else {
                    unreachable!()
                };
                let conflict = k1 == k2 && c1 == c2 && (a1 || a2 || s1 == s2);
                let id_i = keys[i].1;
                let id_j = keys[j].1;
                assert!(!conflict, "组合冲突:{id_i} 与 {id_j}(key={k1} ctrl={c1})");
            }
        }
    }

    #[test]
    fn conflict_of_detects_duplicate_binding() {
        // 空格已被 play.toggle 占用:候选同组合 → 命中;Gesture 恒 None
        assert_eq!(
            conflict_of(Kind::Key {
                key: " ",
                ctrl: false,
                shift: false,
                shift_any: false
            }),
            Some("play.toggle")
        );
        assert_eq!(
            conflict_of(Kind::Key {
                key: " ",
                ctrl: false,
                shift: true,
                shift_any: false
            }),
            None
        );
        assert_eq!(conflict_of(Kind::Gesture), None);
    }

    // ---- 查表分发对拍原 on_key ----

    #[test]
    fn resolve_matches_original_on_key_table() {
        assert_eq!(resolve(" ", false, false).unwrap().id, "play.toggle");
        assert_eq!(resolve("l", false, false).unwrap().id, "play.l");
        assert_eq!(resolve("l", false, true).unwrap().id, "play.slow");
        assert_eq!(resolve("j", false, false).unwrap().id, "play.j");
        assert_eq!(resolve("k", false, false).unwrap().id, "play.k");
        assert_eq!(resolve("left", false, false).unwrap().id, "play.frameBack");
        assert_eq!(resolve("left", false, true).unwrap().id, "play.secBack");
        assert_eq!(resolve("home", false, true).unwrap().id, "play.home"); // shift 无关档
        assert_eq!(
            resolve("escape", false, false).unwrap().id,
            "view.immersiveExit"
        );
        assert_eq!(resolve("s", false, false).unwrap().id, "clip.split");
        assert_eq!(resolve("t", false, false).unwrap().id, "clip.splitAll");
        assert_eq!(resolve("d", false, false).unwrap().id, "clip.duplicate");
        assert_eq!(resolve("c", true, false).unwrap().id, "edit.copy");
        assert_eq!(resolve("x", true, false).unwrap().id, "edit.cut");
        assert_eq!(resolve("v", true, false).unwrap().id, "edit.paste");
        assert_eq!(resolve("g", false, false).unwrap().id, "edit.closeGap");
        assert_eq!(resolve("z", true, false).unwrap().id, "edit.undo");
        assert_eq!(resolve("z", true, true).unwrap().id, "edit.redo");
        assert_eq!(resolve("y", true, true).unwrap().id, "edit.redo");
        assert_eq!(resolve("=", false, true).unwrap().id, "view.zoomIn");
        assert_eq!(resolve("-", false, false).unwrap().id, "view.zoomOut");
        assert!(resolve("q", false, false).is_none());
        assert!(resolve("delete", false, true).is_some()); // shift 无关档
    }

    #[test]
    fn shortcut_rows_derived_and_merged() {
        let rows = shortcut_rows();
        assert!(rows.len() < REGISTRY.len(), "同 id 多组合应合并");
        // 重做两组合合并为一条
        let redo: Vec<_> = rows
            .iter()
            .filter(|(_, _, label)| *label == "重做")
            .collect();
        assert_eq!(redo.len(), 1);
        let keys = redo[0].1.join(" / ");
        assert!(keys.contains("Ctrl+Shift+Z") && keys.contains("Ctrl+Y"));
        // 手势行(按钮/拖拽)不丢
        assert!(rows.iter().any(|(_, keys, _)| keys[0] == "循环按钮"));
        // 分组只取已知三组,且每行标题非空
        assert!(
            rows.iter()
                .all(|(g, _, label)| matches!(*g, "播放" | "编辑" | "视图") && !label.is_empty())
        );
    }
}
