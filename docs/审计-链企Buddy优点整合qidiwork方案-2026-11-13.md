# 审计报告：链企Buddy 优点整合 qidiwork 方案

**审计日期**：2026-11-13  
**审计对象**：《如何把链企Buddy的优点整合到我们自己的 qidiwork 里面》方案（P0–P3）  
**审计方式**：先自由审查（F1–F14，未读清单）→ 后对照 `docs/audit-rubrics/RUBRICS.md` 注入 ≤5 条  
**结论**：方向正确，但 **P0 按原文不可执行（NO-GO）**；完成四项前置整改后条件 GO。

---

## 一、自由审查发现（先落盘，未看清单）

| ID | 严重度 | 发现 | 证据 |
|---|---|---|---|
| F1 | **阻断** | 方案称“qidiwork 标书技能族已经有了”，但**随包技能包缺关键件**：`qidiwork-skills-bundle.zip` 里没有 `bid-write`、`bid-review`、`hefei-bid-review`、`construction-tender-proposal`、`bid-content-diff`，也没有 `md2docx.py`。开发机 `~/.qidi/skills` 有，不等于用户机器有。 | `unzip -l gui/src-tauri/resources/qidiwork-skills-bundle.zip` 仅见 bid-creation/cost-analysis/diagram/docx-fix/drawing/perfect/render/office-tools（2026-10-16 快照） |
| F2 | **阻断** | 方案称“复用 `md2docx.py`，把 DocStyle 注入为 Word 样式（不改生成逻辑，只改样式表）”。实测该脚本**没有**页边距、1.5 倍行距、标题颜色、表头颜色、三栏图桌、图注的任何设置；只有 Normal 12pt 宋体、首行缩进 0.74cm、黑三线表、单图 `add_picture(width=Cm(15.5))`。所谓“注入”= 重写导出器。 | `rg "line_spacing\|section\|page_margins\|color\|caption" md2docx.py` 无命中；仅 194 行 `doc.add_picture(...)`、77 行三线表 |
| F3 | **阻断** | **没有 Python 运行时**。`md2docx.py` 依赖 `python-docx`，`bid-diagram` 依赖 `matplotlib`；qidiwork 安装包只内置 AI 内核与技能 zip，未见 Python/依赖。用户机器未装 Python 时，导出与图表直接失败。 | `gui/src-tauri/resources/` 仅 `qidiwork-skills-bundle.zip`；`rg -i "python" gui/src-tauri/src tauri.conf.json Cargo.toml` 无命中；脚本自述 `pip install python-docx/matplotlib` |
| F4 | 高 | **P0 工期 1–2 天严重低估**。需同时做：导出引擎选型、DocStyle 全量映射、三栏图桌、图注、样本回归；且 F2/F3 未解决前无法开工。按现状至少 3–5 天。 | 方案 P0 条目 vs F2/F3 工作量 |
| F5 | 高 | **导出保真度未定义唯一真值源**。预览用 `docx-preview`（JS），导出用 Python，两套样式各写各的；方案未定义“预览 CSS ↔ DOCX 样式”的单一映射，也没有保真测试矩阵。 | `gui/src/components/DocxPreview.vue` + `docx-preview`；`spike-docx` 显示同一 DOCX 的 facts 与 render 文本量差异（574,734 vs 714,003） |
| F6 | 高 | **缺少“存量文档导入”路径**。方案从新建 `project.toml/outline.json` 开始，但用户手里是 DOCX/PDF 招标文件与既有标书；没有 docx→outline/chapters、PDF→项目概述的导入步骤，采用门槛高。 | 方案数据模型无 import；`construction-tender-proposal/tender_extract.py` 在用户目录但不在随包 zip |
| F7 | 中 | **图文 QC 规则不可执行**。“caption ↔ content ↔ chartText 一致”“并列/二选一不得画成顺序”目前是口号；缺：outline 的 `relation: sequence\|alternative` 字段、chartText 契约、校验失败是阻断还是告警、自动修复策略。 | 方案 P1/P2 条目 |
| F8 | 中 | **AI 图合规策略未落地**。方案写“标注或换真实照片”，但未定义标注位置（元数据/图注/水印/导出层）、用户能否移除、与暗标规则如何交互。 | 方案“不照搬清单”第 2 条 |
| F9 | 中 | **office 数据契约冲突未处理**。`office_open` 拒绝相对路径（manifest 必须登记绝对路径）；`office_read_file` 明确剔除 html 防 XSS；`office_scan` 把每个子目录当工作区。`bid/` 子目录与 manifest/产物面板如何注册未定义。 | `gui/src-tauri/src/office/mod.rs`：相对路径拒绝、html 不进 webview、`office_scan` 枚举子目录 |
| F10 | 中 | **Agent/ACP 工具接入缺失**。方案只列 Tauri 命令 `office_bid_*`，未定义 agent 工具名、schema、权限、广告面；没有这些，技能仍靠人工点击，无法形成“解析→规划→写章→配图→导出”的自动链。 | 方案“目标架构”无 ACP tool 层 |
| F11 | 中 | **验收样本不足**。只提一个 `施工组织设计技术暗标.docx`，无 golden 输出、无自动 diff、无页数/字体/图注的量化阈值；不足以证明“没有改坏”。 | 方案 P0 验收 |
| F12 | 中 | **图片生成成本未预算**。P1 “1 章 + 3 图”与 P2 “33 章回归”都依赖生图 API/额度；未列预算、失败回退（占位图/真实照片）、重试策略。 | 方案 P1/P2 |
| F13 | 低 | **`office/bid` 模块 vs 独立 crate 未论证**。现有 `office` 模块职责是产物扫描/预览/打开；把标书流水线塞进去会膨胀 Tauri 层，且难做 Rust 单测。建议独立 `cf-bid` crate 或 `office/bid` 子模块并明确边界。 | `gui/src-tauri/src/office/mod.rs` 职责注释 |
| F14 | 低 | **专家包 IP/分发未处理**。本地提示词是核心资产；若 qidiwork 对外分发，需明确“专家包不进公开仓库/可加密/可审计”，否则等于把标书 know-how 随包送人。 | 方案 P2 专家包 |

---

## 二、声称-判定表（R-014 格式）

| # | 方案声称 | 判定 | 证据 |
|---|---|---|---|
| 1 | “qidiwork 标书技能族已经有了，整合是加一层” | ⚠️ 部分属实 | 开发机 `~/.qidi/skills` 有 14 个 bid-*；但随包 zip 缺 bid-write/review/hefei/construction-tender/bid-content-diff |
| 2 | “P0 复用 md2docx.py，把 DocStyle 注入为 Word 样式” | ❌ 不实 | md2docx 无页边距/行距/颜色/图注/图桌；属重写而非注入 |
| 3 | “P0 1–2 天” | ❌ 不实 | F2+F3 未解，至少 3–5 天 |
| 4 | “图桌组件 FigureTable.vue 可直接复用” | ⚠️ 部分属实 | Vue 组件可做；但导出侧不支持三栏/图注，需新导出器 |
| 5 | “office/bid 数据契约可落到 office-workspaces” | ⚠️ 部分属实 | 可落，但 manifest 必须绝对路径、html 不进 webview、子目录会被 `office_scan` 当工作区 |
| 6 | “图文 QC：caption↔content↔chartText” | ⚠️ 无法核实 | 方案未给 schema/算法/失败策略，当前不可执行 |
| 7 | “并列/二选一不得画成顺序” | ✅ 方向属实 | 但缺 `relation` 字段与检测器 |
| 8 | “AI 伪实景要标注” | ⚠️ 无法核实 | 未定义标注位置与不可移除性 |
| 9 | “P1 3–5 天生成 1 章 + 3 图 + 导出” | ❌ 不实 | 依赖 F2/F3/F6 与生图额度，未预算 |
| 10 | “P2 33 章回归” | ⚠️ 无法核实 | 无 golden 样本、无成本预算 |

---

## 三、RUBRIC 对照（注入 5 条）

### R-025【门控】篇幅硬上限必须用渲染器实测＋校准系数建模，禁止按“字符数÷每页字数”估页

- **触发点**：方案引用 49,100 字 / 982 字每页 ≈ 50 页，并设 45–49 页目标。
- **判定**：❌ 违反。982 是链企服务端的经验值，不是 qidiwork 渲染器的实测值；字体、行距、表格密度、图片高度都会改变每页字数。
- **整改**：用 qidiwork 的 DOCX 导出器对 3–5 份真实样本实测“页数 vs 字数/表格数/图片数”，拟合校准系数；每章预算按实测回填，禁止直接除。

### R-022【文档】docx 固定行距吞图；含图交付物必须视觉抽检前置

- **触发点**：方案 P0 要做 1.5 倍行距 + 三栏图桌 + 图注，并导出 DOCX。
- **判定**：❌ 未覆盖。固定行距会裁掉内嵌图片；方案没有“出图后视觉抽检/页面截图核对”的门禁。
- **整改**：导出后用 Word/渲染器逐页截图或转 PDF 抽检：图片是否完整、图注是否跟随、表格是否跨页断裂；把“页面截图 + 图片计数 + 图注计数”作为 P0 验收证据。

### R-030【门控】图表类交付必须校验“印刷有效字号”，脚本字号与无告警都不算证据

- **触发点**：方案 P2 复用 `bid-diagram` 的横道图/网络图/曲线。
- **判定**：❌ 未覆盖。脚本画布字号 ≠ 插入 Word 后的有效字号；“脚本无告警”不构成证据。
- **整改**：按最终插入宽度（如 15.5cm 或图桌 1/3 栏）反算有效字号；低于 8pt 的图表必须重排或改版式；把有效字号写进验收脚本。

### R-032【门控】图内文字重叠用渲染器几何判定，不靠视觉席“看像不像”

- **触发点**：方案 P2 的流程图/横道图/网络图含多处文字标注。
- **判定**：❌ 未覆盖。方案没有文字包围盒/重叠检测。
- **整改**：用渲染器（matplotlib/SVG）取文本 bbox，做两两重叠检测；流程图节点文字与箭头、图注与图片边界同样检测；重叠即失败，不靠视觉席主观判断。

### R-031【门控】写进规范文档的每条纪律必须当场配一条可跑断言（有纪律无执法＝没有纪律）

- **触发点**：方案写入多条纪律：并列工艺不得线性化、图注必须一致、AI 图必须标注、图桌落版规则。
- **判定**：❌ 未覆盖。四条纪律都没有对应脚本/测试。
- **整改**：每条纪律配一个可执行断言：
  - `check_outline_relation.py`：含 alternative 的章节不得出现在 sequence 流程；
  - `check_figure_caption.py`：caption 必须命中 content 关键词；
  - `check_ai_image_label.py`：AI 图必须有 source 标记；
  - `check_image_table.py`：图桌列数/宽高比/图注数量一致。

---

## 四、整改后的建议路线（条件 GO）

### 前置整改（必须先做，1–2 天）

1. **导出引擎定案**：优先 Rust 侧 DOCX 生成（避免 Python 依赖）；若坚持 Python，必须把 Python + python-docx + matplotlib 随包装入，并在安装器里做运行时探测与降级。
2. **技能包补齐**：把 `bid-write`（含 md2docx）、`bid-review`、`hefei-bid-review`、`construction-tender-proposal`、`bid-content-diff` 同步进 `qidiwork-skills-bundle.zip`，并建立版本清单。
3. **DocStyle 真值源**：定义 `DocStyle`（页边距/字体/行距/颜色/表格/图桌/图注），映射到“预览 CSS + DOCX 样式”两处，附一份 golden 样本。
4. **导入路径**：先做 DOCX/PDF → `project.toml + outline.json + chapters/*.md` 的最小导入，否则新项目从零开始，采用率低。

### 修订后的阶段

| 阶段 | 内容 | 工期 | 门禁 |
|---|---|---|---|
| P0a | 导出引擎 spike（Rust docx 或随包 Python）+ 1 份真实样本渲染 | 2 天 | 页数/边距/图片完整（R-025/R-022） |
| P0b | DocStyle + FigureTable + 暗标样本回归 | 2–3 天 | 页面截图 + golden diff（R-029 触发） |
| P1 | bid 数据面 + 导入 + 三栏工作台 + ACP 工具 | 5–7 天 | 1 章 + 3 图端到端 |
| P2 | 图元数据双账 + 图文 QC + 图表接入 | 7–10 天 | 4 条可跑断言全绿（R-031） |
| P3 | 专家包 + 场景工具面 + 守卫 | 7–10 天 | 单测 + 真实项目回归 |

---

## 五、rubric 未触发类说明

| 条目 | 为什么未触发 |
|---|---|
| R-029 交付就绪度绑定产物指纹 | 本轮审的是方案，不是交付就绪度；P0b 验收时触发 |
| R-009 新守卫对称检验假阴性 | 方案尚未落地守卫代码；实现时触发 |
| R-006 文档事实逐条对照代码 | 本报告已逐条取证（F1–F3）；方案本身的事实断言已进“声称-判定表” |
| R-023 第三方库行为探针实证 | 本轮未断言 python-docx/docx-preview 的具体边界行为；P0a 做探针时触发 |
| R-011 新测试外部二进制依赖可跳过 | 方案未新增测试；落地时触发 |
| R-021 零命中结论双搜法 | 未做“无调用方/不存在”类负向结论 |
| R-001/R-010 改公共默认值 | 方案未改现有默认值 |
| R-003/R-005 测试断言/改测试凑通过 | 方案未新增/修改测试 |
| R-019 并行写章口径对账 | 方案未涉及并行写章 |
| R-028 整行删除序号连续性 | 方案未涉及删行压缩 |

---

## 六、结论

- **方向正确**：版式令牌、三段式节奏、图元数据双账、确定性图表、守卫套件，都是可迁移的高价值资产。
- **P0 按原文 NO-GO**：F1（技能包缺件）、F2（md2docx 能力不足）、F3（无 Python 运行时）三者任一都足以让 P0 失败。
- **条件 GO**：先完成“导出引擎定案 + 技能包补齐 + DocStyle 真值源 + 导入路径”四项前置整改，再按修订后的 P0a/P0b 执行。
- **最大风险**：导出保真度与页数控制（R-025/R-022），其次是图文 QC 的可执行性（R-031/R-032）。

---

## 附录：取证命令

```bash
unzip -l gui/src-tauri/resources/qidiwork-skills-bundle.zip
rg -n "add_picture|line_spacing|section|page_margins|color|caption" ~/.qidi/skills/bid-write/scripts/md2docx.py
rg -n -i "python" gui/src-tauri/src gui/src-tauri/tauri.conf.json gui/src-tauri/Cargo.toml
sed -n '410,470p' gui/src-tauri/src/office/mod.rs
```

---

适用条目：R-025 / R-022 / R-030 / R-032 / R-031  
（R-014 为审计报告格式要求，本报告已按“声称-判定表”执行，不另计为审计条目。）