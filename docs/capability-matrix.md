# CutForge 渲染后端能力对等矩阵(V2 · M11 追平后口径,计划书 §5 / ADR-0003)

> **本表由 `docs/capability-matrix.json`(唯一真相源)生成;MCP `capability_matrix` 工具与之同源。**
> status 只认**实码 + 对拍夹具证据**,四值封闭:`achieved`(有专属夹具)/`partial`(字段被契约接受、渲染端不完整消费)/`missing`(契约字段存在、渲染零消费)/`optional`。
> V1 版本(2026-09-18)宣称"必达 13/13 达成、整体 93.3%"。V2 审查(ITERATION-PLAN-v2.0 §5)逐项 grep 实码对照后证明其中 4 项零代码、2 项半实现、1 项实现即 Bug——本版按实码全部改判。**降级不是倒退,是把地基从沙地换成岩层**;M11 逐项补齐并回填证据。

| # | 能力 | 剪映 5.9 | ffmpeg 后端 | cutforge 后端(V2 实码) | 要求 | 证据 / M11 目标 |
|---|---|---|---|---|---|---|
| 1 | 视频片段裁剪/排序 | 支持 | 支持 | ✅ achieved(scale/pad/fps/concat) | 必达 | segment/concat 管线 + 契约对拍 |
| 2 | 变速 0.25–4x | 支持 | 支持 | ✅ achieved(setpts/atempo;A4 起分段恒速链) | 必达 | M11:setpts/atempo;A4-BE2:曲线分段(filter_complex 分支 trim/setpts/fps/concat),时长=分段积分对拍 |
| 3 | 音量/淡入淡出 | 支持 | 支持 | ⚠️ partial(volume M8 起生效;fade 不消费) | 必达 | render_matrix_fixture / M11:afade |
| 4 | 位置/缩放/旋转 | 支持 | 支持 | ✅ achieved(overlay;A4 起 rotation/crop/flip 契约字段+渲染链) | 必达 | M11:overlay;A4-BE2(T4.9):变换链 裁剪→翻转→旋转→画幅归一,90° transpose 实渲夹具 |
| 5 | 转场(三级语法) | 支持 | 支持 | ✅ achieved(xfade 链+尾帧扩展;A4-BE3a:offset 修复为名义时长口径,截断虫修复) | 必达 | M11:零漂移夹具;A4-BE3a:容器与视频流双 4.000s + 120 帧锁 |
| 6 | 关键词 | 支持 | 不支持 | ⬜ optional | 可选 | — |
| 7 | 花字/描边/底衬 | 支持 | 支持 | ✅ achieved(ASS styles) | 必达 | 字幕样式表 + 烧录链 |
| 8 | 音效落点 | 支持 | 支持 | ✅ achieved(M8 修复:adelay 落点) | 必达 | **修复前实现即 Bug(全部 0 秒炸响)**;render_matrix_fixture 起播断言 |
| 9 | BGM ducking | 支持 | 支持 | ✅ achieved(sidechain 侧链) | 必达 | M11:on/off 能量差夹具 |
| 10 | 蒙版 | 支持 | 支持 | ⬜ optional | 可选 | — |
| 11 | 冻结帧补长 | 支持 | 支持 | ✅ achieved(tpad clone;A4 起 freezeMs 定格真消费) | 必达 | ADR-0027;A4-BE2(T4.4):-t 截断到定格点+tpad 一次补足,与变速/倒放组合单测锁定 |
| 12 | punch-in 变焦 | 支持 | 支持 | ✅ achieved(中心裁剪紧构图) | 必达 | M11:帧字节改变夹具 |
| 13 | 字幕(ASS 烧录) | 支持 | 支持 | ✅ achieved(subtitles 滤镜,最后叠) | 必达 | S8 护栏,M6 对拍实测 |
| 14 | 多画幅变体 | 支持 | 支持 | ✅ achieved(M8 修复:缓存键并入 canvas+fps) | 必达 | **修复前缓存跨画幅污染(P0-3)**;render_matrix_fixture 分辨率断言;真·分叉 encode 在 M11 |
| 15 | 工程可继续精修 | 原生 | 不支持 | ✅ achieved(OpLog/undo 语义为真) | 必达 | M8 P0-1/P0-5 修复:file_level_undo + concurrent_writes_no_loss 夹具 |
| 16 | 曲线变速 speedCurve | 支持 | 支持 | ✅ achieved(A4-BE2:分段积分源消耗+恒速段拼接) | 必达 | parity_matrix 曲线夹具(时长对拍=分段积分,两段曲线实渲) |
| 17 | 倒放 reverse | 支持 | 支持 | ✅ achieved(A4-BE2:视频 reverse+PTS 重盖/音频 areverse,先于变速) | 必达 | 短片夹具首末帧对调断言;整段缓冲内存风险已在契约与渲染链注释声明(长素材先切短再倒) |
| 18 | 画布范围约束 64–7680 偶数 | 支持 | 支持 | ✅ achieved(A4-BE2/ADR-0015:schema+mcp-tools+scaffold 三面闸) | 必达 | 双端引擎 multipleOf 对拍边界样本(64/66/7678/7680 过,63/65/7681 拒);canvasAllowed 降格预设推荐集 |
| 19 | 转场库 50+(目录化) | 支持 | 支持 | ✅ achieved(A4-BE3a/T4.5:58 项目录 tr.* 直通 + acrossfade) | 必达 | 五分类各 2 项实渲视频流零漂移;acrossfade 音频流零漂移 + 转场窗交叉淡变凹陷;GET /catalogs 下发;缩略帧生成器(docs/design/transitions/) |
| 20 | 特效库(combo 叠加) | 支持 | 支持 | ✅ achieved(A4-BE3a/T4.6:fx 注册表 11 项 + combo 上限 3) | 必达 | mono 去色/vignette 角部衰减/grain 时变噪声/mosaic 块归并像素级夹具;未注册 fxId 逐项降级 WARN |
| 21 | 动效库(入场/出场) | 支持 | 支持 | ✅ achieved(A4-BE3a/T4.6:motion 目录 19 项,既有 6+4 首次全部真实渲染) | 必达 | fadeIn 首帧亮度 + slideInLeft 黑底平移夹具;不能真实渲染的动效不进目录(诚实纪律) |
| 22 | 调色一级校色(色温/色调/曝光/对比/高光阴影/饱和度/LGG) | 支持 | 支持 | ✅ achieved(册五 T5.2:clip.grade → colorbalance/eq/colorchannelmixer) | 必达 | parity 夹具 G1 LGG 色偏像素断言/G4 饱和度归零=灰度(fx.mono 同口径);grade 缺省链零变化 |
| 23 | RGB/亮度曲线 | 支持 | 支持 | ✅ achieved(册五 T5.2:grade.curves 点集 → curves 滤镜,master=亮度近似) | 必达 | parity 夹具 G2 曲线提亮亮度断言;端点补全/乱序归一单测锁定 |
| 24 | LUT(.cube 导入/库/应用) | 部分 | 支持 | ✅ achieved(册五 T5.2:lut_import 校验(parse_cube 3D 主格式)+ .cutforge/luts/ + grade.lut lut3d 应用) | 必达 | parity 夹具 G3 LUT 前后帧差;LUT 内容哈希入段缓存键(改文件必 miss) |
| 25 | HSL 限定器 | 支持 | 支持 | ❌ missing(册五 T5.2:IR 承载防丢,渲染端登记降级 WARN) | 加分 | ffmpeg 简单滤镜不达选色(hue 无范围控制;split+maskedmerge 重且脆)——评估后明确降级,能力面诚实标注 |
| 26 | 示波器(波形/矢量/直方图) | 支持 | 支持 | ✅ achieved(册五 T5.2:scope_data 数据后端,三类数据 JSON 缓存内容寻址) | 必达 | 纯函数单测锁定分桶/聚合;壳 canvas 绘制是 FE 活(登记) |
| 27 | 分屏对比/调色拷贝粘贴 | 支持 | 支持 | ✅ achieved(册五 T5.2:拷贝粘贴走 clip_update patch.grade 整对象替换;分屏对比=纯壳活) | 加分 | 与 crop/fx 同模式(登记) |
| 28 | 轨道级 EQ(多段) | 支持 | 支持 | ✅ achieved(册五 T5.3:track.eq → per-track biquad 链 equalizer/lowshelf/highshelf) | 必达 | parity 夹具 A1 440Hz 频段能量衰减断言(频域);无声明轨混音图零变化 |
| 29 | 轨道级动态(压缩/限幅) | 支持 | 支持 | ✅ achieved(册五 T5.3:track.dyn → acompressor 参数子集 + alimiter) | 必达 | parity 夹具 A2 压缩动态范围收窄断言;dB↔线性换算单测锁定 |
| 30 | 响度计/响度单参数化 | 支持 | 支持 | ✅ achieved(册五 T5.3:audio_loudness 工具 + render loudnormTarget 选项) | 必达 | parity 夹具 A3 输出 LUFS 与目标偏差 ≤1LU(AC-5.3 数值断言);缺省 -14/-1.0 逐字兼容 |
| 31 | ducking 参数化 | 支持 | 支持 | ✅ achieved(册五 T5.3:bgm.duck* 四字段 → sidechaincompress 参数) | 加分 | 缺省=既有常量(threshold 0.03/ratio 8/attack 80/release 500),格式化去尾零逐字一致 |
| 32 | 硬件编码探测/优雅降级 | 不支持 | 支持 | ✅ achieved(册五 T5.6:encode_probe(-encoders+试编)+ encoder=hw 试编失败降级 libx264 WARN) | 必达 | AC-5.6 降级断言;本机 AMD 无 nvenc 降级路径实测取证 |
| 33 | 编码参数面 | 部分 | 支持 | ✅ achieved(册五 T5.6:crf/bitrate/gop/pixFmt/quality 预设映射) | 必达 | encode.rs 单测锁定映射;缺省=现值零行为变化 |
| 34 | 输出色彩标签(bt709) | 部分 | 支持 | ✅ achieved(册五 T5.6/ADR-0020:encode 重编码写三枚举+tv range,ffprobe 复验入自检) | 必达 | RENDERER_VERSION 8.0;复验缺失 WARN 留痕不阻塞 |
| 35 | 渲染队列 | 部分 | 支持 | ✅ achieved(册五 T5.6:render_run 入队 + render_queue list/pause/resume/cancel/retry,并发上限 env 可配) | 必达 | AC-5.6 排队场景;暂停语义=终止+重入队(缓存吸收重跑,诚实声明);状态事件接 render.progress |
| 36 | 渲染日志(耗时/命令开关) | 部分 | 支持 | ✅ achieved(册五 T5.6:progress 事件 elapsedMs 恒开;verboseCmd 命令原文缺省关) | 加分 | 安全口径:命令原文含素材路径,缺省不上事件面 |
| 37 | HDR(bt2020/PQ/HLG) | 部分 | 部分 | ❌ missing(ADR-0020 明确暂缓) | 可选 | 无素材无用例不做 tone mapping;触发条件=真实 HDR 交付用例(逐案 ADR 重开,挂载点=输入归一层) |
| 38 | 复合片段(嵌套时间线) | 支持 | 部分 | ✅ achieved(册五 T5.4/ADR-0019:compound 内联子时间线,深度≤两级;create/unbind 单 Op;渲染递归展开 compose 层内容寻址) | 必达 | parity C1 两级实渲(总时长/颜色序列/叠加上层/二级缓存全命中);子 clips 音频暂不渲染(WARN 留痕登记) |
| 39 | 调整层(kind=adjust) | 支持 | 支持 | ✅ achieved(册五 T5.4:adjust 步 trim 抽窗→fx/grade 链→overlay enable 贴回) | 必达 | parity C2 fx.blur 窗内峰值梯度骤降、窗外画面不变;文本随 textass 同通道 |
| 40 | 多机位(同步+展开) | 支持 | 部分 | ✅ achieved(册五 T5.4/ADR-0019 展开方案:multicam_sync pcm-xcorr 启发式诚实标注 + multicam_cut 序列展开单 Op;IR 不建实体) | 加分 | parity C3 500ms 固定偏移恢复 ±50ms;渲染色块时间线无缝 |
| 41 | 场景剪切检测 | 支持 | 支持 | ✅ achieved(册五 T5.4:scene_detect 抽帧差分 frame-diff 启发式 + TrackSplitAt 单 Op 自动切段) | 加分 | analyze 纯函数单测(硬切 ±100ms/渐变零误报);parity C4 硬切色块检测 |
| 42 | OTIO/EDL 互操作 | 支持 | 部分 | ✅ achieved(册五 T5.5/ADR-0019 手写最小 OTIO JSON 子集 + EDL CMX3600 导出 + VTT 往返;子集外 WARN 留痕不静默丢) | 加分 | 往返语义等价单测(出→入→再出 diff=0 + 工程投影等价);EDL 外部解析人工验证候用户执行(AC-5.5) |

## 结论(M11-1 追平后口径)

- 必达 13 项:**实码达成 13/13**(M8 止血后 7 项 → M11 补齐 变速/转场/ducking/punch-in/afade/overlay)
- 册四 A4-BE3a 新增 3 项(#19/#20/#21)全部 achieved:转场库 58 项目录直通 + acrossfade 声画同步 / fx 注册表 11 项 combo 叠加 / motion 19 项真实渲染
- 册五 T5.2/T5.3/T5.6 新增 16 项(#22–#37):13 项 achieved(调色一级/曲线/LUT/示波器数据/分屏与拷贝/轨道 EQ/动态/响度计/ducking 参数化/硬件编码降级/编码参数面/bt709 标签/渲染队列/渲染日志)、HSL 限定器登记降级 + HDR 按 ADR-0020 明确暂缓(missing=诚实标注,非未评估)
- 可选 2 项:0 项实现(关键词 6、蒙版 10)
- **整体 = 16/18 ≈ 89%**(M8 止血时 47%;V1 虚报 93.3% 的差值 = 可选两项;基线 15 项口径 87%,A4-BE3a 新增 3 项全达成分母加 3)
- ~~旋转:契约无字段~~ 册四 A4(T4.9)起 rotation/crop/flip 入契约并渲染落地(#4/#16/#18,ADR-0015 画布范围同批)
- 证据:crates/cutforge-render/tests/parity_matrix.rs 九项 ffmpeg 实测(转场零漂移/变速时长语义/punch-in 帧差/overlay 时间窗/ducking 能量差/afade RMS/文件名消毒/文本轨口径/mix 真分叉);册四 A4-BE2 增五项(曲线变速时长对拍/倒放首末帧对调/90° 旋转朝向/crop 象限/flip 镜像);册四 A4-BE3a 增四项(转场五分类各 2 项目录直通零漂移/acrossfade 音频链/特效四项像素级断言/motion 淡入滑入)——夹具 achieved 总数 18

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
