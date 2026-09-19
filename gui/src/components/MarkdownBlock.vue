<script setup lang="ts">
// 流式 markdown 块:内容变化经 50ms 节流再重渲染(性能审计:每 token
// 全量重解析大 markdown 是长回答掉帧主因)。首帧同步渲染,尾帧必达:
// 节流窗口内的更新暂存,窗口到期后应用最新值。
import { ref, watch, onBeforeUnmount } from "vue";
import { renderMarkdown } from "../services/render";

const props = defineProps<{ src: string }>();
const html = ref(renderMarkdown(props.src));
let pending: string | null = null;
let timer: ReturnType<typeof setTimeout> | null = null;

watch(
  () => props.src,
  (next) => {
    if (timer === null) {
      html.value = renderMarkdown(next);
      timer = setTimeout(() => {
        timer = null;
        if (pending !== null) {
          html.value = renderMarkdown(pending);
          pending = null;
        }
      }, 50);
    } else {
      pending = next;
    }
  },
);

onBeforeUnmount(() => {
  if (timer !== null) clearTimeout(timer);
});
</script>

<template>
  <!-- eslint-disable-next-line vue/no-v-html: 内容已经 DOMPurify 消毒 -->
  <div v-html="html"></div>
</template>
