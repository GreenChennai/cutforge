#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""OpenAPI 3.1 生成器(册七 T7.1/AC-7.1;ADR-0003「能力矩阵是生成物」同纪律)。

    python tools/gen_openapi.py            # 生成 docs/api/openapi.json
    python tools/gen_openapi.py --check    # 校验:0=一致;2=GEN_STALE(生成物过期)

单一真相源 = schemas/mcp-tools.json(工具契约):78 个工具各映射为
`POST /api/v1/tools/<tool>`(requestBody = inputSchema 原样,$ref 重写到
components);常用查询另给 GET 别名(/api/v1/project 等,查询参数由对应工具的
inputSchema 派生,root 除外——工作区通道绑定根注入);事件流 /api/v1/events
(SSE,同一 serve_sse 实现)。/api/v1 与 /rpc 是同一 dispatch 单表的两个皮,
HTTP 状态码 = envelope.code 确定性映射(ADR-0025)。

本生成器不手写任何工具名清单——工具集漂移(加/删工具)必然反映到生成物,
--check 进 CI 拦截。退出码:0 一致 / 2 漂移(生成物须重跑生成器)。
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

REPO = Path(__file__).resolve().parents[1]
CONTRACT = REPO / "schemas" / "mcp-tools.json"
OUT = REPO / "docs" / "api" / "openapi.json"

API_PREFIX = "/api/v1"
API_VERSION = "1.0.0"

# GET 别名 → 工具名(与 crates/cutforge-mcp/src/transport/rest.rs GET_ALIASES 同域;
# 改动必须两处同批,rest.rs 单测锁行为,本表锁文档)
GET_ALIASES: list[tuple[str, str]] = [
    ("project", "project_get"),
    ("timeline", "timeline_get"),
    ("notes", "notes_list"),
    ("oplog", "oplog_tail"),
    ("cutlist", "cutlist_get"),
    ("wordline", "wordline_get"),
    ("capabilities", "capability_matrix"),
]

# envelope.code → HTTP 状态(与 rest.rs status_of 同域)
CODE_STATUS: dict[str, str] = {
    "OK": "200",
    "NO_CONFIG": "404",
    "CONFLICT": "409",
    "JIANYING_RUNNING": "409",
    "DEP_MISSING": "503",
    "INTERNAL": "500",
    "SCHEMA_INVALID": "400",
    "PRECONDITION_FAILED": "400",
    "GUARD_FAILED": "400",
    "GREEN_SCREEN_INPUT": "400",
}

ENVELOPE_RESPONSE = {
    "description": "结果协议 envelope(ok/code/ns/message/data);HTTP 状态码由 code 确定性映射"
                   "(OK→200,NO_CONFIG→404,CONFLICT→409,DEP_MISSING→503,INTERNAL→500,其余→400)。",
    "content": {"application/json": {"schema": {"$ref": "#/components/schemas/envelope"}}},
}

SECURITY = [{"bearerAuth": []}]


def rewrite_refs(node, defs_prefix: str):
    """把契约内 $ref(#/definitions/x、#/$defs/x)重写为 OpenAPI components 引用。"""
    if isinstance(node, dict):
        out = {}
        for k, v in node.items():
            if k == "$ref" and isinstance(v, str):
                if v.startswith("#/definitions/"):
                    out[k] = "#/components/schemas/" + v.rsplit("/", 1)[1]
                elif v.startswith("#/$defs/"):
                    out[k] = "#/components/schemas/" + v.rsplit("/", 1)[1]
                else:
                    out[k] = v
            else:
                out[k] = rewrite_refs(v, defs_prefix)
        return out
    if isinstance(node, list):
        return [rewrite_refs(x, defs_prefix) for x in node]
    return node


def alias_query_params(tool: dict) -> list[dict]:
    """GET 别名的查询参数 = 工具 inputSchema.properties(root 除外)。"""
    schema = tool.get("inputSchema", {})
    props = schema.get("properties", {})
    required = [r for r in schema.get("required", []) if r != "root"]
    params = []
    for name, prop in props.items():
        if name == "root":
            continue
        p = {
            "name": name,
            "in": "query",
            "description": prop.get("description", ""),
            "schema": rewrite_refs({k: v for k, v in prop.items() if k != "description"}, None),
            "required": name in required,
        }
        params.append(p)
    return params


def build_doc() -> dict:
    contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
    tools = contract["tools"]

    schemas: dict = {
        "envelope": {
            "type": "object",
            "required": ["ok", "code", "message", "data"],
            "properties": {
                "ok": {"type": "boolean"},
                "code": {"enum": ["OK", "CONFLICT", "SCHEMA_INVALID", "PRECONDITION_FAILED",
                                  "GUARD_FAILED", "JIANYING_RUNNING", "NO_CONFIG", "DEP_MISSING",
                                  "GREEN_SCREEN_INPUT", "INTERNAL"],
                         "description": "计划书 5.4 错误码表;ns = 命名空间加法字段派生源"},
                "ns": {"type": "string",
                       "enum": ["ok", "io", "core", "mcp", "render", "unknown"],
                       "description": "错误码命名空间(加法维度;registry::CODE_NS 派生)"},
                "message": {"type": "string"},
                "data": {"type": "object"},
            },
        },
    }
    # 契约内公共定义(envelope / $defs)并入 components(同名的以契约为准)
    for name, schema in contract.get("definitions", {}).items():
        schemas[name] = rewrite_refs(schema, None)
    for name, schema in contract.get("$defs", {}).items():
        schemas[name] = rewrite_refs(schema, None)

    paths: dict = {}
    tag_of = {"query": "查询", "write": "写", "orchestrate": "编排"}

    # 1) 工具统一入口:POST /api/v1/tools/<tool>(78 个逐一展开,路径即契约)
    for t in tools:
        name = t["name"]
        kind = t.get("kind", "?")
        paths[f"{API_PREFIX}/tools/{name}"] = {
            "post": {
                "operationId": name,
                "summary": t.get("description", name),
                "description": (f"kind: {kind} · idempotent: {t.get('idempotent')} · "
                                f"与 /rpc 的 tools/call 同一 dispatch 单表(M4-1 三通道同源)。"
                                f"工作区通道(serve --root)对缺 root 的调用注入绑定根;"
                                f"辅通道(/rpc 同端口面)root 必须显式给出。"),
                "tags": [tag_of.get(kind, kind)],
                "security": SECURITY,
                "requestBody": {
                    "required": True,
                    "content": {"application/json": {
                        "schema": rewrite_refs(t.get("inputSchema", {"type": "object"}), None),
                    }},
                },
                "responses": {"200": ENVELOPE_RESPONSE, "default": ENVELOPE_RESPONSE},
            },
        }

    # 2) GET /api/v1/tools:工具清单(注册表同源)
    paths[f"{API_PREFIX}/tools"] = {
        "get": {
            "operationId": "list_tools",
            "summary": "工具清单(名/kind/描述;schemas/mcp-tools.json 单一真相源)",
            "tags": ["API"],
            "security": SECURITY,
            "responses": {
                "200": {
                    "description": "envelope(data.tools = 工具清单)",
                    "content": {"application/json": {"schema": {"$ref": "#/components/schemas/envelope"}}},
                },
            },
        }
    }

    # 3) GET 别名:常用查询同参转发(查询参数由工具 inputSchema 派生,root 除外)
    for alias, tool_name in GET_ALIASES:
        tool = next((t for t in tools if t["name"] == tool_name), None)
        assert tool is not None, f"GET 别名指向未注册工具: {alias} → {tool_name}"
        paths[f"{API_PREFIX}/{alias}"] = {
            "get": {
                "operationId": f"get_{alias}",
                "summary": f"{tool['description']}(GET 别名 → {tool_name} 同参转发)",
                "tags": ["查询"],
                "security": SECURITY,
                "parameters": alias_query_params(tool),
                "responses": {"200": ENVELOPE_RESPONSE, "default": ENVELOPE_RESPONSE},
            },
        }

    # 4) 事件流(SSE + 长轮询降级;transport::events 单一实现)
    paths[f"{API_PREFIX}/events"] = {
        "get": {
            "operationId": "events",
            "summary": "工作区事件流(workspace/notes/cutlist.changed + render.progress)",
            "description": ("`Accept: text/event-stream` → SSE 单向流(id:/event:/data: 帧,"
                            "Last-Event-ID 续传,滚出窗口发 resync);旧客户端不带该头 → 长轮询降级"
                            "(≤900ms 内有新事件即回,负载 ok/code/event/seq)。"
                            "工作区通道绑定根免 root;辅通道 root 查询参数必给。"),
            "tags": ["API"],
            "security": SECURITY,
            "parameters": [
                {"name": "Accept", "in": "header", "schema": {"type": "string"},
                 "description": "text/event-stream = SSE"},
                {"name": "since", "in": "query", "schema": {"type": "integer", "minimum": 0},
                 "description": "长轮询起点 seq"},
                {"name": "root", "in": "query", "schema": {"type": "string"},
                 "description": "工程目录(仅辅通道必给;工作区通道绑定根)"},
            ],
            "responses": {
                "200": {"description": "SSE 流(text/event-stream)或长轮询 JSON",
                        "content": {"application/json": {
                            "schema": {"type": "object",
                                       "properties": {"ok": {"type": "boolean"},
                                                      "code": {"type": "string"},
                                                      "event": {"type": "string"},
                                                      "seq": {"type": "integer"}}}}}},
            },
        }
    }

    return {
        "openapi": "3.1.0",
        "info": {
            "title": "CutForge Editor API",
            "version": API_VERSION,
            "description": (
                "URL 版本化 Editor API(册七 T7.1/ADR-0025)。工具统一入口 "
                f"`POST {API_PREFIX}/tools/<tool>`;常用查询 GET 别名;事件流 `{API_PREFIX}/events`。"
                "本文件是**生成物**(tools/gen_openapi.py ← schemas/mcp-tools.json 单一真相源,"
                "禁止手改);所有工具返回 `{ok, code, ns, message, data}` envelope,HTTP 状态码 "
                "由 code 确定性映射。鉴权:Bearer token(serve 启动时落盘 .cutforge/session),"
                "仅监听 127.0.0.1;CORS 默认关闭。旧 `/rpc` 保留别名至少两册(册七/册八)。"
                f"工具数以契约文件实时为准(本文件生成时点:{len(tools)})。"
            ),
            "x-contract-version": contract.get("version"),
            "x-adrs": ["docs/adr/0025", "docs/adr/0009", "docs/adr/0003"],
        },
        "servers": [{"url": "http://127.0.0.1:{port}",
                     "description": "本地工作区服务(cutforge-mcp serve --root <工程>;端口见 .cutforge/session)",
                     "variables": {"port": {"default": "8765"}}}],
        "tags": [
            {"name": "查询", "description": "只读查询工具(GET 别名与 POST 入口等价)"},
            {"name": "写", "description": "写工具(经 Workspace::apply 唯一写入口,OpLog 全留痕)"},
            {"name": "编排", "description": "编排/渲染/导出工具(免锁面或子进程编排)"},
            {"name": "API", "description": "API 自描述端点(清单/事件流)"},
        ],
        "components": {
            "securitySchemes": {"bearerAuth": {"type": "http", "scheme": "bearer"}},
            "schemas": schemas,
        },
        "paths": paths,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description="OpenAPI 3.1 生成器(mcp-tools.json 单一真相源)")
    ap.add_argument("--check", action="store_true", help="校验生成物与契约一致(漂移退出码 2)")
    a = ap.parse_args()

    doc = build_doc()
    rendered = json.dumps(doc, ensure_ascii=False, indent=1) + "\n"

    if a.check:
        if not OUT.is_file():
            print(f"[GEN_STALE] {OUT} 不存在;先跑 python tools/gen_openapi.py")
            return 2
        on_disk = OUT.read_text(encoding="utf-8")
        if on_disk != rendered:
            import difflib
            diff = list(difflib.unified_diff(
                on_disk.splitlines(), rendered.splitlines(),
                "openapi.json(盘面)", "openapi.json(重生成)", lineterm="", n=1))
            print("[GEN_STALE] docs/api/openapi.json 与契约漂移(先跑 python tools/gen_openapi.py);差异头部:")
            for line in diff[:30]:
                print("   " + line)
            return 2
        paths_n = len(doc["paths"])
        tools_n = sum(1 for p in doc["paths"] if p.startswith(API_PREFIX + "/tools/"))
        print(f"[OK] OpenAPI 生成物零漂移:{paths_n} 路径 = {tools_n} 工具入口 + "
              f"GET 别名 {len(GET_ALIASES)} + tools 清单 + events;契约 version "
              f"{doc['info'].get('x-contract-version')}")
        return 0

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(rendered, encoding="utf-8", newline="\n")
    paths_n = len(doc["paths"])
    tools_n = sum(1 for p in doc["paths"] if p.startswith(API_PREFIX + "/tools/"))
    print(f"[OK] OpenAPI 3.1 已生成:{OUT}(相对仓库根)")
    print(f"     {paths_n} 路径 = {tools_n} 工具入口 + GET 别名 {len(GET_ALIASES)} + tools 清单 + events")
    return 0


if __name__ == "__main__":
    sys.exit(main())
