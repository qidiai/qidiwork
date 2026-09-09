<script setup lang="ts">
import { onMounted } from "vue";
import { startSession, pushSystem, useAgentState } from "../composables/useAgent";
import { initOffice, switchTask, useOfficeState } from "../composables/useOffice";

async function newSession() {
  try {
    await startSession();
  } catch (e) {
    pushSystem(`开启会话失败:${String(e)}`);
  }
}

const { connected } = useAgentState();

const { workspaces, currentTask } = useOfficeState();

onMounted(async () => {
  await initOffice();
});

// 左区:任务工作区列表(真实数据)+ 常用技能入口(技能面板 P1b 接入)。
const skills = ["标书编写", "周报汇总", "文档转换", "图纸生成"];

function pick(name: string): void {
  if (name !== currentTask.value) void switchTask(name);
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
          @click="pick(ws.name)"
        >
          <span class="ws-dot" :class="{ on: ws.name === currentTask }"></span>
          <span class="ws-name">{{ ws.name }}</span>
          <span class="ws-count">{{ ws.artifact_count }}</span>
        </li>
      </ul>
    </div>

    <div class="section">
      <span class="section-title">常用技能</span>
      <div class="skill-grid">
        <button
          v-for="skill in skills"
          :key="skill"
          class="skill-chip"
          disabled
          title="技能面板在 P1 接入(读取 ~/.qidi/skills)"
        >
          {{ skill }}
        </button>
      </div>
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
  cursor: not-allowed;
}
</style>
