<script setup lang="ts">
// 中区 docx 预览 Tab:加载产物字节 → 矢量图形探测 → 渲染或降级提示。
// 降级判定依据 gui/spike-docx spike(2026-09-09):含 DrawingML 矢量
// 图形(总平面图类)docx-preview 渲染为空白,必须引导"系统打开"。
import { ref, onMounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { probeDocx, renderDocx, base64ToBytes } from "../services/docxPreview";
import { openPreviewArtifact } from "../composables/useOffice";
import { pushSystem } from "../composables/useAgent";

const props = defineProps<{ task: string; name: string }>();

type Phase =
  | { kind: "loading" }
  | { kind: "vector" } // 检测到矢量图形,降级
  | { kind: "error"; message: string }
  | { kind: "done" };

const phase = ref<Phase>({ kind: "loading" });
const containerRef = ref<HTMLElement | null>(null);

async function load(): Promise<void> {
  phase.value = { kind: "loading" };
  try {
    const b64 = await invoke<string>("office_read_file", {
      task: props.task,
      name: props.name,
    });
    const bytes = base64ToBytes(b64);
    const probe = await probeDocx(bytes);
    if (probe.vectorGraphics) {
      phase.value = { kind: "vector" };
      return;
    }
    if (containerRef.value) {
      containerRef.value.innerHTML = "";
      await renderDocx(bytes, containerRef.value);
    }
    phase.value = { kind: "done" };
  } catch (e) {
    phase.value = { kind: "error", message: String(e) };
  }
}

async function openInSystem(): Promise<void> {
  // 与 ArtifactPanel「打开」同一错误反馈模式(deepseek 审计 W3):
  // manifest 过期/文件被删时 office_open 会 reject,需给用户可见反馈。
  try {
    await openPreviewArtifact(props.task, props.name);
  } catch (e) {
    pushSystem(`打开失败:${String(e)}`);
  }
}

// 组件随 MainTabs 的 :key 销毁重建,props 生命周期内不变,无 watch 必要
onMounted(load);
</script>

<template>
  <div class="docx-preview">
    <div v-if="phase.kind === 'loading'" class="center-hint">正在加载预览…</div>
    <div v-else-if="phase.kind === 'vector'" class="center-hint fallback">
      <p class="fallback-title">该文档包含矢量图形（如示意图、平面图）</p>
      <p class="fallback-desc">网页预览无法呈现图形内容，请用 WPS/Office 打开查看完整效果。</p>
      <button class="open-btn" @click="openInSystem">用系统程序打开</button>
    </div>
    <div v-else-if="phase.kind === 'error'" class="center-hint">
      <p class="fallback-title">预览加载失败</p>
      <p class="fallback-desc">{{ phase.message }}</p>
      <button class="open-btn" @click="openInSystem">用系统程序打开</button>
    </div>
    <div v-show="phase.kind === 'done'" ref="containerRef" class="docx-container"></div>
  </div>
</template>

<style scoped>
.docx-preview {
  flex: 1;
  min-height: 0;
  overflow: auto;
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

.docx-container {
  flex: 1;
}

/* docx-preview 的分页包装:居中 + 页面留白 */
.docx-container :deep(.docx-wrapper) {
  background: var(--bg-panel);
  padding: 16px 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
}

/* 三线表等无边框单元格补浅灰网格线(对齐 WPS/Word 的表格线辅助显示):
   有 tcBorders 的单元格由 docx-preview 输出的内联样式优先,不受影响;
   无边框单元格原本完全不画线,审阅时无法辨认表格结构 */
.docx-container :deep(.docx-wrapper table td),
.docx-container :deep(.docx-wrapper table th) {
  border: 1px solid #c9c9c9;
}

.docx-container :deep(.docx-wrapper > section.docx) {
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.25);
  margin: 0;
}
</style>
