//! T2b：产物分层校验器（规格 §6.5 / P0b §4.3）。
//!
//! 职责：对填充/导出后的 docx 做 21 项分层校验（样式 #1-#7 / 页面 #8-#9 / 文档结构 #10-#14 / 计数 #15-#21），
//! 顺序即规格表格顺序，输出 `VerifyReport`（`Serialize`，供 T3 命令返回）。
//!
//! 实现约定（规格 §6.5）：
//! - styles.xml 按 `w:styleId="X"` 定位样式块：向前找最近 `<w:style`，向后找 `</w:style>`；
//!   找不到该样式 → 该 check `ok=false`、detail 写 `缺样式 X`，不 panic、不 Err。
//! - 计数用 `xml.matches(needle).count()`。
//! - 比较前先 `replace(" />", "/>")` 归一。
//! - `expect=None` → #14-#20 计数类 check `ok=true` + detail `skipped`。

use serde::Serialize;
use std::path::Path;

use super::docx::DocxPackage;
use super::BidResult;

/// 期望计数（`expect=None` → 只查结构 #1-#13，#14-#20 skipped）
#[derive(Debug, Clone, Copy)]
pub struct Expect {
    pub chapters: usize,
    pub figures: usize,
    pub captions: usize,
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

/// 十三项 Bid 样式 id（规格 §6.5 #1；P0b §4.3 加 BidTOC/BidHeader/BidFooter）
const BID_STYLES: [&str; 13] = [
    "BidBody",
    "BidTitle1",
    "BidTitle2",
    "BidTitle3",
    "BidTitle4",
    "BidTableHeader",
    "BidTableBody",
    "BidCaption",
    "BidFigureCell",
    "BidTOC",
    "BidHeader",
    "BidFooter",
    "BidTable",
];

/// 对 `path` 处 docx 做 21 项分层校验；`expect=None` 时跳过计数类 check（#15-#21）
pub fn verify(path: &Path, expect: Option<&Expect>) -> BidResult<VerifyReport> {
    let pkg = DocxPackage::open(path)?;
    let styles = pkg.text("word/styles.xml")?;
    let document = pkg.text("word/document.xml")?;

    // 归一：属性尾部的 " />" 并成 "/>"，使属性写法断言与生成器输出对齐
    let styles = styles.replace(" />", "/>");
    let document = document.replace(" />", "/>");

    let mut checks = Vec::with_capacity(21);

    // #1 styles.present：w:styleId="Bid…" 十三项齐全
    let missing: Vec<&str> = BID_STYLES
        .iter()
        .filter(|id| !styles.contains(&format!("w:styleId=\"{id}\"")))
        .copied()
        .collect();
    checks.push(Check {
        name: "styles.present".to_string(),
        ok: missing.is_empty(),
        detail: if missing.is_empty() {
            "Bid 样式十三项齐全".to_string()
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
            r#"<w:outlineLvl w:val="0"/>"#,
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

    // #10-#14 文档结构 check（P0b §4.3 新增 4 条，插在 page.margins 之后；B6 插 doc.styles_link）
    // #10 doc.toc：目录域仍在（填充器不动 BID_TOC）。限定 w:val="BID_TOC" 的 sdt 块内断言；找不到块 → ok=false
    let toc_ok = match sdt_block(&document, "BID_TOC") {
        None => false,
        Some(block) => block.contains("fldSimple") && block.contains("TOC"),
    };
    checks.push(Check {
        name: "doc.toc".to_string(),
        ok: toc_ok,
        detail: if toc_ok {
            "BID_TOC 块目录域齐全".to_string()
        } else {
            "缺 BID_TOC 块或块内无 fldSimple/TOC".to_string()
        },
    });
    // #11 doc.header_footer：页眉页脚部件 + content-types + document 引用齐全
    let content_types = pkg.text("[Content_Types].xml")?;
    let hf_ok = pkg.contains("word/header1.xml")
        && pkg.contains("word/footer1.xml")
        && content_types.contains("header+xml")
        && content_types.contains("footer+xml")
        && document.contains("headerReference")
        && document.contains("footerReference");
    checks.push(Check {
        name: "doc.header_footer".to_string(),
        ok: hf_ok,
        detail: if hf_ok {
            "页眉页脚部件齐全".to_string()
        } else {
            "缺页眉/页脚部件、content-types 或 document 引用".to_string()
        },
    });
    // #12 doc.styles_link：document.xml.rels 须声明 styles.xml / settings.xml 关系
    //     （缺这两条 WPS 不应用样式、TOC 更新为空；P0a 模板 bug B6）
    let rels = pkg.text("word/_rels/document.xml.rels")?;
    let styles_link_ok = rels.contains(r#"relationships/styles" Target="styles.xml""#)
        && rels.contains(r#"relationships/settings" Target="settings.xml""#);
    checks.push(Check {
        name: "doc.styles_link".to_string(),
        ok: styles_link_ok,
        detail: if styles_link_ok {
            "styles/settings 关系齐全".to_string()
        } else {
            "document.xml.rels 缺 styles.xml 或 settings.xml 关系".to_string()
        },
    });
    // #13 page_number_field：页脚含 PAGE / NUMPAGES 域
    let footer = pkg.text("word/footer1.xml")?;
    checks.push(contains_check(
        "page_number_field",
        &footer,
        &[r#"instr=" PAGE ""#, r#"instr=" NUMPAGES ""#],
    ));
    // #14 table.first_col_fill：含 BidTableHeader 的业务表逐行断言首列底纹；无业务表 → skipped
    //     （块内 <w:tr> 数 = R（含表头）；w:fill="E6F2F9" 出现次数 == R-1；第 2..R 行首个 <w:tc> 必含该 shd）
    let biz_tables: Vec<&str> = tables_with(&document, "BidTableHeader");
    let first_col_ok = biz_tables.iter().all(|t| first_col_fill_ok(t));
    checks.push(Check {
        name: "table.first_col_fill".to_string(),
        ok: first_col_ok,
        detail: if biz_tables.is_empty() {
            "skipped".to_string()
        } else if first_col_ok {
            format!("{} 张业务表首列底纹逐行齐全", biz_tables.len())
        } else {
            "业务表首列底纹逐行断言不通过（fill 计数或首列 shd 缺失）".to_string()
        },
    });

    // #15 doc.no_template_parts：不得含模板部件残留
    //     （BID_DOC_TITLE/BID_CHAPTER* 的外层 sdt 包装在成品里是正常的，不断言其不存在）
    let forbidden = [
        "BID_TABLE_ROWS",
        "BID_FIGURE_SINGLE",
        "BID_FIGURE_3UP",
        "BID_FIGURE_WIDE",
        "BID_TABLE_ROW",
        "BID_FIGURE_CAPTION",
        "{{DOC_TITLE}}",
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

    // #15-#21 计数类 check：expect=None → ok=true + detail "skipped"
    checks.push(count_check(
        "doc.chapters",
        expect,
        document.matches(r#"w:pStyle w:val="BidTitle1""#).count(),
        |e| e.chapters + 1,
    ));
    checks.push(count_check(
        "doc.page_breaks",
        expect,
        document.matches("w:pageBreakBefore").count(),
        |e| e.chapters,
    ));
    checks.push(count_check(
        "doc.paragraphs",
        expect,
        document.matches(r#"w:pStyle w:val="BidBody""#).count(),
        |e| e.paragraphs,
    ));
    checks.push(count_check(
        "doc.tables",
        expect,
        count_tables_with(&document, "BidTableHeader"),
        |e| e.tables,
    ));
    checks.push(count_check(
        "doc.figures",
        expect,
        document.matches("<w:drawing").count(),
        |e| e.figures,
    ));
    checks.push(count_check(
        "doc.captions",
        expect,
        document.matches("BidCaption").count(),
        |e| e.captions,
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

/// 计数断言（#14-#20）：got == want_of(expect)
fn count_check(
    name: &str,
    expect: Option<&Expect>,
    got: usize,
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
    Check {
        name: name.to_string(),
        ok: got == want,
        detail: format!("计数 {got}，期望 {want}"),
    }
}

/// 按 `<w:tbl>` 切块，返回含 `needle` 的表块（`</w:tbl>` 止；3up 图桌不含 BidTableHeader，自然排除）
fn tables_with<'a>(xml: &'a str, needle: &str) -> Vec<&'a str> {
    xml.split("<w:tbl>")
        .skip(1)
        .filter_map(|chunk| {
            let end = chunk.find("</w:tbl>")?;
            let block = &chunk[..end + "</w:tbl>".len()];
            block.contains(needle).then_some(block)
        })
        .collect()
}

/// 取 `w:tag w:val="{tag}"` 所在 sdt 的**整块**原文（<w:sdt>…</w:sdt>）；找不到返回 None
fn sdt_block<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let needle = format!("w:val=\"{tag}\"");
    let at = xml.find(&needle)?;
    let start = xml[..at].rfind("<w:sdt>")?;
    // 从 start 起做 <w:sdt> / </w:sdt> 深度计数找配对结尾
    let mut depth = 0usize;
    let mut i = start;
    while i < xml.len() {
        if xml[i..].starts_with("<w:sdt>") { depth += 1; i += 7; }
        else if xml[i..].starts_with("</w:sdt>") {
            depth -= 1; i += 8;
            if depth == 0 { return Some(&xml[start..i]); }
        } else { i += 1; }
    }
    None
}

/// 单张业务表首列底纹逐行判定：<w:tr> 数 R，E6F2F9 出现 R-1 次，且第 2..R 行首个 <w:tc> 内含该 shd
fn first_col_fill_ok(table: &str) -> bool {
    let rows: Vec<&str> = table.split("<w:tr>").skip(1).collect(); // 每段以该行内容开头
    let r = rows.len();
    if r == 0 { return false; }
    if table.matches("w:fill=\"E6F2F9\"").count() != r - 1 { return false; }
    rows.iter().skip(1).all(|row| {
        match row.find("<w:tc>") {
            Some(pos) => {
                let tail = &row[pos..];
                let cell_end = tail.find("</w:tc>").unwrap_or(tail.len());
                tail[..cell_end].contains("w:fill=\"E6F2F9\"")
            }
            None => false,
        }
    })
}

/// 含 `needle` 的 `<w:tbl>` 块个数（#18 doc.tables 计数口径：3up 图桌不计入）
fn count_tables_with(xml: &str, needle: &str) -> usize {
    tables_with(xml, needle).len()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use super::super::fill::export;
    use super::super::model::{BidExportRequest, Block, Chapter};
    use crate::office::bid::docx::DocxPackage;

    const TEMPLATE: &str = "resources/templates/bid-engineering-blue-v1.docx";

    /// §6.5 表格顺序（styles.present 打头，doc.captions 收尾；P0b §4.3 在 page.margins 后插 4 条；B6 在 doc.header_footer 后插 doc.styles_link → 21 项）
    const SPEC_ORDER: [&str; 21] = [
        "styles.present",
        "styles.body",
        "styles.title1",
        "styles.title2",
        "styles.table_header",
        "styles.caption",
        "styles.table_borders",
        "page.size",
        "page.margins",
        "doc.toc",
        "doc.header_footer",
        "doc.styles_link",
        "page_number_field",
        "table.first_col_fill",
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

    /// 对模板 docx 跑 verify(None)：21 条 check，名字顺序与 §6.5 表格一致
    #[test]
    fn report_has_16_checks_in_spec_order() {
        let report = verify(Path::new(TEMPLATE), None).unwrap();
        assert_eq!(report.checks.len(), 21);
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
                captions: 9,
                tables: 9,
                paragraphs: 9,
            }),
        )
        .unwrap();
        assert!(!check(&report, "doc.chapters").ok, "{:?}", check(&report, "doc.chapters"));
        assert!(!check(&report, "doc.figures").ok, "{:?}", check(&report, "doc.figures"));
    }

    /// 反向篡改测试用最小请求：1 章 1 段 1 表（3 列表头 + 2 行数据）、无图
    fn minimal_export_req(out: &Path) -> BidExportRequest {
        BidExportRequest {
            title: "t".to_string(),
            chapters: vec![Chapter {
                title: "c".to_string(),
                blocks: vec![
                    Block::Paragraph { text: "正文段".to_string() },
                    Block::Table {
                        header: vec!["列一".into(), "列二".into(), "列三".into()],
                        rows: vec![
                            vec!["a1".into(), "a2".into(), "a3".into()],
                            vec!["b1".into(), "b2".into(), "b3".into()],
                        ],
                    },
                ],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        }
    }

    /// 反向篡改：导出件 document.xml 首个 `w:fill="E6F2F9"` 改成 FFFFFF → table.first_col_fill 必须 ok=false
    #[test]
    fn tampered_shading_fails() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-verify-fill-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("shading.docx");
        export(&minimal_export_req(&out), Path::new(TEMPLATE), &out).unwrap();

        let mut pkg = DocxPackage::open(&out).unwrap();
        let document = pkg.text("word/document.xml").unwrap();
        assert!(document.contains(r#"w:fill="E6F2F9""#));
        pkg.set_text(
            "word/document.xml",
            document.replacen(r#"w:fill="E6F2F9""#, r#"w:fill="FFFFFF""#, 1),
        );
        let tampered = dir.join("tampered.docx");
        pkg.save(&tampered).unwrap();

        let report = verify(&tampered, None).unwrap();
        assert!(
            !check(&report, "table.first_col_fill").ok,
            "{:?}",
            check(&report, "table.first_col_fill")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 反向篡改：导出件 document.xml `w:val="BID_TOC"` 改成 BID_TOC_REMOVED → doc.toc 必须 ok=false
    #[test]
    fn missing_toc_block_fails() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-verify-toc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("toc.docx");
        export(&minimal_export_req(&out), Path::new(TEMPLATE), &out).unwrap();

        let mut pkg = DocxPackage::open(&out).unwrap();
        let document = pkg.text("word/document.xml").unwrap();
        assert!(document.contains(r#"w:val="BID_TOC""#));
        pkg.set_text(
            "word/document.xml",
            document.replace(r#"w:val="BID_TOC""#, r#"w:val="BID_TOC_REMOVED""#),
        );
        let tampered = dir.join("tampered.docx");
        pkg.save(&tampered).unwrap();

        let report = verify(&tampered, None).unwrap();
        assert!(!check(&report, "doc.toc").ok, "{:?}", check(&report, "doc.toc"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
