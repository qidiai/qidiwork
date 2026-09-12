<script setup lang="ts">
import WorkspaceSidebar from "./components/WorkspaceSidebar.vue";
import MainTabs from "./components/MainTabs.vue";
import ArtifactPanel from "./components/ArtifactPanel.vue";
import StatusBar from "./components/StatusBar.vue";
import PermissionDialog from "./components/PermissionDialog.vue";
import { onMounted } from "vue";
import { autoConnect, initAgent } from "./composables/useAgent";

// 启动即自动连接内核(登记簿有会话时):免去每次手动「恢复会话」。
// ChatView 的 onMounted 先于父组件执行,事件监听此时已绑定。
onMounted(() => {
  void (async () => {
    await initAgent();
    await autoConnect();
  })();
});
</script>

<template>
  <div class="app-shell">
    <div class="app-main">
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
  grid-template-columns: 232px minmax(0, 1fr) 320px;
  flex: 1;
  min-height: 0;
}

@media (max-width: 1100px) {
  .app-main {
    grid-template-columns: 200px minmax(0, 1fr) 260px;
  }
}
</style>
