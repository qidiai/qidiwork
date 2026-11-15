#!/usr/bin/env bash
# P0b §5.2 参数化产物校验脚本：纯 unzip + grep + awk。
# 用法：check.sh <docx> <chapters> <figures> <captions> <tables> <paragraphs>
# $1 = 待检 docx（相对脚本所在目录解析）；$2..$6 = 期望计数；缺参数 → echo 用法; exit 2。
# 逐条打印 PASS/FAIL name；任一 FAIL exit 1。
set -u

if [ "$#" -lt 6 ]; then
  echo "用法: $0 <docx> <chapters> <figures> <captions> <tables> <paragraphs>"
  exit 2
fi

DOCX=$1
ch=$2; fig=$3; cap=$4; tbl=$5; par=$6
# $1 相对脚本所在目录解析（out/ 与脚本同目录），绝对路径原样保留
case "$DOCX" in
  /*) ;;
  [A-Za-z]:[\\/]*) ;;
  \\\\*) ;;
  *) DOCX="$(dirname "$0")/$DOCX" ;;
esac

unzip -p "$DOCX" word/styles.xml   > /tmp/bid-styles.xml
unzip -p "$DOCX" word/document.xml > /tmp/bid-doc.xml

fail=0
check() { # $1=name, $2=exit-status-of-check
  if [ "$2" -eq 0 ]; then echo "PASS $1"; else echo "FAIL $1"; fail=1; fi
}

# 1) 13 个 Bid 样式
[ "$(grep -o 'w:styleId="Bid[A-Za-z0-9]*"' /tmp/bid-styles.xml | sort -u | wc -l)" -eq 13 ]
check "bid-styles-13" $?
# 2) 页面
grep -q 'w:w="11906"' /tmp/bid-doc.xml && grep -q 'w:h="16838"' /tmp/bid-doc.xml
check "page-size" $?
grep -q 'w:top="680"' /tmp/bid-doc.xml
check "page-margin-top" $?
# 3) TOC 域
grep -q 'TOC' /tmp/bid-doc.xml
check "toc-field" $?
# 4) 页眉页脚部件与页码域
unzip -l "$DOCX" | grep -q 'header1' && unzip -l "$DOCX" | grep -q 'footer1'
check "header-footer-parts" $?
unzip -p "$DOCX" word/footer1.xml | grep -q 'PAGE' && unzip -p "$DOCX" word/footer1.xml | grep -q 'NUMPAGES'
check "page-number-field" $?
# 5) 计数（BidTitle1 = 文档标题 1 + 每章 1；tables 按 <w:tbl> 切块判 BidTableHeader，3up 图桌不计）
[ "$(grep -o 'w:pStyle w:val="BidTitle1"' /tmp/bid-doc.xml | wc -l)" -eq "$((ch + 1))" ]
check "chapters-title1" $?
[ "$(grep -o 'w:pageBreakBefore' /tmp/bid-doc.xml | wc -l)" -eq "$ch" ]
check "chapters-pagebreak" $?
[ "$(grep -o '<w:drawing' /tmp/bid-doc.xml | wc -l)" -eq "$fig" ]
check "figures" $?
[ "$(grep -o 'w:pStyle w:val="BidCaption"' /tmp/bid-doc.xml | wc -l)" -eq "$cap" ]
check "captions" $?
[ "$(awk 'BEGIN{RS="<w:tbl>"} /BidTableHeader/{c++} END{print c+0}' /tmp/bid-doc.xml)" -eq "$tbl" ]
check "tables" $?
[ "$(grep -o 'w:pStyle w:val="BidBody"' /tmp/bid-doc.xml | wc -l)" -eq "$par" ]
check "paragraphs" $?
# 6) 部件库残留（只查被整块删除的部件与占位文本；外层 sdt 包装保留属正常）
! grep -q 'BID_TABLE_ROWS\|BID_FIGURE_SINGLE\|BID_FIGURE_3UP\|BID_FIGURE_WIDE\|正文占位\|图片占位' /tmp/bid-doc.xml
check "no-fragment-left" $?

exit $fail
