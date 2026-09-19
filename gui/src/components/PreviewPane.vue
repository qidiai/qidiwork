<script setup lang="ts">
// 中区预览 Tab 统一分发:按产物扩展名选择渲染器。
// docx → DocxPreview(含矢量降级);xlsx/xls → XlsxPreview;pdf → PdfPreview;
// md/markdown → MdPreview(marked+DOMPurify 安全渲染链);
// 其余格式(含 txt/html/png 等白名单格式)暂不支持网页预览,引导系统打开。
import { computed, ref, defineAsyncComponent } from "vue";
// 预览器全部异步分包:jszip/docx-preview/SheetJS/marked 等重依赖不进
// 首屏 chunk,首次打开对应格式的预览 Tab 时才加载(启动性能审计)
const DocxPreview = defineAsyncComponent(() => import("./DocxPreview.vue"));
const XlsxPreview = defineAsyncComponent(() => import("./XlsxPreview.vue"));
const PdfPreview = defineAsyncComponent(() => import("./PdfPreview.vue"));
const MdPreview = defineAsyncComponent(() => import("./MdPreview.vue"));
import { openPreviewArtifact } from "../composables/useOffice";
import { pushSystem } from "../composables/useAgent";

const props = defineProps<{ task: string; name: string }>();

// 手动刷新:agent 重写产物后重挂当前渲染器(各预览组件只在挂载时读取)
const reloadNonce = ref(0);

type Renderer = "docx" | "xlsx" | "pdf" | "md" | "unsupported";

const renderer = computed<Renderer>(() => {
  const dot = props.name.lastIndexOf(".");
  const ext = dot >= 0 ? props.name.slice(dot + 1).toLowerCase() : "";
  if (ext === "docx") return "docx";
  if (ext === "xlsx" || ext === "xls") return "xlsx";
  if (ext === "pdf") return "pdf";
  if (ext === "md" || ext === "markdown") return "md";
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
  <div class="preview-wrap">
    <!-- :key 强制重建实例(k3 审计 W1):文件切换 + 手动刷新都走重挂载,
         各预览组件只在挂载时读取文件 -->
    <DocxPreview v-if="renderer === 'docx'" :key="task + ':' + name + ':' + reloadNonce" :task="task" :name="name" />
    <XlsxPreview v-else-if="renderer === 'xlsx'" :key="task + ':' + name + ':' + reloadNonce" :task="task" :name="name" />
    <PdfPreview v-else-if="renderer === 'pdf'" :key="task + ':' + name + ':' + reloadNonce" :task="task" :name="name" />
    <MdPreview v-else-if="renderer === 'md'" :key="task + ':' + name + ':' + reloadNonce" :task="task" :name="name" />
    <div v-else class="center-hint">
      <p class="fallback-title">该格式暂不支持网页预览</p>
      <p class="fallback-desc">{{ name }}</p>
      <button class="open-btn" @click="openInSystem">用系统程序打开</button>
    </div>
    <button
      v-if="renderer !== 'unsupported'"
      class="reload-btn"
      title="重新读取文件(agent 重写产物后点此刷新)"
      @click="reloadNonce++"
    >⟳ 刷新</button>
  </div>
</template>

<style scoped>
.preview-wrap {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  position: relative;
}

/* 悬浮刷新:幽灵按钮,不与正文抢空间 */
.reload-btn {
  position: absolute;
  top: 8px;
  right: 14px;
  z-index: 5;
  border: 1px solid var(--border);
  background: var(--bg-panel);
  border-radius: var(--radius);
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  padding: 2px 10px;
  cursor: pointer;
  opacity: 0.75;
}

.reload-btn:hover {
  opacity: 1;
  color: var(--text-primary);
  border-color: var(--accent);
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
