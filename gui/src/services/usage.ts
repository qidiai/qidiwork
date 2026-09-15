// 回合用量解析与格式化。
// 数据源:session/prompt 应答的 `_meta`(内核 cf-shell PromptResponseMeta /
// PromptUsage,camelCase)。解析按 feature-detect 宽松兼容双 casing:
// 内核字段演进时 GUI 只降级显示、不报错(桥层原样透传,见 bridge.rs)。
// 成本语义沿用内核 fail-close 约定:costUsdTicks 缺失 = 不可信,不显示;
// costIsPartial / usageIsIncomplete = 账单可能少计,显示时加 ≈ 前缀。

export interface UsageInfo {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  modelCalls: number;
  /** USD ticks(1e10 ticks = $1);null = 内核未给出可信成本 */
  costUsdTicks: number | null;
  costPartial: boolean;
  incomplete: boolean;
  turns: number;
  /** 本次回合实际使用的模型 id(modelUsage 键;缺省回退 meta.modelId) */
  models: string[];
}

/** 会话累计用量(逐回合累加)。costPartial/incomplete 一旦出现即置位,
 * 之后累计成本一律按 ≈ 显示。 */
export interface SessionUsage {
  inputTokens: number;
  outputTokens: number;
  turns: number;
  costUsdTicks: number;
  costSeen: boolean;
  costPartial: boolean;
  models: string[];
}

export function emptySessionUsage(): SessionUsage {
  return {
    inputTokens: 0,
    outputTokens: 0,
    turns: 0,
    costUsdTicks: 0,
    costSeen: false,
    costPartial: false,
    models: [],
  };
}

function asRecord(v: unknown): Record<string, unknown> | null {
  return v && typeof v === "object" && !Array.isArray(v)
    ? (v as Record<string, unknown>)
    : null;
}

function pickNum(o: Record<string, unknown>, ...keys: string[]): number {
  for (const k of keys) {
    const v = o[k];
    if (typeof v === "number" && Number.isFinite(v)) return v;
  }
  return 0;
}

function pickBool(o: Record<string, unknown>, ...keys: string[]): boolean {
  for (const k of keys) {
    if (o[k] === true) return true;
  }
  return false;
}

/** 从 `_meta` 提取本回合用量;无有效数据(缺 usage 或全零)返回 null。 */
export function parseUsageMeta(meta: unknown): UsageInfo | null {
  const m = asRecord(meta);
  if (!m) return null;
  const u = asRecord(m.usage ?? m.promptUsage);
  if (!u) return null;

  const modelUsage = asRecord(u.modelUsage ?? u.model_usage);
  const models = modelUsage ? Object.keys(modelUsage) : [];
  const modelId =
    typeof m.modelId === "string"
      ? m.modelId
      : typeof m.model_id === "string"
        ? m.model_id
        : "";
  if (!models.length && modelId) models.push(modelId);

  const cost = pickNum(u, "costUsdTicks", "cost_usd_ticks");

  const info: UsageInfo = {
    inputTokens: pickNum(u, "inputTokens", "input_tokens"),
    outputTokens: pickNum(u, "outputTokens", "output_tokens"),
    totalTokens: pickNum(u, "totalTokens", "total_tokens"),
    modelCalls: pickNum(u, "modelCalls", "model_calls"),
    costUsdTicks: cost > 0 ? cost : null,
    costPartial: pickBool(u, "costIsPartial", "cost_is_partial"),
    incomplete: pickBool(u, "usageIsIncomplete", "usage_is_incomplete"),
    turns: pickNum(u, "numTurns", "num_turns"),
    models,
  };
  // 无任何可展示信息(如 mock 内核)不上屏;cost-only 回合仍保留
  // (k3 审计 Note4:门槛不看成本会丢"仅成本非零"的边缘回合)
  if (!info.inputTokens && !info.outputTokens && !info.models.length && info.costUsdTicks == null)
    return null;
  return info;
}

/** 把一回合用量累加进会话累计。 */
export function accumulateUsage(acc: SessionUsage, u: UsageInfo | null): void {
  if (!u) return;
  acc.inputTokens += u.inputTokens;
  acc.outputTokens += u.outputTokens;
  acc.turns += 1;
  if (u.costUsdTicks != null) {
    acc.costUsdTicks += u.costUsdTicks;
    acc.costSeen = true;
  }
  if (u.costPartial || u.incomplete) acc.costPartial = true;
  for (const model of u.models) {
    if (!acc.models.includes(model)) acc.models.push(model);
  }
}

/** 1234 → "1.2k";12345678 → "12.3M";<1000 原样。 */
export function fmtTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1000 * 1000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / (1000 * 1000)).toFixed(1)}M`;
}

/** ticks → "$0.0123";不可信返回 null。 */
export function fmtCost(costUsdTicks: number | null): string | null {
  if (costUsdTicks == null) return null;
  return `$${(costUsdTicks / 1e10).toFixed(4)}`;
}
