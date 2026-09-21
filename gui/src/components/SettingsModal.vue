<script setup lang="ts">
// 设置弹窗(方案 v2 P2 发布阻塞项的最小实现):
// 默认模型下拉([model.*] 中选择)+ 该模型 API key(可选,留空不改)。
// 保存后内核需重启生效:「保存并重启内核」直接 agent_stop,
// 下一条任务发送时 session_start 自动以新配置重启。
import { computed, onMounted, ref } from "vue";
import {
  readSettings,
  saveSettings,
  createModel,
  stopKernel,
  type ModelOption,
  type SettingsInfo,
} from "../composables/useSettings";
import { pushSystem, useAgentState } from "../composables/useAgent";
import { useAuth } from "../composables/useAuth";

const emit = defineEmits<{ close: [] }>();

const { turnInProgress } = useAgentState();
const {
  status: auth,
  loading: authLoading,
  loginPending,
  loginUri,
  loginCode,
  loginError,
  refresh: refreshAuth,
  startLogin,
  logout,
} = useAuth();
const loading = ref(true);
const loadError = ref("");
const info = ref<SettingsInfo | null>(null);
const selectedId = ref("");
const apiKey = ref("");
const saving = ref(false);
const showKey = ref(false);

// 新建模型(首次运行引导):无任何 [model.*] 时自动展开表单,
// 让用户能在 GUI 里从零配出第一个模型(发布链审计:此前必报错)
const creating = ref(false);
const commonPresets = [
  { key: "", label: "自定义(OpenAI 兼容)", model: "", baseUrl: "" },
  { key: "deepseek", label: "DeepSeek", model: "deepseek-chat", baseUrl: "https://api.deepseek.com/v1" },
  { key: "openai", label: "OpenAI", model: "gpt-4o", baseUrl: "https://api.openai.com/v1" },
  { key: "moonshot", label: "Moonshot Kimi", model: "moonshot-v1-8k", baseUrl: "https://api.moonshot.cn/v1" },
  { key: "bigmodel", label: "智谱 GLM", model: "glm-4-plus", baseUrl: "https://open.bigmodel.cn/api/paas/v4" },
  { key: "siliconflow", label: "硅基流动", model: "deepseek-ai/DeepSeek-V3", baseUrl: "https://api.siliconflow.cn/v1" },
];
const cPreset = ref(commonPresets[0]!);
const cId = ref("");
const cModel = ref("");
const cBaseUrl = ref("");
const cName = ref("");
const cApiKey = ref("");
const cError = ref("");

function pickPreset(key: string): void {
  cPreset.value = commonPresets.find((p) => p.key === key) ?? commonPresets[0]!;
  if (cPreset.value.model) cModel.value = cPreset.value.model;
  if (cPreset.value.baseUrl) cBaseUrl.value = cPreset.value.baseUrl;
}

async function submitCreate(): Promise<void> {
  cError.value = "";
  if (!/^[A-Za-z0-9_-]+$/.test(cId.value.trim())) {
    cError.value = "id 只允许字母、数字、- 和 _(如 deepseek)";
    return;
  }
  if (!cModel.value.trim() || !cBaseUrl.value.trim()) {
    cError.value = "模型名与 base_url 不能为空";
    return;
  }
  saving.value = true;
  try {
    await createModel({
      id: cId.value.trim(),
      model: cModel.value.trim(),
      baseUrl: cBaseUrl.value.trim(),
      name: cName.value.trim() || null,
      apiKey: cApiKey.value.trim() || null,
    });
    cApiKey.value = "";
    creating.value = false;
    await load();
    selectedId.value = cId.value.trim();
    pushSystem(`模型 ${cId.value.trim()} 已创建;重启内核后生效,可点「保存并重启内核」。`);
  } catch (e) {
    cError.value = String(e);
  } finally {
    saving.value = false;
  }
}

const selected = computed<ModelOption | null>(
  () => info.value?.models.find((m) => m.id === selectedId.value) ?? null,
);

function fmt(n: number | null | undefined): string {
  return n == null ? "—" : n.toLocaleString("zh-CN");
}

async function openLoginUri(): Promise<void> {
  if (!loginUri.value) return;
  try {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(loginUri.value);
  } catch {
    /* opener 不可用时用户可手动复制 URL */
  }
}

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
    // 无任何模型定义:自动展开新建表单(首次运行引导)
    if (!info.value.models.length && !info.value.default_model) {
      creating.value = true;
    }
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

onMounted(() => {
  void load();
  void refreshAuth();
});
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
        <!-- 账号(方案 P1-3):登录态 / 设备码登录 / 套餐与今日配额 -->
        <div class="account-box">
          <div class="account-head">
            <span class="create-title">账号</span>
            <button class="link-btn" :disabled="authLoading" @click="refreshAuth">
              {{ authLoading ? "检测中…" : "刷新" }}
            </button>
          </div>

          <template v-if="auth?.logged_in">
            <div class="account-row">
              <span class="account-email">{{ auth.email ?? auth.user_id ?? "已登录" }}</span>
              <span class="plan-badge">{{ auth.plan ?? "—" }}</span>
            </div>
            <div v-if="auth.usage_today && auth.quota" class="quota-grid">
              <div class="quota-cell">
                <span class="quota-num">
                  {{ fmt(auth.usage_today.requests) }} /
                  {{ auth.quota.requests_limit > 0 ? fmt(auth.quota.requests_limit) : "不限" }}
                </span>
                <span class="quota-label">今日请求</span>
              </div>
              <div class="quota-cell">
                <span class="quota-num">
                  {{ fmt(auth.usage_today.tokens) }} /
                  {{ auth.quota.tokens_limit > 0 ? fmt(auth.quota.tokens_limit) : "不限" }}
                </span>
                <span class="quota-label">今日 tokens</span>
              </div>
            </div>
            <p v-if="auth.detail" class="field-hint">{{ auth.detail }}</p>
            <div class="create-actions">
              <button class="btn ghost" :disabled="loginPending" @click="logout">退出登录</button>
            </div>
          </template>

          <template v-else>
            <p class="field-hint">登录后可用自动模型路由与云端推理配额。</p>
            <div v-if="loginPending" class="login-pending">
              <p v-if="loginCode">
                请在浏览器打开下方地址并输入验证码
                <code class="code-chip">{{ loginCode }}</code>
              </p>
              <p v-else-if="!loginError">正在等待内核返回登录地址…</p>
              <p v-if="loginUri">
                <button class="link-btn" @click.prevent="openLoginUri">{{ loginUri }}</button>
              </p>
              <p v-if="loginError" class="err">{{ loginError }}</p>
              <p class="field-hint">完成浏览器授权后此处会自动更新。</p>
            </div>
            <div v-else class="create-actions">
              <button class="btn primary" @click="startLogin">登录 QIDI 账号</button>
            </div>
          </template>
        </div>

        <!-- 新建模型表单(首次运行引导) -->
        <template v-if="creating">
          <div class="create-box">
            <div class="create-title">新建模型</div>
            <label class="field">
              <span class="field-label">服务商模板</span>
              <select class="input" :value="cPreset.key" @change="pickPreset(($event.target as HTMLSelectElement).value)">
                <option v-for="p in commonPresets" :key="p.key" :value="p.key">{{ p.label }}</option>
              </select>
            </label>
            <label class="field">
              <span class="field-label">id(配置内唯一,如 deepseek)</span>
              <input v-model="cId" class="input" placeholder="deepseek" />
            </label>
            <label class="field">
              <span class="field-label">模型名(上游 model)</span>
              <input v-model="cModel" class="input" placeholder="deepseek-chat" />
            </label>
            <label class="field">
              <span class="field-label">base_url(OpenAI 兼容端点)</span>
              <input v-model="cBaseUrl" class="input" placeholder="https://api.deepseek.com/v1" />
            </label>
            <label class="field">
              <span class="field-label">显示名(可选)</span>
              <input v-model="cName" class="input" placeholder="留空回落 id" />
            </label>
            <label class="field">
              <span class="field-label">API Key</span>
              <input v-model="cApiKey" class="input" type="password" autocomplete="off" placeholder="sk-…" />
            </label>
            <p v-if="cError" class="err">{{ cError }}</p>
            <div class="create-actions">
              <button class="btn ghost" @click="creating = false">收起</button>
              <button class="btn primary" :disabled="saving" @click="submitCreate">
                {{ saving ? "创建中…" : "创建模型" }}
              </button>
            </div>
          </div>
        </template>

        <template v-else>
          <label class="field">
            <span class="field-label">默认模型</span>
            <select v-model="selectedId" class="input">
              <option v-for="m in options" :key="m.id" :value="m.id">
                {{ m.id === info?.default_model ? "● " : "" }}{{ m.name }}
              </option>
            </select>
            <span v-if="selected?.base_url" class="field-hint">{{ selected.base_url }}</span>
            <button class="link-btn self-start" @click.prevent="creating = true">+ 新建模型</button>
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
        </template>

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

/* 新建模型表单(首次运行引导) */
.create-box {
  display: flex;
  flex-direction: column;
  gap: 12px;
  border: 1px solid var(--accent-soft, var(--border));
  border-radius: var(--radius);
  padding: 12px 14px;
}

.create-title {
  font-weight: 600;
  color: var(--text-primary);
}

.create-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.self-start {
  align-self: flex-start;
}

/* 账号区块(方案 P1-3) */
.account-box {
  display: flex;
  flex-direction: column;
  gap: 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 12px 14px;
}

.account-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.account-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.account-email {
  font-weight: 600;
  color: var(--text-primary);
  word-break: break-all;
}

.plan-badge {
  font-size: var(--font-size-sm);
  padding: 1px 8px;
  border-radius: 999px;
  background: var(--accent-soft, var(--bg-panel));
  color: var(--accent);
  border: 1px solid var(--border);
  text-transform: uppercase;
}

.quota-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
}

.quota-cell {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-panel);
}

.quota-num {
  font-variant-numeric: tabular-nums;
  color: var(--text-primary);
}

.quota-label {
  font-size: var(--font-size-sm);
  color: var(--text-disabled);
}

.login-pending {
  display: flex;
  flex-direction: column;
  gap: 6px;
  color: var(--text-primary);
}

.code-chip {
  display: inline-block;
  padding: 1px 8px;
  border-radius: var(--radius);
  background: var(--bg-panel);
  border: 1px solid var(--border);
  font-family: monospace;
  font-weight: 700;
  letter-spacing: 1px;
}
</style>
