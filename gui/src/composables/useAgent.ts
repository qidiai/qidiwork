// Agent/桥状态中心:M2 传输 + M3 ACP 桥的前端单向镜像。
// 事件契约来自 commands.rs:acp-event(BridgeEvent,serde tag=type)/ agent-exit。
// 不变量(k3 M4 审计):入站行走可靠 mpsc 单消费者队列,传输层不丢帧,
// 无需"断序/不完整"标记;若未来换 broadcast 需补 degraded 标记。
// 单活动会话假设:sessionId 过滤已存在,多会话分桶为未来债(方案登记)。
import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { initNotifyPermission, isWindowFocused, notify, notifyThrottled } from "../services/notify";

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
  toolCalls: ToolCallItem[];
  done: boolean;
}

export interface PermissionState {
  requestId: number;
  sessionId: string;
  title: string;
  options: { id: string; name: string; kind?: string }[];
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
}

const messages = ref<ChatMessage[]>([]);
const connected = ref(false);
const sessionId = ref<string>("");
const turnInProgress = ref(false);
const permission = ref<PermissionState | null>(null);
const lastError = ref("");
let messageId = 0;
let listenersBound = false;
/** 当前会话是否已记标题(首条 prompt 回写;恢复的会话保持 true 不覆盖) */
let titleRecorded = false;

function pushAssistantChunk(session: string, text: string) {
  // 只有一个活动会话(M3 单 session_start 契约);sessionId 不匹配时忽略
  if (session !== sessionId.value) return;
  const last = messages.value[messages.value.length - 1];
  if (last && last.role === "assistant" && !last.done) {
    last.content += text;
  } else {
    messages.value.push({
      id: ++messageId,
      role: "assistant",
      content: text,
      toolCalls: [],
      done: false,
    });
  }
}

function upsertToolCall(session: string, update: NonNullable<AcpEvent["update"]>) {
  if (session !== sessionId.value) return;
  const last = messages.value[messages.value.length - 1];
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
    case "session_update":
      if (ev.update?.sessionUpdate === "agent_message_chunk") {
        pushAssistantChunk(ev.session_id ?? "", ev.update.content?.text ?? "");
      } else if (ev.update?.sessionUpdate === "tool_call" || ev.update?.sessionUpdate === "tool_call_update") {
        upsertToolCall(ev.session_id ?? "", ev.update);
      }
      // plan 等其余更新类型 M4 暂不渲染
      break;
    case "permission_request":
      permission.value = {
        requestId: ev.request_id ?? 0,
        sessionId: ev.session_id ?? "",
        title: ev.tool_call?.title ?? "agent 请求权限",
        options: ev.options ?? [],
      };
      // 失焦时系统提醒,办公用户切走窗口也不错过审批(P2);同 title 节流
      if (!isWindowFocused()) {
        notifyThrottled(
          permission.value.title,
          "QIDI 办公工作台",
          `等待权限审批:${permission.value.title}`,
        );
      }
      break;
    case "turn_completed":
      turnInProgress.value = false;
      {
        const last = messages.value[messages.value.length - 1];
        if (last && last.role === "assistant") last.done = true;
        const reason = ev.stop_reason ?? "";
        if (reason.startsWith("error:")) {
          lastError.value = reason;
          if (!isWindowFocused()) {
            // 出错恰是最需要召回的场景(k3 审计),失焦必通知
            notify("QIDI 办公工作台", "任务出错,点击窗口查看详情。");
          }
        } else if (reason === "cancelled" || reason === "rejected") {
          // 用户主动取消/拒绝,无需召回自己
        } else if (!isWindowFocused()) {
          // 失焦时系统提醒任务完成(P2)
          notify("QIDI 办公工作台", "任务已完成,点击窗口查看结果。");
        }
      }
      break;
    case "disconnected":
      connected.value = false;
      turnInProgress.value = false;
      messages.value.push({
        id: ++messageId,
        role: "system",
        content: "内核已断开。点击状态栏「恢复」或重新发送任务以重连(会话将自动续接)。",
        toolCalls: [],
        done: true,
      });
      break;
    case "session_restored":
      if (ev.session_id) {
        sessionId.value = ev.session_id;
        connected.value = true;
        // 恢复的会话标题已在登记簿里,首条 prompt 不覆盖
        titleRecorded = true;
        // 消息不回放:清掉上一个会话的残留,避免混排误读(k3 审计)
        messages.value = [];
      }
      messages.value.push({
        id: ++messageId,
        role: "system",
        content: `会话 ${ev.session_id} 已恢复(历史上下文已在内核侧续接;本次界面从新消息开始)。`,
        toolCalls: [],
        done: true,
      });
      break;
    case "session_restore_failed":
      messages.value.push({
        id: ++messageId,
        role: "system",
        content: `会话 ${ev.session_id} 恢复失败:${ev.error ?? "未知原因"}`,
        toolCalls: [],
        done: true,
      });
      break;
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

/** 新建任务会话(用户点「+ 新建任务」或首次发送)。 */
export async function startSession(cwd?: string): Promise<void> {
  sessionId.value = await invoke<string>("session_start", { cwd: cwd ?? null });
  connected.value = true;
  titleRecorded = false; // 新会话:首条 prompt 记为标题
  messages.value.push({
    id: ++messageId,
    role: "system",
    content: `已开启任务会话(${sessionId.value})。`,
    toolCalls: [],
    done: true,
  });
}

/** 发送一条任务;无会话时自动开启。 */
export function pushSystem(text: string): void {
  messages.value.push({
    id: ++messageId,
    role: "system",
    content: text,
    toolCalls: [],
    done: true,
  });
}

export async function sendTask(text: string): Promise<void> {
  const trimmed = text.trim();
  if (!trimmed || turnInProgress.value) return;
  try {
    if (!connected.value || !sessionId.value) {
      await startSession();
    }
  } catch (e) {
    pushSystem(`连接内核失败:${String(e)}`);
    return;
  }
  messages.value.push({
    id: ++messageId,
    role: "user",
    content: trimmed,
    toolCalls: [],
    done: true,
  });
  turnInProgress.value = true;
  try {
    await invoke("session_prompt", { sessionId: sessionId.value, text: trimmed });
    // 首条 prompt 截断记为会话标题(失败不影响任务下发)
    if (!titleRecorded) {
      titleRecorded = true;
      // 标题按字符截断,避免 emoji 代理对被切半(k3 审计次要项)
      void invoke("session_set_title", {
        sessionId: sessionId.value,
        title: Array.from(trimmed).slice(0, 40).join(""),
      }).catch(() => {});
    }
  } catch (e) {
    turnInProgress.value = false;
    pushSystem(`任务下发失败:${String(e)}`);
  }
}

export async function cancelTurn(): Promise<void> {
  if (!sessionId.value) return;
  await invoke("session_cancel", { sessionId: sessionId.value });
}

export function resolvePermission(optionId: string): void {
  const p = permission.value;
  if (!p) return;
  permission.value = null;
  void invoke("permission_respond", { requestId: p.requestId, optionId });
}

export function cancelPermission(): void {
  const p = permission.value;
  if (!p) return;
  permission.value = null;
  void invoke("permission_cancel", { requestId: p.requestId });
}

/** 崩溃恢复(状态栏按钮)。返回恢复的会话数。 */
export async function recoverAgent(): Promise<number> {
  const n = await invoke<number>("agent_recover");
  if (n > 0) connected.value = true;
  return n;
}

export function useAgentState() {
  return {
    messages,
    connected: computed(() => connected.value),
    sessionId: computed(() => sessionId.value),
    turnInProgress: computed(() => turnInProgress.value),
    permission,
    lastError,
  };
}
