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

M0–M4 已完成并通过门禁。路线图：M0 合规立项 ✓ → M1 契约固化 ✓ → M2 Rust 内核 ✓ → M3 双向同步与标注 ✓ → M4 MCP 与脚本 ✓ → M5 多端壳 → M6 渲染后端 → M7 开源发布。

- **M0**:ARL-1.0 混合授权三件套、命名核查存档、工具链 pin(与上游一致)、统一门禁入口、CI 骨架。
- **M1**:五份 schema(唯一手写契约)+ 双端代码生成(Python 生成校验器 / Rust `cutforge-schema`)+ 常量单源零漂移 + 迁移器幂等 + 回归集对拍(双端结论逐样本一致)。
- **M2**:`cutforge-core`(领域模型/命令通道/撤销栈/OpLog/三路合并骨架/锚点,行覆盖 ≥80%,wasm32 可构建)+ `cutforge-io`(工程读写/原子写唯一落盘点/锁/备份/媒体探测/轮询 watcher)+ `cutforge-cli`(打开/查询/应用/撤销重做/OpLog + 门禁判定器)。
- **M3**:双向同步全链——三路合并九行判定表零静默覆盖(12,000 组属性测试)、OpLog 回放等价(含 undo/redo 混入)、冲突三方快照落盘(`.cutforge/conflicts/`)、标注(notes.json)读写/结案回执绑定 opIds/锚点重定位(100 组场景零丢失)、阶段脏传播(改 IR 只标 S3+;改字幕只重烧 S8)、往返延迟基准(AI 可见 P95 ≤100ms,实测个位数毫秒)。
- **M4**:MCP 层——单注册表 28 工具(9 查询+13 写+6 编排封装 CutFlow 脚本;M4 时点数字,当前以 `schemas/mcp-tools.json` 为准),stdio 与内嵌 HTTP(127.0.0.1+token)双通道共用同一 dispatch;脚本宿主 `cutforge-script`(批式步骤+策略沙箱,六类逃逸零到达派发器);CutFlow 侧四个桥脚本(rs_editor/rs_notes/rs_oplog/rs_gate)入 doctor 体检与命令速查表。

## 快速开始(编辑器,3 步)

**无需 Rust 工具链**:GitHub Release 下载对应平台压缩包(`cutforge-windows.zip` / `cutforge-linux.zip` / `cutforge-macos.zip`,内含 `cutforge-cli` / `cutforge-mcp` / `cutforge-render` 三个二进制与 `web/` 静态资源,E1-2),解压即用;校验见随包 `SHA256SUMS-<os>.txt`。

1. **启动**:`cutforge-cli serve --open`(推荐;无参数时交互选择工程,回车 = 最近工程),或 `cutforge-mcp serve --root <工程目录> --open`。Windows 也可双击仓库根的 [start-editor.cmd](start-editor.cmd)。
2. **浏览器**:带 `--open` 自动打开;否则手动访问控制台打印的 `http://127.0.0.1:<端口>/?token=<T>`。
3. **编辑与导出**:时间线拖拽 / trim / 分割 / 波纹删,预览(画质代理,空格播放、←/→ 逐帧),检查器,标注,差异面板;导出选 `cutforge` 后端即由本机内核出片,不依赖 CutFlow。

要点:服务仅监听 127.0.0.1;数据面(/rpc /media /session)经 Bearer token 鉴权,重启服务会换新 token;退出 = 在服务窗口按 Ctrl+C。启动自检会逐项报告工程 / Web 资源 / ffmpeg / cutforge-render 的就绪状态与补救命令(E1-6)。预览不含转场 / 特效 / 字幕烧录的最终效果,成片请用导出。

## 许可

混合授权，三点必须读清（全文见各文件）：

1. 本项目**原创部分**适用 [ARL-1.0](LICENSE)（弱传染：核心文件的修改须回传开源；插件、壳、商业应用可闭源）。**正式发布前协议文本尚待执业律师复核。**
2. 派生/参考自 OpenCut 的部分必须遵守其 **MIT 许可**，全文见 [LICENSE-OPENCUT.MIT](LICENSE-OPENCUT.MIT)（逐字保留，未修改）。
3. CutFlow 的**已发布 MIT 版本（`v0.1.0`–`v0.12` 等 tag）授权不可撤回**；任何许可变更仅对其后发布的新版本生效。

## 开发

工具链与上游 OpenCut 保持一致（`proto` + `moon` + `bun` + `rust 1.97.0`，edition 2024），保留未来接口层回流上游的可能。**整体流程与工作区地图见 [docs/FLOW.md](docs/FLOW.md)。**

```bash
# 门禁(统一完成判定入口,人眼判断不作为通过依据)
python tools/gates/gate.py M0 --json
python tools/gates/gate.py M1 --json
python tools/gates/gate.py M2 --json
python tools/gates/gate.py M3 --json
python tools/gates/gate.py M4 --json
```

结果协议:`{"ok":bool,"code":str,"message":str,"data":object}`;退出码 `0`=通过、`2`=门禁失败、`3`=前置/环境缺失、`4`=内部错误。

参与贡献前请读 [CONTRIBUTING.md](CONTRIBUTING.md)。
