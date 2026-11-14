//! T2a：sdt 锚点索引与替换（规格 §6.1）。
//!
//! 职责：对模板 `document.xml` 的事件流建索引，得到全部 `w:sdt` 锚点的字节区间
//! （`SdtRange`），并提供按 start 降序应用的 `apply_edits`，供后续填充/校验片消费。
//! 禁止字符串 `find("<w:sdt")` + 手写深度计数（嵌套会错），必须走事件流。

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::{BidError, BidResult};

/// 一个 `w:sdt` 锚点的字节区间（字段语义见规格 §6.1）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdtRange {
    pub tag: String,
    pub block_start: usize,   // "<w:sdt" 起始字节
    pub block_end: usize,     // "</w:sdt>" 结束后字节（含）
    pub content_start: usize, // <w:sdtContent> 结束 ">" 之后
    pub content_end: usize,   // </w:sdtContent> 起始 "<" 之前
}

/// 事件流处理用的栈帧
struct Frame {
    tag: Option<String>,
    content_start: Option<usize>,
    content_end: Option<usize>,
    block_start: usize,
}

/// 从 `w:tag` 元素上取 `w:val` 属性值
fn tag_val(e: &BytesStart) -> BidResult<Option<String>> {
    for attr in e.attributes() {
        let attr = attr.map_err(|e| BidError::new(format!("w:tag 属性解析失败: {e}")))?;
        if attr.key.as_ref() == "w:val" {
            return Ok(Some(attr.value.into_owned()));
        }
    }
    Ok(None)
}

/// 对 `xml` 事件流建索引，返回按出现顺序排列的全部 `w:sdt` 锚点区间
pub fn index_sdt(xml: &str) -> BidResult<Vec<SdtRange>> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Frame> = Vec::new();
    let mut ranges: Vec<SdtRange> = Vec::new();
    loop {
        let prev = reader.buffer_position() as usize;
        let ev = reader
            .read_event()
            .map_err(|e| BidError::new(format!("XML 事件流解析失败: {e}")))?;
        let cur = reader.buffer_position() as usize;
        match ev {
            Event::Start(e) => match e.name().as_ref() {
                "w:sdt" => stack.push(Frame {
                    tag: None,
                    content_start: None,
                    content_end: None,
                    block_start: prev,
                }),
                "w:tag" => {
                    if let Some(frame) = stack.last_mut() {
                        if let Some(v) = tag_val(&e)? {
                            frame.tag = Some(v);
                        }
                    }
                }
                "w:sdtContent" => {
                    if let Some(frame) = stack.last_mut() {
                        frame.content_start = Some(cur);
                    }
                }
                _ => {}
            },
            // 本模板全部 <w:tag …/> 都是自闭合，走 Empty 不走 Start；Start 分支兜底（见上）
            Event::Empty(e) => {
                if e.name().as_ref() == "w:tag" {
                    if let Some(frame) = stack.last_mut() {
                        if let Some(v) = tag_val(&e)? {
                            frame.tag = Some(v);
                        }
                    }
                }
                // Event::Empty(w:sdt) 理论不出现，忽略
            }
            Event::End(e) => match e.name().as_ref() {
                "w:sdtContent" => {
                    if let Some(frame) = stack.last_mut() {
                        frame.content_end = Some(prev);
                    }
                }
                "w:sdt" => {
                    if let Some(frame) = stack.pop() {
                        if let (Some(tag), Some(content_start), Some(content_end)) =
                            (frame.tag, frame.content_start, frame.content_end)
                        {
                            ranges.push(SdtRange {
                                tag,
                                block_start: frame.block_start,
                                block_end: cur,
                                content_start,
                                content_end,
                            });
                        }
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(ranges)
}

/// 按 tag 精确匹配查找锚点（首个命中）
pub fn find<'a>(ranges: &'a [SdtRange], tag: &str) -> Option<&'a SdtRange> {
    ranges.iter().find(|r| r.tag == tag)
}

/// 锚点内容区原文（不含 `<w:sdtContent>` / `</w:sdtContent>` 标签本身）
pub fn content<'a>(xml: &'a str, r: &SdtRange) -> &'a str {
    &xml[r.content_start..r.content_end]
}

/// 锚点整块原文（含 `<w:sdt …>…</w:sdt>`）
pub fn block<'a>(xml: &'a str, r: &SdtRange) -> &'a str {
    &xml[r.block_start..r.block_end]
}

/// 应用若干替换（按 start 降序处理，避免偏移漂移）。
/// 区间越界或相互重叠 → `Err(BidError)`；调用方必须 `?`（唯一签名，`-> String` 旧写法作废）
pub fn apply_edits(xml: &str, edits: &mut [(usize, usize, String)]) -> BidResult<String> {
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out = xml.to_owned();
    let mut prev_start = xml.len() + 1;
    for &(start, end, ref repl) in edits.iter() {
        if start > end || end > xml.len() {
            return Err(BidError::new(format!(
                "编辑区间 ({start}..{end}) 越界（xml 长度 {}）",
                xml.len()
            )));
        }
        if !xml.is_char_boundary(start) || !xml.is_char_boundary(end) {
            return Err(BidError::new(format!(
                "编辑区间 ({start}..{end}) 落在 UTF-8 字符中间"
            )));
        }
        if end > prev_start {
            return Err(BidError::new(format!(
                "编辑区间 ({start}..{end}) 与前一区间重叠"
            )));
        }
        out.replace_range(start..end, repl);
        prev_start = start;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 单测（规格 §6.1 原文 5 条）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use super::super::template_gen::{
        template_document_xml, ALL_TAGS, TAG_CHAPTER, TAG_CHAPTER_BODY, TAG_CHAPTERS,
        TAG_CHAPTER_TITLE,
    };

    /// 1. 对 template_gen::template_document_xml() 建索引 → 恰好 9 条，tag 集合 == ALL_TAGS
    #[test]
    fn nine_sdts_indexed_matching_all_tags() {
        let xml = template_document_xml();
        let ranges = index_sdt(&xml).unwrap();
        assert_eq!(ranges.len(), ALL_TAGS.len());
        let mut found: Vec<&str> = ranges.iter().map(|r| r.tag.as_str()).collect();
        found.sort_unstable();
        let mut expected: Vec<&str> = ALL_TAGS.to_vec();
        expected.sort_unstable();
        assert_eq!(found, expected);
    }

    /// 2. 区间序关系 + 6/15 字节切片断言
    #[test]
    fn ranges_are_ordered_and_anchored() {
        let xml = template_document_xml();
        let ranges = index_sdt(&xml).unwrap();
        for r in &ranges {
            assert!(r.content_start <= r.content_end, "tag {}", r.tag);
            assert!(r.block_start < r.content_start, "tag {}", r.tag);
            assert!(r.content_end < r.block_end, "tag {}", r.tag);
            assert_eq!(&xml[r.block_start..r.block_start + 6], "<w:sdt");
            assert_eq!(
                &xml[r.content_end..r.content_end + 15],
                "</w:sdtContent>"
            );
        }
    }

    /// 3. 嵌套正确：BID_CHAPTER 内容含 BID_CHAPTER_TITLE/BID_CHAPTER_BODY；BID_CHAPTERS 含 BID_CHAPTER
    #[test]
    fn nesting_is_correct() {
        let xml = template_document_xml();
        let ranges = index_sdt(&xml).unwrap();
        let chapter = find(&ranges, TAG_CHAPTER).unwrap();
        let chapters = find(&ranges, TAG_CHAPTERS).unwrap();
        let chapter_content = content(&xml, chapter);
        assert!(chapter_content.contains(TAG_CHAPTER_TITLE));
        assert!(chapter_content.contains(TAG_CHAPTER_BODY));
        assert!(content(&xml, chapters).contains(TAG_CHAPTER));
    }

    /// 4. apply_edits 两处编辑（前面的先改、偏移不漂移）+ 越界/重叠 → Err
    #[test]
    fn apply_edits_two_edits_no_drift() {
        let xml = "abcdefghij";
        let mut edits = vec![
            (2usize, 4usize, "XY".to_string()),
            (7usize, 8usize, "Z".to_string()),
        ];
        let out = apply_edits(xml, &mut edits).unwrap();
        assert_eq!(out, "abXYefgZij");

        // 越界 → Err
        let mut out_of_range = vec![(5usize, 99usize, "X".to_string())];
        assert!(apply_edits(xml, &mut out_of_range).is_err());
        // 重叠 → Err
        let mut overlapping = vec![
            (2usize, 6usize, "X".to_string()),
            (4usize, 5usize, "Y".to_string()),
        ];
        assert!(apply_edits(xml, &mut overlapping).is_err());
    }

    /// 5. 不含 sdt 的 XML → 索引返回空 vec，不 panic
    #[test]
    fn xml_without_sdt_indexes_empty() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document><w:body><w:p><w:r><w:t>无锚点</w:t></w:r></w:p></w:body></w:document>"#;
        let ranges = index_sdt(xml).unwrap();
        assert!(ranges.is_empty());
    }
}
