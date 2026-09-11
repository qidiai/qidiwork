<script setup lang="ts">
// 中区预览 Tab 统一分发:按产物扩展名选择渲染器。
// docx → DocxPreview(含矢量降级);xlsx/xls → XlsxPreview;pdf → PdfPreview;
// 其余格式(含 txt/md/html/png 等白名单格式)暂不支持网页预览,引导系统打开。
import { computed } from "vue";
import DocxPreview from "./DocxPreview.vue";
import XlsxPreview from "./XlsxPreview.vue";
import PdfPreview from "./PdfPreview.vue";
import { openPreviewArtifact } from "../composables/useOffice";
import { pushSystem } from "../composables/useAgent";

const props = defineProps<{ task: string; name: string }>();

type Renderer = "docx" | "xlsx" | "pdf" | "unsupported";

const renderer = computed<Renderer>(() => {
  const dot = props.name.lastIndexOf(".");
  const ext = dot >= 0 ? props.name.slice(dot + 1).toLowerCase() : "";
  if (ext === "docx") return "docx";
  if (ext === "xlsx" || ext === "xls") return "xlsx";
  if (ext === "pdf") return "pdf";
  return "unsupported";
});

async function openInSystem(): Promise<void> {
  try {
    await openPreviewArtifact(props.task, props.name);
  } catch (e) {
    pushSystem(`打开失败:${String(e)}`);
  }
}
</script>

<template>
  <DocxPreview v-if="renderer === 'docx'" :task="task" :name="name" />
  <XlsxPreview v-else-if="renderer === 'xlsx'" :task="task" :name="name" />
  <PdfPreview v-else-if="renderer === 'pdf'" :task="task" :name="name" />
  <div v-else class="center-hint">
    <p class="fallback-title">该格式暂不支持网页预览</p>
    <p class="fallback-desc">{{ name }}</p>
    <button class="open-btn" @click="openInSystem">用系统程序打开</button>
  </div>
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
</style>
