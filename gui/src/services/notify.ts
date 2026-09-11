// 系统通知服务(方案 v2 P2):任务完成/需审批且窗口失焦时提醒。
// 只在失焦时发,避免用户盯着窗口时的通知噪音;权限在启动时请求一次。
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

export function isWindowFocused(): boolean {
  return document.hasFocus();
}

/** 应用启动时调用一次;Windows 上通常直接授予,失败静默(通知是增强项)。 */
export async function initNotifyPermission(): Promise<void> {
  try {
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === "granted";
    if (!granted) {
      console.warn("系统通知权限未授予,失焦提醒不可用");
    }
  } catch (e) {
    console.warn("系统通知初始化失败", e);
  }
}

/** 发一条系统通知(调用方自行判断失焦)。权限缺失时静默。 */
export function notify(title: string, body: string): void {
  void (async () => {
    try {
      if (await isPermissionGranted()) sendNotification({ title, body });
    } catch (e) {
      console.warn("系统通知发送失败", e);
    }
  })();
}
