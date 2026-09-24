// 首次启动引导(onboarding)状态与门控(R10 / K3 审计修正版)。
//
// 完成标记**双写**:
//   - localStorage `qidi.onboarded`(前端即时可见);
//   - `<app_data_dir>/onboarding_done` 文件(Rust 命令,跨 localStorage 清空可恢复)。
// `shouldShowOnboarding()` 两处任一缺失即视为未完成 → 展示引导。
//
// 门控(见 App.vue):引导未完成前不 initAgent / autoConnect,
// 避免未登录态 spawn 出无效内核进程(K3 修正 1)。
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

/** 完成标记的 localStorage 键(与后端标记文件双写)。 */
const LS_KEY = "qidi.onboarded";

/** 引导步骤总数:1 欢迎 / 2 登录 / 3 选择模型。 */
export const TOTAL_STEPS = 3;

/** 当前步骤(1 起)。 */
export const currentStep = ref(1);

/** 是否已走完/跳过引导(完成标记写入后置位)。 */
const finished = ref(false);

/** 引导完成状态(响应式;供 OnboardingView 判断是否 emit done)。 */
export const isComplete = computed<boolean>(() => finished.value);

/** 服务端可用模型(server_models 命令返回;OpenAI /v1/models 投影)。 */
export interface ServerModel {
  id: string;
  name: string;
  /** 描述,服务端未提供时为 null。 */
  description: string | null;
}

/**
 * 拉取服务端模型列表。token 在 Rust 侧,前端不直连 /v1/models
 * (GUI 的 CSP 亦禁直连网络)。
 */
export function fetchServerModels(): Promise<ServerModel[]> {
  return invoke<ServerModel[]>("server_models");
}

/**
 * 引导用登录态检测:读 auth.json(轻量、不走网络),供 Step 2 轮询
 * 「浏览器授权是否完成」。
 */
export function checkOnboardingAuth(): Promise<boolean> {
  return invoke<boolean>("check_onboarding_auth");
}

/**
 * 是否需要展示引导:localStorage 与标记文件任一缺失即为真。
 * 标记文件命令不可用(旧后端)时保守视为未完成。
 */
export async function shouldShowOnboarding(): Promise<boolean> {
  const ls = localStorage.getItem(LS_KEY) === "true";
  let file = false;
  try {
    file = await invoke<boolean>("onboarding_marker_read");
  } catch {
    file = false;
  }
  return !(ls && file);
}

/**
 * 完成引导:双写完成标记(localStorage + app_data_dir 文件)。
 * 返回 markerOk:文件写入失败时调用方需如实告知——门控要求两处都在,
 * 否则下次启动会再次引导,静默重复会消耗用户耐心。
 */
export async function completeOnboarding(): Promise<{ markerOk: boolean }> {
  localStorage.setItem(LS_KEY, "true");
  let markerOk = false;
  try {
    await invoke("onboarding_marker_write");
    markerOk = true;
  } catch {
    /* localStorage 已置位,本次仍可进入 */
  }
  finished.value = true;
  return { markerOk };
}

export function useOnboarding() {
  return {
    currentStep,
    isComplete,
    fetchServerModels,
    checkOnboardingAuth,
    shouldShowOnboarding,
    completeOnboarding,
  };
}
