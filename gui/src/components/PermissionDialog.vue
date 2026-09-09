<script setup lang="ts">
// 权限审批对话框:ACP request_permission 的前端落点。
// 「允许」走 resolve(选中项),「拒绝」= deny 选项,「取消/关闭」= cancelled。
import { ref, watch } from "vue";
import { useAgentState, resolvePermission, cancelPermission } from "../composables/useAgent";

const { permission } = useAgentState();
// 防双击重复 invoke(k3 M4 审计 P1-3):一次交互只发一次应答
const settled = ref(false);
watch(permission, () => {
  settled.value = false;
});

function pick(optionId: string) {
  if (settled.value) return;
  settled.value = true;
  resolvePermission(optionId);
}

function dismiss() {
  if (settled.value) return;
  settled.value = true;
  cancelPermission();
}

function isDangerous(kind?: string): boolean {
  return kind === "reject_once" || kind === "reject_always";
}
</script>

<template>
  <div v-if="permission" class="dialog-mask">
    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="perm-title">
      <h2 id="perm-title" class="dialog-title">需要你的确认</h2>
      <p class="dialog-sub">{{ permission.title }}</p>
      <p class="dialog-hint">任务会话:{{ permission.sessionId }}</p>
      <div class="option-row">
        <button
          v-for="opt in permission.options"
          :key="opt.id"
          class="option-btn"
          :class="{ danger: isDangerous(opt.kind) }"
          @click="pick(opt.id)"
        >
          {{ opt.name }}
        </button>
      </div>
      <button class="dismiss" @click="dismiss()">暂不处理(取消)</button>
    </div>
  </div>
</template>

<style scoped>
.dialog-mask {
  position: fixed;
  inset: 0;
  background: rgba(15, 18, 25, 0.45);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.dialog {
  width: 420px;
  max-width: calc(100vw - 48px);
  background: var(--bg-panel);
  border-radius: 12px;
  padding: 20px 22px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.22);
}

.dialog-title {
  margin: 0 0 6px;
  font-size: 17px;
}

.dialog-sub {
  margin: 0 0 4px;
  color: var(--text-primary);
}

.dialog-hint {
  margin: 0 0 16px;
  font-size: var(--font-size-sm);
  color: var(--text-disabled);
}

.option-row {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
}

.option-btn {
  flex: 1;
  border: none;
  border-radius: var(--radius);
  padding: 10px 14px;
  font-size: var(--font-size-base);
  background: var(--accent);
  color: #fff;
}

.option-btn.danger {
  background: var(--bg-hover);
  color: var(--danger);
  border: 1px solid var(--danger);
}

.dismiss {
  margin-top: 14px;
  width: 100%;
  background: none;
  border: none;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}
</style>
