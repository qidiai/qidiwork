<script setup lang="ts">
import { computed, watch } from "vue";
import ChatView from "./ChatView.vue";
import PreviewPane from "./PreviewPane.vue";
import UsageSummary from "./UsageSummary.vue";
import { usePreviewTabs, closePreview, closeAllPreviews } from "../composables/usePreview";
import { useAgentState } from "../composables/useAgent";

const { openPreviews, activePreview } = usePreviewTabs();
const { sessionId } = useAgentState();
const selected = computed(() => openPreviews.value.find((p) => p.key === activePreview.value));
watch(sessionId, closeAllPreviews);
</script>

<template>
  <main class="main-tabs">
    <div class="workbench-toolbar"><span>办公工作台</span><UsageSummary /></div>
    <div class="tab-bar" aria-label="工作视图">
      <button class="tab" :class="{ active: !selected }" @click="activePreview = ''">任务工作台</button>
      <div v-for="tab in openPreviews" :key="tab.key" class="preview-tab" :class="{ active: tab.key === activePreview }">
        <button class="tab" :title="tab.name" @click="activePreview = tab.key">{{ tab.name }}</button>
        <button class="tab-close" :aria-label="`关闭 ${tab.name} 预览`" @click="closePreview(tab.key)">×</button>
      </div>
    </div>
    <div class="tab-body" :class="{ reviewing: selected }">
      <ChatView />
      <section v-if="selected" class="document-view" aria-label="成果文件预览">
        <div class="document-head"><strong :title="selected.name">{{ selected.name }}</strong><button @click="activePreview = ''">收起预览</button></div>
        <PreviewPane :key="selected.key" :task="selected.task" :name="selected.name" />
      </section>
    </div>
  </main>
</template>

<style scoped>
.workbench-toolbar { display: flex; flex-shrink: 0; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 8px; padding: 10px 16px; background: var(--bg-panel); border-bottom: 1px solid var(--border); color: var(--text-secondary); font-size: 12px; }
.main-tabs { display: flex; flex-direction: column; min-width: 0; min-height: 0; background: var(--bg-base); }
.tab-bar { display: flex; align-items: stretch; gap: 8px; min-height: 48px; padding: 8px 16px 0; border-bottom: 1px solid var(--border); background: var(--bg-panel); overflow-x: auto; }
.tab { border: 0; background: transparent; color: var(--text-secondary); padding: 9px 12px; font-size: 13px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 240px; }
.tab.active, .preview-tab.active { color: var(--accent); border-bottom: 2px solid var(--accent); }
.preview-tab { display: flex; min-width: 100px; align-items: center; }
.preview-tab.active .tab { color: var(--accent); }
.tab-close { border: 0; background: none; color: var(--text-secondary); padding: 6px; }
.tab-close:hover { color: var(--danger); }
.tab-body { flex: 1; min-height: 0; display: flex; }
.tab-body.reviewing { display: grid; grid-template-columns: minmax(280px, .8fr) minmax(0, 1.5fr); }
.document-view { display: flex; flex-direction: column; min-height: 0; min-width: 0; border-left: 1px solid var(--border); background: var(--bg-panel); overflow: hidden; }
.document-head { display: flex; align-items: center; gap: 12px; padding: 10px 16px; border-bottom: 1px solid var(--border); font-size: 12px; }
.document-head strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.document-head button { margin-left: auto; flex-shrink: 0; border: 0; background: transparent; color: var(--accent); font-size: 12px; }
</style>
