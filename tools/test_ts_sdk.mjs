#!/usr/bin/env node
// AC-7.1 门禁:TS SDK 冒烟(册七 T7.1)——生成物 apps/sdk/cutforge.ts 对真 serve
// 跑 ≥10 个典型调用(REST 统一入口 + 事件订阅 + .cfpkg 打包/解包 + 负例)。
//
//     node tools/test_ts_sdk.mjs [--bin target/debug/cutforge-mcp]
//
// 运行形态:node ≥23.6(类型剥离默认)或 bun 直跑;.ts 生成物零依赖 import。
// 退出码:0 全过 / 2 失败。依赖:已构建的 cutforge-mcp(serve)。

import { spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { CutforgeClient } from "../apps/sdk/cutforge.ts";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TOKEN = "sdk-smoke-token";
const WEB = path.join(REPO, "apps", "web");

const args = process.argv.slice(2);
const binFlag = args.indexOf("--bin");
let binPath = binFlag >= 0 ? path.resolve(args[binFlag + 1]) : null;
if (!binPath) {
  for (const cand of [
    "target/debug/cutforge-mcp.exe", "target/debug/cutforge-mcp",
    "target/release/cutforge-mcp.exe", "target/release/cutforge-mcp",
  ]) {
    if (fs.existsSync(path.join(REPO, cand))) { binPath = path.join(REPO, cand); break; }
  }
}
if (!binPath || !fs.existsSync(binPath)) {
  console.error("FAIL: 先 cargo build -p cutforge-mcp(未找到二进制)");
  process.exit(2);
}

const freePort = () => new Promise((resolve, reject) => {
  const s = net.createServer();
  s.listen(0, "127.0.0.1", () => {
    const port = s.address().port;
    s.close(() => resolve(port));
  });
  s.on("error", reject);
});

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let passed = 0;
function ok(name, cond, detail = "") {
  if (!cond) {
    throw new Error(`断言失败: ${name}${detail ? ` — ${detail}` : ""}`);
  }
  passed += 1;
  console.log(`  [${passed}] ${name} PASS`);
}

async function ready(port) {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/session`, {
        headers: { Authorization: `Bearer ${TOKEN}` },
      });
      if (res.ok) { return; }
    } catch { /* 未就绪 */ }
    await sleep(150);
  }
  throw new Error("serve 未就绪(15s)");
}

async function main() {
  const port = await freePort();
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cutforge-sdk-smoke-"));
  const ws = path.join(tmp, "proj");
  // serve preflight 要求工程在位:预写最小 v3 project.json(sdk 内 projectNew 另建独立工程体)
  fs.mkdirSync(ws, { recursive: true });
  fs.writeFileSync(path.join(ws, "project.json"), JSON.stringify({
    version: 1, schemaVersion: "3.0.0", slug: "sdk壳", fps: 30,
    canvas: { width: 320, height: 240 }, backends: ["ffmpeg"], tracks: [],
  }));
  const serve = spawn(binPath, ["serve", "--root", ws, "--port", String(port), "--token", TOKEN, "--web", WEB],
    { stdio: ["ignore", "ignore", "pipe"] });
  let serveLog = "";
  serve.stderr.on("data", (d) => { serveLog += d; });
  try {
    await ready(port);
    const client = new CutforgeClient({ baseUrl: `http://127.0.0.1:${port}`, token: TOKEN, root: ws });
    // 业务工程独立于 serve 壳工程(project_new 建盘后换绑定根客户端)
    const work = path.join(tmp, "work");

    // 1-2) API 自描述面
    const caps = await client.capabilityMatrix();
    ok("capabilityMatrix() → OK envelope", caps.ok === true && typeof caps.data.matrix === "object", JSON.stringify(caps).slice(0, 200));
    const list = await client.listTools();
    ok(`listTools() → ${list.data.tools.length} 工具(=78)`, list.ok === true && list.data.tools.length === 78);

    // 3-4) 从零建 v3 工程 + 读回(绑定根客户端)
    const created = await client.call("project_new", { root: work, slug: "sdk冒烟", fps: 30, canvasW: 320, canvasH: 240, tracks: ["video"], layout: "v3" });
    ok("projectNew(v3) → OK", created.ok === true, JSON.stringify(created).slice(0, 200));
    const api = new CutforgeClient({ baseUrl: `http://127.0.0.1:${port}`, token: TOKEN, root: work });
    const pv = await api.projectGet({});
    ok("projectGet() 绑定根注入 → slug=sdk冒烟", pv.ok === true && pv.data.project.slug === "sdk冒烟");

    // 5-7) 写路径:片段/属性/转场(单一 dispatch 单表,rev 前进)
    fs.mkdirSync(path.join(work, "media"), { recursive: true });
    fs.writeFileSync(path.join(work, "media", "a.mp4"), Buffer.from("fake-video"));
    const add = await api.clipAdd({ trackId: "V1", src: "media/a.mp4", startMs: 0, durationMs: 2000, requestId: "sdk-add-1" });
    ok("clipAdd → OK(opIds+rev)", add.ok === true && add.data.rev === 1, JSON.stringify(add).slice(0, 200));
    const upd = await api.clipUpdate({ clipId: "V1-001", patch: { volume: 0.8 } });
    ok("clipUpdate(volume 0.8) → OK", upd.ok === true && upd.data.rev === 2);
    const tr = await api.transitionSet({ clipId: "V1-001", type: "fade", durMs: 300 });
    ok("transitionSet(fade 300ms) → OK", tr.ok === true && tr.data.rev === 3);

    // 8-10) 标注/OpLog/投影(AI 协作闭环面)
    const note = await api.notesAdd({ anchor: { kind: "clip", ref: "V1-001", tMs: 0 }, body: "sdk 冒烟标注", author: "agent" });
    ok("notesAdd(agent 归因) → OK", note.ok === true && typeof note.data.noteId === "string");
    const notes = await api.notesList({ state: "open" });
    ok("notesList → total=1", notes.ok === true && notes.data.total === 1);
    const tail = await api.oplogTail({ limit: 5 });
    ok("oplogTail → ops ≥3(rev=4 含 notes)", tail.ok === true && tail.data.ops.length >= 3 && tail.data.rev === 4);

    // 11) 投影
    const tl = await api.timelineGet({});
    const clip = tl.data.clips.find((c) => c.id === "V1-001");
    ok("timelineGet → 片段投影(volume/transition 可见)",
      tl.ok === true && clip && clip.volume === 0.8 && clip.transition && clip.transition.type === "fade");

    // 12) 负例:未知工具 → ok=false(INTERNAL)
    const neg = await client.call("无此工具", {});
    ok("call(未知工具) → ok=false + INTERNAL", neg.ok === false && neg.code === "INTERNAL");

    // 13-14) .cfpkg 打包/解包(T7.6 新工具经 SDK)
    const pkgPath = path.join(tmp, "链.cfpkg");
    const packed = await api.projectPackage({ out: pkgPath });
    ok("projectPackage → 容器落盘 + counts.media=1",
      packed.ok === true && packed.data.counts.media === 1 && fs.existsSync(pkgPath), JSON.stringify(packed).slice(0, 200));
    const dest = path.join(tmp, "还原");
    const unpacked = await api.projectUnpackage({ src: pkgPath, root: dest });
    ok("projectUnpackage → 解包 v3 + media=1",
      unpacked.ok === true && unpacked.data.media === 1 && fs.existsSync(path.join(dest, "project.json")));
    const client2 = new CutforgeClient({ baseUrl: `http://127.0.0.1:${port}`, token: TOKEN, root: dest });
    const pv2 = await client2.projectGet({});
    ok("解包工程照常打开(slug 同名)", pv2.ok === true && pv2.data.project.slug === "sdk冒烟");

    // 15) 事件订阅:外部改 serve 壳工程 project.json → workspace.changed(SSE 手写解析)
    const pj = path.join(ws, "project.json");
    let grow = 0;
    const gotEvent = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("SSE 8s 内未收到 workspace.changed")), 8000);
      client.subscribeEvents((ev) => {
        if (ev.event === "workspace.changed") {
          clearTimeout(timer);
          resolve(true);
        }
      }).then(() => {
        setTimeout(() => {
          const doc = JSON.parse(fs.readFileSync(pj, "utf-8"));
          grow += 1;
          doc._sdkProbe = "x".repeat(grow);
          fs.writeFileSync(pj, JSON.stringify(doc));
        }, 200);
      }, reject);
    });
    await gotEvent;
    ok("subscribeEvents → workspace.changed(外部改动 ≤8s)", true);

    console.log(`\n[OK] TS SDK 冒烟全过:${passed} 断言(≥10 典型调用,真 serve ${path.basename(binPath)})`);
    process.exitCode = 0;
  } catch (err) {
    console.error(`\n[FAIL] ${err.message ?? err}`);
    if (serveLog) {
      console.error("── serve stderr 尾部 ──");
      console.error(serveLog.split("\n").slice(-15).join("\n"));
    }
    process.exitCode = 2;
  } finally {
    serve.kill();
    try { fs.rmSync(tmp, { recursive: true, force: true }); } catch { /* Windows 句柄延迟 */ }
  }
}

main();
