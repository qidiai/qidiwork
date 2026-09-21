// Agent/桥状态中心:M2 传输 + M3 ACP 桥的前端单向镜像。
// 事件契约来自 commands.rs:acp-event(BridgeEvent,serde tag=type)/ agent-exit。
// 可靠性分层(k3 审计 W3 厘清):传输层→桥入站为可靠 mpsc 单消费者,
// 协议帧不丢;桥→前端是 broadcast 展示流,转发任务滞后超 EVENT_CAPACITY
// 会 Lagged 丢帧(Rust 侧记日志)——丢 TurnUsage 仅缺用量 chip,丢
// TurnCompleted 会 busy 卡住,用户可用「中止」恢复。
//
// 多会话分桶(2026-09-12):每会话独立 messages/busy/queue,事件按
// session_id 路由到桶,后台会话的流式更新不丢。回合进行中仍可输入:
// 新消息进该会话的 queue,回合结束自动 FIFO 续跑;「立即插入」= 取消
// 当前回合并立即续跑队列(qidicode TUI 的两次 Enter 语义)。中止则
// 连同队列一起清空(除非正处于「立即插入」的取消路径)。桥层本就按
// session 多路复用(prompt 独立等应答、事件带 session_id),后端零改动。
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { initNotifyPermission, isWindowFocused, notify, notifyThrottled } from "../services/notify";
import { parseUsageMeta, accumulateUsage, emptySessionUsage, type UsageInfo, type SessionUsage } from "../services/usage";
import { currentWorkspaceName, switchTask } from "./useOffice";

export interface ToolCallItem {
  toolCallId: string;
  title: string;
  status: string;
  raw: unknown;
}

export interface ChatMessage {
  id: number;
  role: "user" | "assistant" | "system";
  content: string;
  /** 模型思考过程(agent_thought_chunk 流式累积;渲染为可折叠块) */
  thought?: string;
  toolCalls: ToolCallItem[];
  done: boolean;
  /** 本回合用量(turn_usage 事件在 turn_completed 前到达时挂载) */
  usage?: UsageInfo;
  /** 历史回放消息(R5 续接回放):与实时消息样式区分,只读、不参与实时流 */
  replay?: boolean;
}

/** 排队中的用户消息(回合进行中提交,回合结束自动续跑)。 */
export interface QueuedMessage {
  id: number;
  text: string;
}

export interface PermissionState {
  requestId: number;
  sessionId: string;
  title: string;
  options: { id: string; name: string; kind?: string }[];
}

/** 侧栏「任务会话」条目:活动会话的运行态快照。 */
export interface SessionListItem {
  id: string;
  busy: boolean;
  queued: number;
  title: string;
}

interface AcpEvent {
  type: string;
  session_id?: string;
  update?: { sessionUpdate?: string; content?: { text?: string }; toolCallId?: string; title?: string; status?: string; [k: string]: unknown };
  request_id?: number;
  tool_call?: { title?: string };
  options?: { id: string; name: string; kind?: string }[];
  stop_reason?: string;
  session_restored?: never;
  error?: string;
  /** turn_usage 事件:session/prompt 应答 _meta 原样透传(桥层) */
  meta?: unknown;
  /** model_state 事件:内核 SessionModelState 原样透传(桥层) */
  state?: unknown;
}

/** 会话模型状态(session/new|load 应答的 models 字段,内核原样)。 */
export interface ModelState {
  currentModelId: string;
  availableModels: { modelId: string; name: string }[];
}

/** 宽松解析内核模型状态(字段缺失/形态异常一律降级为 null,不抛错)。 */
export function parseModelState(v: unknown): ModelState | null {
  if (!v || typeof v !== "object") return null;
  const o = v as { currentModelId?: unknown; availableModels?: unknown };
  const current = typeof o.currentModelId === "string" ? o.currentModelId : "";
  const available = Array.isArray(o.availableModels)
    ? o.availableModels
        .map((m) => {
          const mm = (m ?? {}) as { modelId?: unknown; name?: unknown };
          return {
            modelId: String(mm.modelId ?? ""),
            name: typeof mm.name === "string" && mm.name ? mm.name : String(mm.modelId ?? ""),
          };
        })
        .filter((m) => m.modelId)
    : [];
  if (!current && !available.length) return null;
  return { currentModelId: current, availableModels: available };
}

/** 内核 chat_history.jsonl 单条原始消息(Anthropic 消息格式)。 */
interface RawTranscriptMessage {
  type?: string;
  content?: unknown;
  summary?: unknown;
  tool_calls?: unknown;
  [k: string]: unknown;
}

/** transcript 命令返回页(字段名与 Rust 结构一致,serde snake_case)。 */
interface TranscriptPage {
  messages: RawTranscriptMessage[];
  total_messages: number;
  truncated: boolean;
  missing: boolean;
  loaded_upto: number;
  size_bytes: number;
}

/** 单会话回放状态(折叠横幅 + 翻页游标)。 */
export interface ReplayState {
  /** 该会话的工作目录(定位内核转录文件所需) */
  cwd: string;
  /** summary.json 报告的消息总数(横幅「共 N 条」) */
  total: number;
  /** 已回放进转录区的条数 */
  loaded: number;
  /** 当前最早已加载行的全局行号(0 = 已到文件头) */
  earliest: number;
  /** 前面已无更多历史(横幅消失) */
  fullyLoaded: boolean;
  /** 正在拉取更早消息 */
  loading: boolean;
  /** 会话文件字节数(>10MB 时「加载全部」先确认) */
  sizeBytes: number;
}

/** 单会话桶:界面转录 + 回合状态 + 排队消息。 */
interface SessionBucket {
  id: string;
  messages: ChatMessage[];
  busy: boolean;
  queue: QueuedMessage[];
  /** 「立即插入」已请求取消:turn(cancelled) 到达时保留队列并直接续跑 */
  flushing: boolean;
  /** 首条 prompt 已回写标题(恢复的会话为 true,不覆盖登记簿) */
  titleRecorded: boolean;
  title: string;
  /** 待挂载的回合用量(turn_usage 先于 turn_completed 到达) */
  pendingUsage: UsageInfo | null;
  /** 会话累计用量(token/成本/模型清单) */
  usage: SessionUsage;
  /** 内核报告的模型状态(未拿到前为 null,状态栏降级用到合上报) */
  modelState: ModelState | null;
  /** 续接/崩溃恢复时的历史回放状态(未回放为 null) */
  replay: ReplayState | null;
}

const buckets = ref<Record<string, SessionBucket>>({});
const activeId = ref<string>("");
const connected = ref(false);
const lastError = ref("");
let messageId = 0;
let listenersBound = false;

/** 权限请求 FIFO:多会话并发审批时排队展示,不互相覆盖。 */
const permissionQueue = ref<PermissionState[]>([]);

/** 会话创建前的系统提示(如自动连接失败):暂存,首个桶创建时补挂。 */
let pendingSystem: ChatMessage[] = [];

function newSystemMessage(content: string): ChatMessage {
  return { id: ++messageId, role: "system", content, toolCalls: [], done: true };
}

function ensureBucket(id: string): SessionBucket {
  if (!buckets.value[id]) {
    buckets.value[id] = {
      id,
      messages: [],
      busy: false,
      queue: [],
      flushing: false,
      titleRecorded: false,
      title: "",
      pendingUsage: null,
      usage: emptySessionUsage(),
      modelState: null,
      replay: null,
    };
    // 先建桶再补挂暂存消息,保证响应式(读回代理再改)
    const bucket = buckets.value[id]!;
    if (pendingSystem.length) {
      bucket.messages.push(...pendingSystem);
      pendingSystem = [];
    }
  }
  return buckets.value[id]!;
}

function bucketOf(id: string | undefined | null): SessionBucket | null {
  if (!id) return null;
  return buckets.value[id] ?? null;
}

function activeBucket(): SessionBucket | null {
  return bucketOf(activeId.value);
}

/** 指定会话的系统提示(进对应桶的转录)。 */
function pushBucketSystem(bucket: SessionBucket, text: string): void {
  bucket.messages.push(newSystemMessage(text));
}

/** 全局系统提示:进当前活动会话;无会话时暂存,待首个会话创建补挂。 */
export function pushSystem(text: string): void {
  const bucket = activeBucket();
  if (bucket) {
    pushBucketSystem(bucket, text);
  } else {
    pendingSystem.push(newSystemMessage(text));
  }
}

function pushAssistantChunk(bucket: SessionBucket, text: string) {
  const last = bucket.messages[bucket.messages.length - 1];
  if (last && last.role === "assistant" && !last.done) {
    last.content += text;
  } else {
    bucket.messages.push({
      id: ++messageId,
      role: "assistant",
      content: text,
      thought: "",
      toolCalls: [],
      done: false,
    });
  }
}

/** 思考过程流式累积(agent_thought_chunk)。 */
function pushThoughtChunk(bucket: SessionBucket, text: string): void {
  const last = bucket.messages[bucket.messages.length - 1];
  if (last && last.role === "assistant" && !last.done) {
    last.thought = (last.thought ?? "") + text;
  } else {
    bucket.messages.push({
      id: ++messageId,
      role: "assistant",
      content: "",
      thought: text,
      toolCalls: [],
      done: false,
    });
  }
}

function upsertToolCall(bucket: SessionBucket, update: NonNullable<AcpEvent["update"]>) {
  const last = bucket.messages[bucket.messages.length - 1];
  if (!last || last.role !== "assistant") return;
  const id = String(update.toolCallId ?? update.title ?? "tool");
  const title = String(update.title ?? "工具调用");
  const status = String(update.status ?? "pending");
  const existing = last.toolCalls.find((t) => t.toolCallId === id);
  if (existing) {
    existing.title = title;
    existing.status = status;
    existing.raw = update;
  } else {
    last.toolCalls.push({ toolCallId: id, title, status, raw: update });
  }
}

// ------------------------------------------------------- 会话转录回放(R5) --
//
// 续接(session_resume)/ 崩溃恢复(agent_recover)成功后,内核只在自身侧续接
// 上下文,界面默认从新消息开始。这里直读内核会话文件回放最近若干条,并把更早
// 消息折叠成顶部横幅(需要时「加载全部」)。一切失败静默降级,不阻断续接。

/** 续接时回放的尾部条数。 */
const REPLAY_TAIL_LIMIT = 50;
/** 「加载全部」前的大文件确认阈值(10MB)。 */
const REPLAY_CONFIRM_BYTES = 10 * 1024 * 1024;

/** session_id → cwd 缓存(登记簿字段;回放定位文件所需)。 */
const sessionCwd = new Map<string, string>();

/** session_id → 绑定的办公任务工作区名(登记簿 task 字段;产物面板同步所需)。 */
const sessionTask = new Map<string, string>();

/** 登记某会话的 cwd(续接点击处已知,免去额外拉取)。 */
export function rememberSessionCwd(id: string, cwd: string): void {
  if (id && cwd) sessionCwd.set(id, cwd);
}

/** 登记某会话的办公工作区绑定(新建/续接点击处已知,免去额外拉取)。 */
export function rememberSessionBinding(id: string, task?: string | null): void {
  if (id && task) sessionTask.set(id, task);
}

/** 登记簿整表加载一次(并发去重:崩溃恢复会同时抛多个 session_restored)。 */
let registryLoading: Promise<void> | null = null;
async function ensureRegistryLoaded(): Promise<void> {
  if (!registryLoading) {
    registryLoading = (async () => {
      try {
        const list = await invoke<
          { session_id: string; cwd: string; task?: string | null }[]
        >("sessions_history");
        for (const h of list) {
          sessionCwd.set(h.session_id, h.cwd);
          if (h.task) sessionTask.set(h.session_id, h.task);
        }
      } catch {
        /* 登记簿读取失败:回放/绑定同步降级为不可用 */
      } finally {
        registryLoading = null;
      }
    })();
  }
  await registryLoading;
}

/** 取会话 cwd:先查缓存,未命中则拉一次登记簿(失败静默,回退 null)。 */
async function cwdFor(id: string): Promise<string | null> {
  const cached = sessionCwd.get(id);
  if (cached) return cached;
  await ensureRegistryLoaded();
  return sessionCwd.get(id) ?? null;
}

/** 取会话绑定的办公工作区名:先查缓存,未命中则拉一次登记簿(失败静默)。 */
async function taskFor(id: string): Promise<string | null> {
  const cached = sessionTask.get(id);
  if (cached) return cached;
  await ensureRegistryLoaded();
  return sessionTask.get(id) ?? null;
}

/** 活动会话切换时:按登记簿绑定把产物面板切到该会话的工作区(会话 → 面板
 * 单向同步)。无绑定/读取失败保持现状,不打扰用户当前视图。 */
async function syncWorkspaceForSession(id: string): Promise<void> {
  if (!id) return;
  try {
    const task = await taskFor(id);
    if (task) await switchTask(task);
  } catch {
    /* 面板同步失败:保持现状,不阻断会话切换 */
  }
}

/** 内核原始消息抽取纯文本(user 的 content 可能是 content-blocks 数组)。 */
function rawText(raw: RawTranscriptMessage): string {
  const c = raw.content;
  if (typeof c === "string") return c;
  if (Array.isArray(c)) {
    const parts: string[] = [];
    for (const block of c) {
      if (!block || typeof block !== "object") continue;
      const b = block as { type?: string; text?: unknown };
      if (b.type === "text" && typeof b.text === "string") parts.push(b.text);
    }
    return parts.join("");
  }
  return "";
}

/** 内核原始消息 → 前端 ChatMessage。
 * 只回放 user/assistant(system/reasoning/tool_result 等内部类型跳过,避免刷屏);
 * 纯工具调用的 assistant 回合渲染为一行工具摘要,不重建权限管道。 */
function rawToChatMessage(raw: RawTranscriptMessage): ChatMessage | null {
  const role = raw.type;
  if (role !== "user" && role !== "assistant") return null;
  const content = rawText(raw);
  if (!content.trim()) {
    if (role === "assistant" && Array.isArray(raw.tool_calls) && raw.tool_calls.length) {
      const names = raw.tool_calls
        .map((t) => String((t as { name?: unknown })?.name ?? "工具"))
        .join("、");
      return {
        id: ++messageId,
        role,
        content: `🔧 调用工具:${names}`,
        toolCalls: [],
        done: true,
        replay: true,
      };
    }
    return null;
  }
  return { id: ++messageId, role, content, toolCalls: [], done: true, replay: true };
}

/** 把原始消息数组转成可渲染的 ChatMessage 列表(过滤掉跳过的类型)。 */
function toReplayMessages(raws: RawTranscriptMessage[]): ChatMessage[] {
  const out: ChatMessage[] = [];
  for (const raw of raws) {
    const m = rawToChatMessage(raw);
    if (m) out.push(m);
  }
  return out;
}

/** 回放某会话最近若干条历史(续接/恢复成功后调用,失败静默降级)。 */
async function injectReplay(bucket: SessionBucket, cwd: string): Promise<void> {
  let page: TranscriptPage;
  try {
    page = await invoke<TranscriptPage>("session_transcript_tail", {
      sessionId: bucket.id,
      cwd,
      limit: REPLAY_TAIL_LIMIT,
    });
  } catch (e) {
    pushBucketSystem(bucket, `历史消息回放不可用:${String(e)}`);
    return;
  }
  const msgs = toReplayMessages(page.messages ?? []);
  if (page.missing || !msgs.length) {
    pushBucketSystem(
      bucket,
      `会话 ${bucket.id} 已恢复(未找到可回放的历史记录,本次界面从新消息开始)。`,
    );
    return;
  }
  // 回放消息按原顺序置于转录区顶部,再补一条分界提示
  bucket.messages = msgs.concat(bucket.messages);
  bucket.replay = {
    cwd,
    total: page.total_messages,
    // 「已加载最近 N 条」按窗口内加载的原始消息数计(与 limit 对齐;内部类型
    // 不渲染,故可见气泡数可能少于 N)
    loaded: page.messages.length,
    earliest: page.loaded_upto,
    fullyLoaded: page.loaded_upto === 0,
    loading: false,
    sizeBytes: page.size_bytes,
  };
  pushBucketSystem(
    bucket,
    `已回放该会话最近 ${page.messages.length} 条历史消息(历史上下文已在内核侧续接)。`,
  );
}

/** 续接/恢复成功后:解析 cwd 并回放(无 cwd 时降级为提示)。 */
async function replayRestored(bucket: SessionBucket): Promise<void> {
  const cwd = await cwdFor(bucket.id);
  if (!cwd) {
    pushBucketSystem(
      bucket,
      `会话 ${bucket.id} 已恢复(历史上下文已在内核侧续接;本次界面从新消息开始)。`,
    );
    return;
  }
  await injectReplay(bucket, cwd);
}

/** 「加载全部」:一次拉齐当前窗口之前的全部历史,并序合并到转录区顶部。
 * 拉齐后横幅消失(fullyLoaded);>10MB 的文件确认在组件层先做。 */
export async function loadEarlierMessages(): Promise<void> {
  const bucket = activeBucket();
  const r = bucket?.replay;
  if (!bucket || !r || r.loading || r.fullyLoaded) return;
  if (r.earliest <= 0) {
    r.fullyLoaded = true;
    return;
  }
  r.loading = true;
  try {
    const page = await invoke<TranscriptPage>("session_transcript_earlier", {
      sessionId: bucket.id,
      cwd: r.cwd,
      before: r.earliest,
      limit: r.earliest,
    });
    const msgs = toReplayMessages(page.messages ?? []);
    bucket.messages = msgs.concat(bucket.messages);
    r.loaded += page.messages.length;
    r.earliest = page.loaded_upto;
    r.fullyLoaded = true;
  } catch (e) {
    pushBucketSystem(bucket, `加载更早消息失败:${String(e)}`);
  } finally {
    r.loading = false;
  }
}

/** 会话文件是否超过「加载全部」确认阈值。 */
export function replayNeedsConfirm(): boolean {
  const r = activeBucket()?.replay;
  return !!r && r.sizeBytes > REPLAY_CONFIRM_BYTES;
}

async function onAcpEvent(ev: AcpEvent) {
  switch (ev.type) {
    case "session_update": {
      // 事件按 session_id 入桶:后台会话的流式更新持续累积,切回可见
      const bucket = bucketOf(ev.session_id);
      if (!bucket) break;
      if (ev.update?.sessionUpdate === "agent_message_chunk") {
        pushAssistantChunk(bucket, ev.update.content?.text ?? "");
      } else if (ev.update?.sessionUpdate === "agent_thought_chunk") {
        // 思考过程可见(P2 反馈):用户能看到推理与数据来源,方向不对可及时中止
        pushThoughtChunk(bucket, ev.update.content?.text ?? "");
      } else if (ev.update?.sessionUpdate === "tool_call" || ev.update?.sessionUpdate === "tool_call_update") {
        upsertToolCall(bucket, ev.update);
      }
      // plan 等其余更新类型 M4 暂不渲染
      break;
    }
    case "permission_request":
      permissionQueue.value.push({
        requestId: ev.request_id ?? 0,
        sessionId: ev.session_id ?? "",
        title: ev.tool_call?.title ?? "agent 请求权限",
        options: ev.options ?? [],
      });
      // 失焦时系统提醒,办公用户切走窗口也不错过审批(P2);同 title 节流
      if (!isWindowFocused()) {
        notifyThrottled(
          permissionQueue.value[permissionQueue.value.length - 1]?.title ?? "agent 请求权限",
          "QIDI 办公工作台",
          "等待权限审批,点击窗口处理。",
        );
      }
      break;
    case "turn_usage": {
      // 回合用量:先暂存桶上,turn_completed 时挂到刚完成的 assistant
      // 消息并累加进会话累计(桥层保证 usage 先于 completed 发出)
      const bucket = bucketOf(ev.session_id);
      if (!bucket) break;
      bucket.pendingUsage = parseUsageMeta(ev.meta);
      break;
    }
    case "turn_completed": {
      const bucket = bucketOf(ev.session_id);
      if (!bucket) break;
      bucket.busy = false;
      const wasFlushing = bucket.flushing;
      bucket.flushing = false;
      {
        const last = bucket.messages[bucket.messages.length - 1];
        if (last && last.role === "assistant") {
          last.done = true;
          if (bucket.pendingUsage) last.usage = bucket.pendingUsage;
        }
        accumulateUsage(bucket.usage, bucket.pendingUsage);
        bucket.pendingUsage = null;
      }
      const reason = ev.stop_reason ?? "";
      if (reason.startsWith("error:")) {
        lastError.value = reason;
        if (!isWindowFocused()) {
          // 出错恰是最需要召回的场景(k3 审计),失焦必通知
          notify("QIDI 办公工作台", "任务出错,点击窗口查看详情。");
        }
      } else if (reason === "cancelled" || reason === "rejected") {
        if (wasFlushing) {
          // 「立即插入」的取消:队列是用户意图,保留并立即续跑
          drain(bucket);
          break;
        }
        // 用户主动中止:连同排队消息一起清空,避免中止后突然续跑
        if (bucket.queue.length) {
          const n = bucket.queue.length;
          bucket.queue = [];
          pushBucketSystem(bucket, `已中止任务,并清空 ${n} 条排队消息。`);
        }
        break;
      } else if (!isWindowFocused()) {
        // 失焦时系统提醒任务完成(P2)
        notify("QIDI 办公工作台", "任务已完成,点击窗口查看结果。");
      }
      drain(bucket);
      break;
    }
    case "disconnected":
      connected.value = false;
      // 全部桶停表;队列随连接失效清空(重连后续跑旧队列语义不明,宁缺)
      for (const b of Object.values(buckets.value)) {
        b.busy = false;
        b.flushing = false;
        b.queue = [];
        b.pendingUsage = null;
      }
      permissionQueue.value = [];
      {
        const bucket = activeBucket();
        pushBucketSystem(
          bucket ?? ensurePlaceholder(),
          "内核已断开。点击状态栏「恢复」或重新发送任务以重连(会话将自动续接)。",
        );
      }
      break;
    case "model_state": {
      const bucket = bucketOf(ev.session_id);
      const st = parseModelState(ev.state);
      if (bucket && st) bucket.modelState = st;
      break;
    }
    case "session_restored": {
      const id = ev.session_id;
      if (!id) break;
      const existing = bucketOf(id);
      const bucket = ensureBucket(id);
      activeId.value = id;
      connected.value = true;
      // 恢复的会话标题已在登记簿里,首条 prompt 不覆盖
      bucket.titleRecorded = true;
      if (!existing) {
        // 新恢复的会话:回放最近历史(续接/崩溃恢复同一路径;失败静默降级)
        void replayRestored(bucket);
      }
      // 重连换桥后模型缓存是新的:补查一次,状态栏不空窗
      void refreshModelState(id);
      // 续接/恢复后按登记簿还原绑定,产物面板跟着会话走(无绑定保持现状)
      void syncWorkspaceForSession(id);
      // 桶已存在(切回活动会话)时不重放提示、不清转录,保留实时流
      break;
    }
    case "session_restore_failed": {
      const bucket = bucketOf(ev.session_id);
      const text = `会话 ${ev.session_id} 恢复失败:${ev.error ?? "未知原因"}`;
      if (bucket) pushBucketSystem(bucket, text);
      else pushSystem(text);
      break;
    }
  }
}

/** disconnected 时可能尚无任何桶:借一个兜底桶承载提示,不进会话列表。 */
function ensurePlaceholder(): SessionBucket {
  return ensureBucket("");
}

/** 队列自动续跑:回合空闲且有排队消息时发出下一条(FIFO)。 */
function drain(bucket: SessionBucket): void {
  if (bucket.busy) return;
  const next = bucket.queue.shift();
  if (!next) return;
  void doSend(bucket, next.text);
}

/** 实际下发一条 prompt(前置:该会话空闲)。失败时置闲并提示,队列
 * 不自动续跑(下发失败通常是连接问题,续跑会连环失败刷屏)。 */
async function doSend(bucket: SessionBucket, text: string): Promise<void> {
  bucket.messages.push({
    id: ++messageId,
    role: "user",
    content: text,
    toolCalls: [],
    done: true,
  });
  bucket.busy = true;
  try {
    await invoke("session_prompt", { sessionId: bucket.id, text });
    // 首条 prompt 截断记为会话标题(失败不影响任务下发)
    if (!bucket.titleRecorded) {
      bucket.titleRecorded = true;
      bucket.title = Array.from(text).slice(0, 40).join("");
      // 标题按字符截断,避免 emoji 代理对被切半(k3 审计次要项)
      void invoke("session_set_title", {
        sessionId: bucket.id,
        title: bucket.title,
      }).catch(() => {});
    }
  } catch (e) {
    bucket.busy = false;
    pushBucketSystem(bucket, `任务下发失败:${String(e)}`);
  }
}

/** 应用启动时调用一次:绑定事件监听并探测内核状态。 */
export async function initAgent(): Promise<void> {
  if (listenersBound) return;
  listenersBound = true;
  const unlisteners: UnlistenFn[] = [];
  unlisteners.push(await listen<AcpEvent>("acp-event", (e) => onAcpEvent(e.payload)));
  unlisteners.push(
    await listen<{ code: number | null }>("agent-exit", () => {
      // 进程级退出由 acp-event/disconnected 表达;此处仅兜底标记
      connected.value = false;
    }),
  );
  void unlisteners; // 监听与应用同生命周期,无需解绑
  void initNotifyPermission(); // 失焦系统提醒的权限,启动时请求一次
}

/** 用户最近一次显式选择的工作目录:新建/自动开会话都默认复用它。 */
const LAST_CWD_KEY = "qidi.lastCwd";

/** 会话绑定的办公工作区名取值策略:优先「当前工作区」(GUI 的工作区由
 * card.py 按 --task 创建,无独立创建入口,故以用户当前查看的工作区为首选);
 * 未选工作区时回退 cwd 目录名;两者皆无则不绑定。 */
function resolveOfficeTask(cwd: string | undefined): string | undefined {
  const current = currentWorkspaceName();
  if (current) return current;
  if (cwd) {
    const base = cwd.split(/[\\/]/).filter(Boolean).pop();
    if (base) return base;
  }
  return undefined;
}

/** 新建任务会话(用户点「+ 新建任务」或首次发送)。回合进行中也可开
 * 新会话:各会话桶独立,后台会话继续跑。未显式传 cwd 时复用上次选择的
 * 目录;都没选过则交给内核默认(用户主目录)。
 * 新建即绑定:会话创建成功后把 task=工作区名落进登记簿(后端 upsert),
 * 并让产物面板切到该工作区。 */
export async function startSession(cwd?: string): Promise<void> {
  const dir = cwd ?? localStorage.getItem(LAST_CWD_KEY) ?? undefined;
  const officeTask = resolveOfficeTask(dir);
  const id = await invoke<string>("session_start", {
    cwd: dir ?? null,
    officeTask: officeTask ?? null,
  });
  if (dir) localStorage.setItem(LAST_CWD_KEY, dir);
  const bucket = ensureBucket(id);
  activeId.value = id;
  connected.value = true;
  if (officeTask) rememberSessionBinding(id, officeTask);
  pushBucketSystem(bucket, `已开启任务会话(${id})${dir ? `，工作目录 ${dir}` : ""}。`);
  void refreshModelState(id);
  // 产物面板跟着新会话的绑定走(无绑定保持现状)
  if (officeTask) void syncWorkspaceForSession(id);
}

/** 拉取会话模型状态入桶(session/new|load 应答由桥缓存,事件与查询双通道
 * 以最后到达者为准)。失败静默:旧内核无此面时状态栏走降级显示。 */
export async function refreshModelState(id: string): Promise<void> {
  const bucket = bucketOf(id);
  if (!bucket) return;
  try {
    const st = parseModelState(await invoke("session_models", { sessionId: id }));
    if (st) bucket.modelState = st;
  } catch {
    /* 桥未就绪/旧内核:保留现状 */
  }
}

/** 会话内热切换模型(不重启内核,后续回合生效)。结果写回本桶并系统提示。 */
export async function setModel(modelId: string): Promise<void> {
  const bucket = activeBucket();
  if (!bucket) return;
  try {
    const st = parseModelState(
      await invoke("session_set_model", { sessionId: bucket.id, modelId }),
    );
    if (st) bucket.modelState = st;
    const name = st?.availableModels.find((m) => m.modelId === modelId)?.name || modelId;
    pushBucketSystem(bucket, `模型已切换为「${name}」,本会话后续回合生效。`);
  } catch (e) {
    pushBucketSystem(bucket, `模型切换失败:${String(e)}`);
  }
}

/** 发送一条任务;无会话或已断线时自动开启(断线重发=重连,原契约);
 * 回合进行中进队列(qidicode 语义)。 */
export async function sendTask(text: string): Promise<void> {
  const trimmed = text.trim();
  if (!trimmed) return;
  if (!connected.value || !activeBucket()) {
    try {
      await startSession();
    } catch (e) {
      pushSystem(`连接内核失败:${String(e)}`);
      return;
    }
  }
  const bucket = activeBucket();
  if (!bucket) return;
  if (bucket.busy) {
    bucket.queue.push({ id: ++messageId, text: trimmed });
    return;
  }
  await doSend(bucket, trimmed);
}

export async function cancelTurn(): Promise<void> {
  const bucket = activeBucket();
  if (!bucket) return;
  await invoke("session_cancel", { sessionId: bucket.id });
  permissionQueue.value = permissionQueue.value.filter((p) => p.sessionId !== bucket.id);
}

/** 「立即插入」(两次 Enter 的第二次):取消当前回合,队列立即续跑。
 * 当前空闲时等价于直接发出队首。 */
export function flushQueueNow(): void {
  const bucket = activeBucket();
  if (!bucket || !bucket.queue.length) return;
  if (bucket.busy) {
    bucket.flushing = true;
    void invoke("session_cancel", { sessionId: bucket.id }).catch(() => {
      bucket.flushing = false;
    });
  } else {
    drain(bucket);
  }
}

/** 撤回一条排队消息。 */
export function removeQueued(id: number): void {
  const bucket = activeBucket();
  if (!bucket) return;
  bucket.queue = bucket.queue.filter((q) => q.id !== id);
}

/** 切换到指定会话(仅限已在本进程建有桶的会话,即「任务会话」)。
 * 产物面板同步跟着切(按登记簿绑定;无绑定保持现状)。 */
export function switchSession(id: string): void {
  if (buckets.value[id]) {
    activeId.value = id;
    void syncWorkspaceForSession(id);
  }
}

export function resolvePermission(optionId: string): void {
  const p = permissionQueue.value.shift();
  if (!p) return;
  void invoke("permission_respond", { requestId: p.requestId, optionId });
}

export function cancelPermission(): void {
  const p = permissionQueue.value.shift();
  if (!p) return;
  void invoke("permission_cancel", { requestId: p.requestId });
}

/** 崩溃恢复(状态栏按钮)。返回恢复的会话数;-1 表示已在恢复中
 * (防重入:状态栏连点或自动连接与手动恢复并发会连环换桥丢会话,k3 审计)。 */
let recoverInFlight = false;
export async function recoverAgent(): Promise<number> {
  if (recoverInFlight) return -1;
  recoverInFlight = true;
  try {
    const n = await invoke<number>("agent_recover");
    if (n > 0) connected.value = true;
    return n;
  } finally {
    recoverInFlight = false;
  }
}

/** 启动自动连接(P2:免手动激活内核):登记簿里有会话才拉起内核并续接。 */
export async function autoConnect(): Promise<void> {
  try {
    const n = await invoke<number>("sessions_count");
    if (n > 0) await recoverAgent();
  } catch (e) {
    pushSystem(`自动连接内核失败:${String(e)}`);
  }
}

export function useAgentState() {
  return {
    messages: computed(() => activeBucket()?.messages ?? []),
    connected: computed(() => connected.value),
    sessionId: computed(() => activeId.value),
    turnInProgress: computed(() => activeBucket()?.busy ?? false),
    permission: computed(() => permissionQueue.value[0] ?? null),
    lastError,
    /** 活动会话的排队消息(回合进行中提交,待自动续跑)。 */
    queued: computed<QueuedMessage[]>(() => activeBucket()?.queue ?? []),
    /** 活动会话累计用量(token/成本/模型)。 */
    sessionUsage: computed<SessionUsage>(() => activeBucket()?.usage ?? emptySessionUsage()),
    /** 活动会话的模型状态(内核应答;null=尚未拿到)。 */
    modelState: computed<ModelState | null>(() => activeBucket()?.modelState ?? null),
    /** 活动会话的历史回放状态(未回放为 null)。 */
    replay: computed<ReplayState | null>(() => activeBucket()?.replay ?? null),
    /** 全部活动会话(侧栏「任务会话」;不含 disconnected 兜底空桶)。 */
    sessionList: computed<SessionListItem[]>(() =>
      Object.values(buckets.value)
        .filter((b) => b.id !== "")
        .map((b) => ({ id: b.id, busy: b.busy, queued: b.queue.length, title: b.title })),
    ),
  };
}
