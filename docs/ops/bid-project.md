# 标书项目工作区（bid_project）

## 用途

把「一份标书 = 一个目录」落成工程制品：目录里放一份清单 + 一堆按章拆分的 Markdown，
`bid_project build` 一键合并成 `BidExportRequest` 灌进工程蓝模板，产出可直接交付的 `.docx`，
并立刻跑 24 项结构校验（样式 / 页边距 / 目录 / 章节 / 表格 / 图注 / 暗标元数据等）。
适合多人分章协作（一人一章一个 md）与批量出稿；退出码机器可读，失败会点名不合格的 check。

CLI 位置：`gui/src-tauri/cli/src/bin/bid_project.rs`（crate `qidiwork-cli`，`cargo run -p qidiwork-cli --bin bid_project`）。
默认模板：`gui/src-tauri/resources/templates/bid-engineering-blue-v1.docx`（清单 `style_id` 可换）。
（只导出单份 md 时仍可用 `bid_export --md`，见 `docs/ops/bid-export-cli.md`。）

## 目录结构

```text
<proj>/
  bid.project.json            # 项目清单（书名 / 模板样式 / 图宽 / 章节顺序）
  chapters/
    01-概述.md                # new 时自动生成，含 2 个示例小节
    02-施工部署.md            # 自己加；默认按文件名升序合并
  assets/
    image3.png                # 章节里 ![图注](assets/image3.png) 引用
  out.docx                    # build 默认输出（可用 -o 改）
```

`bid.project.json`：

```json
{
  "title": "某县2026年度高标准农田建设项目施工组织设计",
  "style_id": null,
  "figure_width_cm": null,
  "chapters": []
}
```

- `title`：书名。**各 md 文件自己的 H1 一律忽略**，书名只认这里。
- `chapters` 为 `[]` 或缺省 → 取 `chapters/*.md` **按文件名升序**（推荐 `01-`/`02-` 前缀）；
  显式列成 `["chapters/02-施工部署.md", "chapters/01-概述.md"]` 则**以列表顺序为准**。
- `style_id`：`build` 用它选模板 `gui/src-tauri/resources/templates/<style_id>.docx`；
  `null` / 缺省 → `bid-engineering-blue-v1`（工程蓝）。id 非法或模板不存在 →
  `ERROR 模板解析失败: 模板未找到: <style_id>.docx` exit 2。
- `figure_width_cm`：透传给导出层，`null` 用默认 15.5cm。

Markdown 口径与 `bid_export --md` 完全一致（见 P1-1 规格 §1）：`##` 成章、`###`/`####` 成小节、
GFM 表格、整行 `![图注](路径)` 成图（连写 3 行合成三栏图桌、`{wide}` 后缀为通栏图、
`{ai}` 后缀给**该张**图图注追加「（AI 生成示意图）」、`{wide,ai}` 可组合）。图片标记内只认
`wide` / `ai`（大小写敏感），出现其它标记或空标记（`{AI}`、`{width}`、`{wide,}`、`{}`）→
`ERROR 章节解析失败 …: 图片标记不识别: <花括号内原文>` + exit 2，不会静默丢图。
图片相对路径一律按**项目根目录**（`<proj>/`）解析——`assets/` 与 `chapters/` 平级，
所以 `chapters/01-概述.md` 里写 `![施工现场平面布置](assets/image3.png)`，
指向 `<proj>/assets/image3.png`（写成 `chapters/assets/…` 会找不到图）。

## 三个子命令

在工作目录 `gui/src-tauri` 下执行：

```bash
# 1) 建骨架：清单 + chapters/01-概述.md + assets/
bid_project new <dir> --title "某县2026年度高标准农田建设项目施工组织设计"

# 2) 导出：合并所有章节 → docx（默认 <dir>/out.docx）→ 24 项校验
bid_project build <dir> [-o <out.docx>] [--no-number]

# 3) 体检：不导出，只查清单 / 章节文件 / 图片引用 / 结构合法性
bid_project check <dir>
```

- `build` 默认自动编号：合并完成后**统一重号**（`第N章` / `N.M` / `N.M.K`），
  各文件不再各自从「第一章」重来；已显式编号的标题原样保留。
  `--no-number` 关掉自动编号（标题按作者原样保留）。
- `build` 成功后先打一行 JSON 摘要、再打 `BUILD OK`；
  结构校验不合格打印 `BUILD FAIL <失败 check 名,逗号分隔>` 并 exit 1；
  参数 / 清单 / 章节解析 / 导出错误打印 `ERROR <原因>` 并 exit 2。
- `check` 健康 → `PROJECT OK` exit 0；有问题 → `PROJECT FAIL <问题,逗号分隔>` exit 1。
- `new` 在已有 `bid.project.json` 的目录上重复执行 → `ERROR 项目已存在` exit 2（不覆盖稿件）。

## 完整示例

```bash
cd gui/src-tauri
PROJ="G:/qidicode/target/tmp/bid-p0a/proj-demo"

# 1) 建项目
cargo run -p qidiwork-cli --bin bid_project -- new "$PROJ" --title "某县2026年度高标准农田建设项目施工组织设计"
# {"dir":"…/proj-demo","manifest":"…/proj-demo\bid.project.json"}
# NEW OK

# 2) 写第二章（文件名前缀决定合并顺序；文件里的 H1 会被忽略）
cat > "$PROJ/chapters/02-施工部署.md" <<'EOF'
# 忽略的书名

## 施工部署

先地下后地上，分区分段流水施工。

### 施工顺序

测量放线 → 土方开挖 → 渠系砌筑 → 田间道路。
EOF

# 3) 导出（输出 …/proj-demo/out.docx）
cargo run -p qidiwork-cli --bin bid_project -- build "$PROJ"
# {"path":"…/proj-demo\out.docx","bytes":4899,"chapters":3,"paragraphs":3,"tables":0,"figures":0}
# BUILD OK

# 4) 体检（不产出文件）
cargo run -p qidiwork-cli --bin bid_project -- check "$PROJ"
# PROJECT OK
```

`out.docx` 即可直接用 WPS / Word 打开；样式、页边距、目录域、页眉页脚都来自模板，
正文内容全部由 Markdown 生成。要改模板样式参数属另一条线（P0b `DocStyle`），不在本文范围。

## 排障对照

| 现象 | 原因 | 处理 |
|---|---|---|
| `ERROR 项目清单缺失: bid.project.json` | 目录里没有清单（先 `new`，或路径指错） | 确认 `<dir>/bid.project.json` 存在 |
| `ERROR 项目已存在` | 目标目录已有清单 | 换目录；确需重建先自行备份并删除清单 |
| `ERROR 章节目录为空: chapters/` | 清单 `chapters=[]` 且 `chapters/` 无 `.md` | 往 `chapters/` 放 `01-….md`，或在清单里显式列出 |
| `ERROR 章节文件不存在: …` | 清单里列的文件不在盘上 | 改清单或补文件 |
| `ERROR 章节解析失败 …: 图片不存在: …` | md 引用的图片缺失（相对**项目根目录**解析） | 图片放进 `<proj>/assets/`，md 写 `assets/xxx.png` |
| `ERROR 模板解析失败: 模板未找到: …` | 清单 `style_id` 指向的模板不存在 / id 非法 | 改成 `null` 用默认工程蓝，或把 `<style_id>.docx` 放进 `resources/templates/` |
| `ERROR 章节解析失败 …: 表格第 N 行列数不符` | GFM 表格列数不一致 | 补齐竖线单元格（每行列数一致） |
| `PROJECT FAIL 至少需要一章` | 所有 md 都没有 `##` 小节 | 章文件里至少一个 `##` |
| `PROJECT FAIL 图注不能为空` | 图片行 `![…]()` 图注为空且单图无组图注 | 给图注补字 |
| `BUILD FAIL <check 名>` | docx 结构校验不过（模板/样式被改） | 用 `bid_export --check-only out.docx` 复现，核对模板未被篡改 |
