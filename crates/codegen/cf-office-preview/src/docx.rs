//! docx → 纯文本提取
//!
//! docx 即 zip 包，正文在 `word/document.xml`；用 quick-xml 流式提取
//! `<w:p>` 段落内的 `<w:t>` 文本 run。表格/图片暂不结构化渲染。

use super::PreviewContent;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::io::Read;

/// 将 docx 文件转为纯文本预览
pub fn docx_to_preview(path: &str) -> Result<PreviewContent, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("zip {path}: {e}"))?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|e| format!("word/document.xml: {e}"))?
        .read_to_string(&mut xml)
        .map_err(|e| format!("read document.xml: {e}"))?;
    Ok(PreviewContent::Docx {
        text: extract_text(&xml),
    })
}

/// 从 document.xml 提取段落文本（每 `<w:p>` 一行，`<w:tab>`/`<w:br>` 保留）
fn extract_text(xml: &str) -> String {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut out = String::new();
    let mut para = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match e.name().as_ref() {
                b"w:tab" => para.push('\t'),
                b"w:br" => para.push('\n'),
                _ => {}
            },
            Ok(Event::Text(e)) => {
                if let Ok(t) = e.decode() {
                    para.push_str(&t);
                }
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"w:p" => {
                out.push_str(para.trim_end());
                out.push('\n');
                para.clear();
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_paragraph_text() {
        let xml = r#"<w:document><w:body>
            <w:p><w:r><w:t>第一段</w:t></w:r><w:r><w:t>接续</w:t></w:r></w:p>
            <w:p><w:r><w:t>第二段</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let text = extract_text(xml);
        assert_eq!(text, "第一段接续\n第二段\n");
    }

    #[test]
    fn handles_tab_and_br() {
        let xml = r#"<w:p><w:r><w:t>a</w:t><w:tab/><w:t>b</w:t><w:br/><w:t>c</w:t></w:r></w:p>"#;
        let text = extract_text(xml);
        assert_eq!(text, "a\tb\nc\n");
    }

    #[test]
    fn malformed_xml_does_not_panic() {
        let text = extract_text("<w:p><w:t>未闭合");
        assert!(text.contains('未') || text.is_empty());
    }
}
