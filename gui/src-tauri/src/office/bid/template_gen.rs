//! T1b：标书导出模板生成器（P0a §5 + P0b §3 模板 v2）。
//!
//! 职责：程序化生成最小合法 docx 模板包（§5.1 条目 + §3.3 document.xml + §3.4 styles.xml
//! + §3.2 header1/footer1），并导出 §3.6 的 12 个锚点常量（9 旧 + TOC/3UP/WIDE）
//!
//! 供后续填充/校验片使用。
//!
//! 片段库（§3.5）落在 `resources/templates/fragments/`，由填充片 `include_str!` 消费，本片只负责落盘。
//!
//! 对外仅暴露：`generate_to` / `generate_to_with` / `template_document_xml[_for]` /
//! `template_styles_xml[_for]` / `TAG_*` / `ALL_TAGS`（旧签名保留，内部转调 `*_for(&DocStyle)`；
//! `template_document_xml_for` 返回 `BidResult`，纸型不受支持即 Err）。

use std::collections::BTreeMap;
use std::path::Path;

use super::docx::DocxPackage;
use super::model::DocStyle;
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
/// 锚点：目录域（块级 sdt；填充器原样保留，不删除不替换）
pub const TAG_TOC: &str = "BID_TOC";
/// 锚点：3up 图桌部件库（外层；内容仅空段落，B2 起由 fill 从片段生成表格）
pub const TAG_FIGURE_3UP: &str = "BID_FIGURE_3UP";
/// 锚点：wide 图部件库（外层，导入时删除）
pub const TAG_FIGURE_WIDE: &str = "BID_FIGURE_WIDE";

/// 全部 12 个锚点，顺序即 §3.6 清单顺序（9 旧 + TOC + 3UP + WIDE）
pub const ALL_TAGS: [&str; 12] = [
    TAG_DOC_TITLE,
    TAG_CHAPTERS,
    TAG_CHAPTER,
    TAG_CHAPTER_TITLE,
    TAG_CHAPTER_BODY,
    TAG_TABLE_ROWS,
    TAG_TABLE_ROW,
    TAG_FIGURE_SINGLE,
    TAG_FIGURE_CAPTION,
    TAG_TOC,
    TAG_FIGURE_3UP,
    TAG_FIGURE_WIDE,
];

// ---------------------------------------------------------------------------
// §3.3 / §3.4 全文（唯一真值，按 DocStyle 渲染）
// ---------------------------------------------------------------------------

/// §3.3 `word/document.xml` 全文（默认 DocStyle，含 XML 声明）。
/// 默认路径恒为 A4 纵向（11906×16838），直接传尺寸常量，不经过纸型解析，无 Err 路径。
pub fn template_document_xml() -> String {
    document_xml_with(&DocStyle::default(), 11906, 16838)
}

/// §3.3 `word/document.xml` 全文（指定 DocStyle）；纸型不受支持时返回 Err
pub fn template_document_xml_for(style: &DocStyle) -> BidResult<String> {
    let (w, h) = pg_sz_twips(&style.page.paper)?;
    Ok(document_xml_with(style, w, h))
}

/// §3.4 `word/styles.xml` 全文（默认 DocStyle，含 XML 声明）
pub fn template_styles_xml() -> String {
    template_styles_xml_for(&DocStyle::default())
}

/// §3.4 `word/styles.xml` 全文（指定 DocStyle）
pub fn template_styles_xml_for(style: &DocStyle) -> String {
    styles_xml(style)
}

/// 按 §5.1 组装包内全部条目并写出 docx（默认 DocStyle），返回写入字节数
pub fn generate_to(out: &Path) -> BidResult<u64> {
    generate_to_with(out, &DocStyle::default())
}

/// 按 §5.1 + §3.2 组装包内全部条目并写出 docx（指定 DocStyle），返回写入字节数
pub fn generate_to_with(out: &Path, style: &DocStyle) -> BidResult<u64> {
    let mut entries = BTreeMap::new();
    entries.insert(
        "[Content_Types].xml".to_string(),
        CONTENT_TYPES_XML.as_bytes().to_vec(),
    );
    entries.insert("_rels/.rels".to_string(), ROOT_RELS_XML.as_bytes().to_vec());
    entries.insert(
        "word/document.xml".to_string(),
        template_document_xml_for(style)?.into_bytes(),
    );
    entries.insert(
        "word/styles.xml".to_string(),
        template_styles_xml_for(style).into_bytes(),
    );
    entries.insert(
        "word/settings.xml".to_string(),
        SETTINGS_XML.as_bytes().to_vec(),
    );
    entries.insert("word/header1.xml".to_string(), HEADER_XML.as_bytes().to_vec());
    entries.insert("word/footer1.xml".to_string(), FOOTER_XML.as_bytes().to_vec());
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

/// §3.3 `word/document.xml` 全文：12 锚点 + TOC/3up/wide 部件库 + header/footer 引用。
/// 页边距由 `DocStyle::margin_twips()` 渲染；pgSz 由调用方传入的 `(w, h)` twips 渲染；
/// 其余版式常量与 P0a 模板逐字一致。
fn document_xml_with(style: &DocStyle, w: u32, h: u32) -> String {
    let m = style.margin_twips();
    let pg_sz = format!("<w:pgSz w:w=\"{w}\" w:h=\"{h}\"/>");
    format!(
r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">
  <w:body>
    <!-- ① 文档标题 -->
    <w:sdt><w:sdtPr><w:tag w:val="BID_DOC_TITLE"/></w:sdtPr><w:sdtContent>
      <w:p><w:pPr><w:pStyle w:val="BidTitle1"/></w:pPr><w:r><w:t>文档标题</w:t></w:r></w:p>
    </w:sdtContent></w:sdt>

    <!-- ② 目录域（块级 sdt；内容由 Word/WPS 更新域时生成，填充器原样保留） -->
    <w:sdt><w:sdtPr><w:tag w:val="BID_TOC"/></w:sdtPr><w:sdtContent>
      <w:p><w:pPr><w:pStyle w:val="BidTOC"/></w:pPr>
        <w:fldSimple w:instr=" TOC \o &quot;1-3&quot; \h \z \u "/>
      </w:p>
    </w:sdtContent></w:sdt>

    <!-- ③ 章循环区 -->
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

    <!-- ④ 部件库：表格行模板（block 级 sdt 包一张 tbl；导出时整体移除） -->
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

    <!-- ⑤ 部件库：单图模板（导出时整体移除） -->
    <w:sdt><w:sdtPr><w:tag w:val="BID_FIGURE_SINGLE"/></w:sdtPr><w:sdtContent>
      <w:p><w:pPr><w:pStyle w:val="BidFigureCell"/><w:spacing w:line="240" w:lineRule="auto"/><w:jc w:val="center"/></w:pPr><w:r><w:t>图片占位</w:t></w:r></w:p>
      <w:sdt><w:sdtPr><w:tag w:val="BID_FIGURE_CAPTION"/></w:sdtPr><w:sdtContent>
        <w:p><w:pPr><w:pStyle w:val="BidCaption"/></w:pPr><w:r><w:t>图注</w:t></w:r></w:p>
      </w:sdtContent></w:sdt>
    </w:sdtContent></w:sdt>

    <!-- ⑥ 部件库：3up 图桌（导入时删除。只能空段落不能表格——旧 fill 不删它，
         放表格会让 doc.tables 计数 +1；3up 表格由 fill 从片段生成） -->
    <w:sdt><w:sdtPr><w:tag w:val="BID_FIGURE_3UP"/></w:sdtPr><w:sdtContent>
      <w:p><w:pPr><w:pStyle w:val="BidFigureCell"/></w:pPr></w:p>
    </w:sdtContent></w:sdt>

    <!-- ⑦ 部件库：wide 图（导入时删除） -->
    <w:sdt><w:sdtPr><w:tag w:val="BID_FIGURE_WIDE"/></w:sdtPr><w:sdtContent>
      <w:p><w:pPr><w:pStyle w:val="BidFigureCell"/></w:pPr></w:p>
    </w:sdtContent></w:sdt>

    <!-- ⑧ 页面设置：引用必须在 pgSz 之前（EG_HdrFtrReferences 顺序） -->
    <w:sectPr>
      <w:headerReference w:type="default" r:id="rId1"/>
      <w:footerReference w:type="default" r:id="rId2"/>
      {pg_sz}
      <w:pgMar w:top="{m}" w:right="{m}" w:bottom="{m}" w:left="{m}" w:header="851" w:footer="992" w:gutter="0"/>
      <w:cols w:space="425"/>
      <w:docGrid w:linePitch="312"/>
    </w:sectPr>
  </w:body>
</w:document>
"#
    )
}

/// §3.4 `word/styles.xml` 全文：10 个旧样式由 `DocStyle` 计算（默认值与 P0a 模板逐字一致），
/// BidTOC / BidHeader / BidFooter 为 §3.4 新增固定样式（10 → 13）。
fn styles_xml(style: &DocStyle) -> String {
    let font = &style.body.font;
    let body_sz = style.half_points(style.body.size_pt);
    let line = style.line_twips();
    let indent = style.first_line_chars_x100();
    let body_jc = jc_val(&style.body.align);
    let h1_sz = style.half_points(style.heading1.size_pt);
    let h1_bold = bold_tag(style.heading1.bold);
    let h2_sz = style.half_points(style.heading2.size_pt);
    let h2_bold = bold_tag(style.heading2.bold);
    let h3_sz = style.half_points(style.heading3.size_pt);
    let h3_bold = bold_tag(style.heading3.bold);
    let h4_sz = style.half_points(style.heading4.size_pt);
    let h4_bold = bold_tag(style.heading4.bold);
    let cap_sz = style.half_points(style.figure.caption_size_pt);
    let border_sz = style.border_eighth_pt();
    // §7.3：表字号走 `table.size_pt`（f32，可表达 10.5pt → w:sz 21）
    let table_sz = style.table_half_points();
    // §7.3：段后距 → `w:spacing/@w:after`（pt*20）；0pt 时不输出该属性，默认产物字节级不变
    let space_after = if style.body.space_after_pt > 0 {
        format!(" w:after=\"{}\"", style.space_after_twips())
    } else {
        String::new()
    };
    format!(
r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="{font}"/><w:sz w:val="{body_sz}"/><w:szCs w:val="{body_sz}"/></w:rPr></w:rPrDefault>
    <w:pPrDefault><w:pPr><w:spacing w:line="{line}" w:lineRule="auto"/></w:pPr></w:pPrDefault>
  </w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal">
    <w:name w:val="Normal"/>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidBody">
    <w:name w:val="BidBody"/>
    <w:pPr><w:spacing w:line="{line}" w:lineRule="auto"{space_after}/><w:ind w:firstLineChars="{indent}"/><w:jc w:val="{body_jc}"/></w:pPr>
    <w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="{font}"/><w:sz w:val="{body_sz}"/><w:szCs w:val="{body_sz}"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidTitle1">
    <w:name w:val="BidTitle1"/>
    <w:pPr><w:jc w:val="{h1_align}"/><w:outlineLvl w:val="0"/></w:pPr>
    <w:rPr>{h1_bold}<w:color w:val="{h1_color}"/><w:sz w:val="{h1_sz}"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidTitle2">
    <w:name w:val="BidTitle2"/>
    <w:pPr><w:jc w:val="{h2_align}"/><w:outlineLvl w:val="1"/></w:pPr>
    <w:rPr>{h2_bold}<w:color w:val="{h2_color}"/><w:sz w:val="{h2_sz}"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidTitle3">
    <w:name w:val="BidTitle3"/>
    <w:pPr><w:jc w:val="{h3_align}"/><w:outlineLvl w:val="2"/></w:pPr>
    <w:rPr>{h3_bold}<w:color w:val="{h3_color}"/><w:sz w:val="{h3_sz}"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidTitle4">
    <w:name w:val="BidTitle4"/>
    <w:pPr><w:jc w:val="{h4_align}"/><w:outlineLvl w:val="3"/></w:pPr>
    <w:rPr>{h4_bold}<w:color w:val="{h4_color}"/><w:sz w:val="{h4_sz}"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidTableHeader">
    <w:name w:val="BidTableHeader"/>
    <w:pPr><w:shd w:val="clear" w:color="auto" w:fill="{header_fill}"/><w:jc w:val="center"/></w:pPr>
    <w:rPr><w:b/><w:color w:val="{header_color}"/><w:sz w:val="{table_sz}"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidTableBody">
    <w:name w:val="BidTableBody"/>
    <w:pPr><w:jc w:val="center"/></w:pPr>
    <w:rPr><w:sz w:val="{table_sz}"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidCaption">
    <w:name w:val="BidCaption"/>
    <w:pPr><w:jc w:val="{cap_align}"/></w:pPr>
    <w:rPr><w:b/><w:sz w:val="{cap_sz}"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidFigureCell">
    <w:name w:val="BidFigureCell"/>
    <w:pPr><w:spacing w:line="240" w:lineRule="auto"/><w:jc w:val="center"/></w:pPr>
    <w:rPr><w:sz w:val="{body_sz}"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidTOC">
    <w:name w:val="BidTOC"/>
    <w:pPr><w:spacing w:line="360" w:lineRule="auto"/><w:jc w:val="left"/></w:pPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidHeader">
    <w:name w:val="BidHeader"/>
    <w:pPr><w:jc w:val="center"/></w:pPr>
    <w:rPr><w:color w:val="595959"/><w:sz w:val="18"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="BidFooter">
    <w:name w:val="BidFooter"/>
    <w:pPr><w:jc w:val="center"/></w:pPr>
    <w:rPr><w:color w:val="595959"/><w:sz w:val="18"/></w:rPr>
  </w:style>
  <w:style w:type="table" w:styleId="BidTable">
    <w:name w:val="BidTable"/>
    <w:tblPr><w:tblBorders>
      <w:top w:val="single" w:sz="{border_sz}" w:space="0" w:color="{outer_color}"/>
      <w:left w:val="single" w:sz="{border_sz}" w:space="0" w:color="{outer_color}"/>
      <w:bottom w:val="single" w:sz="{border_sz}" w:space="0" w:color="{outer_color}"/>
      <w:right w:val="single" w:sz="{border_sz}" w:space="0" w:color="{outer_color}"/>
      <w:insideH w:val="single" w:sz="{border_sz}" w:space="0" w:color="{inner_color}"/>
      <w:insideV w:val="single" w:sz="{border_sz}" w:space="0" w:color="{inner_color}"/>
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
"#,
        font = font,
        body_sz = body_sz,
        line = line,
        space_after = space_after,
        table_sz = table_sz,
        indent = indent,
        body_jc = body_jc,
        h1_align = style.heading1.align,
        h1_color = style.heading1.color,
        h1_sz = h1_sz,
        h1_bold = h1_bold,
        h2_align = style.heading2.align,
        h2_color = style.heading2.color,
        h2_sz = h2_sz,
        h2_bold = h2_bold,
        h3_align = style.heading3.align,
        h3_color = style.heading3.color,
        h3_sz = h3_sz,
        h3_bold = h3_bold,
        h4_align = style.heading4.align,
        h4_color = style.heading4.color,
        h4_sz = h4_sz,
        h4_bold = h4_bold,
        header_fill = style.table.header_fill,
        header_color = style.table.header_color,
        cap_align = style.figure.caption_align,
        cap_sz = cap_sz,
        border_sz = border_sz,
        outer_color = style.table.outer_color,
        inner_color = style.table.inner_color,
    )
}

/// 正文 align 语义映射：justify → both，其余（center/left）原样透传
fn jc_val(align: &str) -> &str {
    match align {
        "justify" => "both",
        other => other,
    }
}

/// 纸型 → pgSz 宽高（twips）：A4 / A4V 同值 11906×16838（A4 纵向）；其它值报错
fn pg_sz_twips(paper: &str) -> BidResult<(u32, u32)> {
    match paper {
        "A4" | "A4V" => Ok((11906, 16838)),
        other => Err(BidError::new(format!("不支持的纸型: {other}"))),
    }
}

/// 加粗标记：true → `<w:b/>`，false → 空串（保持 P0a 模板逐字一致）
fn bold_tag(bold: bool) -> &'static str {
    if bold { "<w:b/>" } else { "" }
}

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
  <Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>
  <Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/>
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

/// `word/_rels/document.xml.rels`：**必须显式写闭合标签**（不可自闭合），由 add_png 追加。
/// rId1=header1.xml / rId2=footer1.xml 固定（document.xml 的 sectPr 引用这两个 id）；
/// rId3=styles.xml / rId4=settings.xml 固定（缺这两条 WPS 丢样式、目录为空）
const DOCUMENT_RELS_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n  <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/header\" Target=\"header1.xml\"/>\n  <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer\" Target=\"footer1.xml\"/>\n  <Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\n  <Relationship Id=\"rId4\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>\n</Relationships>\n";

/// `word/header1.xml`：标题页眉（`{{DOC_TITLE}}` 由 B2 填充器替换）
const HEADER_XML: &str = r#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:p><w:pPr><w:pStyle w:val="BidHeader"/><w:jc w:val="center"/></w:pPr><w:r><w:t xml:space="preserve">{{DOC_TITLE}}</w:t></w:r></w:p></w:hdr>
"#;

/// `word/footer1.xml`：页脚（PAGE / NUMPAGES 域，默认占位值 1）
const FOOTER_XML: &str = r#"<w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:p><w:pPr><w:pStyle w:val="BidFooter"/><w:jc w:val="center"/></w:pPr><w:r><w:t xml:space="preserve">第 </w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple><w:r><w:t xml:space="preserve"> 页 / 共 </w:t></w:r><w:fldSimple w:instr=" NUMPAGES "><w:r><w:t>1</w:t></w:r></w:fldSimple><w:r><w:t xml:space="preserve"> 页</w:t></w:r></w:p></w:ftr>
"#;

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

    /// 十二个锚点逐个出现在 document.xml 的 w:val="…" 中
    #[test]
    fn all_twelve_tags_present_in_document_xml() {
        let doc = template_document_xml();
        for tag in ALL_TAGS {
            assert!(
                doc.contains(&format!("w:val=\"{tag}\"")),
                "document.xml 缺少锚点 {tag}"
            );
        }
    }

    /// 十三个 w:styleId="Bid…" 齐全（12 段落 + 1 表格）
    #[test]
    fn thirteen_bid_styles_present_in_styles_xml() {
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
            "BidTOC",
            "BidHeader",
            "BidFooter",
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
            13,
            "Bid 样式必须恰好 13 个"
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

    /// B1 新测 1：默认 DocStyle 渲染的 styles.xml 与 P0a 参数一致
    #[test]
    fn default_style_renders_p0a_params() {
        let styles = template_styles_xml();
        assert!(styles.contains(r#"<w:sz w:val="24"/>"#), "正文 12pt → sz 24");
        assert!(
            styles.contains(r#"<w:spacing w:line="360" w:lineRule="auto"/>"#),
            "1.5 倍行距 → line 360"
        );
        assert!(styles.contains("253D57"), "h1 颜色 253D57");
        assert!(styles.contains(r#"w:sz="4""#), "边框 0.5pt → sz 4");
    }

    /// B1 新测 2：DocStyle 改动必须可观测（字号/颜色/页边距）
    #[test]
    fn docstyle_changes_are_observable() {
        let mut style = DocStyle::default();
        style.body.size_pt = 14;
        style.heading1.color = "112233".to_string();
        style.page.margin_cm = 2.0;
        let styles = template_styles_xml_for(&style);
        assert!(styles.contains(r#"<w:sz w:val="28"/>"#), "14pt → sz 28: {styles}");
        assert!(styles.contains("112233"), "h1 颜色被覆盖: {styles}");
        let doc = template_document_xml_for(&style).unwrap();
        assert!(doc.contains(r#"w:top="1134""#), "2.0cm → 1134 twips: {doc}");
    }

    /// B1 新测 3：v2 包内 9 条目 + header/footer/TOC 齐全
    #[test]
    fn template_v2_has_header_footer_toc() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-b1-v2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("bid-v2.docx");
        generate_to_with(&out, &DocStyle::default()).unwrap();

        let pkg = DocxPackage::open(&out).unwrap();
        assert_eq!(pkg.entries.len(), 9, "包内必须 9 条目");
        for name in ["word/header1.xml", "word/footer1.xml"] {
            assert!(pkg.contains(name), "包内缺少条目 {name}");
        }
        let ct = pkg.text("[Content_Types].xml").unwrap();
        assert!(
            ct.contains(r#"/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml""#),
            "content-types 缺 header Override"
        );
        assert!(
            ct.contains(r#"/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml""#),
            "content-types 缺 footer Override"
        );
        let rels = pkg.text("word/_rels/document.xml.rels").unwrap();
        assert!(
            rels.contains(r#"Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml""#),
            "rels 缺 rId1 header"
        );
        assert!(
            rels.contains(r#"Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml""#),
            "rels 缺 rId2 footer"
        );
        let doc = pkg.text("word/document.xml").unwrap();
        assert!(doc.contains("headerReference"), "document.xml 缺 headerReference");
        assert!(doc.contains("footerReference"), "document.xml 缺 footerReference");
        assert!(doc.contains("BID_TOC"), "document.xml 缺 BID_TOC");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// B1 新测 4：ALL_TAGS 恰好 12 且全部出现在 document.xml
    #[test]
    fn all_tags_is_twelve() {
        assert_eq!(ALL_TAGS.len(), 12, "ALL_TAGS 必须 12 个");
        let doc = template_document_xml();
        for tag in ALL_TAGS {
            assert!(
                doc.contains(&format!("w:val=\"{tag}\"")),
                "document.xml 缺少锚点 {tag}"
            );
        }
    }

    /// B2 新测：纸型不受支持（A3）→ template_document_xml_for 返回 Err 且含"不支持的纸型"
    #[test]
    fn unsupported_paper_rejected() {
        let mut style = DocStyle::default();
        style.page.paper = "A3".to_string();
        let err = template_document_xml_for(&style).unwrap_err();
        assert!(
            err.to_string().contains("不支持的纸型"),
            "错误信息不符: {err}"
        );
    }

    /// B2 新测：A4 与 A4V 渲染的 pgSz 同为 11906×16838
    #[test]
    fn a4_and_a4v_same_pgsz() {
        for paper in ["A4", "A4V"] {
            let mut style = DocStyle::default();
            style.page.paper = paper.to_string();
            let doc = template_document_xml_for(&style).unwrap();
            assert!(
                doc.contains("w:w=\"11906\"") && doc.contains("w:h=\"16838\""),
                "纸型 {paper} 的 pgSz 不符: {doc}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // §7.3 压缩档位（B 片）：表字号占位 + 段后距占位 + 默认产物字节稳定
    // -----------------------------------------------------------------------

    /// 取 styles.xml 里某个 styleId 的整块（`<w:style …>` 到下一个 `<w:style` 之前）
    fn style_block(styles: &str, id: &str) -> String {
        let marker = format!("w:styleId=\"{id}\"");
        let marker_pos = styles
            .find(&marker)
            .unwrap_or_else(|| panic!("styles.xml 缺样式 {id}"));
        let start = styles[..marker_pos].rfind("<w:style").expect("style 起始标签缺失");
        let end = styles[start + 1..]
            .find("<w:style")
            .map_or(styles.len(), |p| start + 1 + p);
        styles[start..end].to_string()
    }

    /// §7.3：`BidTableHeader`/`BidTableBody` 的 `<w:sz>` 取自 `style.table.size_pt`
    /// （默认 12pt → 24；压缩 10.5pt → 21）
    #[test]
    fn table_style_size_follows_docstyle() {
        let default_styles = template_styles_xml();
        for id in ["BidTableHeader", "BidTableBody"] {
            let block = style_block(&default_styles, id);
            assert!(
                block.contains(r#"<w:sz w:val="24"/>"#),
                "{id} 默认须 12pt（sz 24）: {block}"
            );
        }

        let compact = DocStyle::default().compact();
        assert_eq!(compact.table.size_pt, 10.5);
        let compact_styles = template_styles_xml_for(&compact);
        for id in ["BidTableHeader", "BidTableBody"] {
            let block = style_block(&compact_styles, id);
            assert!(
                block.contains(r#"<w:sz w:val="21"/>"#),
                "{id} 压缩后须 10.5pt（sz 21）: {block}"
            );
            assert!(
                !block.contains(r#"<w:sz w:val="24"/>"#),
                "{id} 压缩后不得残留 12pt: {block}"
            );
        }
        // 正文字号不随表字号变化（压缩只把表字号降到 10.5）
        let body = style_block(&compact_styles, "BidBody");
        assert!(
            body.contains(r#"<w:sz w:val="24"/>"#),
            "BidBody 正文字号不应被表字号影响: {body}"
        );
    }

    /// §7.3：`BidBody` 段后距 → `w:spacing/@w:after`（pt*20）；0pt 时不输出该属性
    #[test]
    fn body_space_after_rendered_only_when_positive() {
        let default_body = style_block(&template_styles_xml(), "BidBody");
        assert!(
            !default_body.contains("w:after="),
            "默认 0pt 不得输出 w:after: {default_body}"
        );
        assert!(
            default_body.contains(r#"<w:spacing w:line="360" w:lineRule="auto"/>"#),
            "默认段后距下 spacing 须与 P0a 逐字一致: {default_body}"
        );

        let mut style = DocStyle::default();
        style.body.space_after_pt = 6;
        let body = style_block(&template_styles_xml_for(&style), "BidBody");
        assert!(
            body.contains(r#"w:after="120""#),
            "6pt → w:after 120: {body}"
        );
        assert!(
            body.contains(r#"w:line="360""#),
            "段后距不得顶掉行距: {body}"
        );
    }

    /// 默认 DocStyle 的 document.xml / styles.xml sha256 钉死（P0a 断言不得因新字段漂移）
    #[test]
    fn default_template_bytes_are_pinned() {
        assert_eq!(
            sha256_hex(template_document_xml().as_bytes()),
            "bf4d37b28304784fe5f1aee11a2179ee0b013af0010ddb147b9582d0818d1d9b",
            "默认 document.xml 与 P0a 模板字节级不一致"
        );
        assert_eq!(
            sha256_hex(template_styles_xml().as_bytes()),
            "c042c97121843518f030c86d7e1dee0f936adcd8f4a259e47e06ebb30bc344bb",
            "默认 styles.xml 与拆分前字节级不一致"
        );
    }

    /// sha256 → 小写十六进制
    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }
}
