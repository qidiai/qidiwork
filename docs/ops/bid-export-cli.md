# 标书导出 CLI（bid_export）

## 用途

把一份请求 JSON（`BidExportRequest`）灌进工程蓝标书模板，产出可直接交付的 `.docx`，
并在写盘后立刻跑 24 项结构校验（样式 / 页边距 / 目录 / 章节 / 表格 / 图注 / 暗标元数据等）。
适合 CI、批量出稿和人工排障：退出码机器可读，失败会点名不合格的 check。

CLI 位置：`gui/src-tauri/src/bin/bid_export.rs`（`cargo run --bin bid_export`）。
默认模板：`gui/src-tauri/resources/templates/bid-engineering-blue-v1.docx`。

## 用法

在工作目录 `gui/src-tauri` 下执行：

```bash
# 1) 导出（--out 覆盖请求 JSON 里的 output_path）
bid_export --input <req.json> --out <out.docx> [--template <tpl.docx>]

# 2) 只校验（不导出，跑 verify::verify(&p, None)）
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

## 请求 JSON 示例

顶层字段对应 `office::bid::model::BidExportRequest`：`title`、`chapters[]`、
可选 `output_path`（可省略，被 `--out` 覆盖）、`style_id`、`figure_width_cm`。
`chapters[].blocks[]` 用内部标签 `type` 区分三种块：

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

- `type`：`paragraph` / `table` / `figure`（`snake_case`）。
- `figure.layout`：`single` / `wide` / `three_up`；`three_up` 不足三张时补空单元格，不拉伸、不报错。
- `figure.caption_mode`：`per_image`（逐图图注，默认）/ `per_group`（整组一条图注）。
- `figure.items[].path` 支持相对路径（按**请求 JSON 所在目录**解析，与 `bid_smoke` 同口径）与绝对路径。
- `ai_generated: true` 时图注自动追加「（AI 生成示意图）」标识。
- 请求 JSON 里的多余字段（如 fixture 的 `expect`）被忽略，可直接复用 `gui/tests/bid-export/` 下的夹具。

## 退出码

| 退出码 | 含义 | 输出 |
| --- | --- | --- |
| `0` | 导出成功且 24 项 check 全过；或 `--check-only` 全过 | 一行 JSON 摘要 + `EXPORT OK` / `{"checks":N,"ok":true}` + `CHECK OK` |
| `1` | 已产出 docx，但结构校验有 `ok=false` | `CHECK FAIL <失败 check 名,逗号分隔>` |
| `2` | 参数错误 / 读盘失败 / JSON 解析失败 / 导出或校验异常 | `ERROR <原因>`（stderr） |

`--check-only` 传入不存在的文件属于读盘错误 → exit 2（不是 1）。

## 注意事项

- **暗标元数据已自动清空**：导出末尾会重写 `docProps/core.xml`，`dc:creator` 与
  `cp:lastModifiedBy` 置空、`dc:title` 只写本次文档标题，不会残留 `qidiwork` /
  `Bid Template v1`。这条由 check #24 `doc.metadata_anonymous` 兜底校验，
  因此**老产物**（P0c 之前生成、仍是模板元数据的 docx）跑 `--check-only` 会报
  `CHECK FAIL doc.metadata_anonymous`，重新导出一次即可。
- **页数不要用本 CLI 取**：`bid_export` 只做结构校验，不渲染。页数请用
  `gui/tests/bid-export/render-wps.ps1`（WPS 转 PDF 后读页数），口径见
  `docs/ops/wps-render-verify.md`。
- 输出目录不存在会自动创建；`--out` 相对路径按当前工作目录解析。
- 默认模板可用 `--template` 覆盖，换模板后 13 项 Bid 样式的 check 依然生效（模板漂移会直接报错）。
- 回归：`bash ../tests/bid-export/golden.sh` 跑 4 份 golden 夹具，期望 `GOLDEN OK (4/4)`。
