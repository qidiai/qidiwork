<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { initOffice, openArtifact, useOfficeState, type ArtifactCard } from "../composables/useOffice";
import { openPreview, previewKey } from "../composables/usePreview";
import { pushSystem, useAgentState } from "../composables/useAgent";
import { documentsBySession, loadDocuments } from "../composables/useOfficeFiles";
import { panelCollapsed, panelWidth, PANEL_MIN, PANEL_MAX } from "../composables/useUiLayout";

const { currentTask, artifacts } = useOfficeState();
const { sessionId, connected } = useAgentState();
const documents = computed(() => documentsBySession.value[sessionId.value] ?? []);
const documentError = ref("");
const loading = ref(false);
const opening = ref("");
const unseen = ref(false);

async function refreshDocuments(): Promise<void> {
  const id = sessionId.value;
  documentError.value = "";
  if (!id || !connected.value) return;
  loading.value = true;
  try {
    await loadDocuments(id);
  } catch (e) {
    if (sessionId.value === id) documentError.value = `资料列表读取失败：${String(e)}`;
  } finally {
    if (sessionId.value === id) loading.value = false;
  }
}
watch([sessionId, connected], () => { loading.value = false; void refreshDocuments(); }, { immediate: true });
watch(() => artifacts.value.length, (count, previous) => {
  if (count > previous && panelCollapsed.value) unseen.value = true;
});
watch(panelCollapsed, (collapsed) => { if (!collapsed) unseen.value = false; });
void initOffice();

async function open(card: ArtifactCard): Promise<void> {
  opening.value = card.path;
  try { await openArtifact(card); }
  catch (e) { pushSystem(`打开文件失败：${String(e)}`); }
  finally { opening.value = ""; }
}
function preview(card: ArtifactCard): void {
  openPreview({ key: previewKey(currentTask.value, card.name), task: currentTask.value, name: card.name });
}
function sizeText(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}
function fileKind(name: string): string { return name.split(".").pop()?.toUpperCase() ?? "文件"; }
function startResize(e: MouseEvent): void {
  const startX = e.clientX;
  const startW = panelWidth.value;
  const move = (ev: MouseEvent) => { panelWidth.value = Math.min(PANEL_MAX, Math.max(PANEL_MIN, startW - (ev.clientX - startX))); };
  const up = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); };
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", up);
  e.preventDefault();
}
</script>

<template>
  <aside v-if="panelCollapsed" class="artifact-rail">
    <button class="rail-expand" title="展开资料与成果" @click="panelCollapsed = false">«</button>
    <button class="rail-text" @click="panelCollapsed = false">资料与成果</button>
    <span v-if="unseen" class="rail-dot" aria-label="有新成果"></span>
  </aside>
  <aside v-else class="artifact-panel" aria-label="资料与成果">
    <div class="resize-handle" @mousedown="startResize"></div>
    <div class="panel-title">资料与成果<button class="collapse-btn" title="折叠资料与成果" @click="panelCollapsed = true">»</button></div>
    <div class="panel-body">
      <section class="file-section">
        <div class="section-heading"><h2>参考资料 <span>{{ documents.length }}</span></h2><button v-if="sessionId && connected" class="text-btn" :disabled="loading" @click="refreshDocuments">{{ loading ? '读取中…' : '刷新' }}</button></div>
        <p class="section-description">当前任务导入的文档副本</p>
        <div v-if="documentError" class="file-error" role="alert">{{ documentError }}</div>
        <ul v-if="documents.length" class="card-list">
          <li v-for="doc in documents" :key="doc.path" class="reference-card">
            <div class="card-head"><span class="file-type">{{ fileKind(doc.name) }}</span><span class="card-name" :title="doc.name">{{ doc.name }}</span></div>
            <div class="card-meta">{{ sizeText(doc.size) }} · 本地副本</div>
            <details class="file-location"><summary>查看保存位置</summary><p>{{ doc.path }}</p></details>
          </li>
        </ul>
        <div v-else-if="!loading && !documentError" class="empty-state"><span class="empty-symbol">＋</span><p>把资料带进来</p><small>在输入框添加文档并发送后，<br>参考资料会保存在这里。</small></div>
      </section>
      <section class="file-section results-section">
        <div class="section-heading"><h2>成果文件 <span>{{ artifacts.length }}</span></h2></div>
        <p class="section-description">{{ currentTask ? `文件分组：${currentTask}` : '完成后的文件，随时查看和编辑' }}</p>
        <ul v-if="artifacts.length" class="card-list">
          <li v-for="card in artifacts" :key="card.path" class="card">
            <div class="card-head"><span class="file-type">{{ fileKind(card.name) }}</span><span class="card-name" :title="card.name">{{ card.name }}</span></div>
            <div class="card-meta">{{ sizeText(card.size) }}</div>
            <p v-if="card.note" class="card-note">{{ card.note }}</p>
            <div class="card-actions"><button class="act" @click="preview(card)">查看预览</button><button class="act primary" :disabled="!!opening" title="使用电脑上的默认程序打开，例如 WPS 或 Office" @click="open(card)">{{ opening === card.path ? '打开中…' : '打开文件' }}</button></div>
            <details class="file-location"><summary>查看保存位置</summary><p>{{ card.path }}</p></details>
          </li>
        </ul>
        <div v-else class="empty-state"><span class="empty-symbol">▤</span><p>成果会出现在这里</p><small>预览后可以继续提修改意见，<br>也可以用 WPS / Office 编辑。</small></div>
      </section>
    </div>
  </aside>
</template>

<style scoped>
.artifact-panel { background: var(--bg-panel); border-left: 1px solid var(--border); display: flex; flex-direction: column; min-height: 0; min-width: 0; position: relative; }
.artifact-rail { background: var(--bg-panel); border-left: 1px solid var(--border); display: flex; flex-direction: column; align-items: center; padding-top: 12px; gap: 12px; }
.rail-expand, .rail-text { border: 0; background: none; color: var(--text-secondary); padding: 2px; }
.rail-text { writing-mode: vertical-rl; font-size: 12px; letter-spacing: .15em; }
.rail-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--accent); }
.resize-handle { position: absolute; left: -3px; top: 0; bottom: 0; width: 7px; cursor: col-resize; z-index: 5; }
.resize-handle:hover { background: var(--accent-soft); }
.panel-title { display: flex; justify-content: space-between; align-items: center; height: 48px; flex-shrink: 0; padding: 12px 18px; font-size: 13px; font-weight: 600; border-bottom: 1px solid var(--border); }
.collapse-btn { border: 0; background: none; color: var(--text-secondary); font-size: 16px; }
.panel-body { flex: 1; overflow-y: auto; padding: 18px 16px; }
.section-heading { display: flex; align-items: center; justify-content: space-between; }
h2 { font-size: 13px; font-weight: 600; margin: 0; }
h2 span { color: var(--text-secondary); margin-left: 6px; font-size: 12px; font-weight: 400; }
.text-btn { border: 0; background: none; color: var(--accent); font-size: 12px; }
.section-description { font-size: 12px; color: var(--text-secondary); margin: 8px 0 16px; overflow-wrap: anywhere; }
.results-section { border-top: 1px solid var(--border); margin-top: 24px; padding-top: 24px; }
.card-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 10px; }
.card, .reference-card { border: 1px solid var(--border); border-radius: 9px; padding: 12px; background: var(--bg-panel); }
.reference-card { background: var(--bg-base); border-color: transparent; }
.card-head { display: flex; align-items: center; gap: 8px; }
.file-type { flex-shrink: 0; font-size: 10px; font-weight: 600; color: var(--accent); padding: 5px; border: 1px solid var(--accent-soft); border-radius: 4px; }
.card-name { font-weight: 500; font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.card-meta { margin-top: 8px; font-size: 11px; color: var(--text-secondary); }
.card-note { font-size: 12px; line-height: 1.6; color: var(--text-secondary); overflow-wrap: anywhere; }
.card-actions { display: flex; gap: 8px; margin-top: 12px; }
.act { flex: 1; border: 1px solid var(--border); background: var(--bg-panel); border-radius: 6px; padding: 6px 4px; font-size: 12px; color: var(--text-secondary); }
.act.primary { background: var(--accent); border-color: var(--accent); color: #fff; }
.act:disabled { opacity: .6; }
.file-location { margin-top: 10px; font-size: 11px; color: var(--text-secondary); }
.file-location summary { cursor: pointer; }
.file-location p { user-select: text; overflow-wrap: anywhere; line-height: 1.6; }
.empty-state { text-align: center; padding: 20px 10px; background: var(--bg-base); border-radius: 8px; color: var(--text-secondary); }
.empty-symbol { font-size: 23px; color: #9caac0; }
.empty-state p { font-size: 13px; margin: 10px 0 6px; }
.empty-state small { font-size: 12px; line-height: 1.8; }
.file-error { font-size: 12px; color: var(--danger); overflow-wrap: anywhere; }
</style>
