<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { pushSystem, sendTask, startSession, useAgentState } from "../composables/useAgent";
import { deleteWorkspace, initOffice, switchTask, useOfficeState } from "../composables/useOffice";
import { initSkills, isOfficeSkill, useSkills, type SkillInfo } from "../composables/useSkills";
import { closeAllPreviews } from "../composables/usePreview";

async function newSession() {
  try {
    await startSession();
  } catch (e) {
    pushSystem(`开启会话失败:${String(e)}`);
  }
}

const { connected, turnInProgress, sessionId } = useAgentState();

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

// 历史会话(登记簿):展示 + 点击续接(agent 侧上下文经 session/load
// 续上;界面消息不回放,历史内容仍在会话记录里)。
interface HistoryEntry {
  session_id: string;
  cwd: string;
  title: string | null;
}
const history = ref<HistoryEntry[]>([]);
const resuming = ref(false);
const confirmingClear = ref(false);
let clearTimer = 0;

function historyLabel(h: HistoryEntry): string {
  if (h.title) return h.title;
  const task = h.cwd.split(/[\\/]/).pop() || h.cwd;
  return `${task} · ${h.session_id.slice(0, 8)}`;
}

async function loadHistory(): Promise<void> {
  try {
    history.value = await invoke<HistoryEntry[]>("sessions_history");
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
  if (h.session_id === sessionId.value || turnInProgress.value || resuming.value) return;
  resuming.value = true;
  try {
    await invoke("session_resume", { sessionId: h.session_id });
  } catch (e) {
    pushSystem(`恢复会话失败:${String(e)}`);
  } finally {
    resuming.value = false;
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
  const t = (e.payload as { type?: string }).type;
  if (t === "turn_completed" || t === "session_restored") void loadHistory();
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
  <aside class="sidebar">
    <div class="section">
      <div class="section-row">
        <span class="section-title">任务工作区</span>
      </div>
      <button
        class="new-task"
        :disabled="!connected"
        :title="connected ? '开启新会话' : '内核未连接,发送任务后自动开启'"
        @click="newSession"
      >
        + 新建任务
      </button>
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
          :disabled="turnInProgress"
          :title="skill.description || skill.name"
          @click="runSkill(skill)"
        >
          {{ skill.display_name }}
        </button>
      </div>
      <p v-else class="skill-empty">未发现技能(读取 ~/.qidi/skills)</p>
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
      <ul class="ws-list">
        <li
          v-for="h in history"
          :key="h.session_id"
          class="ws-item"
          :class="{ active: h.session_id === sessionId }"
          :title="h.session_id === sessionId ? '当前会话' : '点击续接(agent 上下文恢复)'"
          @click="resumeSession(h)"
        >
          <span class="ws-dot" :class="{ on: h.session_id === sessionId }"></span>
          <span class="ws-name">{{ historyLabel(h) }}</span>
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

.new-task {
  border: 1px dashed var(--border);
  background: transparent;
  border-radius: var(--radius);
  padding: 8px;
  color: var(--text-secondary);
  cursor: not-allowed;
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

.ws-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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
  font-size: var(--font-size-sm);
  color: var(--text-disabled);
}
</style>
