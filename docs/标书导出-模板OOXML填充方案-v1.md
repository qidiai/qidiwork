# 标书导出：Rust 模板 + OOXML 填充方案 v1

**日期**：2026-11-14  
**状态**：方案（P0a/P0b 前置设计，未开始实现）  
**决策**：导出引擎走 **B 路线——Word 模板定义版式，Rust 用 `zip + quick-xml` 填充内容与图片**。不引入 Python 运行时，不从零实现 Word 布局引擎。

---

## 0. 结论

1. 版式（页边距、字号、行距、标题颜色、表格边框/底纹、图桌、图注、页眉页脚、目录域）**全部由 `.docx` 模板定义**，代码只引用样式 ID（`w:pStyle`），不重建样式。
2. 内容填充用 **内容控件（`w:sdt`）+ 片段拼接**：标量替换、章节循环、表格行克隆、图片 relationship 注入。
3. 复用 qidiwork 现有依赖：`zip 3`、`quick-xml 0.41`、`image 0.25.9`；新增零运行时依赖。
4. 验收以 **真实 Word/WPS/LibreOffice 渲染**为准：页数、边距、图片完整性、图注数量、表格跨页，不以“字符数÷每页字数”估算（R-025）。
5. 先做 P0a 两日 spike：模板 v0 + 填充器最小闭环 + 一份暗标样本；通过后再做 P0b。

---

## 1. 背景与目标

### 1.1 背景

- 审计结论：`md2docx.py` 无页边距/行距/颜色/图注/图桌能力，且依赖 Python + python-docx；qidiwork 安装包不带 Python（审计 F1–F3）。
- 目标不是“能生成 docx”，而是“**导出件与预览/模板版式一致**”，并能在用户机器上零额外依赖运行。

### 1.2 目标（P0a/P0b）

- 输入：一份结构化标书（章节 + 表格 + 图片/图表）。
- 输出：`.docx`，满足：
  - A4 竖版，四边 1.2cm；宋体 12pt、1.5 倍行距、两端对齐、首行缩进 2 字符；
  - 章 15pt 加粗居中 `#253D57`；节 14pt 加粗居中 `#315276`；三/四级 12–14pt 黑色；
  - 表格 0.5pt 边框（外 `#74A6CC` / 内 `#B0D0E8`），表头 `#4877AC` 白字，首列 `#E6F2F9`；
  - 图桌三种：三栏搜图（3×1）、单图+图注（工程图）、宽条流程（流程图）；
  - 图注 12pt 加粗居中；AI 图带来源标注；
  - 目录域、页码、页眉页脚由模板提供；
  - 每章另起页。
- 验收：用真实样本渲染，页数与目标偏差 ≤1 页；图片无吞图/错位；图注计数与图片计数一致。

### 1.3 非目标（本轮）

- 不做 Word 级所见即所得编辑器；
- 不做 PDF 生成（用 Word/LibreOffice 转）；
- 不做图表绘制引擎（P2 再评估 Rust 绘图或复用现有脚本）；
- 不做多用户协同/模板市场。

---

## 2. 总体架构

```
Vue (gui/src)
  BidWorkspace / ChapterEditor / FigureTable / LayoutSettings
        │ Tauri invoke
        ▼
Tauri commands (gui/src-tauri/src/commands.rs)
  bid_project_open / bid_project_save
  bid_chapter_get / bid_chapter_save
  bid_figure_add / bid_figure_update
  bid_export_docx / bid_check_docx
        │
        ▼
Rust core (gui/src-tauri/src/office/bid/ 或独立 crate cf-bid)
  model.rs      # DocStyle / BidProject / Outline / Chapter / Figure
  template.rs   # 模板加载、sdt 锚点定位、片段库
  fill.rs       # 标量替换、章节循环、表格行克隆、图片注入
  docx.rs       # zip 读写、quick-xml、rels/media、drawing XML
  verify.rs     # 导出后 XML 断言（边距/字号/颜色/图片数/图注数）
```

**边界**：`office/bid` 只处理“结构化标书 → docx”；产物登记仍走 `manifest.json`，但 `bid/` 下的中间文件不登记为产物，最终 `.docx` 由 `card.py`/命令登记。

---

## 3. 模板规范（唯一版式真值源）

### 3.1 模板文件

```
gui/src-tauri/resources/templates/
  bid-engineering-blue-v1.docx     # 工程·经典蓝
  bid-service-blue-v1.docx         # 服务类（后续）
  bid-business-official-v1.docx    # 商务标（后续）
```

模板由 Word/WPS 制作，**冻结后只改版本号，不在代码里改样式**。模板内定义：

| 样式 ID | 用途 | 参数 |
|---|---|---|
| `BidBody` | 正文 | 宋体 12pt，1.5 倍行距，两端对齐，首行缩进 2 字符 |
| `BidTitle1` | 章标题 | 15pt 粗，居中，`#253D57`，`第%1章` |
| `BidTitle2` | 节标题 | 14pt 粗，居中，`#315276`，`第%2节` |
| `BidTitle3/4` | 三级/四级 | 14/12pt 粗，左对齐，黑色 |
| `BidTableHeader` | 表头 | 白字 12pt 粗居中，底 `#4877AC` |
| `BidTableBody` | 表体 | 12pt，居中 |
| `BidCaption` | 图注 | 12pt 粗，居中 |
| `BidFigureCell` | 图桌单元格 | 无边框，2px padding |
| `BidTOC` | 目录 | 目录域 |
| `BidHeader/BidFooter` | 页眉页脚 | 模板定义 |

### 3.2 页面与分节

- `w:sectPr`：A4（`w:w=11906 w:h=16838`），`w:pgMar top/bottom/left/right=680`（1.2cm），页眉/页脚距按模板。
- 每章另起页：章标题段落设 `w:pageBreakBefore`，或填充时插入 `<w:br w:type="page"/>`。
- 目录域：`w:fldSimple w:instr=" TOC \o \"1-3\" \h \z \u "`；`w:updateFields w:val="true"` 使 Word 打开时更新。
- 页码：页脚 `PAGE`/`NUMPAGES` 域，模板内置。

### 3.3 锚点（内容控件）

模板内放置 `w:sdt`，用 `w:tag` 标识，填充器按 tag 查找并替换 `sdtContent`：

| tag | 用途 | 填充方式 |
|---|---|---|
| `BID_DOC_TITLE` | 文档标题 | 标量 |
| `BID_TOC` | 目录 | 保留域，不动 |
| `BID_CHAPTERS` | 章循环区 | 复制 `BID_CHAPTER` 片段 N 次 |
| `BID_CHAPTER_TITLE` | 章标题 | 标量（保留样式） |
| `BID_CHAPTER_BODY` | 章正文 | 由块片段拼接（段落/表格/图） |
| `BID_TABLE_ROWS` | 表格行区 | 复制 `BID_TABLE_ROW` 片段 N 次 |
| `BID_FIGURE_3UP` | 三栏图桌 | 生成 1 行 3 单元格，每格图片 |
| `BID_FIGURE_SINGLE` | 单图+图注 | 图片段落 + `BidCaption` 段落 |
| `BID_FIGURE_WIDE` | 宽条流程 | 全宽图片 + `BidCaption` |
| `BID_FIGURE_CAPTION` | 图注 | 标量 |

> 用 `w:sdt` 而不是裸 `{{...}}` 文本：Word 会把占位文本拆成多个 run，裸文本替换易漏；`sdt` 是结构锚点，定位稳定。

---

## 4. 数据契约

### 4.1 DocStyle（`docstyle.toml` / Rust serde）

```toml
[page]
paper = "A4V"
margin_top_cm = 1.2
margin_bottom_cm = 1.2
margin_left_cm = 1.2
margin_right_cm = 1.2

[body]
font = "宋体"
size_pt = 12
line_spacing = 1.5
align = "justify"
first_line_indent_chars = 2

[heading1]
size_pt = 15
bold = true
align = "center"
color = "253D57"
template = "第%1章 "

[heading2]
size_pt = 14
bold = true
align = "center"
color = "315276"
template = "第%2节 "

[table]
border_pt = 0.5
outer_color = "74A6CC"
inner_color = "B0D0E8"
header_fill = "4877AC"
header_color = "FFFFFF"
first_col_fill = "E6F2F9"
cant_split = true

[figure]
layouts = ["3up", "single", "wide"]
caption_size_pt = 12
caption_align = "center"
ai_label = "（AI 生成示意图）"
```

### 4.2 BidProject（`project.toml`）

```toml
id = "proj-xxx"
title = "槐林镇2026年度和美乡村省级中心村建设项目"
industry = "engineering"
expert_pack = "engineering-general"
style_id = "bid-engineering-blue-v1"
word_target = 49100
page_target = 48
tender_files = ["招标文件/xxx.pdf"]
```

### 4.3 Outline（`outline.json`）

```json
{
  "chapters": [
    {
      "id": "ch-1",
      "title": "工程概况",
      "score_points": ["工程概况"],
      "page_budget": 7,
      "relation": "sequence",
      "children": [
        {"id": "ch-1-1", "title": "项目概况", "relation": "sequence", "children": []}
      ]
    }
  ]
}
```

> `relation: "sequence" | "alternative"` 是图文 QC 的基础字段；`alternative` 章节不得被画成顺序流程（审计 R-031）。

### 4.4 Chapter（`chapters/ch-1-1.md` 或 `.json`）

块序列：

```json
[
  {"type": "paragraph", "style": "BidBody", "text": "..."},
  {"type": "table", "header": ["工序节点", "施工方法及控制参数"], "rows": [["...", "..."]]},
  {"type": "figure", "figure_id": "fig-001"},
  {"type": "chart", "chart_id": "chart-001"}
]
```

### 4.5 Figure（`figures/fig-001.json`）

```json
{
  "id": "fig-001",
  "kind": "search",
  "caption": "水塘清淤后塘底整平",
  "aspect": "1:1",
  "source": "assets/fig-001.jpg",
  "ai_generated": true,
  "prompt": "纪实风格…",
  "content_ref": "ch-2-4-3",
  "label": "（AI 生成示意图）"
}
```

`kind`：`search`（三栏）、`agentimg`（单图+图注）、`flow`（宽条）、`chart`（宽条/单图）。

---

## 5. 填充器设计

### 5.1 处理流程

```
1. 读模板 docx（zip）→ 取 word/document.xml、styles.xml、rels、media
2. 解析 document.xml（quick-xml），建立 sdt tag → 区间 的索引
3. 展开章循环：复制 BID_CHAPTER 片段，替换标题，展开正文块
4. 块渲染：
   - paragraph → <w:p><w:pPr><w:pStyle w:val="BidBody"/>…</w:p>
   - table → <w:tbl> + tblPr（边框/底纹）+ 行克隆
   - figure → 按 kind 生成 3up/single/wide
   - chart → 单图/宽条（图片来自预渲染 PNG）
5. 图片注入：
   - 写 word/media/imageN.ext
   - 在 word/_rels/document.xml.rels 增加 Relationship
   - 在 [Content_Types].xml 增加 Default/Override（如 png/jpeg）
   - 生成 <w:drawing><wp:inline>…<a:blip r:embed="rIdN"/>…</wp:inline></w:drawing>
   - wp:docPr id 唯一、name/descr 写图注
6. 写回 zip（保留 [Content_Types].xml、styles、theme、header/footer）
7. verify.rs：解析输出，断言边距/字号/颜色/表格数/图片数/图注数
```

### 5.2 片段库

```
templates/fragments/
  paragraph.xml
  table.xml
  table_row.xml
  figure_3up.xml
  figure_single.xml
  figure_wide.xml
  caption.xml
```

片段用 `{{STYLE_ID}}`、`{{TEXT}}`、`{{RID}}`、`{{CAPTION}}` 等占位；Rust 做 **XML 转义**后替换。片段自身不依赖 Word run 拆分，因为是我们生成的整段 XML。

### 5.3 OOXML 关键参数

| 项 | XML |
|---|---|
| A4 | `<w:pgSz w:w="11906" w:h="16838"/>` |
| 1.2cm 边距 | `<w:pgMar w:top="680" w:bottom="680" w:left="680" w:right="680"/>` |
| 1.5 倍行距 | `<w:spacing w:line="360" w:lineRule="auto"/>` |
| 首行缩进 2 字符 | `<w:ind w:firstLine="480"/>` |
| 12pt | `<w:sz w:val="24"/>`（半磅） |
| 15pt | `<w:sz w:val="30"/>` |
| 章标题色 | `<w:color w:val="253D57"/>` |
| 0.5pt 边框 | `<w:sz w:val="4"/>`（八分之一磅） |
| 表头底纹 | `<w:shd w:fill="4877AC"/>` |
| 首列底纹 | `<w:shd w:fill="E6F2F9"/>` |
| 行不跨页 | `<w:trPr><w:cantSplit/></w:trPr>` |
| 图片关系 | `<Relationship Id="rIdN" Type=".../image" Target="media/imageN.png"/>` |
| 图片引用 | `<a:blip r:embed="rIdN"/>` |
| 图注 | `<w:p><w:pPr><w:pStyle w:val="BidCaption"/></w:pPr><w:r><w:t>图注</w:t></w:r></w:p>` |
| 目录域 | `<w:fldSimple w:instr=" TOC \o &quot;1-3&quot; \h \z \u "/>` |
| 打开时更新域 | `<w:updateFields w:val="true"/>`（settings.xml） |

### 5.4 图片与图桌

- **3up**：3 列表格，每列宽 33.33%，无边框；单元格内 `wp:inline` 图片；图注取 `alt` 或 `BidCaption`（按模板选择）。三张不足时留空单元格，不拉伸。
- **single**：居中图片 + 下一段 `BidCaption`；图片宽度按内容区宽度与宽高比计算（EMU：1cm=360000）。
- **wide**：全宽图片 + `BidCaption`；适合 3081×188 流程图长条。
- **宽高比**：`1:1 / 4:3 / 2:1` 由 `Figure.aspect` 决定；不足时 `object-fit` 由 Word 裁切/留白策略决定（模板中 `wp:inline` 固定 extent，不裁切原图）。
- **AI 标注**：`ai_generated=true` 时，图注追加 `label`（默认“（AI 生成示意图）”）；同时写 `wp:docPr descr`。是否显示由项目 `figure.ai_label` 配置控制，默认显示。

### 5.5 图表（P2）

- 本轮只接“预渲染 PNG”；`charts/chart-001.json` 提供 `render_path/width/height/chart_text`。
- P2 再评估：Rust 绘图（`plotters`）或继续用现有脚本（需解决 Python 依赖）。
- 图表插入后必须验“印刷有效字号”（R-030）与文字重叠（R-032）。

---

## 6. 验收

### 6.1 Golden 语料

- `gui/spike-docx/samples/施工组织设计技术暗标.docx`
- `gui/spike-docx/samples/附表四横道图.docx`
- `gui/spike-docx/samples/附表五施工总平面图暗标修复.docx`
- 额外 2 份真实合肥样本（脱敏）

### 6.2 门禁（P0a/P0b）

| 门禁 | 方法 | 阈值 |
|---|---|---|
| 页数 | Word/WPS/LibreOffice 转 PDF 后计数 | 与目标偏差 ≤1 页（R-025） |
| 边距/纸张 | unzip 后断言 `w:pgSz`/`w:pgMar` | 精确匹配 |
| 字号/行距/缩进 | 断言 `w:sz`/`w:spacing`/`w:ind` | 精确匹配 |
| 标题颜色 | 断言 `w:color` | 精确匹配 |
| 表格边框/底纹 | 断言 `w:tblBorders`/`w:shd` | 精确匹配 |
| 图片完整性 | 渲染后逐页截图 + 图片计数 | 无吞图/错位（R-022） |
| 图注计数 | 图片数 = 图注数 | 一致 |
| 图表有效字号 | 按插入宽度反算 | ≥8pt（R-030） |
| 文字重叠 | 渲染器 bbox 两两检测 | 0 重叠（R-032） |
| 图文一致 | caption 命中 content 关键词；alternative 不线性化 | 100%（R-031） |

### 6.3 自动断言（示例）

```bash
unzip -p out.docx word/document.xml > /tmp/document.xml
rg -o 'w:pgMar[^/]*' /tmp/document.xml
rg -c '<w:drawing>' /tmp/document.xml
rg -c 'BidCaption' /tmp/document.xml
```

> 断言脚本放 `gui/tests/bid-export/`；每个门禁一条可执行断言（R-031）。

---

## 7. 分阶段与工作量

| 阶段 | 内容 | 工期 | 交付 |
|---|---|---|---|
| **P0a** | 模板 v0（样式/页边距/页眉页脚/目录域）+ 填充器最小闭环（标题/段落/表格/单图）+ 1 份暗标样本 | 2 天 | `bid_export_docx` 命令 + 验收截图 |
| **P0b** | DocStyle + 三栏图桌 + 图注 + 宽条流程 + golden 断言 | 3 天 | 4 份样本全绿 |
| **P1** | bid 数据面（project/outline/chapter/figure）+ 导入 + 三栏工作台 + ACP 工具 | 5–7 天 | 1 章 + 3 图端到端 |
| **P2** | 图表接入 + 有效字号/重叠/图文一致断言 | 7–10 天 | 4 条断言全绿 |
| **P3** | 专家包 + 场景工具面 + 守卫 | 7–10 天 | 单测 + 真实项目回归 |

---

## 8. 风险与对策

| 风险 | 对策 |
|---|---|
| 模板漂移（Word/WPS 差异） | 模板版本化；每次改样式重生成 golden；固定验收渲染器（优先 WPS/Word，记录版本） |
| `w:sdt` 被 Word 保存后改写 | 模板冻结；填充器只读模板不改模板；每次导出从模板副本开始 |
| 图片 relationship 冲突 | media 文件名唯一（fig id），rels 去重；[Content_Types] 补扩展名 |
| 固定行距吞图（R-022） | 图片段落使用 `w:spacing lineRule="atLeast"` 或单倍行距；逐页截图抽检 |
| 页数估算偏差（R-025） | 一律渲染器实测；建立“字数/表格/图片 → 页数”校准表 |
| 图表字号/重叠（R-030/R-032） | 按插入宽度反算有效字号；bbox 重叠检测 |
| 目录域不更新 | `w:updateFields=true`；文档注明 Word 打开时更新 |
| 中文字体缺失 | 模板写 `w:eastAsia="宋体"`；导出不嵌字体（许可）；预览用字体回退，不作为验收 |
| 专家包泄露 | 专家包不进公开仓库；本地加密/权限；导出件不含提示词 |

---

## 9. 交付物清单

- `gui/src-tauri/resources/templates/bid-engineering-blue-v1.docx`
- `gui/src-tauri/resources/templates/fragments/*.xml`
- `gui/src-tauri/src/office/bid/{model,template,fill,docx,verify}.rs`
- `gui/src-tauri/src/commands.rs`：`bid_export_docx` / `bid_check_docx`
- `gui/src/components/bid/FigureTable.vue`、`ChapterEditor.vue`
- `gui/tests/bid-export/`：4 份 golden + 断言脚本
- `docs/标书导出-模板OOXML填充方案-v1.md`（本文）

---

## 附录 A：模板制作检查清单

1. 页面：A4 竖版、四边 1.2cm、页眉页脚距。
2. 样式：`BidBody/BidTitle1-4/BidTableHeader/BidTableBody/BidCaption/BidFigureCell` 全部存在且命名精确。
3. 目录域：`TOC \o "1-3" \h \z \u`；`updateFields=true`。
4. 页脚：`PAGE`、`NUMPAGES`。
5. 锚点：`BID_*` 内容控件齐全，`w:tag` 与代码常量一致。
6. 表格：3 列图桌样例、表头底纹、首列底纹、`cantSplit`。
7. 图注：`BidCaption` 样式，居中 12pt 粗。
8. 保存为 `.docx`（不是 `.dotx`），关闭“兼容模式”。

## 附录 B：依赖与命令

- 现有依赖：`zip 3`、`quick-xml 0.41`、`image 0.25.9`（无需新增运行时）。
- 可选：`roxmltree`（只读 DOM 便于断言）、`plotters`（P2 图表）。
- 测试：`cargo test -p cf-bid`（或 `gui/src-tauri` 对应模块）；`cargo clippy --all-targets`。
- 渲染验收：WPS/Word 打开 → 另存 PDF → 计页；或 LibreOffice `--headless --convert-to pdf`。

---

**适用条目**：R-025 / R-022 / R-030 / R-032 / R-031  
（本方案把每条纪律落成可执行门禁；未触发条目见审计报告。）