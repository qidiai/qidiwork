<script setup lang="ts">
// 中区 Markdown 预览 Tab:office_read_file 取字节 → UTF-8 解码 →
// renderMarkdown(marked + DOMPurify)→ 与聊天同一条安全渲染链
// (白名单标签/https?+mailto URI/外链走系统浏览器)。md 已在
// OPEN_ALLOWED_EXTENSIONS 白名单,office_read_file 的校验链直接复用。
// 上限 2MB:markdown 全量渲染是主线程活,超大文件引导系统打开。
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { renderMarkdown, handleLinkClick } from "../services/render";
import { base64ToBytes } from "../services/docxPreview";
import { openPreviewArtifact } from "../composables/useOffice";
import { pushSystem } from "../composables/useAgent";

const props = defineProps<{ task: string; name: string }>();

const MD_MAX_BYTES = 2 * 1024 * 1024;

type Phase =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "too-large"; size: number }
  | { kind: "done" };

const phase = ref<Phase>({ kind: "loading" });
const html = ref("");

async function load(): Promise<void> {
  phase.value = { kind: "loading" };
  try {
    const b64 = await invoke<string>("office_read_file", {
      task: props.task,
      name: props.name,
      // 后端读文件前短路(k3 审计 Note5),免超大文件白走 IPC
      maxBytes: MD_MAX_BYTES,
    });
    const bytes = base64ToBytes(b64);
    if (bytes.length > MD_MAX_BYTES) {
      phase.value = { kind: "too-large", size: bytes.length };
      return;
    }
    // UTF-8 宽松解码(替换非法序列)+ 去 BOM;markdown 按不可信输入处理
    let text = new TextDecoder("utf-8").decode(bytes);
    if (text.startsWith("\uFEFF")) text = text.slice(1);
    html.value = renderMarkdown(text);
    phase.value = { kind: "done" };
  } catch (e) {
    phase.value = { kind: "error", message: String(e) };
  }
}

async function openInSystem(): Promise<void> {
  try {
    await openPreviewArtifact(props.task, props.name);
  } catch (e) {
    pushSystem(`打开失败:${String(e)}`);
  }
}

function onBodyClick(e: MouseEvent): void {
  handleLinkClick(e);
}

onMounted(load);
</script>

<template>
  <div v-if="phase.kind === 'loading'" class="center-hint">加载中…</div>
  <div v-else-if="phase.kind === 'error'" class="center-hint">
    <p class="fallback-title">预览读取失败</p>
    <p class="fallback-desc">{{ phase.message }}</p>
    <button class="open-btn" @click="openInSystem">用系统程序打开</button>
  </div>
  <div v-else-if="phase.kind === 'too-large'" class="center-hint">
    <p class="fallback-title">文件 {{ (phase.size / (1024 * 1024)).toFixed(1) }}MB,超过网页预览上限 2MB</p>
    <button class="open-btn" @click="openInSystem">用系统程序打开</button>
  </div>
  <!-- eslint-disable-next-line vue/no-v-html: renderMarkdown 内部已经 DOMPurify 消毒 -->
  <div v-else class="md-body md" @click="onBodyClick" v-html="html"></div>
</template>

<style scoped>
.center-hint {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--text-secondary);
  padding: 24px;
  text-align: center;
}

.fallback-title {
  font-size: 16px;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0;
}

.fallback-desc {
  font-size: var(--font-size-sm);
  margin: 0;
  max-width: 420px;
  word-break: break-all;
}

.open-btn {
  margin-top: 8px;
  padding: 8px 24px;
  background: var(--accent);
  color: #fff;
  border: none;
  border-radius: var(--radius);
  cursor: pointer;
  font-size: var(--font-size-sm);
}

.open-btn:hover {
  filter: brightness(1.1);
}

.md-body {
  flex: 1;
  overflow-y: auto;
  padding: 20px 28px;
  line-height: 1.7;
  word-break: break-word;
}

.md-body :deep(h1),
.md-body :deep(h2),
.md-body :deep(h3),
.md-body :deep(h4) {
  margin: 18px 0 8px;
  line-height: 1.4;
}

.md-body :deep(p) {
  margin: 8px 0;
}

.md-body :deep(ul),
.md-body :deep(ol) {
  margin: 8px 0;
  padding-left: 24px;
}

.md-body :deep(code) {
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 1px 5px;
  font-size: 0.9em;
}

.md-body :deep(pre) {
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 10px 12px;
  overflow-x: auto;
}

.md-body :deep(pre code) {
  border: none;
  background: none;
  padding: 0;
}

.md-body :deep(blockquote) {
  margin: 8px 0;
  padding: 4px 12px;
  border-left: 3px solid var(--border);
  color: var(--text-secondary);
}

.md-body :deep(table) {
  border-collapse: collapse;
  margin: 10px 0;
}

.md-body :deep(th),
.md-body :deep(td) {
  border: 1px solid var(--border);
  padding: 5px 10px;
}

.md-body :deep(hr) {
  border: none;
  border-top: 1px solid var(--border);
  margin: 14px 0;
}
</style>
