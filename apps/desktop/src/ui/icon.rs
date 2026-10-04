//! 桌面壳图标系统(A-09/§9.5):**禁止文本字形作图标**。
//!
//! 资产:`apps/desktop/assets/icons/*.svg`(46 个,16×16 viewBox,几何不透明;
//! gpui 0.2.2 的 svg 渲染为 alpha mask,实际颜色由元素 `text_color` 提供——
//! 即 SVG 的 `currentColor` 语义在 GPUI 侧由 text color 承担)。其中 15 个与
//! `apps/web/assets/icons.js` 的 symbol **path 逐字同源**(一份资产,两处渲染;
//! 生成器 `tools/gen_desktop_icons.py`)。`Icon::icon_at` 的 size 参数即
//! §9.5 要求的 16/20 两档渲染口径(单文件 16×16 无损缩放到 20)。
//!
//! 资产内嵌([`Assets`] 实现 gpui `AssetSource`,`include_bytes!` 编译期打包):
//! 二进制自包含,无运行时文件依赖;`main.rs` 经
//! `Application::new().with_assets(ui::icon::Assets)` 接线。
//!
//! 静态门禁:TC-DESK-ICON-002(tools/gates/a1.py `desktop-glyph-icon`)扫描
//! `apps/desktop/src` 的字形字面量(▶✂⧉❄↶↷…含 timeline 旧"卑"乱码),清零为绿。

// 本目录为 token/图标/动效**基建层**:部分槽位与 helper 由第 4 波(逐面板
// 美化,报告 §9.6)按需消费;门禁 clippy -D warnings 下的 dead_code 豁免
// 仅限本目录(与裸色值/字形/散写 ms 的"唯一定义点"豁免同口径)。
#![allow(dead_code)]
use std::borrow::Cow;

use sable::gpui::{AssetSource, Hsla, Result, SharedString, Styled as _, Svg, px, svg};

/// 图标枚举(单一定义点;变体名 = 资产文件名)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    // —— 媒体类型(web 同源)——
    /// 视频
    Video,
    /// 音频
    Audio,
    /// 图片
    Image,
    /// 成片
    Film,
    /// 标注
    Note,
    /// 波形
    Wave,
    // —— 轨道 ——
    /// 显示轨道
    Eye,
    /// 隐藏轨道
    EyeOff,
    /// 锁定
    Lock,
    /// 解锁
    LockOpen,
    /// 独奏
    Solo,
    // —— 传输 ——
    /// 播放
    Play,
    /// 暂停
    Pause,
    /// 逐帧后退
    StepBack,
    /// 逐帧前进
    StepForward,
    /// 回到开头
    SkipStart,
    /// 跳到结尾
    SkipEnd,
    /// 循环
    Loop,
    /// 音量
    Volume,
    /// 静音
    VolumeOff,
    /// 全屏/沉浸
    Maximize,
    /// 截图
    Camera,
    /// 刷新
    Refresh,
    // —— 编辑 ——
    /// 分割
    Scissors,
    /// 全分割(全部轨在播放头切开)
    SplitAll,
    /// 复制
    Copy,
    /// 粘贴
    ClipboardPaste,
    /// 删除
    Trash,
    /// 关闭
    Close,
    /// 冻结帧
    Snowflake,
    /// 撤销
    Undo,
    /// 重做
    Redo,
    /// 吸附(磁铁)
    Magnet,
    /// 关闭空隙
    GapClose,
    // —— 视图/时间线 ——
    /// 新增
    Plus,
    /// 加轨
    AddTrack,
    /// 缩放放大
    ZoomIn,
    /// 缩放缩小
    ZoomOut,
    /// 缩放适配
    ZoomFit,
    /// 关键帧(菱形)
    Keyframe,
    /// 标记(旗标)
    Marker,
    /// 转场
    Transition,
    /// 动效
    Motion,
    /// 调色
    Palette,
    /// 字幕
    Caption,
    /// 倍速(表盘)
    Speed,
}

impl Icon {
    /// 资产路径(gpui `svg()` 经 AssetSource 解析)。
    pub fn path(self) -> &'static str {
        self.asset_pair().0
    }

    /// (路径, 文件字节);`asset_pair` 是枚举↔内嵌资产的唯一对齐点。
    fn asset_pair(self) -> (&'static str, &'static [u8]) {
        match self {
            Icon::Video => (
                "icons/video.svg",
                include_bytes!("../../assets/icons/video.svg"),
            ),
            Icon::Audio => (
                "icons/audio.svg",
                include_bytes!("../../assets/icons/audio.svg"),
            ),
            Icon::Image => (
                "icons/image.svg",
                include_bytes!("../../assets/icons/image.svg"),
            ),
            Icon::Film => (
                "icons/film.svg",
                include_bytes!("../../assets/icons/film.svg"),
            ),
            Icon::Note => (
                "icons/note.svg",
                include_bytes!("../../assets/icons/note.svg"),
            ),
            Icon::Wave => (
                "icons/wave.svg",
                include_bytes!("../../assets/icons/wave.svg"),
            ),
            Icon::Eye => (
                "icons/eye.svg",
                include_bytes!("../../assets/icons/eye.svg"),
            ),
            Icon::EyeOff => (
                "icons/eye-off.svg",
                include_bytes!("../../assets/icons/eye-off.svg"),
            ),
            Icon::Lock => (
                "icons/lock.svg",
                include_bytes!("../../assets/icons/lock.svg"),
            ),
            Icon::LockOpen => (
                "icons/lock-open.svg",
                include_bytes!("../../assets/icons/lock-open.svg"),
            ),
            Icon::Solo => (
                "icons/solo.svg",
                include_bytes!("../../assets/icons/solo.svg"),
            ),
            Icon::Play => (
                "icons/play.svg",
                include_bytes!("../../assets/icons/play.svg"),
            ),
            Icon::Pause => (
                "icons/pause.svg",
                include_bytes!("../../assets/icons/pause.svg"),
            ),
            Icon::StepBack => (
                "icons/step-back.svg",
                include_bytes!("../../assets/icons/step-back.svg"),
            ),
            Icon::StepForward => (
                "icons/step-forward.svg",
                include_bytes!("../../assets/icons/step-forward.svg"),
            ),
            Icon::SkipStart => (
                "icons/skip-start.svg",
                include_bytes!("../../assets/icons/skip-start.svg"),
            ),
            Icon::SkipEnd => (
                "icons/skip-end.svg",
                include_bytes!("../../assets/icons/skip-end.svg"),
            ),
            Icon::Loop => (
                "icons/loop.svg",
                include_bytes!("../../assets/icons/loop.svg"),
            ),
            Icon::Volume => (
                "icons/volume.svg",
                include_bytes!("../../assets/icons/volume.svg"),
            ),
            Icon::VolumeOff => (
                "icons/volume-off.svg",
                include_bytes!("../../assets/icons/volume-off.svg"),
            ),
            Icon::Maximize => (
                "icons/maximize.svg",
                include_bytes!("../../assets/icons/maximize.svg"),
            ),
            Icon::Camera => (
                "icons/camera.svg",
                include_bytes!("../../assets/icons/camera.svg"),
            ),
            Icon::Refresh => (
                "icons/refresh.svg",
                include_bytes!("../../assets/icons/refresh.svg"),
            ),
            Icon::Scissors => (
                "icons/scissors.svg",
                include_bytes!("../../assets/icons/scissors.svg"),
            ),
            Icon::SplitAll => (
                "icons/split-all.svg",
                include_bytes!("../../assets/icons/split-all.svg"),
            ),
            Icon::Copy => (
                "icons/copy.svg",
                include_bytes!("../../assets/icons/copy.svg"),
            ),
            Icon::ClipboardPaste => (
                "icons/clipboard-paste.svg",
                include_bytes!("../../assets/icons/clipboard-paste.svg"),
            ),
            Icon::Trash => (
                "icons/trash.svg",
                include_bytes!("../../assets/icons/trash.svg"),
            ),
            Icon::Close => (
                "icons/close.svg",
                include_bytes!("../../assets/icons/close.svg"),
            ),
            Icon::Snowflake => (
                "icons/snowflake.svg",
                include_bytes!("../../assets/icons/snowflake.svg"),
            ),
            Icon::Undo => (
                "icons/undo.svg",
                include_bytes!("../../assets/icons/undo.svg"),
            ),
            Icon::Redo => (
                "icons/redo.svg",
                include_bytes!("../../assets/icons/redo.svg"),
            ),
            Icon::Magnet => (
                "icons/magnet.svg",
                include_bytes!("../../assets/icons/magnet.svg"),
            ),
            Icon::GapClose => (
                "icons/gap-close.svg",
                include_bytes!("../../assets/icons/gap-close.svg"),
            ),
            Icon::Plus => (
                "icons/plus.svg",
                include_bytes!("../../assets/icons/plus.svg"),
            ),
            Icon::AddTrack => (
                "icons/add-track.svg",
                include_bytes!("../../assets/icons/add-track.svg"),
            ),
            Icon::ZoomIn => (
                "icons/zoom-in.svg",
                include_bytes!("../../assets/icons/zoom-in.svg"),
            ),
            Icon::ZoomOut => (
                "icons/zoom-out.svg",
                include_bytes!("../../assets/icons/zoom-out.svg"),
            ),
            Icon::ZoomFit => (
                "icons/zoom-fit.svg",
                include_bytes!("../../assets/icons/zoom-fit.svg"),
            ),
            Icon::Keyframe => (
                "icons/keyframe.svg",
                include_bytes!("../../assets/icons/keyframe.svg"),
            ),
            Icon::Marker => (
                "icons/marker.svg",
                include_bytes!("../../assets/icons/marker.svg"),
            ),
            Icon::Transition => (
                "icons/transition.svg",
                include_bytes!("../../assets/icons/transition.svg"),
            ),
            Icon::Motion => (
                "icons/motion.svg",
                include_bytes!("../../assets/icons/motion.svg"),
            ),
            Icon::Palette => (
                "icons/palette.svg",
                include_bytes!("../../assets/icons/palette.svg"),
            ),
            Icon::Caption => (
                "icons/caption.svg",
                include_bytes!("../../assets/icons/caption.svg"),
            ),
            Icon::Speed => (
                "icons/speed.svg",
                include_bytes!("../../assets/icons/speed.svg"),
            ),
        }
    }

    /// 渲染一个图标(SVG 元素;颜色 = `color`,即 web `currentColor` 口径)。
    ///
    /// `size`:标准两档 16(`ICON_S`)/20(`ICON_M`);svg 无损缩放,非标尺寸
    /// (如轨道头 10px 小徽标)同样合法。
    pub fn icon_at(self, size: f32, color: Hsla) -> Svg {
        svg()
            .path(SharedString::from(self.path()))
            .text_color(color)
            .size(px(size))
    }
}

/// 标准图标尺寸:工具行/传输条(§9.5 16 档)。
pub const ICON_S: f32 = 16.0;
/// 标准图标尺寸:面板/空态强调(§9.5 20 档)。
pub const ICON_M: f32 = 20.0;

/// 全部枚举变体(资产存在性/完整性测试与注册表遍历用)。
pub const ALL: &[Icon] = &[
    Icon::Video,
    Icon::Audio,
    Icon::Image,
    Icon::Film,
    Icon::Note,
    Icon::Wave,
    Icon::Eye,
    Icon::EyeOff,
    Icon::Lock,
    Icon::LockOpen,
    Icon::Solo,
    Icon::Play,
    Icon::Pause,
    Icon::StepBack,
    Icon::StepForward,
    Icon::SkipStart,
    Icon::SkipEnd,
    Icon::Loop,
    Icon::Volume,
    Icon::VolumeOff,
    Icon::Maximize,
    Icon::Camera,
    Icon::Refresh,
    Icon::Scissors,
    Icon::SplitAll,
    Icon::Copy,
    Icon::ClipboardPaste,
    Icon::Trash,
    Icon::Close,
    Icon::Snowflake,
    Icon::Undo,
    Icon::Redo,
    Icon::Magnet,
    Icon::GapClose,
    Icon::Plus,
    Icon::AddTrack,
    Icon::ZoomIn,
    Icon::ZoomOut,
    Icon::ZoomFit,
    Icon::Keyframe,
    Icon::Marker,
    Icon::Transition,
    Icon::Motion,
    Icon::Palette,
    Icon::Caption,
    Icon::Speed,
];

/// 内嵌资产源(图标 SVG;`include_bytes!` 编译期打包,二进制自包含)。
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ALL
            .iter()
            .find(|icon| icon.path() == path)
            .map(|icon| Cow::Borrowed(icon.asset_pair().1)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ALL
            .iter()
            .map(|icon| icon.path())
            .filter(|p| p.starts_with(path))
            .map(SharedString::from)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A-09 资产底量:枚举覆盖 ≥40(报告 §9.5 清单 40~50 个)。
    #[test]
    fn icon_set_size() {
        assert!(ALL.len() >= 40, "图标全集仅 {} 个(<40)", ALL.len());
        assert!(ALL.len() <= 60, "图标全集 {} 个(>60,疑冗余)", ALL.len());
    }

    /// 每个变体的内嵌资产存在且形似 SVG(有 <svg 与 viewBox="0 0 16 16")。
    #[test]
    fn assets_embedded_and_wellformed() {
        for icon in ALL {
            let (path, bytes) = icon.asset_pair();
            let text = std::str::from_utf8(bytes).unwrap_or_else(|e| panic!("{path} 非 UTF-8:{e}"));
            assert!(text.contains("<svg"), "{path} 缺 <svg 根");
            assert!(
                text.contains("viewBox=\"0 0 16 16\""),
                "{path} viewBox 非 16×16"
            );
        }
    }

    /// 路径唯一(枚举↔文件一一对应,无别名漂移)。
    #[test]
    fn paths_unique() {
        let mut paths: Vec<&str> = ALL.iter().map(|i| i.path()).collect();
        paths.sort_unstable();
        let n = paths.len();
        paths.dedup();
        assert_eq!(paths.len(), n, "Icon 路径存在重复");
    }

    /// AssetSource 命中自检(load 命中字节;list 列出全集;未知名返回 None)。
    #[test]
    fn asset_source_roundtrip() {
        let assets = Assets;
        let got = assets
            .load("icons/play.svg")
            .expect("load 不应失败")
            .expect("icons/play.svg 应命中");
        assert!(!got.is_empty());
        assert!(assets.load("icons/nope.svg").expect("ok").is_none());
        let listed = assets.list("icons/").expect("list 不应失败");
        assert_eq!(listed.len(), ALL.len());
    }

    /// web 同源底量:15 个 web 图标(icons.js)必须都在枚举内(两壳一份资产)。
    #[test]
    fn web_parity_icons_present() {
        let web_icons = [
            "icons/video.svg",
            "icons/audio.svg",
            "icons/image.svg",
            "icons/eye.svg",
            "icons/eye-off.svg",
            "icons/refresh.svg",
            "icons/scissors.svg",
            "icons/plus.svg",
            "icons/trash.svg",
            "icons/copy.svg",
            "icons/play.svg",
            "icons/pause.svg",
            "icons/film.svg",
            "icons/note.svg",
            "icons/wave.svg",
        ];
        for path in web_icons {
            assert!(
                ALL.iter().any(|i| i.path() == path),
                "web 同源图标 {path} 不在枚举内"
            );
        }
    }
}
