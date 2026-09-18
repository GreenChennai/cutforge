# CutForge 渲染后端能力对等矩阵(V2 诚实口径,计划书 §5 / ADR-0003)

> **本表由 `docs/capability-matrix.json`(唯一真相源)生成;MCP `capability_matrix` 工具与之同源。**
> status 只认**实码 + 对拍夹具证据**,四值封闭:`achieved`(有专属夹具)/`partial`(字段被契约接受、渲染端不完整消费)/`missing`(契约字段存在、渲染零消费)/`optional`。
> V1 版本(2026-09-18)宣称"必达 13/13 达成、整体 93.3%"。V2 审查(ITERATION-PLAN-v2.0 §5)逐项 grep 实码对照后证明其中 4 项零代码、2 项半实现、1 项实现即 Bug——本版按实码全部改判。**降级不是倒退,是把地基从沙地换成岩层**;M11 逐项补齐并回填证据。

| # | 能力 | 剪映 5.9 | ffmpeg 后端 | cutforge 后端(V2 实码) | 要求 | 证据 / M11 目标 |
|---|---|---|---|---|---|---|
| 1 | 视频片段裁剪/排序 | 支持 | 支持 | ✅ achieved(scale/pad/fps/concat) | 必达 | segment/concat 管线 + 契约对拍 |
| 2 | 变速 0.25–4x | 支持 | 支持 | ❌ missing(clip.speed 零消费) | 必达 | M11:setpts/atempo+曲线 |
| 3 | 音量/淡入淡出 | 支持 | 支持 | ⚠️ partial(volume M8 起生效;fade 不消费) | 必达 | render_matrix_fixture / M11:afade |
| 4 | 位置/缩放/旋转 | 支持 | 支持 | ⚠️ partial(仅画面适配 pad/scale) | 必达 | M11:overlay/rotate |
| 5 | 转场(三级语法) | 支持 | 支持 | ❌ missing(compose 直通) | 必达 | M11:xfade 链+尾帧扩展 |
| 6 | 关键词 | 支持 | 不支持 | ⬜ optional | 可选 | — |
| 7 | 花字/描边/底衬 | 支持 | 支持 | ✅ achieved(ASS styles) | 必达 | 字幕样式表 + 烧录链 |
| 8 | 音效落点 | 支持 | 支持 | ✅ achieved(M8 修复:adelay 落点) | 必达 | **修复前实现即 Bug(全部 0 秒炸响)**;render_matrix_fixture 起播断言 |
| 9 | BGM ducking | 支持 | 支持 | ❌ missing(bgm 零引用) | 必达 | M11:侧链 amix→asplit |
| 10 | 蒙版 | 支持 | 支持 | ⬜ optional | 可选 | — |
| 11 | 冻结帧补长 | 支持 | 支持 | ✅ achieved(tpad clone 尾帧) | 必达 | ADR-0027,纯动画必需 |
| 12 | punch-in 变焦 | 支持 | 支持 | ❌ missing(punchIn 零消费) | 必达 | M11:zoompan |
| 13 | 字幕(ASS 烧录) | 支持 | 支持 | ✅ achieved(subtitles 滤镜,最后叠) | 必达 | S8 护栏,M6 对拍实测 |
| 14 | 多画幅变体 | 支持 | 支持 | ✅ achieved(M8 修复:缓存键并入 canvas+fps) | 必达 | **修复前缓存跨画幅污染(P0-3)**;render_matrix_fixture 分辨率断言;真·分叉 encode 在 M11 |
| 15 | 工程可继续精修 | 原生 | 不支持 | ✅ achieved(OpLog/undo 语义为真) | 必达 | M8 P0-1/P0-5 修复:file_level_undo + concurrent_writes_no_loss 夹具 |

## 结论(M8-5 诚实口径)

- 必达 13 项:**实码达成 7 项**(1/7/8/11/13/14/15),半实现 2 项(3/4),未实现 4 项(2/5/9/12)
- 可选 2 项:0 项实现
- **整体 = 7/15 ≈ 47%**(V1 虚报 93.3%)。M11 门禁 M11-1:每项一个专属合成用例,cutforge 与 rs_render 双跑对拍,证据自动回填 JSON

## 与 V1 版本的差异说明

| 项 | V1 判 | V2 实判 | 原因 |
|---|---|---|---|
| 2/5/9/12 | ✅ 达成 | ❌ missing | 渲染端无任何对应代码,夹具不覆盖 |
| 3/4 | ✅ 达成 | ⚠️ partial | 字段入契约但 mix/compose 不消费(volume 于 M8 P0-4 修复后生效) |
| 8 | ✅ 达成 | ✅(先 🐛) | 有实现但无 adelay——所有音效 0 秒同时炸响;M8 修复并以夹具锁定 |
| 14 | ✅ 达成 | ✅(先 🐛) | 缓存键不含 canvas——先渲 9x16 再渲 16x9 命中前者分段;M8 修复并以夹具锁定 |

## 对拍记录(2026-09-18,合成媒体 10s/1080×1920@30;基础链真实性证据,V1 复核仍站得住)

| 指标 | rs_render(Golden) | cutforge-render | 差 | 阈值 |
|---|---|---|---|---|
| 成片时长 | 10.033s | 10.000s | 33.3ms | ≤1 帧 ✅ |
| 总线响度 | −14.00 LUFS | −14.02 LUFS | 0.02 LU | ≤0.5 LU ✅ |
| 字幕↔wordline↔成片偏移中位 | — | 0.0ms | — | ≤40ms ✅ |
| QC(黑帧/冻结/VFR/响度) | — | 全过 | — | ✅ |
| 二次渲染(段级缓存) | — | 0.7s(首次 2.8s) | — | 观察 ✅ |
| 剪映 5.9 草稿生成 | 正常 | — | — | 退路未破坏 ✅ |

> 注:上表证明**七步管线顺序与基础配方**真实,不证明 #2/5/9/12 等高级能力的存在——这正是 V1 误读的根源(管线对拍 ≠ 能力覆盖)。
