// 账号登录数据源:auth_status(登录态/套餐/配额)、auth_login(设备码)、auth_logout。
// token 只在 Rust 侧,前端只见脱敏 AuthStatus。登录进度经 auth-event 事件推送。
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface UsageToday {
  requests: number;
  tokens: number;
}

export interface Quota {
  requests_limit: number;
  tokens_limit: number;
  requests_left: number;
  tokens_left: number;
}

export interface AuthStatus {
  logged_in: boolean;
  email: string | null;
  user_id: string | null;
  plan: string | null;
  usage_today: UsageToday | null;
  quota: Quota | null;
  detail: string | null;
}

export interface AuthEvent {
  phase: "starting" | "pending" | "success" | "error";
  verification_uri?: string | null;
  user_code?: string | null;
  message?: string | null;
}

const status = ref<AuthStatus | null>(null);
const loading = ref(false);
const error = ref("");
// 登录过程中的设备码/URL 与错误(展示在设置页登录区)。
const loginPending = ref(false);
const loginUri = ref("");
const loginCode = ref("");
const loginError = ref("");

let bound = false;

async function refresh(): Promise<void> {
  loading.value = true;
  error.value = "";
  try {
    status.value = await invoke<AuthStatus>("auth_status");
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

function bindEvents(): void {
  if (bound) return;
  bound = true;
  void listen<AuthEvent>("auth-event", (ev) => {
    const p = ev.payload;
    if (p.phase === "pending") {
      loginUri.value = p.verification_uri ?? "";
      loginCode.value = p.user_code ?? "";
      loginError.value = "";
    } else if (p.phase === "success") {
      loginPending.value = false;
      loginUri.value = "";
      loginCode.value = "";
      loginError.value = "";
      void refresh();
    } else if (p.phase === "error") {
      loginPending.value = false;
      loginError.value = p.message ?? "登录失败";
    }
  });
}

/** 触发设备码登录:内核打开浏览器 + 轮询,进度经 auth-event 回流。 */
async function startLogin(): Promise<void> {
  bindEvents();
  loginError.value = "";
  loginUri.value = "";
  loginCode.value = "";
  loginPending.value = true;
  try {
    await invoke("auth_login");
  } catch (e) {
    loginPending.value = false;
    loginError.value = String(e);
  }
}

async function logout(): Promise<void> {
  error.value = "";
  try {
    status.value = await invoke<AuthStatus>("auth_logout");
  } catch (e) {
    error.value = String(e);
  }
  loginPending.value = false;
  loginUri.value = "";
  loginCode.value = "";
}

export function useAuth() {
  return {
    status,
    loading,
    error,
    loginPending,
    loginUri,
    loginCode,
    loginError,
    refresh,
    startLogin,
    logout,
  };
}
