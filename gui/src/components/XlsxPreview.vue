<script setup lang="ts">
// 中区 xlsx 预览 Tab:office_read_file 取字节 → SheetJS 解析 → 分 sheet
// 纯 DOM 建表(安全边界见 services/sheetPreview.ts 头注)。
import { ref, nextTick, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { WorkBook } from "xlsx";
import { base64ToBytes } from "../services/docxPreview";
import { parseWorkbook, renderSheet } from "../services/sheetPreview";
import { openPreviewArtifact } from "../composables/useOffice";
import { pushSystem } from "../composables/useAgent";

const props = defineProps<{ task: string; name: string }>();

type Phase =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "done" };

const phase = ref<Phase>({ kind: "loading" });
const sheetNames = ref<string[]>([]);
const activeSheet = ref("");
const truncated = ref(false);
const containerRef = ref<HTMLElement | null>(null);
// workbook 由组件实例持有:模块级缓存会在异步加载期间被其他实例串档(k3 补充审计)
let wb: WorkBook | null = null;

async function load(): Promise<void> {
  phase.value = { kind: "loading" };
  sheetNames.value = [];
  activeSheet.value = "";
  try {
    const b64 = await invoke<string>("office_read_file", {
      task: props.task,
      name: props.name,
    });
    const bytes = base64ToBytes(b64);
    const parsed = parseWorkbook(bytes);
    wb = parsed.wb;
    sheetNames.value = parsed.names;
    if (sheetNames.value.length === 0) {
      phase.value = { kind: "error", message: "工作簿不含任何工作表" };
      return;
    }
    // 先切 done 让 v-else 分支(含容器)挂载,再渲染首个 sheet
    phase.value = { kind: "done" };
    await nextTick();
    await showSheet(sheetNames.value[0]!);
  } catch (e) {
    phase.value = { kind: "error", message: String(e) };
  }
}

async function showSheet(name: string): Promise<void> {
  if (!wb || !containerRef.value) return;
  activeSheet.value = name;
  const result = renderSheet(wb, name, containerRef.value);
  truncated.value = result.truncated;
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
  wb = null;
});
</script>

<template>
  <div class="xlsx-preview">
    <div v-if="phase.kind === 'loading'" class="center-hint">正在加载预览…</div>
    <div v-else-if="phase.kind === 'error'" class="center-hint">
      <p class="fallback-title">预览加载失败</p>
      <p class="fallback-desc">{{ phase.message }}</p>
      <button class="open-btn" @click="openInSystem">用系统程序打开</button>
    </div>
    <template v-else>
      <div class="sheet-bar">
        <button
          v-for="s in sheetNames"
          :key="s"
          class="sheet-tab"
          :class="{ active: s === activeSheet }"
          @click="showSheet(s)"
        >
          {{ s }}
        </button>
      </div>
      <div v-if="truncated" class="trunc-hint">
        仅预览前 500 行 × 100 列,完整内容请用系统程序打开
      </div>
      <div ref="containerRef" class="sheet-container"></div>
    </template>
  </div>
</template>

<style scoped>
.xlsx-preview {
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

.sheet-bar {
  display: flex;
  gap: 4px;
  padding: 6px 12px;
  border-bottom: 1px solid var(--border);
  overflow-x: auto;
  flex: none;
}

.sheet-tab {
  border: 1px solid var(--border);
  background: var(--bg-panel);
  border-radius: var(--radius);
  padding: 4px 12px;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  cursor: pointer;
  white-space: nowrap;
}

.sheet-tab.active {
  background: var(--bg-active);
  color: var(--text-primary);
  font-weight: 600;
}

.trunc-hint {
  padding: 6px 12px;
  font-size: var(--font-size-sm);
  color: var(--warning, #b58900);
  border-bottom: 1px solid var(--border);
  flex: none;
}

.sheet-container {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 8px;
}
</style>

<style>
/* 表格样式不能 scoped:表格是命令式 DOM 创建的,不带 scope 属性 */
.sheet-table {
  border-collapse: collapse;
  font-size: var(--font-size-sm);
}

.sheet-table td {
  border: 1px solid var(--border);
  padding: 3px 8px;
  white-space: nowrap;
  max-width: 320px;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
