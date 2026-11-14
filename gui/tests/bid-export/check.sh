#!/usr/bin/env bash
# T2c2 产物校验脚本（规格 §6.7）：纯 unzip + grep + wc。
# $1 = 待检 docx（默认 out/p0a-sample.docx，相对脚本所在目录解析）；逐条打印 PASS/FAIL name；任一 FAIL exit 1。
set -u

DOCX="${1:-out/p0a-sample.docx}"
# $1 相对脚本所在目录解析（out/ 与脚本同目录），绝对路径原样保留
case "$DOCX" in
  /*) ;;
  *) DOCX="$(dirname "$0")/$DOCX" ;;
esac

unzip -p "$DOCX" word/styles.xml   > /tmp/bid-styles.xml
unzip -p "$DOCX" word/document.xml > /tmp/bid-doc.xml

fail=0
check() { # $1=name, $2=exit-status-of-check
  if [ "$2" -eq 0 ]; then echo "PASS $1"; else echo "FAIL $1"; fail=1; fi
}

# 1) 10 个 Bid 样式
[ "$(grep -o 'w:styleId="Bid[A-Za-z0-9]*"' /tmp/bid-styles.xml | sort -u | wc -l)" -eq 10 ]
check "bid-styles-10" $?
# 2) 页面
grep -q 'w:w="11906"' /tmp/bid-doc.xml && grep -q 'w:h="16838"' /tmp/bid-doc.xml
check "page-size" $?
grep -q 'w:top="680"' /tmp/bid-doc.xml
check "page-margin-top" $?
# 3) 计数（必须 -o | wc -l，禁止 grep -c；BidTitle1 = 文档标题 1 + 每章 1）
[ "$(grep -o '<w:drawing' /tmp/bid-doc.xml | wc -l)" -eq 1 ]
check "drawing-1" $?
[ "$(grep -o 'w:pStyle w:val="BidCaption"' /tmp/bid-doc.xml | wc -l)" -eq 1 ]
check "caption-1" $?
[ "$(grep -o 'w:pStyle w:val="BidTitle1"' /tmp/bid-doc.xml | wc -l)" -eq 3 ]
check "title1-3" $?
[ "$(grep -o 'w:pStyle w:val="BidBody"' /tmp/bid-doc.xml | wc -l)" -eq 4 ]
check "body-4" $?
[ "$(grep -o '<w:tbl>' /tmp/bid-doc.xml | wc -l)" -eq 1 ]
check "tbl-1" $?
[ "$(grep -o 'w:pageBreakBefore' /tmp/bid-doc.xml | wc -l)" -eq 2 ]
check "pageBreakBefore-2" $?
# 4) 部件库残留（只查被整块删除的部件与占位文本；外层 sdt 包装保留属正常）
! grep -q 'BID_TABLE_ROWS\|BID_FIGURE_SINGLE\|正文占位\|图片占位\|>图注<' /tmp/bid-doc.xml
check "no-fragment-left" $?

exit $fail
