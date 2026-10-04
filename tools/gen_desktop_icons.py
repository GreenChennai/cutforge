#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成 apps/desktop/assets/icons/*.svg(A-09/§9.5:桌面壳图标全集,单一生成点)。

口径:
- viewBox 0 0 16 16;几何一律不透明——gpui 0.2.2 的 svg 渲染为 **alpha mask**,
  实际颜色由元素 text_color 提供(故 fill="currentColor" 仅为语义占位);
- 前 15 个与 apps/web/assets/icons.js 的 symbol path **逐字同源**(两壳一份资产);
- 其余为 NLE 扩展自绘(报告 §9.5 清单:传输/编辑/轨道/标注四族)。

重跑幂等:`uv run --no-project python tools/gen_desktop_icons.py`。
"""
from pathlib import Path

OUT = Path(__file__).resolve().parents[1] / "apps" / "desktop" / "assets" / "icons"
OUT.mkdir(parents=True, exist_ok=True)

ICONS = {
    # ===== web assets/icons.js 同源 15 个(path 逐字一致;语义见 web <title>)=====
    "video":       '<path fill="currentColor" d="M2 4h8a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zm10 3.2 3-2.2v6l-3-2.2z"/>',
    "audio":       '<path fill="currentColor" d="M8 1v9.2a2.6 2.6 0 1 1-1.4-2.3V3.5L13 2v6.3a2.6 2.6 0 1 1-1.4-2.3V2.4z"/>',
    "image":       '<path fill="currentColor" d="M2 3h12a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zm1 8h10l-3.2-4.3-2.3 3-1.7-2z"/>',
    "eye":         '<path fill="currentColor" d="M8 3C4.5 3 1.7 5.3.5 8c1.2 2.7 4 5 7.5 5s6.3-2.3 7.5-5C14.3 5.3 11.5 3 8 3zm0 8a3 3 0 1 1 0-6 3 3 0 0 1 0 6zm0-1.5a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3z"/>',
    "eye-off":     '<path fill="currentColor" d="M1.4.6 15.4 14.6l-1 1-2.7-2.7A8.6 8.6 0 0 1 8 13C4.5 13 1.7 10.7.5 8a9.9 9.9 0 0 1 3.3-3.9L.4 1.6zM8 5a3 3 0 0 0-2.8 4L8.9 7.2 9.9 8z"/>',
    "refresh":     '<path fill="currentColor" d="M8 2a6 6 0 0 1 5.7 4.1l-1.9.6A4 4 0 0 0 4.3 6H6v2H1V3h2v1.5A6 6 0 0 1 8 2zm5 9.7V13h2v3h-5v-2h1.6A6 6 0 0 1 2.3 9.9l1.9-.6A4 4 0 0 0 13 11.7z" transform="translate(0 -1)"/>',
    "scissors":    '<path fill="currentColor" d="M9.4 8 14 3.4A2 2 0 1 0 12.6 2L8 6.6 3.4 2A2 2 0 1 0 2 3.4L6.6 8 2 12.6A2 2 0 1 0 3.4 14L8 9.4l4.6 4.6A2 2 0 1 0 14 12.6z"/>',
    "plus":        '<path fill="currentColor" d="M7 2h2v5h5v2H9v5H7V9H2V7h5z"/>',
    "trash":       '<path fill="currentColor" d="M6 2h4v1h4v2H2V3h4zM3 6h10l-.8 8.1a1 1 0 0 1-1 .9H4.8a1 1 0 0 1-1-.9z"/>',
    "copy":        '<path fill="currentColor" d="M4 1h8a1 1 0 0 1 1 1v8h-2V3H4zm-2 3h8a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1z"/>',
    "play":        '<path fill="currentColor" d="M4 2l10 6-10 6z"/>',
    "pause":       '<path fill="currentColor" d="M3 2h4v12H3zM9 2h4v12H9z"/>',
    "film":        '<path fill="currentColor" d="M1 3h14v10H1zm2 2v2h2V5zm8 0v2h2V5zM3 9v2h2V9zm8 0v2h2V9zM7 5v6h2V5z"/>',
    "note":        '<path fill="currentColor" d="M2 2h12a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H8l-4 4v-4H2a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1z"/>',
    "wave":        '<path fill="currentColor" d="M1 7h1v2H1zm2-2h1v6H3zm2-2h1v10H5zm2 1h1v8H7zm2-3h1v14H9zm2 4h1v6h-1zm2 2h1v2h-1z"/>',
    # ===== NLE 扩展:传输(报告 §9.5)=====
    "skip-start":   '<path fill="currentColor" d="M2 3h2v10H2zM14 3v10L6 8z"/>',
    "step-back":    '<path fill="currentColor" d="M3 3h2v10H3zM13 3v10L6 8z"/>',
    "step-forward": '<path fill="currentColor" d="M11 3h2v10h-2zM3 3v10l7-5z"/>',
    "skip-end":     '<path fill="currentColor" d="M12 3h2v10h-2zM2 3v10l8-5z"/>',
    "loop":         '<path fill="currentColor" d="M4.5 5H10a3 3 0 0 1 3 3h-1.6l2.3 2.8L16 8h-1.5A4.5 4.5 0 0 0 10 3.5H4.5zM11.5 11H6a3 3 0 0 1-3-3h1.6L2.3 5.2 0 8h1.5a4.5 4.5 0 0 0 4.5 4.5h5.5z"/>',
    "volume":       '<path fill="currentColor" d="M2 6h3l4-3.5v11L5 10H2z"/><path fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" d="M11.2 5.6a3.4 3.4 0 0 1 0 4.8M13.2 3.6a6.2 6.2 0 0 1 0 8.8"/>',
    "volume-off":   '<path fill="currentColor" d="M2 6h3l4-3.5v11L5 10H2z"/><path fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" d="M11 6l4 4M15 6l-4 4"/>',
    "maximize":     '<path fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" d="M2 6V2h4M10 2h4v4M14 10v4h-4M6 14H2v-4"/>',
    "camera":       '<path fill="currentColor" fill-rule="evenodd" d="M5.8 3.6A1 1 0 0 1 6.6 3h2.8a1 1 0 0 1 .8.4L11.4 5H14a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h2.6zM8 6.6a2.7 2.7 0 1 0 0 5.4 2.7 2.7 0 0 0 0-5.4z"/>',
    # ===== NLE 扩展:时间线编辑 =====
    "split-all":    '<path fill="currentColor" d="M1.5 2h5v2.6h-5zM1.5 6.7h5v2.6h-5zM1.5 11.4h5V14h-5zM9.5 2h5v2.6h-5zM9.5 6.7h5v2.6h-5zM9.5 11.4h5V14h-5z"/><path fill="none" stroke="currentColor" stroke-width="1.3" stroke-dasharray="1.7 1.5" d="M8 1v14"/>',
    "clipboard-paste": '<path fill="currentColor" d="M6 1.5h4V3H6zM4.5 2.5V4h7V2.5h1a1 1 0 0 1 1 1V13a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1V3.5a1 1 0 0 1 1-1z"/>',
    "snowflake":    '<path fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" d="M8 1.5v13M2.4 4.75l11.2 6.5M13.6 4.75 2.4 11.25M6.3 3.2 8 1.5l1.7 1.7M6.3 12.8 8 14.5l1.7-1.7"/>',
    "undo":         '<path fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" d="M5.5 3 2.5 6l3 3M2.5 6h7.75a3.125 3.125 0 0 1 0 6.25H8"/>',
    "redo":         '<path fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" d="m10.5 3 3 3-3 3M13.5 6H5.75a3.125 3.125 0 0 0 0 6.25H8"/>',
    "magnet":       '<path fill="currentColor" d="M3 2h4v6a1 1 0 0 0 2 0V2h4v6A5 5 0 0 1 3 8zM3 2v2.4h4V2zm10 0v2.4H9V2z"/>',
    "gap-close":    '<path fill="currentColor" d="M2 2h2v12H2zM14 7H6.8l2.6-2.6L8 3 3 8l5 5 1.4-1.4L6.8 9H14z"/>',
    "add-track":    '<path fill="currentColor" d="M1.5 2h13v2h-13zM1.5 6h13v2h-13zM7 10h2v1.8h1.8v2H9V16H7v-2.2H5.2v-2H7z"/>',
    "zoom-in":      '<circle cx="6.2" cy="6.2" r="4.5" fill="none" stroke="currentColor" stroke-width="1.5"/><path fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" d="M4.2 6.2h4M6.2 4.2v4M9.7 9.7l4.3 4.3"/>',
    "zoom-out":     '<circle cx="6.2" cy="6.2" r="4.5" fill="none" stroke="currentColor" stroke-width="1.5"/><path fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" d="M4.2 6.2h4M9.7 9.7l4.3 4.3"/>',
    "zoom-fit":     '<path fill="currentColor" d="M9 2h5v5l-1.9-1.9-2.2 2.2-1.2-1.2 2.2-2.2zM7 14H2V9l1.9 1.9 2.2-2.2 1.2 1.2-2.2 2.2z"/>',
    "lock":         '<path fill="currentColor" d="M4 7h8a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V8a1 1 0 0 1 1-1z"/><path fill="none" stroke="currentColor" stroke-width="1.5" d="M5.5 7V4.8a2.5 2.5 0 0 1 5 0V7"/>',
    "lock-open":    '<path fill="currentColor" d="M4 7h8a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V8a1 1 0 0 1 1-1z"/><path fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" d="M5.5 7V4.8A2.5 2.5 0 0 1 10.4 4"/>',
    # ===== NLE 扩展:标注/动效/调色/字幕/视图 =====
    "keyframe":     '<path fill="currentColor" d="M8 1.5 14.5 8 8 14.5 1.5 8z"/>',
    "marker":       '<path fill="currentColor" d="M3 1.5h1.8v13H3zM4.8 2h8.2l-2.3 2.9 2.3 2.9H4.8z"/>',
    "transition":   '<path fill="currentColor" d="M1.5 3.5h6v9h-6z"/><path fill="none" stroke="currentColor" stroke-width="1.4" d="M8.5 3.5h6v9h-6z"/>',
    "motion":       '<path fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" d="M2.5 2.5v11h11"/><path fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" d="M4.5 11c3 0 4.6-1.2 6.5-6"/><path fill="currentColor" d="m10.4 2.8 3 1.2-2.5 1.9z"/>',
    "palette":      '<path fill="currentColor" d="M8 1.5C5 5 3.5 7.3 3.5 9.5a4.5 4.5 0 0 0 9 0c0-2.2-1.5-4.5-4.5-8zM8 12.6a3.1 3.1 0 0 1-3.1-3.1h1.6A1.5 1.5 0 0 0 8 11z"/>',
    "caption":      '<path fill="none" stroke="currentColor" stroke-width="1.4" d="M1.7 3.7h12.6v8.6H1.7z"/><path fill="currentColor" d="M3.7 6h4.4v1.7H3.7zM3.7 9h8.6v1.7H3.7z"/>',
    "solo":         '<path fill="currentColor" d="M8 2a6 6 0 0 0-6 6v5a1 1 0 0 0 1 1h2.3a.7.7 0 0 0 .7-.7V9.7a.7.7 0 0 0-.7-.7H4V8a4 4 0 0 1 8 0v1h-1.3a.7.7 0 0 0-.7.7v3.6a.7.7 0 0 0 .7.7H13a1 1 0 0 0 1-1V8a6 6 0 0 0-6-6z"/>',
    "speed":        '<path fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" d="M3 13A6 6 0 1 1 13 13"/><path fill="currentColor" d="M8 9.2 11.6 4.6 7.3 8.2A1.35 1.35 0 1 0 8 9.2z"/>',
    "close":        '<path fill="currentColor" d="M6.1 8 1.6 3.5 3 2.1 7.5 6.6 12 2.1l1.4 1.4L8.9 8l4.5 4.5-1.4 1.4L7.5 9.4 3 13.9l-1.4-1.4z"/>',
}


def main() -> None:
    for name, body in ICONS.items():
        content = ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">'
                   + body + "</svg>\n")
        (OUT / f"{name}.svg").write_text(content, encoding="utf-8")
    print(f"written {len(ICONS)} icons -> {OUT}")


if __name__ == "__main__":
    main()
