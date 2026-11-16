<script setup lang="ts">
import WorkspaceSidebar from "./components/WorkspaceSidebar.vue";
import MainTabs from "./components/MainTabs.vue";
import ArtifactPanel from "./components/ArtifactPanel.vue";
import StatusBar from "./components/StatusBar.vue";
import BidExportModal from "./components/BidExportModal.vue";
import PermissionDialog from "./components/PermissionDialog.vue";
import OnboardingView from "./components/OnboardingView.vue";
import { computed, onMounted, ref } from "vue";
import { autoConnect, initAgent } from "./composables/useAgent";
import { shouldShowOnboarding } from "./composables/useOnboarding";
import { usePreviewTabs } from "./composables/usePreview";
import {
  PANEL_STRIP,
  panelCollapsed,
  panelWidth,
  SIDEBAR_STRIP,
  sidebarCollapsed,
} from "./composables/useUiLayout";

const { activePreview } = usePreviewTabs();
function preventFileNavigation(event: DragEvent): void {
  if (event.dataTransfer?.types.includes("Files")) event.preventDefault();
}

// 首次启动引导门控(K3 修正 1):onboarding 未完成前不 initAgent/autoConnect,
// 避免未登录态拉起无效内核进程。
const showOnboarding = ref(false);

// 导出标书弹窗(P1-3):后端入口是 bid_export_md 命令(Markdown → docx)。
const showBidExport = ref(false);

// 连接内核(登记簿有会话时自动续接):免去每次手动「恢复会话」。
// ChatView 的 onMounted 先于父组件执行,事件监听此时已绑定。
// 自动连接延后到首帧渲染之后:内核冷启动 + 逐会话串行恢复不再与首屏抢 CPU。
function connectAgent(): void {
  void (async () => {
    await initAgent();
    setTimeout(() => void autoConnect(), 500);
  })();
}

onMounted(() => {
  void (async () => {
    if (await shouldShowOnboarding()) {
      // 引导中:只展示 OnboardingView,不 initAgent / autoConnect。
      showOnboarding.value = true;
      return;
    }
    connectAgent();
  })();
});

// 引导完成/跳过:关闭覆盖层,此时才打开内核门控。
function onOnboardingDone(): void {
  showOnboarding.value = false;
  connectAgent();
}

// 列宽由 useUiLayout 的折叠/拖拽状态驱动;折叠时收为窄竖条(组件自渲染把手)。
const gridColumns = computed(() => {
  const left = sidebarCollapsed.value ? `${SIDEBAR_STRIP}px` : "232px";
  const right = activePreview.value ? "0px" : panelCollapsed.value ? `${PANEL_STRIP}px` : `${panelWidth.value}px`;
  return `${left} minmax(0, 1fr) ${right}`;
});
</script>

<template>
  <div class="app-shell" @dragover="preventFileNavigation" @drop="preventFileNavigation">
    <div class="app-main" :style="{ gridTemplateColumns: gridColumns }">
      <WorkspaceSidebar />
      <MainTabs />
      <ArtifactPanel v-show="!activePreview" />
    </div>
    <button class="bid-export-fab" title="把 Markdown 标书导出为 docx" @click="showBidExport = true">
      📄 导出标书
    </button>
    <BidExportModal v-if="showBidExport" @close="showBidExport = false" />
    <StatusBar />
    <PermissionDialog />
    <OnboardingView v-if="showOnboarding" @done="onOnboardingDone" />
  </div>
</template>

<style scoped>
.app-shell {
  display: flex;
  flex-direction: column;
  height: 100vh;
  overflow: hidden;
}

.app-main {
  display: grid;
  flex: 1;
  min-height: 0;
}

/* 导出标书入口(P1-3):悬浮在状态栏上方,不参与 app-main 的网格布局 */
.bid-export-fab {
  position: fixed;
  right: 18px;
  bottom: 46px;
  z-index: 60;
  padding: 7px 14px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: var(--bg-panel);
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  box-shadow: 0 4px 14px rgba(0, 0, 0, 0.25);
  cursor: pointer;
}

.bid-export-fab:hover {
  border-color: var(--accent);
  color: var(--accent);
}
</style>
