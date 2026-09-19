// 二进制解码工具:独立模块,避免各预览组件为拿 base64ToBytes 而
// 静态引入 docxPreview 的 jszip/docx-preview 重依赖(分包审计)。

/** base64 → 字节(IPC 传 String,前端解码)。返回类型钉住 ArrayBuffer
 * 背衬:new Blob()/XLSX.read 等 API 在 TS 5.7+ 拒绝 ArrayBufferLike。 */
export function base64ToBytes(b64: string): Uint8Array<ArrayBuffer> {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}
