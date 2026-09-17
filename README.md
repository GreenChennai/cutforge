# CutForge

> **与 OpenCut 的关系**：CutForge 是一个**独立项目**，不是 OpenCut 的官方版本、
> 分支或衍生发行版，与其维护者无隶属或背书关系。我们在领域模型与工程结构上
> 参考了 [OpenCut](https://github.com/OpenCut-app/OpenCut)（MIT，**活跃开发中**）
> 与其经典版 [opencut-classic](https://github.com/OpenCut-app/opencut-classic)
> （MIT，**已于 2026-05-17 归档**）。其 MIT 许可全文见 `LICENSE-OPENCUT.MIT`，
> 完整归属见 `NOTICE.md`。本项目原创部分适用 `LICENSE`（ARL-1.0）。

CutForge 是一个以 Rust 内核为单一实现、服务 Web/桌面/脚本/MCP 多端接入的开源视频编辑器。

- **一个内核，多个壳**：时间线模型、命令与撤销、操作日志（OpLog）、渲染调度只实现一次，Web 壳与桌面壳都是薄壳，杜绝双实现语义漂移。
- **文件是真相源**：工程（`project.json` / `wordline.json` / `cutlist.json` / `notes.json`）即同步面，任何写入者（人、AI、脚本）经同一命令通道产生可审计、可撤销的 Op。
- **为 AI 而生**：内置 MCP server 与脚本宿主；AI 的每次改动可 diff、可回滚，用户在时间轴上打的标注 AI 能读到、能执行、能回执。
- **与 CutFlow 分工而非合并**：[CutFlow](https://github.com/GreenChennai/CutFlow)（Python）负责 S0–S11 视频管线的批量机械工作（转写对齐、粗剪、合成、字幕、烧录、自检）；CutForge 负责"人的手"——交互、预览、标注与精确编辑。两者读写同一份工程文件。

## 状态

M0–M2 已完成并通过门禁。路线图：M0 合规立项 ✓ → M1 契约固化 ✓ → M2 Rust 内核 ✓ → M3 双向同步与标注 → M4 MCP 与脚本 → M5 多端壳 → M6 渲染后端 → M7 开源发布。

- **M0**:ARL-1.0 混合授权三件套、命名核查存档、工具链 pin(与上游一致)、统一门禁入口、CI 骨架。
- **M1**:五份 schema(唯一手写契约)+ 双端代码生成(Python 生成校验器 / Rust `cutforge-schema`)+ 常量单源零漂移 + 迁移器幂等 + 回归集对拍(双端结论逐样本一致)。
- **M2**:`cutforge-core`(领域模型/命令通道/撤销栈/OpLog/三路合并骨架/锚点,行覆盖 ≥80%,wasm32 可构建)+ `cutforge-io`(工程读写/原子写唯一落盘点/锁/备份/媒体探测/轮询 watcher)+ `cutforge-cli`(打开/查询/应用/撤销重做/OpLog + 门禁判定器)。

## 许可

混合授权，三点必须读清（全文见各文件）：

1. 本项目**原创部分**适用 [ARL-1.0](LICENSE)（弱传染：核心文件的修改须回传开源；插件、壳、商业应用可闭源）。**正式发布前协议文本尚待执业律师复核。**
2. 派生/参考自 OpenCut 的部分必须遵守其 **MIT 许可**，全文见 [LICENSE-OPENCUT.MIT](LICENSE-OPENCUT.MIT)（逐字保留，未修改）。
3. CutFlow 的**已发布 MIT 版本（`v0.1.0`–`v0.12` 等 tag）授权不可撤回**；任何许可变更仅对其后发布的新版本生效。

## 开发

工具链与上游 OpenCut 保持一致（`proto` + `moon` + `bun` + `rust 1.97.0`，edition 2024），保留未来接口层回流上游的可能。

```bash
# 门禁(统一完成判定入口,人眼判断不作为通过依据)
python tools/gates/gate.py M0 --json
python tools/gates/gate.py M1 --json
python tools/gates/gate.py M2 --json
```

结果协议:`{"ok":bool,"code":str,"message":str,"data":object}`;退出码 `0`=通过、`2`=门禁失败、`3`=前置/环境缺失、`4`=内部错误。

参与贡献前请读 [CONTRIBUTING.md](CONTRIBUTING.md)。
