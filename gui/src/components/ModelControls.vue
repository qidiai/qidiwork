<script setup lang="ts">
import { computed, defineAsyncComponent, onMounted, ref, watch } from "vue";
import { setModel, setEffort, useAgentState } from "../composables/useAgent";
import { readSettings } from "../composables/useSettings";

const SettingsModal = defineAsyncComponent(() => import("./SettingsModal.vue"));
const props = defineProps<{ disabled: boolean }>();
const emit = defineEmits<{ busy: [value: boolean] }>();
const { connected, sessionId, modelState } = useAgentState();
const pending = ref(false);
const error = ref("");
const showSettings = ref(false);
const defaultName = ref("");
watch(sessionId, () => { error.value = ""; });
const currentModel = computed(() => modelState.value?.availableModels.find((m) => m.modelId === modelState.value?.currentModelId));
const canSelect = computed(() => connected.value && !!sessionId.value && !!modelState.value?.availableModels.length);
const effortOptions = computed(() => currentModel.value?.supportsReasoningEffort ? currentModel.value.reasoningEfforts : []);
const currentEffort = computed(() => currentModel.value?.reasoningEffort ?? "");
const modelPlaceholder = computed(() => !connected.value ? (defaultName.value ? `${defaultName.value}（默认）` : "未连接 · 可先设置默认模型") : "尚未收到模型列表");
const effortPlaceholder = computed(() => !connected.value || !currentModel.value ? "连接模型后确认支持情况" : !effortOptions.value.length ? "当前模型未提供强度设置" : "使用模型默认强度");

async function refreshDefault(): Promise<void> {
  try {
    const settings = await readSettings();
    defaultName.value = settings.models.find((m) => m.id === settings.default_model)?.name ?? settings.default_model ?? "";
  } catch {
    defaultName.value = "";
  }
}
onMounted(refreshDefault);

async function pick(event: Event, kind: "model" | "effort"): Promise<void> {
  const select = event.target as HTMLSelectElement;
  const value = select.value;
  if (pending.value || props.disabled || !connected.value || !value) return;
  const id = sessionId.value;
  pending.value = true;
  emit("busy", true);
  error.value = "";
  try {
    const ok = kind === "model" ? await setModel(value) : await setEffort(value);
    if (!ok && sessionId.value === id) error.value = "设置未成功，已恢复原选项，请重试。";
  } finally {
    select.value = kind === "model" ? modelState.value?.currentModelId ?? "" : currentEffort.value;
    pending.value = false;
    emit("busy", false);
  }
}
</script>

<template>
  <div class="model-controls">
    <div class="model-options">
      <label>模型
        <select aria-label="当前模型" :value="modelState?.currentModelId ?? ''" :disabled="disabled || pending || !canSelect" @change="pick($event, 'model')">
          <option v-if="!canSelect" :value="modelState?.currentModelId ?? ''">{{ modelPlaceholder }}</option>
          <option v-for="model in modelState?.availableModels ?? []" :key="model.modelId" :value="model.modelId">{{ model.name }}</option>
        </select>
      </label>
      <label>思考强度
        <select aria-label="思考强度" :value="currentEffort" :disabled="disabled || pending || !canSelect || !effortOptions.length" @change="pick($event, 'effort')">
          <option v-if="!currentEffort || !effortOptions.length" :value="currentEffort" disabled>{{ effortPlaceholder }}</option>
          <option v-for="option in effortOptions" :key="option.id" :value="option.value">{{ option.label }}</option>
        </select>
      </label>
      <button type="button" class="model-settings" :disabled="disabled || pending" @click="showSettings = true">模型设置</button>
    </div>
    <p v-if="pending" class="control-hint" role="status">正在更新设置，请稍候…</p>
    <p v-else-if="disabled" class="control-hint">当前处理结束后可以更换模型和思考强度。</p>
    <p v-if="error" class="control-error" role="alert">{{ error }}</p>
  </div>
  <SettingsModal v-if="showSettings" @close="showSettings = false; refreshDefault()" />
</template>

<style scoped>
.model-controls { margin-top: 10px; }
.model-options { display: flex; align-items: center; flex-wrap: wrap; gap: 8px 14px; }
.model-options label { display: flex; align-items: center; gap: 6px; color: var(--text-secondary); font-size: 12px; min-width: 0; }
.model-options select { min-width: 85px; max-width: 185px; min-height: 30px; border: 1px solid var(--border); border-radius: 6px; color: var(--text-primary); background: var(--bg-panel); font: inherit; padding: 3px 5px; }
.model-options select:disabled { color: var(--text-secondary); }
.model-settings { border: 0; color: var(--accent); background: transparent; padding: 5px 0; font-size: 12px; }
.control-hint, .control-error { font-size: 12px; margin: 6px 0 0; }
.control-hint { color: var(--text-secondary); }
.control-error { color: var(--danger); }
</style>
