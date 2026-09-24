<script setup lang="ts">
// 首次启动引导(R10 / K3 审计修正版):全屏覆盖层,三步走。
//   Step 1 欢迎 → Step 2 登录(设备码)→ Step 3 选择模型 → 完成/跳过。
// 暗色主题:在 .onboarding 作用域内覆盖全局设计令牌(变量名复用 global.css),
// 不改全局 :root。
//
// 完成后动作(K3 修正 3):Step 3 选了非当前默认的模型 → settings_save +
// agent_stop(下次启动以新配置重启内核);随后 completeOnboarding 双写标记,
// 再 emit("done") 由 App.vue 打开内核门控(initAgent + autoConnect)。
import { onBeforeUnmount, onMounted, ref } from "vue";
import { useAuth } from "../composables/useAuth";
import { readSettings, setDefaultModel, stopKernel } from "../composables/useSettings";
import {
  checkOnboardingAuth,
  completeOnboarding,
  currentStep,
  fetchServerModels,
  TOTAL_STEPS,
  type ServerModel,
} from "../composables/useOnboarding";

const emit = defineEmits<{ done: [] }>();

// 登录复用 SettingsModal 同一数据源(设备码经 auth-event 回流)。
const { loginUri, loginCode, loginError, startLogin } = useAuth();

// ---------------------------------------------------- Step 2:登录轮询 --

let pollTimer: number | undefined;
let pollBusy = false;

function stopPolling(): void {
  if (pollTimer !== undefined) {
    window.clearInterval(pollTimer);
    pollTimer = undefined;
  }
}

// 轮询本地登录态(auth.json):浏览器完成授权后自动进入 Step 3。
function startPolling(): void {
  stopPolling();
  pollTimer = window.setInterval(() => {
    void (async () => {
      if (pollBusy) return;
      pollBusy = true;
      let ok = false;
      try {
        ok = await checkOnboardingAuth();
      } catch {
        ok = false;
      } finally {
        pollBusy = false;
      }
      if (ok) {
        stopPolling();
        goModels();
      }
    })();
  }, 1500);
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

// ---------------------------------------------------------- 步骤切换 --

function goWelcome(): void {
  stopPolling();
  currentStep.value = 1;
}

function goLogin(): void {
  currentStep.value = 2;
  // 无条件触发:auth.json 里有 token ≠ token 还有效(内核会自行刷新,引导命令不会)。
  // 已失效时若跳过登录,Step 3 只会拿到 401。重复点击由 useAuth 单飞守卫拦住。
  void startLogin();
  startPolling();
}

function goModels(): void {
  stopPolling();
  currentStep.value = 3;
  void loadModels();
}

async function beginSetup(): Promise<void> {
  // 已是登录态(如经 TUI 登录过)直接进入模型选择,省去重复授权。
  try {
    if (await checkOnboardingAuth()) {
      goModels();
      return;
    }
  } catch {
    /* 忽略:继续走登录 */
  }
  goLogin();
}

// -------------------------------------------------------- Step 3:模型 --

const models = ref<ServerModel[]>([]);
const modelsLoading = ref(false);
const modelsError = ref("");
const selectedId = ref("");
const currentDefault = ref("");

async function loadModels(): Promise<void> {
  modelsLoading.value = true;
  modelsError.value = "";
  try {
    models.value = await fetchServerModels();
    try {
      const info = await readSettings();
      currentDefault.value = info.default_model ?? "";
    } catch {
      currentDefault.value = "";
    }
    // 预选当前默认:未改动即不写配置,「完成」语义 = 保持现状。
    if (models.value.some((m) => m.id === currentDefault.value)) {
      selectedId.value = currentDefault.value;
    }
  } catch (e) {
    modelsError.value = String(e);
  } finally {
    modelsLoading.value = false;
  }
}

// ------------------------------------------------------- 完成 / 跳过 --

const finishing = ref(false);
const finishHint = ref("");

async function finish(): Promise<void> {
  if (finishing.value) return;
  finishing.value = true;
  finishHint.value = "";
  try {
    // 选了非当前默认的模型 → 写默认并停内核(下次启动以新配置重启)。
    // 用 setDefaultModel 而非 saveSettings:Step 3 列的是服务端目录模型,
    // 大多不在本地 [model.*] 中定义,后者会一律报"未在 config.toml 中定义"。
    if (selectedId.value && selectedId.value !== currentDefault.value) {
      try {
        await setDefaultModel(selectedId.value);
        await stopKernel();
      } catch (e) {
        finishHint.value = `未能写入默认模型:${String(e)}`;
        return;
      }
    }
    const { markerOk } = await completeOnboarding();
    if (!markerOk) {
      // 标记文件写不进:下次启动还会再引导。不阻断进入,但要如实告知。
      finishHint.value = "已进入。完成标记写入失败,下次启动可能再次显示引导。";
      return;
    }
    emit("done");
  } finally {
    finishing.value = false;
  }
}

// 写入失败后仍要进入:只完成引导,不阻断。
async function finishAnyway(): Promise<void> {
  await completeOnboarding();
  emit("done");
}

async function skip(): Promise<void> {
  stopPolling();
  await completeOnboarding();
  emit("done");
}

onMounted(() => {
  currentStep.value = 1;
});

onBeforeUnmount(stopPolling);
</script>

<template>
  <div class="onboarding">
    <button class="skip-link" @click="skip">跳过引导</button>

    <div class="card">
      <div class="steps-dots" aria-hidden="true">
        <span
          v-for="n in TOTAL_STEPS"
          :key="n"
          class="dot"
          :class="{ active: currentStep === n, done: currentStep > n }"
        />
      </div>

      <!-- Step 1 欢迎 -->
      <section v-if="currentStep === 1" class="step">
        <h1 class="brand">QIDI 办公工作台</h1>
        <p class="lead">把办公任务交给 AI,自动产出文档、表格与汇报。</p>
        <div class="caps">
          <div class="cap">
            <div class="cap-icon">📝</div>
            <div class="cap-name">写文档</div>
            <div class="cap-desc">标书 / 周报 / 方案</div>
          </div>
          <div class="cap">
            <div class="cap-icon">📊</div>
            <div class="cap-name">做表格</div>
            <div class="cap-desc">数据整理 / 分析</div>
          </div>
          <div class="cap">
            <div class="cap-icon">🔎</div>
            <div class="cap-name">查资料</div>
            <div class="cap-desc">联网检索 / 汇总</div>
          </div>
        </div>
        <button class="btn primary lg" @click="beginSetup">开始</button>
      </section>

      <!-- Step 2 登录(设备码) -->
      <section v-else-if="currentStep === 2" class="step">
        <h2 class="step-title">登录 QIDI 账号</h2>
        <p class="lead">登录后即可使用云端模型与推理配额。</p>

        <div class="code-box">
          <span class="code-label">设备码</span>
          <code class="code-chip">{{ loginCode || "······" }}</code>
        </div>

        <button class="btn" :disabled="!loginUri" @click="openLoginUri">打开授权页面</button>

        <p class="waiting"><span class="spinner" /> 等待授权中…</p>
        <p v-if="loginError" class="err">{{ loginError }}</p>
        <p v-if="loginUri" class="uri-hint" :title="loginUri">{{ loginUri }}</p>

        <div class="nav">
          <button class="link-btn" @click="goWelcome">上一步</button>
        </div>
      </section>

      <!-- Step 3 选择模型 -->
      <section v-else class="step">
        <h2 class="step-title">选择默认模型</h2>

        <p v-if="modelsLoading" class="lead">正在获取可用模型…</p>
        <p v-else-if="modelsError" class="err">
          {{ modelsError }}
          <button v-if="modelsError.includes('登录')" class="link-btn" @click="goLogin">
            重新登录
          </button>
          <button v-else class="link-btn" @click="loadModels">重试</button>
        </p>
        <p v-else-if="!models.length" class="lead">没有可用模型。</p>
        <div v-else class="model-list">
          <button
            v-for="m in models"
            :key="m.id"
            class="model-card"
            :class="{ selected: m.id === selectedId }"
            @click="selectedId = m.id"
          >
            <span class="model-name">
              {{ m.name }}
              <span v-if="m.id === currentDefault" class="badge">当前默认</span>
            </span>
            <span class="model-id">{{ m.id }}</span>
            <span v-if="m.description" class="model-desc">{{ m.description }}</span>
          </button>
        </div>

        <p v-if="finishHint" class="hint">{{ finishHint }}</p>

        <div class="nav">
          <button class="link-btn" @click="goLogin">上一步</button>
          <button v-if="finishHint" class="btn" @click="finishAnyway">仍然进入</button>
          <button class="btn primary" :disabled="finishing" @click="finish">
            {{ finishing ? "正在完成…" : "完成设置" }}
          </button>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
/* 暗色主题:复用 global.css 的令牌名,在本作用域覆盖为暗色取值。 */
.onboarding {
  --bg-base: #12141a;
  --bg-panel: #1b1e26;
  --bg-hover: #262b36;
  --bg-active: #2d3444;
  --border: #2c313d;
  --text-primary: #e8eaed;
  --text-secondary: #a4abb8;
  --text-disabled: #6b7280;
  --accent: #4c8dff;
  --accent-soft: #1f2b45;

  position: fixed;
  inset: 0;
  z-index: 200;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  color: var(--text-primary);
  background: radial-gradient(1200px 620px at 50% -10%, #1d2740 0%, var(--bg-base) 62%);
}

.skip-link {
  position: absolute;
  top: 16px;
  right: 20px;
  background: none;
  border: none;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  cursor: pointer;
}

.skip-link:hover {
  color: var(--text-primary);
  text-decoration: underline;
}

.card {
  width: 560px;
  max-width: 100%;
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 14px;
  box-shadow: 0 16px 48px rgba(0, 0, 0, 0.45);
  padding: 32px 32px 28px;
  display: flex;
  flex-direction: column;
  gap: 18px;
}

.steps-dots {
  display: flex;
  gap: 8px;
  justify-content: center;
}

.dot {
  width: 8px;
  height: 8px;
  border-radius: 999px;
  background: var(--border);
  transition: background 0.2s;
}

.dot.active {
  background: var(--accent);
  width: 22px;
}

.dot.done {
  background: var(--text-secondary);
}

.step {
  display: flex;
  flex-direction: column;
  gap: 16px;
  text-align: center;
  align-items: center;
}

.brand {
  margin: 0;
  font-size: 24px;
  font-weight: 700;
  letter-spacing: 0.5px;
}

.step-title {
  margin: 0;
  font-size: 18px;
  font-weight: 600;
}

.lead {
  margin: 0;
  color: var(--text-secondary);
  line-height: 1.7;
}

.caps {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 12px;
  width: 100%;
}

.cap {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 16px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-base);
}

.cap-icon {
  font-size: 26px;
}

.cap-name {
  font-weight: 600;
}

.cap-desc {
  font-size: var(--font-size-sm);
  color: var(--text-disabled);
}

.code-box {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 16px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-base);
}

.code-label {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}

.code-chip {
  font-family: monospace;
  font-size: 18px;
  font-weight: 700;
  letter-spacing: 2px;
  color: var(--text-primary);
}

.waiting {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.spinner {
  width: 12px;
  height: 12px;
  border: 2px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.uri-hint {
  margin: 0;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--font-size-sm);
  color: var(--text-disabled);
}

.model-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
  width: 100%;
  max-height: 320px;
  overflow-y: auto;
}

.model-card {
  display: grid;
  grid-template-columns: 1fr auto;
  grid-template-areas:
    "name id"
    "desc desc";
  gap: 2px 10px;
  text-align: left;
  padding: 12px 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-base);
  color: var(--text-primary);
  cursor: pointer;
}

.model-card:hover {
  background: var(--bg-hover);
}

.model-card.selected {
  border-color: var(--accent);
  background: var(--accent-soft);
}

.model-name {
  grid-area: name;
  font-weight: 600;
}

.badge {
  margin-left: 6px;
  padding: 1px 6px;
  border: 1px solid var(--accent);
  border-radius: 999px;
  color: var(--accent);
  font-size: 11px;
  font-weight: 500;
}

.model-id {
  grid-area: id;
  font-family: monospace;
  font-size: var(--font-size-sm);
  color: var(--text-disabled);
  align-self: center;
}

.model-desc {
  grid-area: desc;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}

.hint {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--warning, #ff8800);
  word-break: break-all;
}

.err {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--danger, #f54a45);
  word-break: break-all;
}

.nav {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
}

.nav .link-btn {
  margin-right: auto;
}

.btn {
  border: 1px solid var(--border);
  background: var(--bg-base);
  color: var(--text-primary);
  border-radius: var(--radius);
  padding: 8px 20px;
  font-size: var(--font-size-base);
  cursor: pointer;
}

.btn:hover:not(:disabled) {
  background: var(--bg-hover);
}

.btn:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.btn.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}

.btn.primary:hover:not(:disabled) {
  filter: brightness(1.1);
}

.btn.lg {
  padding: 10px 40px;
  font-size: var(--font-size-lg);
}

.link-btn {
  background: none;
  border: none;
  color: var(--accent);
  cursor: pointer;
  font-size: var(--font-size-sm);
  padding: 0;
}

.link-btn:hover {
  text-decoration: underline;
}
</style>
