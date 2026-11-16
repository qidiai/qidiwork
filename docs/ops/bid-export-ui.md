# 标书导出 GUI（bid_export_md + 导出标书弹窗）

## 用途

把一份 Markdown 稿子一键导出成工程蓝标书 `.docx`，供不熟悉命令行的使用者操作。
底层与 CLI `bid-export --md` 完全同一条链路：
读 md → 以 **md 所在目录**为基准解析图片相对路径 → `markdown::parse_markdown_with`
→ `fill::export` 填充模板。模板固定用内置默认样式 `bid-engineering-blue-v1`，
弹窗因此不暴露 `style_id` / 图宽参数。

## 入口位置

- 主窗口右下角、状态栏上方的悬浮按钮 **「📄 导出标书」**（`gui/src/App.vue` 的
  `.bid-export-fab`）。按钮为 `position: fixed`，不参与 `app-main` 的网格布局，
  折叠侧栏 / 拖拽列宽都不受影响。
- 点击后打开 **`gui/src/components/BidExportModal.vue`** 弹窗；点遮罩、✕ 或「关闭」退出。

## 三个字段

| 字段 | 说明 |
|---|---|
| **Markdown 路径** | 待导出的 md 绝对路径。可手填，也可点「选择…」用系统文件框挑选（只过滤 `md` / `markdown`）。图片的相对路径按该文件所在目录解析。 |
| **输出 docx 路径** | 目标 `.docx` 绝对路径。父目录不存在会自动创建；同名文件直接覆盖。留空且刚选过 md 时，按同名 `.docx` 自动预填。 |
| **不自动编号**（复选框） | 勾选 = CLI 的 `--no-number`：章节标题原样输出、不补「第 N 章」编号。默认不勾选（自动编号）。 |

「导出」按钮调用后端命令：

```ts
invoke("bid_export_md", { mdPath, outPath, number })  // number = !noNumber
```

后端签名（`gui/src-tauri/src/commands.rs`，注册于 `lib.rs`）：

```rust
#[tauri::command]
pub async fn bid_export_md(
    app: tauri::AppHandle,
    md_path: String,
    out_path: String,
    number: Option<bool>,
) -> Result<crate::office::bid::model::BidExportResult, String>
```

## 成功与错误提示

- **成功**：结果区显示输出路径与统计 `章 / 段落 / 表格 / 图 / 文件大小`
  （对应 `BidExportResult` 的 `chapters` / `paragraphs` / `tables` / `figures` / `bytes`），
  同时在对话区推送一条「标书已导出：<路径>」系统消息。
- **失败**：后端 `String` 错误原文显示在弹窗内，常见几类：
  - `Markdown 路径为空` / `输出路径为空` —— 前端已提前拦，正常不会出现。
  - `读 Markdown 文件失败 <路径>: ...` —— 路径不存在或无权限。
  - `Markdown 解析失败: ...` —— md 结构不合法（无可用章节等）。
  - `输出路径为空` / `标题不能为空` / `至少需要一章` —— 来自 `BidExportRequest::validate`。
  - 模板相关错误 —— 安装包缺 `resources/templates/bid-engineering-blue-v1.docx`。
  - `选择文件失败: ...` —— 系统文件框不可用（`dialog:allow-open` 权限或运行环境问题）。
- 导出只写文件、不做结构校验；需要 24 项校验请走 CLI `bid_export`（见 `bid-export-cli.md`）。
