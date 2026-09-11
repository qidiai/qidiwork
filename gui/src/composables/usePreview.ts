// 预览 Tab 注册表:产物卡片「预览」→ 此处登记 → MainTabs 渲染对应 Tab。
import { ref } from "vue";

export interface PreviewTab {
  /** 稳定键 = MainTabs 的 Tab key(preview:<task>/<name>),两端共用 previewKey 派生 */
  key: string;
  task: string;
  name: string;
}

const openPreviews = ref<PreviewTab[]>([]);
/** 最近一次请求激活的预览 Tab key;MainTabs watch 它切换激活页 */
const activePreview = ref("");
/** 激活请求序号:Vue ref 同值赋值不通知,同 key 重开需靠序号强制触发激活 watch */
const activationSeq = ref(0);

/** 注册表 key 派生规则(ArtifactPanel 开 Tab 与 MainTabs 关 Tab 共用,保证同链同格式) */
export function previewKey(task: string, name: string): string {
  return `preview:${task}/${name}`;
}

export function openPreview(tab: PreviewTab): void {
  if (!openPreviews.value.some((t) => t.key === tab.key)) {
    openPreviews.value.push(tab);
  }
  activePreview.value = tab.key;
  activationSeq.value++;
}

export function closePreview(key: string): void {
  const idx = openPreviews.value.findIndex((t) => t.key === key);
  if (idx >= 0) openPreviews.value.splice(idx, 1);
  // 残留的 activePreview 会让"重开同一产物"因 ref 同值不触发
  // MainTabs 的激活 watch,Tab 重新出现但不激活(deepseek 交叉审计 B1)。
  if (activePreview.value === key) activePreview.value = "";
}

/** 任务切换时清理全部预览:旧任务的产物 Tab 不跨工作区残留(N6)。 */
export function closeAllPreviews(): void {
  openPreviews.value.splice(0);
  activePreview.value = "";
}

export function usePreviewTabs() {
  return { openPreviews, activePreview, activationSeq };
}
