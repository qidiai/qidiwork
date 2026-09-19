<script setup lang="ts">
// 工具原始数据明细:仅在展开时挂载本组件才序列化(性能审计:
// 折叠状态下 JSON.stringify 也在每次重渲染执行)。
import { computed } from "vue";

const props = defineProps<{ raw: unknown }>();
const text = computed(() => JSON.stringify(props.raw, null, 2));
</script>

<template>
  <pre class="tool-raw">{{ text }}</pre>
</template>

<style scoped>
.tool-raw {
  margin: 0;
  padding: 8px 10px;
  border-top: 1px solid var(--border);
  max-height: 220px;
  overflow: auto;
  font-size: var(--font-size-sm);
}
</style>
