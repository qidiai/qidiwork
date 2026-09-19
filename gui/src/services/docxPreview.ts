// docx 预览服务:docx-preview.js 渲染 + 矢量图形检测(spike 结论:
// gui/spike-docx,2026-09-09)。文字/表格类标书可渲染;含 DrawingML
// 矢量图形(如施工总平面图)的文档渲染为空白,必须降级"系统打开"。
// base64ToBytes 已移至 services/bytes.ts(避免重依赖被顺带引入)。
import JSZip from "jszip";
import { renderAsync } from "docx-preview";

export interface DocxProbe {
  drawings: number;
  blips: number;
  /** true = 存在位图之外的矢量图形,网页预览不可用,应降级系统打开 */
  vectorGraphics: boolean;
}

/** 解包 docx 数图形:有 <w:drawing> 但 <a:blip>(位图引用)更少 → 含矢量形状 */
export async function probeDocx(bytes: Uint8Array): Promise<DocxProbe> {
  const zip = await JSZip.loadAsync(bytes);
  const xml = (await zip.file("word/document.xml")?.async("string")) ?? "";
  const drawings = (xml.match(/<w:drawing>/g) ?? []).length;
  const blips = (xml.match(/<a:blip\b/g) ?? []).length;
  return {
    drawings,
    blips,
    vectorGraphics: drawings > blips,
  };
}

/** 渲染 docx 到容器(spike 同参数;useBase64URL 使图片走 data:,契合 CSP img-src data:) */
export async function renderDocx(bytes: Uint8Array, container: HTMLElement): Promise<void> {
  await renderAsync(bytes, container, undefined, {
    inWrapper: true,
    ignoreWidth: false,
    ignoreHeight: false,
    renderHeaders: true,
    renderFooters: true,
    renderFootnotes: true,
    breakPages: true,
    experimental: true,
    useBase64URL: true,
  });
  markThreeLineTables(container);
}

/** 三线表标记(k3 补审计):仅当表内存在内联边框单元格(docx-preview 输出
 * 为 td 的内联 style)时加 has-cell-borders 类,样式表再为无内联边框的
 * 单元格补浅灰网格线。布局用/设计无边框的表格不加线。 */
function markThreeLineTables(container: HTMLElement): void {
  container.querySelectorAll("table").forEach((tbl) => {
    const cells = tbl.querySelectorAll("td, th");
    for (const cell of cells) {
      const style = (cell as HTMLElement).style;
      for (const side of ["top", "right", "bottom", "left"]) {
        const v = style.getPropertyValue(`border-${side}-style`);
        if (v && v !== "none" && v !== "hidden") {
          tbl.classList.add("has-cell-borders");
          return;
        }
      }
    }
  });
}
