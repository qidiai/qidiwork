<script setup lang="ts">
// 权限审批对话框:ACP request_permission 的前端落点。
// 选项按钮走 resolve(optionId),「暂不处理」= cancelled。
import { computed, ref, watch } from "vue";
import {
  useAgentState,
  resolvePermission,
  cancelPermission,
  pendingPermissionCount,
} from "../composables/useAgent";

const { permission, permissionSubmitting } = useAgentState();
// 防双击重复 invoke(k3 M4 审计 P1-3):一次交互只发一次应答
const settled = ref(false);
watch(permission, () => {
  settled.value = false;
});

const busy = computed(() => settled.value || permissionSubmitting.value);
const remainingQueued = computed(() => pendingPermissionCount() - 1);

// 内核给的选项名是英文("Allow once" 等),办公用户看不懂;按 ACP 的四种
// kind 中文化,kind 缺失或属未来新增时回落原串,绝不编造按钮。
const OPTION_LABELS: Record<string, string> = {
  allow_once: "允许这一次",
  allow_always: "总是允许",
  reject_once: "拒绝这一次",
  reject_always: "总是拒绝",
};

function labelOf(opt: { name: string; kind?: string }): string {
  return (opt.kind ? OPTION_LABELS[opt.kind] : undefined) ?? opt.name;
}

function pick(optionId: string) {
  if (busy.value) return;
  settled.value = true;
  resolvePermission(optionId);
}

function dismiss() {
  if (busy.value) return;
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
      <p v-if="remainingQueued > 0" class="dialog-hint">
        后面还有 {{ remainingQueued }} 项待确认,处理完这项会接着问下一项。
      </p>
      <div class="option-row">
        <button
          v-for="opt in permission.options"
          :key="opt.optionId"
          class="option-btn"
          :class="{ danger: isDangerous(opt.kind) }"
          :disabled="busy"
          @click="pick(opt.optionId)"
        >
          {{ labelOf(opt) }}
        </button>
      </div>
      <button class="dismiss" :disabled="busy" @click="dismiss()">暂不处理(取消)</button>
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

.option-btn:disabled,
.dismiss:disabled {
  opacity: 0.55;
  cursor: default;
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
