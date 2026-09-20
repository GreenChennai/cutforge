# ADR-0006:CutForge 定位升级为"可独立起步的编辑器"(空工程模板 + 导入)

日期:2026-09-20 · 状态:已采纳(2026-09-20 迭代落地)
关联:ITERATION-PLAN-v2.0 B11/E3 · ADR-0003(能力矩阵是生成物) · README 定位段

## 背景

B11 指出:CutForge 结构上曾被锁死为"CutFlow 工程的编辑器"——`serve --root` 必填、没有空工程起步路径,导致"手里没有 CutFlow 工程就进不去编辑器"。这与"像剪映一样用"的目标(目标 3)直接冲突:一个编辑器的入口不应该是"先去跑另一条管线"。

## 决策

1. **提供空 IR 模板 + 新建路径**:脚手架(空工程模板:project.json 骨架 + 默认轨)→ CLI `new` 子命令与 MCP `project_new` 工具(同一脚手架,不另造并行实现)→ `clip_add` / 素材浏览完成导入。
2. **保留"打开 CutFlow 工程"能力**:`serve --root` 语义不变;定位从"附属工具"扩大为"可独立起步的编辑器,也能打开 CutFlow 工程",两者共用同一份工程文件与同一套命令通道。
3. **独立起步必须有独立验收**:以"从零剪"e2e 为准——新建空工程到导出成片全程不依赖 CutFlow;能力矩阵(ADR-0003)相应扩容。

## 取舍

- 定位扩大 → 需要独立的工程样板与门禁(scaffold 成为工程文件契约的一部分,改动即契约变更)。
- 换来:目标 3"像剪映一样用"达成;新用户零门槛进入;CutFlow 依赖从"必需"降级为"可选增强"。

## 被否决的替代

- **维持"CutFlow 工程附属工具"定位**:否决——没有 CutFlow 工程就进不去,目标 3 无法达成。
- **只加 CLI `new` 不进 MCP**:否决——AI 同样需要"从零起步"能力,两入口必须同走一个脚手架,否则制造双实现漂移面。

## 后果

- README / NOTICE 的定位措辞随之更新;新增"空工程起步"e2e 并入验收。
- 工程文件契约新增"合法空工程"这一基线形态,迁移器与校验器必须接受它。

## 落地证据

- `crates/cutforge-io/src/scaffold.rs`:`scaffold_project` / `new_project_value` 空工程模板与建盘(原子落盘走工程唯一写入点)。
- CLI:`cutforge-cli` 的 `new` 子命令(`new <目录> --slug <名> --fps 30 --track video,audio` 形态,README 快速开始收录)。
- MCP:`project_new` 工具(B11-1,与 CLI 同走 `cutforge_io::scaffold`),登记于 `schemas/mcp-tools.json`(能力矩阵同步再生成,`tools/check_doc_counts.py` 零漂移)。
- e2e:`tools/e2e_from_zero.py`「从零剪」全链(CLI new → 导入素材 → 改字段 → 导出成片时长对拍,纯服务端路径、全程不依赖 CutFlow)。
