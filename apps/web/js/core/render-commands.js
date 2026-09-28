/* 渲染类命令(T2.3/T2.5):导出双后端 / 剪映草稿 / 精确预览 / 产物清单。
 *
 * 刻意不进 commands 的串行写队列:渲染以分钟计,入队会让 undo/redo 在整个渲染期
 * 被壳自身阻塞(违背 E6-3「渲染期间编辑不被阻塞」的壳侧语义)。初始提交
 * (render_run/export_jianying)是一次普通 RPC,轮询在队列外自旋。
 */
import { call } from "./api.js";
import { projectStore, uiStore } from "./store.js";
import { messageOf, isInternalCode } from "./errors.js";
import { toast } from "../ui/toast.js";

function report(env, name, okMsg) {
  if (!env.ok) {
    if (isInternalCode(env.code)) {
      uiStore.set({ internalErrors: uiStore.get().internalErrors + 1 });
    } else if (!env.net) {
      toast(messageOf(env, name), false);
    }
  } else if (okMsg) {
    toast(okMsg);
  }
  return env;
}

/**
 * cutforge 后端异步导出:onProgress(state, text) 轮询回调(500ms,≈2Hz 达导出进度预算)。
 * @param {(state: string, text: string) => void} onProgress
 */
export async function runExportCutforge(onProgress) {
  const project = projectStore.get().project;
  const ass = project && project.subtitle && project.subtitle.ass;
  onProgress("submit", "提交中…");
  const r = await call("render_run", ass ? { ass } : {});
  if (!r.ok) {
    onProgress("fail", `提交失败:${r.code}`);
    return report(r, "render_run");
  }
  const runId = r.data.runId;
  onProgress("running", "渲染中…");
  for (;;) {
    await new Promise((res) => setTimeout(res, 500));
    const s = await call("render_progress", { runId });
    if (!s.ok) continue; // 单次轮询失败不打断渲染等待(与旧壳口径一致)
    const tail = (s.data.lines || []).slice(-3).join("\n");
    onProgress(s.data.state, `[${s.data.state}] ${tail}`);
    if (s.data.state !== "running") {
      if (s.data.state === "ok") {
        onProgress("done", `完成:${s.data.output}`);
        await refreshRenderOutputs();
      } else {
        onProgress("fail", `失败:${s.data.error || "见服务端日志"}`);
      }
      break;
    }
  }
  return r;
}

/** ffmpeg 后端(CutFlow rs_render 编排;工程相对路径由 /session 下发,壳不硬编码目录)。 */
export async function runExportFfmpeg(ratio, onProgress) {
  const projectRel = projectStore.get().projectRel;
  onProgress("running", "提交 CutFlow rs_render…");
  const env = await call("render", {
    backend: "ffmpeg", ratio,
    scriptArgs: [projectRel, "--ratio", ratio, "--profile", "final"],
  });
  if (env.ok) {
    onProgress("done", "完成(CutFlow rs_render)");
    await refreshRenderOutputs();
  } else {
    onProgress("fail", `失败:${env.code} ${env.message}`);
  }
  return report(env, "render");
}

/** 剪映草稿(export_jianying;名称经对话框确认)。 */
export async function exportJianying(name, onProgress) {
  onProgress("running", "生成剪映草稿…");
  const env = await call("export_jianying", { name });
  if (env.ok) {
    onProgress("done", `剪映草稿完成:${env.message || name}`);
  } else {
    onProgress("fail", `失败:${env.code} ${env.message || ""}`);
  }
  return report(env, "export_jianying");
}

/** 精确预览(render_frame;工具由并行任务落库,未就绪时诚实呈现服务端错误)。 */
export function precisePreview(atMs) {
  return call("render_frame", { atMs });
}

/** 产物清单(render_probe;只读,不入投影、不触 rev)。 */
export async function refreshRenderOutputs() {
  const env = await call("render_probe");
  return env.ok ? (env.data.files || []) : [];
}
