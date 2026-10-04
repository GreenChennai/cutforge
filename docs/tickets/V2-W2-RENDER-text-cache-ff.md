# V2-W2-RENDER 文本诚实化/缓存稳定化/ffmpeg 超时

范围:crates/cutforge-render/**(textass.rs/subtitle.rs/cache.rs/lib.rs run_ff 族)。

条目(报告 v2 §4/§5):
- BUG-13(P2)+BUG-21(P2):escape_text 收口唯一实现(subtitle.rs 改调 textass::escape_text);
  先 warn 列出被改写行,再精确打断(仅当 {} 内为合法 ASS 标签才插 U+200B,否则原样输出);
  TC-RENDER-TEXT-001(半角花括号保真)/002({\pos 注入中和})/003(两导出路径逐字节相等);
- BUG-14(P3):SRT 毫秒 1~3 位右对齐补零,错误带行号与原文;TC-RENDER-SRT-001/002;
- R-06(P1):缓存键 DefaultHasher→FNV-1a 64(或 blake3),索引增 hash_algo 版本字段;
  索引 read-modify-write 全程持 .cutforge/cache-index.lock(复用 io 锁原语);
  TC-RENDER-CACHE-001(黄金哈希断言)/002(两进程并发无丢失更新);
- R-07(P1):run_ff/run_ff_capture/run_ff_in 增 timeout(缺省 10min,encode 按预估×3 封顶),
  超时 kill + RenderError::FfmpegTimeout{stage};stderr 流式读取只留尾部 4KB 环形
  (模板:apps/desktop/src/playback/decoder.rs 看门狗——只参照写法,不引依赖);
  TC-RENDER-FF-001(mock 挂起→超时 kill+阶段名)/002(100MB stderr→内存峰值<50MB)。
注意:render 现 12 个 >800 行文件之一(lib.rs 1009)——本工单不要求拆分(另轮),
但新增代码不得使文件显著增长;parity_matrix/render_matrix 全绿不回退。
状态:待第 1 波集成后开工
