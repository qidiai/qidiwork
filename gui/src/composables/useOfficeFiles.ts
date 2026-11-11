import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

export interface OfficeDocument {
  name: string;
  path: string;
  size: number;
}

export const DOCUMENT_EXTENSIONS = ["doc", "docx", "xls", "xlsx", "ppt", "pptx", "pdf", "txt", "md", "csv"];
export const MAX_DOCUMENT_BYTES = 32 * 1024 * 1024;
export const documentsBySession = ref<Record<string, OfficeDocument[]>>({});
const revisions = new Map<string, number>();

/** 已删除会话:迟到结果不得复活缓存(只失效缓存,不删磁盘副本)。 */
const removedSessions = new Set<string>();

/** 会话被删除时调用:失效在途读取并清掉本会话资料缓存。 */
export function forgetDocuments(sessionId: string): void {
  if (!sessionId) return;
  removedSessions.add(sessionId);
  revisions.set(sessionId, (revisions.get(sessionId) ?? 0) + 1);
  delete documentsBySession.value[sessionId];
}

export function readFileData(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(",")[1] ?? "");
    reader.onerror = () => reject(new Error(`无法读取「${file.name}」，请重新选择文件`));
    reader.readAsDataURL(file);
  });
}

export async function loadDocuments(sessionId: string): Promise<void> {
  const revision = (revisions.get(sessionId) ?? 0) + 1;
  revisions.set(sessionId, revision);
  const docs = await invoke<OfficeDocument[]>("session_documents", { sessionId });
  if (removedSessions.has(sessionId)) return;
  if (revisions.get(sessionId) === revision) documentsBySession.value[sessionId] = docs;
}

export async function importDocument(sessionId: string, file: File): Promise<OfficeDocument> {
  if (removedSessions.has(sessionId)) throw new Error(removedMessage(sessionId));
  const data = await readFileData(file);
  if (removedSessions.has(sessionId)) throw new Error(removedMessage(sessionId));
  const doc = await invoke<OfficeDocument>("session_import_document", { sessionId, name: file.name, data });
  // 后端可能已落盘成功,但会话已被删:不写缓存,向调用方明确中止
  if (removedSessions.has(sessionId)) throw new Error(removedMessage(sessionId));
  revisions.set(sessionId, (revisions.get(sessionId) ?? 0) + 1);
  const docs = documentsBySession.value[sessionId] ?? [];
  documentsBySession.value[sessionId] = [...docs.filter((d) => d.path !== doc.path), doc];
  return doc;
}

function removedMessage(sessionId: string): string {
  return `会话「${sessionId}」已删除，本次资料导入已中止`;
}
