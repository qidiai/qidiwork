<script setup lang="ts">
import { onMounted, ref, computed, defineAsyncComponent } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { setModel, setEffort, useAgentState, recoverAgent, startSession, type ModelInfo } from "../composables/useAgent";
import { useAuth } from "../composables/useAuth";
import { fmtTokens, fmtCost } from "../services/usage";
// 设置弹窗按需加载,不进首屏 chunk
const SettingsModal = defineAsyncComponent(() => import("./SettingsModal.vue"));

// 底部状态栏:内核连接状态(由 acp-event 驱动)+ 版本号(IPC 冒烟)。
const { connected, sessionId, sessionUsage, modelState } = useAgentState();
const switchingModel = ref(false);
const switchingEffort = ref(false);
const { status: auth, refresh: refreshAuth } = useAuth();
const guiVersion = ref("…");
const ipcOk = ref(false);
const recovering = ref(false);
const showSettings = ref(false);

const accountLabel = computed(() =>
  auth.value?.logged_in
    ? `${auth.value.email ?? auth.value.user_id ?? "已登录"}${auth.value.plan ? ` · ${auth.value.plan}` : ""}`
    : "未登录",
);

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

// 模型指示降级链:内核模型状态 → 最近回合实际上报的模型 → 不显示。
const modelLabel = computed(() => {
  const st = modelState.value;
  if (st?.currentModelId) {
    return st.availableModels.find((m) => m.modelId === st.currentModelId)?.name ?? st.currentModelId;
  }
  const used = sessionUsage.value.models;
  return used.length ? used[used.length - 1] : "";
});

// 可切换 = 内核给了候选清单且已连接;旧内核/无清单时退化为纯展示。
const canSwitchModel = computed(
  () => connected.value && (modelState.value?.availableModels.length ?? 0) > 0,
);

// 当前模型(带内核扩展 meta:思考强度支持标记 + 档位清单/当前值)。
const currentModel = computed<ModelInfo | null>(() => {
  const st = modelState.value;
  if (!st) return null;
  return st.availableModels.find((m) => m.modelId === st.currentModelId) ?? null;
});

// 思考强度下拉:仅当当前模型声明 supportsReasoningEffort 且有档位清单时显示;
// 旧内核/不支持时隐藏(与模型下拉同一降级逻辑)。
const canSwitchEffort = computed(
  () =>
    connected.value &&
    !!currentModel.value?.supportsReasoningEffort &&
    (currentModel.value?.reasoningEfforts.length ?? 0) > 0,
);

// 当前档位(内核 meta.reasoningEffort;与档位清单的 value 同为规范值,可精确匹配)。
const currentEffort = computed(() => currentModel.value?.reasoningEffort ?? "");

async function onEffortPick(e: Event): Promise<void> {
  const sel = e.target as HTMLSelectElement;
  const effort = sel.value;
  if (!effort || effort === currentEffort.value) return;
  switchingEffort.value = true;
  try {
    await setEffort(effort);
  } finally {
    switchingEffort.value = false;
  }
}

async function onModelPick(e: Event): Promise<void> {
  const sel = e.target as HTMLSelectElement;
  const id = sel.value;
  if (!id || id === modelState.value?.currentModelId) return;
  switchingModel.value = true;
  try {
    await setModel(id);
  } finally {
    switchingModel.value = false;
  }
}

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
  void refreshAuth();
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
    <select
      v-if="canSwitchModel && modelState"
      class="model-select"
      :value="modelState.currentModelId"
      :disabled="switchingModel"
      title="当前模型(点击切换,后续回合生效)"
      @change="onModelPick"
    >
      <option v-for="m in modelState.availableModels" :key="m.modelId" :value="m.modelId">
        {{ m.name }}
      </option>
    </select>
    <span v-else-if="modelLabel" class="status-item muted" title="当前模型(内核上报)">{{ modelLabel }}</span>
    <select
      v-if="canSwitchEffort && currentModel"
      class="model-select"
      :value="currentEffort"
      :disabled="switchingEffort"
      title="思考强度(点击切换,后续回合生效)"
      @change="onEffortPick"
    >
      <option v-if="!currentEffort" value="" disabled>强度</option>
      <option v-for="opt in currentModel.reasoningEfforts" :key="opt.id" :value="opt.value">
        {{ opt.label }}
      </option>
    </select>
    <span class="spacer"></span>
    <button
      class="link-btn"
      :class="{ muted: !auth?.logged_in }"
      :title="auth?.logged_in ? '账号与配额' : '点击登录 QIDI 账号'"
      @click="showSettings = true"
    >{{ accountLabel }}</button>
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

.model-select {
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  max-width: 160px;
  padding: 2px 4px;
}

.model-select:disabled {
  opacity: 0.6;
}
</style>
