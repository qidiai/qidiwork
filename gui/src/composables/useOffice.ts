// 办公工作台状态:工作区列表 + 当前任务产物卡片。
// 数据面来自 commands.rs:office_scan / office_artifacts / office_open /
// office_watch_start;实时刷新走 office-event(manifest 变更推送)。
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface WorkspaceInfo {
  name: string;
  artifact_count: number;
}

export interface ArtifactCard {
  path: string;
  name: string;
  size: number;
  mtime: number;
  registered_at: number;
  skill: string;
  note: string;
}

const workspaces = ref<WorkspaceInfo[]>([]);
const currentTask = ref("");
const artifacts = ref<ArtifactCard[]>([]);
let bound = false;

async function refreshWorkspaces(): Promise<void> {
  workspaces.value = await invoke<WorkspaceInfo[]>("office_scan");
}

/** 应用启动时调用一次(幂等):启动 manifest 监听 + 首次扫描。 */
export async function initOffice(): Promise<void> {
  if (bound) return;
  bound = true;
  await invoke("office_watch_start");
  // 事件是"失效通知"(k3 P1a 审计 §4):收到后重拉,天然消除
  // 快照与拉取的乱序覆盖。当前 task 重拉产物;工作区计数全量刷新。
  const unlisten: UnlistenFn = await listen<{ task: string }>("office-event", (e) => {
    void (async () => {
      if (e.payload.task === currentTask.value) {
        await switchTask(currentTask.value);
      }
      await refreshWorkspaces();
    })();
  });
  void unlisten;
  await refreshWorkspaces();
  if (workspaces.value.length > 0 && !currentTask.value) {
    await switchTask(workspaces.value[0].name);
  }
}

/** 切换当前查看的工作区(产物视图)。 */
export async function switchTask(name: string): Promise<void> {
  currentTask.value = name;
  artifacts.value = await invoke<ArtifactCard[]>("office_artifacts", { task: name });
}

/** 用系统默认程序打开产物(白名单+路径约束在 Rust 侧)。 */
export async function openArtifact(card: ArtifactCard): Promise<void> {
  // office_open 按 (task, name) 服务端重读 manifest 校验(k3 P1a 审计 P1),
  // 不接受裸路径——传 card.path 会被拒绝。
  await invoke("office_open", { task: currentTask.value, name: card.name });
}

/** 预览降级路径:直接按 task+name 唤起系统程序。 */
export async function openPreviewArtifact(task: string, name: string): Promise<void> {
  await invoke("office_open", { task, name });
}

export function useOfficeState() {
  return { workspaces, currentTask, artifacts };
}
