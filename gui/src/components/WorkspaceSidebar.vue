<script setup lang="ts">
import { ref } from "vue";

// 左区:任务工作区列表 + 常用技能入口。
// M1 为静态骨架;工作区数据与技能发现分别在 P1 接入(读 ~/.qidi/office-workspaces 与 SKILL.md)。
interface WorkspaceItem {
  id: string;
  name: string;
  active: boolean;
}

// 占位数据:真实列表由 M2 内核接入后的 P1 模块替换
const workspaces = ref<WorkspaceItem[]>([
  { id: "default", name: "default", active: true },
]);
const skills = ["标书编写", "周报汇总", "文档转换", "图纸生成"];
</script>

<template>
  <aside class="sidebar">
    <div class="section">
      <div class="section-row">
        <span class="section-title">任务工作区</span>
      </div>
      <button class="new-task" disabled title="内核接入后可用(P0/M2)">+ 新建任务</button>
      <ul class="ws-list">
        <li
          v-for="ws in workspaces"
          :key="ws.id"
          class="ws-item"
          :class="{ active: ws.active }"
        >
          <span class="ws-dot" :class="{ on: ws.active }"></span>
          <span class="ws-name">{{ ws.name }}</span>
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
          title="内核接入后可用(P0/M2)"
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
