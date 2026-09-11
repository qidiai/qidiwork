<script setup lang="ts">
// 中区 PDF 预览 Tab:office_read_file 取字节 → %PDF 校验 → blob: URL →
// WebView2 内置查看器(iframe)。blob 由本组件创建,卸载时 revoke。
// 沙箱说明(step 审计 B2 实证):sandbox="allow-scripts" 会使 opaque
// origin 无法 fetch 应用源的 blob,查看器直接加载失败;而补 allow-
// same-origin 后沙箱形同虚设。PDF 内嵌 JS 由 Chromium 隔离在查看器的
// 扩展源执行,进不了本页源;若未来需更强隔离,换 pdf.js worker 方案。
import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { base64ToBytes } from "../services/docxPreview";
import { openPreviewArtifact } from "../composables/useOffice";
import { pushSystem } from "../composables/useAgent";

const props = defineProps<{ task: string; name: string }>();

type Phase =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "done" };

const phase = ref<Phase>({ kind: "loading" });
const blobUrl = ref("");

async function load(): Promise<void> {
  phase.value = { kind: "loading" };
  try {
    const b64 = await invoke<string>("office_read_file", {
      task: props.task,
      name: props.name,
    });
    const bytes = base64ToBytes(b64);
    // PDF 首字节校验(%PDF-):防 manifest 指向非 PDF 内容被误投给查看器
    const magic = new TextDecoder().decode(bytes.slice(0, 5));
    if (!magic.startsWith("%PDF")) {
      phase.value = { kind: "error", message: "文件头不是 %PDF,可能已损坏或被替换" };
      return;
    }
    const blob = new Blob([bytes], { type: "application/pdf" });
    blobUrl.value = URL.createObjectURL(blob);
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

onMounted(load);
onUnmounted(() => {
  if (blobUrl.value) URL.revokeObjectURL(blobUrl.value);
});
</script>

<template>
  <div class="pdf-preview">
    <div v-if="phase.kind === 'loading'" class="center-hint">正在加载预览…</div>
    <div v-else-if="phase.kind === 'error'" class="center-hint">
      <p class="fallback-title">预览加载失败</p>
      <p class="fallback-desc">{{ phase.message }}</p>
      <button class="open-btn" @click="openInSystem">用系统程序打开</button>
    </div>
    <iframe
      v-else-if="blobUrl"
      :src="blobUrl"
      class="pdf-frame"
      title="PDF 预览"
      referrerpolicy="no-referrer"
    ></iframe>
  </div>
</template>

<style scoped>
.pdf-preview {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

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

.pdf-frame {
  flex: 1;
  border: none;
}
</style>
