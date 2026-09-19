#!/usr/bin/env sh
# CutForge 编辑器一键启动器(产品自带,E1-4):PATH 的 cutforge-cli 优先,回退 target 构建产物。
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if command -v cutforge-cli >/dev/null 2>&1; then
  CLI=cutforge-cli
elif [ -x "$DIR/target/release/cutforge-cli" ]; then
  CLI="$DIR/target/release/cutforge-cli"
elif [ -x "$DIR/target/debug/cutforge-cli" ]; then
  CLI="$DIR/target/debug/cutforge-cli"
else
  echo "未找到 cutforge-cli:请下载官方预编译包并加入 PATH,或在仓库根执行 cargo build --release" >&2
  exit 3
fi
exec "$CLI" serve --open "$@"
