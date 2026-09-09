<script setup lang="ts">
import WorkspaceSidebar from "./components/WorkspaceSidebar.vue";
import MainTabs from "./components/MainTabs.vue";
import ArtifactPanel from "./components/ArtifactPanel.vue";
import StatusBar from "./components/StatusBar.vue";
import PermissionDialog from "./components/PermissionDialog.vue";
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
