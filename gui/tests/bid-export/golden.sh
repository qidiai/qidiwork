#!/usr/bin/env bash
# 依次跑 4 份 golden：bid_smoke 导出 + check.sh 交叉断言
# 调用方约定：在 gui/src-tauri 下执行（与 bid_smoke 的 CWD 一致）
set -u
FAIL=0
run() { # name chapters figures captions tables paragraphs
  local name=$1 ch=$2 fig=$3 cap=$4 tbl=$5 par=$6
  echo "== $name"
  cargo run --quiet --bin bid_smoke -- \
    --fixture "../tests/bid-export/golden/$name.json" \
    --out     "../tests/bid-export/out/$name.docx" || FAIL=1
  bash ../tests/bid-export/check.sh "out/$name.docx" "$ch" "$fig" "$cap" "$tbl" "$par" || FAIL=1
}

run g1-construction 5 2 2 2 15
run g2-gantt        1 1 1 0 2
run g3-siteplan     2 3 3 0 3
run g4-supervision  8 6 2 1 24

[ "$FAIL" -eq 0 ] && echo "GOLDEN OK (4/4)" || { echo "GOLDEN FAIL"; exit 1; }
