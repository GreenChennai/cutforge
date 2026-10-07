"""审片台闭环 e2e(MV 审片台资产包 20261007 → cutforge 移植;对应
apps/web/references/review-loop.md 的机检口径)。断言四条:
①「锚定此帧」创建的意见 anchor.kind=time + tMs=播放头 + tags=审片(铁律⑨:意见=Agent 接口);
②拾取器 Esc 退出零事件残留(overlay 隐藏 + body class 复位);
③拾取点击 → 预填正文模板(selector 锚);
④open 意见机检面(body 非空 / anchor 五类 / tMs 合法)。

骨架与 tools/e2e_edit_ops.py 同款(fixtures/real_ir + Driver);退出码 0/2。
adapted from MV审片台资产包 03_脚本/端到端回归 骨架, Apache-2.0/MIT。
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
FIXTURE = REPO / "tests" / "fixtures" / "real_ir" / "project.json"
TOKEN = "e2e-token"

TAB_NOTES = '[data-tab="notes"]'
BODY = '[data-testid="note-body"]'
CREATE = '[data-testid="note-create"]'
PIN_FRAME = '[data-testid="note-pin-frame"]'
PICK = '[data-testid="note-pick"]'
ANCHOR_BADGE = '[data-testid="note-anchor-badge"]'


def rpc(port: int, name: str, args: dict) -> dict:
    body = json.dumps({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": name, "arguments": {"root": ROOT[0], **args}},
    }).encode()
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/rpc", data=body,
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {TOKEN}"})
    with urllib.request.urlopen(req, timeout=10) as r:
        env = json.loads(r.read())["result"]["content"][0]["text"]
    return json.loads(env)


ROOT = [None]  # spawn 后填工程根(notes_list 必需 root)


def notes_list(port: int, state: str | None = None) -> list[dict]:
    args = {"state": state} if state else {}
    env = rpc(port, "notes_list", args)
    assert env.get("ok"), f"notes_list 失败:{env}"
    return env.get("data", {}).get("notes", [])


def main() -> int:
    bin_path = REPO / "target" / "debug" / "cutforge-mcp.exe"
    if not bin_path.is_file():
        bin_path = REPO / "target" / "release" / "cutforge-mcp.exe"
    if not bin_path.is_file():
        print("FAIL: 先 cargo build(cutforge-mcp)", file=sys.stderr)
        return 2

    from playwright.sync_api import sync_playwright

    tmp = Path(tempfile.mkdtemp(prefix="cutforge-e2e-review-"))
    ws = tmp / "ws"
    (ws / "05_ir").mkdir(parents=True)
    shutil.copy(FIXTURE, ws / "05_ir" / "project.json")

    port = 8899
    serve = subprocess.Popen(
        [str(bin_path), "serve", "--root", str(ws), "--port", str(port),
         "--token", TOKEN, "--web", str(REPO / "apps" / "web")],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.time() + 10
    ready = False
    while time.time() < deadline:
        try:
            urllib.request.urlopen(
                f"http://127.0.0.1:{port}/session?token={TOKEN}", timeout=1).read()
            ready = True
            break
        except Exception:
            time.sleep(0.15)
    assert ready, "serve 10s 未就绪"
    ROOT[0] = str(ws)

    try:
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            page = browser.new_page()
            page.goto(f"http://127.0.0.1:{port}/?token={TOKEN}")
            page.wait_for_function(
                "document.querySelector('[data-testid=\"rev\"]').textContent !== '-'",
                timeout=20000)
            page.click(TAB_NOTES)

            # ① 锚定此帧 → 创建 → notes_list 断言 kind=time + tags=审片
            page.click(PIN_FRAME)
            badge = page.text_content(ANCHOR_BADGE) or ""
            assert "此帧" in badge, f"锚定徽标未更新:{badge!r}"
            page.fill(BODY, "审片意见:此帧调色偏冷,请加暖")
            page.click(CREATE)
            page.wait_for_timeout(800)
            mine = [n for n in notes_list(port) if "调色偏冷" in (n.get("body") or "")]
            assert mine, "意见未在 notes_list 出现(违反铁律⑨ 意见=Agent 接口)"
            n0 = mine[0]
            anchor = n0.get("anchor") or {}
            assert anchor.get("kind") == "time", f"锚定帧意见必须 kind=time:{anchor}"
            assert isinstance(anchor.get("tMs"), int) and anchor["tMs"] >= 0, \
                f"锚定帧意见必须带合法 tMs:{anchor}"
            assert "审片" in (n0.get("tags") or []), f"审片口径 tags 缺失:{n0.get('tags')}"
            print("E2E-R1 锚定此帧→notes_add(kind=time,tMs,tags=审片): PASS")

            # ② 拾取器进出零残留(Esc;不点击元素,避免落第二条意见)
            page.click(PICK)
            page.wait_for_timeout(300)
            assert page.evaluate(
                "!!document.getElementById('review-inspect-overlay')"
            ), "拾取 overlay 未创建"
            page.keyboard.press("Escape")
            page.wait_for_timeout(200)
            assert not page.evaluate(
                "document.body.classList.contains('review-inspecting')"
            ), "检查模式退出后 body class 未复位(事件残留口径)"
            assert page.evaluate(
                "document.getElementById('review-inspect-overlay').style.display === 'none'"
            ), "拾取 overlay 未隐藏"
            print("E2E-R2 拾取器 Esc 退出零残留: PASS")

            # ③ 拾取点击 → 预填正文(点确定元素:标注页签按钮;force 穿透 overlay)
            page.click(PICK)
            page.wait_for_timeout(200)
            page.click('[data-testid="tab-notes"]', force=True)
            page.wait_for_timeout(300)
            val = page.input_value(BODY) or ""
            assert val.startswith("["), f"拾取预填正文缺 selector 锚:{val!r}"
            assert "这里要怎么改" in val, f"拾取预填正文缺模板:{val!r}"
            page.fill(BODY, "")  # 清场:预填不落意见
            page.keyboard.press("Escape")  # 兜底退出(若点击被 overlay 吞)
            page.wait_for_timeout(200)
            print("E2E-R3 拾取点击预填正文(selector+模板): PASS")

            # ④ open 意见机检面(铁律⑨:body 非空 + anchor 五类 + tMs 合法)
            for n in notes_list(port, "open"):
                assert (n.get("body") or "").strip(), \
                    f"open 意见 {n.get('id')} body 为空(流程违规)"
                a = n.get("anchor") or {}
                assert a.get("kind") in ("clip", "track", "time", "word", "subtitleCard"), \
                    f"open 意见 {n.get('id')} anchor 非法:{a}"
                assert isinstance(a.get("tMs", 0), int) and a.get("tMs", 0) >= 0, \
                    f"open 意见 {n.get('id')} tMs 非法:{a}"
            print("E2E-R4 open 意见机检面(body/anchor/tMs): PASS")

            print("审片台闭环 e2e:4/4 PASS")
            return 0
    finally:
        serve.terminate()


if __name__ == "__main__":
    sys.exit(main())
