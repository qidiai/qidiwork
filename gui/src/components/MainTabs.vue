<script setup lang="ts">
import { ref, watch } from "vue";
import ChatView from "./ChatView.vue";
import PreviewPane from "./PreviewPane.vue";
import { usePreviewTabs, closePreview } from "../composables/usePreview";

// 中区多 Tab:对话为常驻 Tab;产物预览 Tab 由点击产物卡片时打开。
type TabKey = string;
const tabs = ref<{ key: TabKey; label: string; closable: boolean }[]>([
  { key: "chat", label: "对话", closable: false },
]);
const activeTab = ref<TabKey>("chat");

const { openPreviews, activePreview, activationSeq } = usePreviewTabs();

// 预览注册表 → Tab 列表(注册表 key 即 Tab key,由 previewKey 统一派生)
watch(
  openPreviews,
  (list) => {
    const keys = new Set(list.map((t) => t.key));
    // 移除已关闭的预览 Tab
    tabs.value = tabs.value.filter((t) => t.closable === false || keys.has(t.key));
    // 追加新预览 Tab
    for (const p of list) {
      if (!tabs.value.some((t) => t.key === p.key)) {
        tabs.value.push({ key: p.key, label: p.name, closable: true });
      }
    }
    // 批量清理(任务切换 closeAllPreviews)可能移除正激活的 Tab,
    // activeTab 残留会指向不存在的 key,回落到对话
    if (activeTab.value.startsWith("preview:") && !keys.has(activeTab.value)) {
      activeTab.value = "chat";
      activePreview.value = "";
    }
  },
  { deep: true },
);

// 重复打开同一产物 → 激活已有 Tab。watch 序号而非 key:同 key 重开时
// activePreview 同值赋值不触发 Vue watch,激活会脱节(step 审计 W2)
watch(activationSeq, () => {
  if (activePreview.value) activeTab.value = activePreview.value;
});

// 手动点 Tab 时同步 activePreview:否则注册表激活状态与实际激活页
// 脱节,之后重开同一产物会因 ref 同值不触发激活 watch(deepseek 审计 B1/C3)。
function selectTab(key: TabKey) {
  activeTab.value = key;
  activePreview.value = key.startsWith("preview:") ? key : "";
}

function closeTab(key: TabKey) {
  const idx = tabs.value.findIndex((t) => t.key === key);
  if (idx < 0 || !tabs.value[idx].closable) return;
  tabs.value.splice(idx, 1);
  // Tab key 即注册表 key,直接同步清理,否则下次打开同一产物被去重跳过
  if (key.startsWith("preview:")) closePreview(key);
  if (activeTab.value === key) {
    activeTab.value = tabs.value[Math.max(0, idx - 1)]!.key;
  }
}

function previewOf(key: TabKey) {
  const p = openPreviews.value.find((t) => t.key === key);
  return { task: p?.task ?? "", name: p?.name ?? "" };
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
        :title="tab.label"
        @click="selectTab(tab.key)"
      >
        <span class="tab-label">{{ tab.label }}</span>
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
      <PreviewPane
        v-else-if="activeTab.startsWith('preview:')"
        :key="activeTab"
        v-bind="previewOf(activeTab)"
      />
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
