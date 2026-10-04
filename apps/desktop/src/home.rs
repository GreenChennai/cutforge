//! 开始界面(剪映对标 docs/upstream/04 S1):品牌栏 + 新建工程 hero 卡 +
//! 最近工程网格。选择后 spawn 自身 `--root <目录>` 进入编辑器(编辑器视图
//! 零改动;内核子进程随编辑器壳生命周期)。
//!
//! recent.json 落 `%APPDATA%\cutforge-desktop\`(编辑器启动时也写入,两侧
//! 共用同一真相文件;纯壳态,不属工程数据)。

use std::io::Write as _;
use std::path::PathBuf;

use sable::gpui::WeakEntity;
use sable::gpui::prelude::FluentBuilder as _;
use sable::gpui::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use sable::widgets::prelude::{SpacingTokens, h_flex, v_flex};
use sable::widgets::theme::theme;
use sable::widgets::tokens::{FONT_SIZE_CAPTION, FONT_SIZE_HEADING};

use crate::ui::icon::Icon;
use crate::ui::theme as cf_theme;

/// recent.json 读写。
fn recent_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|base| {
        PathBuf::from(base)
            .join("cutforge-desktop")
            .join("recent.json")
    })
}

pub fn load_recent() -> Vec<String> {
    let Some(p) = recent_path() else {
        return Vec::new();
    };
    std::fs::read_to_string(p)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| {
            v.get("recent")
                .and_then(serde_json::Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
        })
        .unwrap_or_default()
}

/// 记录一次打开(去重置顶,上限 12;坏目录滤除)。
pub fn record_recent(root: &str) {
    let Some(p) = recent_path() else { return };
    if !std::path::Path::new(root).join("project.json").is_file()
        && !std::path::Path::new(root).exists()
    {
        return;
    }
    let mut items: Vec<String> = load_recent().into_iter().filter(|x| x != root).collect();
    items.insert(0, root.to_string());
    items.truncate(12);
    let _ = std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new(".")));
    let json = serde_json::json!({ "recent": items });
    if let Ok(mut f) = std::fs::File::create(&p) {
        let _ = f.write_all(
            serde_json::to_string_pretty(&json)
                .unwrap_or_default()
                .as_bytes(),
        );
    }
}

/// Home 全局句柄(hero 卡 click 闭包只有 App;经此桥接实体)。
struct HomeHandle(WeakEntity<HomeApp>);
impl sable::gpui::Global for HomeHandle {}

pub struct HomeApp {
    recents: Vec<String>,
    /// 点击反馈(状态文本)
    status: String,
}

impl HomeApp {
    pub fn new(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let app = cx.new(|_| Self {
            recents: load_recent(),
            status: String::new(),
        });
        cx.set_global(HomeHandle(app.downgrade()));
        let _ = window;
        app
    }

    /// 打开工程:spawn 自身 --root + 退出(编辑器视图零改动)。
    fn open_project(root: String, cx: &mut Context<Self>) {
        record_recent(&root);
        let exe = std::env::current_exe().ok();
        cx.spawn(async move |_, cx| {
            if let Some(exe) = exe {
                let _ = std::process::Command::new(exe)
                    .arg("--root")
                    .arg(&root)
                    .spawn();
            }
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
    }
}

/// 新建工程:默认根 + 时间戳目录 → cutforge-cli new → 打开。
fn new_project(cx: &mut Context<HomeApp>) {
    let default_root = std::env::var_os("CUTFORGE_PROJECTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("D:\\cutforge-projects"));
    let stamp = {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("工程-{t}")
    };
    let dir = default_root.join(stamp);
    let cli = crate::kernel::find_cli();
    cx.spawn(async move |this, cx| {
        let result = match cli {
            Some(cli) => std::process::Command::new(cli)
                .arg("new")
                .arg(&dir)
                .output(),
            None => Err(std::io::Error::other("未找到 cutforge-cli")),
        };
        let _ = this.update(cx, |home, cx| match result {
            Ok(out) if out.status.success() => {
                HomeApp::open_project(dir.to_string_lossy().into_owned(), cx);
            }
            Ok(out) => {
                home.status = format!(
                    "创建失败:{}",
                    String::from_utf8_lossy(&out.stderr)
                        .chars()
                        .take(120)
                        .collect::<String>()
                );
                cx.notify();
            }
            Err(e) => {
                home.status = format!("创建失败:{e}");
                cx.notify();
            }
        });
    })
    .detach();
}

/// 网格卡片(最近工程)。
fn recent_card(
    ix: usize,
    root: String,
    colors: &sable::widgets::tokens::ColorTokens,
    weak: WeakEntity<HomeApp>,
) -> impl IntoElement + use<> {
    let name = std::path::Path::new(&root)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.clone());
    div()
        .id(sable::gpui::ElementId::Name(format!("recent-{ix}").into()))
        .w(px(148.0))
        .rounded(px(6.0))
        .overflow_hidden()
        .bg(colors.surface_1)
        .border_1()
        .border_color(colors.border_subtle)
        .cursor_pointer()
        .hover(|s| s.border_color(colors.accent))
        .child(
            div()
                .h(px(80.0))
                .w_full()
                .bg(colors.surface_2)
                .flex()
                .items_center()
                .justify_center()
                // A-09:封面占位字形 → SVG 图标(首帧封面留第 4 波)
                .child(Icon::Play.icon_at(20.0, colors.text_secondary)),
        )
        .child(
            v_flex()
                .px(px(SpacingTokens::XS))
                .py(px(4.0))
                .gap(px(1.0))
                .child(
                    div()
                        .text_size(px(FONT_SIZE_CAPTION))
                        .text_color(colors.text_primary)
                        .child(name)
                        .truncate(),
                )
                .child(
                    div()
                        .text_size(px(9.0))
                        .text_color(colors.text_secondary)
                        .child(root.clone())
                        .truncate(),
                ),
        )
        .on_click(move |_, _, cx: &mut App| {
            if let Some(home) = weak.upgrade() {
                let root = root.clone();
                home.update(cx, |_, cx| HomeApp::open_project(root, cx));
            }
        })
}

impl Render for HomeApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme(cx).colors;
        let status = self.status.clone();
        let recents = self.recents.clone();
        let weak = cx.entity().downgrade();

        v_flex()
            .size_full()
            .bg(colors.surface_0)
            .text_color(colors.text_primary)
            .child(
                // —— 顶部品牌栏 ——
                h_flex()
                    .w_full()
                    .h(px(44.0))
                    .px(px(SpacingTokens::SM))
                    .items_center()
                    .gap(px(SpacingTokens::XS))
                    .bg(colors.surface_1)
                    .border_b_1()
                    .border_color(colors.border_subtle)
                    .child(
                        div()
                            .text_size(px(FONT_SIZE_HEADING))
                            .text_color(colors.accent)
                            .child("CutForge"),
                    )
                    .child(
                        div()
                            .text_size(px(FONT_SIZE_CAPTION))
                            .text_color(colors.text_secondary)
                            .child("视频创作工作台"),
                    ),
            )
            .child(
                // —— 主区 ——
                div()
                    .id("home-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(SpacingTokens::LG))
                    .child(
                        v_flex()
                            .gap(px(SpacingTokens::LG))
                            // hero:新建工程品牌色块(§9.6 ①:第 4 波换首帧封面)
                            .child(
                                div()
                                    .id("home-new")
                                    .h(px(120.0))
                                    .w_full()
                                    .rounded(px(8.0))
                                    .bg(cf_theme::semantic().accent)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap(px(SpacingTokens::SM))
                                    .cursor_pointer()
                                    .hover(|s| s.opacity(0.92))
                                    .on_click(move |_, _, cx: &mut App| {
                                        if let Some(home) = cx
                                            .try_global::<HomeHandle>()
                                            .and_then(|h| h.0.upgrade())
                                        {
                                            home.update(cx, |_, cx| new_project(cx));
                                        }
                                    })
                                    .child(
                                        div()
                                            .w(px(30.0))
                                            .h(px(30.0))
                                            .rounded_full()
                                            .bg(colors.surface_0.opacity(0.85))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            // A-09:占位字形 → SVG 图标
                                            .child(Icon::Plus.icon_at(16.0, colors.text_primary)),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(16.0))
                                            .text_color(colors.surface_0)
                                            .child("开始创作"),
                                    ),
                            )
                            // —— 最近工程 ——
                            .child(
                                v_flex()
                                    .gap(px(SpacingTokens::SM))
                                    .child(
                                        div()
                                            .text_size(px(FONT_SIZE_HEADING))
                                            .text_color(colors.text_primary)
                                            .child("本地工程"),
                                    )
                                    .when(recents.is_empty(), |c| {
                                        c.child(
                                            div()
                                                .text_size(px(FONT_SIZE_CAPTION))
                                                .text_color(colors.text_secondary)
                                                .child("暂无最近工程;点上方「开始创作」新建(目录可在 CUTFORGE_PROJECTS 配置)"),
                                        )
                                    })
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap(px(SpacingTokens::SM))
                                            .children(
                                                recents.iter().enumerate().map(|(i, root)| {
                                                    recent_card(
                                                        i,
                                                        root.clone(),
                                                        &colors,
                                                        weak.clone(),
                                                    )
                                                    .into_any_element()
                                                }),
                                            ),
                                    ),
                            )
                            .when(!status.is_empty(), |c| {
                                c.child(
                                    div()
                                        .text_size(px(FONT_SIZE_CAPTION))
                                        .text_color(colors.danger)
                                        .child(status),
                                )
                            }),
                    ),
            )
    }
}
