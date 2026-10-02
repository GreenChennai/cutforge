/* 示例插件 3/3:面板贡献点(T7.2 · AC-7.2 验收口径)。
 *
 * 面板扩展点:contributes.panels.stats → 启用后出现「插件:工程统计」页签;
 * 插件经桥 setPanel 推送 HTML 片段,宿主消毒后受控渲染(标签白名单+剥属性,
 * 禁 script/交互控件——面板是展示面,交互走命令贡献点)。
 * Worker 内 setInterval 可用(独立全局,不碰主线程);只读面 timeline_get。
 */
"use strict";

function esc(s) {
  return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

async function push() {
  let html;
  try {
    const data = await cutforge.call("timeline_get");
    const clips = (data && data.clips) || [];
    const byTrack = new Map();
    let totalMs = 0;
    for (const c of clips) {
      const t = c.track || "?";
      if (!byTrack.has(t)) byTrack.set(t, { n: 0, ms: 0 });
      const row = byTrack.get(t);
      row.n += 1;
      row.ms += (c.endMs || 0) - (c.startMs || 0);
      totalMs += (c.endMs || 0) - (c.startMs || 0);
    }
    const rows = [...byTrack.keys()].sort().map((t) => {
      const r = byTrack.get(t);
      return `<tr><td>${esc(t)}</td><td>${r.n}</td><td>${Math.round(r.ms)}ms</td></tr>`;
    }).join("");
    html = `<h4>时间线统计(插件每 5s 刷新)</h4>`
      + `<p>片段 ${clips.length} 个 · 总时长 ${Math.round(totalMs)}ms</p>`
      + `<table><tr><td>轨</td><td>片段数</td><td>总时长</td></tr>${rows}</table>`
      + `<p><small>数据经宿主代理 timeline_get(rev ${data ? data.rev : "?"});面板受控渲染。</small></p>`;
  } catch (e) {
    html = `<h4>时间线统计</h4><p>读取失败:${esc((e && e.message) || e)}</p>`;
  }
  cutforge.setPanel("stats", html);
}

push();
setInterval(push, 5000);
cutforge.ready();
