// docx 预览服务:docx-preview.js 渲染 + 矢量图形检测(spike 结论:
// gui/spike-docx,2026-09-09)。文字/表格类标书可渲染;含 DrawingML
// 矢量图形(如施工总平面图)的文档渲染为空白,必须降级"系统打开"。
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
}

/** base64 → 字节(IPC 传 String,前端解码) */
export function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}
