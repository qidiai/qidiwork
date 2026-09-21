<script setup lang="ts">
import WorkspaceSidebar from "./components/WorkspaceSidebar.vue";
import MainTabs from "./components/MainTabs.vue";
import ArtifactPanel from "./components/ArtifactPanel.vue";
import StatusBar from "./components/StatusBar.vue";
import PermissionDialog from "./components/PermissionDialog.vue";
import { computed, onMounted } from "vue";
import { autoConnect, initAgent } from "./composables/useAgent";
import {
  PANEL_STRIP,
  panelCollapsed,
  panelWidth,
  SIDEBAR_STRIP,
  sidebarCollapsed,
} from "./composables/useUiLayout";

// 启动即自动连接内核(登记簿有会话时):免去每次手动「恢复会话」。
// ChatView 的 onMounted 先于父组件执行,事件监听此时已绑定。
// 自动连接延后到首帧渲染之后:内核冷启动 + 逐会话串行恢复不再与首屏抢 CPU。
onMounted(() => {
  void (async () => {
    await initAgent();
    setTimeout(() => void autoConnect(), 500);
  })();
});

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
