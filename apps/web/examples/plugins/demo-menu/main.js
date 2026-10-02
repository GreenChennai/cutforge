/* 示例插件 2/3:菜单贡献点(T7.2 · AC-7.2 验收口径)。
 *
 * 菜单扩展点由 commands 载体呈现(manifest schema additionalProperties 收口,
 * 无独立 menus 键):启用后时间线/片段右键出现「插件:片段信息卡」项,菜单点击
 * 把 {via:"menu", ctx:"clip"|"timeline", clipId} 作为 payload 传给处理函数。
 * 只读面:timeline_get(permissions.read);越权调用写工具会被宿主拦截。
 */
"use strict";

cutforge.onCommand("clip-info", async (payload) => {
  const data = await cutforge.call("timeline_get");
  const clips = (data && data.clips) || [];
  if (payload && payload.clipId) {
    const c = clips.find((x) => x.id === payload.clipId);
    if (!c) return `片段 ${payload.clipId} 不在投影中(可能已被删除)`;
    return `片段 ${c.id}(轨 ${c.track}):${Math.round(c.startMs)}ms → ${Math.round(c.endMs)}ms`
      + `(时长 ${Math.round(c.endMs - c.startMs)}ms)`;
  }
  if (!clips.length) return "时间线为空:先从素材面板插入一段,再点片段右键菜单";
  const first = clips.slice().sort((a, b) => a.startMs - b.startMs)[0];
  return `未指定片段:给出首段 ${first.id}(轨 ${first.track},起点 ${Math.round(first.startMs)}ms);右键点片段可看该片信息`;
});

cutforge.ready();
