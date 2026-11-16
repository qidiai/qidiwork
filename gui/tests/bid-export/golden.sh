#!/usr/bin/env bash
# 依次跑 4 份 golden + 1 组 md：bid_smoke / bid_export 导出 + check.sh 交叉断言
# 调用方约定：在 gui/src-tauri 下执行（与 bid_smoke 的 CWD 一致）
set -u
FAIL=0
run() { # name chapters figures captions tables paragraphs
  local name=$1 ch=$2 fig=$3 cap=$4 tbl=$5 par=$6
  echo "== $name"
  cargo run -q -p qidiwork-cli --bin bid_smoke -- \
    --fixture "../tests/bid-export/golden/$name.json" \
    --out     "../tests/bid-export/out/$name.docx" || FAIL=1
  bash ../tests/bid-export/check.sh "out/$name.docx" "$ch" "$fig" "$cap" "$tbl" "$par" || FAIL=1
}

run g1-construction 5 2 2 2 15
run g2-gantt        1 1 1 0 2
unzip -p ../tests/bid-export/out/g2-gantt.docx word/document.xml | grep -q 'cx="6696000"' || { echo "FAIL wide-width-18.6cm"; FAIL=1; }
run g3-siteplan     2 3 3 0 3
run g4-supervision  8 6 2 1 24

# ===== md 内容入口（P1-1 D3）：demo.md → md-demo.docx =====
echo "== md-demo"
cargo run -q -p qidiwork-cli --bin bid_export -- --md ../tests/bid-export/md/demo.md --out ../tests/bid-export/out/md-demo.docx || FAIL=1
bash ../tests/bid-export/check.sh "out/md-demo.docx" 3 5 5 1 6 || FAIL=1
# BidTitle2 期望 3、BidTitle3 期望 2
MD_DOCX=../tests/bid-export/out/md-demo.docx
md_title2=$(unzip -p "$MD_DOCX" word/document.xml | grep -o 'w:val="BidTitle2"' | wc -l | tr -d ' ')
md_title3=$(unzip -p "$MD_DOCX" word/document.xml | grep -o 'w:val="BidTitle3"' | wc -l | tr -d ' ')
[ "$md_title2" -eq 3 ] && [ "$md_title3" -eq 2 ] || { echo "FAIL md-headings"; FAIL=1; }

[ "$FAIL" -eq 0 ] && echo "GOLDEN OK (5/5)" || { echo "GOLDEN FAIL"; exit 1; }
