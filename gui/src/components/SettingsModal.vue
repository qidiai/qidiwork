<script setup lang="ts">
// 设置弹窗(方案 v2 P2 发布阻塞项的最小实现):
// 默认模型下拉([model.*] 中选择)+ 该模型 API key(可选,留空不改)。
// 保存后内核需重启生效:「保存并重启内核」直接 agent_stop,
// 下一条任务发送时 session_start 自动以新配置重启。
import { computed, onMounted, ref } from "vue";
import {
  readSettings,
  saveSettings,
  stopKernel,
  type ModelOption,
  type SettingsInfo,
} from "../composables/useSettings";
import { pushSystem, useAgentState } from "../composables/useAgent";

const emit = defineEmits<{ close: [] }>();

const { turnInProgress } = useAgentState();
const loading = ref(true);
const loadError = ref("");
const info = ref<SettingsInfo | null>(null);
const selectedId = ref("");
const apiKey = ref("");
const saving = ref(false);
const showKey = ref(false);

const selected = computed<ModelOption | null>(
  () => info.value?.models.find((m) => m.id === selectedId.value) ?? null,
);

// 下拉框选项 = config.toml 里定义的模型;当前默认若是内置模型(不在
// 本文件定义),也作为一项列出供选回。
const options = computed(() => {
  const list = info.value?.models ?? [];
  const cur = info.value?.default_model ?? null;
  if (cur && !list.some((m) => m.id === cur)) {
    return [{ id: cur, name: `${cur}(内置默认)`, base_url: null, has_api_key: false }, ...list];
  }
  return list;
});

async function load(): Promise<void> {
  loading.value = true;
  loadError.value = "";
  try {
    info.value = await readSettings();
    selectedId.value = info.value.default_model ?? info.value.models[0]?.id ?? "";
  } catch (e) {
    loadError.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function save(restart: boolean): Promise<void> {
  if (!selectedId.value) return;
  saving.value = true;
  try {
    const key = apiKey.value.trim();
    await saveSettings(selectedId.value, key.length > 0 ? key : null);
    apiKey.value = ""; // 保存后不残留明文(k3 审计 N1)
    if (restart) {
      await stopKernel();
      pushSystem(
        `默认模型已设为 ${selectedId.value},内核已停止;发送下一条任务时自动以新配置重启。`,
      );
    } else {
      pushSystem(`默认模型已设为 ${selectedId.value},重启内核后生效。`);
    }
    emit("close");
  } catch (e) {
    loadError.value = String(e);
  } finally {
    saving.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="modal-mask" @click.self="emit('close')">
    <div class="modal">
      <div class="modal-head">
        <span class="modal-title">设置</span>
        <button class="modal-close" @click="emit('close')">✕</button>
      </div>

      <div v-if="loading" class="modal-body">正在读取设置…</div>
      <div v-else-if="loadError" class="modal-body">
        <p class="err">{{ loadError }}</p>
        <button class="btn" @click="load">重试</button>
      </div>

      <div v-else class="modal-body">
        <label class="field">
          <span class="field-label">默认模型</span>
          <select v-model="selectedId" class="input">
            <option v-for="m in options" :key="m.id" :value="m.id">
              {{ m.id === info?.default_model ? "● " : "" }}{{ m.name }}
            </option>
          </select>
          <span v-if="selected?.base_url" class="field-hint">{{ selected.base_url }}</span>
        </label>

        <label class="field">
          <span class="field-label">API Key({{ selected?.id ?? "—" }})</span>
          <input
            v-model="apiKey"
            class="input"
            :type="showKey ? 'text' : 'password'"
            autocomplete="off"
            :placeholder="
              selected?.has_api_key ? '已设置,留空保持不变' : '未设置,留空跳过'
            "
          />
          <span class="field-hint">
            留空不修改。保存后明文写入 config.toml(与 TUI 同一配置)。
            <button class="link-btn" @click.prevent="showKey = !showKey">
              {{ showKey ? "隐藏" : "显示" }}
            </button>
          </span>
        </label>

        <p class="field-hint">
          内核在启动时读取配置:「保存并重启内核」立即生效;仅保存则下一条任务发送时自动重启生效。
        </p>
      </div>

      <div class="modal-foot">
        <button class="btn ghost" @click="emit('close')">取消</button>
        <button
          class="btn"
          :disabled="loading || !!loadError || !selectedId || saving"
          @click="save(false)"
        >
          {{ saving ? "保存中…" : "保存" }}
        </button>
        <button
          class="btn primary"
          :disabled="loading || !!loadError || !selectedId || saving || turnInProgress"
          :title="
            turnInProgress ? '内核正在执行任务,禁止重启;等任务完成后再试' : ''
          "
          @click="save(true)"
        >
          保存并重启内核
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.modal-mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.modal {
  width: 480px;
  max-width: calc(100vw - 48px);
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: 12px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.35);
  display: flex;
  flex-direction: column;
}

.modal-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 18px;
  border-bottom: 1px solid var(--border);
}

.modal-title {
  font-weight: 600;
  color: var(--text-primary);
}

.modal-close {
  background: none;
  border: none;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 14px;
}

.modal-body {
  padding: 16px 18px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  color: var(--text-primary);
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field-label {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  font-weight: 600;
}

.input {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 8px 10px;
  background: var(--bg-panel);
  color: var(--text-primary);
  font-size: var(--font-size-sm);
}

.field-hint {
  font-size: var(--font-size-sm);
  color: var(--text-disabled);
}

.err {
  color: var(--danger, #c0392b);
  font-size: var(--font-size-sm);
  margin: 0;
  word-break: break-all;
}

.ok {
  color: var(--success);
  font-size: var(--font-size-sm);
  margin: 0;
}

.modal-foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 18px;
  border-top: 1px solid var(--border);
}

.btn {
  border: 1px solid var(--border);
  background: var(--bg-panel);
  color: var(--text-primary);
  border-radius: var(--radius);
  padding: 7px 16px;
  font-size: var(--font-size-sm);
  cursor: pointer;
}

.btn:hover:not(:disabled) {
  background: var(--bg-hover);
}

.btn:disabled {
  cursor: not-allowed;
  opacity: 0.6;
}

.btn.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}

.btn.primary:hover:not(:disabled) {
  filter: brightness(1.1);
}

.btn.ghost {
  background: transparent;
}
</style>
