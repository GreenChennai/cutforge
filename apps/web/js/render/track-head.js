/* 轨道头全功能(T4.2):锁定/静音/独奏/隐藏(眼)/重命名(双击)/高度拖拽/标识色。
 * 全部经 track_update 七字段(name/locked/mute/solo/hidden/heightPx/color;hidden 仍走
 * ephemeral 眼睛口径——「视图隐藏」与「合成隐藏」语义分离,见 ui-fields trackEditable)。
 * 轨道顺序(上移/下移)与删除:后端无 track_reorder/track_delete 工具,菜单项诚实禁用。
 * 锁定联动:locked 轨 lane 挂 .lane-locked(手势层拒绝编辑);mute/solo 音频波形灰化。 */
import { h } from "../ui/dom.js";
import { ephemeralStore } from "../core/store.js";
import { updateTrack } from "../core/edit-commands.js";
import { runGesture } from "./gesture-kit.js";
import { cssVar } from "./theme.js";
import { svgUse } from "../../assets/icons.js";
import { toggleTrackVisible } from "./timeline-view.js"; // 环依赖安全:仅函数体内调用(延迟绑定)

/** 轨头按钮工厂(字标 + aria-label;激活态 aria-pressed,非颜色单线索)。 */
function headBtn(cls, glyph, label, tip, onclick) {
  return h("button", {
    class: `lane-btn ${cls}`, "aria-label": label, "aria-pressed": "false",
    title: tip, "data-tip": tip, onclick,
  }, [glyph]);
}

/**
 * 构建轨头内容(timeline-view createLane 调用一次;状态更新走 syncTrackHead)。
 * @param {Object} t 投影轨道行 {id, kind, name?, locked?, mute?, solo?, color?}
 */
export function buildTrackHead(t) {
  const isAudio = t.kind === "audio";
  const eye = h("button", {
    class: "lane-eye", testid: `track-visibility-${t.id}`,
    "aria-pressed": "true", "aria-label": `切换 ${t.id} 轨视图显示(不落盘)`,
    "data-tip": "眼睛:仅隐藏视图(ephemeral;合成隐藏走右键「隐藏此轨」)",
    onclick: (e) => {
      e.stopPropagation();
      toggleTrackVisible(t.id);
    },
  }, [svgUse("icon-eye")]);
  const lock = headBtn("lane-lock", "L", `锁定/解锁 ${t.id} 轨(锁定后拒绝编辑手势)`,
    "锁定:拒绝拖拽/裁剪/插入等编辑手势(track_update.locked)",
    (e) => {
      e.stopPropagation();
      updateTrack(t.id, { locked: !(e.currentTarget.getAttribute("aria-pressed") === "true") });
    });
  const mute = isAudio ? headBtn("lane-mute", "M", `静音/取消静音 ${t.id} 轨`,
    "静音(track_update.mute;渲染混音联动候 BE3,先落字段)",
    (e) => {
      e.stopPropagation();
      updateTrack(t.id, { mute: !(e.currentTarget.getAttribute("aria-pressed") === "true") });
    }) : null;
  const solo = isAudio ? headBtn("lane-solo", "S", `独奏/取消独奏 ${t.id} 轨`,
    "独奏:仅独奏轨出声(track_update.solo;渲染混音联动候 BE3)",
    (e) => {
      e.stopPropagation();
      updateTrack(t.id, { solo: !(e.currentTarget.getAttribute("aria-pressed") === "true") });
    }) : null;
  const name = h("span", {
    class: "lane-name", testid: `track-name-${t.id}`,
    title: "双击重命名(track_update.name;Enter 确认 / Esc 取消)",
    "data-tip": "双击重命名",
    ondblclick: (e) => startRename(e.currentTarget, t.id),
  }, [t.name || t.id]);
  const kindBadge = h("span", { class: "badge", "data-tip": "轨型" }, [t.id]);
  const label = h("span", { class: "lane-label", testid: `lane-label-${t.id}` }, [
    h("span", { class: "lane-kind" }, [t.id]),
    lock, ...(mute ? [mute] : []), ...(solo ? [solo] : []), eye,
    name, kindBadge,
  ]);
  // 高度拖拽把手(lane 底边):拖拽期只写内联高度,松手一笔 track_update.heightPx
  const grip = h("span", {
    class: "lane-grip", "aria-hidden": "true", "data-tip": "拖动调轨道高度",
    title: "拖动调整轨道高度(track_update.heightPx)",
  });
  mountGrip(grip, t.id);
  return { label, grip };
}

/** 重命名:span → input,Enter/Esc/失焦收束(空值回退原名;一笔 track_update)。 */
function startRename(span, trackId) {
  const cur = span.textContent || "";
  const input = h("input", {
    class: "lane-rename", "aria-label": `重命名轨道 ${trackId}`,
    title: "Enter 确认 / Esc 取消",
  });
  input.value = cur === trackId ? "" : cur;
  input.placeholder = cur;
  span.textContent = "";
  span.appendChild(input);
  input.focus();
  input.select();
  let done = false;
  const finish = (commit) => {
    if (done) return;
    done = true;
    const v = input.value.trim();
    span.textContent = commit && v ? v : cur;
    if (commit && v && v !== cur) updateTrack(trackId, { name: v });
  };
  input.addEventListener("keydown", (e) => {
    e.stopPropagation(); // 重命名输入态不吃全局快捷键
    if (e.key === "Enter") finish(true);
    else if (e.key === "Escape") finish(false);
  });
  input.addEventListener("blur", () => finish(true));
  input.addEventListener("click", (e) => e.stopPropagation());
}

/** 高度拖拽:pointer 管线;区间 [28, 160]px,松手一笔 Op。 */
function mountGrip(grip, trackId) {
  grip.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const lane = grip.closest(".track");
    if (!lane) return;
    const h0 = lane.getBoundingClientRect().height;
    const y0 = e.clientY;
    runGesture(grip, e, {
      move: (ev) => {
        const nh = Math.min(160, Math.max(28, Math.round(h0 + (ev.clientY - y0))));
        lane.style.height = `${nh}px`;
      },
      end: (ev) => {
        const nh = Math.min(160, Math.max(28, Math.round(h0 + (ev.clientY - y0))));
        lane.style.height = "";
        updateTrack(trackId, { heightPx: nh }); // 一笔 Op;投影回来按 heightPx 落样式
      },
      cancel: () => { lane.style.height = ""; },
    });
  });
}

/**
 * 轨头/lane 状态同步(renderLanes 每轮调用;只碰变化的 class/aria/内联色)。
 * @param {HTMLElement} lane
 * @param {Object} t 投影轨道行
 */
export function syncTrackHead(lane, t) {
  const isAudio = t.kind === "audio";
  const lock = lane.querySelector(".lane-lock");
  const mute = lane.querySelector(".lane-mute");
  const solo = lane.querySelector(".lane-solo");
  const eye = lane.querySelector(".lane-eye");
  const name = lane.querySelector(".lane-name");
  const viewHidden = (ephemeralStore.get().hiddenTracks || []).includes(t.id);
  const locked = Boolean(t.locked);
  const muted = Boolean(t.mute);
  const soloed = Boolean(t.solo);
  if (lock) press(lock, locked);
  if (mute) press(mute, muted);
  if (solo) press(solo, soloed);
  if (eye) {
    eye.setAttribute("aria-pressed", viewHidden ? "false" : "true");
    eye.classList.toggle("is-on", !viewHidden);
  }
  if (name && name.textContent !== (t.name || t.id) && !name.querySelector("input")) {
    name.textContent = t.name || t.id;
  }
  lane.classList.toggle("lane-locked", locked);
  lane.classList.toggle("lane-muted", isAudio && muted);
  lane.classList.toggle("lane-soloed", isAudio && soloed);
  lane.classList.toggle("hidden-by-user", viewHidden);
  lane.style.height = t.heightPx ? `${Math.min(160, Math.max(28, t.heightPx))}px` : "";
  // 标识色:数据驱动内联色(值来自投影,非 js 字面量;R5 口径不涉)
  lane.style.borderLeft = t.color ? `3px solid ${t.color}` : "";
  if (t.color) lane.dataset.color = t.color;
  else delete lane.dataset.color;
}

function press(btn, on) {
  btn.setAttribute("aria-pressed", on ? "true" : "false");
  btn.classList.toggle("is-on", on);
}

/** 标识色候选(菜单用;值经 token 运行时解析,零字面量)。
 * 无「清除」项:TrackPatch 按字段合并(null = 不改),换色即可,无复位语义。 */
export function trackColorChoices() {
  return [
    [cssVar("--cf-blue-500"), "蓝"],
    [cssVar("--cf-green-450"), "绿"],
    [cssVar("--cf-violet-300"), "紫"],
    [cssVar("--cf-amber-300"), "橙"],
    [cssVar("--cf-red-450"), "红"],
  ];
}
