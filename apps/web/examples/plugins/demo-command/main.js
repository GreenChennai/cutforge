/* 示例插件 1/3:命令贡献点(T7.2 · AC-7.2 验收口径)。
 *
 * 运行位置:Web Worker(宿主注入 cutforge 桥;全局无 DOM、无 token)。
 * 贡献点:contributes.commands.clip-count →
 *   - keymap 注册(group=插件,设置面板可绑组合);
 *   - 时间线右键菜单项「插件:统计片段数」(可逐条隐藏);
 * 行为:经桥代理调 query 类工具 timeline_get(需 permissions.read;越权被宿主
 * GUARD_FAILED/FORBIDDEN 拦截,服务端 plugin-call 通道二次兜底)。
 */
"use strict";

cutforge.onCommand("clip-count", async () => {
  const data = await cutforge.call("timeline_get");
  const clips = (data && data.clips) || [];
  const byTrack = {};
  for (const c of clips) {
    const t = c.track || "?";
    byTrack[t] = (byTrack[t] || 0) + 1;
  }
  const detail = Object.keys(byTrack).sort()
    .map((t) => `${t}×${byTrack[t]}`).join("、");
  if (!clips.length) return "时间线为空(0 个片段)";
  return `时间线共 ${clips.length} 个片段(${detail})`;
});

cutforge.ready();
