#!/usr/bin/env bash
set -euo pipefail
# MSYS/Git Bash 下 pwd 输出 /g/qidiwork/... 风格路径,npm(Windows 程序)会当成
# 相对路径拼出 G:\g\qidiwork\... 而找不到 package.json;pwd -W 输出 G:/... 的
# Windows 路径。-W 不存在时(WSL/Linux)退回普通 pwd。
ROOT="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
HOST="ubuntu@111.231.166.204"

npm --prefix "$ROOT" test
if [ "${DRY_RUN:-0}" = "1" ]; then
  printf '%s\n' '构建和测试完成，未上传。'
  exit 0
fi

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/public" "$STAGE/download" "$STAGE/server"
cp "$ROOT/dist/index.html" "$ROOT/dist/sitemap.xml" "$ROOT/dist/robots.txt" "$ROOT/dist/og-cover.png" "$STAGE/public/"
cp -R "$ROOT/dist/assets" "$ROOT/dist/tech" "$ROOT/dist/blog" "$ROOT/dist/admin" "$ROOT/dist/privacy" "$STAGE/public/"
cp "$ROOT/dist/download/index.html" "$ROOT/dist/download/analytics.js" "$ROOT/dist/download/qidi-logo.svg" "$STAGE/download/"
cp "$ROOT/server/analytics.py" "$ROOT/server/deploy.py" "$ROOT/server/qidi-analytics.service" "$ROOT/server/qidi-analytics.nginx" "$STAGE/server/"
tar -czf "$STAGE/release.tar.gz" -C "$STAGE" public download server

REMOTE_DIR=$(ssh -o BatchMode=yes -o ConnectTimeout=15 "$HOST" 'mktemp -d "$HOME/qidi-deploy-XXXXXXXX"')
[[ "$REMOTE_DIR" =~ ^/home/ubuntu/qidi-deploy-[A-Za-z0-9]+$ ]] || { printf '%s\n' '远程暂存目录异常'; exit 1; }
scp "$STAGE/release.tar.gz" "$HOST:$REMOTE_DIR/release.tar.gz"
ssh -o BatchMode=yes "$HOST" "tar -xzf '$REMOTE_DIR/release.tar.gz' -C '$REMOTE_DIR' && sudo -n python3 '$REMOTE_DIR/server/deploy.py' '$REMOTE_DIR'"
printf '部署命令完成；暂存目录 %s，仍需线上浏览器验收。\n' "$REMOTE_DIR"
