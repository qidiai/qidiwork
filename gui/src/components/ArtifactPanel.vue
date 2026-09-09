<script setup lang="ts">
// 右区:产物面板。数据来自 office-artifact 技能登记的 manifest
// (~/.qidi/office-workspaces/<task>/manifest.json),office-event 实时刷新。
// 「打开」= 系统默认程序(WPS 等);「预览」Tab 在 P1b 接入。
import { initOffice, openArtifact, useOfficeState, type ArtifactCard } from "../composables/useOffice";
import { pushSystem } from "../composables/useAgent";

async function open(card: ArtifactCard): Promise<void> {
  try {
    await openArtifact(card);
  } catch (e) {
    pushSystem(`打开失败:${String(e)}`);
  }
}

const { currentTask, artifacts } = useOfficeState();

void initOffice();

function sizeText(n: number): string {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${(n / (1024 * 1024)).toFixed(1)}MB`;
}

function timeText(epoch: number): string {
  if (!epoch) return "";
  const d = new Date(epoch * 1000);
  const pad = (v: number) => String(v).padStart(2, "0");
  return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function iconFor(name: string): string {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (["docx", "doc"].includes(ext)) return "📄";
  if (["xlsx", "xls"].includes(ext)) return "📊";
  if (["pptx", "ppt"].includes(ext)) return "📽";
  if (ext === "pdf") return "📕";
  if (["png", "jpg", "jpeg"].includes(ext)) return "🖼";
  if (["html", "md", "txt"].includes(ext)) return "📝";
  return "📎";
}
</script>

<template>
  <aside class="artifact-panel">
    <div class="panel-title">产物面板</div>
    <div class="panel-body">
      <p v-if="!currentTask" class="empty-hint">
        任务产出的交付文件会出现在这里。
      </p>
      <template v-else>
        <p class="task-line">任务:{{ currentTask }} · {{ artifacts.length }} 项</p>
        <ul class="card-list">
          <li v-for="card in artifacts" :key="card.path" class="card">
            <div class="card-head">
              <span class="card-icon">{{ iconFor(card.name) }}</span>
              <span class="card-name" :title="card.path">{{ card.name }}</span>
            </div>
            <div class="card-meta">
              {{ sizeText(card.size) }} · {{ timeText(card.registered_at) }}
              <span v-if="card.skill" class="skill-tag">{{ card.skill }}</span>
            </div>
            <div class="card-actions">
              <button class="act" disabled title="预览在 P1b 接入">预览</button>
              <button class="act primary" @click="open(card)">打开</button>
            </div>
          </li>
        </ul>
        <p v-if="artifacts.length === 0" class="empty-hint">
          本任务暂无产物。产出的文件由办公技能自动登记至此。
        </p>
      </template>
    </div>
  </aside>
</template>

<style scoped>
.artifact-panel {
  background: var(--bg-panel);
  border-left: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.panel-title {
  padding: 12px 14px;
  font-size: var(--font-size-sm);
  font-weight: 600;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border);
}

.panel-body {
  flex: 1;
  overflow-y: auto;
  padding: 12px;
}

.task-line {
  margin: 0 0 10px;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}

.card-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.card {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 10px;
  background: var(--bg-base);
}

.card-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.card-icon {
  font-size: 18px;
}

.card-name {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.card-meta {
  margin-top: 4px;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}

.skill-tag {
  margin-left: 6px;
  background: var(--accent-soft);
  color: var(--accent);
  border-radius: 4px;
  padding: 1px 6px;
  font-size: var(--font-size-sm);
}

.card-actions {
  margin-top: 8px;
  display: flex;
  gap: 8px;
}

.act {
  flex: 1;
  border: 1px solid var(--border);
  background: var(--bg-panel);
  border-radius: 6px;
  padding: 5px 0;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}

.act.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}

.act:disabled {
  color: var(--text-disabled);
  border-color: var(--border);
}
</style>
