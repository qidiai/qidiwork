# 标书导出 CLI（bid_export）

## 用途

把一份请求 JSON（`BidExportRequest`）或一份 Markdown 稿子（P1-1 `--md`，自动分章编号）
灌进工程蓝标书模板，产出可直接交付的 `.docx`，
并在写盘后立刻跑 24 项结构校验（样式 / 页边距 / 目录 / 章节 / 表格 / 图注 / 暗标元数据等）。
适合 CI、批量出稿和人工排障：退出码机器可读，失败会点名不合格的 check。

CLI 位置：`gui/src-tauri/cli/src/bin/bid_export.rs`（crate `qidiwork-cli`，`cargo run -p qidiwork-cli --bin bid_export`）。
默认模板：`gui/src-tauri/resources/templates/bid-engineering-blue-v1.docx`。

> CLI 已独立为 `qidiwork-cli` crate（`gui/src-tauri/cli`），GUI 安装包不再携带 CLI 二进制；
> CLI 产物单独分发，见 `docs/ops/bid-packaging.md`「CLI 独立分发」。

## 用法

在工作目录 `gui/src-tauri` 下执行：

```bash
# 1) 从请求 JSON 导出（--out 覆盖请求 JSON 里的 output_path）
bid_export --input <req.json> --out <out.docx> [--template <tpl.docx>] [--pages]

# 2) 从 Markdown 导出（P1-1：默认自动编号；--no-number 关掉自动编号）
bid_export --md <file.md> --out <out.docx> [--no-number] [--template <tpl.docx>] [--pages]

# 3) 只校验（不导出，跑 verify::verify(&p, None)）
bid_export --check-only <docx>
```

成功输出（先一行 JSON 摘要，再 `EXPORT OK`）：

```json
{"path":"…/out/p0a-sample.docx","bytes":20226,"chapters":2,"paragraphs":4,"tables":1,"figures":1}
EXPORT OK
```

`--check-only` 成功输出：

```json
{"checks":24,"ok":true}
CHECK OK
```

结构校验失败：

```
CHECK FAIL doc.metadata_anonymous,doc.captions
```

（逗号分隔，只列 `ok=false` 的 check 名，便于 `grep` / 分支判断。）

## Markdown 内容入口（`--md`）

正文写成 Markdown 直接出稿，不必手写请求 JSON（解析器：`office/bid/markdown.rs`）。

```bash
# 示例夹具：1 H1 + 3 H2（含自带「第二章」）+ H3/H4 + 6 段 + 3 列表格 + 单图/三栏/宽图
cargo run -p qidiwork-cli --bin bid_export -- --md ../tests/bid-export/md/demo.md --out ../tests/bid-export/out/md-demo.docx

# 关掉自动编号：标题原样保留（已带编号的仍原样）
cargo run -p qidiwork-cli --bin bid_export -- --md ../tests/bid-export/md/demo.md --out ../tests/bid-export/out/md-demo-nonum.docx --no-number

# 换模板 / 自定义稿子
cargo run -p qidiwork-cli --bin bid_export -- --md ./稿件/施工组织设计.md --out ./out/标书.docx --template ./tpl/某业主模板.docx
```

口径：

- `--md` 与 `--input` **互斥**，同时给 → `ERROR --md 与 --input 不能同时使用` + exit 2。
- 图片相对路径按 **Markdown 文件所在目录** 解析；路径不存在 → `ERROR Markdown 解析失败: 图片不存在: <path>` + exit 2。
- 解析实现：`parse_markdown_with(src, base_dir, !no_number)`；除内容来源外，
  与 `--input` **完全一致**——同样导出、跑 24 项 check、打印 JSON 摘要 + `EXPORT OK`。
- 已知取舍①（图片路径）：`![..](路径)` 接受**绝对路径**与 `../` 相对路径，即可读出
  Markdown 文件所在目录之外的文件——属**有意放开**（出稿常复用上级素材目录），不是路径穿越漏洞；
  仍要求目标文件真实存在，否则 `ERROR Markdown 解析失败: 图片不存在: <path>`。
- 已知取舍②（编号）：H4 先于本章任何 H3 出现时，节号自动补 1，产出 `N.1.K`（不会出现 `N.0.K`）。
- `demo.md` 实测产物：`BidTitle1=4`（文档标题 1 + 3 章）、`BidTitle2=3`（3 个 H3）、
  `BidTitle3≥1`（H4 子节）、带 `BidTableHeader` 的 `<w:tbl>` 1 张、
  `<w:drawing` 5 处（单图 1 + 三栏 3 + 宽图 1）、横道图 `<w:tbl>` 1 张（含 `#4877AC` 底纹 7 格，
  无 `BidTableHeader`，故仍计入 `tables=1`）。

### Markdown 语法速查

唯一真值见 `docs/施工规格-P1-1-2026-11-16.md` §1（写法 → 产出）：

- `# 标题`（首个）→ 文档标题（`BidTitle1`）；后续 H1 忽略。
- `## 文本` → 一章：未编号自动 `第N章 文本`；已带 `第一章…` / `1.` 原样保留。
- `### 文本` → 节标题 `BidTitle2`：自动 `N.M 文本`；已带 `N.M` 原样保留。
- `#### 文本` → 子节 `BidTitle3`：自动 `N.M.K 文本`。
- 空行分隔的普通文本 → 每段一个 `BidBody` 段（段内换行合并为空格）。
- 首行 `| a | b |` 紧跟分隔行 `| --- | --- |` → GFM 表格（首行表头，其余数据行，列数以表头为准）。
- `![图注](路径)` 单行 → 单图 `single`（无图注可写 `![](...)`）。
- `![图注](路径){wide}` → 宽图 `wide`（内容区全宽 18.6cm）。
- `![图注](路径){ai}` → 该图图注追加「（AI 生成示意图）」；**只标写了 `{ai}` 的那张**，
  三栏组图里同组其它图不受影响（逐图 `item.ai`）。
- `{wide,ai}` 可组合（逗号分隔、容忍空格）。标记内**只认** `wide` / `ai`（大小写敏感），
  出现其它 token 或空 token（`{AI}`、`{width}`、`{wide,}`、`{}`）→
  `ERROR Markdown 解析失败: 图片标记不识别: <花括号内原文>` + exit 2，不再静默降级成段落丢图。
- 连续 3 行图片（行间无空行）→ 三栏组图 `three_up` + 逐图图注。
- 围栏 ```` ```gantt [标题] ```` … ```` ``` ```` → 原生表格横道图（P1-5，无图片、无新依赖）：
  每行一个工序，写作 `开始..结束 | 工序名`（`YYYY-MM-DD`，含首含尾），空行忽略，
  围栏首行 ```` ```gantt ```` 之后的文本即图注（留空则不加图注）；
  时间桶默认 7 天/桶，跨度 > 112 天改 14 天/桶，> 16 桶 → `ERROR ... 横道图跨度过大（>16 个时间桶）` + exit 2。
- `**粗体**` / 链接 / `-` `>` 列表符号 → 不解析，按纯文本原样保留。

编号规则：中文数字仅 1–10（`一…十`），11 起用阿拉伯数字；章/节序号按出现顺序递增。

横道图写法与产物（P1-5，规格 `docs/施工规格-P1-5-gantt-2026-11-16.md`）：

````
```gantt 图4-1 施工进度横道图
2026-12-01..2026-12-10 | 塘底清淤
2026-12-05..2026-12-20 | 管网敷设
2026-12-15..2026-12-25 | 路基整平
```
````

- 表头行 `工序` + 各桶起始日（同年写 `MM-DD`；`min_start` 与 `max_end` **跨年**时改带年份
  `YYYY-MM-DD`）；任务覆盖的桶填 `#4877AC` 底纹（判定
  `task_start < bucket_end && task_end >= bucket_start`），其余桶留空。
- 表格复用 `tblStyle=BidTable`；**所有单元格（含表头行）一律 `BidTableBody`，不用 `BidTableHeader`**，
  因此业务表计数 `doc.tables` / `check.sh tables` 不受横道图影响。
- 日期用纯整数 civil-from-days 换算（不引日期库）；`start > end` / 日期不存在 →
  `ERROR 导出失败: 横道图日期非法: <start>..<end>`；跨度过大见上条。
- `demo.md` 里的横道图为 3 工序 / 25 天跨度 → 4 个 7 天桶，导出件含 7 格底纹（覆盖桶数）。

## 页数实测（`--pages` / `bid_pages`）

页数**不估算**，用本机 WPS（COM `KWPS.Application`）只读打开导出件、`Repaginate()` 后
`ComputeStatistics(2)`（页）/ `ComputeStatistics(0)`（字）实测，**不保存、不改动原文件**。
实现：`office/bid/pages.rs`（`pages::measure(&docx) -> PageStat { pages, words }`），
脚本写法照 `gui/tests/bid-export/render-wps.ps1`（先 `Get-Process wps` 判断是否已有实例，
只在自己拉起实例时才 `Quit()`，不会关掉用户正开着的窗口）。

独立 CLI（`gui/src-tauri/cli/src/bin/bid_pages.rs`）：

```bash
cargo run -p qidiwork-cli --bin bid_pages -- ../tests/bid-export/out/md-demo.docx
```

```json
{"pages":5,"words":514}
PAGES OK
```

导出时顺带实测（`--pages`，`--input` / `--md` 两条路径都支持）：

```bash
cargo run -p qidiwork-cli --bin bid_export -- \
    --md ../tests/bid-export/md/demo.md \
    --out ../tests/bid-export/out/md-demo.docx \
    --pages
```

```json
{"path":"…/out/md-demo.docx","bytes":78335,"chapters":3,"paragraphs":6,"tables":1,"figures":5,"pages":5}
PAGES=5
EXPORT OK
```

口径：

- `--pages` 只在**导出 + 24 项 check 全过之后**才测页数：check 失败（exit 1）不测。
- 摘要行仍是第一行（字段顺序不变），`pages` 追加在末尾；随后一行 `PAGES=<n>`，再 `EXPORT OK`。
- 测量失败（本机没装 WPS / COM 未注册 / 文档打不开 / 输出无法解析）只打
  `WARN 页数测量失败: <原因>` 到 stderr，**摘要不带 `pages`、退出码不变（仍 0）**——
  页数是旁证，不是导出门槛。
- `bid_pages` 失败统一 `ERROR <原因>`（stderr）+ exit 2：缺参 / 文件不存在 /
  `无法解析页数输出: …` / PowerShell 非 0 退出。
- `--check-only` 与 `--pages` 不能混用（`ERROR --check-only 不能与 --pages 混用` + exit 2）；
  只校验不导出，没有可测的新产物。
- **平台**：仅 Windows（需本机 WPS，本项目实测 `F:\wps\WPS Office\12.1.0.29690`，
  COM 已注册为 `KWPS.Application`）。非 Windows 平台 `pages::measure` 直接返回
  `Err("页数测量仅支持 Windows（需本机 WPS）")`。
- 备注：`bid_pages` 对**不是 docx** 的文件不做内容校验——WPS 会当纯文本开成新文档并返回
  1 页，页数无意义；要断言「是不是有效 docx」请用 `bid_export --check-only`。
- 与 `gui/tests/bid-export/render-wps.ps1` 的分工：本 CLI 只报页/字（快）；
  `render-wps.ps1` 另外落一份 PDF（人工核对版式、`p0b-page-baseline.md` 基线），
  两者页数应一致。

## 请求 JSON 示例

顶层字段对应 `office::bid::model::BidExportRequest`：`title`、`chapters[]`、
可选 `output_path`（可省略，被 `--out` 覆盖）、`style_id`、`figure_width_cm`。
`chapters[].blocks[]` 用内部标签 `type` 区分**五种**块（`paragraph` / `heading` / `table` / `figure` / `gantt`）：

```json
{
  "title": "槐林镇2026年度和美乡村省级中心村建设项目施工组织设计",
  "chapters": [
    {
      "title": "第一章 工程概况",
      "blocks": [
        { "type": "paragraph", "text": "本工程主要建设内容包括道路硬化、排水管网、文化广场及绿化提升。" },
        {
          "type": "table",
          "header": ["工序节点", "施工方法及控制参数", "质量控制要点"],
          "rows": [
            ["塘底清淤", "长臂挖机分段清淤，深度偏差≤50mm", "隐蔽验收合格后方可回填"],
            ["路基整平", "推土机粗平+平地机精平，压实度≥93%", "每200m检测一点"]
          ]
        },
        {
          "type": "figure",
          "layout": "three_up",
          "caption_mode": "per_group",
          "caption": "图1-1 塘底整平与排水示意组图",
          "items": [
            { "path": "assets/image3.png" },
            { "path": "assets/image3.png" },
            { "path": "assets/image3.png" }
          ],
          "ai_generated": true
        }
      ]
    }
  ]
}
```

要点：

- `type`：`paragraph` / `heading` / `table` / `figure`（`snake_case`）。
- `type: "gantt"`（P1-5）：`{ "type": "gantt", "title": "图4-1 施工进度横道图", "tasks": [{ "name": "塘底清淤", "start": "2026-12-01", "end": "2026-12-10" }] }`；
  `title` 可省略（空则不加图注），`tasks` 至少 1 条，日期 `YYYY-MM-DD` 且 `start <= end`。
- `heading.level` 仅 **3 或 4**（H3→`BidTitle2`、H4→`BidTitle3`；H2 是章标题，不进 `heading`）。
- `figure.layout`：`single` / `wide` / `three_up`；`three_up` 不足三张时补空单元格，不拉伸、不报错。
- `figure.caption_mode`：`per_image`（逐图图注，默认）/ `per_group`（整组一条图注）。
- `figure.items[].path` 支持相对路径（按**请求 JSON 所在目录**解析，与 `bid_smoke` 同口径）与绝对路径。
- `figure.items[].ai`（默认 `false`）逐图标 AI：`per_image` 时**只**给 `ai: true` 的那张图注追加
  「（AI 生成示意图）」；`per_group` 时按组级 `ai_generated` 决定（组图注只有一条）。
- `ai_generated: true` 时该组图注自动追加「（AI 生成示意图）」标识（组级判定；`per_image` 下以
  `items[].ai` 为准，任一 `items[].ai` 为真时 `ai_generated` 也应为真，供 `per_group` 用）。
- 请求 JSON 里的多余字段（如 fixture 的 `expect`）被忽略，可直接复用 `gui/tests/bid-export/` 下的夹具。

## 退出码

| 退出码 | 含义 | 输出 |
| --- | --- | --- |
| `0` | 导出成功且 24 项 check 全过；或 `--check-only` 全过 | 一行 JSON 摘要 + `EXPORT OK` / `{"checks":N,"ok":true}` + `CHECK OK` |
| `1` | 结构校验有 `ok=false`（导出后自检或 `--check-only` 均同） | 先打印 `{"checks":N,"ok":false}`，再打印 `CHECK FAIL <失败 check 名,逗号分隔>` |
| `2` | 参数错误 / 读盘失败 / JSON 解析失败 / 导出或校验异常 | `ERROR <原因>`（stderr） |

`--check-only` 传入不存在的文件属于读盘错误 → exit 2（不是 1）。
带值旗标（`--input/--md/--out/--template/--check-only`）后缺值 → `ERROR 缺少 <flag> 的值` + exit 2。
`--md` 与 `--input` 同时出现 → `ERROR --md 与 --input 不能同时使用` + exit 2；
`--no-number` 只与 `--md` 搭配（单独配 `--input` → exit 2）。

## 注意事项

- **暗标元数据已自动清空**：导出末尾会重写 `docProps/core.xml`，`dc:creator` 与
  `cp:lastModifiedBy` 置空、`dc:title` 只写本次文档标题，不会残留 `qidiwork` /
  `Bid Template v1`。这条由 check #24 `doc.metadata_anonymous` 兜底校验，
  因此**老产物**（P0c 之前生成、仍是模板元数据的 docx）跑 `--check-only` 会报
  `CHECK FAIL doc.metadata_anonymous`，重新导出一次即可。
- **页数用 `--pages` 实测**（本机 WPS COM，见上节「页数实测」）；不加 `--pages` 时
  `bid_export` 只做结构校验、不渲染。需要同时落 PDF 人工核对版式时用
  `gui/tests/bid-export/render-wps.ps1`，口径见 `docs/ops/wps-render-verify.md`。
- 输出目录不存在会自动创建；`--out` 相对路径按当前工作目录解析。
- 默认模板可用 `--template` 覆盖，换模板后 13 项 Bid 样式的 check 依然生效（模板漂移会直接报错）。
- 回归：`bash ../tests/bid-export/golden.sh` 跑 4 份 golden 夹具 + `md/demo.md`，期望 `GOLDEN OK (5/5)`。
