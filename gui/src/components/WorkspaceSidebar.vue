<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { deleteSession, pushSystem, rememberSessionBinding, rememberSessionCwd, sendTask, startSession, switchSession, useAgentState, type SessionListItem } from "../composables/useAgent";
import { deleteWorkspace, initOffice, switchTask, useOfficeState } from "../composables/useOffice";
import { initSkills, installSkillsPackage, isOfficeSkill, useSkills, type SkillInfo } from "../composables/useSkills";
import { closeAllPreviews } from "../composables/usePreview";
import { open as pickDirectory } from "@tauri-apps/plugin-dialog";
import { sidebarCollapsed } from "../composables/useUiLayout";

/** 新建任务。withDir=true 先弹系统目录选择器(取消则不动作)。
 * 内核未连接也可点:session_start 自己会拉起内核(旧版 !connected
 * 置灰是新手"新建任务永远点不动"的根因,勿再加回)。 */
async function newSession(withDir = false) {
  let cwd: string | undefined;
  if (withDir) {
    const dir = await pickDirectory({ directory: true, title: "选择项目文件夹" });
    if (typeof dir !== "string") return;
    cwd = dir;
  }
  try {
    await startSession(cwd);
    // 新会话落盘后立即可见(运行中开第二个会话不彼此阻塞)
    await loadHistory();
  } catch (e) {
    pushSystem(`开启会话失败:${String(e)}`);
  }
}

const { sessionId, sessionList } = useAgentState();

/** 本进程已建有桶的会话(实时流在手):点击直接切换,不走重恢复。 */
function liveOf(id: string): SessionListItem | undefined {
  return sessionList.value.find((s) => s.id === id);
}

const { workspaces, currentTask } = useOfficeState();
const { skills } = useSkills();

// 方案 v2 P1:office-* / bid-* 是办公用户的常用技能面,其余技能
// (开发向)不进左区,免得淹没办公用户。
const officeSkills = computed(() => skills.value.filter((s) => isOfficeSkill(s.name)));

// 点技能 = 以该技能的名义发一个任务(sendTask 无会话时自动开启)。
// 任务文本经 ACP 下发,技能的权限审批仍在内核侧走 RequestPermission。
async function runSkill(skill: SkillInfo): Promise<void> {
  if (!currentTask.value) {
    pushSystem("请先选择任务工作区再触发技能");
    return;
  }
  try {
    await sendTask(`请使用 ${skill.name} 技能,处理任务工作区「${currentTask.value}」`);
  } catch (e) {
    pushSystem(`技能任务下发失败:${String(e)}`);
  }
}

// R8:技能目录为空时的一键安装(官方技能包)。Rust 侧下载 + 解压 +
// 重拉列表;成功后 skill-grid 出现,空态自动消失。
const installing = ref(false);
const installError = ref("");

async function installSkills(): Promise<void> {
  if (installing.value) return;
  installing.value = true;
  installError.value = "";
  try {
    const report = await installSkillsPackage();
    pushSystem(`官方技能包安装完成:新增 ${report.installed} 个技能(${report.path})`);
    if (!report.installed) installError.value = "技能目录中已存在同名技能,未覆盖";
  } catch (e) {
    installError.value = String(e);
    pushSystem(`技能包安装失败:${String(e)}`);
  } finally {
    installing.value = false;
  }
}

// 历史会话(登记簿):展示 + 点击续接(agent 侧上下文经 session/load
// 续上;界面消息不回放,历史内容仍在会话记录里)。
interface HistoryEntry {
  session_id: string;
  cwd: string;
  title: string | null;
  /** 绑定的办公任务工作区名(登记簿 task;null = 未绑定)。 */
  task: string | null;
}
const history = ref<HistoryEntry[]>([]);
const resuming = ref(false);
/** 恢复反馈:常驻错误条(点击任意历史条目或 × 可清)+ 正在恢复的条目 id。 */
const resumeError = ref<string | null>(null);
const resumingSessionId = ref("");
/** 超时看门狗句柄与点击时捕获的标题(session_restored 负载不带 title)。 */
let resumeWatchdog = 0;
let resumePendingTitle = "";

/** session_restored 事件收尾:清看门狗、解除恢复态并给成功确认提示。
 * 非本次恢复目标的事件不收尾(多会话并发时不误清)。 */
function finishResume(sid: string): void {
  if (!resuming.value || sid !== resumingSessionId.value) return;
  window.clearTimeout(resumeWatchdog);
  resuming.value = false;
  resumingSessionId.value = "";
  pushSystem(`已恢复会话「${resumePendingTitle}」`);
}
const confirmingClear = ref(false);
let clearTimer = 0;

function historyLabel(h: HistoryEntry): string {
  if (h.title) return h.title;
  const task = h.cwd.split(/[\\/]/).pop() || h.cwd;
  return `${task} · ${h.session_id.slice(0, 8)}`;
}

/** 会话摘要标题缓存:标题很少变,避免 turn_completed/session_restored 反复拉取。
 * 只缓存非空标题(空标题即未命中,下次重试,新会话 summary 落盘后能升级)。 */
const summaryCache = new Map<string, string>();

/** 并行拉取前 50 条历史的真实标题(session_summary),返回 id→title(仅非空)。 */
async function fetchTitles(entries: HistoryEntry[]): Promise<Map<string, string>> {
  const out = new Map<string, string>();
  await Promise.all(
    entries.map(async (h) => {
      const cached = summaryCache.get(h.session_id);
      if (cached) {
        out.set(h.session_id, cached);
        return;
      }
      try {
        const s = await invoke<{ title: string | null }>("session_summary", {
          sessionId: h.session_id,
          cwd: h.cwd,
        });
        const title = s?.title ?? "";
        if (title) {
          summaryCache.set(h.session_id, title);
          out.set(h.session_id, title);
        }
      } catch {
        /* 摘要缺失:保留登记簿标题(historyLabel 回退 cwd 目录名) */
      }
    }),
  );
  return out;
}

async function loadHistory(): Promise<void> {
  try {
    const list = await invoke<HistoryEntry[]>("sessions_history");
    history.value = list;
    // 标题升级:session_summary 优先(前 50 条;超出不拉,避免刷屏)
    const titles = await fetchTitles(list.slice(0, 50));
    if (titles.size) {
      history.value = list.map((h) =>
        titles.has(h.session_id) ? { ...h, title: titles.get(h.session_id)! } : h,
      );
    }
  } catch (e) {
    pushSystem(`历史会话读取失败:${String(e)}`);
  }
}

/** 清空历史会话(两步确认;登记簿自动归档备份,agent 侧内容不动)。 */
async function clearHistory(): Promise<void> {
  if (!confirmingClear.value) {
    confirmingClear.value = true;
    window.clearTimeout(clearTimer);
    clearTimer = window.setTimeout(() => (confirmingClear.value = false), 3000);
    return;
  }
  confirmingClear.value = false;
  window.clearTimeout(clearTimer);
  try {
    pushSystem(await invoke<string>("sessions_clear", { archive: true }));
    await loadHistory();
  } catch (e) {
    pushSystem(`清空失败:${String(e)}`);
  }
}

async function resumeSession(h: HistoryEntry): Promise<void> {
  // 任意历史条目被点击即清除上一轮恢复失败的常驻错误条
  resumeError.value = null;
  if (h.session_id === sessionId.value || resuming.value) return;
  // 活动会话(已有实时转录):直接切换视图,不重恢复、不丢流
  const live = liveOf(h.session_id);
  if (live) {
    switchSession(h.session_id);
    return;
  }
  resuming.value = true;
  resumingSessionId.value = h.session_id;
  // 点击时捕获标题:session_restored 事件负载不一定带 title
  resumePendingTitle = h.title ?? historyLabel(h);
  // 超时看门狗:15 秒未收到 session_restored 也未 reject 则兜底收尾
  window.clearTimeout(resumeWatchdog);
  resumeWatchdog = window.setTimeout(() => {
    if (!resuming.value) return;
    resuming.value = false;
    resumingSessionId.value = "";
    resumeError.value = "恢复超时，请重试";
  }, 15000);
  try {
    // 先登记 cwd 与绑定:回放(session_restored 事件)定位内核转录文件、
    // 同步产物面板时复用,免去额外拉取
    rememberSessionCwd(h.session_id, h.cwd);
    rememberSessionBinding(h.session_id, h.task);
    await invoke("session_resume", { sessionId: h.session_id });
    // resolve 不做任何状态变更:等 session_restored 事件统一收尾
  } catch (e) {
    // 失败收尾:错误条保留后端原始文案,直到下次点击或手动 ×
    window.clearTimeout(resumeWatchdog);
    resuming.value = false;
    resumingSessionId.value = "";
    resumeError.value = String(e);
  }
}

// 工作区删除(两步确认;二次点击才执行,3 秒不点自动复位)。
const deletingName = ref("");
let deleteTimer = 0;

function askDelete(name: string): void {
  if (deletingName.value === name) {
    deletingName.value = "";
    void doDelete(name);
    return;
  }
  deletingName.value = name;
  window.clearTimeout(deleteTimer);
  deleteTimer = window.setTimeout(() => (deletingName.value = ""), 3000);
}

async function doDelete(name: string): Promise<void> {
  if (name === currentTask.value) {
    pushSystem("请先切换到其他工作区,再删除当前工作区");
    return;
  }
  try {
    await deleteWorkspace(name);
    pushSystem(`已删除工作区「${name}」及其目录内全部产物`);
  } catch (e) {
    pushSystem(`删除失败:${String(e)}`);
  }
}

// 历史会话删除(两步确认;二次点击才执行,3 秒不点自动复位)。
const deletingSessionId = ref("");
let deleteSessionTimer = 0;

function askDeleteSession(id: string): void {
  if (deletingSessionId.value === id) {
    deletingSessionId.value = "";
    window.clearTimeout(deleteSessionTimer);
    void doDeleteSession(id);
    return;
  }
  deletingSessionId.value = id;
  window.clearTimeout(deleteSessionTimer);
  deleteSessionTimer = window.setTimeout(() => (deletingSessionId.value = ""), 3000);
}

async function doDeleteSession(id: string): Promise<void> {
  if (id === sessionId.value) {
    pushSystem("请先切换到其他会话，再删除当前会话");
    return;
  }
  try {
    // 后端返回文案(已删除会话「xxxxxxxx」)原样提示;随后刷新登记簿
    pushSystem(await deleteSession(id));
    await loadHistory();
  } catch (e) {
    pushSystem(`删除失败:${String(e)}`);
  }
}

onMounted(async () => {
  await initOffice();
  try {
    await initSkills();
  } catch (e) {
    pushSystem(`技能列表加载失败:${String(e)}`);
  }
  await loadHistory();
});

// 回合结束/会话恢复后刷新历史(新标题、新条目可见;k3 审计)
void listen("acp-event", (e) => {
  const t = (e.payload as { type?: string; session_id?: string }).type;
  if (t === "turn_completed" || t === "session_restored") {
    if (t === "session_restored") {
      const sid = (e.payload as { session_id?: string }).session_id ?? "";
      finishResume(sid);
    }
    void loadHistory();
  }
});

// 左区:任务工作区列表(真实数据)+ 常用技能入口(真实技能,office-*/bid-*)。
function pick(name: string): void {
  if (name !== currentTask.value) {
    // 旧任务的产物预览 Tab 不随任务切换残留(step 审计 N6)
    closeAllPreviews();
    void switchTask(name);
  }
}
</script>

<template>
  <!-- 折叠态图标条:展开 / 新建 / 选目录 -->
  <aside v-if="sidebarCollapsed" class="rail">
    <button class="rail-btn" title="展开侧栏" @click="sidebarCollapsed = false">☰</button>
    <button class="rail-btn" title="新建任务：无需选目录，开箱即用" @click="newSession()">＋</button>
    <button class="rail-btn" title="选择项目文件夹后新建（需要指定目录时用这个）" @click="newSession(true)">📁</button>
  </aside>

  <aside v-else class="sidebar">
    <div class="section">
      <div class="section-row">
        <span class="section-title">任务工作区</span>
        <button class="rail-btn" title="折叠侧栏" @click="sidebarCollapsed = true">⇆</button>
      </div>
      <div class="new-task-row">
        <button class="new-task" title="新建任务：无需选目录，开箱即用（已选过目录则复用上次）" @click="newSession()">
          + 新建任务
        </button>
        <button class="new-task dir" title="选择项目文件夹后新建（需要指定目录时用这个）" @click="newSession(true)">
          📁
        </button>
      </div>
      <ul class="ws-list">
        <li
          v-for="ws in workspaces"
          :key="ws.name"
          class="ws-item"
          :class="{ active: ws.name === currentTask }"
          :title="ws.name === currentTask ? '当前工作区(先切换再删除)' : ws.name"
          @click="pick(ws.name)"
        >
          <span class="ws-dot" :class="{ on: ws.name === currentTask }"></span>
          <span class="ws-name">{{ ws.name }}</span>
          <span
            v-if="deletingName === ws.name"
            class="ws-del confirm"
            @click.stop="askDelete(ws.name)"
          >确认删除?</span>
          <span v-else class="ws-count">{{ ws.artifact_count }}</span>
          <button
            class="ws-del"
            title="删除工作区及目录内全部产物"
            @click.stop="askDelete(ws.name)"
          >✕</button>
        </li>
      </ul>
    </div>

    <div class="section">
      <span class="section-title">常用技能</span>
      <div v-if="officeSkills.length" class="skill-grid">
        <button
          v-for="skill in officeSkills"
          :key="skill.name"
          class="skill-chip"
          :title="skill.description || skill.name"
          @click="runSkill(skill)"
        >
          {{ skill.display_name }}
        </button>
      </div>
      <div v-else class="skill-empty">
        <p class="skill-empty-hint">未发现技能(读取 ~/.qidi/skills)</p>
        <button
          class="skill-install"
          :disabled="installing"
          title="从官网下载官方技能包并解压到 ~/.qidi/skills(已存在的同名技能不会被覆盖)"
          @click="installSkills"
        >
          {{ installing ? "⏳ 正在安装..." : "📦 一键安装官方技能包（30 个）" }}
        </button>
        <p v-if="installError" class="skill-install-error">{{ installError }}</p>
      </div>
    </div>

    <div v-if="history.length || confirmingClear" class="section">
      <div class="section-row">
        <span class="section-title">历史会话</span>
        <button
          class="history-clear"
          :class="{ confirm: confirmingClear }"
          :title="confirmingClear ? '再次点击执行(登记簿先归档备份)' : '清空历史会话列表(自动归档备份)'"
          @click="clearHistory"
        >
          {{ confirmingClear ? "确认清空?" : "清空" }}
        </button>
      </div>
      <div v-if="resumeError" class="resume-error">
        <span class="resume-error-text">{{ resumeError }}</span>
        <button class="resume-error-close" title="关闭" @click="resumeError = null">×</button>
      </div>
      <ul class="ws-list">
        <li
          v-for="h in history"
          :key="h.session_id"
          class="ws-item"
          :class="{
            active: h.session_id === sessionId,
            resuming: resumingSessionId === h.session_id,
          }"
          :title="
            h.session_id === sessionId
              ? '当前会话'
              : liveOf(h.session_id)
                ? '点击切换(会话进行中,实时转录已保留)'
                : '点击续接(agent 上下文恢复)'
          "
          @click="resumeSession(h)"
        >
          <span
            class="ws-dot"
            :class="{ on: h.session_id === sessionId, run: liveOf(h.session_id)?.busy }"
          ></span>
          <span class="ws-name">{{ historyLabel(h) }}</span>
          <span v-if="h.task" class="ws-task" :title="`绑定工作区:${h.task}`">{{
            h.task
          }}</span>
          <span v-if="resumingSessionId === h.session_id" class="ws-run">恢复中…</span>
          <span v-else-if="liveOf(h.session_id)?.busy" class="ws-run">运行中</span>
          <span v-else-if="liveOf(h.session_id)?.queued" class="ws-count">{{
            liveOf(h.session_id)!.queued
          }}</span>
          <span
            v-if="deletingSessionId === h.session_id"
            class="ws-del confirm"
            @click.stop="askDeleteSession(h.session_id)"
          >确认删除?</span>
          <button
            v-else
            class="ws-del"
            title="删除本条历史会话(登记簿先归档备份)"
            @click.stop="askDeleteSession(h.session_id)"
          >✕</button>
        </li>
      </ul>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  background: var(--bg-panel);
  border-right: 1px solid var(--border);
  padding: 12px 10px;
  display: flex;
  flex-direction: column;
  gap: 20px;
  overflow-y: auto;
}

.section {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.section-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.history-clear {
  background: none;
  border: none;
  color: var(--text-disabled);
  font-size: var(--font-size-sm);
  cursor: pointer;
  padding: 0;
}

.history-clear:hover {
  color: var(--text-primary);
}

.history-clear.confirm {
  color: #fff;
  background: var(--danger, #c0392b);
  border-radius: var(--radius);
  padding: 2px 8px;
}

.section-title {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  font-weight: 600;
  letter-spacing: 0.05em;
}

.new-task-row {
  display: flex;
  gap: 6px;
}

.new-task {
  flex: 1;
  border: 1px dashed var(--border);
  background: transparent;
  border-radius: var(--radius);
  padding: 8px;
  color: var(--text-secondary);
  cursor: pointer;
}

.new-task:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.new-task.dir {
  flex: none;
  padding: 8px 10px;
}

/* 折叠后的左栏图标条 */
.rail {
  background: var(--bg-panel);
  border-right: 1px solid var(--border);
  padding: 10px 6px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.rail-btn {
  border: none;
  background: none;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 15px;
  padding: 4px 6px;
  border-radius: var(--radius);
}

.rail-btn:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.ws-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.ws-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  border-radius: var(--radius);
  cursor: pointer;
}

.ws-item:hover {
  background: var(--bg-hover);
}

.ws-item.active {
  background: var(--bg-active);
}

.ws-del {
  margin-left: auto;
  flex: none;
  border: none;
  background: none;
  color: var(--text-disabled);
  cursor: pointer;
  font-size: 12px;
  padding: 0 2px;
  opacity: 0;
}

.ws-item:hover .ws-del {
  opacity: 1;
}

.ws-del:hover {
  color: var(--danger, #c0392b);
}

.ws-del.confirm {
  color: #fff;
  background: var(--danger, #c0392b);
  border-radius: var(--radius);
  padding: 2px 8px;
  white-space: nowrap;
}

.ws-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-disabled);
  flex: none;
}

.ws-dot.on {
  background: var(--success);
}

/* 会话运行中:呼吸点 + 右侧标签(多会话并跑时的后台活动可见) */
.ws-dot.run {
  background: var(--accent);
  animation: run-pulse 1.2s ease-in-out infinite;
}

@keyframes run-pulse {
  50% {
    opacity: 0.35;
  }
}

.ws-run {
  margin-left: auto;
  flex: none;
  font-size: var(--font-size-sm);
  color: var(--accent);
}

.ws-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 历史会话条目的绑定工作区小标签(克制:小号弱化,不与主标题抢注意力) */
.ws-task {
  flex: none;
  max-width: 96px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--font-size-sm);
  color: var(--text-disabled);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 1px 6px;
}

/* 恢复反馈:历史会话列表上方的常驻错误条(下次点击或 × 才消失) */
.resume-error {
  display: flex;
  align-items: center;
  gap: 6px;
  border: 1px solid var(--danger, #c0392b);
  border-radius: var(--radius);
  padding: 5px 8px;
  font-size: var(--font-size-sm);
}

.resume-error-text {
  flex: 1;
  color: var(--danger, #c0392b);
  word-break: break-all;
}

.resume-error-close {
  flex: none;
  border: none;
  background: none;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  cursor: pointer;
  padding: 0 2px;
}

.resume-error-close:hover {
  color: var(--danger, #c0392b);
}

/* 恢复进行中的历史条目:置灰禁点 + 呼吸点,替代一闪而过的临时消息 */
.ws-item.resuming {
  opacity: 0.6;
  cursor: wait;
  pointer-events: none;
}

.skill-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
}

.skill-chip {
  border: 1px solid var(--border);
  background: var(--bg-base);
  border-radius: var(--radius);
  padding: 8px 4px;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  cursor: pointer;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.skill-chip:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.skill-chip:disabled {
  cursor: not-allowed;
  opacity: 0.6;
}

.skill-empty {
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.skill-empty-hint {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--text-disabled);
}

/* R8 一键安装:沿用 skill-chip / new-task 的描边按钮惯例 */
.skill-install {
  border: 1px solid var(--border);
  background: var(--bg-base);
  border-radius: var(--radius);
  padding: 8px 6px;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  cursor: pointer;
}

.skill-install:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.skill-install:disabled {
  cursor: not-allowed;
  opacity: 0.6;
}

.skill-install-error {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--danger, #c0392b);
  word-break: break-all;
}
</style>
