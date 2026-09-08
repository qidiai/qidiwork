<script setup lang="ts">
import { ref } from "vue";

// 对话区。M1 为静态骨架:内核状态由 M2(ACP 桥)接入后启用输入,
// 消息流 / 工具调用折叠 / 流式渲染在 M4 实现。
const draft = ref("");

function send() {
  // M2/M3 接入 ACP session/prompt 后实现
}
</script>

<template>
  <div class="chat-view">
    <div class="chat-scroll">
      <div class="welcome">
        <div class="welcome-icon">Q</div>
        <h1 class="welcome-title">QIDI 办公工作台</h1>
        <p class="welcome-sub">用一句话下达办公任务:写标书、做周报、转换文档、生成图纸</p>
        <div class="welcome-tips">
          <span class="tip">试试:「根据附件大纲生成立标函初稿」</span>
          <span class="tip">试试:「把这周会议纪要整理成周报」</span>
        </div>
      </div>
    </div>

    <div class="composer">
      <textarea
        v-model="draft"
        class="composer-input"
        rows="2"
        placeholder="内核未连接 · ACP 桥接入后可用(方案 P0/M2)"
        disabled
      ></textarea>
      <button class="composer-send" :disabled="true" @click="send">发送</button>
    </div>
  </div>
</template>

<style scoped>
.chat-view {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.chat-scroll {
  flex: 1;
  overflow-y: auto;
  display: flex;
  align-items: center;
  justify-content: center;
}

.welcome {
  text-align: center;
  max-width: 520px;
  padding: 24px;
}

.welcome-icon {
  width: 64px;
  height: 64px;
  margin: 0 auto 16px;
  border-radius: 16px;
  background: var(--accent);
  color: #fff;
  font-size: 36px;
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
}

.welcome-title {
  margin: 0 0 8px;
  font-size: 22px;
}

.welcome-sub {
  margin: 0 0 16px;
  color: var(--text-secondary);
}

.welcome-tips {
  display: flex;
  flex-direction: column;
  gap: 6px;
  align-items: center;
}

.tip {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 999px;
  padding: 4px 12px;
}

.composer {
  display: flex;
  gap: 10px;
  padding: 12px 16px;
  background: var(--bg-panel);
  border-top: 1px solid var(--border);
}

.composer-input {
  flex: 1;
  resize: none;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 10px 12px;
  font-family: inherit;
  font-size: var(--font-size-base);
  color: var(--text-primary);
  background: var(--bg-base);
}

.composer-input:focus {
  outline: none;
  border-color: var(--accent);
}

.composer-input:disabled {
  color: var(--text-disabled);
}

.composer-send {
  align-self: flex-end;
  border: none;
  border-radius: var(--radius);
  background: var(--accent);
  color: #fff;
  padding: 10px 22px;
  font-size: var(--font-size-base);
}

.composer-send:disabled {
  background: var(--accent-soft);
  color: var(--text-disabled);
}
</style>
