//! T1b：标书导出 P0a 模板生成器（规格 §5）。
//!
//! 职责：程序化生成最小合法 docx 模板包（§5.1 条目 + §5.3 document.xml + §5.4 styles.xml），
//! 并导出 §5.2 的 9 个锚点常量供后续填充/校验片使用。
//! 片段库（§5.5）落在 `resources/templates/fragments/`，由填充片 `include_str!` 消费，本片只负责落盘。
//!
//! 对外仅暴露：`generate_to` / `template_document_xml` / `template_styles_xml` / `TAG_*` / `ALL_TAGS`。

use std::collections::BTreeMap;
use std::path::Path;

use super::docx::DocxPackage;
use super::{BidError, BidResult};

// ---------------------------------------------------------------------------
// §5.2 锚点清单（9 个 w:sdt，w:tag w:val 精确匹配）
// ---------------------------------------------------------------------------

/// 锚点：文档标题
pub const TAG_DOC_TITLE: &str = "BID_DOC_TITLE";
/// 锚点：章循环区（外层，内含 BID_CHAPTER）
pub const TAG_CHAPTERS: &str = "BID_CHAPTERS";
/// 锚点：单章（BID_CHAPTERS 内，内含标题 + 正文）
pub const TAG_CHAPTER: &str = "BID_CHAPTER";
/// 锚点：章标题（BID_CHAPTER 内）
pub const TAG_CHAPTER_TITLE: &str = "BID_CHAPTER_TITLE";
/// 锚点：章正文（BID_CHAPTER 内）
pub const TAG_CHAPTER_BODY: &str = "BID_CHAPTER_BODY";
/// 锚点：表格行部件库（外层，内含 BID_TABLE_ROW）
pub const TAG_TABLE_ROWS: &str = "BID_TABLE_ROWS";
/// 锚点：单表格行（BID_TABLE_ROWS 内）
pub const TAG_TABLE_ROW: &str = "BID_TABLE_ROW";
/// 锚点：单图部件库（外层，内含 BID_FIGURE_CAPTION）
pub const TAG_FIGURE_SINGLE: &str = "BID_FIGURE_SINGLE";
/// 锚点：图注（BID_FIGURE_SINGLE 内）
pub const TAG_FIGURE_CAPTION: &str = "BID_FIGURE_CAPTION";

/// 全部 9 个锚点，顺序即 §5.2 清单顺序
pub const ALL_TAGS: [&str; 9] = [
    TAG_DOC_TITLE,
    TAG_CHAPTERS,
    TAG_CHAPTER,
    TAG_CHAPTER_TITLE,
    TAG_CHAPTER_BODY,
    TAG_TABLE_ROWS,
    TAG_TABLE_ROW,
    TAG_FIGURE_SINGLE,
    TAG_FIGURE_CAPTION,
];

// ---------------------------------------------------------------------------
// §5.3 / §5.4 全文（唯一真值）
// ---------------------------------------------------------------------------

/// §5.3 `word/document.xml` 全文（含 XML 声明）
pub fn template_document_xml() -> String {
    DOCUMENT_XML.to_string()
}

/// §5.4 `word/styles.xml` 全文（含 XML 声明）
pub fn template_styles_xml() -> String {
    STYLES_XML.to_string()
}

/// 按 §5.1 组装包内全部条目并写出 docx，返回写入字节数
pub fn generate_to(out: &Path) -> BidResult<u64> {
    let mut entries = BTreeMap::new();
    entries.insert(
        "[Content_Types].xml".to_string(),
        CONTENT_TYPES_XML.as_bytes().to_vec(),
    );
    entries.insert("_rels/.rels".to_string(), ROOT_RELS_XML.as_bytes().to_vec());
    entries.insert(
        "word/document.xml".to_string(),
        template_document_xml().into_bytes(),
    );
    entries.insert(
        "word/styles.xml".to_string(),
        template_styles_xml().into_bytes(),
    );
    entries.insert(
        "word/settings.xml".to_string(),
        SETTINGS_XML.as_bytes().to_vec(),
    );
    entries.insert(
        "word/_rels/document.xml.rels".to_string(),
        DOCUMENT_RELS_XML.as_bytes().to_vec(),
    );
    entries.insert("docProps/core.xml".to_string(), CORE_XML.as_bytes().to_vec());

    DocxPackage { entries }.save(out)?;

    let bytes = std::fs::metadata(out)
        .map_err(|e| BidError::new(format!("读取生成结果 {out:?} 失败: {e}")))?
        .len();
    Ok(bytes)
}

const DOCUMENT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">
  <w:body>
    <!-- ① 文档标题 -->
    <w:sdt><w:sdtPr><w:tag w:val="BID_DOC_TITLE"/></w:sdtPr><w:sdtContent>
      <w:p><w:pPr><w:pStyle w:val="BidTitle1"/></w:pPr><w:r><w:t>文档标题</w:t></w:r></w:p>
    </w:sdtContent></w:sdt>

    <!-- ② 章循环区 -->
    <w:sdt><w:sdtPr><w:tag w:val="BID_CHAPTERS"/></w:sdtPr><w:sdtContent>
      <w:sdt><w:sdtPr><w:tag w:val="BID_CHAPTER"/></w:sdtPr><w:sdtContent>
        <w:sdt><w:sdtPr><w:tag w:val="BID_CHAPTER_TITLE"/></w:sdtPr><w:sdtContent>
          <w:p><w:pPr><w:pStyle w:val="BidTitle1"/><w:pageBreakBefore/></w:pPr><w:r><w:t>章标题</w:t></w:r></w:p>
        </w:sdtContent></w:sdt>
        <w:sdt><w:sdtPr><w:tag w:val="BID_CHAPTER_BODY"/></w:sdtPr><w:sdtContent>
          <w:p><w:pPr><w:pStyle w:val="BidBody"/></w:pPr><w:r><w:t>正文占位</w:t></w:r></w:p>
        </w:sdtContent></w:sdt>
      </w:sdtContent></w:sdt>
    </w:sdtContent></w:sdt>

    <!-- ③ 部件库：表格行模板（block 级 sdt 包一张 tbl；导出时整体移除） -->
    <w:sdt><w:sdtPr><w:tag w:val="BID_TABLE_ROWS"/></w:sdtPr><w:sdtContent>
      <w:tbl>
        <w:tblPr><w:tblStyle w:val="BidTable"/><w:tblW w:w="5000" w:type="pct"/>
          <w:jc w:val="center"/><w:tblLayout w:type="fixed"/></w:tblPr>
        <w:tblGrid><w:gridCol w:w="3516"/><w:gridCol w:w="3515"/><w:gridCol w:w="3515"/></w:tblGrid>
        <w:sdt><w:sdtPr><w:tag w:val="BID_TABLE_ROW"/></w:sdtPr><w:sdtContent>
          <w:tr><w:trPr><w:cantSplit/></w:trPr>
            <w:tc><w:tcPr><w:tcW w:w="3516" w:type="dxa"/></w:tcPr>
              <w:p><w:pPr><w:pStyle w:val="BidTableBody"/></w:pPr><w:r><w:t>单元格</w:t></w:r></w:p></w:tc>
            <w:tc><w:tcPr><w:tcW w:w="3515" w:type="dxa"/></w:tcPr>
              <w:p><w:pPr><w:pStyle w:val="BidTableBody"/></w:pPr><w:r><w:t>单元格</w:t></w:r></w:p></w:tc>
            <w:tc><w:tcPr><w:tcW w:w="3515" w:type="dxa"/></w:tcPr>
              <w:p><w:pPr><w:pStyle w:val="BidTableBody"/></w:pPr><w:r><w:t>单元格</w:t></w:r></w:p></w:tc>
          </w:tr>
        </w:sdtContent></w:sdt>
      </w:tbl>
    </w:sdtContent></w:sdt>

    <!-- ④ 部件库：单图模板（导出时整体移除） -->
    <w:sdt><w:sdtPr><w:tag w:val="BID_FIGURE_SINGLE"/></w:sdtPr><w:sdtContent>
      <w:p><w:pPr><w:pStyle w:val="BidFigureCell"/><w:spacing w:line="240" w:lineRule="auto"/><w:jc w:val="center"/></w:pPr><w:r><w:t>图片占位</w:t></w:r></w:p>
      <w:sdt><w:sdtPr><w:tag w:val="BID_FIGURE_CAPTION"/></w:sdtPr><w:sdtContent>
        <w:p><w:pPr><w:pStyle w:val="BidCaption"/></w:pPr><w:r><w:t>图注</w:t></w:r></w:p>
      </w:sdtContent></w:sdt>
    </w:sdtContent></w:sdt>

    <!-- ⑤ 页面设置 -->
    <w:sectPr>
      <w:pgSz w:w="11906" w:h="16838"/>
      <w:pgMar w:top="680" w:right="680" w:bottom="680" w:left="680" w:header="851" w:footer="992" w:gutter="0"/>
      <w:cols w:space="425"/>
      <w:docGrid w:linePitch="312"/>
    </w:sectPr>
  </w:body>
</w:document>
"#;

const STYLES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="宋体"/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr></w:rPrDefault>
    <w:pPrDefault><w:pPr><w:spacing w:line="360" w:lineRule="auto"/></w:pPr></w:pPrDefault>
  </w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal">
    <w:name w:val="Normal"/>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidBody">
    <w:name w:val="BidBody"/>
    <w:pPr><w:spacing w:line="360" w:lineRule="auto"/><w:ind w:firstLineChars="200"/><w:jc w:val="both"/></w:pPr>
    <w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="宋体"/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidTitle1">
    <w:name w:val="BidTitle1"/>
    <w:pPr><w:jc w:val="center"/></w:pPr>
    <w:rPr><w:color w:val="253D57"/><w:sz w:val="30"/><w:b/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidTitle2">
    <w:name w:val="BidTitle2"/>
    <w:pPr><w:jc w:val="center"/></w:pPr>
    <w:rPr><w:color w:val="315276"/><w:sz w:val="28"/><w:b/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidTitle3">
    <w:name w:val="BidTitle3"/>
    <w:pPr><w:jc w:val="left"/></w:pPr>
    <w:rPr><w:color w:val="000000"/><w:sz w:val="28"/><w:b/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidTitle4">
    <w:name w:val="BidTitle4"/>
    <w:pPr><w:jc w:val="left"/></w:pPr>
    <w:rPr><w:color w:val="000000"/><w:sz w:val="24"/><w:b/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidTableHeader">
    <w:name w:val="BidTableHeader"/>
    <w:pPr><w:jc w:val="center"/><w:shd w:val="clear" w:color="auto" w:fill="4877AC"/></w:pPr>
    <w:rPr><w:color w:val="FFFFFF"/><w:sz w:val="24"/><w:b/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidTableBody">
    <w:name w:val="BidTableBody"/>
    <w:pPr><w:jc w:val="center"/></w:pPr>
    <w:rPr><w:sz w:val="24"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidCaption">
    <w:name w:val="BidCaption"/>
    <w:pPr><w:jc w:val="center"/></w:pPr>
    <w:rPr><w:sz w:val="24"/><w:b/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidFigureCell">
    <w:name w:val="BidFigureCell"/>
    <w:pPr><w:spacing w:line="240" w:lineRule="auto"/><w:jc w:val="center"/></w:pPr>
    <w:rPr><w:sz w:val="24"/></w:rPr>
  </w:style>
  <w:style w:type="table" w:styleId="BidTable">
    <w:name w:val="BidTable"/>
    <w:tblPr><w:tblBorders>
      <w:top w:val="single" w:sz="4" w:space="0" w:color="74A6CC"/>
      <w:left w:val="single" w:sz="4" w:space="0" w:color="74A6CC"/>
      <w:bottom w:val="single" w:sz="4" w:space="0" w:color="74A6CC"/>
      <w:right w:val="single" w:sz="4" w:space="0" w:color="74A6CC"/>
      <w:insideH w:val="single" w:sz="4" w:space="0" w:color="B0D0E8"/>
      <w:insideV w:val="single" w:sz="4" w:space="0" w:color="B0D0E8"/>
    </w:tblBorders>
    <w:tblCellMar>
      <w:top w:w="0" w:type="dxa"/>
      <w:left w:w="108" w:type="dxa"/>
      <w:bottom w:w="0" w:type="dxa"/>
      <w:right w:w="108" w:type="dxa"/>
    </w:tblCellMar>
    </w:tblPr>
  </w:style>
</w:styles>
"#;

// ---------------------------------------------------------------------------
// §5.1 其余包内条目
// ---------------------------------------------------------------------------

/// `[Content_Types].xml`：Default 全部排在 Override 之前
const CONTENT_TYPES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
  <Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>
  <Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
</Types>
"#;

/// `_rels/.rels`：rId1 → document（officeDocument），rId2 → core（core-properties）
const ROOT_RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
</Relationships>
"#;

/// `word/settings.xml`
const SETTINGS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:updateFields w:val="true"/></w:settings>
"#;

/// `word/_rels/document.xml.rels`：**必须显式写闭合标签**（不可自闭合），由 add_png 追加
const DOCUMENT_RELS_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>\n";

/// `docProps/core.xml`（命名空间齐全即可）
const CORE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <dc:title>Bid Template v1</dc:title>
  <dc:creator>qidiwork</dc:creator>
</cp:coreProperties>
"#;

// ---------------------------------------------------------------------------
// 单测
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    /// 九个锚点逐个出现在 document.xml 的 w:val="…" 中
    #[test]
    fn all_nine_tags_present_in_document_xml() {
        let doc = template_document_xml();
        for tag in ALL_TAGS {
            assert!(
                doc.contains(&format!("w:val=\"{tag}\"")),
                "document.xml 缺少锚点 {tag}"
            );
        }
    }

    /// 十个 w:styleId="Bid…" 齐全（9 段落 + 1 表格）
    #[test]
    fn ten_bid_styles_present_in_styles_xml() {
        let styles = template_styles_xml();
        let expected = [
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
        for id in expected {
            assert!(
                styles.contains(&format!("w:styleId=\"{id}\"")),
                "styles.xml 缺少样式 {id}"
            );
        }
        assert_eq!(
            styles.matches("w:styleId=\"Bid").count(),
            10,
            "Bid 样式必须恰好 10 个"
        );
    }

    /// w:pgMar 含 w:top="680"
    #[test]
    fn pgmar_top_is_680() {
        let doc = template_document_xml();
        let start = doc.find("<w:pgMar").expect("document.xml 缺少 w:pgMar");
        let end = doc[start..]
            .find("/>")
            .expect("w:pgMar 未闭合")
            + start;
        let pgmar = &doc[start..end];
        assert!(
            pgmar.contains("w:top=\"680\""),
            "w:pgMar 必须含 w:top=\"680\": {pgmar}"
        );
    }

    /// w:sectPr 含 w:w="11906"（A4 宽度）
    #[test]
    fn sectpr_contains_a4_width() {
        let doc = template_document_xml();
        let start = doc
            .find("<w:sectPr")
            .expect("document.xml 缺少 w:sectPr");
        let end = doc[start..]
            .find("</w:sectPr>")
            .expect("w:sectPr 未闭合")
            + start;
        let sectpr = &doc[start..end];
        assert!(
            sectpr.contains("w:w=\"11906\""),
            "w:sectPr 必须含 w:w=\"11906\": {sectpr}"
        );
    }

    /// generate_to 写出 §5.1 全部条目，内容与生成器字符串一致，约束格式满足铁律
    #[test]
    fn generate_to_writes_all_required_entries() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-t1b-gen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("bid-engineering-blue-v1.docx");

        let bytes = generate_to(&out).unwrap();
        assert!(bytes > 3 * 1024, "模板应 > 3KB，实际 {bytes}");

        let pkg = DocxPackage::open(&out).unwrap();
        for name in [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/styles.xml",
            "word/settings.xml",
            "word/_rels/document.xml.rels",
            "docProps/core.xml",
        ] {
            assert!(pkg.contains(name), "包内缺少条目 {name}");
        }
        assert_eq!(pkg.text("word/document.xml").unwrap(), template_document_xml());
        assert_eq!(pkg.text("word/styles.xml").unwrap(), template_styles_xml());

        // document.xml.rels 必须显式闭合，不能自闭合
        let rels = pkg.text("word/_rels/document.xml.rels").unwrap();
        assert!(rels.contains("</Relationships>"), "document.xml.rels 必须显式闭合");
        assert!(
            !rels.trim_end().ends_with("/>"),
            "document.xml.rels 不得自闭合"
        );

        // [Content_Types].xml 的 Default 必须全部排在 Override 之前
        let ct = pkg.text("[Content_Types].xml").unwrap();
        let first_override = ct.find("<Override").unwrap();
        for (pos, _) in ct.match_indices("<Default") {
            assert!(pos < first_override, "Default 必须全部排在 Override 之前");
        }

        // settings.xml / core.xml 内容抽查
        assert!(pkg.text("word/settings.xml").unwrap().contains("<w:updateFields w:val=\"true\"/>"));
        let core = pkg.text("docProps/core.xml").unwrap();
        assert!(core.contains("<dc:title>Bid Template v1</dc:title>"));
        assert!(core.contains("<dc:creator>qidiwork</dc:creator>"));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
