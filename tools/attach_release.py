#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""M13 收尾:从 CI run 下载三平台产物并附加到 GitHub Release v0.2.0。"""
import json
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

REPO = "GreenChennai/cutforge"
RUN = "35390276916"
TAG = "v0.2.0"

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")


def gh(*args, binary=False):
    r = subprocess.run(["gh", *args], capture_output=True)
    if r.returncode != 0:
        print("gh fail:", r.stderr.decode("utf-8", "replace")[:300], file=sys.stderr)
        return None
    return r.stdout if binary else r.stdout.decode("utf-8", "replace")


# 1) run 级 artifacts(ubuntu/windows/macos 三份)
data = gh("api", f"repos/{REPO}/actions/runs/{RUN}/artifacts")
assert data is not None
all_arts = [(a["id"], a["name"]) for a in json.loads(data)["artifacts"]]
# 同平台取 id 最大(最新)一份
best = {}
for aid, name in all_arts:
    best[name] = max(best.get(name, 0), aid)
artifacts = [(aid, name) for name, aid in best.items()]
print("artifacts:", artifacts)

# 2) 下载并解压到暂存目录,收集 dist 内全部文件
tmp = Path(tempfile.mkdtemp(prefix="cutforge-rel-"))
uploads = []
for aid, name in artifacts:
    z = tmp / f"{name}.zip"
    z.write_bytes(gh("api", f"repos/{REPO}/actions/artifacts/{aid}/zip", binary=True))
    out_dir = tmp / name
    with zipfile.ZipFile(z) as zf:
        zf.extractall(out_dir)
    # 统一平台前缀:所有产物改名 {artifact}-{原名},跨平台绝不重名
    for f in out_dir.rglob("*"):
        if f.is_file():
            target = f.parent / f"{name}-{f.name}"
            k = 1
            while target.exists():
                target = f.parent / f"{name}-{k}-{f.name}"
                k += 1
            f.rename(target)
    for f in out_dir.rglob("*"):
        if f.is_file():
            uploads.append(f)
print("files to upload:", [f.name for f in uploads])

# 3) 转正 Release(删 tag 时被转为 draft)并附加
gh("release", "edit", TAG, "--draft=false", "-R", REPO)
# 清空旧资产(前次部分上传的残缺组合)
cur = json.loads(gh("release", "view", TAG, "--json", "assets", "-R", REPO))
for a in cur["assets"]:
    gh("api", "-X", "DELETE", f"repos/{REPO}/releases/assets/{a['id']}")
cmd = ["gh", "release", "upload", TAG, *[str(f) for f in uploads], "--clobber", "-R", REPO]
r = subprocess.run(cmd, capture_output=True)
print("upload:", r.returncode, r.stderr.decode("utf-8", "replace")[:200])
final = json.loads(gh("release", "view", TAG, "--json", "assets", "-R", REPO))
print("assets:", [a["name"] for a in final["assets"]])
