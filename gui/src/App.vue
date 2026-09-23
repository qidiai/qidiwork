<script setup lang="ts">
import WorkspaceSidebar from "./components/WorkspaceSidebar.vue";
import MainTabs from "./components/MainTabs.vue";
import ArtifactPanel from "./components/ArtifactPanel.vue";
import StatusBar from "./components/StatusBar.vue";
import PermissionDialog from "./components/PermissionDialog.vue";
import OnboardingView from "./components/OnboardingView.vue";
import { computed, onMounted, ref } from "vue";
import { autoConnect, initAgent } from "./composables/useAgent";
import { shouldShowOnboarding } from "./composables/useOnboarding";
import {
  PANEL_STRIP,
  panelCollapsed,
  panelWidth,
  SIDEBAR_STRIP,
  sidebarCollapsed,
} from "./composables/useUiLayout";

// 首次启动引导门控(K3 修正 1):onboarding 未完成前不 initAgent/autoConnect,
// 避免未登录态拉起无效内核进程。
const showOnboarding = ref(false);

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
  const right = panelCollapsed.value ? `${PANEL_STRIP}px` : `${panelWidth.value}px`;
  return `${left} minmax(0, 1fr) ${right}`;
});
</script>

<template>
  <div class="app-shell">
    <div class="app-main" :style="{ gridTemplateColumns: gridColumns }">
      <WorkspaceSidebar />
      <MainTabs />
      <ArtifactPanel />
    </div>
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
</style>
