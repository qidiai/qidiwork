# DOCX 导入（bid_import）

把已有 `.docx`（我们导出的、或旧 python-docx 稿）转回 Markdown，供 `bid_export --md` 重新出稿。

## 用法

```bash
cd G:/qidiwork/gui/src-tauri

# 默认把图片抽到 <md 同目录>/assets/
cargo run --bin bid_import -- --input 旧稿.docx --out 新稿.md

# 指定素材目录
cargo run --bin bid_import -- --input 旧稿.docx --out 新稿.md --assets ./素材
```

成功输出一行 JSON 摘要 + `IMPORT OK`：

```json
{"path":"…/新稿.md","assets":"…/assets","chapters":3,"headings":9,"images":5,"paragraphs":6,"tables":1}
```

失败：`ERROR <原因>`，退出码 **2**（文件不存在、docx 损坏、包内缺 `word/document.xml` 等）。

## 映射规则

| docx | Markdown |
|---|---|
| `BidTitle1`（**首个**）/ `Heading1` / `w:val="1"` | `# 书名` |
| `BidTitle1`（后续） | `## 章标题` |
| `BidTitle2` / `Heading2` / `w:val="2"` | `### 节` |
| `BidTitle3` / `Heading3` / `w:val="3"` | `#### 小节` |
| `BidTitle4` | `#####` |
| `BidCaption` 紧跟在图片后 | 作为该图的图注 `![图注](assets/img-n.png)` |
| `BidCaption` 其它位置 / `BidBody` / 无样式段落 | 普通段落 |
| `<w:tbl>` | GFM 表格（首行表头） |
| `<w:drawing>` | 抽图到 `assets/img-<n>.<ext>`，md 写 `![](assets/img-n.png)` |
| 页眉 / 页脚 / TOC 域结果 / `w:sdt` 包装 | 跳过（只取内容） |

## 已知限制

- 只做**结构**迁移：粗体/斜体/字体/颜色/字号不保留（导出时由模板统一定版式）。
- 合并单元格（`gridSpan` / `vMerge`）按原单元格逐个输出，会丢合并语义。
- 图片按文档出现顺序编号，文件名不与原包内名对应。
- 不支持 `.doc`（旧二进制格式）——先用 WPS/Word 另存为 `.docx`。
- 旧 python-docx 稿的 `Heading1/2/3` 已覆盖；自定义样式名（非上述四种）按普通段落处理。

## 回环

`md → docx → md` 的结构是稳定的（章/节/表/图数量一致），但**文本会被重排**（段落合并、表格对齐），不要用它做字节级 diff；用于"把旧稿接进新流水线"。
