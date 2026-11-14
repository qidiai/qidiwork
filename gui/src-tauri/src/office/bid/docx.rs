//! docx 包读写底座（zip + 条目内存化）。
//!
//! docx = zip 包；`entries` 全量读入内存（模板 <50KB，样本导出 <2MB，可接受）。
//! `add_png` 负责 media 文件 + rels + `[Content_Types].xml` 三处联动（规格 §4.2）。

use std::collections::BTreeMap;
use std::io::Read as _;
use std::io::Write as _;
use std::path::Path;

use super::{BidError, BidResult};

/// docx = zip 包；entries 全量读入内存（模板 <50KB，样本导出 <2MB，可接受）
pub struct DocxPackage {
    pub entries: BTreeMap<String, Vec<u8>>,
}

impl DocxPackage {
    pub fn open(path: &Path) -> BidResult<Self> {
        let file = std::fs::File::open(path)?;
        let mut archive = zip::read::ZipArchive::new(file)
            .map_err(|e| BidError::new(format!("解析 docx 失败: {e}")))?;
        let mut entries = BTreeMap::new();
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| BidError::new(format!("读取 docx 条目失败: {e}")))?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().to_string();
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| BidError::new(format!("读取 docx 条目 {name} 失败: {e}")))?;
            entries.insert(name, buf);
        }
        Ok(Self { entries })
    }

    /// 写 zip；`[Content_Types].xml` 必须第一个条目
    pub fn save(&self, path: &Path) -> BidResult<()> {
        let file = std::fs::File::create(path)?;
        let mut zip = zip::write::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let content_types = "[Content_Types].xml";
        if let Some(bytes) = self.entries.get(content_types) {
            zip.start_file(content_types, opts)
                .map_err(|e| BidError::new(format!("写 docx 条目失败: {e}")))?;
            zip.write_all(bytes)?;
        }
        for (name, bytes) in &self.entries {
            if name == content_types {
                continue;
            }
            zip.start_file(name, opts)
                .map_err(|e| BidError::new(format!("写 docx 条目失败: {e}")))?;
            zip.write_all(bytes)?;
        }
        zip.finish()
            .map_err(|e| BidError::new(format!("收尾 docx 失败: {e}")))?;
        Ok(())
    }

    pub fn text(&self, name: &str) -> BidResult<String> {
        let bytes = self
            .entries
            .get(name)
            .ok_or_else(|| BidError::new(format!("条目不存在: {name}")))?;
        String::from_utf8(bytes.clone())
            .map_err(|e| BidError::new(format!("条目 {name} 不是合法 UTF-8: {e}")))
    }

    pub fn set_text(&mut self, name: &str, xml: String) {
        self.entries.insert(name.to_string(), xml.into_bytes());
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// 写 word/media/<stem>-<n>.png + 补 rels + 补 [Content_Types]；返回 rId（形如 "rId5"）
    pub fn add_png(&mut self, bytes: &[u8], stem: &str) -> BidResult<String> {
        // 1. 媒体名 word/media/<stem>-<n>.png，n 从 1 起，冲突递增
        let mut n = 1usize;
        while self.entries.contains_key(&format!("word/media/{stem}-{n}.png")) {
            n += 1;
        }
        let media_name = format!("word/media/{stem}-{n}.png");

        // 2. rels：rId 取现有数字最大值 +1，在 </Relationships> 前插入；
        //    自闭合根元素兜底：把结尾 /> 改写成 >…</Relationships> 再插
        let rels_name = "word/_rels/document.xml.rels";
        let rels = self.entries.get(rels_name).cloned().unwrap_or_default();
        let rels = String::from_utf8(rels)
            .map_err(|e| BidError::new(format!("{rels_name} 不是合法 UTF-8: {e}")))?;
        let rid = format!("rId{}", max_rid_number(&rels) + 1);
        let relationship = format!(
            "<Relationship Id=\"{rid}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/{stem}-{n}.png\"/>"
        );
        let new_rels = match insert_before(&rels, "</Relationships>", &relationship) {
            Some(s) => s,
            None => {
                // 兜底：自闭合根元素 <Relationships …/> → 改写为 >…</Relationships> 再插
                let trimmed = rels.trim_end();
                let head = trimmed.strip_suffix("/>").ok_or_else(|| {
                    BidError::new(format!("{rels_name} 缺少 </Relationships> 且根元素非自闭合写法"))
                })?;
                format!("{head}>{relationship}</Relationships>")
            }
        };
        self.set_text(rels_name, new_rels);

        // 3. [Content_Types].xml：png Default 必须排在所有 Override 之前
        let ct_name = "[Content_Types].xml";
        let ct = self.entries.get(ct_name).cloned().unwrap_or_default();
        let ct = String::from_utf8(ct)
            .map_err(|e| BidError::new(format!("{ct_name} 不是合法 UTF-8: {e}")))?;
        if !ct.contains("Extension=\"png\"") {
            let default = "<Default Extension=\"png\" ContentType=\"image/png\"/>";
            let new_ct = insert_before(&ct, "<Override", default)
                .or_else(|| insert_before(&ct, "</Types>", default))
                .ok_or_else(|| {
                    BidError::new(format!("{ct_name} 缺少 <Override 与 </Types>，无法插入 png Default"))
                })?;
            self.set_text(ct_name, new_ct);
        }

        // 4. 媒体字节入包
        self.entries.insert(media_name, bytes.to_vec());
        Ok(rid)
    }
}

/// 在 `marker` 首次出现处之前插入 `insert`；找不到返回 None
fn insert_before(xml: &str, marker: &str, insert: &str) -> Option<String> {
    xml.find(marker).map(|pos| {
        let mut out = String::with_capacity(xml.len() + insert.len());
        out.push_str(&xml[..pos]);
        out.push_str(insert);
        out.push_str(&xml[pos..]);
        out
    })
}

/// 现有 rId 数字最大值（无 rId 时为 0）；取最大值 +1，不用条目数
fn max_rid_number(rels: &str) -> u32 {
    let needle = "Id=\"rId";
    let mut max = 0u32;
    let mut from = 0;
    while let Some(at) = rels[from..].find(needle) {
        let digits_at = from + at + needle.len();
        let digits: String = rels[digits_at..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(v) = digits.parse::<u32>() {
            max = max.max(v);
        }
        from = digits_at;
    }
    max
}

/// 1cm = 360000 EMU
pub fn emu_from_cm(cm: f32) -> u64 {
    (cm * 360_000.0).round() as u64
}

/// 用 image crate 读像素尺寸（只支持 png/jpeg）
pub fn image_size_px(path: &Path) -> BidResult<(u32, u32)> {
    image::ImageReader::open(path)
        .map_err(|e| BidError::new(format!("打开图片失败: {e}")))?
        .with_guessed_format()
        .map_err(|e| BidError::new(format!("猜测图片格式失败: {e}")))?
        .into_dimensions()
        .map_err(|e| BidError::new(format!("读取图片尺寸失败: {e}")))
}

/// 生成 <w:r><w:drawing>…</w:drawing></w:r>（规格 §6.3 逐字模板）
pub fn drawing_run(rid: &str, cx: u64, cy: u64, doc_pr_id: u32, caption: &str) -> String {
    let id = doc_pr_id;
    let caption = escape_xml(caption);
    format!(
        "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">\n\
<wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>\n\
<wp:docPr id=\"{id}\" name=\"Figure {id}\" descr=\"{caption}\"/>\n\
<wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr>\n\
<a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\n\
<pic:pic><pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"Figure {id}\"/><pic:cNvPicPr/></pic:nvPicPr>\n\
<pic:blipFill><a:blip r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\n\
<pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\n\
<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>\n\
</a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"
    )
}

/// XML 文本转义：& < > " '
pub fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn asset_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/bid-export/assets/image3.png")
    }

    /// 测试用最小 docx 包（模板文件由下一片 template_gen 生成，此处自建于临时目录）
    fn write_test_docx(path: &Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::write::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let entries: [(&str, &str); 4] = [
            ("[Content_Types].xml", "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>"),
            ("_rels/.rels", "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>"),
            ("word/document.xml", "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t>round-trip 模板正文</w:t></w:r></w:p></w:body></w:document>"),
            ("word/_rels/document.xml.rels", "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>"),
        ];
        for (name, data) in entries {
            zip.start_file(name, opts).unwrap();
            zip.write_all(data.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn escape_xml_escapes_all_five() {
        assert_eq!(escape_xml("a<b&c>\"d'"), "a&lt;b&amp;c&gt;&quot;d&apos;");
    }

    #[test]
    fn emu_from_cm_matches_spec() {
        assert_eq!(emu_from_cm(1.0), 360000);
        assert_eq!(emu_from_cm(15.5), 5580000);
    }

    #[test]
    fn image_size_px_reads_asset() {
        let path = asset_path();
        assert_eq!(image_size_px(&path).unwrap(), (840, 289));
    }

    #[test]
    fn docx_package_round_trip() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-t1a-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("tpl.docx");
        let dst = dir.join("out.docx");
        write_test_docx(&src);
        let original = DocxPackage::open(&src).unwrap();
        let original_text = original.text("word/document.xml").unwrap();
        original.save(&dst).unwrap();

        // [Content_Types].xml 必须是保存后 zip 的第一个条目
        let saved = std::fs::File::open(&dst).unwrap();
        let mut archive = zip::read::ZipArchive::new(saved).unwrap();
        assert_eq!(archive.by_index(0).unwrap().name(), "[Content_Types].xml");

        let reopened = DocxPackage::open(&dst).unwrap();
        assert_eq!(reopened.text("word/document.xml").unwrap(), original_text);
    }

    #[test]
    fn add_png_injects_media_rels_and_content_types() {
        let png = std::fs::read(asset_path()).unwrap();

        let mut pkg = DocxPackage { entries: BTreeMap::new() };
        pkg.set_text("[Content_Types].xml", "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>".to_string());
        pkg.set_text("word/_rels/document.xml.rels", "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>".to_string());

        let rid = pkg.add_png(&png, "x").unwrap();
        assert_eq!(rid, "rId2");
        assert!(pkg.contains("word/media/x-1.png"));
        let rels = pkg.text("word/_rels/document.xml.rels").unwrap();
        assert!(rels.contains(&format!("Id=\"{rid}\"")));
        assert!(rels.contains("Target=\"media/x-1.png\""));
        let ct = pkg.text("[Content_Types].xml").unwrap();
        let png_pos = ct.find("Extension=\"png\"").unwrap();
        let override_pos = ct.find("<Override").unwrap();
        assert!(png_pos < override_pos, "png Default 必须排在所有 Override 之前");

        // 第二张图：媒体名 n 递增、rId 取现有最大值 +1
        let rid2 = pkg.add_png(&png, "x").unwrap();
        assert_eq!(rid2, "rId3");
        assert!(pkg.contains("word/media/x-2.png"));

        // 自闭合根元素兜底（已知坑 1）
        let mut pkg2 = DocxPackage { entries: BTreeMap::new() };
        pkg2.set_text("[Content_Types].xml", "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>".to_string());
        pkg2.set_text("word/_rels/document.xml.rels", "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>".to_string());
        let rid3 = pkg2.add_png(&png, "y").unwrap();
        assert_eq!(rid3, "rId1");
        let rels2 = pkg2.text("word/_rels/document.xml.rels").unwrap();
        assert!(rels2.contains("</Relationships>"));
        assert!(rels2.contains("Id=\"rId1\""));
        assert!(rels2.contains("Target=\"media/y-1.png\""));
    }
}
