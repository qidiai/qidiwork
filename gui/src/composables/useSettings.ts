// 设置面数据源:读写 ~/.qidi/config.toml 的模型选择与 API key。
// 密钥卫生:settings_read 永不回传明文,只回传 has_api_key。
import { invoke } from "@tauri-apps/api/core";

export interface ModelOption {
  id: string;
  name: string;
  base_url: string | null;
  has_api_key: boolean;
}

export interface SettingsInfo {
  default_model: string | null;
  models: ModelOption[];
}

export function readSettings(): Promise<SettingsInfo> {
  return invoke<SettingsInfo>("settings_read");
}

/** 保存默认模型;apiKey 非空时一并写入该模型配置。 */
export function saveSettings(modelId: string, apiKey: string | null): Promise<void> {
  return invoke("settings_save", { modelId, apiKey });
}

/** 停止内核:下一条任务发送时自动以新配置重启(session_start 兜底)。 */
export function stopKernel(): Promise<void> {
  return invoke("agent_stop");
}
