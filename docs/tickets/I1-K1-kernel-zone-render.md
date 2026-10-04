# I1-K1 内核:preview_zone_render 工具 + hidden 轨道渲染验证

来源:docs/upstream/05-ui-feature-iterations.md §I1 M2(硬骨头 B1)。

## 目标
1. 新 MCP 工具 `preview_zone_render {root, startMs, endMs}`:对时间线 [startMs,endMs] 区间
   以半分辨率走既有渲染管线,产物落 `.cutforge/preview-cache/`(内容寻址:工作区指纹
   + 区间 + RENDERER_VERSION),复用 RenderPlan/steps,不复制第二条管线。
2. 实测并落档:track_update.hidden=true 后 render_frame/render 是否尊重(04 缺口④)。
   小修即修;大修只报告不扩scope。

## 边界
- 允许:crates/cutforge-render/**、crates/cutforge-mcp/**、schemas/mcp-tools.json、
  内核侧测试、能力矩阵再生成(若属生成物,docs/adr/0003)。
- 禁止:apps/**、Cargo.lock/Cargo.toml(零新依赖)、一切 git 写操作、删 target。

## 验收
- protocol conformance 增该工具用例;真实 ffmpeg 集成测试按既有门控模式;
- fmt / clippy -p cutforge-render -p cutforge-mcp -D warnings / cargo test -p 两 crate 全绿(引用退出码)。

状态:已完工——zone 工具落地;回炉两次:①xfade 链收紧+源窗钳制(RENDERER_VERSION 11.0,9 新测试)②mix 步纯视频无音轨占位图(6 新测试,cf-1080p zone 实测 exit 0);parity 全绿
