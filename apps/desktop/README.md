# apps/desktop(GPUI 桌面壳)

**状态:降级为观察项(ADR-0039,计划书 7.6 风险 R2 的预案)。**

上游 OpenCut 的桌面壳自述 "Very early. Right now this is just a window that opens",
GPUI 0.2.2 生态尚在演进且对 Linux/WSL 有硬性平台约束。按计划书既定降级路径:
本项目先以 **wasm 内核 + Web 壳(apps/web)** 交付多端能力,桌面壳待 GPUI
0.3+ 或上游 crates/* 落地后复评——内核单一实现不变,壳只是薄皮,届时以
`cutforge-wasm` 同源投影接入 GPUI。

恢复为阻断门禁(M5-4)的条件:GPUI 稳定版发布 + 本仓引入 `apps/desktop`
二进制并通过 Windows 冒烟(开窗+加载工程+渲染时间线)。
