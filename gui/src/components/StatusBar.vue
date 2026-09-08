<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

// 底部状态栏。左侧内核状态由 M2(ACP 桥)驱动;右侧版本号通过
// IPC 命令 app_version 取回,同时充当 M1 的 IPC 通路冒烟验证。
const guiVersion = ref("…");
const ipcOk = ref(false);

onMounted(async () => {
  try {
    guiVersion.value = (await invoke<string>("app_version")) ?? "";
    ipcOk.value = guiVersion.value.length > 0;
  } catch {
    guiVersion.value = "IPC 不可用";
  }
});
</script>

<template>
  <footer class="status-bar">
    <span class="status-item">
      <span class="dot off"></span>
      内核未连接
    </span>
    <span class="spacer"></span>
    <span class="status-item muted">
      GUI v{{ guiVersion }}
      <template v-if="ipcOk"> · IPC 正常</template>
    </span>
  </footer>
</template>

<style scoped>
.status-bar {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 5px 14px;
  background: var(--bg-panel);
  border-top: 1px solid var(--border);
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  user-select: none;
}

.spacer {
  flex: 1;
}

.status-item {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.muted {
  color: var(--text-disabled);
}

.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
}

.dot.off {
  background: var(--text-disabled);
}
</style>
