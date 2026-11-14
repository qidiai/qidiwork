//! T2b：产物分层校验器（规格 §6.5）。
//!
//! 职责：对填充/导出后的 docx 做 16 项分层校验（样式 #1-#7 / 页面 #8-#9 / 文档 #10-#16），
//! 顺序即规格表格顺序，输出 `VerifyReport`（`Serialize`，供 T3 命令返回）。
//!
//! 实现约定（规格 §6.5）：
//! - styles.xml 按 `w:styleId="X"` 定位样式块：向前找最近 `<w:style`，向后找 `</w:style>`；
//!   找不到该样式 → 该 check `ok=false`、detail 写 `缺样式 X`，不 panic、不 Err。
//! - 计数用 `xml.matches(needle).count()`。
//! - 比较前先 `replace(" />", "/>")` 归一。
//! - `expect=None` → #11-#16 计数类 check `ok=true` + detail `skipped`。

use serde::Serialize;
use std::path::Path;

use super::docx::DocxPackage;
use super::BidResult;

/// 期望计数（`expect=None` → 只查结构 #1-#10，#11-#16 skipped）
#[derive(Debug, Clone, Copy)]
pub struct Expect {
    pub chapters: usize,
    pub figures: usize,
    pub tables: usize,
    pub paragraphs: usize,
}

/// 单条校验结果
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// 校验报告（T3 命令返回体）
#[derive(Debug, Clone, Serialize)]
pub struct VerifyReport {
    pub checks: Vec<Check>,
}

impl VerifyReport {
    /// 全部 check 通过才算通过
    pub fn ok(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }

    /// 序列化为 JSON 字符串（serde_json）
    pub fn to_json(&self) -> String {
        serde_json::json!({ "ok": self.ok(), "checks": self.checks }).to_string()
    }
}

/// 十项 Bid 样式 id（规格 §6.5 #1）
const BID_STYLES: [&str; 10] = [
    "BidBody",
    "BidTitle1",
    "BidTitle2",
    "BidTitle3",
    "BidTitle4",
    "BidTableHeader",
    "BidTableBody",
    "BidCaption",
    "BidFigureCell",
    "BidTable",
];

/// 对 `path` 处 docx 做 16 项分层校验；`expect=None` 时跳过计数类 check（#11-#16）
pub fn verify(path: &Path, expect: Option<&Expect>) -> BidResult<VerifyReport> {
    let pkg = DocxPackage::open(path)?;
    let styles = pkg.text("word/styles.xml")?;
    let document = pkg.text("word/document.xml")?;

    // 归一：属性尾部的 " />" 并成 "/>"，使属性写法断言与生成器输出对齐
    let styles = styles.replace(" />", "/>");
    let document = document.replace(" />", "/>");

    let mut checks = Vec::with_capacity(16);

    // #1 styles.present：w:styleId="Bid…" 十项齐全
    let missing: Vec<&str> = BID_STYLES
        .iter()
        .filter(|id| !styles.contains(&format!("w:styleId=\"{id}\"")))
        .copied()
        .collect();
    checks.push(Check {
        name: "styles.present".to_string(),
        ok: missing.is_empty(),
        detail: if missing.is_empty() {
            "Bid 样式十项齐全".to_string()
        } else {
            format!("缺样式: {}", missing.join("、"))
        },
    });

    // #2-#6 段落样式块内容断言
    checks.push(style_check(
        "styles.body",
        &styles,
        "BidBody",
        &[
            r#"<w:spacing w:line="360" w:lineRule="auto"/>"#,
            r#"<w:ind w:firstLineChars="200"/>"#,
            r#"<w:jc w:val="both"/>"#,
            r#"<w:sz w:val="24"/>"#,
        ],
    ));
    checks.push(style_check(
        "styles.title1",
        &styles,
        "BidTitle1",
        &[
            "253D57",
            r#"<w:sz w:val="30"/>"#,
            "<w:b/>",
            r#"<w:jc w:val="center"/>"#,
        ],
    ));
    checks.push(style_check(
        "styles.title2",
        &styles,
        "BidTitle2",
        &["315276", r#"<w:sz w:val="28"/>"#, "<w:b/>"],
    ));
    checks.push(style_check(
        "styles.table_header",
        &styles,
        "BidTableHeader",
        &[r#"w:fill="4877AC""#, r#"w:val="FFFFFF""#, "<w:b/>"],
    ));
    checks.push(style_check(
        "styles.caption",
        &styles,
        "BidCaption",
        &[r#"<w:sz w:val="24"/>"#, "<w:b/>", r#"<w:jc w:val="center"/>"#],
    ));

    // #7 styles.table_borders：BidTable 块边框属性（w:sz="4" 为属性写法，非元素写法）
    checks.push(style_check(
        "styles.table_borders",
        &styles,
        "BidTable",
        &[
            r#"w:color="74A6CC""#,
            r#"w:color="B0D0E8""#,
            r#"w:sz="4""#,
        ],
    ));

    // #8 page.size
    checks.push(contains_check(
        "page.size",
        &document,
        &[r#"w:w="11906""#, r#"w:h="16838""#],
    ));

    // #9 page.margins
    checks.push(contains_check(
        "page.margins",
        &document,
        &[
            r#"w:top="680""#,
            r#"w:bottom="680""#,
            r#"w:left="680""#,
            r#"w:right="680""#,
        ],
    ));

    // #10 doc.no_template_parts：不得含模板部件残留
    //     （BID_DOC_TITLE/BID_CHAPTER* 的外层 sdt 包装在成品里是正常的，不断言其不存在）
    let forbidden = [
        "BID_TABLE_ROWS",
        "BID_FIGURE_SINGLE",
        "正文占位",
        "图片占位",
        ">图注<",
    ];
    let leaked: Vec<&str> = forbidden
        .iter()
        .copied()
        .filter(|n| document.contains(n))
        .collect();
    checks.push(Check {
        name: "doc.no_template_parts".to_string(),
        ok: leaked.is_empty(),
        detail: if leaked.is_empty() {
            "无模板部件残留".to_string()
        } else {
            format!("残留模板部件: {}", leaked.join("、"))
        },
    });

    // #11-#16 计数类 check：expect=None → ok=true + detail "skipped"
    checks.push(count_check(
        "doc.chapters",
        expect,
        &document,
        r#"w:pStyle w:val="BidTitle1""#,
        |e| e.chapters + 1,
    ));
    checks.push(count_check(
        "doc.page_breaks",
        expect,
        &document,
        "w:pageBreakBefore",
        |e| e.chapters,
    ));
    checks.push(count_check(
        "doc.paragraphs",
        expect,
        &document,
        r#"w:pStyle w:val="BidBody""#,
        |e| e.paragraphs,
    ));
    checks.push(count_check(
        "doc.tables",
        expect,
        &document,
        "<w:tbl>",
        |e| e.tables,
    ));
    checks.push(count_check(
        "doc.figures",
        expect,
        &document,
        "<w:drawing",
        |e| e.figures,
    ));
    checks.push(count_check(
        "doc.captions",
        expect,
        &document,
        r#"w:pStyle w:val="BidCaption""#,
        |e| e.figures,
    ));

    Ok(VerifyReport { checks })
}

/// 按 `w:styleId="X"` 定位样式块：向前找最近 `<w:style`，向后找 `</w:style>`
fn style_block<'a>(styles: &'a str, id: &str) -> Option<&'a str> {
    let at = styles.find(&format!("w:styleId=\"{id}\""))?;
    let start = styles[..at].rfind("<w:style")?;
    let end_rel = styles[at..].find("</w:style>")?;
    Some(&styles[start..at + end_rel + "</w:style>".len()])
}

/// 样式块内容断言（#2-#7）：块内须含全部 needle；找不到样式 → ok=false + `缺样式 X`
fn style_check(name: &str, styles: &str, id: &str, needles: &[&str]) -> Check {
    match style_block(styles, id) {
        None => Check {
            name: name.to_string(),
            ok: false,
            detail: format!("缺样式 {id}"),
        },
        Some(block) => {
            let missing: Vec<&str> = needles
                .iter()
                .copied()
                .filter(|n| !block.contains(n))
                .collect();
            Check {
                name: name.to_string(),
                ok: missing.is_empty(),
                detail: if missing.is_empty() {
                    format!("{id} 块 {} 项齐全", needles.len())
                } else {
                    format!("{id} 块缺: {}", missing.join("、"))
                },
            }
        }
    }
}

/// 包含断言（#8-#9）：xml 须含全部 needle
fn contains_check(name: &str, xml: &str, needles: &[&str]) -> Check {
    let missing: Vec<&str> = needles.iter().copied().filter(|n| !xml.contains(n)).collect();
    Check {
        name: name.to_string(),
        ok: missing.is_empty(),
        detail: if missing.is_empty() {
            format!("{} 项齐全", needles.len())
        } else {
            format!("缺: {}", missing.join("、"))
        },
    }
}

/// 计数断言（#11-#16）：`xml.matches(needle).count()` == want_of(expect)
fn count_check(
    name: &str,
    expect: Option<&Expect>,
    xml: &str,
    needle: &str,
    want_of: impl FnOnce(&Expect) -> usize,
) -> Check {
    let Some(e) = expect else {
        return Check {
            name: name.to_string(),
            ok: true,
            detail: "skipped".to_string(),
        };
    };
    let want = want_of(e);
    let got = xml.matches(needle).count();
    Check {
        name: name.to_string(),
        ok: got == want,
        detail: format!("{needle} 计数 {got}，期望 {want}"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::office::bid::docx::DocxPackage;

    const TEMPLATE: &str = "resources/templates/bid-engineering-blue-v1.docx";

    /// §6.5 表格顺序（styles.present 打头，doc.captions 收尾）
    const SPEC_ORDER: [&str; 16] = [
        "styles.present",
        "styles.body",
        "styles.title1",
        "styles.title2",
        "styles.table_header",
        "styles.caption",
        "styles.table_borders",
        "page.size",
        "page.margins",
        "doc.no_template_parts",
        "doc.chapters",
        "doc.page_breaks",
        "doc.paragraphs",
        "doc.tables",
        "doc.figures",
        "doc.captions",
    ];

    fn check<'a>(report: &'a VerifyReport, name: &str) -> &'a Check {
        report.checks.iter().find(|c| c.name == name).unwrap()
    }

    /// 对模板 docx 跑 verify(None)：16 条 check，名字顺序与 §6.5 表格一致
    #[test]
    fn report_has_16_checks_in_spec_order() {
        let report = verify(Path::new(TEMPLATE), None).unwrap();
        assert_eq!(report.checks.len(), 16);
        let names: Vec<&str> = report.checks.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, SPEC_ORDER);
        assert_eq!(report.checks.first().unwrap().name, "styles.present");
        assert_eq!(report.checks.last().unwrap().name, "doc.captions");
    }

    /// styles.xml 的 `<w:sz w:val="30"/>` 改成 28 → styles.title1 必须 ok=false
    #[test]
    fn tampered_title1_size_fails() {
        let mut pkg = DocxPackage::open(Path::new(TEMPLATE)).unwrap();
        let styles = pkg.text("word/styles.xml").unwrap();
        assert!(styles.contains(r#"<w:sz w:val="30"/>"#));
        pkg.set_text(
            "word/styles.xml",
            styles.replace(r#"<w:sz w:val="30"/>"#, r#"<w:sz w:val="28"/>"#),
        );
        let path = std::env::temp_dir().join(format!("qidi-bid-t2b-verify-sz-{}", std::process::id()));
        pkg.save(&path).unwrap();

        let report = verify(&path, None).unwrap();
        assert!(!check(&report, "styles.title1").ok, "{:?}", check(&report, "styles.title1"));

        let _ = std::fs::remove_file(&path);
    }

    /// document.xml 的 `w:top="680"` 改成 681 → page.margins 必须 ok=false
    #[test]
    fn tampered_margin_fails() {
        let mut pkg = DocxPackage::open(Path::new(TEMPLATE)).unwrap();
        let document = pkg.text("word/document.xml").unwrap();
        assert!(document.contains(r#"w:top="680""#));
        pkg.set_text(
            "word/document.xml",
            document.replace(r#"w:top="680""#, r#"w:top="681""#),
        );
        let path =
            std::env::temp_dir().join(format!("qidi-bid-t2b-verify-margin-{}", std::process::id()));
        pkg.save(&path).unwrap();

        let report = verify(&path, None).unwrap();
        assert!(!check(&report, "page.margins").ok, "{:?}", check(&report, "page.margins"));

        let _ = std::fs::remove_file(&path);
    }

    /// expect 全 9 → doc.chapters / doc.figures 必须 ok=false
    /// （模板里 BidTitle1 只有 2 处、drawing 0 处）
    #[test]
    fn expect_mismatch_fails() {
        let report = verify(
            Path::new(TEMPLATE),
            Some(&Expect {
                chapters: 9,
                figures: 9,
                tables: 9,
                paragraphs: 9,
            }),
        )
        .unwrap();
        assert!(!check(&report, "doc.chapters").ok, "{:?}", check(&report, "doc.chapters"));
        assert!(!check(&report, "doc.figures").ok, "{:?}", check(&report, "doc.figures"));
    }
}
