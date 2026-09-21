// 三列布局的折叠/宽度共享状态。项目内首个 localStorage 持久化模式:
// 用户手工调整过什么,下次启动就保持什么(两侧面板折叠态、右栏宽度)。
import { ref, watch } from "vue";

/** 左栏折叠后的图标条宽度 / 右栏折叠后的竖条宽度(px)。 */
export const SIDEBAR_STRIP = 44;
export const PANEL_STRIP = 34;
export const PANEL_MIN = 280;
export const PANEL_MAX = 560;

function load(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

// ≤1100px 且用户没有存过偏好时,默认折叠产物面板,优先保聊天区。
const narrow = typeof window !== "undefined" && window.innerWidth <= 1100;

export const sidebarCollapsed = ref(load("qidi.sidebarCollapsed") === "1");
export const panelCollapsed = ref(
  load("qidi.panelCollapsed") !== null
    ? load("qidi.panelCollapsed") === "1"
    : narrow
);
export const panelWidth = ref(
  Math.min(PANEL_MAX, Math.max(PANEL_MIN, Number(load("qidi.panelWidth")) || 320))
);

watch(sidebarCollapsed, (v) => localStorage.setItem("qidi.sidebarCollapsed", v ? "1" : "0"));
watch(panelCollapsed, (v) => localStorage.setItem("qidi.panelCollapsed", v ? "1" : "0"));
watch(panelWidth, (v) => localStorage.setItem("qidi.panelWidth", String(v)));
