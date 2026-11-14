# P0b 渲染页数基线（WPS 12.1.0.29690）

**日期**：2026-11-15
**用途**：R-025「页数必须渲染器实测」的基线；P1 起以本表做页数预算校准。
**输入**：`gui/tests/bid-export/out/*.docx`（由 `golden.sh` + `bid_smoke` 生成，可复现）
**渲染器**：WPS Office **12.1.0.29690**（COM `KWPS.Application`，脚本 `render-wps.ps1`）

## 基线表

| 产物 | 章节 | 段落 | 表 | 图 | 图注 | **页数** | 字数 | PDF 字节 |
|---|---|---|---|---|---|---|---|---|
| `p0a-sample.docx` | 2 | 4 | 1 | 1 | 1 | **3** | 283 | 114,375 |
| `g1-construction.docx` | 5 | 15 | 2 | 2 | 2 | **6** | 696 | 142,885 |
| `g2-gantt.docx` | 1 | 2 | 0 | 1（wide） | 1 | **2** | 95 | 94,611 |
| `g3-siteplan.docx` | 2 | 3 | 0 | 3（3up） | 3 | **3** | 155 | 99,900 |
| `g4-supervision.docx` | 8 | 24 | 1 | 6（2×3up） | 2 | **9** | 738 | 136,926 |

## 交叉证据（PDF 内容流独立复核）

| 产物 | 页对象 | 图片绘制次数 | 出现字号（×20） | 章标题色 #253D57 |
|---|---|---|---|---|
| p0a-sample | 3 | 1 | 9/12/15pt | ✅ |
| g1-construction | 6 | 2 | 9/12/15pt | ✅ |
| g2-gantt | 2 | 1 | 9/12/15pt | ✅ |
| g3-siteplan | 3 | 3 | 9/12/15pt | ✅ |
| g4-supervision | 9 | 6 | 9/12/15pt | ✅ |

> 图片绘制次数与 docx 内 `w:drawing` 计数逐一吻合（3up 的 3 张同源图在 PDF 中复用一个 XObject、绘制 3 次，故必须数 `Do` 操作而不是 XObject 个数）。

## 目录域（TOC）实测

用 WPS COM 打开 `p0a-sample.docx` → `Fields.Update()` → 保存副本，`document.xml` 中出现：

```
TOC \o "1-3" \h \z \u  →  separate →  HYPERLINK \l _Toc15880 …  第一章 工程概况  …  PAGEREF _Toc15880  2
```

即：**2 章均被收集**，无"错误！未找到目录项"。关键前提是模板 `document.xml.rels` **必须声明 styles/settings 关系**（见下节）。

## 三个必须知道的坑（实测）

1. **`word/_rels/document.xml.rels` 缺 `styles` 关系 → 全套样式静默失效**
   症状：PDF 里标题不是 15pt 深蓝、表格无底色、TOC 报"未找到目录项"；WPS 打开再存盘会把 13 个 `Bid*` 样式全部丢掉。
   修法：rels 必须有 `relationships/styles → styles.xml`（settings 同理），对应提交 `B6`。
   **这条只靠 XML 断言抓不到**（document.xml 里 `pStyle` 引用是对的），必须跑渲染。
2. **WPS 另存会重写样式**：COM `Save()` 后自定义样式被转成内置样式 id（`w:val="2"` 等），`Bid*` 消失。
   → **不要用 WPS 保存过的 docx 当 golden / 回灌仓库**；WPS 只用于"打开 + 转 PDF + 取页数"。
3. **TOC 依赖大纲级别**：`BidTitle1..4` 必须有 `w:outlineLvl`（0..3），且章节标题段内联再写一份 `outlineLvl 0` 做双保险；文档标题段写 `outlineLvl 9`（不进目录）。缺了就是空目录。

## 复现

```bash
cd G:/qidiwork/gui/src-tauri
bash ../tests/bid-export/golden.sh            # 4 份 golden + 断言
MSYS_NO_PATHCONV=1 powershell -NoProfile -ExecutionPolicy Bypass \
  -File ../tests/bid-export/render-wps.ps1 \
  -Docx ../tests/bid-export/out/g3-siteplan.docx \
  -Pdf  ../tests/bid-export/out/g3-siteplan.pdf
```

## 尚缺（不冒充）

- **Word 侧**未测（本机仅 WPS；台式机 Ubuntu 无 wps/libreoffice）。
- 逐页截图目视抽检（图注跟随、表格跨页、R-032 文字重叠）未做。
- 页数随内容变化，本表只对上述 5 份 fixture 有效；P1 需按真实项目重建校准。
