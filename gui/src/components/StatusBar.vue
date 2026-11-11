<script setup lang="ts">
import { onMounted, ref, computed, defineAsyncComponent } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useAgentState, recoverAgent, startSession, pushSystem } from "../composables/useAgent";
import { useAuth } from "../composables/useAuth";
const SettingsModal = defineAsyncComponent(() => import("./SettingsModal.vue"));

const { connected, sessionId } = useAgentState();
const { status: auth, refresh: refreshAuth } = useAuth();
const guiVersion = ref("…");
const recovering = ref(false);
const showSettings = ref(false);
const accountLabel = computed(() => auth.value?.logged_in
  ? `${auth.value.email ?? auth.value.user_id ?? "已登录"}${auth.value.plan ? ` · ${auth.value.plan}` : ""}` : "未登录");
const kernelLabel = computed(() => connected.value ? (sessionId.value ? "助手已就绪" : "已连接") : "发送任务即可开始");

async function recover() {
  recovering.value = true;
  try { await recoverAgent(); }
  catch (e) { pushSystem(`重新连接失败：${String(e)}`); }
  finally { recovering.value = false; }
}
async function newSession() {
  try { await startSession(); }
  catch (e) { pushSystem(`新建任务失败：${String(e)}`); }
}
onMounted(async () => {
  try { guiVersion.value = (await invoke<string>("app_version")) ?? ""; }
  catch { guiVersion.value = "版本暂不可用"; }
  void refreshAuth();
});
</script>

<template>
  <footer class="status-bar">
    <span class="status-item"><span class="dot" :class="connected ? 'on' : 'off'"></span>{{ kernelLabel }}</span>
    <button v-if="!connected" class="link-btn" :disabled="recovering" @click="recover">{{ recovering ? '连接中…' : '重新连接' }}</button>
    <button v-else class="link-btn" @click="newSession">新任务</button>
    <span class="spacer"></span>
    <button class="link-btn account" :class="{ muted: !auth?.logged_in }" :title="auth?.logged_in ? '账号与配额' : '点击登录启迪账号'" @click="showSettings = true">{{ accountLabel }}</button>
    <span class="status-item muted">v{{ guiVersion }}</span>
    <button class="link-btn" @click="showSettings = true">设置</button>
  </footer>
  <SettingsModal v-if="showSettings" @close="showSettings = false" />
</template>

<style scoped>
.status-bar { display: flex; align-items: center; gap: 12px; padding: 5px 14px; background: var(--bg-panel); border-top: 1px solid var(--border); font-size: var(--font-size-sm); color: var(--text-secondary); user-select: none; }
.spacer { flex: 1; }
.status-item { display: inline-flex; align-items: center; gap: 6px; }
.muted { color: var(--text-disabled); }
.dot { width: 8px; height: 8px; border-radius: 50%; }
.dot.off { background: var(--text-disabled); }
.dot.on { background: var(--success); }
.link-btn { background: none; border: none; color: var(--accent); font-size: var(--font-size-sm); padding: 0; }
.account { max-width: 240px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
