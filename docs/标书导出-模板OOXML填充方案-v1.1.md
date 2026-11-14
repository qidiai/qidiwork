# 标书导出：Rust 模板 + OOXML 填充方案 v1.1

**日期**：2026-11-14  
**状态**：方案（按 2026-11-14 审计意见修订；P0a/P0b 前置设计，未开始实现）  
**决策**：导出引擎走 **B 路线——Word 模板定义版式，Rust 用 `zip + quick-xml` 填充内容与图片**。安装包**零外部运行时**（不依赖 Python / Node / pip / npm）。

> v1.1 修订点：验收断言分层（styles.xml vs document.xml）、修正事实性错误（md_to_docx.py/.mjs 与 md2docx.py 的区别）、补齐路线取舍记录、明确 3up 图注口径、页数门禁降级、依赖/命令注册/模板分发/manifest 单写者约束、模板 v0 程序化生成决策。

---

## 依据文件（可追溯）

| 依据 | 路径 |
|---|---|
| 审计准则库（R-025/R-022/R-030/R-032/R-031 定义） | `G:\qidicode\docs\audit-rubrics\RUBRICS.md` |
| 本方案审计报告（F1–F3 结论） | `G:\qidiwork\docs\审计-链企Buddy优点整合qidiwork方案-2026-11-13.md`（随本方案一并提交） |
| 真实样本语料 | `gui/spike-docx/samples/施工组织设计技术暗标.docx` 等 3 份 |
| 现有生成器事实 | `gui/src-tauri/resources/qidiwork-skills-bundle.zip` → `office-tools/scripts/md_to_docx.py/.mjs`；`~/.qidi/skills/bid-write/scripts/md2docx.py`（开发机独有，**不在随包**） |

---

## 0. 结论

1. 版式（页边距、字号、行距、标题颜色、表格边框/底纹、图桌、图注、页眉页脚、目录域）**全部由 `.docx` 模板定义**，代码只引用样式 ID（`w:pStyle`），不重建样式。
2. 内容填充用 **内容控件（`w:sdt`）+ 片段拼接**：标量替换、章节循环、表格行克隆、图片 relationship 注入。
3. **零外部运行时**：不依赖 Python/Node/pip/npm；Rust 侧使用 `zip` + `quick-xml` + `image`，随二进制交付。
4. 验收分两层：**`styles.xml` 断样式参数真值；`document.xml` 断样式引用、页面设置、图片/图注计数**。页数由真实渲染器实测（R-025），P0a/P0b 只记录基线，页数目标门禁放 P1。
5. 先做 P0a 两日 spike（范围收窄：样式/边距/锚点/单图闭环）；目录域与页眉页脚降级到 P0b。

---

## 1. 背景与目标

### 1.1 背景（按仓库事实修正）

- 随包 `office-tools` 实际有两个生成器：
  - `md_to_docx.py`：**pandoc 包装器**（不是 python-docx），依赖用户机有 pandoc；
  - `md_to_docx.mjs`：**Node `docx` 库**，已能零 Python 生成标题/表格/图片，但依赖用户机有 Node + npm 包。
- 开发机 `~/.qidi/skills/bid-write/scripts/md2docx.py` 是另一份 python-docx 脚本，**不在 `qidiwork-skills-bundle.zip`**；它不能代表随包能力。
- 因此“零 Python”只对 `.py` 成立；**对 `.mjs` 不成立**（Node 也是外部运行时）。
- 选 Rust 的真正理由：**安装包零外部运行时** + Tauri 命令一体化 + 既有 manifest/命令链复用 + XML 级可控（样式/图片/目录域）。
- 审计 F1–F3（详见审计报告）：
  - F1 随包技能缺 `bid-write` 等关键件；
  - F2 `md2docx.py` 无页边距/行距/颜色/图注/图桌；
  - F3 无 Python 运行时。

### 1.2 目标（P0a/P0b）

- 输入：结构化标书（章节 + 表格 + 图片/图表）。
- 输出：`.docx`，满足：
  - A4 竖版，四边 1.2cm；宋体 12pt、1.5 倍行距、两端对齐、首行缩进 2 字符；
  - 章 15pt 加粗居中 `#253D57`；节 14pt 加粗居中 `#315276`；三/四级 12–14pt 黑色；
  - 表格 0.5pt 边框（外 `#74A6CC` / 内 `#B0D0E8`），表头 `#4877AC` 白字，首列 `#E6F2F9`；
  - 图桌三种：三栏搜图、单图+图注、宽条流程；
  - 图注 12pt 加粗居中；AI 图带来源标注；
  - 目录域、页码、页眉页脚由模板提供；
  - 每章另起页。
- 验收：真实渲染无错、版式断言全绿、页数基线可复现（P0b）；页数目标在 P1 验收。

### 1.3 非目标（本轮）

- 不做 Word 级所见即所得编辑器；
- 不做 PDF 生成（用 Word/WPS/LibreOffice 转）；
- 不做图表绘制引擎（P2 再评估 Rust 绘图或复用现有脚本）；
- 不做多用户协同/模板市场。

### 1.4 路线取舍决策记录（A/B/C/D）

| 维度 | A. `docx-rs` 从零生成 | **B. 模板 + OOXML 填充（选定）** | C. 扩展 Node 生成器 `md_to_docx.mjs` | D. 随包 Python |
|---|---|---|---|---|
| 外部运行时 | 无 | **无** | **需 Node + npm 包** | 需 Python + pip |
| 版式保真 | 中（需手写 OOXML） | **高（Word 模板定义样式）** | 中（库 API 受限，复杂版式仍要 XML） | 高 |
| 与 Tauri/命令链 | 好 | **好** | 差（需 Node 子进程/打包） | 差（需运行时探测） |
| 可测试性 | 好 | **好（XML 断言 + golden）** | 中（Node 测试链另起） | 中 |
| 开发量 | 中 | 中高 | 低（已有脚本） | 低 |
| 结论 | 备选（简单文档） | **主链路** | 不选：不满足“零外部运行时”，且样式控制弱 | 不选：安装包/兼容风险 |

**为何不扩展 `md_to_docx.mjs`**：它能零 Python 生成标题/表格/图片，但（1）仍要求用户机有 Node + npm 包，不满足“安装包零外部运行时”；（2）`docx` 库对页边距/单元格底纹/图桌/目录域的控制不如直接操作 OOXML；（3）无法复用 Tauri 命令与 `manifest`/`office-event` 链。保留它作为“无模板简单文档”的备选，不作为标书主链路。

---

## 2. 总体架构

```
Vue (gui/src)
  BidWorkspace / ChapterEditor / FigureTable / LayoutSettings
        │ Tauri invoke
        ▼
Tauri commands (gui/src-tauri/src/commands.rs + lib.rs generate_handler!)
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
  verify.rs     # 导出后 XML 断言（分层：styles.xml / document.xml）
```

**边界**：`office/bid` 只处理“结构化标书 → docx”；产物登记仍走 `manifest.json`。**`bid_export_docx` 只写 `.docx` 并返回路径，不写 manifest**；登记由 `card.py`（唯一写者，方案 v2 D3）或 agent 调用完成，避免破坏单写者约束。

---

## 3. 模板规范（唯一版式真值源）

### 3.1 模板文件

```
gui/src-tauri/resources/templates/
  bid-engineering-blue-v1.docx     # 工程·经典蓝（P0a）
  bid-service-blue-v1.docx         # 服务类（后续）
  bid-business-official-v1.docx    # 商务标（后续）
```

模板**程序化生成 v0**（见 §10），Word/WPS 只复核、不手编；冻结后只改版本号。模板内定义：

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

- `w:sectPr`：A4（`w:w=11906 w:h=16838`），`w:pgMar top/bottom/left/right=680`（1.2cm）。
- 每章另起页：章标题段落设 `w:pageBreakBefore`，或填充时插入 `<w:br w:type="page"/>`。
- 目录域：`w:fldSimple w:instr=" TOC \o \"1-3\" \h \z \u "`；`w:updateFields w:val="true"` 使 Word 打开时更新。
- 页码：页脚 `PAGE`/`NUMPAGES` 域，模板内置。

### 3.3 锚点（内容控件）

模板内放置 `w:sdt`，用 `w:tag` 标识；填充器按 tag 查找并替换 `sdtContent`：

| tag | 用途 | 填充方式 |
|---|---|---|
| `BID_DOC_TITLE` | 文档标题 | 标量 |
| `BID_TOC` | 目录 | 保留域，不动 |
| `BID_CHAPTERS` | 章循环区 | 复制 `BID_CHAPTER` 片段 N 次 |
| `BID_CHAPTER` | 单章容器 | 章级片段（内含标题/正文锚点） |
| `BID_CHAPTER_TITLE` | 章标题 | 标量（保留样式） |
| `BID_CHAPTER_BODY` | 章正文 | 由块片段拼接（段落/表格/图） |
| `BID_TABLE_ROWS` | 表格行区 | 复制 `BID_TABLE_ROW` 片段 N 次 |
| `BID_TABLE_ROW` | 单行模板 | 行级片段 |
| `BID_FIGURE_3UP` | 三栏图桌 | 生成 1 行 3 单元格，每格图片+图注 |
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

> `relation: "sequence" | "alternative"` 是图文 QC 的基础字段；`alternative` 章节不得被画成顺序流程（R-031）。

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
  "caption_mode": "per_image",
  "aspect": "1:1",
  "source": "assets/fig-001.jpg",
  "ai_generated": true,
  "prompt": "纪实风格…",
  "content_ref": "ch-2-4-3",
  "label": "（AI 生成示意图）"
}
```

- `kind`：`search`（三栏）、`agentimg`（单图+图注）、`flow`（宽条）、`chart`（宽条/单图）。
- `caption_mode`：
  - `per_image`（默认）：每张图各带图注；3up = 3 图 3 注；
  - `per_group`：一组图共用一个图注；3up = 3 图 1 注。
- 门禁按 `caption_mode` 计算，不写死“图片数 = 图注数”。

---

## 5. 填充器设计

### 5.1 处理流程

```
1. 读模板 docx（zip）→ 取 word/document.xml、styles.xml、rels、media
2. 解析 document.xml（quick-xml），建立 sdt tag → 区间 的索引
3. 展开章循环：复制 BID_CHAPTER 片段，替换标题，展开正文块
4. 块渲染：
   - paragraph → <w:p><w:pPr><w:pStyle w:val="BidBody"/>…</w:p>
   - table → <w:tbl> + tblPr（边框/底纹）+ 行克隆（BID_TABLE_ROW）
   - figure → 按 kind 生成 3up/single/wide
   - chart → 单图/宽条（图片来自预渲染 PNG）
5. 图片注入：
   - 写 word/media/imageN.ext
   - 在 word/_rels/document.xml.rels 增加 Relationship
   - 在 [Content_Types].xml 增加 Default/Override（如 png/jpeg）
   - 生成 <w:drawing><wp:inline>…<a:blip r:embed="rIdN"/>…</wp:inline></w:drawing>
   - wp:docPr id 唯一、name/descr 写图注
6. 写回 zip（保留 [Content_Types].xml、styles、theme、header/footer）
7. verify.rs：分层断言（styles.xml 断参数；document.xml 断引用/页面/计数）
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
| 首行缩进 2 字符 | `<w:ind w:firstLineChars="200"/>`（Word 的“2 字符”语义；`w:firstLine="480"` 仅 12pt 近似，样式与断言统一用 `firstLineChars`） |
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

- **3up**：3 列表格，每列宽 33.33%，无边框；单元格内 `wp:inline` 图片；默认 `caption_mode=per_image`，**每格各带图注**；不足三张留空单元格，不拉伸。
- **single**：居中图片 + 下一段 `BidCaption`；图片宽度按内容区宽度与宽高比计算（EMU：1cm=360000）。
- **wide**：全宽图片 + `BidCaption`；适合 3081×188 流程图长条。
- **宽高比**：`1:1 / 4:3 / 2:1` 由 `Figure.aspect` 决定；`wp:inline` 固定 extent，不裁切原图。
- **AI 标注**：`ai_generated=true` 时，图注追加 `label`（默认“（AI 生成示意图）”）；同时写 `wp:docPr descr`。是否显示由项目配置控制，默认显示。

### 5.5 图表（P2）

- 本轮只接“预渲染 PNG”；`charts/chart-001.json` 提供 `render_path/width/height/chart_text`。
- P2 再评估：Rust 绘图（`plotters`）或继续用现有脚本（需解决 Node/Python 依赖）。
- 图表插入后必须验“印刷有效字号”（R-030）与文字重叠（R-032）。

---

## 6. 验收

### 6.1 Golden 语料（固定 4 份）

1. `gui/spike-docx/samples/施工组织设计技术暗标.docx`
2. `gui/spike-docx/samples/附表四横道图.docx`
3. `gui/spike-docx/samples/附表五施工总平面图暗标修复.docx`
4. 1 份真实合肥样本（脱敏后入库，P0b 前落实）

> P0b 交付口径统一为“4 份样本全绿”；不再写“3 份 + 额外 2 份”。

### 6.2 门禁（分层断言）

| 层 | 断言对象 | 方法 | 阈值 |
|---|---|---|---|
| `styles.xml` | 字号/行距/缩进/颜色/底纹/表格边框 | 解析 `w:style` 的 `w:sz`/`w:spacing`/`w:ind`/`w:color`/`w:shd`/`w:tblBorders` | 精确匹配 DocStyle |
| `document.xml` | 样式引用 | 断言 `w:pStyle w:val="BidTitle1/BidBody/...` 存在且数量符合预期 | 章节/段落数一致 |
| `document.xml` | 页面设置 | `w:pgSz`、`w:pgMar` | A4 / 680 twips |
| `document.xml` | 图片/图注 | `w:drawing` 计数、`BidCaption` 计数、`w:docPr descr` | 按 `caption_mode`：per_image 图=注；per_group 注=组 |
| `document.xml` | 分页 | 章标题 `w:pageBreakBefore` 或 `w:br type="page"` | 每章 1 处 |
| 渲染层 | 页数基线 | Word/WPS 更新域 → 导出 PDF → 计页 | P0a/P0b 只记录基线且可复现；P1 再设 ±1 页目标 |
| 渲染层 | 图片完整性 | 逐页截图/PDF 检查 | 无吞图/错位（R-022） |
| 渲染层 | 图表有效字号 | 按插入宽度反算 | ≥8pt（R-030） |
| 渲染层 | 文字重叠 | 渲染器 bbox 两两检测 | 0 重叠（R-032） |
| 逻辑层 | 图文一致 | caption 命中 content 关键词；alternative 不线性化 | 100%（R-031） |

### 6.3 自动断言（修正计数）

```bash
# 1) 解包
unzip -p out.docx word/styles.xml   > /tmp/styles.xml
unzip -p out.docx word/document.xml > /tmp/document.xml

# 2) styles.xml：样式参数真值
rg -o 'w:sz w:val="[0-9]+"' /tmp/styles.xml | sort | uniq -c
rg -o 'w:spacing w:line="[0-9]+"' /tmp/styles.xml
rg -o 'w:ind w:firstLineChars="[0-9]+"' /tmp/styles.xml
rg -o 'w:color w:val="[0-9A-Fa-f]{6}"' /tmp/styles.xml | sort | uniq -c

# 3) document.xml：样式引用 + 页面设置 + 计数
rg -o 'w:pStyle w:val="BidTitle[0-9]"' /tmp/document.xml | sort | uniq -c
rg -o 'w:pgSz[^/]*' /tmp/document.xml
rg -o 'w:pgMar[^/]*' /tmp/document.xml
rg -o '<w:drawing' /tmp/document.xml | wc -l          # 必须 -o + wc -l，不能 rg -c
rg -o 'w:pStyle w:val="BidCaption"' /tmp/document.xml | wc -l
```

> `rg -c` 计的是“命中行数”，document.xml 通常整篇一行，计数恒为 1；图片/图注计数必须 `rg -o … | wc -l` 或用 XML 解析。

### 6.4 目录域与页数

- **Word/WPS 路径**：打开文档 → 更新域（目录/页码）→ 导出 PDF → 计页。
- **LibreOffice headless 路径**：不更新 `TOC` 域；目录不展开会污染页数。若用 LO，必须单独标注“目录未展开”的偏差，不能作为最终页数验收。
- P0a/P0b 只记录“同一输入、同一渲染器、同一设置”的页数基线；页数目标（±1 页）在 P1 有字数/页数预算后再启用。

---

## 7. 分阶段与工作量

| 阶段 | 内容 | 工期 | 交付 |
|---|---|---|---|
| **P0a** | 模板 v0（**仅样式/边距/锚点**；目录域/页眉页脚降级到 P0b）+ 填充器最小闭环（标题/段落/表格/单图）+ 1 份暗标样本 | 2 天（spike，范围收窄） | `bid_export_docx` 命令 + 样式/页面断言 + 页数基线 |
| **P0b** | DocStyle + 三栏图桌 + 图注 + 宽条流程 + 目录域/页眉页脚 + 4 份样本 golden 断言 | 3 天 | 4 份样本全绿 |
| **P1** | bid 数据面（project/outline/chapter/figure）+ 导入 + 三栏工作台 + ACP 工具 + 页数目标 | 5–7 天 | 1 章 + 3 图端到端 |
| **P2** | 图表接入 + 有效字号/重叠/图文一致断言 | 7–10 天 | 4 条断言全绿 |
| **P3** | 专家包 + 场景工具面 + 守卫 | 7–10 天 | 单测 + 真实项目回归 |

---

## 8. 风险与对策

| 风险 | 对策 |
|---|---|
| 模板漂移（Word/WPS 差异） | 模板版本化；每次改样式重生成 golden；固定验收渲染器（优先 WPS/Word，记录版本） |
| `w:sdt` 被 Word 保存后改写 | 模板冻结；填充器只读模板；每次导出从模板副本开始 |
| 图片 relationship 冲突 | media 文件名唯一（fig id），rels 去重；[Content_Types] 补扩展名 |
| 固定行距吞图（R-022） | 图片段落使用 `w:spacing lineRule="atLeast"` 或单倍行距；逐页截图抽检 |
| 页数估算偏差（R-025） | 一律渲染器实测；P0a/P0b 只记基线；P1 建立校准表 |
| 图表字号/重叠（R-030/R-032） | 按插入宽度反算有效字号；bbox 重叠检测 |
| 目录域不更新 | Word/WPS 更新域后再计页；LO 路线单独标注偏差 |
| 中文字体缺失 | 模板写 `w:eastAsia="宋体"`；导出不嵌字体（许可）；预览字体回退不作为验收 |
| 专家包泄露 | 专家包不进公开仓库；本地加密/权限；导出件不含提示词 |
| **依赖未进独立 workspace** | `gui/src-tauri` 与根 workspace 分离，需**显式新增** `zip`/`quick-xml`/`image` 直接依赖；锁文件 image 实为 0.25.10 |
| **`cf-office-preview` 的 `zip.workspace=true` 无根 pin** | 现存隐患；建议在根 `Cargo.toml` 补 `[workspace.dependencies] zip = "3"` 或把该 crate 纳入 members |
| **命令未注册** | 新命令必须同时加入 `gui/src-tauri/src/lib.rs` 的 `generate_handler!` 宏 |
| **模板未分发** | `tauri.conf.json` 的 `bundle.resources` 需加入 `resources/templates/*.docx`；并实现 dev/安装态路径解析 |
| **manifest 单写者被破坏** | `bid_export_docx` 只返回路径，不写 manifest；登记走 `card.py` 或 agent 命令 |

---

## 9. 交付物清单

- `gui/src-tauri/resources/templates/bid-engineering-blue-v1.docx`
- `gui/src-tauri/resources/templates/fragments/*.xml`
- `gui/src-tauri/src/office/bid/{model,template,fill,docx,verify}.rs`
- `gui/src-tauri/src/commands.rs`：`bid_export_docx` / `bid_check_docx`
- `gui/src-tauri/src/lib.rs`：`generate_handler!` 注册上述命令
- `gui/src-tauri/Cargo.toml`：新增 `zip` / `quick-xml` / `image` 直接依赖
- `gui/src-tauri/tauri.conf.json`：`bundle.resources` 加入模板目录；路径解析（dev/安装态）
- `gui/src/components/bid/FigureTable.vue`、`ChapterEditor.vue`
- `gui/tests/bid-export/`：4 份 golden + 分层断言脚本
- `docs/标书导出-模板OOXML填充方案-v1.md`（本文）
- `docs/审计-链企Buddy优点整合qidiwork方案-2026-11-13.md`（审计依据）

---

## 10. 模板 v0 决策（用户定）

**定：程序化生成 v0；Word/WPS 只复核、不手编。**

理由：

1. 嵌套 `sdt`（`BID_CHAPTERS` 内套 `BID_CHAPTER`、表格行内 sdt）在 Word 手工插入内容控件并逐个设 tag 操作繁琐、易错、不可复现；`w:tag` 与代码常量的一致性只能靠生成器保证。
2. 模板是“唯一版式真值源”，必须可 diff、可版本化；手改 docx 会把真值源漂移进二进制，与 §3.1 治理目标冲突。
3. 首版样式参数已在 §5.3 全部量化，生成器直接翻译；Word 复核只验证视觉，不需要手工编辑权。

**降级预案**：若 P0a 发现手写全套 `styles.xml` 超预期，改为“Word 手工做纯样式页（无锚点）→ 脚本注入 sdt 骨架”；两条路线都保持“锚点程序化”底线。

---

## 附录 A：模板制作检查清单

1. 页面：A4 竖版、四边 1.2cm、页眉页脚距。
2. 样式：`BidBody/BidTitle1-4/BidTableHeader/BidTableBody/BidCaption/BidFigureCell` 全部存在且命名精确。
3. 目录域：`TOC \o "1-3" \h \z \u`；`updateFields=true`。
4. 页脚：`PAGE`、`NUMPAGES`。
5. 锚点：`BID_*` 内容控件齐全（含 `BID_CHAPTER`/`BID_TABLE_ROW`），`w:tag` 与代码常量一致。
6. 表格：3 列图桌样例、表头底纹、首列底纹、`cantSplit`。
7. 图注：`BidCaption` 样式，居中 12pt 粗。
8. 保存为 `.docx`（不是 `.dotx`），关闭“兼容模式”。

## 附录 B：依赖与命令

- 新增直接依赖（`gui/src-tauri/Cargo.toml`）：`zip = "3"`、`quick-xml = "0.41"`、`image = "0.25"`（锁文件实际 0.25.10）。
- 根 workspace：补 `[workspace.dependencies] zip` 或把 `cf-office-preview` 纳入 members（现存隐患）。
- 命令注册：`gui/src-tauri/src/lib.rs` 的 `tauri::generate_handler!`。
- 模板资源：`tauri.conf.json` `bundle.resources` + dev/安装态路径解析。
- 测试：`cargo test`（新增模块）；`cargo clippy --all-targets`。
- 渲染验收：WPS/Word 打开 → 更新域 → 另存 PDF → 计页；或 LibreOffice `--headless --convert-to pdf`（标注 TOC 偏差）。

---

## 附录 C：适用条目定义（内联，出处见“依据文件”）

- **R-025【门控】篇幅硬上限必须用渲染器实测＋校准系数建模，禁止按“字符数÷每页字数”估页。**
- **R-022【文档】docx 固定行距吞图；含图交付物必须视觉抽检前置。**
- **R-030【门控】图表类交付必须校验“印刷有效字号”，脚本字号与无告警都不算证据。**
- **R-032【门控】图内文字重叠用渲染器几何判定，不靠视觉席“看像不像”。**
- **R-031【门控】写进规范文档的每条纪律必须当场配一条可跑断言（有纪律无执法＝没有纪律）。**

适用条目：R-025 / R-022 / R-030 / R-032 / R-031