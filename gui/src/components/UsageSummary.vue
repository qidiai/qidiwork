<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useAgentState } from "../composables/useAgent";
import { fmtCost, fmtTokens } from "../services/usage";

const { sessionUsage, runningUsage, sessionId } = useAgentState();
const details = ref<HTMLDetailsElement | null>(null);
// chip = 会话累计 + 回合中运行态快照(输入+输出);cost 行不动(运行值无可信成本)。
const tokens = computed(
  () =>
    sessionUsage.value.inputTokens +
    sessionUsage.value.outputTokens +
    (runningUsage.value?.inputTokens ?? 0) +
    (runningUsage.value?.outputTokens ?? 0),
);
const cost = computed(() => sessionUsage.value.costSeen ? `${sessionUsage.value.costPartial ? '≈ ' : ''}${fmtCost(sessionUsage.value.costUsdTicks)}` : "尚无统计");
watch(sessionId, () => { if (details.value) details.value.open = false; });
</script>

<template>
  <details ref="details" class="usage-summary">
    <summary aria-label="查看本任务用量与费用">
      <span>用量 <strong>{{ tokens ? `${fmtTokens(tokens)} tokens` : '尚无统计' }}</strong></span>
      <span>费用 <strong>{{ cost }}</strong></span>
    </summary>
    <div class="usage-popover">
      <strong>本任务 · 已上报用量</strong>
      <dl>
        <div><dt>输入 tokens</dt><dd>{{ tokens ? (sessionUsage.inputTokens + (runningUsage?.inputTokens ?? 0)).toLocaleString() : '尚无统计' }}</dd></div>
        <div><dt>输出 tokens</dt><dd>{{ tokens ? (sessionUsage.outputTokens + (runningUsage?.outputTokens ?? 0)).toLocaleString() : '尚无统计' }}</dd></div>
        <div><dt>已上报费用（美元）</dt><dd>{{ cost }}</dd></div>
      </dl>
      <p v-if="!sessionUsage.costSeen">尚未收到费用数据，不代表免费。</p>
      <p v-else-if="sessionUsage.costPartial">部分费用未完整上报，显示金额可能少计。</p>
      <p>只统计本次打开后收到的数据，历史回放不计入；最终扣费以服务商账单为准。</p>
    </div>
  </details>
</template>

<style scoped>
.usage-summary { position: relative; font-size: 12px; color: var(--text-secondary); }
.usage-summary summary { cursor: pointer; display: flex; flex-wrap: wrap; gap: 8px 16px; padding: 7px 10px; border: 1px solid var(--border); border-radius: 7px; background: var(--bg-panel); }
.usage-summary summary::after { content: '⌄'; }
.usage-summary summary strong { color: var(--text-primary); font-weight: 500; margin-left: 4px; }
.usage-popover { position: absolute; right: 0; top: calc(100% + 6px); z-index: 20; width: min(330px, 75vw); padding: 16px; background: var(--bg-panel); color: var(--text-primary); border: 1px solid var(--border); border-radius: 9px; box-shadow: 0 6px 20px #0001; }
.usage-popover dl > div { display: flex; justify-content: space-between; gap: 12px; padding: 6px 0; }
.usage-popover dd { margin: 0; }
.usage-popover p { margin: 8px 0 0; color: var(--text-secondary); line-height: 1.7; }
</style>
