<script setup lang="ts">
// 导出标书弹窗(P1-3 前端接入):Markdown → docx,走后端 `bid_export_md` 命令。
// 与 CLI `bid-export --md` 同一终点(同一条 parse_markdown_with + fill::export 链路),
// 模板固定为内置默认样式,因此这里不暴露 style_id / 图宽参数。
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { pushSystem } from "../composables/useAgent";

const emit = defineEmits<{ close: [] }>();

/** 与后端 BidExportResult 同构(serde 默认字段名,均为 snake_case)。 */
type BidExportResult = {
  path: string;
  bytes: number;
  chapters: number;
  paragraphs: number;
  tables: number;
  figures: number;
};

const mdPath = ref("");
const outPath = ref("");
// 勾选 = --no-number(章节不自动编号);取消勾选即开启自动编号。
const noNumber = ref(false);
const busy = ref(false);
const error = ref("");
const result = ref<BidExportResult | null>(null);

const canExport = computed(
  () => !busy.value && mdPath.value.trim().length > 0 && outPath.value.trim().length > 0,
);

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(2)} MB`;
}

/** 选 Markdown 文件;输出路径留空时按同名 .docx 预填(可再手改)。 */
async function pickMd(): Promise<void> {
  error.value = "";
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({
      multiple: false,
      directory: false,
      title: "选择 Markdown 文件",
      filters: [{ name: "Markdown", extensions: ["md", "markdown"] }],
    });
    if (typeof picked !== "string" || !picked) return;
    mdPath.value = picked;
    if (!outPath.value.trim()) {
      outPath.value = picked.replace(/\.(md|markdown)$/i, "") + ".docx";
    }
  } catch (e) {
    error.value = `选择文件失败:${String(e)}`;
  }
}

async function doExport(): Promise<void> {
  if (busy.value) return;
  error.value = "";
  result.value = null;
  if (!mdPath.value.trim()) {
    error.value = "请填写 Markdown 路径";
    return;
  }
  if (!outPath.value.trim()) {
    error.value = "请填写输出的 docx 路径";
    return;
  }
  busy.value = true;
  try {
    const res = await invoke<BidExportResult>("bid_export_md", {
      mdPath: mdPath.value.trim(),
      outPath: outPath.value.trim(),
      number: !noNumber.value,
    });
    result.value = res;
    pushSystem(`标书已导出:${res.path}`);
  } catch (e) {
    // 后端错误文案已含具体原因(读文件失败 / 解析失败 / 模板缺失等),原样展示。
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div class="modal-mask" @click.self="emit('close')">
    <div class="modal">
      <div class="modal-head">
        <span class="modal-title">导出标书</span>
        <button class="modal-close" @click="emit('close')">✕</button>
      </div>

      <div class="modal-body">
        <label class="field">
          <span class="field-label">Markdown 路径</span>
          <div class="path-row">
            <input
              v-model="mdPath"
              class="input"
              placeholder="D:\项目\chapters\标书.md"
              spellcheck="false"
            />
            <button class="btn ghost" :disabled="busy" @click="pickMd">选择…</button>
          </div>
          <span class="field-hint">图片的相对路径按该 md 文件所在目录解析。</span>
        </label>

        <label class="field">
          <span class="field-label">输出 docx 路径</span>
          <input
            v-model="outPath"
            class="input"
            placeholder="D:\项目\标书.docx"
            spellcheck="false"
          />
          <span class="field-hint">绝对路径;父目录不存在会自动创建,同名文件直接覆盖。</span>
        </label>

        <label class="check-row">
          <input v-model="noNumber" type="checkbox" :disabled="busy" />
          <span>不自动编号(章节标题原样输出)</span>
        </label>

        <p v-if="error" class="err">{{ error }}</p>

        <div v-if="result" class="result-box">
          <div class="result-title">导出成功</div>
          <p class="result-path">{{ result.path }}</p>
          <div class="result-grid">
            <div class="result-cell"><span class="num">{{ result.chapters }}</span><span class="lbl">章</span></div>
            <div class="result-cell"><span class="num">{{ result.paragraphs }}</span><span class="lbl">段落</span></div>
            <div class="result-cell"><span class="num">{{ result.tables }}</span><span class="lbl">表格</span></div>
            <div class="result-cell"><span class="num">{{ result.figures }}</span><span class="lbl">图</span></div>
            <div class="result-cell"><span class="num">{{ fmtBytes(result.bytes) }}</span><span class="lbl">文件大小</span></div>
          </div>
        </div>
      </div>

      <div class="modal-foot">
        <button class="btn ghost" @click="emit('close')">关闭</button>
        <button class="btn primary" :disabled="!canExport" @click="doExport">
          {{ busy ? "导出中…" : "导出" }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.modal-mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.modal {
  width: 520px;
  max-width: calc(100vw - 48px);
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: 12px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.35);
  display: flex;
  flex-direction: column;
}

.modal-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 18px;
  border-bottom: 1px solid var(--border);
}

.modal-title {
  font-weight: 600;
  color: var(--text-primary);
}

.modal-close {
  background: none;
  border: none;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 14px;
}

.modal-body {
  padding: 16px 18px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  color: var(--text-primary);
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field-label {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  font-weight: 600;
}

.field-hint {
  font-size: var(--font-size-sm);
  color: var(--text-disabled);
}

.input {
  flex: 1;
  min-width: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 8px 10px;
  background: var(--bg-panel);
  color: var(--text-primary);
  font-size: var(--font-size-sm);
}

.path-row {
  display: flex;
  gap: 8px;
  align-items: center;
}

.check-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--font-size-sm);
  color: var(--text-primary);
  cursor: pointer;
}

.err {
  color: var(--danger, #c0392b);
  font-size: var(--font-size-sm);
  margin: 0;
  word-break: break-all;
}

.result-box {
  display: flex;
  flex-direction: column;
  gap: 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 12px 14px;
}

.result-title {
  font-weight: 600;
  color: var(--success);
}

.result-path {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  word-break: break-all;
}

.result-grid {
  display: grid;
  grid-template-columns: repeat(5, 1fr);
  gap: 8px;
}

.result-cell {
  display: flex;
  flex-direction: column;
  gap: 2px;
  align-items: center;
  padding: 6px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
}

.num {
  font-weight: 600;
  color: var(--text-primary);
}

.lbl {
  font-size: var(--font-size-sm);
  color: var(--text-disabled);
}

.modal-foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 18px;
  border-top: 1px solid var(--border);
}

.btn {
  border: 1px solid var(--border);
  background: var(--bg-panel);
  color: var(--text-primary);
  border-radius: var(--radius);
  padding: 7px 16px;
  font-size: var(--font-size-sm);
  cursor: pointer;
  white-space: nowrap;
}

.btn:hover:not(:disabled) {
  background: var(--bg-hover);
}

.btn:disabled {
  cursor: not-allowed;
  opacity: 0.6;
}

.btn.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}

.btn.primary:hover:not(:disabled) {
  filter: brightness(1.1);
}

.btn.ghost {
  background: transparent;
}
</style>
