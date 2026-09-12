<script setup lang="ts">
import { ref, nextTick, computed } from "vue";
import { useAgentState, sendTask, cancelTurn, initAgent } from "../composables/useAgent";
import { renderMarkdown, handleLinkClick } from "../services/render";

const { messages, connected, turnInProgress, permission } = useAgentState();
const draft = ref("");
const msgBox = ref<HTMLElement | null>(null);

// 外链点击统一拦截(系统浏览器打开,webview 不导航)。
function onMsgClick(e: MouseEvent): void {
  handleLinkClick(e);
}

// 每条消息的工具调用分组折叠(默认收起,只留一行摘要);
// 消息 id → 展开。工具执行期间自动展开,完成后收起。
const expandedTools = ref<Record<number, boolean>>({});

function toolSummary(msg: (typeof messages)["value"][number]): string {
  const done = msg.toolCalls.filter((t) => t.status === "completed").length;
  const failed = msg.toolCalls.filter((t) => t.status === "failed").length;
  const running = msg.toolCalls.filter(
    (t) => t.status === "in_progress" || t.status === "pending",
  ).length;
  const parts: string[] = [];
  if (done) parts.push(`✓${done}`);
  if (running) parts.push(`运行中 ${running}`);
  if (failed) parts.push(`✗${failed}`);
  return parts.length ? parts.join(" ") : "等待中…";
}

function statusLabel(status: string): string {
  switch (status) {
    case "completed":
      return "✓ 完成";
    case "failed":
      return "✗ 失败";
    case "in_progress":
      return "运行中…";
    default:
      return status || "等待中…";
  }
}

async function send() {
  const text = draft.value.trim();
  if (!text || turnInProgress.value) return;
  draft.value = "";
  await sendTask(text);
  await scrollToBottom();
}

// Enter 发送;中文输入法组词中的回车放行给 IME(确认选字),不触发发送
function onEnterKey(e: KeyboardEvent): void {
  if (e.isComposing || e.keyCode === 229) return;
  e.preventDefault();
  void send();
}

async function scrollToBottom() {
  await nextTick();
  msgBox.value?.scrollTo({ top: msgBox.value.scrollHeight });
}

const placeholder = computed(() =>
  connected.value
    ? "输入任务…(Enter 发送,Shift+Enter 换行)"
    : "发送任务将自动连接内核并开启会话(Enter 发送)",
);

// 首次进入即初始化事件监听(幂等)
void initAgent();
</script>

<template>
  <div class="chat-view">
    <div ref="msgBox" class="chat-scroll" @click="onMsgClick" @scroll.passive>
      <div v-if="messages.length === 0" class="welcome">
        <div class="welcome-icon">Q</div>
        <h1 class="welcome-title">QIDI 办公工作台</h1>
        <p class="welcome-sub">用一句话下达办公任务:写标书、做周报、转换文档、生成图纸</p>
        <div class="welcome-tips">
          <span class="tip">试试:「根据附件大纲生成立标函初稿」</span>
          <span class="tip">试试:「把这周会议纪要整理成周报」</span>
        </div>
      </div>

      <div v-for="msg in messages" :key="msg.id" class="msg-row" :class="msg.role">
        <span class="msg-badge">{{
          msg.role === "user" ? "我" : msg.role === "assistant" ? "QIDI" : "系统"
        }}</span>
        <div class="msg-body">
          <!-- 思考过程:流式可见,完成后自动收起(方向不对可随时中止) -->
          <details
            v-if="msg.thought"
            class="thought-block"
            :open="msg.role === 'assistant' && !msg.done"
          >
            <summary>💭 思考过程</summary>
            <div class="thought-text">{{ msg.thought }}</div>
          </details>

          <!-- eslint-disable-next-line vue/no-v-html: 内容已经 DOMPurify 消毒 -->
          <div
            v-if="msg.role !== 'user'"
            class="msg-content md"
            v-html="renderMarkdown(msg.content)"
          ></div>
          <div v-else class="msg-content user-text">{{ msg.content }}</div>

          <!-- 工具调用分组:一条摘要代替 N 行,展开看逐条与原始数据 -->
          <details
            v-if="msg.toolCalls.length"
            class="tool-group"
            :open="
              expandedTools[msg.id] ??
              msg.toolCalls.some(
                (t) => t.status === 'in_progress' || t.status === 'pending',
              )
            "
            @toggle="
              expandedTools[msg.id] = ($event.target as HTMLDetailsElement).open
            "
          >
            <summary class="tool-group-head">
              🔧 工具调用 × {{ msg.toolCalls.length }} ·
              {{ toolSummary(msg) }}(点击{{ expandedTools[msg.id] ? "收起" : "展开" }})
            </summary>
            <details
              v-for="tool in msg.toolCalls"
              :key="tool.toolCallId"
              class="tool-call"
            >
              <summary>
                <span class="tool-status" :data-status="tool.status">{{
                  statusLabel(tool.status)
                }}</span>
                {{ tool.title }}
              </summary>
              <pre class="tool-raw">{{ JSON.stringify(tool.raw, null, 2) }}</pre>
            </details>
          </details>

          <span
            v-if="msg.role === 'assistant' && !msg.done"
            class="typing"
            aria-label="生成中"
          ></span>
        </div>
      </div>
    </div>

    <div class="composer">
      <textarea
        v-model="draft"
        class="composer-input"
        rows="2"
        :placeholder="placeholder"
        :disabled="turnInProgress || !!permission"
        @keydown.enter.exact="onEnterKey"
        @keydown.ctrl.enter.prevent="send"
        @keydown.meta.enter.prevent="send"
      ></textarea>
      <button
        v-if="!turnInProgress"
        class="composer-send"
        :disabled="!draft.trim() || !!permission"
        @click="send"
      >
        发送
      </button>
      <button v-else class="composer-send cancel" @click="cancelTurn()">
        中止
      </button>
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
  padding: 18px 20px;
}

.welcome {
  text-align: center;
  max-width: 520px;
  margin: 8vh auto 0;
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

.msg-row {
  display: flex;
  gap: 10px;
  margin-bottom: 14px;
  align-items: flex-start;
}

.msg-badge {
  flex: none;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 2px 8px;
  margin-top: 2px;
}

.msg-row.user .msg-badge {
  color: var(--accent);
  border-color: var(--accent-soft);
  background: var(--accent-soft);
}

.msg-body {
  flex: 1;
  min-width: 0;
}

.msg-content {
  line-height: 1.7;
  word-break: break-word;
}

.msg-row.assistant .msg-content {
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 10px 14px;
}

.msg-row.system .msg-content {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.user-text {
  white-space: pre-wrap;
}

/* 思考过程块:与正文区分,弱化显示 */
.thought-block {
  margin-bottom: 8px;
  border: 1px dashed var(--border);
  border-radius: var(--radius);
  background: var(--bg-panel);
  font-size: var(--font-size-sm);
}

.thought-block summary {
  cursor: pointer;
  padding: 6px 10px;
  color: var(--text-secondary);
}

.thought-text {
  padding: 4px 12px 10px;
  color: var(--text-secondary);
  white-space: pre-wrap;
  line-height: 1.6;
  max-height: 260px;
  overflow: auto;
}

/* 工具调用分组:默认只占一行 */
.tool-group {
  margin-top: 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-panel);
  font-size: var(--font-size-sm);
}

.tool-group-head {
  cursor: pointer;
  padding: 6px 10px;
  color: var(--text-secondary);
  user-select: none;
}

.tool-group > .tool-call {
  margin: 0 8px 8px;
}

.tool-call {
  margin-top: 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-panel);
  font-size: var(--font-size-sm);
}

.tool-call summary {
  cursor: pointer;
  padding: 6px 10px;
  color: var(--text-secondary);
}

.tool-status {
  margin-right: 6px;
  color: var(--accent);
}

.tool-status[data-status="failed"] {
  color: var(--danger);
}

.tool-status[data-status="completed"] {
  color: var(--success);
}

.tool-raw {
  margin: 0;
  padding: 8px 10px;
  border-top: 1px solid var(--border);
  max-height: 220px;
  overflow: auto;
  font-size: var(--font-size-sm);
}

/* 极简打点动画 */
.typing::after {
  content: "…";
  animation: blink 1.2s steps(3) infinite;
  color: var(--text-disabled);
}

@keyframes blink {
  50% {
    opacity: 0.3;
  }
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

.composer-send.cancel {
  background: var(--danger);
}
</style>
