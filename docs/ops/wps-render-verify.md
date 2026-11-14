# 用本机 WPS 做 docx 渲染实测（页数 / PDF / 吞图抽检）

**日期**：2026-11-14
**适用**：标书导出（P0a/P0b）等一切需要"渲染器实测页数"的 docx 交付（R-025 / R-022）。

## 1. 环境事实（2026-11-14 实测；此前判断有误，以此为准）

| 项 | 事实 |
|---|---|
| WPS 安装位置 | **`F:\wps\WPS Office\12.1.0.29690\office6\wps.exe`**（用户态安装，卸载项在 `HKCU\...\Uninstall\Kingsoft Office`）。另存旧版 `12.1.0.29161` |
| COM ProgID | `KWPS.Application`（WPS Writer）已注册；`Word.Application` 亦被注册（兼容层） |
| 易误判点 | `C:\Program Files (x86)\Kingsoft\office6` **只剩 `wpscloudsvr.exe`**（云同步），`C:\...\Microsoft Office\Office12` 只剩 `STSLIST.DLL`。只搜 C 盘会得出"没装 WPS"的错误结论 |
| 台式机 `192.168.1.222` | Ubuntu，**无** wps / libreoffice / soffice（`dpkg -l` 命中 0，`/usr/bin` 无相关命令）→ **不能分担负测** |
| LibreOffice | 本机与台式机均无 |

## 2. 用法（已入库为测试资产）

```bash
cd G:/qidiwork/gui/src-tauri
MSYS_NO_PATHCONV=1 powershell -NoProfile -ExecutionPolicy Bypass \
  -File ../tests/bid-export/render-wps.ps1 \
  -Docx ../tests/bid-export/out/p0a-sample.docx \
  -Pdf  ../tests/bid-export/out/p0a-sample.pdf
```

输出：`PAGES=<页数>` / `WORDS=<字数>` / `PDF_SAVED=True` / `PDF_BYTES=<n>`。
实现要点：`Documents.Open(path, false, true)` 只读打开 → `Repaginate()` → `ComputeStatistics(2)`（wdStatisticPages）→ `SaveAs(pdf, 17)`（失败回退 `ExportAsFixedFormat`）→ 只 `Close` 本文档。

## 3. 三条硬注意事项

1. **COM 会附着到用户已开的 WPS 实例**。脚本先探测 `Get-Process wps`，**只有我们自己拉起的实例才 `Quit()`**——否则会把用户正在编辑的窗口一起关掉。派活时也要写清这条。
2. **页数是渲染器实测值，不是估算**。可直接作为 R-025 证据；`字符数÷每页字数` 一律禁止。
3. **WPS ≠ Word**。WPS 的 Word 兼容实现能覆盖本文档用到的样式/表格/图片/分页，但 Word 侧行为（尤其 P0b 的 TOC 域更新）仍需在装有 Word 的机器补测；报告里必须写渲染器名称+版本。

## 4. 实测基线（P0a 样本，2026-11-14）

| 指标 | 值 |
|---|---|
| 输入 | `gui/tests/bid-export/out/p0a-sample.docx`（18814 B，2 章 / 4 段 / 1 表 3×3 / 1 图） |
| 渲染器 | WPS Office **12.1.0.29690** |
| 页数 | **3**（`ComputeStatistics(2)`；PDF 独立复核 `/Count 3`、page 对象 3） |
| 字数 | 283（`ComputeStatistics(0)`） |
| 导出 PDF | 98570 B，1.7；图像 XObject **1** 个（图未被固定行距吞掉，R-022 初检通过） |
| 嵌入字体 | `SimSun`（宋体）+ `Calibri`（数字/西文） |
| 复跑一致性 | 两次运行 PDF 字节数一致（98570） |

## 5. 仍缺（不冒充）

- 逐页截图目视抽检（图注跟随、表格跨页断裂、文字重叠 R-032）——需人或视觉席看图。
- Word 侧打开与 TOC 更新后的页数（P0b 引入目录域后必须补）。
