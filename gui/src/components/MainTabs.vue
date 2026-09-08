<script setup lang="ts">
import { ref } from "vue";
import ChatView from "./ChatView.vue";

// 中区多 Tab:对话为常驻 Tab;产物预览 Tab 由 P1 点击产物卡片时打开。
type TabKey = string;
const tabs = ref<{ key: TabKey; label: string; closable: boolean }[]>([
  { key: "chat", label: "对话", closable: false },
]);
const activeTab = ref<TabKey>("chat");

function closeTab(key: TabKey) {
  const idx = tabs.value.findIndex((t) => t.key === key);
  if (idx < 0 || !tabs.value[idx].closable) return;
  tabs.value.splice(idx, 1);
  if (activeTab.value === key) {
    activeTab.value = tabs.value[Math.max(0, idx - 1)]!.key;
  }
}
</script>

<template>
  <main class="main-tabs">
    <div class="tab-bar">
      <div
        v-for="tab in tabs"
        :key="tab.key"
        class="tab"
        :class="{ active: tab.key === activeTab }"
        @click="activeTab = tab.key"
      >
        <span>{{ tab.label }}</span>
        <span
          v-if="tab.closable"
          class="tab-close"
          @click.stop="closeTab(tab.key)"
          >×</span
        >
      </div>
    </div>
    <div class="tab-body">
      <ChatView v-if="activeTab === 'chat'" />
      <div v-else class="empty-hint preview-placeholder">
        文档预览将在产物卡片接入后可用(方案 P1)
      </div>
    </div>
  </main>
</template>

<style scoped>
.main-tabs {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  background: var(--bg-base);
}

.tab-bar {
  display: flex;
  gap: 4px;
  padding: 8px 12px 0;
  border-bottom: 1px solid var(--border);
  background: var(--bg-panel);
}

.tab {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 7px 14px;
  border-radius: 8px 8px 0 0;
  cursor: pointer;
  color: var(--text-secondary);
  border: 1px solid transparent;
  border-bottom: none;
  user-select: none;
}

.tab.active {
  background: var(--bg-base);
  border-color: var(--border);
  color: var(--text-primary);
  font-weight: 600;
}

.tab-close {
  color: var(--text-disabled);
  padding: 0 2px;
}

.tab-close:hover {
  color: var(--danger);
}

.tab-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.preview-placeholder {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
</style>
