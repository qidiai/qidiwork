<script setup lang="ts">
import { onMounted, ref, computed, defineAsyncComponent } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useAgentState, recoverAgent, startSession } from "../composables/useAgent";
import { fmtTokens, fmtCost } from "../services/usage";
// 设置弹窗按需加载,不进首屏 chunk
const SettingsModal = defineAsyncComponent(() => import("./SettingsModal.vue"));

// 底部状态栏:内核连接状态(由 acp-event 驱动)+ 版本号(IPC 冒烟)。
const { connected, sessionId, sessionUsage } = useAgentState();
const guiVersion = ref("…");
const ipcOk = ref(false);
const recovering = ref(false);
const showSettings = ref(false);

const kernelLabel = computed(() =>
  connected.value ? `已连接 · ${sessionId.value || "会话就绪"}` : "未连接",
);

// 活动会话累计用量:有 token 数据才显示;≈ = 账单可能不完整(内核标记)
const usageLabel = computed(() => {
  const u = sessionUsage.value;
  if (!u || (!u.inputTokens && !u.outputTokens)) return "";
  const cost = u.costSeen ? fmtCost(u.costUsdTicks) : null;
  return `本会话 ↑${fmtTokens(u.inputTokens)} ↓${fmtTokens(u.outputTokens)}${
    cost ? ` · ${u.costPartial ? "≈" : ""}${cost}` : ""
  }`;
});

async function recover() {
  recovering.value = true;
  try {
    await recoverAgent();
  } finally {
    recovering.value = false;
  }
}

async function newSession() {
  await startSession();
}

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
      <span class="dot" :class="connected ? 'on' : 'off'"></span>
      {{ kernelLabel }}
    </span>
    <button v-if="!connected" class="link-btn" :disabled="recovering" @click="recover">
      {{ recovering ? "恢复中…" : "恢复会话" }}
    </button>
    <button v-else class="link-btn" @click="newSession">新会话</button>
    <span
      v-if="usageLabel"
      class="status-item muted"
      title="当前会话累计 token 用量与成本(≈ 为内核标记的不完整账单)"
    >{{ usageLabel }}</span>
    <span class="spacer"></span>
    <span class="status-item muted">
      GUI v{{ guiVersion }}
      <template v-if="ipcOk"> · IPC 正常</template>
    </span>
    <button class="link-btn" @click="showSettings = true">设置</button>
  </footer>
  <SettingsModal v-if="showSettings" @close="showSettings = false" />
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

.dot.on {
  background: var(--success);
}

.link-btn {
  background: none;
  border: none;
  color: var(--accent);
  font-size: var(--font-size-sm);
  padding: 0;
}
</style>
