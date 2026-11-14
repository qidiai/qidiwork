//! T2c：填充器（规格 §6.2）。
//!
//! 职责：把 BidExportRequest 灌进模板锚点，产出 docx。
//! - 章模板（BID_CHAPTER）整段取用，逐章替换 TITLE/BODY 后拼接
//! - 表格/图片/图注走 §5.5 片段库（include_str!）
//! - BID_TABLE_ROWS / BID_FIGURE_SINGLE 两个部件库块整块删成空串

use std::path::Path;

use super::docx::{drawing_run, emu_from_cm, escape_xml, image_size_px, DocxPackage};
use super::model::{BidExportRequest, Block, FillStats};
use super::template::{apply_edits, content, find, index_sdt, SdtRange};
use super::template_gen::{
    ALL_TAGS, TAG_CHAPTER, TAG_CHAPTER_BODY, TAG_CHAPTER_TITLE, TAG_CHAPTERS, TAG_DOC_TITLE,
    TAG_FIGURE_SINGLE, TAG_TABLE_ROWS,
};
use super::{BidError, BidResult};

/// §5.5 片段库（编译期内联）
const PARAGRAPH_XML: &str = include_str!("../../../resources/templates/fragments/paragraph.xml");
const TABLE_XML: &str = include_str!("../../../resources/templates/fragments/table.xml");
const TABLE_ROW_XML: &str = include_str!("../../../resources/templates/fragments/table_row.xml");
const FIGURE_SINGLE_XML: &str = include_str!("../../../resources/templates/fragments/figure_single.xml");
const CAPTION_XML: &str = include_str!("../../../resources/templates/fragments/caption.xml");

/// 内容区宽（twips）：A4 宽 11906 - 2×680 页边距 = 10546
const CONTENT_WIDTH_TWIPS: u32 = 10546;

/// 锚点查找（缺失即报错，供 export 主流程与章内渲染共用）
fn anchor<'a>(ranges: &'a [SdtRange], tag: &str) -> BidResult<&'a SdtRange> {
    find(ranges, tag).ok_or_else(|| BidError::new(format!("模板缺少锚点 {tag}")))
}

/// 导出主流程（规格 §6.2 八步）
pub fn export(req: &BidExportRequest, template: &Path, out: &Path) -> BidResult<(FillStats, u64)> {
    // 1) 校验 + 输出路径非空
    req.validate()?;
    if req.output_path.trim().is_empty() {
        return Err(BidError::new("输出路径为空"));
    }
    // 2) 开包，取 word/document.xml
    let mut pkg = DocxPackage::open(template)?;
    let xml = pkg.text("word/document.xml")?;
    // 3) 锚点索引，ALL_TAGS 逐个 find，缺失即报错
    let ranges = index_sdt(&xml)?;
    for tag in ALL_TAGS {
        anchor(&ranges, tag)?;
    }
    // 4) 部件取用：仅章模板需从原串切片
    let chapter_tpl = content(&xml, anchor(&ranges, TAG_CHAPTER)?);
    // 5) 章渲染：章模板再建索引，逐章替换 TITLE/BODY
    let chapter_ranges = index_sdt(chapter_tpl)?;
    let title_rng = anchor(&chapter_ranges, TAG_CHAPTER_TITLE)?;
    let body_rng = anchor(&chapter_ranges, TAG_CHAPTER_BODY)?;
    let mut chapters_xml = String::new();
    let mut stats = FillStats::default();
    let mut fig_no = 0u32;
    for chapter in &req.chapters {
        let title_para = format!(
            "<w:p><w:pPr><w:pStyle w:val=\"BidTitle1\"/><w:pageBreakBefore/></w:pPr>\
             <w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
            escape_xml(&chapter.title)
        );
        let mut body_xml = String::new();
        for block in &chapter.blocks {
            match block {
                Block::Paragraph { .. } => stats.paragraphs += 1,
                Block::Table { .. } => stats.tables += 1,
                Block::Figure { .. } => stats.figures += 1,
            }
            body_xml.push_str(&render_block(block, &mut pkg, &mut fig_no, req.figure_width_cm)?);
        }
        let mut edits = vec![
            (title_rng.content_start, title_rng.content_end, title_para),
            (body_rng.content_start, body_rng.content_end, body_xml),
        ];
        chapters_xml.push_str(&apply_edits(chapter_tpl, &mut edits)?);
    }
    stats.chapters = req.chapters.len();
    // 6) 文档装配：标题段 / 章区 / 两个部件库块整块删除
    let doc_title = anchor(&ranges, TAG_DOC_TITLE)?;
    let chapters = anchor(&ranges, TAG_CHAPTERS)?;
    let table_rows = anchor(&ranges, TAG_TABLE_ROWS)?;
    let figure_single = anchor(&ranges, TAG_FIGURE_SINGLE)?;
    let doc_title_para = format!(
        "<w:p><w:pPr><w:pStyle w:val=\"BidTitle1\"/></w:pPr>\
         <w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
        escape_xml(&req.title)
    );
    let mut edits = vec![
        (doc_title.content_start, doc_title.content_end, doc_title_para),
        (chapters.content_start, chapters.content_end, chapters_xml),
        (table_rows.block_start, table_rows.block_end, String::new()),
        (figure_single.block_start, figure_single.block_end, String::new()),
    ];
    let new_xml = apply_edits(&xml, &mut edits)?;
    // 7) 写回 document.xml
    pkg.set_text("word/document.xml", new_xml);
    // 8) 父目录不存在先创建，再保存
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    pkg.save(out)?;
    Ok((stats, std::fs::metadata(out)?.len()))
}

/// 块渲染（§6.4）：段落 → paragraph 片段；表格 → render_table；图片 → render_figure
fn render_block(
    block: &Block,
    pkg: &mut DocxPackage,
    fig_no: &mut u32,
    figure_width_cm: Option<f32>,
) -> BidResult<String> {
    match block {
        Block::Paragraph { text } => Ok(PARAGRAPH_XML
            .replace("{{STYLE}}", "BidBody")
            .replace("{{TEXT}}", &escape_xml(text))),
        Block::Table { header, rows } => render_table(header, rows),
        Block::Figure { path, caption, ai_generated } => {
            *fig_no += 1;
            render_figure(pkg, *fig_no, path, caption, *ai_generated, figure_width_cm)
        }
    }
}

/// 表格渲染（§6.4 + §5.5 table.xml / table_row.xml）
fn render_table(header: &[String], rows: &[Vec<String>]) -> BidResult<String> {
    let cols = header.len();
    if cols == 0 {
        return Err(BidError::new("表格表头不能为空"));
    }
    // 内容区 10546 twips 均分，余数给首列
    let base = CONTENT_WIDTH_TWIPS / cols as u32;
    let rem = CONTENT_WIDTH_TWIPS % cols as u32;
    let widths: Vec<u32> = (0..cols).map(|c| base + if c == 0 { rem } else { 0 }).collect();
    let grid: String = widths
        .iter()
        .map(|w| format!("<w:gridCol w:w=\"{w}\"/>"))
        .collect();

    let mut rows_xml = String::new();
    // 表头行：{{HEADER_FLAG}} = <w:tblHeader/>，单元格样式 BidTableHeader
    rows_xml.push_str(&render_row(&widths, header, "<w:tblHeader/>", "BidTableHeader"));
    // 数据行：flag 空串 + BidTableBody
    for row in rows {
        rows_xml.push_str(&render_row(&widths, row, "", "BidTableBody"));
    }
    Ok(TABLE_XML
        .replace("{{GRID}}", &grid)
        .replace("{{ROWS}}", &rows_xml))
}

/// 单行渲染（§5.5 table_row.xml）：表头行带 <w:tblHeader/>，数据行 flag 为空串
fn render_row(widths: &[u32], cells: &[String], header_flag: &str, style: &str) -> String {
    let mut cells_xml = String::new();
    for (i, text) in cells.iter().enumerate() {
        let w = widths.get(i).copied().unwrap_or(0);
        cells_xml.push_str(&format!(
            "<w:tc><w:tcPr><w:tcW w:w=\"{w}\" w:type=\"dxa\"/></w:tcPr>\
             <w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr>\
             <w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p></w:tc>",
            escape_xml(text)
        ));
    }
    TABLE_ROW_XML
        .replace("{{HEADER_FLAG}}", header_flag)
        .replace("{{CELLS}}", &cells_xml)
}

/// 单图渲染（§6.3 尺寸算法 + §6.4 figure_single.xml / caption.xml）
fn render_figure(
    pkg: &mut DocxPackage,
    fig_no: u32,
    path: &str,
    caption: &str,
    ai_generated: bool,
    figure_width_cm: Option<f32>,
) -> BidResult<String> {
    let (px_w, px_h) = image_size_px(Path::new(path))?;
    // 宽 = figure_width_cm（默认 15.5cm）；高按像素比换算，整数四舍五入
    let mut width_em = emu_from_cm(figure_width_cm.unwrap_or(15.5));
    let mut height_em = (width_em * px_h as u64 + px_w as u64 / 2) / px_w as u64;
    // 高超过 20cm 上限时按高反算宽，保持比例
    let max_h_em = emu_from_cm(20.0);
    if height_em > max_h_em {
        width_em = max_h_em * px_w as u64 / px_h as u64;
        height_em = max_h_em;
    }
    // P0a 恒"每图一注"：ai_generated 时图注追加 AI 标注
    let caption_final = if ai_generated {
        format!("{caption}（AI 生成示意图）")
    } else {
        caption.to_string()
    };
    let png = std::fs::read(path)?;
    let rid = pkg.add_png(&png, &format!("fig-{fig_no}"))?;
    let drawing = drawing_run(&rid, width_em, height_em, fig_no, &caption_final);
    let caption_xml = CAPTION_XML.replace("{{TEXT}}", &escape_xml(&caption_final));
    Ok(FIGURE_SINGLE_XML
        .replace("{{DRAWING}}", &drawing)
        .replace("{{CAPTION}}", &caption_xml))
}

// ---------------------------------------------------------------------------
// 单测（规格 §6.2 验收：最小样本导出 / 转义 / 缺锚点报错）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use super::super::model::Chapter;
    use super::super::verify::{self, Expect};
    use std::collections::BTreeMap;

    const TEMPLATE: &str = "resources/templates/bid-engineering-blue-v1.docx";
    fn asset_png() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/bid-export/assets/image3.png")
            .to_string_lossy()
            .to_string()
    }

    fn minimal_req(body: &str, out: &Path) -> BidExportRequest {
        BidExportRequest {
            title: "t".to_string(),
            chapters: vec![Chapter {
                title: "c".to_string(),
                blocks: vec![Block::Paragraph { text: body.to_string() }],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        }
    }

    /// 最小样本（§7.1 口径）：2 章 4 段 1 表 1 图 → verify 16 项全过
    #[test]
    fn export_minimal_project_passes_verify() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-t2c1-fill-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("minimal.docx");
        let img = asset_png();
        let req = BidExportRequest {
            title: "槐林镇2026年度和美乡村省级中心村建设项目施工组织设计".to_string(),
            chapters: vec![
                Chapter {
                    title: "第一章 工程概况".to_string(),
                    blocks: vec![
                        Block::Paragraph { text: "本工程为槐林镇2026年度和美乡村省级中心村建设项目，主要建设内容包括道路硬化、排水管网、文化广场及绿化提升。".to_string() },
                        Block::Paragraph { text: "施工组织设计遵循先地下后地上、先深后浅的原则，确保工序衔接与成品保护。".to_string() },
                        Block::Table {
                            header: vec!["工序节点".into(), "施工方法及控制参数".into(), "质量控制要点".into()],
                            rows: vec![
                                vec!["塘底清淤".into(), "长臂挖机分段清淤，深度偏差≤50mm".into(), "隐蔽验收合格后方可回填".into()],
                                vec!["路基整平".into(), "推土机粗平+平地机精平，压实度≥93%".into(), "每200m检测一点".into()],
                            ],
                        },
                        Block::Figure { path: img.clone(), caption: "图1-1 塘底整平与排水示意".to_string(), ai_generated: true },
                    ],
                },
                Chapter {
                    title: "第二章 施工部署".to_string(),
                    blocks: vec![
                        Block::Paragraph { text: "项目部设项目经理1人、技术负责人1人，按专业划分三个作业队。".to_string() },
                        Block::Paragraph { text: "总工期90日历天，其中土方与管网45天，道路与广场30天，绿化及收尾15天。".to_string() },
                    ],
                },
            ],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        };

        let (stats, bytes) = export(&req, Path::new(TEMPLATE), &out).unwrap();
        assert_eq!((stats.chapters, stats.paragraphs, stats.tables, stats.figures), (2, 4, 1, 1));
        assert!(bytes > 0);
        let report = verify::verify(&out, Some(&Expect { chapters: 2, figures: 1, tables: 1, paragraphs: 4 })).unwrap();
        let bad: Vec<String> = report.checks.iter().filter(|c| !c.ok)
            .map(|c| format!("{}:{}", c.name, c.detail)).collect();
        assert!(bad.is_empty(), "不合格 check: {}", bad.join("; "));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 正文含 <&>"' → document.xml 必须含对应实体串
    #[test]
    fn export_escapes_text() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-t2c1-esc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("esc.docx");
        let req = minimal_req("<&>\"'", &out);
        export(&req, Path::new(TEMPLATE), &out).unwrap();
        let pkg = DocxPackage::open(&out).unwrap();
        let doc = pkg.text("word/document.xml").unwrap();
        assert!(
            doc.contains("&lt;&amp;&gt;&quot;&apos;"),
            "document.xml 未含五实体转义串: {doc}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 自造无锚点最小包 → export 必须 Err 且信息含"模板缺少锚点"
    #[test]
    fn export_missing_anchor_fails() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-t2c1-miss-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tpl = dir.join("tpl.docx");
        let mut pkg = DocxPackage { entries: BTreeMap::new() };
        pkg.set_text("[Content_Types].xml", "<Types/>".to_string());
        pkg.set_text("_rels/.rels", "<Relationships/>".to_string());
        pkg.set_text("word/document.xml", "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body/></w:document>".to_string());
        pkg.set_text("word/_rels/document.xml.rels", "<Relationships/>".to_string());
        pkg.save(&tpl).unwrap();

        let out = dir.join("out.docx");
        let err = export(&minimal_req("b", &out), &tpl, &out).unwrap_err();
        assert!(err.to_string().contains("模板缺少锚点"), "错误信息不符: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
