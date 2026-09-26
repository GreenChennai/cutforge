# CutForge Web 编辑器 v0.6 改造计划(NLE 化)

> 2026-09-26 · 依据用户验收反馈:①打开工程报 INTERNAL(契约不符);②UI/功能/体验糟糕,
> 要求参考剪映/达芬奇,可独立完成剪辑(转场/入场/出场/多轨/剪辑),不依赖 CutFlow。

## 0. 现状诊断(实证)

- OCR 截图(1600x900):布局为「素材面板 | 黑屏预览 | IR 原始字段表单」,底部三大块
  INTERNAL 红错。用户直接面对 startMs/durationMs 等契约字段——是调试面板,不是剪辑工具。
- 后端 38 个 RPC 工具:**没有任何 transition/motion/volume/bgm 设置端点**(grep 证实)。
  转场/入场/出场在 UI 与后端双缺。
- INTERNAL 根因(两处,均为 CutFlow 侧产出不合 cutforge v2 契约):
  1. `rs_ir build` 产 clip id `c001`,不匹配 pattern `^[VAT][0-9]+-[0-9]{3}$`(V1-001 形态);
  2. 实剪工程 sfx 轨 clip 带 schema 外键 `atMs`(构建脚本泄漏内部 placement 结构)。

## 1. P0 契约修复(先让工程能打开)

- CutFlow `rs_ir`:clip id 统一 `{V|A}{轨号}-{序号:03d}`(主轨 V1-001…,音轨 A1-001…);
- CutFlow `rs_sfx`:IR 落盘字段白名单化(src/startMs/durationMs/role/volume),
  内部 placement 结构(atMs 等)不再泄漏;
- 双仓 schema 的 clip.id pattern 同步(CutFlow 侧补同款 pattern,防再漂移);
- 迁移两个已交付实剪工程 IR(id 重排 + 去 atMs),`rs_ir validate` + cutforge session 双验。

## 2. P1 后端:新增 5 个受控写 RPC 工具(带 OpLog/rev/冲突检测,复用既有 apply 管线)

| 工具 | 写入目标 | 说明 |
|---|---|---|
| transition_set | clip.transition {type,durMs,fx} | type ∈ schema 枚举;fx ∈ 效果目录可执行档 |
| motion_set | clip.motion {in,inMs,out,outMs} | 入场/出场(渲染端 M11 起全枚举可渲) |
| audio_gain | clip.volume | 音量(0–2.0) |
| bgm_set / bgm_clear | doc.bgm {src,gainDb,ducking,loop} | 背景乐 |
| output_set | doc.outputs / canvas | 画幅 |

## 3. P1 前端:apps/web 重写为三栏 NLE(剪映/达芬奇式,零依赖原生 JS 不变)

- **左栏 媒体池**:素材浏览(media_browse)+导入(clip_add),缩略卡;
- **中栏 预览播放器**:video 标签按播放头所在段切换媒体源(段级精准预览);
  「预览本段」按需渲当前 clip;导出成片后可播完整成片;
- **右栏 属性面板**(按选中 clip):转场(类型+时长)、入场/出场(类型+时长)、
  音量、变速、文本、位置/缩放——人话字段,不再暴露 IR 原始键;
- **底部 多轨时间线**:轨头(visibility)+轨道块(视频/覆盖/音频分色),clip 块可
  点击选中、拖拽换位(clip_move)、边缘 trim(clip_update)、标尺+播放头拖拽;
- **工具条**:分割(S)/删除(Del)/复制/撤销(Ctrl+Z)/重做(Ctrl+Y)/磁吸开关/ ripple 开关;
- **顶部**:工程名+rev 状态、导出成片、导出剪映草稿;
- 观感:深色 NLE 风(类达芬奇),style.css 全重写。

## 4. 范围控制(本轮不做)

实时混合预览(逐帧合成)、关键帧曲线、多机协作、音频波形绘制。
预览以「段级原始素材预览 + 导出后整片预览」替代,属性正确性以 IR/渲染为权威。

## 5. 验收标准

1. 打开两个实剪工程:**零 INTERNAL 报错**;
2. UI 全程可用:导入→时间线拖放→分割→转场→入场/出场→音量→导出成片,全程不接触 JSON;
3. 新增 5 工具有 pytest 覆盖(写 IR+OpLog+冲突);cargo/pytest 全绿;
4. 双仓推送,CI 绿。
