<script setup lang="ts">
import { ref, nextTick, computed } from "vue";
import { useAgentState, sendTask, cancelTurn, flushQueueNow, removeQueued, initAgent, loadEarlierMessages, replayNeedsConfirm, type ToolCallItem, type PromptImage } from "../composables/useAgent";
import { handleLinkClick } from "../services/render";
import { fmtTokens, fmtCost } from "../services/usage";
import MarkdownBlock from "./MarkdownBlock.vue";
import ToolRaw from "./ToolRaw.vue";

const { messages, connected, turnInProgress, permission, queued, replay } = useAgentState();
const draft = ref("");
const msgBox = ref<HTMLElement | null>(null);

// ---------------------------------------------------------- 图片附件(P1-2) --
// 待发图片:📎 按钮 / 粘贴 / 拖拽三路汇入同一队列;base64(去前缀)+ 缩略图。
// 单张 ≤ 10MB 与 MIME 白名单前端先校验(与 bridge 侧一致),不合规即时提示。
interface PendingImage {
  id: number;
  name: string;
  mimeType: string;
  /** base64 无前缀(下发给内核) */
  data: string;
  /** data URL(仅用于缩略图预览) */
  preview: string;
}

const MAX_IMAGE_BYTES = 10 * 1024 * 1024;
const ALLOWED_IMAGE_MIMES = ["image/png", "image/jpeg", "image/webp", "image/gif"];
const pendingImages = ref<PendingImage[]>([]);
const attachError = ref("");
const fileInput = ref<HTMLInputElement | null>(null);
const dragging = ref(false);
let pendingImageId = 0;
// 拖拽进入/离开的深度计数:子元素间移动也会触发 leave,计数归零才算真正离开
let dragDepth = 0;

function readAsDataURL(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () => reject(reader.error ?? new Error("读取失败"));
    reader.readAsDataURL(file);
  });
}

async function addFiles(files: FileList | File[]): Promise<void> {
  attachError.value = "";
  for (const file of Array.from(files)) {
    if (!file.type.startsWith("image/")) continue;
    if (!ALLOWED_IMAGE_MIMES.includes(file.type)) {
      attachError.value = `不支持的图片类型 ${file.type || file.name}:仅支持 PNG/JPEG/WebP/GIF`;
      continue;
    }
    if (file.size > MAX_IMAGE_BYTES) {
      attachError.value = `图片「${file.name || "未命名"}」超过 10MB 上限,已跳过`;
      continue;
    }
    try {
      const dataUrl = await readAsDataURL(file);
      const comma = dataUrl.indexOf(",");
      if (comma < 0) continue;
      const header = dataUrl.slice(0, comma); // data:image/png;base64
      const data = dataUrl.slice(comma + 1);
      const mimeType = /^data:([^;]+);base64$/.exec(header)?.[1] || file.type || "image/png";
      pendingImages.value.push({
        id: ++pendingImageId,
        name: file.name || "image",
        mimeType,
        data,
        preview: dataUrl,
      });
    } catch (e) {
      attachError.value = `图片读取失败:${String(e)}`;
    }
  }
}

function removeImage(id: number): void {
  pendingImages.value = pendingImages.value.filter((p) => p.id !== id);
}

function pickFiles(): void {
  fileInput.value?.click();
}

function onFileChange(e: Event): void {
  const input = e.target as HTMLInputElement;
  if (input.files?.length) void addFiles(input.files);
  input.value = ""; // 复位:允许重复选择同一文件
}

// 粘贴图片(clipboardData.items)
function onPaste(e: ClipboardEvent): void {
  const items = e.clipboardData?.items;
  if (!items) return;
  const files: File[] = [];
  for (const item of items) {
    if (item.kind === "file" && item.type.startsWith("image/")) {
      const f = item.getAsFile();
      if (f) files.push(f);
    }
  }
  if (files.length) {
    e.preventDefault();
    void addFiles(files);
  }
}

// 拖拽图片入聊天区
function onDragEnter(e: DragEvent): void {
  if (!e.dataTransfer?.types.includes("Files")) return;
  e.preventDefault();
  dragDepth += 1;
  dragging.value = true;
}

function onDragOver(e: DragEvent): void {
  if (e.dataTransfer?.types.includes("Files")) e.preventDefault();
}

function onDragLeave(): void {
  dragDepth = Math.max(0, dragDepth - 1);
  if (dragDepth === 0) dragging.value = false;
}

function onDrop(e: DragEvent): void {
  dragDepth = 0;
  dragging.value = false;
  const files = e.dataTransfer?.files;
  if (files?.length) {
    e.preventDefault();
    void addFiles(files);
  }
}

// 回放折叠横幅「加载全部」:大会话(>10MB)先确认再拉齐全部历史。
async function onLoadAll(): Promise<void> {
  if (
    replayNeedsConfirm() &&
    !window.confirm("该会话记录较大(超过 10MB),加载全部可能较慢,是否继续?")
  ) {
    return;
  }
  await loadEarlierMessages();
}

// 外链点击统一拦截(系统浏览器打开,webview 不导航)。
function onMsgClick(e: MouseEvent): void {
  handleLinkClick(e);
}

// 每条消息的工具调用分组折叠(默认收起,只留一行摘要);
// 消息 id → 展开。工具执行期间自动展开,完成后收起。
const expandedTools = ref<Record<number, boolean>>({});
// 单条工具明细展开态(key=消息id:工具id):未展开不挂载 ToolRaw,
// 避免折叠时也对 raw 做 JSON.stringify(性能审计)
const expandedRaw = ref<Record<string, boolean>>({});

// 自动化流程清单:agent 用 todowrite 工具维护分步流程(与编码助手同源)。
// 从工具调用原始数据提取 todos;全量替换语义 → 最新一份即当前流程状态。
interface TodoItem {
  content: string;
  status: string;
}

const todoCache = new WeakMap<object, TodoItem[] | null>();

function todoListOf(tool: ToolCallItem): TodoItem[] | null {
  // 工具名门槛:只认 todowrite 家族,其他工具 raw 里恰好嵌套 todos 数组
  // (如读到的 todo JSON 文件)不误判为流程卡片(k3 补审计)
  if (!/todo/i.test(tool.title)) return null;
  if (!tool.raw || typeof tool.raw !== "object") return null;
  if (todoCache.has(tool.raw)) return todoCache.get(tool.raw) ?? null;
  const find = (o: unknown): TodoItem[] | null => {
    if (!o || typeof o !== "object") return null;
    const obj = o as Record<string, unknown>;
    if (Array.isArray(obj.todos)) {
      const list = obj.todos.filter(
        (x): x is TodoItem =>
          !!x && typeof x === "object" && typeof (x as TodoItem).content === "string",
      );
      if (list.length) return list;
    }
    for (const v of Object.values(obj)) {
      const found = find(v);
      if (found) return found;
    }
    return null;
  };
  const list = find(tool.raw);
  todoCache.set(tool.raw, list);
  return list;
}

function lastTodoIndex(msg: (typeof messages)["value"][number]): number {
  let last = -1;
  msg.toolCalls.forEach((t, i) => {
    if (todoListOf(t)) last = i;
  });
  return last;
}

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
  const images: PromptImage[] = pendingImages.value.map((p) => ({
    data: p.data,
    mimeType: p.mimeType,
  }));
  if (!text && !images.length) {
    // 两次 Enter 语义(qidicode):运行中空输入按 Enter = 取消当前回合,
    // 排队消息立即续跑
    if (queued.value.length) flushQueueNow();
    return;
  }
  draft.value = "";
  pendingImages.value = [];
  attachError.value = "";
  // 回合进行中 sendTask 自动排队(回合结束 FIFO 续跑),不再禁止输入
  await sendTask(text, images);
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

const placeholder = computed(() => {
  if (!connected.value) return "发送任务将自动连接内核并开启会话(Enter 发送)";
  if (turnInProgress.value)
    return "任务运行中:输入后 Enter 排队,回合结束自动发送;再按 Enter(空输入)= 立即插入";
  return "输入任务…(Enter 发送,Shift+Enter 换行)";
});

// 首次进入即初始化事件监听(幂等)
void initAgent();
</script>

<template>
  <div
    class="chat-view"
    :class="{ dragging }"
    @dragenter="onDragEnter"
    @dragover="onDragOver"
    @dragleave="onDragLeave"
    @drop="onDrop"
  >
    <div v-if="dragging" class="drop-hint">松开以添加图片</div>
    <div ref="msgBox" class="chat-scroll" @click="onMsgClick" @scroll.passive>
      <!-- 回放折叠横幅:更早消息未加载时显示(需要时一次拉齐) -->
      <div
        v-if="replay && !replay.fullyLoaded && replay.total > replay.loaded"
        class="replay-banner"
      >
        <span class="replay-banner-text">
          已加载最近 {{ replay.loaded }} 条 · 共 {{ replay.total }} 条
        </span>
        <button
          class="replay-load-btn"
          :disabled="replay.loading"
          @click="onLoadAll"
        >
          {{ replay.loading ? "加载中…" : "加载全部" }}
        </button>
      </div>

      <div v-if="messages.length === 0" class="welcome">
        <div class="welcome-icon">Q</div>
        <h1 class="welcome-title">QIDI 办公工作台</h1>
        <p class="welcome-sub">用一句话下达办公任务:写标书、做周报、转换文档、生成图纸</p>
        <div class="welcome-tips">
          <span class="tip">试试:「根据附件大纲生成立标函初稿」</span>
          <span class="tip">试试:「把这周会议纪要整理成周报」</span>
        </div>
      </div>

      <div
        v-for="msg in messages"
        :key="msg.id"
        class="msg-row"
        :class="[msg.role, { replay: msg.replay }]"
      >
        <span class="msg-badge">{{
          msg.role === "user" ? "我" : msg.role === "assistant" ? "QIDI" : "系统"
        }}</span>
        <span v-if="msg.replay" class="replay-tag">回放</span>
        <div class="msg-body">
          <!-- 自动化流程清单:最新一份 todowrite 渲染为分步卡片(与编码助手
               的任务清单同源)。全量替换语义,只展示最后一份。 -->
          <div
            v-if="msg.role === 'assistant' && lastTodoIndex(msg) >= 0"
            class="flow-card"
          >
            <div class="flow-head">📋 自动化流程</div>
            <ol class="flow-steps">
              <li
                v-for="(todo, i) in todoListOf(
                  msg.toolCalls[lastTodoIndex(msg)]!,
                )!"
                :key="i"
                class="flow-step"
                :data-status="todo.status"
              >
                <span class="flow-icon">{{
                  todo.status === "completed"
                    ? "✓"
                    : todo.status === "in_progress"
                      ? "▶"
                      : todo.status === "cancelled"
                        ? "⊘"
                        : "○"
                }}</span>
                <span class="flow-text" :class="{ done: todo.status === 'completed' }">{{
                  todo.content
                }}</span>
              </li>
            </ol>
          </div>

          <!-- 思考过程:流式可见,完成后自动收起(方向不对可随时中止) -->
          <details
            v-if="msg.thought"
            class="thought-block"
            :open="msg.role === 'assistant' && !msg.done"
          >
            <summary>💭 思考过程</summary>
            <div class="thought-text">{{ msg.thought }}</div>
          </details>

          <!-- 流式 markdown:节流渲染 + 结果缓存(MarkdownBlock) -->
          <MarkdownBlock
            v-if="msg.role !== 'user'"
            class="msg-content md"
            :src="msg.content"
          />
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
              {{ toolSummary(msg) }} · 明细
            </summary>
            <details
              v-for="tool in msg.toolCalls"
              :key="tool.toolCallId"
              class="tool-call"
              @toggle="
                expandedRaw[`${msg.id}:${tool.toolCallId}`] = (
                  $event.target as HTMLDetailsElement
                ).open
              "
            >
              <summary>
                <span class="tool-status" :data-status="tool.status">{{
                  statusLabel(tool.status)
                }}</span>
                {{ tool.title }}
              </summary>
              <ToolRaw
                v-if="expandedRaw[`${msg.id}:${tool.toolCallId}`]"
                :raw="tool.raw"
              />
            </details>
          </details>

          <!-- 回合用量:token/成本/模型(内核 _meta.usage;mock 或未上报
               时 usage 为空,不占位)。≈ = 内核标记账单可能不完整 -->
          <div
            v-if="msg.role === 'assistant' && msg.usage"
            class="usage-chip"
          >
            Tokens ↑{{ fmtTokens(msg.usage.inputTokens) }} ↓{{ fmtTokens(msg.usage.outputTokens) }}<template v-if="msg.usage.costUsdTicks != null"> · {{ msg.usage.costPartial || msg.usage.incomplete ? "≈" : "" }}{{ fmtCost(msg.usage.costUsdTicks) }}</template><template v-if="msg.usage.models.length"> · {{ msg.usage.models.join("、") }}</template><template v-if="msg.usage.turns"> · {{ msg.usage.turns }} 轮</template>
          </div>

          <span
            v-if="msg.role === 'assistant' && !msg.done"
            class="typing"
            aria-label="生成中"
          ></span>
        </div>
      </div>

      <!-- 排队中的消息:运行中提交,回合结束自动续跑;可撤回或立即插入 -->
      <div v-for="q in queued" :key="q.id" class="msg-row user">
        <span class="msg-badge queued-badge">排队</span>
        <div class="msg-body">
          <div class="msg-content user-text">{{ q.text || (q.images?.length ? `📎 ${q.images.length} 张图片` : "") }}</div>
          <div class="queued-actions">
            <button
              class="queued-btn"
              title="取消当前回合并立即发送这条排队消息"
              @click="flushQueueNow()"
            >▶ 立即插入</button>
            <button class="queued-btn" title="撤回这条排队消息" @click="removeQueued(q.id)">✕ 撤回</button>
          </div>
        </div>
      </div>
    </div>

    <div v-if="pendingImages.length || attachError" class="attach-tray">
      <div v-if="pendingImages.length" class="attach-thumbs">
        <div v-for="img in pendingImages" :key="img.id" class="attach-thumb">
          <img :src="img.preview" :alt="img.name" :title="img.name" />
          <button class="attach-remove" type="button" title="移除" @click="removeImage(img.id)">×</button>
        </div>
      </div>
      <div v-if="attachError" class="attach-error">{{ attachError }}</div>
    </div>
    <div class="composer">
      <input
        ref="fileInput"
        type="file"
        accept="image/*"
        multiple
        class="file-input"
        @change="onFileChange"
      />
      <button
        class="attach-btn"
        type="button"
        title="添加图片(PNG/JPEG/WebP/GIF,单张 ≤10MB;也可粘贴或拖拽)"
        @click="pickFiles"
      >📎</button>
      <textarea
        v-model="draft"
        class="composer-input"
        rows="2"
        :placeholder="placeholder"
        :disabled="!!permission"
        @paste="onPaste"
        @keydown.enter.exact="onEnterKey"
        @keydown.ctrl.enter.prevent="send"
        @keydown.meta.enter.prevent="send"
      ></textarea>
      <button
        v-if="turnInProgress"
        class="composer-send cancel"
        title="中止当前回合(排队消息一并清空)"
        @click="cancelTurn()"
      >
        中止
      </button>
      <button
        class="composer-send"
        :disabled="(!draft.trim() && !pendingImages.length) || !!permission"
        @click="send"
      >
        {{ turnInProgress ? "排队" : "发送" }}
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
  position: relative;
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

/* 历史回放消息(R5):浅色底 + 「回放」角标,与实时消息区分(克制,不加复杂结构) */
.msg-row.replay .msg-content {
  background: var(--bg-hover);
}
.msg-row.replay.assistant .msg-content {
  border-style: dashed;
}
.replay-tag {
  flex: none;
  font-size: var(--font-size-sm);
  color: var(--text-disabled);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 2px 6px;
  margin-top: 2px;
}

/* 回放折叠横幅:更早消息未加载时显示「已加载最近 N 条 · 共 M 条 [加载全部]」 */
.replay-banner {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  margin: 0 0 14px;
  padding: 5px 14px;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 999px;
}
.replay-load-btn {
  border: 1px solid var(--border);
  background: var(--bg-base);
  border-radius: 999px;
  color: var(--accent);
  font-size: var(--font-size-sm);
  padding: 2px 12px;
  cursor: pointer;
}
.replay-load-btn:hover:not(:disabled) {
  border-color: var(--accent);
  background: var(--accent-soft);
}
.replay-load-btn:disabled {
  color: var(--text-disabled);
  cursor: default;
}

/* 排队消息(运行中提交):徽标区分 + 行内操作 */
.queued-badge {
  color: var(--accent);
  border-color: var(--accent-soft);
  background: var(--accent-soft);
}

.queued-actions {
  display: flex;
  gap: 8px;
  margin-top: 4px;
}

.queued-btn {
  border: 1px solid var(--border);
  background: var(--bg-base);
  border-radius: var(--radius);
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  padding: 2px 10px;
  cursor: pointer;
}

.queued-btn:hover {
  color: var(--text-primary);
  border-color: var(--accent);
}

/* 自动化流程清单卡片(todowrite 可视化) */
.flow-card {
  margin-bottom: 8px;
  border: 1px solid var(--accent-soft);
  border-radius: var(--radius);
  background: var(--bg-panel);
  font-size: var(--font-size-sm);
  overflow: hidden;
}

.flow-head {
  padding: 6px 12px;
  background: var(--accent-soft);
  color: var(--text-primary);
  font-weight: 600;
}

.flow-steps {
  list-style: none;
  margin: 0;
  padding: 8px 12px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.flow-step {
  display: flex;
  align-items: flex-start;
  gap: 8px;
}

.flow-icon {
  flex: none;
  color: var(--text-disabled);
}

.flow-step[data-status="completed"] .flow-icon {
  color: var(--success);
}

.flow-step[data-status="in_progress"] .flow-icon {
  color: var(--accent);
}

.flow-text {
  color: var(--text-primary);
  line-height: 1.5;
}

.flow-text.done {
  color: var(--text-secondary);
  text-decoration: line-through;
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

/* 回合用量 chip:弱化展示,不与正文抢注意力 */
.usage-chip {
  margin-top: 4px;
  font-size: var(--font-size-sm);
  color: var(--text-disabled);
  user-select: none;
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

/* 图片附件(P1-2):隐藏的原生 file input + 📎 触发按钮 */
.file-input {
  display: none;
}

.attach-btn {
  align-self: flex-end;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-base);
  color: var(--text-secondary);
  padding: 10px 12px;
  font-size: var(--font-size-base);
  cursor: pointer;
}

.attach-btn:hover {
  border-color: var(--accent);
  color: var(--text-primary);
}

/* 待发图片托盘:缩略图(≤96px 高)+ 移除按钮 + 校验提示 */
.attach-tray {
  padding: 8px 16px 0;
  background: var(--bg-panel);
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.attach-thumbs {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.attach-thumb {
  position: relative;
  height: 96px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--bg-base);
}

.attach-thumb img {
  height: 100%;
  max-width: 160px;
  object-fit: contain;
  display: block;
}

.attach-remove {
  position: absolute;
  top: 2px;
  right: 2px;
  width: 18px;
  height: 18px;
  line-height: 1;
  border: none;
  border-radius: 50%;
  background: rgba(0, 0, 0, 0.55);
  color: #fff;
  font-size: 12px;
  cursor: pointer;
  padding: 0;
}

.attach-remove:hover {
  background: var(--danger);
}

.attach-error {
  font-size: var(--font-size-sm);
  color: var(--danger);
}

/* 拖拽图片时的整窗高亮提示 */
.drop-hint {
  position: absolute;
  inset: 8px;
  z-index: 5;
  display: flex;
  align-items: center;
  justify-content: center;
  border: 2px dashed var(--accent);
  border-radius: var(--radius);
  background: var(--accent-soft);
  color: var(--accent);
  font-size: var(--font-size-base);
  pointer-events: none;
}
</style>
