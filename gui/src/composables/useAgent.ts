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
        // 消息不回放:恢复场景界面从新消息开始(历史上下文在内核侧续接)
        pushBucketSystem(
          bucket,
          `会话 ${id} 已恢复(历史上下文已在内核侧续接;本次界面从新消息开始)。`,
        );
      }
      // 重连换桥后模型缓存是新的:补查一次,状态栏不空窗
      void refreshModelState(id);
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

/** 新建任务会话(用户点「+ 新建任务」或首次发送)。回合进行中也可开
 * 新会话:各会话桶独立,后台会话继续跑。未显式传 cwd 时复用上次选择的
 * 目录;都没选过则交给内核默认(用户主目录)。 */
export async function startSession(cwd?: string): Promise<void> {
  const dir = cwd ?? localStorage.getItem(LAST_CWD_KEY) ?? undefined;
  const id = await invoke<string>("session_start", { cwd: dir ?? null });
  if (dir) localStorage.setItem(LAST_CWD_KEY, dir);
  const bucket = ensureBucket(id);
  activeId.value = id;
  connected.value = true;
  pushBucketSystem(bucket, `已开启任务会话(${id})${dir ? `，工作目录 ${dir}` : ""}。`);
  void refreshModelState(id);
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

/** 切换到指定会话(仅限已在本进程建有桶的会话,即「任务会话」)。 */
export function switchSession(id: string): void {
  if (buckets.value[id]) activeId.value = id;
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
    /** 全部活动会话(侧栏「任务会话」;不含 disconnected 兜底空桶)。 */
    sessionList: computed<SessionListItem[]>(() =>
      Object.values(buckets.value)
        .filter((b) => b.id !== "")
        .map((b) => ({ id: b.id, busy: b.busy, queued: b.queue.length, title: b.title })),
    ),
  };
}
