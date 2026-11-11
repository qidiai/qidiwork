<script setup lang="ts">
import { ref, reactive, nextTick, computed, watch, onBeforeUnmount } from "vue";
import { useAgentState, startSession, sendTask, cancelTurn, flushQueueNow, removeQueued, initAgent, loadEarlierMessages, replayNeedsConfirm, type ToolCallItem, type PromptImage } from "../composables/useAgent";
import { DOCUMENT_EXTENSIONS, MAX_DOCUMENT_BYTES, importDocument, readFileData, type OfficeDocument } from "../composables/useOfficeFiles";
import { usePreviewTabs } from "../composables/usePreview";
import { useOfficeState } from "../composables/useOffice";
import { handleLinkClick } from "../services/render";
import { fmtTokens, fmtCost } from "../services/usage";
import MarkdownBlock from "./MarkdownBlock.vue";
import ToolRaw from "./ToolRaw.vue";
import ModelControls from "./ModelControls.vue";
import { VIDEO_MIMES, MAX_VIDEO_BYTES, MAX_VIDEO_FRAMES, extractVideoFrames, type VideoAnalysis } from "../services/media";

const { messages, connected, sessionId, lastCreatedSessionId, turnInProgress, permission, queued, replay, deletingSessions, removedSessions } = useAgentState();
const { openPreviews, activePreview } = usePreviewTabs();
const { artifacts, currentTask } = useOfficeState();
const preview = computed(() => openPreviews.value.find((p) => p.key === activePreview.value));
const msgBox = ref<HTMLElement | null>(null);
const composerInput = ref<HTMLTextAreaElement | null>(null);

interface PendingFile {
  id: number;
  file: File;
  kind: "document" | "image" | "video";
  mimeType: string;
  previewUrl?: string;
  analysis?: VideoAnalysis;
  data?: string;
  imported?: OfficeDocument;
  importedSession?: string;
}
interface TaskDraft {
  text: string;
  files: PendingFile[];
  template: string;
  format: string;
  error: string;
}
const drafts = reactive<Record<string, TaskDraft>>({});
const currentDraft = computed(() => {
  const id = sessionId.value;
  if (!drafts[id]) drafts[id] = { text: "", files: [], template: "", format: "Word 文档", error: "" };
  return drafts[id]!;
});
const draft = computed({ get: () => currentDraft.value.text, set: (v: string) => { currentDraft.value.text = v; } });
const pendingFiles = computed(() => currentDraft.value.files);
const attachError = computed(() => currentDraft.value.error);

function draftIsEmpty(state: TaskDraft): boolean {
  return !state.text && !state.files.length && !state.template && !state.error && state.format === "Word 文档";
}

// 只有成功新建才领取首屏未归属草稿(同步整体迁移对象,不重建 File/预览 URL)。
watch(lastCreatedSessionId, (id) => {
  if (!id) return;
  const orphan = drafts[""];
  if (!orphan || draftIsEmpty(orphan)) return;
  const target = drafts[id];
  if (target && !draftIsEmpty(target)) return;
  drafts[id] = orphan;
  delete drafts[""];
}, { flush: "sync" });

const sending = ref(false);
const modelChanging = ref(false);
const preparing = ref(false);
const preparationLabel = ref("");
let preparationController: AbortController | null = null;
/** 本次附件准备所属的会话 id(该目标一旦删除就立即中止准备)。 */
let preparationSessionId = "";
const fileInput = ref<HTMLInputElement | null>(null);
const dragging = ref(false);
let pendingFileId = 0;
let dragDepth = 0;
const IMAGE_MIMES: Record<string, string> = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", webp: "image/webp", gif: "image/gif" };
const acceptFiles = [...DOCUMENT_EXTENSIONS, ...Object.keys(IMAGE_MIMES), ...Object.keys(VIDEO_MIMES)].map((e) => `.${e}`).join(",");
const hasVideo = computed(() => pendingFiles.value.some((f) => f.kind === "video"));
const canSend = computed(() => !sending.value && !modelChanging.value && !permission.value && !!(draft.value.trim() || pendingFiles.value.length));
const hasConversation = computed(() => messages.value.some((m) => m.role !== "system"));
const templates = [
  { id: "document", title: "写文档", mark: "文", desc: "周报、方案、会议纪要", hint: "告诉我文档主题、阅读对象和需要包含的要点…", format: "Word 文档" },
  { id: "sheet", title: "整理表格", mark: "表", desc: "数据清理、汇总与分析", hint: "添加表格，再说明要整理哪些字段、统计什么数据…", format: "Excel 表格" },
  { id: "slides", title: "制作汇报", mark: "报", desc: "汇报大纲、演示文稿", hint: "这次向谁汇报？主题、页数和重点是什么？", format: "PowerPoint 演示文稿" },
  { id: "bid", title: "审阅标书", mark: "审", desc: "要求提取、条款核对", hint: "添加招标文件，说明要核对的资质、工期或评分要求…", format: "Word 文档" },
];
const selectedTemplate = computed(() => templates.find((t) => t.id === currentDraft.value.template));

function selectTemplate(id: string): void {
  const template = templates.find((t) => t.id === id);
  currentDraft.value.template = template?.id ?? "";
  if (template) currentDraft.value.format = template.format;
  void nextTick(() => composerInput.value?.focus());
}

function addFiles(files: FileList | File[]): void {
  if (sending.value || permission.value) return;
  const state = currentDraft.value;
  const errors: string[] = [];
  for (const file of Array.from(files)) {
    const ext = file.name.split(".").pop()?.toLowerCase() ?? "";
    const kind = Object.hasOwn(IMAGE_MIMES, ext) ? "image" : Object.hasOwn(VIDEO_MIMES, ext) ? "video" : "document";
    const mimeType = kind === "image" ? IMAGE_MIMES[ext]! : kind === "video" ? VIDEO_MIMES[ext]! : "";
    if (!mimeType && !DOCUMENT_EXTENSIONS.includes(ext)) {
      errors.push(`「${file.name}」暂不支持，请添加办公文档、图片或 MP4 / WebM / MOV 视频`);
      continue;
    }
    const limit = kind === "video" ? MAX_VIDEO_BYTES : kind === "image" ? 10 * 1024 * 1024 : MAX_DOCUMENT_BYTES;
    if (file.size > limit) {
      errors.push(`「${file.name}」超过 ${limit / 1024 / 1024}MB 上限`);
      continue;
    }
    if (state.files.some((p) => p.file.name === file.name && p.file.size === file.size && p.file.lastModified === file.lastModified)) continue;
    if (state.files.length >= 10) {
      errors.push("每次最多添加 10 个文件，请分批处理");
      break;
    }
    const visualCount = state.files.reduce((sum, f) => sum + (f.kind === "video" ? MAX_VIDEO_FRAMES : f.kind === "image" ? 1 : 0), 0);
    if (visualCount + (kind === "video" ? MAX_VIDEO_FRAMES : kind === "image" ? 1 : 0) > 16) {
      errors.push("本次媒体画面过多，请分批发送；每个视频按最多 6 张画面计入");
      continue;
    }
    state.files.push({ id: ++pendingFileId, file, kind, mimeType, previewUrl: kind === "document" ? undefined : URL.createObjectURL(file) });
  }
  state.error = errors.join("；");
}

function cancelPreparation(): void {
  preparationController?.abort();
}

/** 预览 URL 只释放一次:释放后置空,避免重复 revoke 与内存账本。 */
function releasePreviewUrl(item: PendingFile): void {
  if (!item.previewUrl) return;
  URL.revokeObjectURL(item.previewUrl);
  item.previewUrl = undefined;
}

/** 该会话是否正在删除或已删除(准备与写回的生命周期判据)。 */
function sessionBlocked(id: string): boolean {
  return !!id && (deletingSessions.value.includes(id) || removedSessions.value.includes(id));
}

/** await 边界检查:用户中止或目标被删时立即停手,不再继续读文件/下发 prompt。 */
function checkPreparation(id: string, controller: AbortController): void {
  controller.signal.throwIfAborted();
  if (sessionBlocked(id)) {
    controller.abort();
    controller.signal.throwIfAborted();
  }
}

// 删除发起即取消该目标的准备(flush: sync,不依赖微任务先后)。
watch([deletingSessions, removedSessions], () => {
  const target = preparationSessionId;
  if (target && sessionBlocked(target)) preparationController?.abort();
}, { flush: "sync" });

// 删除成功:整体清掉该会话草稿,并一次性释放它占用的预览 URL。
watch(removedSessions, (ids) => {
  for (const id of ids) {
    const state = drafts[id];
    if (!state) continue;
    for (const item of state.files) releasePreviewUrl(item);
    delete drafts[id];
  }
}, { flush: "sync" });

function removeFile(id: number): void {
  const state = currentDraft.value;
  const file = state.files.find((p) => p.id === id);
  if (file) releasePreviewUrl(file);
  state.files = state.files.filter((p) => p.id !== id);
}

onBeforeUnmount(() => {
  preparationController?.abort();
  for (const state of Object.values(drafts)) {
    for (const item of state.files) releasePreviewUrl(item);
  }
});

function pickFiles(): void {
  if (!sending.value && !permission.value) fileInput.value?.click();
}

function onFileChange(e: Event): void {
  const input = e.target as HTMLInputElement;
  if (input.files?.length) addFiles(input.files);
  input.value = "";
}

function onPaste(e: ClipboardEvent): void {
  const files = e.clipboardData?.files;
  if (files?.length) {
    e.preventDefault();
    addFiles(files);
  }
}

function onDragEnter(e: DragEvent): void {
  if (!e.dataTransfer?.types.includes("Files")) return;
  e.preventDefault();
  dragDepth += 1;
  dragging.value = !sending.value && !permission.value;
}

function onDragOver(e: DragEvent): void {
  if (e.dataTransfer?.types.includes("Files")) e.preventDefault();
}

function onDragLeave(): void {
  dragDepth = Math.max(0, dragDepth - 1);
  if (dragDepth === 0) dragging.value = false;
}

function onDrop(e: DragEvent): void {
  e.preventDefault();
  dragDepth = 0;
  dragging.value = false;
  if (e.dataTransfer?.files.length) addFiles(e.dataTransfer.files);
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
  if (!canSend.value) return;
  const state = currentDraft.value;
  const template = selectedTemplate.value;
  const focusedFile = preview.value?.task === currentTask.value
    ? artifacts.value.find((a) => a.name === preview.value?.name) : undefined;
  let target = sessionId.value;
  sending.value = true;
  preparing.value = true;
  preparationLabel.value = "正在准备附件…";
  const controller = new AbortController();
  preparationController = controller;
  preparationSessionId = target;
  state.error = "";
  try {
    if (target && !connected.value) throw new Error("助手连接已断开，请先点击底部的「重新连接」再发送，资料和要求已保留");
    if (!target) {
      target = await startSession();
      // 新建后立刻登记:随后若这个新会话被删,同样要中止本次准备
      preparationSessionId = target;
      checkPreparation(target, controller);
    }
    const images: PromptImage[] = [];
    const docs: OfficeDocument[] = [];
    const mediaDescriptions: string[] = [];
    for (const item of state.files) {
      checkPreparation(target, controller);
      preparationLabel.value = item.kind === "video" ? `正在从「${item.file.name}」抽取画面…` : `正在读取「${item.file.name}」…`;
      if (item.kind === "video") {
        item.analysis ??= await extractVideoFrames(item.file, controller.signal);
        checkPreparation(target, controller);
        const first = images.length + 1;
        images.push(...item.analysis.frames.map((frame) => ({ data: frame.data, mimeType: frame.mimeType })));
        mediaDescriptions.push(`视频 ${JSON.stringify(item.file.name)}，时长 ${item.analysis.duration.toFixed(1)} 秒；仅均匀抽取 ${item.analysis.frames.length} 张画面，对应本消息第 ${first}–${images.length} 张图片，时间点依次为 ${item.analysis.frames.map((f) => `${f.timestamp.toFixed(2)}秒`).join("、")}。未发送原视频，不含声音，可能遗漏短暂细节；只能根据所提供画面分析，不得声称听到音频或看过完整视频。`);
      } else if (item.kind === "image") {
        if (!item.data) {
          const image = new Image();
          image.src = item.previewUrl!;
          try { await image.decode(); }
          catch { throw new Error(`无法读取图片「${item.file.name}」，请确认文件未损坏或换用 PNG / JPEG。`); }
          checkPreparation(target, controller);
        }
        item.data ??= await readFileData(item.file);
        checkPreparation(target, controller);
        images.push({ data: item.data, mimeType: item.mimeType });
        mediaDescriptions.push(`图片 ${images.length}：${JSON.stringify(item.file.name)}`);
      } else {
        if (!item.imported || item.importedSession !== target) {
          const imported = await importDocument(target, item.file);
          // 先记录已成功导入的副本,再检查生命周期:删除失败后手动重试不重复导入
          item.imported = imported;
          item.importedSession = target;
          checkPreparation(target, controller);
        }
        docs.push(item.imported);
      }
    }
    checkPreparation(target, controller);
    const parts = [state.text.trim() || "请先阅读这些资料，概括内容，并询问我希望如何处理。"];
    if (template) parts.push(`任务类型：${template.title}。期望交付：${state.format}。关键信息不足时先向我确认，不要编造数据。`);
    if (focusedFile) parts.push(`当前正在查看的成果文件：${JSON.stringify(focusedFile.path)}。修改要求针对这个文件。`);
    if (docs.length) parts.push(`以下是本次参考资料的本地副本路径，请使用文件工具读取；不要覆盖原始资料，成果另存为新文件：\n${docs.map((d) => JSON.stringify(d.path)).join("\n")}`);
    if (mediaDescriptions.length) parts.push(mediaDescriptions.join("\n"));
    preparing.value = false;
    const accepted = await sendTask(parts.join("\n\n"), images, target);
    // await 边界:目标已删则不再受理结果(不写错误、不重建草稿、不重复释放)
    checkPreparation(target, controller);
    if (!accepted) throw new Error("任务未能发送，资料和要求已保留，请重试");
    state.text = "";
    for (const item of state.files) releasePreviewUrl(item);
    state.files = [];
    state.template = "";
    await scrollToBottom();
  } catch (e) {
    // 目标已删除时静默收尾:草稿与预览 URL 已由 removedSessions watch 清掉
    if (!(target && removedSessions.value.includes(target))) {
      state.error = controller.signal.aborted ? "已取消准备，资料和要求已保留。" : String(e).replace(/^Error: /, "");
    }
  } finally {
    // 仅当本次准备仍是当前那次时收尾,不干扰其他会话的准备态
    if (preparationController === controller) {
      sending.value = false;
      preparing.value = false;
      preparationController = null;
      preparationSessionId = "";
    }
  }
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
  if (selectedTemplate.value) return selectedTemplate.value.hint;
  if (turnInProgress.value) return "可以继续补充要求，发送后将在当前步骤结束后处理…";
  if (preview.value) return "边看边提修改意见，例如：把第二段改得更简洁…";
  return "描述你要完成的工作，也可以先添加参考资料…";
});
const progressLabel = computed(() => permission.value ? "有操作需要你确认" : turnInProgress.value ? "正在处理你的任务" : queued.value.length ? "有补充要求等待处理" : "随时可以补充要求");

watch(sessionId, () => {
  dragging.value = false;
  dragDepth = 0;
  document.querySelectorAll<HTMLVideoElement>(".attachment-preview").forEach((video) => { if (video instanceof HTMLVideoElement) video.pause(); });
});
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
    <div v-if="dragging" class="drop-hint">松开以添加参考资料</div>
    <div v-if="hasConversation" class="task-status" role="status">
      <span class="status-indicator" :class="{ running: turnInProgress }"></span>
      {{ progressLabel }}
      <span v-if="queued.length" class="queue-count">{{ queued.length }} 条待处理</span>
    </div>
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

      <div v-if="!hasConversation" class="welcome">
        <p class="welcome-eyebrow">启迪 · 办公工作台</p>
        <h1 class="welcome-title">把手头的工作，交给启迪</h1>
        <p class="welcome-sub">添加资料，说清要求，拿到可以继续编辑的成果。</p>
        <div class="task-grid">
          <button v-for="task in templates" :key="task.id" type="button" class="task-card"
            :class="{ selected: currentDraft.template === task.id }" :aria-pressed="currentDraft.template === task.id"
            :disabled="sending" @click="selectTemplate(task.id)">
            <span class="task-mark">{{ task.mark }}</span>
            <strong>{{ task.title }}</strong>
            <span class="task-desc">{{ task.desc }}</span>
          </button>
        </div>
        <button class="welcome-upload" type="button" :disabled="sending || !!permission" @click="pickFiles">
          <span class="upload-plus">＋</span>
          <strong>添加参考资料，开始工作</strong>
          <span>支持办公文档、图片和视频，也可以直接拖入</span>
        </button>
        <p class="welcome-tips-label">也可以直接在下方描述任务，不必选择类型。</p>
      </div>

      <div
        v-for="msg in messages"
        :key="msg.id"
        class="msg-row"
        :class="[msg.role, { replay: msg.replay }]"
      >
        <span class="msg-badge">{{
          msg.role === "user" ? "我" : msg.role === "assistant" ? "启迪" : "系统"
        }}</span>
        <span v-if="msg.replay" class="replay-tag">回放</span>
        <div class="msg-body">
          <!-- 自动化流程清单:最新一份 todowrite 渲染为分步卡片(与编码助手
               的任务清单同源)。全量替换语义,只展示最后一份。 -->
          <div
            v-if="msg.role === 'assistant' && lastTodoIndex(msg) >= 0"
            class="flow-card"
          >
            <div class="flow-head">任务进度</div>
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

          <details
            v-if="msg.thought"
            class="thought-block"
          >
            <summary>查看思考过程</summary>
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
            :open="expandedTools[msg.id] ?? false"
            @toggle="
              expandedTools[msg.id] = ($event.target as HTMLDetailsElement).open
            "
          >
            <summary class="tool-group-head">
              处理记录 · {{ toolSummary(msg) }} · 展开详情
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
          <details v-if="msg.role === 'assistant' && msg.usage" class="usage-chip">
            <summary>本次用量</summary>
            输入 {{ fmtTokens(msg.usage.inputTokens) }} · 输出 {{ fmtTokens(msg.usage.outputTokens) }}<template v-if="msg.usage.costUsdTicks != null"> · {{ msg.usage.costPartial || msg.usage.incomplete ? "≈" : "" }}{{ fmtCost(msg.usage.costUsdTicks) }}</template><template v-if="msg.usage.models.length"> · {{ msg.usage.models.join("、") }}</template><template v-if="msg.usage.turns"> · {{ msg.usage.turns }} 轮</template>
          </details>

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

    <div class="composer-wrap">
      <div v-if="preview" class="preview-context">正在查看：{{ preview.name }} · 可直接提出修改要求</div>
      <div v-if="selectedTemplate" class="task-options">
        <strong>{{ selectedTemplate.title }}</strong>
        <label>交付格式
          <select v-model="currentDraft.format" :disabled="sending">
            <option>Word 文档</option><option>Excel 表格</option><option>PowerPoint 演示文稿</option><option>PDF 文件</option><option>仅在对话中回复</option>
          </select>
        </label>
        <button type="button" :disabled="sending" @click="selectTemplate('')">自由对话</button>
      </div>
      <div v-if="pendingFiles.length" class="attach-tray">
        <div v-for="item in pendingFiles" :key="item.id" class="attachment-chip" :class="{ 'media-attachment': item.kind !== 'document' }">
          <img v-if="item.kind === 'image'" class="attachment-preview" :src="item.previewUrl" :alt="item.file.name" />
          <video v-else-if="item.kind === 'video'" class="attachment-preview" :src="item.previewUrl" :aria-label="`预览视频 ${item.file.name}`" controls muted playsinline preload="metadata"></video>
          <div class="attachment-meta">
            <span class="file-kind">{{ item.kind === 'video' ? '视频 · 抽帧分析' : item.kind === 'image' ? '图片' : item.file.name.split('.').pop()?.toUpperCase() }}</span>
            <span class="attachment-name" :title="item.file.name">{{ item.file.name }}</span>
            <small>{{ (item.file.size / 1024 / 1024).toFixed(2) }} MB</small>
            <button type="button" :disabled="sending" :aria-label="`移除 ${item.file.name}`" @click="removeFile(item.id)">×</button>
          </div>
        </div>
      </div>
      <p v-if="hasVideo" class="media-notice">视频仅发送最多 6 张均匀采样画面，不上传原片、不含声音，可能遗漏短暂细节；图片和画面分析需模型支持图像理解。</p>
      <div v-if="preparing" class="preparation-status" role="status">{{ preparationLabel }}<button type="button" @click="cancelPreparation">取消准备</button></div>
      <div v-if="attachError" class="attach-error" role="alert">{{ attachError }}</div>
      <div class="composer">
        <input ref="fileInput" type="file" :accept="acceptFiles" multiple class="file-input" :disabled="sending || !!permission" @change="onFileChange" />
        <textarea ref="composerInput" v-model="draft" class="composer-input" rows="3" aria-label="任务要求" :placeholder="placeholder"
          :disabled="!!permission || sending" @paste="onPaste" @keydown.enter.exact="onEnterKey"
          @keydown.ctrl.enter.prevent="send" @keydown.meta.enter.prevent="send"></textarea>
        <div class="composer-actions">
          <button class="attach-btn" type="button" :disabled="sending || !!permission" title="图片≤10MB，视频≤100MB（MP4/WebM/MOV，需浏览器可解码），文档≤32MB，每次最多10个文件" @click="pickFiles">＋ 图片 / 视频 / 文件</button>
          <span class="keyboard-hint">Enter 发送 · Shift+Enter 换行</span>
          <button v-if="turnInProgress" class="stop-btn" type="button" title="停止当前任务，并清空待处理要求" @click="cancelTurn().catch((e) => currentDraft.error = String(e))">停止</button>
          <button class="composer-send" :disabled="!canSend" @click="send">{{ sending ? '正在准备…' : turnInProgress ? '补充要求' : '发送任务' }}</button>
        </div>
      </div>
      <ModelControls :disabled="sending || turnInProgress || !!permission" @busy="modelChanging = $event" />
      <p class="composer-note">文档保存本地副本；图片及视频采样画面会交给模型处理，原文件不改动。</p>
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

.welcome { max-width: 680px; margin: 24px auto; }
.welcome-eyebrow { color: var(--accent); font-size: 12px; letter-spacing: .12em; margin: 0 0 14px; }
.welcome-title { margin: 0 0 12px; font-size: clamp(22px, 2.2vw, 30px); font-weight: 600; letter-spacing: -.04em; }
.welcome-sub { margin: 0 0 26px; color: var(--text-secondary); line-height: 1.7; }
.welcome-tips-label { margin: 14px 0 0; font-size: 12px; color: var(--text-secondary); }
.task-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
.task-card { display: grid; grid-template-columns: 36px 1fr; gap: 6px 10px; text-align: left; padding: 16px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-panel); color: var(--text-primary); }
.task-card:hover, .task-card.selected { border-color: var(--accent); background: var(--accent-soft); }
.task-mark { grid-row: span 2; width: 34px; height: 36px; display: grid; place-items: center; background: var(--bg-hover); border-radius: 6px; color: var(--accent); font-size: 16px; }
.task-card strong { font-size: 14px; }
.task-desc { font-size: 12px; color: var(--text-secondary); }
.welcome-upload { width: 100%; display: flex; flex-direction: column; align-items: center; gap: 8px; margin-top: 18px; padding: 18px 12px; border: 1px dashed #b8c4d8; border-radius: 10px; background: transparent; color: var(--text-secondary); }
.welcome-upload:hover { background: var(--bg-panel); border-color: var(--accent); }
.welcome-upload strong { color: var(--text-primary); font-weight: 500; }
.welcome-upload > span:last-child { font-size: 12px; line-height: 1.6; }
.upload-plus { font-size: 24px; color: var(--accent); }
.task-status { display: flex; align-items: center; gap: 8px; padding: 10px 20px; border-bottom: 1px solid var(--border); background: var(--bg-panel); color: var(--text-secondary); font-size: 12px; }
.status-indicator { width: 7px; height: 7px; border-radius: 50%; background: var(--success); }
.status-indicator.running { background: var(--accent); animation: blink 1.4s infinite; }
.queue-count { margin-left: auto; }

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

.composer-wrap { flex-shrink: 0; padding: 12px 20px 8px; background: var(--bg-base); }
.composer { border: 1px solid #cdd5e0; border-radius: 12px; padding: 12px; background: var(--bg-panel); box-shadow: 0 3px 12px #23365005; }
.composer:focus-within { border-color: var(--accent); }
.composer-input { display: block; width: 100%; min-height: 64px; resize: vertical; max-height: 180px; border: 0; padding: 2px 4px 10px; font: inherit; line-height: 1.7; color: var(--text-primary); background: transparent; }
.composer-input:focus { outline: none; }
.composer-input:disabled { color: var(--text-disabled); }
.composer-actions { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
.composer-send { margin-left: auto; border: none; border-radius: 7px; background: var(--accent); color: #fff; padding: 8px 16px; font-size: 13px; white-space: nowrap; }
.composer-send:disabled { background: var(--accent-soft); color: var(--text-disabled); }
.stop-btn { border: 1px solid var(--border); background: var(--bg-panel); border-radius: 7px; padding: 7px 12px; color: var(--danger); }
.file-input { display: none; }
.attach-btn { border: 0; border-radius: 6px; background: var(--bg-hover); color: var(--text-secondary); padding: 7px 10px; font-size: 12px; }
.attach-btn:hover { color: var(--accent); }
.keyboard-hint { color: var(--text-disabled); font-size: 11px; }
.composer-note { margin: 7px 0 0; color: var(--text-secondary); font-size: 11px; text-align: center; line-height: 1.6; }
.task-options { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; margin-bottom: 10px; font-size: 12px; }
.task-options strong { color: var(--accent); }
.task-options select { max-width: 190px; font: inherit; border: 1px solid var(--border); border-radius: 6px; padding: 5px; margin-left: 6px; color: var(--text-primary); background: var(--bg-panel); }
.task-options button { margin-left: auto; font-size: 12px; color: var(--text-secondary); background: none; border: 0; }
.attach-tray { display: flex; flex-wrap: wrap; gap: 6px; max-height: 170px; overflow-y: auto; margin-bottom: 10px; }
.attachment-chip { max-width: 100%; padding: 7px 9px; background: var(--bg-panel); border: 1px solid var(--border); border-radius: 7px; font-size: 12px; }
.attachment-meta { display: flex; align-items: center; gap: 7px; min-width: 0; }
.media-attachment { width: 220px; }
.media-attachment .attachment-meta { flex-wrap: wrap; }
.media-attachment .attachment-name { max-width: 115px; }
.attachment-preview { display: block; width: 100%; height: 90px; object-fit: contain; border-radius: 5px; background: var(--bg-base); margin-bottom: 6px; }
.media-notice { color: var(--text-secondary); font-size: 12px; line-height: 1.6; margin: 0 0 8px; }
.preparation-status { display: flex; align-items: center; gap: 8px; justify-content: space-between; color: var(--accent); font-size: 12px; margin-bottom: 8px; }
.preparation-status button { flex-shrink: 0; border: 1px solid var(--border); border-radius: 5px; padding: 4px 8px; background: var(--bg-panel); color: var(--text-secondary); }
.attachment-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 180px; }
.attachment-chip small { white-space: nowrap; color: var(--text-secondary); }
.attachment-chip button { border: 0; background: none; color: var(--text-secondary); }
.file-kind { font-size: 10px; color: var(--accent); }
.attach-error { margin-bottom: 8px; font-size: 12px; color: var(--danger); max-height: 80px; overflow: auto; overflow-wrap: anywhere; }
.preview-context { margin-bottom: 8px; font-size: 12px; color: var(--accent); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.chat-view { min-width: 0; container-type: inline-size; }
@container (max-width: 450px) {
  .welcome { margin: 8px auto; }
  .welcome-title { font-size: 21px; }
  .task-card { padding: 10px; gap: 4px 6px; }
  .task-desc { line-height: 1.5; }
  .keyboard-hint { display: none; }
  .composer-wrap { padding: 8px 10px; }
  .chat-scroll { padding: 12px; }
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
