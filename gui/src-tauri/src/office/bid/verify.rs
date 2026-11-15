//! T2b：产物分层校验器（规格 §6.5 / P0b §4.3）。
//!
//! 职责：对填充/导出后的 docx 做 23 项分层校验（样式 #1-#8 / 页面 #9-#10 / 文档结构 #11-#17 / 计数 #18-#23），
//! 顺序即规格表格顺序，输出 `VerifyReport`（`Serialize`，供 T3 命令返回）。
//!
//! 实现约定（规格 §6.5）：
//! - styles.xml 按 `w:styleId="X"` 定位样式块：向前找最近 `<w:style`，向后找 `</w:style>`；
//!   找不到该样式 → 该 check `ok=false`、detail 写 `缺样式 X`，不 panic、不 Err。
//! - 计数用 `xml.matches(needle).count()`。
//! - 比较前先 `replace(" />", "/>")` 归一。
//! - `expect=None` → #18-#23 计数类 check `ok=true` + detail `skipped`。
//! - 元素顺序（#8 styles.element_order / #15 doc.element_order，G2 护栏）：容器第一层子元素名
//!   只保留在顺序表内的（表外未知元素跳过），下标须单调不减；自闭合标签不增深度。

use serde::Serialize;
use std::path::Path;

use super::docx::DocxPackage;
use super::BidResult;

/// 期望计数（`expect=None` → 只查结构 #1-#17，#18-#23 skipped）
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

/// OOXML 容器子元素顺序表（G2 护栏；只列本项目会用到的；表外元素视为"未知"，跳过不判）
const PPR: &[&str] = &[
    "pStyle", "keepNext", "keepLines", "pageBreakBefore", "pBdr", "shd", "tabs", "spacing", "ind",
    "contextualSpacing", "jc", "outlineLvl", "rPr", "sectPr",
];
const RPR: &[&str] = &[
    "rStyle", "rFonts", "b", "bCs", "i", "iCs", "caps", "smallCaps", "strike", "dstrike", "color",
    "spacing", "w", "kern", "position", "sz", "szCs", "highlight", "u",
];
const TBLPR: &[&str] = &[
    "tblStyle", "tblW", "jc", "tblCellSpacing", "tblInd", "tblBorders", "shd", "tblLayout",
    "tblCellMar", "tblLook",
];
const TRPR: &[&str] = &["cantSplit", "trHeight", "tblHeader"];
const TCPR: &[&str] = &["tcW", "gridSpan", "vMerge", "tcBorders", "shd", "vAlign"];
const SECTPR: &[&str] = &["headerReference", "footerReference", "pgSz", "pgMar", "cols", "docGrid"];

/// 对 `path` 处 docx 做 23 项分层校验；`expect=None` 时跳过计数类 check（#18-#23）
pub fn verify(path: &Path, expect: Option<&Expect>) -> BidResult<VerifyReport> {
    let pkg = DocxPackage::open(path)?;
    let styles = pkg.text("word/styles.xml")?;
    let document = pkg.text("word/document.xml")?;

    // 归一：属性尾部的 " />" 并成 "/>"，使属性写法断言与生成器输出对齐
    let styles = styles.replace(" />", "/>");
    let document = document.replace(" />", "/>");

    let mut checks = Vec::with_capacity(23);

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

    // #8 styles.element_order（G2 护栏）：styles.xml 全部 pPr/rPr 的第一层子元素顺序须单调
    //     符合 PPR/RPR 顺序表；表外元素跳过不判；首个违规 detail 写该容器元素序列（截断 120 字符）
    checks.push(element_order_check(
        "styles.element_order",
        &[
            (&styles, "pPr", PPR),
            (&styles, "rPr", RPR),
        ],
    ));

    // #9 page.size
    checks.push(contains_check(
        "page.size",
        &document,
        &[r#"w:w="11906""#, r#"w:h="16838""#],
    ));

    // #10 page.margins
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

    // #11-#16 文档结构 check（P0b §4.3 新增 4 条，插在 page.margins 之后；B6 插 doc.styles_link）
    // #11 doc.toc：目录域仍在（填充器不动 BID_TOC）。限定 w:tag w:val="BID_TOC" 的 sdt 块内断言；找不到块 → ok=false
    //     （事件流索引，Word 写成 <w:sdt w:rsidR="..."> 带属性时也能命中；手写深度计数已废弃）
    let ranges = crate::office::bid::template::index_sdt(&document)?;
    let toc_ok = match crate::office::bid::template::find(&ranges, "BID_TOC") {
        None => false,
        Some(r) => {
            let block = &document[r.block_start..r.block_end];
            block.contains("fldSimple") && block.contains("TOC")
        }
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
    // #12 doc.header_footer：页眉页脚部件 + content-types + document 引用齐全；
    //     且页眉部件不得残留 {{DOC_TITLE}} 占位符（占位符只存在于 word/header1.xml，#17 查 document.xml 覆盖不到）
    let content_types = pkg.text("[Content_Types].xml")?;
    let header = if pkg.contains("word/header1.xml") {
        pkg.text("word/header1.xml")?
    } else {
        String::new()
    };
    let hf_ok = pkg.contains("word/header1.xml")
        && pkg.contains("word/footer1.xml")
        && content_types.contains("header+xml")
        && content_types.contains("footer+xml")
        && document.contains("headerReference")
        && document.contains("footerReference")
        && !header.contains("{{DOC_TITLE}}");
    checks.push(Check {
        name: "doc.header_footer".to_string(),
        ok: hf_ok,
        detail: if hf_ok {
            "页眉页脚部件齐全且页眉无 {{DOC_TITLE}} 占位残留".to_string()
        } else {
            "缺页眉/页脚部件、content-types 或 document 引用，或页眉残留 {{DOC_TITLE}} 占位符".to_string()
        },
    });
    // #13 doc.styles_link：document.xml.rels 须声明 styles.xml / settings.xml 关系
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
    // #14 page_number_field：页脚含 PAGE / NUMPAGES 域
    let footer = pkg.text("word/footer1.xml")?;
    checks.push(contains_check(
        "page_number_field",
        &footer,
        &[r#"instr=" PAGE ""#, r#"instr=" NUMPAGES ""#],
    ));
    // #15 doc.element_order（G2 护栏）：document.xml 全 6 容器 + header/footer 的 pPr/rPr 子元素顺序
    //     （" />" 未归一部件就地归一后再判，自闭合标签不增深度）
    let header_norm = header.replace(" />", "/>");
    let footer_norm = footer.replace(" />", "/>");
    checks.push(element_order_check(
        "doc.element_order",
        &[
            (&document, "pPr", PPR),
            (&document, "rPr", RPR),
            (&document, "tblPr", TBLPR),
            (&document, "trPr", TRPR),
            (&document, "tcPr", TCPR),
            (&document, "sectPr", SECTPR),
            (&header_norm, "pPr", PPR),
            (&header_norm, "rPr", RPR),
            (&footer_norm, "pPr", PPR),
            (&footer_norm, "rPr", RPR),
        ],
    ));
    // #16 table.first_col_fill：含 BidTableHeader 的业务表逐行断言首列底纹；无业务表 → skipped
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

    // #17 doc.no_template_parts：不得含模板部件残留
    //     （BID_DOC_TITLE/BID_CHAPTER* 的外层 sdt 包装在成品里是正常的，不断言其不存在；
    //      {{DOC_TITLE}} 只存在于 word/header1.xml，已在 #12 doc.header_footer 断言，此处查 document.xml 覆盖不到）
    let forbidden = [
        "BID_TABLE_ROWS",
        "BID_FIGURE_SINGLE",
        "BID_FIGURE_3UP",
        "BID_FIGURE_WIDE",
        "BID_TABLE_ROW",
        "BID_FIGURE_CAPTION",
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

    // #18-#23 计数类 check：expect=None → ok=true + detail "skipped"
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

/// 容器子元素顺序断言（G2 护栏）：`xml` 中每个 `<w:{container}>…</w:{container}>` 块
/// （容器不会同名嵌套，简单 find 即可）的第一层子元素名，只保留在 `order` 表内的
/// （表外未知元素跳过），下标须单调不减；违规返回 `"pPr: jc 出现在 shd 之前"` 这类信息
/// （附该容器元素序列，供 check detail 截断 120 字符使用）。
fn order_ok(xml: &str, container: &str, order: &[&str]) -> Result<(), String> {
    let open_tag = format!("<w:{container}");
    let close_tag = format!("</w:{container}>");
    let mut from = 0;
    while let Some(rel) = xml[from..].find(&open_tag) {
        let start = from + rel;
        let close_rel = xml[start..]
            .find(&close_tag)
            .ok_or_else(|| format!("{container}: 未找到闭合标签"))?;
        let block = &xml[start..start + close_rel + close_tag.len()];
        let names = first_level_names(block);
        let mut prev: Option<usize> = None;
        for name in names.iter().copied() {
            if let Some(idx) = order.iter().position(|x| *x == name) {
                if let Some(p) = prev {
                    if idx < p {
                        let seq = names.join(", ");
                        let seq: &str = if seq.len() > 120 { &seq[..120] } else { seq.as_str() };
                        let prev_name = order[p];
                        return Err(format!(
                            "{container}: {name} 出现在 {prev_name} 之前；序列: {seq}"
                        ));
                    }
                }
                prev = Some(idx);
            }
        }
        from = start + close_rel + close_tag.len();
    }
    Ok(())
}

/// 解析容器块的第一层子元素名：容器自身开始标签跳过；自闭合标签不增深度（深度 0 时记录）；
/// 其余开标签深度 +1（深度为 1 时记录 name）；遇 `</w:Name>` 深度 -1。
fn first_level_names(block: &str) -> Vec<&str> {
    let inner = match block.find('>') {
        Some(e) => &block[e + 1..],
        None => return Vec::new(),
    };
    let mut names = Vec::new();
    let mut depth = 0usize;
    let mut rest = inner;
    while let Some(lt) = rest.find('<') {
        let tail = &rest[lt..];
        let Some(gt) = tail.find('>') else { break };
        let tag = &tail[..=gt];
        if let Some(name) = tag_name(tag) {
            if tag.starts_with("</") {
                depth = depth.saturating_sub(1);
            } else if tag.ends_with("/>") {
                if depth == 0 {
                    names.push(name);
                }
            } else {
                depth += 1;
                if depth == 1 {
                    names.push(name);
                }
            }
        }
        rest = &tail[gt + 1..];
    }
    names
}

/// 标签名：`<w:pPr>` / `<w:b/>` / `</w:jc>` → `pPr` / `b` / `jc`；非 `w:` 前缀返回 None
fn tag_name(tag: &str) -> Option<&str> {
    let mut inner = tag.strip_prefix('<')?;
    inner = inner.strip_prefix('/').unwrap_or(inner);
    inner = inner.strip_prefix("w:")?;
    let end = inner.find(|c: char| !c.is_ascii_alphanumeric())?;
    Some(&inner[..end])
}

/// 元素顺序 check（styles.element_order / doc.element_order）：对 `(xml, 容器, 顺序表)` 逐对跑
/// `order_ok`；任一违规 → ok=false，detail 写首个违规信息（含元素序列，截断 120 字符内）；
/// 全过 → ok=true，detail 写 `共检查 N 个容器`。
fn element_order_check(name: &str, runs: &[(&str, &str, &[&str])]) -> Check {
    let mut containers = 0usize;
    for (xml, container, order) in runs {
        containers += xml.matches(&format!("<w:{container}")).count();
        if let Err(msg) = order_ok(xml, container, order) {
            let detail: String = msg.chars().take(120).collect();
            return Check {
                name: name.to_string(),
                ok: false,
                detail,
            };
        }
    }
    Check {
        name: name.to_string(),
        ok: true,
        detail: format!("共检查 {containers} 个容器"),
    }
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

/// 包含断言（#9-#10）：xml 须含全部 needle
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

/// 计数断言（#18-#23）：got == want_of(expect)
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

/// 含 `needle` 的 `<w:tbl>` 块个数（#21 doc.tables 计数口径：3up 图桌不计入）
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

    /// §6.5 表格顺序（styles.present 打头，doc.captions 收尾；P0b §4.3 在 page.margins 后插 4 条；B6 在 doc.header_footer 后插 doc.styles_link；G2 在 styles.table_borders 后插 styles.element_order、在 page_number_field 后插 doc.element_order → 23 项）
    const SPEC_ORDER: [&str; 23] = [
        "styles.present",
        "styles.body",
        "styles.title1",
        "styles.title2",
        "styles.table_header",
        "styles.caption",
        "styles.table_borders",
        "styles.element_order",
        "page.size",
        "page.margins",
        "doc.toc",
        "doc.header_footer",
        "doc.styles_link",
        "page_number_field",
        "doc.element_order",
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

    /// 对模板 docx 跑 verify(None)：23 条 check，名字顺序与 §6.5 表格一致
    #[test]
    fn report_has_23_checks_in_spec_order() {
        let report = verify(Path::new(TEMPLATE), None).unwrap();
        assert_eq!(report.checks.len(), 23);
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

    /// 反向篡改：导出件 word/header1.xml 标题文本前插回 {{DOC_TITLE}} → doc.header_footer 必须 ok=false
    /// （占位符只存在于页眉部件，#17 的 document.xml 断言覆盖不到；本测试即 #12 页眉占位检查的反向用例）
    #[test]
    fn header_placeholder_left_fails() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-verify-hdr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("hdr.docx");
        export(&minimal_export_req(&out), Path::new(TEMPLATE), &out).unwrap();

        let mut pkg = DocxPackage::open(&out).unwrap();
        let header = pkg.text("word/header1.xml").unwrap();
        assert!(!header.contains("{{DOC_TITLE}}"), "导出器未替换页眉占位符: {header}");
        pkg.set_text(
            "word/header1.xml",
            header.replacen(
                "<w:t xml:space=\"preserve\">",
                "<w:t xml:space=\"preserve\">{{DOC_TITLE}}",
                1,
            ),
        );
        let tampered = dir.join("tampered.docx");
        pkg.save(&tampered).unwrap();

        let report = verify(&tampered, None).unwrap();
        assert!(
            !check(&report, "doc.header_footer").ok,
            "{:?}",
            check(&report, "doc.header_footer")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 反向篡改：导出最小 docx → styles.xml BidTitle1 块把 `<w:b/>` 从 `<w:color` 前移到
    /// `</w:rPr>` 前（制造 color→b，违反 RPR 顺序表）→ styles.element_order 必须 ok=false
    #[test]
    fn styles_order_tamper_fails() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-verify-order-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("order.docx");
        export(&minimal_export_req(&out), Path::new(TEMPLATE), &out).unwrap();

        let mut pkg = DocxPackage::open(&out).unwrap();
        let styles = pkg.text("word/styles.xml").unwrap();
        let block = style_block(&styles, "BidTitle1").unwrap();
        assert!(
            block.contains("<w:b/><w:color"),
            "BidTitle1 块结构变化，无 <w:b/><w:color 可篡改: {block}"
        );
        let tampered_block = block
            .replace("<w:b/><w:color", "<w:color")
            .replacen("</w:rPr>", "<w:b/></w:rPr>", 1);
        pkg.set_text("word/styles.xml", styles.replacen(block, &tampered_block, 1));
        let tampered = dir.join("tampered.docx");
        pkg.save(&tampered).unwrap();

        let report = verify(&tampered, None).unwrap();
        assert!(
            !check(&report, "styles.element_order").ok,
            "{:?}",
            check(&report, "styles.element_order")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 模板与导出件的 styles/document/header/footer 子元素顺序均合规
    /// （styles.element_order 与 doc.element_order 都必须 ok=true）
    #[test]
    fn current_outputs_pass_order() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-verify-order-pass-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("order-pass.docx");
        export(&minimal_export_req(&out), Path::new(TEMPLATE), &out).unwrap();

        for path in [Path::new(TEMPLATE), out.as_path()] {
            let report = verify(path, None).unwrap();
            assert!(
                check(&report, "styles.element_order").ok,
                "{path:?} styles.element_order: {:?}",
                check(&report, "styles.element_order")
            );
            assert!(
                check(&report, "doc.element_order").ok,
                "{path:?} doc.element_order: {:?}",
                check(&report, "doc.element_order")
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
