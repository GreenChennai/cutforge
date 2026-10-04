#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""T1.6/AC-1.6 门禁:静态托管目录化端到端(纯 stdlib,不起浏览器)。

    python tools/e2e_static.py [--bin target/debug/cutforge-mcp.exe]

断言链:
  1. 兼容红线:根路径 / /index.html /app.js /style.css 字节级与 apps/web 一致,
     Content-Type 同旧;/session?token=… 查询参数流照常(数据面);
  2. 目录化:serve 已就绪后新落 apps/web/js/x.js → GET /assets/js/x.js 立即 200
     (零 Rust 改动、零重启);既有文件经 /assets 同样可达;
  3. 穿越 100% 拒绝:字面 ../、编码 %2e%2e、%2F、反斜杠、盘符、双编码全拒(400/404),
     canary(../../Cargo.toml)体中绝无仓库内容;穿越尝试后正常路径仍健康;
  4. ETag/304:ETag 响应头存在;If-None-Match 命中 → 304 空体(根别名同样生效)。
退出码:0 通过 / 2 失败。依赖:仅 Python 标准库 + 已构建的 cutforge-mcp。
"""
from __future__ import annotations

import argparse
import socket
import subprocess
import sys
import tempfile
import time
import shutil
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
WEB = REPO / "apps" / "web"
FIXTURE = REPO / "tests" / "fixtures" / "real_ir" / "project.json"

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def raw_get(port: int, path: str, headers: dict | None = None) -> tuple[int, dict, bytes, float]:
    """原始 socket GET:URL 按字面送达(不经客户端归一化,穿越路径原样进服务端);
    返回 (状态码, 头 dict(小写键), 体, 实测耗时秒)。"""
    t0 = time.perf_counter()
    s = socket.create_connection(("127.0.0.1", port), timeout=15)
    try:
        req = f"GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n"
        for k, v in (headers or {}).items():
            req += f"{k}: {v}\r\n"
        req += "\r\n"
        s.sendall(req.encode("latin-1"))
        buf = b""
        while True:
            chunk = s.recv(65536)
            if not chunk:
                break
            buf += chunk
    finally:
        s.close()
    head, _, body = buf.partition(b"\r\n\r\n")
    lines = head.decode("latin-1").split("\r\n")
    code = int(lines[0].split()[1])
    hdrs = {}
    for l in lines[1:]:
        if ":" in l:
            k, v = l.split(":", 1)
            hdrs[k.strip().lower()] = v.strip()
    return code, hdrs, body, time.perf_counter() - t0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=None)
    args = ap.parse_args()
    bin_path = Path(args.bin) if args.bin else REPO / "target" / "debug" / "cutforge-mcp.exe"
    if not bin_path.is_file():
        bin_path = REPO / "target" / "debug" / "cutforge-mcp"
    if not bin_path.is_file():
        bin_path = REPO / "target" / "release" / "cutforge-mcp.exe"
    if not bin_path.is_file():
        bin_path = REPO / "target" / "release" / "cutforge-mcp"
    if not bin_path.is_file():
        print("FAIL: 先 cargo build -p cutforge-mcp", file=sys.stderr)
        return 2

    token = "static-e2e-token"
    tmp = Path(tempfile.mkdtemp(prefix="cutforge-static-e2e-"))
    ws = tmp / "ws"
    (ws / "05_ir").mkdir(parents=True)
    shutil.copy(FIXTURE, ws / "05_ir" / "project.json")
    port = free_port()
    serve = subprocess.Popen(
        [str(bin_path), "serve", "--root", str(ws), "--port", str(port),
         "--token", token, "--web", str(WEB)],
        stdout=subprocess.DEVNULL, stderr=open(tmp / "serve.log", "wb"))
    try:
        # 就绪等待(/session 数据面)
        deadline = time.time() + 10
        ready = False
        while time.time() < deadline:
            try:
                code, _, body, _ = raw_get(port, f"/session?token={token}")
                if code == 200 and b'"credential"' in body and b'"sessionId"' in body:
                    ready = True
                    break
            except OSError:
                pass
            time.sleep(0.15)
        assert ready, "serve 未就绪"
        print("启动 serve(/session 数据面就绪): PASS")

        # 1) 兼容红线:根路径四别名字节级一致 + Content-Type 同旧 + token 查询参数流
        for p, f, ctype in [("/", "index.html", "text/html"),
                            ("/index.html", "index.html", "text/html"),
                            ("/app.js", "app.js", "text/javascript"),
                            ("/style.css", "style.css", "text/css")]:
            code, hdrs, body, dt = raw_get(port, p)
            assert code == 200, f"{p} 应 200,实得 {code}"
            assert body == (WEB / f).read_bytes(), f"{p} 内容与 apps/web/{f} 字节不一致(兼容红线)"
            assert hdrs.get("content-type", "").startswith(ctype), \
                f"{p} Content-Type 应 {ctype}*,实得 {hdrs.get('content-type')}"
            print(f"兼容 {p} → {ctype}(内容字节一致,{dt * 1000:.1f}ms): PASS")
        code, _, body, _ = raw_get(port, f"/session?token={token}")
        assert code == 200 and b'"sessionId"' in body, "token 查询参数流(数据面)必须照常(S-01:体含会话元数据,不再含主 token)"
        print("兼容 /session?token= 查询参数流: PASS")

        # 2) 目录化:serve 已就绪后新落 js/x.js,零 Rust 改动、零重启立即可达
        js_dir = WEB / "js"
        created_dir = not js_dir.exists()
        js_dir.mkdir(exist_ok=True)
        xjs = js_dir / "x.js"
        xjs.write_text("// e2e_static 临时探针(跑完即删)\nwindow.X = 42;\n", encoding="utf-8")
        try:
            code, hdrs, body, dt = raw_get(port, "/assets/js/x.js")
            assert code == 200, f"/assets/js/x.js 应 200(零 Rust 改动可访问),实得 {code}"
            assert b"window.X = 42" in body, "/assets/js/x.js 内容不符"
            assert hdrs.get("content-type", "").startswith("text/javascript"), \
                f"MIME 应 text/javascript,实得 {hdrs.get('content-type')}"
            print(f"目录化 /assets/js/x.js(新增文件零改动可访问,{dt * 1000:.1f}ms): PASS")

            code, hdrs, body, _ = raw_get(port, "/assets/app.js")
            assert code == 200 and body == (WEB / "app.js").read_bytes(), "既有文件经 /assets 应同一份内容"
            print("目录化 /assets/app.js(同一目录映射): PASS")

            # 3) 穿越 100% 拒绝(400/404;canary 绝不回仓库内容)
            traversals = [
                "/assets/../Cargo.toml",
                "/assets/../../Cargo.toml",
                "/assets/../../tools/e2e_static.py",
                "/assets/%2e%2e/Cargo.toml",
                "/assets/%2E%2E%2F%2E%2E%2FCargo.toml",
                "/assets/..%2FCargo.toml",
                "/assets/..\\..\\Cargo.toml",
                "/assets/%5C..%5C..%5CCargo.toml",
                "/assets/C:%5CCargo.toml",
                "/assets/C:/Cargo.toml",
                "/assets/%252e%252e/%252e%252e/Cargo.toml",
                "/assets/....//....//Cargo.toml",
            ]
            for bad in traversals:
                code, _, body, _ = raw_get(port, bad)
                assert code in (400, 404), f"穿越路径 {bad} 必须拒绝(400/404),实得 {code}"
                assert b"[workspace]" not in body and b"cutforge-mcp" not in body, \
                    f"穿越路径 {bad} 泄露了仓库内容(canary 命中)"
            print(f"穿越 100% 拒绝({len(traversals)} 条字面/编码/反斜杠/盘符/双编码): PASS")

            # 穿越尝试后正常路径仍健康
            code, _, _, _ = raw_get(port, "/assets/js/x.js")
            assert code == 200, "穿越尝试后正常静态路径必须仍健康"
            print("穿越尝试后正常路径健康: PASS")

            # 4) ETag/304
            code, hdrs, _, _ = raw_get(port, "/assets/js/x.js")
            etag = hdrs.get("etag")
            assert etag, "/assets 响应必须带 ETag"
            code, hdrs, body, dt = raw_get(port, "/assets/js/x.js", {"If-None-Match": etag})
            assert code == 304 and not body, f"ETag 命中应 304 空体,实得 {code}/{len(body)}B"
            print(f"ETag/304(/assets/js/x.js,{dt * 1000:.1f}ms): PASS")
            _, h1, _, _ = raw_get(port, "/app.js")
            code, _, body, _ = raw_get(port, "/app.js", {"If-None-Match": h1.get("etag", "")})
            assert code == 304 and not body, "根别名同样应吃 304"
            print("ETag/304(根别名 /app.js): PASS")
        finally:
            xjs.unlink(missing_ok=True)
            if created_dir:
                js_dir.rmdir()

        print("AC-1.6 静态目录托管 e2e: 全部 PASS")
        return 0
    finally:
        serve.terminate()
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except AssertionError as e:
        print(f"FAIL: {e}", file=sys.stderr)
        sys.exit(2)
